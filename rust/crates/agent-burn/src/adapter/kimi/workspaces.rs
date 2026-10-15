use std::{collections::HashMap, fs, path::Path, sync::Arc};

use serde::Deserialize;

use super::paths::{KIMI_SESSIONS_DIR_NAME, WireFile};

const WORKSPACES_FILE_NAME: &str = "workspaces.json";

/// Kimi Code's `workspaces.json`, which maps the `{workDirKey}` session folder
/// to the directory the sessions ran in.
#[derive(Debug, Default)]
pub(super) struct KimiWorkspaces {
    roots: HashMap<String, Arc<str>>,
}

#[derive(Default, Deserialize)]
struct WorkspacesFile {
    #[serde(default)]
    workspaces: HashMap<String, Workspace>,
}

#[derive(Deserialize)]
struct Workspace {
    root: Option<String>,
}

impl KimiWorkspaces {
    pub(super) fn load(root: &Path) -> Self {
        let file = fs::read(root.join(WORKSPACES_FILE_NAME))
            .ok()
            .and_then(|content| serde_json::from_slice::<WorkspacesFile>(&content).ok())
            .unwrap_or_default();
        let roots = file
            .workspaces
            .into_iter()
            .filter_map(|(key, workspace)| Some((key, Arc::from(workspace.root?))))
            .collect();
        Self { roots }
    }

    pub(super) fn project_path(&self, file: &WireFile) -> Option<Arc<str>> {
        let relative = file
            .path
            .strip_prefix(file.root.join(KIMI_SESSIONS_DIR_NAME))
            .ok()?;
        let key = relative.components().next()?.as_os_str().to_str()?;
        self.roots.get(key).cloned()
    }
}
