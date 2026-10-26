//! Read-only workspace directory metadata, scoped to the configured session.
use super::Cli;
use serde_json::Value;
use std::{io::Read, path::PathBuf};

impl Cli {
    pub(super) fn session_file(&self) -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("KANBAN_HERDR_SESSION_FILE") {
            return Some(path.into());
        }
        // A portable launcher may set its own configuration in the child process.
        // Ask that same launcher for the session directory, then match both name
        // and endpoint before reading metadata. Never guess using our APPDATA or
        // the socket's parent: either may belong to a different installation.
        if self.remote.is_some() {
            return None;
        }
        let endpoint = self.api_endpoint().ok()?;
        let sessions = self.json(&["session", "list", "--json"]).ok()?;
        session_file(&sessions, &self.session, &endpoint)
    }
}

fn session_file(doc: &Value, session: &str, endpoint: &std::path::Path) -> Option<PathBuf> {
    let mut matches = doc
        .get("sessions")?
        .as_array()?
        .iter()
        .filter(|entry| entry.get("name").and_then(Value::as_str) == Some(session));
    let entry = matches.next()?;
    if matches.next().is_some()
        || std::path::Path::new(entry.get("socket_path")?.as_str()?) != endpoint
    {
        return None;
    }
    let directory = PathBuf::from(entry.get("session_dir")?.as_str()?);
    directory
        .is_absolute()
        .then(|| directory.join("session.json"))
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
    fn directory_discovery_requires_a_unique_session_and_matching_endpoint() {
        let directory = std::env::temp_dir().join("herdr-configured-state");
        let endpoint = directory.join("api-elsewhere/herdr.sock");
        let entry =
            serde_json::json!({"name":"custom", "session_dir":directory, "socket_path":endpoint});
        let mut doc = serde_json::json!({"sessions":[entry.clone()]});
        assert_eq!(
            super::session_file(&doc, "custom", &endpoint),
            Some(directory.join("session.json"))
        );
        assert!(super::session_file(&doc, "default", &endpoint).is_none());
        assert!(super::session_file(&doc, "custom", &directory.join("other.sock")).is_none());
        doc["sessions"][0]["session_dir"] = "relative-state".into();
        assert!(super::session_file(&doc, "custom", &endpoint).is_none());
        doc["sessions"] = serde_json::json!([entry.clone(), entry]);
        assert!(super::session_file(&doc, "custom", &endpoint).is_none());
        assert!(super::session_file(&serde_json::json!({}), "custom", &endpoint).is_none());
    }
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
