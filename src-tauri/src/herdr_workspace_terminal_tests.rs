//! Exercise persistence and the actual CLI adapter across a restored terminal ID.
use super::*;
use crate::workflow::Workflow;
use rusqlite::{Connection, params};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

const OPERATION: &str = "22222222-2222-4222-8222-222222222222";

struct Fixture {
    dir: PathBuf,
    cli: Cli,
    app: Workflow,
    saved: Target,
}

impl Fixture {
    fn new() -> Self {
        let dir =
            std::env::temp_dir().join(format!("kanban-shell-restore-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(
            dir.join("herdr.cmd"),
            concat!(
                "@echo off\r\n",
                "echo %~3 %~4>>\"%~dp0calls.txt\"\r\n",
                "if \"%~3 %~4\"==\"workspace list\" (\r\n",
                " type \"%~dp0workspaces.json\"\r\n exit /b 0\r\n)\r\n",
                "if \"%~3 %~4\"==\"tab list\" (\r\n",
                " type \"%~dp0tabs.json\"\r\n exit /b 0\r\n)\r\n",
                "if \"%~3 %~4\"==\"pane list\" (\r\n",
                " if \"%~5\"==\"--workspace\" (\r\n",
                "  type \"%~dp0workspace-panes.json\"\r\n",
                " ) else (\r\n  type \"%~dp0panes.json\"\r\n )\r\n exit /b 0\r\n)\r\n",
                "exit /b 9\r\n",
            ),
        )
        .unwrap();
        std::fs::write(
            dir.join("workspaces.json"),
            json!({"result":{"workspaces":[{"workspace_id":"w1"}]}}).to_string(),
        )
        .unwrap();
        let cli = Cli {
            executable: dir.join("herdr.cmd"),
            session: "default".into(),
            remote: None,
        };
        let saved = Target {
            machine_id: "local".into(),
            session: "default".into(),
            workspace_id: "w1".into(),
            tab_id: "w1:t1".into(),
            pane_id: "w1:p1".into(),
            terminal_id: "before-restart".into(),
        };
        let db = Connection::open_in_memory().unwrap();
        crate::db::init(&db).unwrap();
        db.execute("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at,machine_id,herdr_session,workspace_id) VALUES('p','shell',?1,'shell','now','local','default','w1')", [dir.to_str().unwrap()]).unwrap();
        db.execute("INSERT INTO workspace_terminals(machine_id,session,workspace_id,operation_id,cwd,target_json,state) VALUES('local','default','w1',?1,?2,?3,'ready')", params![OPERATION, dir.to_str().unwrap(), serde_json::to_string(&saved).unwrap()]).unwrap();
        let app = Workflow {
            db: Arc::new(Mutex::new(db)),
            herdr: Arc::new(cli.clone()),
        };
        let fixture = Self {
            dir,
            cli,
            app,
            saved,
        };
        fixture.inventory(vec![fixture.restored()], vec![fixture.marker()]);
        fixture
    }

    fn restored(&self) -> Target {
        Target {
            terminal_id: "after-restart".into(),
            ..self.saved.clone()
        }
    }

    fn marker(&self) -> Value {
        json!({"workspace_id":"w1", "tab_id":"w1:t1", "label":label(OPERATION)})
    }

    fn inventory(&self, panes: Vec<Target>, tabs: Vec<Value>) {
        let scoped: Vec<_> = panes.iter().filter(|p| p.workspace_id == "w1").collect();
        for (file, value) in [
            ("panes.json", json!({"result":{"panes":panes}})),
            ("workspace-panes.json", json!({"result":{"panes":scoped}})),
            ("tabs.json", json!({"result":{"tabs":tabs}})),
        ] {
            std::fs::write(self.dir.join(file), value.to_string()).unwrap();
        }
    }

    fn assert_reads_only(&self) {
        let calls = std::fs::read_to_string(self.dir.join("calls.txt")).unwrap();
        assert!(
            calls
                .lines()
                .all(|line| matches!(line.trim(), "workspace list" | "pane list" | "tab list")),
            "{calls}"
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.dir.starts_with(std::env::temp_dir()));
        assert!(
            self.dir
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("kanban-shell-restore-")
        );
        std::fs::remove_dir_all(&self.dir).unwrap();
    }
}

#[test]
fn restored_workspace_shell_reconnects_and_persists_without_creating_or_sending() {
    let f = Fixture::new();
    // The native viewer's strict identity check must continue to reject old IDs.
    assert_eq!(
        f.cli.workspace_terminal_live(&f.saved).unwrap_err(),
        "binding.identity_changed"
    );
    let restored = f.app.ensure_workspace_terminal("p", &|| Ok(())).unwrap();
    assert_eq!(restored, f.restored());
    assert_eq!(f.app.saved_workspace_terminal("p").unwrap(), restored);
    assert_eq!(
        f.app.ensure_workspace_terminal("p", &|| Ok(())).unwrap(),
        restored
    );
    let operation: String = f
        .app
        .db
        .lock()
        .unwrap()
        .query_row("SELECT operation_id FROM workspace_terminals", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(operation, OPERATION);
    f.assert_reads_only();
}

#[test]
fn restored_shell_requires_unique_marker_same_slot_and_absent_old_terminal() {
    for case in [
        "no-marker",
        "duplicate-marker",
        "different-tab",
        "different-pane",
        "moved-slot-after-restart",
        "moved-workspace-after-restart",
        "multiple-panes",
        "old-terminal-moved",
        "duplicate-terminal",
    ] {
        let f = Fixture::new();
        let mut panes = vec![f.restored()];
        let mut tabs = vec![f.marker()];
        match case {
            "no-marker" => tabs.clear(),
            "duplicate-marker" => tabs.push(f.marker()),
            "different-tab" => {
                panes[0].tab_id = "w1:t2".into();
                tabs[0]["tab_id"] = "w1:t2".into();
            }
            "different-pane" => {
                panes.push(Target {
                    pane_id: "w1:p2".into(),
                    tab_id: "w1:t2".into(),
                    ..f.restored()
                });
                tabs[0]["tab_id"] = "w1:t2".into();
            }
            "moved-slot-after-restart" => {
                panes[0].pane_id = "w1:p2".into();
                panes[0].tab_id = "w1:t2".into();
                tabs[0]["tab_id"] = "w1:t2".into();
            }
            "moved-workspace-after-restart" => {
                panes[0].workspace_id = "w2".into();
                panes[0].pane_id = "w2:p1".into();
                panes[0].tab_id = "w2:t1".into();
                tabs[0]["workspace_id"] = "w2".into();
                tabs[0]["tab_id"] = "w2:t1".into();
            }
            "multiple-panes" => panes.push(Target {
                pane_id: "w1:p2".into(),
                terminal_id: "other".into(),
                ..f.restored()
            }),
            "old-terminal-moved" => panes.push(Target {
                workspace_id: "w2".into(),
                tab_id: "w2:t1".into(),
                pane_id: "w2:p1".into(),
                ..f.saved.clone()
            }),
            "duplicate-terminal" => panes.push(Target {
                workspace_id: "w2".into(),
                tab_id: "w2:t1".into(),
                pane_id: "w2:p1".into(),
                ..f.restored()
            }),
            _ => unreachable!(),
        }
        f.inventory(panes, tabs);
        assert!(
            f.app.ensure_workspace_terminal("p", &|| Ok(())).is_err(),
            "{case}"
        );
        assert_eq!(
            f.app.saved_workspace_terminal("p").unwrap(),
            f.saved,
            "{case}"
        );
        f.assert_reads_only();
    }
}

#[test]
fn restored_shell_cannot_claim_task_retired_or_other_workspace_reservations() {
    for case in ["task-pane", "task-terminal", "retired", "workspace"] {
        let f = Fixture::new();
        let restored = serde_json::to_string(&f.restored()).unwrap();
        {
            let db = f.app.db.lock().unwrap();
            match case {
                "task-pane" | "task-terminal" => {
                    db.execute("INSERT INTO tasks(id,project_id,name,created_at,updated_at) VALUES('task','p','task','now','now')", []).unwrap();
                    let pane = if case == "task-pane" {
                        "w1:p1"
                    } else {
                        "w1:p2"
                    };
                    db.execute("INSERT INTO task_bindings(task_id,machine_id,session,pane_id,target_json,state) VALUES('task','local','default',?1,?2,'reserved')", params![pane,restored]).unwrap();
                }
                "retired" => {
                    db.execute("INSERT INTO task_execution_resets VALUES('reset','task',0,'retired','{}','now')", []).unwrap();
                    db.execute(
                        "INSERT INTO retired_execution_targets VALUES('reset',?1)",
                        [&restored],
                    )
                    .unwrap();
                }
                "workspace" => {
                    db.execute("INSERT INTO workspace_terminals VALUES('local','default','w2','other','cwd',?1,'ready')", [&restored]).unwrap();
                }
                _ => unreachable!(),
            }
        }
        assert_eq!(
            f.app
                .ensure_workspace_terminal("p", &|| Ok(()))
                .unwrap_err(),
            "workspace_terminal.occupied",
            "{case}"
        );
        assert_eq!(
            f.app.saved_workspace_terminal("p").unwrap(),
            f.saved,
            "{case}"
        );
        f.assert_reads_only();
    }
}

#[test]
fn cancelled_or_stale_recovery_preserves_saved_binding() {
    for stale in [false, true] {
        let f = Fixture::new();
        let checks = std::cell::Cell::new(0);
        let result = f.app.ensure_workspace_terminal("p", &|| {
            checks.set(checks.get() + 1);
            if checks.get() == 3 {
                if stale {
                    f.app
                        .db
                        .lock()
                        .unwrap()
                        .execute(
                            "UPDATE workspace_terminals SET operation_id='newer-operation'",
                            [],
                        )
                        .unwrap();
                } else {
                    return Err("terminal.stale".into());
                }
            }
            Ok(())
        });
        assert_eq!(
            result.unwrap_err(),
            if stale {
                "binding.identity_changed"
            } else {
                "terminal.stale"
            }
        );
        assert_eq!(f.app.saved_workspace_terminal("p").unwrap(), f.saved);
        f.assert_reads_only();
    }
}

#[test]
fn unreadable_inventory_cannot_rebind_or_create_and_missing_new_cwd_does_not_reset_shell() {
    let f = Fixture::new();
    std::fs::write(f.dir.join("panes.json"), "unreadable").unwrap();
    assert_eq!(
        f.app
            .ensure_workspace_terminal("p", &|| Ok(()))
            .unwrap_err(),
        "herdr.invalid_response"
    );
    assert_eq!(f.app.saved_workspace_terminal("p").unwrap(), f.saved);
    f.inventory(vec![f.restored()], vec![f.marker()]);
    f.app
        .db
        .lock()
        .unwrap()
        .execute("UPDATE projects SET root_path='missing/new-default'", [])
        .unwrap();
    assert_eq!(
        f.app.ensure_workspace_terminal("p", &|| Ok(())).unwrap(),
        f.restored()
    );
    f.assert_reads_only();
}

#[test]
#[ignore = "read-only real local/SSH inventory; requires KANBAN_SHELL_RESTORE_FIXTURE"]
fn live_workspace_terminal_restoration_read_only() {
    #[derive(serde::Deserialize)]
    struct Input {
        saved: Target,
        operation: String,
        cwd: String,
        machine: Option<crate::machines::Machine>,
    }
    // Delegate observations to the production CLI/helper transport. No code path
    // in this acceptance adapter can create a shell or send user input.
    struct ReadOnly(Cli);
    impl Herdr for ReadOnly {
        fn machine_id(&self) -> &str {
            self.0.machine_id()
        }
        fn session(&self) -> &str {
            &self.0.session
        }
        fn workspace_terminal_live(&self, target: &Target) -> Result<bool, String> {
            self.0.workspace_terminal_live(target)
        }
        fn recover_workspace_terminal(
            &self,
            target: &Target,
            operation: &str,
        ) -> Result<Target, String> {
            self.0.recover_workspace_terminal(target, operation)
        }
        fn find_workspace_terminal(
            &self,
            workspace: &str,
            operation: &str,
        ) -> Result<Option<Target>, String> {
            self.0.find_workspace_terminal(workspace, operation)
        }
        fn targets(&self, _: &str) -> Result<Vec<crate::workflow::Target>, String> {
            panic!("read-only shell acceptance must not query Agents")
        }
        fn send(&self, _: &crate::workflow::Target, _: &str) -> Result<(), SendError> {
            panic!("read-only acceptance must not send input")
        }
    }
    let path = std::env::var_os("KANBAN_SHELL_RESTORE_FIXTURE").expect("explicit fixture path");
    let input: Input = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let mut cli = Cli::from_env();
    cli.session = input.saved.session.clone();
    cli.remote = input.machine;
    let _scope = crate::remote_control::Scope::enter();
    assert_eq!(
        cli.workspace_terminal_live(&input.saved).unwrap_err(),
        "binding.identity_changed"
    );
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    if let Some(machine) = &cli.remote {
        db.execute(
            "INSERT INTO machines(id,name,kind,session) VALUES(?1,'acceptance','ssh',?2)",
            params![machine.id, machine.session],
        )
        .unwrap();
    }
    db.execute("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at,machine_id,herdr_session,workspace_id) VALUES('p','acceptance',?1,'acceptance','now',?2,?3,?4)", params![input.cwd,input.saved.machine_id,input.saved.session,input.saved.workspace_id]).unwrap();
    db.execute(
        "INSERT INTO workspace_terminals VALUES(?1,?2,?3,?4,?5,?6,'ready')",
        params![
            input.saved.machine_id,
            input.saved.session,
            input.saved.workspace_id,
            input.operation,
            input.cwd,
            serde_json::to_string(&input.saved).unwrap()
        ],
    )
    .unwrap();
    let app = Workflow {
        db: Arc::new(Mutex::new(db)),
        herdr: Arc::new(ReadOnly(cli)),
    };
    let restored = app.ensure_workspace_terminal("p", &|| Ok(())).unwrap();
    assert_ne!(restored.terminal_id, input.saved.terminal_id);
    assert_eq!(app.saved_workspace_terminal("p").unwrap(), restored);
    assert_eq!(
        app.ensure_workspace_terminal("p", &|| Ok(())).unwrap(),
        restored
    );
    println!(
        "Verified real {} shell recovery and isolated persistence for {}",
        restored.machine_id, restored.workspace_id
    );
}
