//! Per-view Herdr configuration: a hidden sidebar at every width, without
//! changing the user's config or sharing mutable chrome state with other views.
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(super) struct Profile {
    root: PathBuf,
    config: PathBuf,
    state: PathBuf,
}

pub(super) fn identity(session: &str) -> (String, String) {
    (
        format!("kanban-local:{}", crate::db::db_path().display()),
        session.into(),
    )
}
pub(super) fn machine_identity(cli: &crate::herdr::Cli) -> (String, String) {
    match &cli.remote {
        Some(machine) => (
            format!(
                "kanban-machine:{}:{}",
                crate::db::db_path().display(),
                machine.id
            ),
            cli.session.clone(),
        ),
        None => identity(&cli.session),
    }
}

fn preference_name(identity: &(String, String)) -> String {
    let mut hash = Sha256::new();
    for part in [&identity.0, &identity.1] {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    format!("remote-{:x}.json", hash.finalize())
}

fn config_source() -> PathBuf {
    if let Some(path) = std::env::var_os("HERDR_CONFIG_PATH") {
        return path.into();
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .or_else(|| std::env::var_os("APPDATA"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("herdr/config.toml")
}

fn state_source() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .or_else(|| std::env::var_os("LOCALAPPDATA"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("herdr/client-shell")
}

fn hidden_config(text: &str, source: &Path) -> Result<String, String> {
    let mut config: toml::Table = text
        .trim_start_matches('\u{feff}')
        .parse()
        .map_err(|e| format!("Herdr config: {e}"))?;
    let ui = config
        .entry("ui")
        .or_insert_with(|| toml::Value::Table(Default::default()))
        .as_table_mut()
        .ok_or("Herdr [ui] must be a table")?;
    ui.insert("sidebar_start_collapsed".into(), true.into());
    ui.insert("sidebar_collapsed_mode".into(), "hidden".into());
    // Herdr resolves these three paths relative to HERDR_CONFIG_PATH. Preserve
    // their meaning when placing the generated config in Kanban's data folder.
    if let Some(sound) = ui.get_mut("sound").and_then(toml::Value::as_table_mut) {
        for key in ["path", "done_path", "request_path"] {
            if let Some(value) = sound.get_mut(key) {
                if let Some(path) = value
                    .as_str()
                    .map(PathBuf::from)
                    .filter(|p| p.is_relative())
                {
                    *value = source
                        .parent()
                        .unwrap_or(Path::new("."))
                        .join(path)
                        .to_string_lossy()
                        .into_owned()
                        .into();
                }
            }
        }
    }
    toml::to_string(&config).map_err(|e| e.to_string())
}

impl Profile {
    #[cfg(test)]
    pub(super) fn fixture(base: &Path, session: &str) -> Self {
        Self::create(
            base,
            &base.join("missing.toml"),
            &base.join("missing-state"),
            &identity(session),
        )
        .unwrap()
    }

    pub(super) fn prepare(base: &Path, session: &str) -> Result<Self, String> {
        Self::create(base, &config_source(), &state_source(), &identity(session))
    }
    pub(super) fn prepare_machine(base: &Path, cli: &crate::herdr::Cli) -> Result<Self, String> {
        Self::create(
            base,
            &config_source(),
            &state_source(),
            &machine_identity(cli),
        )
    }

    fn create(
        base: &Path,
        source: &Path,
        old_state: &Path,
        identity: &(String, String),
    ) -> Result<Self, String> {
        let source = std::path::absolute(source).map_err(|e| e.to_string())?;
        let text = match fs::read_to_string(&source) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("Herdr config: {e}")),
        };
        let text = hidden_config(&text, &source)?;
        let root = std::path::absolute(base)
            .map_err(|e| e.to_string())?
            .join(format!("client-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let profile = Self {
            config: root.join("config.toml"),
            state: root.join("state"),
            root,
        };
        fs::write(&profile.config, text).map_err(|e| e.to_string())?;
        // Clone other known/unknown preferences, but never inherit an expanded
        // sidebar. Per-view storage also prevents a late old client write from
        // changing a new client's startup state during rapid task switching.
        let name = preference_name(identity);
        let mut prefs = fs::read(old_state.join(&name))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        prefs.insert("sidebar_collapsed".into(), true.into());
        let directory = profile.state.join("herdr/client-shell");
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        fs::write(
            directory.join(name),
            serde_json::to_vec(&prefs).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        Ok(profile)
    }

    pub(super) fn apply(&self, command: &mut Command) {
        command
            .env("HERDR_CONFIG_PATH", &self.config)
            .env("XDG_STATE_HOME", &self.state);
    }
}

impl Drop for Profile {
    fn drop(&mut self) {
        // root is exclusively created above, never an inherited/user path.
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_hidden_sidebar_without_losing_user_settings_or_relative_sounds() {
        let source = std::path::absolute("user/config.toml").unwrap();
        let result = hidden_config("\u{feff}onboarding = false\n[theme]\nname = 'terminal'\n[ui]\nsidebar_start_collapsed = false\nsidebar_collapsed_mode = 'compact'\n[ui.sound]\ndone_path = 'done.mp3'\n", &source).unwrap();
        let doc: toml::Table = result.parse().unwrap();
        assert_eq!(doc["ui"]["sidebar_start_collapsed"].as_bool(), Some(true));
        assert_eq!(doc["ui"]["sidebar_collapsed_mode"].as_str(), Some("hidden"));
        assert_eq!(doc["theme"]["name"].as_str(), Some("terminal"));
        assert_eq!(doc["onboarding"].as_bool(), Some(false));
        assert_eq!(
            doc["ui"]["sound"]["done_path"].as_str(),
            source.parent().unwrap().join("done.mp3").to_str()
        );
        assert!(hidden_config("ui = 4", &source).is_err());
        assert!(hidden_config("[broken", &source).is_err());
    }

    #[test]
    fn expanded_preferences_and_old_client_writes_cannot_reopen_new_view_sidebar() {
        let base = std::env::temp_dir().join(format!("kanban-sidebar-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&base).unwrap();
        let source = base.join("original.toml");
        let original = "[ui]\nsidebar_start_collapsed = false\n";
        fs::write(&source, original).unwrap();
        let id = ("kanban-test".into(), "test".into());
        let name = preference_name(&id);
        let original_prefs =
            r#"{"sidebar_collapsed":false,"sidebar_width":31,"collapsed_groups":["/repo"]}"#;
        fs::write(base.join(&name), original_prefs).unwrap();
        let first = Profile::create(&base, &source, &base, &id).unwrap();
        let first_path = first.state.join("herdr/client-shell").join(&name);
        fs::write(first_path, original_prefs).unwrap(); // manual expansion in old window
        let second = Profile::create(&base, &source, &base, &id).unwrap();
        let prefs: serde_json::Value = serde_json::from_slice(
            &fs::read(second.state.join("herdr/client-shell").join(&name)).unwrap(),
        )
        .unwrap();
        assert_eq!(prefs["sidebar_collapsed"], true);
        assert_eq!(prefs["sidebar_width"], 31);
        assert_eq!(prefs["collapsed_groups"][0], "/repo");
        assert_eq!(fs::read_to_string(&source).unwrap(), original);
        assert_eq!(fs::read_to_string(base.join(name)).unwrap(), original_prefs);
        let first_root = first.root.clone();
        drop(first);
        assert!(!first_root.exists());
        assert!(second.config.exists());
        drop(second);
        fs::remove_dir_all(base).unwrap();
    }
}
