use std::{
    collections::HashSet,
    env,
    path::{Component, Path, PathBuf},
};

use crate::{Result, collect_files_with_extension};

pub(super) const KIMI_DATA_DIR_ENV: &str = "KIMI_DATA_DIR";
pub(super) const KIMI_CODE_HOME_ENV: &str = "KIMI_CODE_HOME";
pub(super) const KIMI_SESSIONS_DIR_NAME: &str = "sessions";
pub(super) const KIMI_WIRE_FILE_NAME: &str = "wire.jsonl";
const LEGACY_DIR_NAME: &str = ".kimi";
const KIMI_CODE_DIR_NAME: &str = ".kimi-code";
const KIMI_CODE_AGENTS_DIR_NAME: &str = "agents";

/// Which Kimi CLI wrote a `wire.jsonl`, told apart by its depth under `sessions/`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WireLayout {
    /// Legacy Python kimi-cli: `sessions/<group>/<session>/wire.jsonl`.
    Legacy,
    /// Kimi Code: `sessions/<workDirKey>/<session>/agents/<agentId>/wire.jsonl`.
    KimiCode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct WireFile {
    pub(super) path: PathBuf,
    /// Data root the file was found under (holds `config.toml` / `config.json`).
    pub(super) root: PathBuf,
    pub(super) layout: WireLayout,
}

/// Data roots to scan. `KIMI_DATA_DIR` (comma-separated) replaces the defaults
/// and each listed root may hold either layout; otherwise the legacy `~/.kimi`
/// and the Kimi Code home (`KIMI_CODE_HOME`, else `~/.kimi-code`) are read.
pub(super) fn paths() -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    if let Ok(env_paths) = env::var(KIMI_DATA_DIR_ENV) {
        for raw in env_paths
            .split(',')
            .map(str::trim)
            .filter(|path| !path.is_empty())
        {
            let path = PathBuf::from(raw);
            if path.is_dir() && seen.insert(path.clone()) {
                paths.push(path);
            }
        }
        return Ok(paths);
    }

    let legacy = crate::home::home_dir().map(|home| home.join(LEGACY_DIR_NAME));
    for path in [legacy, kimi_code_home()].into_iter().flatten() {
        if path.is_dir() && seen.insert(path.clone()) {
            paths.push(path);
        }
    }
    Ok(paths)
}

/// Kimi Code's data root: `KIMI_CODE_HOME` when set and non-empty, else `~/.kimi-code`.
pub(super) fn kimi_code_home() -> Option<PathBuf> {
    match env::var(KIMI_CODE_HOME_ENV) {
        Ok(home) if !home.trim().is_empty() => Some(PathBuf::from(home)),
        _ => crate::home::home_dir().map(|home| home.join(KIMI_CODE_DIR_NAME)),
    }
}

pub(super) fn discover_wire_files() -> Result<Vec<WireFile>> {
    let mut files = Vec::new();
    for root in paths()? {
        let sessions_path = root.join(KIMI_SESSIONS_DIR_NAME);
        let mut candidates = Vec::new();
        collect_files_with_extension(&sessions_path, "jsonl", &mut candidates);
        files.extend(candidates.into_iter().filter_map(|path| {
            let layout = wire_layout(&sessions_path, &path)?;
            Some(WireFile {
                path,
                root: root.clone(),
                layout,
            })
        }));
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    files.dedup_by(|left, right| left.path == right.path);
    Ok(files)
}

fn wire_layout(sessions_path: &Path, file_path: &Path) -> Option<WireLayout> {
    if file_path.file_name().and_then(|name| name.to_str()) != Some(KIMI_WIRE_FILE_NAME) {
        return None;
    }
    let relative = file_path.strip_prefix(sessions_path).ok()?;
    let parts = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part),
            _ => None,
        })
        .collect::<Vec<_>>();
    match parts.as_slice() {
        [_, _, _] => Some(WireLayout::Legacy),
        [_, _, agents, _, _] if *agents == KIMI_CODE_AGENTS_DIR_NAME => Some(WireLayout::KimiCode),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_burn_test_support::{EnvVarGuard, fs_fixture};

    #[test]
    fn discovers_wire_jsonl_files_under_sessions_group_session() {
        let fixture = fs_fixture!({
            "sessions/group/session/wire.jsonl": "{}\n",
            "sessions/group/session/other.jsonl": "{}\n",
            "sessions/nested/path/session/wire.jsonl": "{}\n",
        });
        let _cleanup = EnvVarGuard::set(KIMI_DATA_DIR_ENV, fixture.root());
        let files = discover_wire_files().unwrap();

        assert_eq!(
            files,
            vec![WireFile {
                path: fixture.path("sessions/group/session/wire.jsonl"),
                root: fixture.root().to_path_buf(),
                layout: WireLayout::Legacy,
            }]
        );
    }

    #[test]
    fn discovers_kimi_code_main_and_sub_agent_wire_files() {
        let fixture = fs_fixture!({
            "sessions/wd/session_a/agents/main/wire.jsonl": "{}\n",
            "sessions/wd/session_a/agents/agent-0/wire.jsonl": "{}\n",
            "sessions/wd/session_a/agents/main/tasks/wire.jsonl": "{}\n",
            "sessions/wd/session_a/subagents/agent-0/wire.jsonl": "{}\n",
            "sessions/wd/session_a/state.json": "{}\n",
        });
        let _cleanup = EnvVarGuard::set(KIMI_DATA_DIR_ENV, fixture.root());
        let files = discover_wire_files().unwrap();

        assert_eq!(
            files
                .iter()
                .map(|file| (file.path.clone(), file.layout))
                .collect::<Vec<_>>(),
            vec![
                (
                    fixture.path("sessions/wd/session_a/agents/agent-0/wire.jsonl"),
                    WireLayout::KimiCode
                ),
                (
                    fixture.path("sessions/wd/session_a/agents/main/wire.jsonl"),
                    WireLayout::KimiCode
                ),
            ]
        );
    }
}
