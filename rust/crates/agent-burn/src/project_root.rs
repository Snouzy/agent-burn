//! Resolves session working directories to the git project that owns them.
//!
//! Paths are normalized lexically and checked ancestor by ancestor, so sessions
//! whose folders were deleted since still land on the repository above them.
//! Linked worktrees resolve to their main repository through `commondir`.
//! A working folder outside any repository is a project of its own when it is
//! a leaf; folders that hold repositories, home, and temp folders are not.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use crate::fast::FxHashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Project {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) kind: ProjectKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProjectKind {
    Repository,
    /// A working folder outside any repository.
    Folder,
}

pub(crate) struct ProjectResolver {
    repositories: RepositoryCache,
    folders: FolderRoots,
    roots: FxHashMap<String, Root>,
}

/// Temp folders, whose sessions are throwaway work rather than projects.
const SCRATCH_ROOTS: &[&str] = &[
    "/tmp",
    "/private/tmp",
    "/var/folders",
    "/private/var/folders",
];
const CLAUDE_SCRATCH_WORKSPACES: &str = "Application Support/Claude/scratch-workspaces";
/// Standard home folders: never projects themselves, and they stop the climb.
const HOME_FOLDERS: &[&str] = &[
    "Applications",
    "Desktop",
    "Documents",
    "Downloads",
    "Library",
    "Movies",
    "Music",
    "Pictures",
    "Public",
];
/// Home subtrees that hold app data or per-task folders, never projects.
const HOME_SCRATCH_ROOTS: &[&str] = &["Library", "Documents/Codex"];
/// Caps the `.git` probes per container check, so folders with many
/// subfolders stay cheap. Dependency and build output folders are skipped.
const MAX_CONTAINER_PROBES: usize = 512;
const SKIPPED_SUBFOLDERS: &[&str] = &["node_modules", "target", "dist", "build", "vendor", "Pods"];

/// Finds the folder project of a working folder outside any repository: its
/// highest ancestor below a container (a folder with a repository among its
/// children or grandchildren), home, or a temp folder.
struct FolderRoots {
    home: Option<PathBuf>,
    scratch_roots: Vec<PathBuf>,
    containers: FxHashMap<PathBuf, bool>,
}

/// Memoized repository root per directory, so repeated lookups in the same
/// folders cost a hash lookup instead of filesystem calls.
struct RepositoryCache {
    home: Option<PathBuf>,
    ancestors: FxHashMap<PathBuf, Option<Arc<str>>>,
}

thread_local! {
    static REPOSITORIES: RefCell<RepositoryCache> =
        RefCell::new(RepositoryCache::new(crate::home::home_dir()));
}

/// Root of the git repository that contains an absolute `path`, if any.
pub(crate) fn repository_of(path: &str) -> Option<Arc<str>> {
    let path = Path::new(path);
    if !path.is_absolute() {
        return None;
    }
    REPOSITORIES.with(|cache| {
        let mut cache = cache.borrow_mut();
        match cache.ancestors.get(path) {
            Some(root) => root.clone(),
            None => cache.root(&normalize(path)),
        }
    })
}

/// Picks the repository each turn of one session file worked in: the one its
/// tool calls referenced most (ties go to the last one), else the previous
/// turn's. `first` lets callers attribute turns before the first touch.
#[derive(Default)]
pub(crate) struct TurnRepositories {
    touches: Vec<(Arc<str>, usize, usize)>,
    references: usize,
    last: Option<Arc<str>>,
    first: Option<Arc<str>>,
}

impl TurnRepositories {
    pub(crate) fn touch(&mut self, path: &str) {
        if let Some(repository) = repository_of(path) {
            self.touch_repository(repository);
        }
    }

    fn touch_repository(&mut self, repository: Arc<str>) {
        self.references += 1;
        match self
            .touches
            .iter_mut()
            .find(|(touched, _, _)| *touched == repository)
        {
            Some((_, count, last)) => {
                *count += 1;
                *last = self.references;
            }
            None => self.touches.push((repository, 1, self.references)),
        }
    }

    /// Ends the current turn. Returns the repository the turn touched itself,
    /// and the one it is attributed to (its own, else the previous turn's).
    pub(crate) fn finish_turn(&mut self) -> (Option<Arc<str>>, Option<Arc<str>>) {
        let own = self
            .touches
            .drain(..)
            .max_by_key(|(_, count, last)| (*count, *last))
            .map(|(repository, _, _)| repository);
        if let Some(repository) = &own {
            self.first.get_or_insert_with(|| Arc::clone(repository));
            self.last = Some(Arc::clone(repository));
        }
        (own, self.last.clone())
    }

    pub(crate) fn first(&self) -> Option<&Arc<str>> {
        self.first.as_ref()
    }
}

struct Root {
    path: PathBuf,
    from_git: bool,
    kind: Option<ProjectKind>,
}

impl ProjectResolver {
    pub(crate) fn new(home: Option<PathBuf>) -> Self {
        let scratch_roots = SCRATCH_ROOTS.iter().map(PathBuf::from).collect();
        Self {
            folders: FolderRoots::new(home.clone(), scratch_roots),
            repositories: RepositoryCache::new(home),
            roots: FxHashMap::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_scratch_roots(mut self, scratch_roots: Vec<PathBuf>) -> Self {
        self.folders.scratch_roots = scratch_roots;
        self
    }

    pub(crate) fn add(&mut self, cwd: &str) {
        if self.roots.contains_key(cwd) {
            return;
        }
        let path = normalize(Path::new(cwd));
        let root = match self.repositories.root(&path) {
            Some(path) => Root {
                path: PathBuf::from(path.as_ref()),
                from_git: true,
                kind: Some(ProjectKind::Repository),
            },
            None => Root {
                path,
                from_git: false,
                kind: None,
            },
        };
        self.roots.insert(cwd.to_owned(), root);
    }

    /// Maps every added cwd to its project, merging Codex worktrees that no
    /// longer exist into the repository they were created from. Cwds that are
    /// in no project map to `None`.
    pub(crate) fn finish(mut self) -> FxHashMap<String, Option<Project>> {
        let mut git_roots_by_name = BTreeMap::<&OsStr, BTreeSet<&Path>>::new();
        for root in self.roots.values().filter(|root| root.from_git) {
            if let Some(name) = root.path.file_name() {
                git_roots_by_name
                    .entry(name)
                    .or_default()
                    .insert(&root.path);
            }
        }
        let mut merges = Vec::new();
        let mut orphans = Vec::new();
        for (cwd, root) in self.roots.iter().filter(|(_, root)| !root.from_git) {
            let Some(worktree) = codex_worktree(&root.path) else {
                continue;
            };
            match git_roots_by_name.get(worktree.file_name().unwrap_or_default()) {
                Some(repos) if repos.len() == 1 => {
                    merges.push((cwd.clone(), repos.first().unwrap().to_path_buf()));
                }
                _ => orphans.push((cwd.clone(), worktree)),
            }
        }
        // Worktrees without a unique repository are grouped by folder name
        // under the first worktree path.
        let mut orphan_roots = BTreeMap::<OsString, PathBuf>::new();
        for (_, worktree) in &orphans {
            let name = worktree.file_name().unwrap_or_default().to_owned();
            let shared = orphan_roots.entry(name).or_insert_with(|| worktree.clone());
            if worktree < shared {
                *shared = worktree.clone();
            }
        }
        for (cwd, worktree) in orphans {
            let shared = orphan_roots[worktree.file_name().unwrap_or_default()].clone();
            merges.push((cwd, shared));
        }
        for (cwd, path) in merges {
            if let Some(root) = self.roots.get_mut(&cwd) {
                root.path = path;
                root.kind = Some(ProjectKind::Repository);
            }
        }
        for root in self.roots.values_mut().filter(|root| root.kind.is_none()) {
            if let Some(folder) = self.folders.root(&root.path) {
                root.path = folder;
                root.kind = Some(ProjectKind::Folder);
            }
        }

        let names = display_names(
            self.roots
                .values()
                .filter(|root| root.kind.is_some())
                .map(|root| root.path.as_path()),
        );
        self.roots
            .iter()
            .map(|(cwd, root)| {
                let project = root.kind.map(|kind| Project {
                    path: root.path.to_string_lossy().into_owned(),
                    name: names[root.path.as_path()].clone(),
                    kind,
                });
                (cwd.clone(), project)
            })
            .collect()
    }
}

impl FolderRoots {
    fn new(home: Option<PathBuf>, scratch_roots: Vec<PathBuf>) -> Self {
        Self {
            home,
            scratch_roots,
            containers: FxHashMap::default(),
        }
    }

    fn root(&mut self, dir: &Path) -> Option<PathBuf> {
        if self.is_excluded(dir) || self.is_container(dir) {
            return None;
        }
        let mut root = dir;
        while let Some(parent) = root.parent() {
            if self.is_excluded(parent) || self.is_container(parent) {
                break;
            }
            root = parent;
        }
        Some(root.to_path_buf())
    }

    fn is_excluded(&self, dir: &Path) -> bool {
        if dir.parent().is_none()
            || self.scratch_roots.iter().any(|root| dir.starts_with(root))
            || dir.to_string_lossy().contains(CLAUDE_SCRATCH_WORKSPACES)
        {
            return true;
        }
        let Some(in_home) = self
            .home
            .as_deref()
            .and_then(|home| dir.strip_prefix(home).ok())
        else {
            return false;
        };
        in_home.as_os_str().is_empty()
            || HOME_FOLDERS
                .iter()
                .any(|folder| in_home == Path::new(folder))
            || HOME_SCRATCH_ROOTS
                .iter()
                .any(|root| in_home.starts_with(root))
    }

    fn is_container(&mut self, dir: &Path) -> bool {
        if let Some(&cached) = self.containers.get(dir) {
            return cached;
        }
        let mut probes = 0;
        let mut has_repository = |dir: &Path| {
            probes += 1;
            probes <= MAX_CONTAINER_PROBES && dir.join(".git").exists()
        };
        let container = subdirectories(dir).any(|child| {
            has_repository(&child)
                || subdirectories(&child).any(|grandchild| has_repository(&grandchild))
        });
        self.containers.insert(dir.to_path_buf(), container);
        container
    }
}

fn subdirectories(dir: &Path) -> impl Iterator<Item = PathBuf> {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .take(MAX_CONTAINER_PROBES)
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            !name.starts_with('.') && !SKIPPED_SUBFOLDERS.contains(&name.as_ref())
        })
        .map(|entry| entry.path())
}

impl RepositoryCache {
    fn new(home: Option<PathBuf>) -> Self {
        Self {
            home,
            ancestors: FxHashMap::default(),
        }
    }

    fn root(&mut self, dir: &Path) -> Option<Arc<str>> {
        let mut visited = Vec::new();
        let mut found = None;
        for ancestor in dir.ancestors() {
            if ancestor.parent().is_none() || self.home.as_deref() == Some(ancestor) {
                break;
            }
            if let Some(cached) = self.ancestors.get(ancestor) {
                found = cached.clone();
                break;
            }
            visited.push(ancestor.to_path_buf());
            if let Some(root) = git_root_at(ancestor) {
                found = Some(Arc::from(root.to_string_lossy().as_ref()));
                break;
            }
        }
        for ancestor in visited {
            self.ancestors.insert(ancestor, found.clone());
        }
        found
    }
}

fn git_root_at(dir: &Path) -> Option<PathBuf> {
    let dot_git = dir.join(".git");
    let metadata = fs::metadata(&dot_git).ok()?;
    if metadata.is_dir() {
        return Some(dir.to_path_buf());
    }
    Some(linked_worktree_root(dir, &dot_git).unwrap_or_else(|| dir.to_path_buf()))
}

/// Follows a `.git` file to the main repository. Submodules have no
/// `commondir`, so they stay projects of their own.
fn linked_worktree_root(dir: &Path, dot_git: &Path) -> Option<PathBuf> {
    let contents = fs::read_to_string(dot_git).ok()?;
    let git_dir = contents.lines().next()?.strip_prefix("gitdir:")?.trim();
    let git_dir = normalize(&dir.join(git_dir));
    let common_dir = fs::read_to_string(git_dir.join("commondir")).ok()?;
    let common_dir = normalize(&git_dir.join(common_dir.trim()));
    if common_dir.file_name() == Some(OsStr::new(".git")) {
        return common_dir.parent().map(Path::to_path_buf);
    }
    Some(common_dir)
}

/// Returns `<...>/.codex/worktrees/<id>/<name>` when the path lies inside one.
fn codex_worktree(path: &Path) -> Option<PathBuf> {
    let components = path.components().collect::<Vec<_>>();
    let start = components.windows(4).position(|window| {
        window[0].as_os_str() == ".codex"
            && window[1].as_os_str() == "worktrees"
            && matches!(window[3], Component::Normal(_))
    })?;
    Some(components[..start + 4].iter().collect())
}

fn display_names<'a>(roots: impl Iterator<Item = &'a Path>) -> FxHashMap<&'a Path, String> {
    let roots = roots.collect::<BTreeSet<_>>();
    let mut basename_counts = FxHashMap::<&OsStr, usize>::default();
    for root in &roots {
        if let Some(name) = root.file_name() {
            *basename_counts.entry(name).or_default() += 1;
        }
    }
    roots
        .into_iter()
        .map(|root| {
            let name = match root.file_name() {
                Some(name) if basename_counts[name] > 1 => {
                    let parent = root
                        .parent()
                        .and_then(Path::file_name)
                        .map(OsStr::to_string_lossy)
                        .unwrap_or_default();
                    format!("{parent}/{}", name.to_string_lossy())
                }
                Some(name) => name.to_string_lossy().into_owned(),
                None => root.to_string_lossy().into_owned(),
            };
            (root, name)
        })
        .collect()
}

fn normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(
                    normalized.components().next_back(),
                    Some(Component::Normal(_))
                ) {
                    normalized.pop();
                } else if !normalized.has_root() {
                    normalized.push("..");
                }
            }
            other => normalized.push(other),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use agent_burn_test_support::Fixture;

    use super::*;

    fn resolve(home: &Path, cwds: &[&Path]) -> Vec<Option<Project>> {
        // Fixtures live under the system temp folder, which is excluded by
        // default, so tests only exclude a dedicated `tmp` next to `home`.
        let mut resolver = ProjectResolver::new(Some(home.to_path_buf()))
            .with_scratch_roots(vec![home.with_file_name("tmp")]);
        for cwd in cwds {
            resolver.add(&cwd.to_string_lossy());
        }
        let projects = resolver.finish();
        cwds.iter()
            .map(|cwd| projects[cwd.to_string_lossy().as_ref()].clone())
            .collect()
    }

    fn project(path: &Path, name: &str) -> Option<Project> {
        Some(Project {
            path: path.to_string_lossy().into_owned(),
            name: name.to_string(),
            kind: ProjectKind::Repository,
        })
    }

    fn folder(path: &Path, name: &str) -> Option<Project> {
        Some(Project {
            kind: ProjectKind::Folder,
            ..project(path, name).unwrap()
        })
    }

    #[test]
    fn resolves_repository_root_and_its_subfolders() {
        let fixture = Fixture::new();
        let _ = fixture.create_dir_all("home/dev/app/.git");
        let _ = fixture.create_dir_all("home/dev/app/src");
        let repo = fixture.path("home/dev/app");

        let projects = resolve(
            &fixture.path("home"),
            &[&repo, &repo.join("src"), &repo.join("src/deleted/deeper")],
        );

        assert_eq!(projects, vec![project(&repo, "app"); 3]);
    }

    #[test]
    fn normalizes_parent_components_without_touching_the_filesystem() {
        let fixture = Fixture::new();
        let _ = fixture.create_dir_all("home/dev/app/.git");
        let repo = fixture.path("home/dev/app");

        let projects = resolve(&fixture.path("home"), &[&repo.join("gone/../src/./x")]);

        assert_eq!(projects, vec![project(&repo, "app")]);
    }

    #[test]
    fn resolves_linked_worktree_with_relative_gitdir_to_main_repository() {
        let fixture = Fixture::new();
        let _ = fixture.create_dir_all("home/dev/app/.git/worktrees/feature");
        let _ = fixture.write_file("home/dev/app/.git/worktrees/feature/commondir", "../..\n");
        let _ = fixture.write_file(
            "home/dev/app/.claude/worktrees/feature/.git",
            "gitdir: ../../../.git/worktrees/feature\n",
        );
        let repo = fixture.path("home/dev/app");

        let projects = resolve(
            &fixture.path("home"),
            &[&repo.join(".claude/worktrees/feature/src")],
        );

        assert_eq!(projects, vec![project(&repo, "app")]);
    }

    #[test]
    fn resolves_linked_worktree_with_absolute_gitdir_to_main_repository() {
        let fixture = Fixture::new();
        let _ = fixture.create_dir_all("home/dev/app/.git/worktrees/feature");
        let _ = fixture.write_file("home/dev/app/.git/worktrees/feature/commondir", "../..\n");
        let git_dir = fixture.path("home/dev/app/.git/worktrees/feature");
        let _ = fixture.write_file(
            "home/elsewhere/app-feature/.git",
            format!("gitdir: {}\n", git_dir.display()),
        );

        let projects = resolve(
            &fixture.path("home"),
            &[&fixture.path("home/elsewhere/app-feature")],
        );

        assert_eq!(
            projects,
            vec![project(&fixture.path("home/dev/app"), "app")]
        );
    }

    #[test]
    fn keeps_submodule_as_its_own_project() {
        let fixture = Fixture::new();
        let _ = fixture.create_dir_all("home/dev/app/.git/modules/lib");
        let _ = fixture.write_file("home/dev/app/lib/.git", "gitdir: ../.git/modules/lib\n");
        let submodule = fixture.path("home/dev/app/lib");

        let projects = resolve(&fixture.path("home"), &[&submodule.join("src")]);

        assert_eq!(projects, vec![project(&submodule, "lib")]);
    }

    #[test]
    fn leaf_folder_without_git_is_a_folder_project() {
        let fixture = Fixture::new();
        let _ = fixture.create_dir_all("home/dev/app/.git");
        let _ = fixture.create_dir_all("home/dev/tool/src");
        let tool = fixture.path("home/dev/tool");

        let projects = resolve(&fixture.path("home"), &[&tool, &tool.join("src/deleted")]);

        assert_eq!(projects, vec![folder(&tool, "tool"); 2]);
    }

    #[test]
    fn leaf_folder_climbs_to_its_highest_non_container_parent() {
        let fixture = Fixture::new();
        let _ = fixture.create_dir_all("home/dev/app/.git");
        let _ = fixture.create_dir_all("home/dev/workspaces/skool/landing");
        let workspaces = fixture.path("home/dev/workspaces");

        let projects = resolve(
            &fixture.path("home"),
            &[&fixture.path("home/dev/workspaces/skool/landing")],
        );

        assert_eq!(projects, vec![folder(&workspaces, "workspaces")]);
    }

    #[test]
    fn folders_with_repositories_two_levels_down_are_containers() {
        let fixture = Fixture::new();
        let _ = fixture.create_dir_all("home/dev/perso/app/.git");
        let _ = fixture.create_dir_all("home/dev/perso/workspaces/web/.git");

        let projects = resolve(
            &fixture.path("home"),
            &[
                &fixture.path("home/dev/perso"),
                &fixture.path("home/dev/perso/workspaces"),
                &fixture.path("home/dev"),
            ],
        );

        assert_eq!(projects, vec![None, None, None]);
    }

    #[test]
    fn excluded_locations_are_never_folder_projects() {
        let fixture = Fixture::new();
        let _ = fixture.create_dir_all("home/notes");
        let _ = fixture.create_dir_all("tmp/scratch");
        let claude_scratch = "home/Library/Application Support/Claude/scratch-workspaces/a/b";
        let _ = fixture.create_dir_all(claude_scratch);

        let projects = resolve(
            &fixture.path("home"),
            &[
                &fixture.path("home"),
                &fixture.path("tmp/scratch"),
                &fixture.path(claude_scratch),
            ],
        );

        assert_eq!(projects, vec![None, None, None]);
    }

    #[test]
    fn folder_directly_under_home_is_a_folder_project() {
        let fixture = Fixture::new();
        let _ = fixture.create_dir_all("home/notes/today");
        let notes = fixture.path("home/notes");

        let projects = resolve(&fixture.path("home"), &[&notes.join("today")]);

        assert_eq!(projects, vec![folder(&notes, "notes")]);
    }

    #[test]
    fn folders_inside_standard_home_folders_are_projects() {
        let fixture = Fixture::new();
        let _ = fixture.create_dir_all("home/Documents/x/notes");
        let x = fixture.path("home/Documents/x");

        let projects = resolve(&fixture.path("home"), &[&x.join("notes")]);

        assert_eq!(projects, vec![folder(&x, "x")]);
    }

    #[test]
    fn standard_home_folders_and_app_data_are_scratch() {
        let fixture = Fixture::new();
        let codex_task = "home/Documents/Codex/2026-10-01/task";
        let icloud = "home/Library/Mobile Documents/iCloud~md~obsidian/Documents/vault";
        let _ = fixture.create_dir_all(codex_task);
        let _ = fixture.create_dir_all(icloud);
        let _ = fixture.create_dir_all("home/Desktop");

        let projects = resolve(
            &fixture.path("home"),
            &[
                &fixture.path(codex_task),
                &fixture.path(icloud),
                &fixture.path("home/Desktop"),
            ],
        );

        assert_eq!(projects, vec![None, None, None]);
    }

    #[test]
    fn memoizes_container_checks() {
        let fixture = Fixture::new();
        let _ = fixture.create_dir_all("dev/app/.git");
        let mut folders = FolderRoots::new(None, Vec::new());
        let dev = fixture.path("dev");

        assert!(folders.is_container(&dev));
        fs::remove_dir_all(fixture.path("dev/app")).unwrap();

        assert!(folders.is_container(&dev));
    }

    #[test]
    fn home_is_never_a_project_even_when_it_is_a_repository() {
        let fixture = Fixture::new();
        let _ = fixture.create_dir_all("home/.git");
        let _ = fixture.create_dir_all("home/notes");
        let home = fixture.path("home");

        let projects = resolve(&home, &[&home]);

        assert_eq!(projects, vec![None]);
    }

    #[test]
    fn merges_deleted_codex_worktree_into_unique_repository_with_same_name() {
        let fixture = Fixture::new();
        let _ = fixture.create_dir_all("home/dev/app/.git");
        let repo = fixture.path("home/dev/app");
        let worktree = fixture.path("home/.codex/worktrees/fd18/app/src");

        let projects = resolve(&fixture.path("home"), &[&repo, &worktree]);

        assert_eq!(projects, vec![project(&repo, "app"); 2]);
    }

    #[test]
    fn keeps_deleted_codex_worktrees_together_when_repository_name_is_ambiguous() {
        let fixture = Fixture::new();
        let _ = fixture.create_dir_all("home/dev/app/.git");
        let _ = fixture.create_dir_all("home/work/app/.git");
        let first = fixture.path("home/.codex/worktrees/aa11/app");
        let second = fixture.path("home/.codex/worktrees/bb22/app/src");

        let projects = resolve(
            &fixture.path("home"),
            &[
                &second,
                &first,
                &fixture.path("home/dev/app"),
                &fixture.path("home/work/app"),
            ],
        );

        assert_eq!(projects[..2], vec![project(&first, "aa11/app"); 2]);
    }

    #[test]
    fn names_deleted_codex_worktree_after_its_repository_folder() {
        let fixture = Fixture::new();
        let worktree = fixture.path("home/.codex/worktrees/fd18/app");

        let projects = resolve(&fixture.path("home"), &[&worktree.join("src")]);

        assert_eq!(projects, vec![project(&worktree, "app")]);
    }

    #[test]
    fn disambiguates_projects_sharing_a_folder_name_with_their_parent() {
        let fixture = Fixture::new();
        let _ = fixture.create_dir_all("home/dev/app/.git");
        let _ = fixture.create_dir_all("home/work/app/.git");
        let dev = fixture.path("home/dev/app");
        let work = fixture.path("home/work/app");

        let projects = resolve(&fixture.path("home"), &[&dev, &work]);

        assert_eq!(
            projects,
            vec![project(&dev, "dev/app"), project(&work, "work/app")]
        );
    }

    fn repo(fixture: &Fixture, path: &str) -> String {
        let _ = fixture.create_dir_all(format!("{path}/.git"));
        fixture.path(path).to_string_lossy().into_owned()
    }

    fn attributed(turn: (Option<Arc<str>>, Option<Arc<str>>)) -> (Option<String>, Option<String>) {
        (
            turn.0.map(|repo| repo.to_string()),
            turn.1.map(|repo| repo.to_string()),
        )
    }

    #[test]
    fn turn_goes_to_the_most_referenced_repository() {
        let fixture = Fixture::new();
        let app = repo(&fixture, "dev/app");
        let web = repo(&fixture, "dev/web");
        let mut turns = TurnRepositories::default();

        turns.touch(&format!("{web}/a.ts"));
        turns.touch(&format!("{app}/src/a.rs"));
        turns.touch(&format!("{app}/src/b.rs"));

        assert_eq!(
            attributed(turns.finish_turn()),
            (Some(app.clone()), Some(app))
        );
    }

    #[test]
    fn tied_turn_goes_to_the_last_referenced_repository() {
        let fixture = Fixture::new();
        let app = repo(&fixture, "dev/app");
        let web = repo(&fixture, "dev/web");
        let mut turns = TurnRepositories::default();

        turns.touch(&format!("{app}/a.rs"));
        turns.touch(&format!("{web}/a.ts"));

        assert_eq!(
            attributed(turns.finish_turn()),
            (Some(web.clone()), Some(web))
        );
    }

    #[test]
    fn turn_without_touches_inherits_the_previous_repository() {
        let fixture = Fixture::new();
        let app = repo(&fixture, "dev/app");
        let mut turns = TurnRepositories::default();

        assert_eq!(attributed(turns.finish_turn()), (None, None));
        turns.touch(&format!("{app}/a.rs"));
        turns.finish_turn();

        assert_eq!(attributed(turns.finish_turn()), (None, Some(app.clone())));
        assert_eq!(turns.first().map(|repo| repo.to_string()), Some(app));
    }

    #[test]
    fn ignores_paths_outside_repositories() {
        let fixture = Fixture::new();
        let _ = fixture.create_dir_all("tmp/scratch");
        let mut turns = TurnRepositories::default();

        turns.touch(&fixture.path("tmp/scratch/a.txt").to_string_lossy());
        turns.touch("relative/path.rs");

        assert_eq!(attributed(turns.finish_turn()), (None, None));
    }
}
