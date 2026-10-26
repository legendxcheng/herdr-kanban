//! Resolve the user's Herdr entry point, including portable Windows launchers.
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

pub(super) fn from_env() -> PathBuf {
    resolve(
        std::env::var_os("KANBAN_HERDR_EXECUTABLE").as_deref(),
        std::env::var_os("PATH").as_deref(),
        std::env::var_os("PATHEXT").as_deref(),
        std::env::var_os("LOCALAPPDATA").as_deref(),
    )
}

fn resolve(
    explicit: Option<&OsStr>,
    path: Option<&OsStr>,
    pathext: Option<&OsStr>,
    local_app_data: Option<&OsStr>,
) -> PathBuf {
    if let Some(explicit) = explicit.filter(|value| !value.is_empty()) {
        let executable = PathBuf::from(explicit);
        // Resolve bare override names too: CreateProcess does not search for
        // .cmd/.bat the way an interactive Windows shell does.
        return if executable.components().count() == 1 {
            on_path(&executable, path, pathext).unwrap_or(executable)
        } else {
            executable
        };
    }
    let installed = local_app_data
        .map(PathBuf::from)
        .map(|p| p.join("Programs/Herdr/bin/herdr.exe"));
    on_path(Path::new("herdr"), path, pathext)
        .or_else(|| installed.filter(|p| p.is_file()))
        .unwrap_or_else(|| "herdr".into())
}

fn on_path(name: &Path, path: Option<&OsStr>, pathext: Option<&OsStr>) -> Option<PathBuf> {
    #[cfg(windows)]
    let extensions: Vec<String> = if name.extension().is_some() {
        vec![String::new()]
    } else {
        pathext
            .unwrap_or(OsStr::new(".COM;.EXE;.BAT;.CMD"))
            .to_string_lossy()
            .split(';')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| matches!(s.as_str(), ".com" | ".exe" | ".bat" | ".cmd"))
            .collect()
    };
    #[cfg(not(windows))]
    let extensions = {
        let _ = pathext;
        vec![String::new()]
    };
    for directory in std::env::split_paths(path?) {
        if directory.as_os_str().is_empty() {
            continue;
        }
        for extension in &extensions {
            let mut filename = name.as_os_str().to_owned();
            filename.push(extension);
            let candidate = directory.join(filename);
            if candidate.is_file() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if candidate.metadata().ok()?.permissions().mode() & 0o111 == 0 {
                        continue;
                    }
                }
                return std::path::absolute(candidate).ok();
            }
        }
    }
    None
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::herdr::Cli;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("kanban herdr entry {}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(root.join("bin")).unwrap();
            std::fs::create_dir_all(root.join("Programs/Herdr/bin")).unwrap();
            Self(root)
        }
        fn resolve(&self, explicit: Option<&OsStr>) -> PathBuf {
            resolve(
                explicit,
                Some(self.0.join("bin").as_os_str()),
                Some(OsStr::new(".COM;.EXE;.BAT;.CMD")),
                Some(self.0.as_os_str()),
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    #[ignore = "requires an explicitly prepared isolated portable Herdr server and launcher on PATH"]
    fn live_portable_entry_keeps_the_real_server_and_project_directory() {
        use crate::workflow::Herdr;
        let launcher = PathBuf::from(
            std::env::var_os("KANBAN_ACCEPTANCE_HERDR_LAUNCHER").expect("explicit launcher"),
        );
        let endpoint = PathBuf::from(
            std::env::var_os("KANBAN_ACCEPTANCE_HERDR_SOCKET").expect("explicit socket"),
        );
        let directory =
            std::env::var("KANBAN_ACCEPTANCE_HERDR_PROJECT").expect("explicit project directory");
        let cli = Cli::from_env();
        assert_eq!(cli.executable, launcher);
        assert_eq!(cli.api_endpoint().unwrap(), endpoint);
        let workspaces = cli.workspaces().unwrap();
        let workspace = workspaces
            .iter()
            .find(|w| w.name == "kanban-acceptance-portable-entry")
            .expect("prepared workspace");
        assert_eq!(
            std::fs::canonicalize(workspace.directory.as_ref().expect("project directory"))
                .unwrap(),
            std::fs::canonicalize(&directory).unwrap()
        );
        assert_eq!(
            cli.client_endpoint().unwrap(),
            endpoint.with_file_name("herdr-client.sock")
        );
        for label in [
            "portable multiline\nliteral & %PATH% ! ^ \"quoted\"",
            "portable literal & %PATH% ! ^ \"quoted\"",
        ] {
            let created = cli
                .request(&[
                    "workspace",
                    "create",
                    "--label",
                    label,
                    "--cwd",
                    &directory,
                    "--no-focus",
                ])
                .map_err(|e| e.message)
                .expect("arguments must remain literal through a batch entry point");
            assert_eq!(created["result"]["workspace"]["label"], label);
        }
        println!(
            "portable launcher, live API, client endpoint, project directory and literal arguments verified"
        );
    }

    #[test]
    fn path_launcher_wins_over_installed_copy_and_keeps_its_connection_configuration() {
        let fixture = Fixture::new();
        let launcher = fixture.0.join("bin/herdr.cmd");
        // The wrapper establishes its own socket; the caller must execute it,
        // rather than discover an unrelated exe beside it or in LocalAppData.
        std::fs::write(&launcher, concat!(
            "@echo off\r\nsetlocal\r\n",
            "set \"HERDR_SOCKET_PATH=D:\\portable-herdr\\herdr.sock\"\r\n",
            "if \"%~3\"==\"status\" (\r\n",
            "echo status: running\r\necho socket: %HERDR_SOCKET_PATH%\r\nexit /b 0\r\n)\r\n",
            "if \"%~3\"==\"workspace\" (\r\n",
            "echo {\"result\":{\"type\":\"workspace_list\",\"workspaces\":[{\"workspace_id\":\"w-portable\",\"label\":\"portable\"}]}}\r\nexit /b 0\r\n)\r\n",
            "exit /b 3\r\n"
        )).unwrap();
        std::fs::write(
            fixture.0.join("Programs/Herdr/bin/herdr.exe"),
            b"other install",
        )
        .unwrap();
        let executable = fixture.resolve(None);
        assert_eq!(
            executable, launcher,
            "PATH must not be shadowed by the installed copy"
        );
        let cli = Cli {
            executable,
            session: "default".into(),
            remote: None,
        };
        assert_eq!(
            cli.client_endpoint().unwrap(),
            PathBuf::from(r"D:\portable-herdr\herdr-client.sock")
        );
        let spaces = cli
            .request(&["workspace", "list"])
            .map_err(|e| e.message)
            .unwrap();
        assert_eq!(
            spaces["result"]["workspaces"][0]["workspace_id"],
            "w-portable"
        );
    }

    #[test]
    fn explicit_selection_is_preserved_and_installed_copy_is_only_a_fallback() {
        let fixture = Fixture::new();
        let installed = fixture.0.join("Programs/Herdr/bin/herdr.exe");
        std::fs::write(&installed, b"installed").unwrap();
        assert_eq!(fixture.resolve(None), installed);
        let launcher = fixture.0.join("bin/herdr.cmd");
        std::fs::write(&launcher, b"@echo off").unwrap();
        assert_eq!(fixture.resolve(Some(installed.as_os_str())), installed);
        assert_eq!(fixture.resolve(Some(OsStr::new("herdr"))), launcher);
        assert_eq!(fixture.resolve(Some(OsStr::new("herdr.cmd"))), launcher);
        let missing = fixture.0.join("missing/herdr.exe");
        assert_eq!(fixture.resolve(Some(missing.as_os_str())), missing);
        std::fs::remove_file(&installed).unwrap();
        std::fs::remove_file(&launcher).unwrap();
        assert_eq!(fixture.resolve(None), PathBuf::from("herdr"));
    }

    #[test]
    fn multiline_api_arguments_do_not_send_when_launcher_discovery_is_invalid_or_offline() {
        let fixture = Fixture::new();
        let launcher = fixture.0.join("bin/herdr.cmd");
        let native = std::env::current_exe().unwrap();
        let good = serde_json::json!({"client":{"binary":native},"server":{"running":true,"socket":"test-socket"}});
        for (pointer, value, expected) in [
            (
                "/client/binary",
                serde_json::json!("relative.exe"),
                "herdr.invalid_response",
            ),
            (
                "/client/binary",
                serde_json::json!(launcher),
                "herdr.invalid_response",
            ),
            (
                "/server/socket",
                serde_json::json!(""),
                "herdr.invalid_response",
            ),
            ("/server/running", serde_json::json!(false), "herdr.failed"),
        ] {
            let mut status = good.clone();
            *status.pointer_mut(pointer).unwrap() = value;
            std::fs::write(&launcher, format!("@echo off\r\necho {status}\r\n")).unwrap();
            let cli = Cli {
                executable: launcher.clone(),
                session: "default".into(),
                remote: None,
            };
            let error = cli
                .request(&["pane", "send-text", "w1:p1", "first\r\nsecond"])
                .unwrap_err();
            assert_eq!(error.message, expected);
            assert!(!error.uncertain);
        }
        std::fs::write(&launcher, "@echo off\r\nexit /b 3\r\n").unwrap();
        let cli = Cli {
            executable: launcher,
            session: "default".into(),
            remote: None,
        };
        let error = cli
            .request(&["agent", "prompt", "w1:p1", "first\nsecond"])
            .unwrap_err();
        assert!(error.message.starts_with("herdr.failed"));
        assert!(!error.uncertain);
    }

    #[test]
    fn workspace_directory_comes_from_the_selected_launcher_session() {
        use crate::workflow::Herdr;
        let fixture = Fixture::new();
        let directory = fixture.0.join("project");
        let state = fixture.0.join("custom-state");
        std::fs::create_dir(&directory).unwrap();
        std::fs::create_dir(&state).unwrap();
        let socket = fixture.0.join("custom-api/herdr.sock");
        let sessions = serde_json::json!({"sessions":[{
            "name":"default", "session_dir":state, "socket_path":socket
        }]});
        let workspaces = serde_json::json!({"result":{"type":"workspace_list","workspaces":[{
            "workspace_id":"w-portable", "label":"portable"
        }]}});
        std::fs::write(
            state.join("session.json"),
            serde_json::json!({"version":3,"workspaces":[{
                "id":"w-portable", "identity_cwd":directory
            }]})
            .to_string(),
        )
        .unwrap();
        let launcher = fixture.0.join("bin/herdr.cmd");
        std::fs::write(&launcher, format!(
            "@echo off\r\nif \"%~3\"==\"status\" (\r\necho status: running\r\necho socket: {}\r\nexit /b 0\r\n)\r\nif \"%~3\"==\"session\" (\r\necho {sessions}\r\nexit /b 0\r\n)\r\nif \"%~3\"==\"workspace\" (\r\necho {workspaces}\r\nexit /b 0\r\n)\r\nexit /b 3\r\n",
            socket.display()
        )).unwrap();
        let cli = Cli {
            executable: fixture.resolve(None),
            session: "default".into(),
            remote: None,
        };
        let workspaces = cli.workspaces().unwrap();
        assert_eq!(workspaces[0].directory.as_deref(), directory.to_str());
    }

    #[test]
    fn discovery_obeys_path_and_pathext_order_without_searching_current_directory() {
        let fixture = Fixture::new();
        let first = fixture.0.join("bin");
        let second = fixture.0.join("second");
        std::fs::create_dir(&second).unwrap();
        for name in ["herdr.cmd", "herdr.exe", "herdr.bat"] {
            std::fs::write(first.join(name), b"entry").unwrap();
        }
        std::fs::write(second.join("herdr.exe"), b"entry").unwrap();
        let path = std::env::join_paths([&first, &second]).unwrap();
        assert_eq!(
            resolve(None, Some(&path), Some(OsStr::new(".CMD;.EXE")), None),
            first.join("herdr.cmd")
        );
        assert_eq!(
            resolve(None, Some(&path), Some(OsStr::new(".EXE;.CMD")), None),
            first.join("herdr.exe")
        );
        assert_eq!(
            resolve(None, Some(&path), Some(OsStr::new(".BAT;.EXE")), None),
            first.join("herdr.bat")
        );
        let path = std::env::join_paths([&second, &first]).unwrap();
        assert_eq!(
            resolve(None, Some(&path), None, None),
            second.join("herdr.exe")
        );
        assert_eq!(
            resolve(None, Some(OsStr::new("")), None, None),
            PathBuf::from("herdr")
        );
    }
}
