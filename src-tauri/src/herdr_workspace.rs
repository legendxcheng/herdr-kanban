//! Read-only workspace directory metadata, scoped to the configured session.
use super::Cli;
use serde_json::Value;
use std::{io::Read, path::PathBuf};

impl Cli {
    pub(super) fn session_file(&self) -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("KANBAN_HERDR_SESSION_FILE") {
            return Some(path.into());
        }
        // Never mix another session's IDs with default's persisted directory metadata.
        if self.session != "default" || std::env::var_os("HERDR_CONFIG_PATH").is_some() {
            return None;
        }
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("herdr/session.json"))
    }
}

pub(super) fn workspace_directories(
    path: Option<&std::path::Path>,
) -> Option<std::collections::HashMap<String, String>> {
    let file = std::fs::File::open(path?).ok()?;
    if file.metadata().ok()?.len() > 8 * 1024 * 1024 {
        return None;
    }
    let doc: Value = serde_json::from_reader(file.take(8 * 1024 * 1024)).ok()?;
    if doc.get("version")?.as_u64()? != 3 {
        return None;
    }
    let mut result = std::collections::HashMap::new();
    let mut ids = std::collections::HashSet::new();
    for workspace in doc.get("workspaces")?.as_array()? {
        let id = workspace.get("id")?.as_str()?;
        if !ids.insert(id) {
            return None;
        }
        if let Some(cwd) = workspace
            .get("identity_cwd")
            .and_then(Value::as_str)
            .filter(|d| std::path::Path::new(d).is_dir())
        {
            result.insert(id.into(), cwd.into());
        }
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    #[test]
    fn workspace_directory_metadata_is_versioned_and_never_uses_pane_cwd() {
        let path =
            std::env::temp_dir().join(format!("kanban-directories-{}.json", uuid::Uuid::new_v4()));
        let cwd = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let data = serde_json::json!({"version":3,"workspaces":[
            {"id":"w1","identity_cwd":cwd},
            {"id":"w2","panes":[{"cwd":cwd}]},
            {"id":"w3","identity_cwd":"Z:/kanban-nonexistent-directory"}
        ]});
        std::fs::write(&path, data.to_string()).unwrap();
        let dirs = super::workspace_directories(Some(&path)).unwrap();
        assert_eq!(dirs.len(), 1);
        assert_eq!(dirs["w1"], cwd);
        let mut incompatible = data.clone();
        incompatible["version"] = 4.into();
        std::fs::write(&path, incompatible.to_string()).unwrap();
        assert!(super::workspace_directories(Some(&path)).is_none());
        std::fs::write(&path, "{broken").unwrap();
        assert!(super::workspace_directories(Some(&path)).is_none());
        std::fs::remove_file(&path).unwrap();
        assert!(super::workspace_directories(Some(&path)).is_none());
        assert!(super::workspace_directories(None).is_none());
    }
}
