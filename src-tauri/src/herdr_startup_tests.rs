use super::*;

#[test]
fn default_session_accepts_explicit_null_or_default_on_both_sides() {
    for client in [Value::Null, serde_json::json!("default")] {
        for server in [Value::Null, serde_json::json!("default")] {
            for running in [true, false] {
                let status = serde_json::json!({
                    "client": {"session": client},
                    "server": {"session": server, "running": running,
                        "status": if running { "running" } else { "not_running" },
                        "compatible": true, "endpoint_compatible": true}
                });
                assert_eq!(
                    status_running(&serde_json::to_vec(&status).unwrap(), "default"),
                    Ok(running)
                );
            }
        }
    }
}

#[test]
fn default_session_alias_does_not_accept_missing_invalid_or_other_session_identity() {
    for selected in ["default", "custom"] {
        for side in ["client", "server"] {
            for invalid in [
                None,
                Some(serde_json::json!("other")),
                Some(serde_json::json!("")),
                Some(serde_json::json!(false)),
                Some(serde_json::json!(0)),
                Some(serde_json::json!({})),
            ] {
                let mut status = serde_json::json!({
                    "client": {"session": selected},
                    "server": {"session": selected, "running": true}
                });
                match invalid {
                    Some(value) => {
                        status[side]["session"] = value;
                    }
                    None => {
                        status[side].as_object_mut().unwrap().remove("session");
                    }
                }
                assert_eq!(
                    status_running(&serde_json::to_vec(&status).unwrap(), selected),
                    Err("herdr.invalid_response".into())
                );
            }
        }
    }
    for side in ["client", "server"] {
        let mut status = serde_json::json!({"client":{"session":"custom"},"server":{"session":"custom","running":true}});
        status[side]["session"] = Value::Null;
        assert_eq!(
            status_running(&serde_json::to_vec(&status).unwrap(), "custom"),
            Err("herdr.invalid_response".into())
        );
    }
}

#[test]
fn default_session_null_keeps_running_and_compatibility_guards() {
    for key in ["compatible", "endpoint_compatible"] {
        let mut status =
            serde_json::json!({"client":{"session":null},"server":{"session":null,"running":true}});
        status["server"][key] = serde_json::json!(false);
        assert_eq!(
            status_running(&serde_json::to_vec(&status).unwrap(), "default"),
            Err("herdr.incompatible".into())
        );
    }
    for server in [
        serde_json::json!({"session":null,"running":false,"status":"permission_denied"}),
        serde_json::json!({"session":null,"status":"not_running"}),
    ] {
        let status = serde_json::json!({"client":{"session":null},"server":server});
        assert_eq!(
            status_running(&serde_json::to_vec(&status).unwrap(), "default"),
            Err("herdr.invalid_response".into())
        );
    }
}

#[test]
fn only_explicit_absence_in_the_selected_session_allows_start() {
    let status = |server: Value| {
        serde_json::to_vec(&serde_json::json!({"client":{"session":"test"},"server":server}))
            .unwrap()
    };
    assert_eq!(
        status_running(
            &status(serde_json::json!({"session":"test","running":false,"status":"not_running"})),
            "test"
        ),
        Ok(false)
    );
    assert_eq!(
        status_running(
            &status(serde_json::json!({"session":"test","running":true,"compatible":true})),
            "test"
        ),
        Ok(true)
    );
    for server in [
        serde_json::json!({"session":"other","running":false,"status":"not_running"}),
        serde_json::json!({"session":"test","running":false,"status":"permission_denied"}),
        serde_json::json!({"session":"test","status":"not_running"}),
        serde_json::json!({"session":"test","running":true,"compatible":false}),
    ] {
        assert!(status_running(&status(server), "test").is_err());
    }
    assert!(status_running(b"broken", "test").is_err());
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::sync::Arc;

    struct Fixture {
        root: PathBuf,
        startup: Arc<LocalHerdrStartup>,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("kanban startup {}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            let entry = root.join("herdr.cmd");
            std::fs::write(&entry, format!(r#"@echo off
setlocal
if not "%~1"=="--session" exit /b 10
if not "%~2"=="test" exit /b 11
if "%~3"=="server" goto server
if exist "{root}\denied" (echo PermissionDenied 1>&2 & exit /b 5)
if exist "{root}\malformed" (echo invalid & exit /b 0)
if exist "{root}\ready" (
echo {{"client":{{"session":"test"}},"server":{{"session":"test","running":true,"compatible":true}}}}
) else (
echo {{"client":{{"session":"test"}},"server":{{"session":"test","running":false,"status":"not_running"}}}}
)
exit /b 0
:server
echo launch>>"{root}\launches"
if exist "{root}\fail" (echo fixture launch failed 1>&2 & exit /b 9)
if exist "{root}\detach" exit /b 0
ping -n 2 127.0.0.1 >nul
echo ready>"{root}\ready"
exit /b 0
"#, root=root.display()).replace('\n', "\r\n")).unwrap();
            let startup = Arc::new(LocalHerdrStartup::new(
                Cli {
                    executable: entry,
                    session: "test".into(),
                    remote: None,
                },
                root.join("startup.log"),
            ));
            Self { root, startup }
        }
        fn mark(&self, name: &str) {
            std::fs::write(self.root.join(name), "yes").unwrap();
        }
        fn launches(&self) -> usize {
            std::fs::read_to_string(self.root.join("launches"))
                .unwrap_or_default()
                .lines()
                .count()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Some(mut child) = self.startup.attempt.lock().unwrap().child.take() {
                let _ = child.wait();
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn running_server_is_reused() {
        let f = Fixture::new();
        f.mark("ready");
        f.startup.ensure(false).unwrap();
        f.startup.ensure(true).unwrap();
        assert_eq!(f.launches(), 0);
    }

    #[test]
    fn simultaneous_calls_share_one_launch_and_wait_for_readiness() {
        let f = Fixture::new();
        let threads: Vec<_> = (0..6)
            .map(|_| {
                let startup = f.startup.clone();
                std::thread::spawn(move || startup.ensure(true))
            })
            .collect();
        for thread in threads {
            thread.join().unwrap().unwrap();
        }
        assert_eq!(f.launches(), 1);
        assert!(f.root.join("ready").exists());
    }

    #[test]
    fn permission_and_invalid_status_never_launch() {
        for mode in ["denied", "malformed"] {
            let f = Fixture::new();
            f.mark(mode);
            assert!(f.startup.ensure(false).is_err());
            assert_eq!(f.launches(), 0);
        }
    }

    #[test]
    fn missing_executable_is_not_absence_of_server() {
        let f = Fixture::new();
        std::fs::remove_file(&f.startup.cli.executable).unwrap();
        assert_eq!(f.startup.ensure(false), Err("herdr.unavailable".into()));
        assert_eq!(f.launches(), 0);
    }

    #[test]
    fn failure_is_cached_until_explicit_retry() {
        let f = Fixture::new();
        f.mark("fail");
        let error = f.startup.ensure(false).unwrap_err();
        assert!(error.contains("fixture launch failed"));
        std::fs::remove_file(f.root.join("fail")).unwrap();
        assert_eq!(f.startup.ensure(false), Err(error));
        assert_eq!(f.launches(), 1);
        f.startup.ensure(true).unwrap();
        assert_eq!(f.launches(), 2);
    }

    #[test]
    fn timeout_retry_keeps_waiting_for_the_same_process() {
        let f = Fixture::new();
        {
            let mut attempt = f.startup.attempt.lock().unwrap();
            assert_eq!(
                f.startup.run(&mut attempt, Duration::from_millis(250)),
                Err("herdr.startup_timeout".into())
            );
            assert!(attempt.child.is_some());
        }
        f.startup.ensure(true).unwrap();
        assert_eq!(f.launches(), 1);
    }

    #[test]
    fn timed_out_detached_launcher_is_not_replayed_on_retry() {
        let f = Fixture::new();
        f.mark("detach");
        let mut attempt = f.startup.attempt.lock().unwrap();
        for _ in 0..2 {
            assert_eq!(
                f.startup.run(&mut attempt, Duration::from_millis(300)),
                Err("herdr.startup_timeout".into())
            );
        }
        assert_eq!(f.launches(), 1);
    }

    #[test]
    #[ignore = "explicit read-only live default Herdr acceptance"]
    fn live_default_session_status_is_accepted() {
        let cli = Cli {
            executable: std::env::var("KANBAN_STARTUP_ACCEPTANCE_EXECUTABLE")
                .expect("explicit Herdr executable")
                .into(),
            session: "default".into(),
            remote: None,
        };
        // Probe only: this acceptance must never start or stop the user's server.
        let startup = LocalHerdrStartup::new(cli.clone(), PathBuf::new());
        assert!(startup.probe(Duration::from_secs(3)).unwrap());
        let workspaces = cli
            .request(&["workspace", "list"])
            .map_err(|e| e.message)
            .unwrap();
        assert!(workspaces.pointer("/result/workspaces").unwrap().is_array());
        println!("Real default Herdr: startup probe and workspace list passed (read-only)");
    }

    #[test]
    #[ignore = "explicit isolated live Herdr startup acceptance"]
    fn live_server_starts_through_custom_launcher_and_survives_manager_drop() {
        let exe = std::env::var("KANBAN_STARTUP_ACCEPTANCE_EXECUTABLE")
            .expect("explicit Herdr executable");
        let root =
            std::env::temp_dir().join(format!("kanban live startup {}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let entry = root.join("herdr.cmd");
        std::fs::write(
            &entry,
            format!(
                "@echo off\r\nsetlocal\r\nset \"APPDATA={}\"\r\n\"{}\" %*\r\n",
                root.display(),
                exe
            ),
        )
        .unwrap();
        let cli = Cli {
            executable: entry,
            session: "startup-acceptance".into(),
            remote: None,
        };
        struct Cleanup(Cli, PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = self.0.output(&["server", "stop"]);
                let _ = std::fs::remove_dir_all(&self.1);
            }
        }
        let _cleanup = Cleanup(cli.clone(), root.clone());
        let startup = LocalHerdrStartup::new(cli.clone(), root.join("startup.log"));
        assert!(!startup.probe(Duration::from_secs(3)).unwrap());
        startup.ensure(false).unwrap();
        assert!(startup.probe(Duration::from_secs(3)).unwrap());
        let workspaces = cli
            .request(&["workspace", "list"])
            .map_err(|e| e.message)
            .unwrap();
        assert!(workspaces.pointer("/result/workspaces").unwrap().is_array());
        drop(startup);
        let second = LocalHerdrStartup::new(cli, root.join("unused.log"));
        second.ensure(false).unwrap();
        assert!(
            !root.join("unused.log").exists(),
            "existing server must not be launched again"
        );
        println!(
            "Real isolated Herdr: offline -> launcher -> ready -> workspace list -> manager drop -> reuse passed"
        );
    }
}
