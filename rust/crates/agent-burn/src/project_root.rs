//! Resolves session working directories to the git project that owns them.
//!
//! Paths are normalized lexically and checked ancestor by ancestor, so sessions
//! whose folders were deleted since still land on the repository above them.
//! Linked worktrees resolve to their main repository through `commondir`.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fs,
    path::{Component, Path, PathBuf},
};

use crate::fast::FxHashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Project {
    pub(crate) path: String,
    pub(crate) name: String,
}

pub(crate) struct ProjectResolver {
    home: Option<PathBuf>,
    ancestors: FxHashMap<PathBuf, Option<PathBuf>>,
    roots: FxHashMap<String, Root>,
}

struct Root {
    path: PathBuf,
    from_git: bool,
}

impl ProjectResolver {
    pub(crate) fn new(home: Option<PathBuf>) -> Self {
        Self {
            home,
            ancestors: FxHashMap::default(),
            roots: FxHashMap::default(),
        }
    }

    pub(crate) fn add(&mut self, cwd: &str) {
        if self.roots.contains_key(cwd) {
            return;
        }
        let path = normalize(Path::new(cwd));
        let root = match self.git_root(&path) {
            Some(path) => Root {
                path,
                from_git: true,
            },
            None => Root {
                path,
                from_git: false,
            },
        };
        self.roots.insert(cwd.to_owned(), root);
    }

    /// Maps every added cwd to its project, merging Codex worktrees that no
    /// longer exist into the repository they were created from.
    pub(crate) fn finish(mut self) -> FxHashMap<String, Project> {
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
            }
        }

        let names = display_names(
            self.roots.values().map(|root| root.path.as_path()),
            self.home.as_deref(),
        );
        self.roots
            .iter()
            .map(|(cwd, root)| {
                let project = Project {
                    path: root.path.to_string_lossy().into_owned(),
                    name: names[root.path.as_path()].clone(),
                };
                (cwd.clone(), project)
            })
            .collect()
    }

    fn git_root(&mut self, dir: &Path) -> Option<PathBuf> {
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
                found = Some(root);
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

fn display_names<'a>(
    roots: impl Iterator<Item = &'a Path>,
    home: Option<&Path>,
) -> FxHashMap<&'a Path, String> {
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
                _ if Some(root) == home => "~".to_string(),
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

    fn resolve(home: &Path, cwds: &[&Path]) -> Vec<Project> {
        let mut resolver = ProjectResolver::new(Some(home.to_path_buf()));
        for cwd in cwds {
            resolver.add(&cwd.to_string_lossy());
        }
        let projects = resolver.finish();
        cwds.iter()
            .map(|cwd| projects[cwd.to_string_lossy().as_ref()].clone())
            .collect()
    }

    fn project(path: &Path, name: &str) -> Project {
        Project {
            path: path.to_string_lossy().into_owned(),
            name: name.to_string(),
        }
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
    fn keeps_folder_outside_any_repository() {
        let fixture = Fixture::new();
        let _ = fixture.create_dir_all("home/notes/today");
        let folder = fixture.path("home/notes/today");

        let projects = resolve(&fixture.path("home"), &[&folder]);

        assert_eq!(projects, vec![project(&folder, "today")]);
    }

    #[test]
    fn stops_before_home_even_when_home_is_a_repository() {
        let fixture = Fixture::new();
        let _ = fixture.create_dir_all("home/.git");
        let _ = fixture.create_dir_all("home/notes");
        let home = fixture.path("home");

        let projects = resolve(&home, &[&home.join("notes"), &home]);

        assert_eq!(
            projects,
            vec![project(&home.join("notes"), "notes"), project(&home, "~")]
        );
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
}
