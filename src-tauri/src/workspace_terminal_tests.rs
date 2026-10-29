use super::*;
use crate::workflow::{Herdr, SendError};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Fake {
    machine: String,
    panes: Mutex<Vec<(String, Target)>>,
    creates: Mutex<Vec<String>>,
    lose_receipt: Mutex<bool>,
    offline: Mutex<bool>,
    fail_before: Mutex<bool>,
}
impl Herdr for Fake {
    fn machine_id(&self) -> &str {
        &self.machine
    }
    fn targets(&self, _: &str) -> Result<Vec<crate::workflow::Target>, String> {
        panic!("must not query Agents")
    }
    fn send(&self, _: &crate::workflow::Target, _: &str) -> Result<(), SendError> {
        panic!("must not send input")
    }
    fn workspace_terminal_live(&self, target: &Target) -> Result<bool, String> {
        if *self.offline.lock().unwrap() {
            return Err("offline".into());
        }
        let panes = self.panes.lock().unwrap();
        for (_, live) in panes.iter() {
            if live.terminal_id == target.terminal_id || live.pane_id == target.pane_id {
                if live != target {
                    return Err("binding.identity_changed".into());
                }
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn find_workspace_terminal(
        &self,
        workspace: &str,
        operation: &str,
    ) -> Result<Option<Target>, String> {
        if *self.offline.lock().unwrap() {
            return Err("offline".into());
        }
        Ok(self
            .panes
            .lock()
            .unwrap()
            .iter()
            .find(|(op, t)| op == operation && t.workspace_id == workspace)
            .map(|(_, t)| t.clone()))
    }
    fn create_workspace_terminal(
        &self,
        workspace: &str,
        cwd: &str,
        operation: &str,
        check: &dyn Fn() -> Result<(), String>,
    ) -> Result<Target, SendError> {
        check().map_err(|message| SendError {
            message,
            uncertain: false,
        })?;
        if *self.fail_before.lock().unwrap() {
            return Err(SendError {
                message: "unavailable".into(),
                uncertain: false,
            });
        }
        self.creates.lock().unwrap().push(cwd.into());
        let target = Target {
            machine_id: self.machine.clone(),
            session: "default".into(),
            workspace_id: workspace.into(),
            tab_id: format!("tab-{operation}"),
            pane_id: operation.into(),
            terminal_id: format!("terminal-{operation}"),
        };
        self.panes
            .lock()
            .unwrap()
            .push((operation.into(), target.clone()));
        if *self.lose_receipt.lock().unwrap() {
            return Err(SendError {
                message: "timeout".into(),
                uncertain: true,
            });
        }
        Ok(target)
    }
}
fn fixture(machine: &str) -> (Arc<Workflow>, Arc<Fake>) {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    if machine != "local" {
        db.execute(
            "INSERT INTO machines(id,name,kind,session,disabled) VALUES(?1,?1,'ssh','default',0)",
            [machine],
        )
        .unwrap();
    }
    // A second machine identity proves lookup never defaults to the local project.
    let cwd = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    db.execute("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at,herdr_session,workspace_id,machine_id) VALUES('p','shell',?1,'shell','now','default','w1',?2)",params![cwd,machine]).unwrap();
    let fake = Arc::new(Fake {
        machine: machine.into(),
        ..Default::default()
    });
    (
        Arc::new(Workflow {
            db: Arc::new(Mutex::new(db)),
            herdr: fake.clone(),
        }),
        fake,
    )
}
#[test]
fn creates_once_in_project_directory_and_reuses_without_agents_or_input() {
    let (app, fake) = fixture("local");
    let first = app.ensure_workspace_terminal("p", &|| Ok(())).unwrap();
    assert_eq!(
        app.ensure_workspace_terminal("p", &|| Ok(())).unwrap(),
        first
    );
    assert_eq!(
        *fake.creates.lock().unwrap(),
        vec![
            std::env::current_dir()
                .unwrap()
                .to_string_lossy()
                .to_string()
        ]
    );
    // A changed Workspace default does not reset an existing shell or require it to be idle.
    app.db
        .lock()
        .unwrap()
        .execute("UPDATE projects SET root_path='missing/new-directory'", [])
        .unwrap();
    assert_eq!(
        app.ensure_workspace_terminal("p", &|| Ok(())).unwrap(),
        first
    );
}
#[test]
fn concurrent_opens_share_one_creation() {
    let (app, fake) = fixture("local");
    let threads: Vec<_> = (0..6)
        .map(|_| {
            let app = app.clone();
            std::thread::spawn(move || app.ensure_workspace_terminal("p", &|| Ok(())).unwrap())
        })
        .collect();
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert!(results.iter().all(|t| *t == results[0]));
    assert_eq!(fake.creates.lock().unwrap().len(), 1);
}
#[test]
fn lost_receipt_recovers_after_database_reopen_without_replay() {
    let (app, fake) = fixture("local");
    *fake.lose_receipt.lock().unwrap() = true;
    assert_eq!(
        app.ensure_workspace_terminal("p", &|| Ok(())).unwrap_err(),
        "timeout"
    );
    let path = std::env::temp_dir().join(format!("kanban-shell-{}.db", uuid::Uuid::new_v4()));
    app.db
        .lock()
        .unwrap()
        .execute("VACUUM INTO ?1", [path.to_str().unwrap()])
        .unwrap();
    let reopened = Connection::open(&path).unwrap();
    crate::db::init(&reopened).unwrap();
    crate::db::recover_interrupted(&reopened).unwrap();
    let app = Workflow {
        db: Arc::new(Mutex::new(reopened)),
        herdr: fake.clone(),
    };
    let recovered = app.ensure_workspace_terminal("p", &|| Ok(())).unwrap();
    assert_eq!(app.saved_workspace_terminal("p").unwrap(), recovered);
    assert_eq!(fake.creates.lock().unwrap().len(), 1);
    drop(app);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn unknown_creation_with_no_observed_pane_is_not_replayed() {
    let (app, fake) = fixture("local");
    *fake.lose_receipt.lock().unwrap() = true;
    assert!(app.ensure_workspace_terminal("p", &|| Ok(())).is_err());
    fake.panes.lock().unwrap().clear();
    assert_eq!(
        app.ensure_workspace_terminal("p", &|| Ok(())).unwrap_err(),
        "workspace_terminal.unknown"
    );
    assert_eq!(fake.creates.lock().unwrap().len(), 1);
}
#[test]
fn only_confirmed_absence_creates_a_replacement() {
    let (app, fake) = fixture("local");
    let first = app.ensure_workspace_terminal("p", &|| Ok(())).unwrap();
    *fake.offline.lock().unwrap() = true;
    assert!(app.ensure_workspace_terminal("p", &|| Ok(())).is_err());
    assert_eq!(fake.creates.lock().unwrap().len(), 1);
    *fake.offline.lock().unwrap() = false;
    fake.panes.lock().unwrap().clear();
    assert_ne!(
        app.ensure_workspace_terminal("p", &|| Ok(())).unwrap(),
        first
    );
    assert_eq!(fake.creates.lock().unwrap().len(), 2);
}
#[test]
fn moved_or_replaced_terminal_cannot_be_reused_or_duplicated() {
    let (app, fake) = fixture("local");
    app.ensure_workspace_terminal("p", &|| Ok(())).unwrap();
    fake.panes.lock().unwrap()[0].1.workspace_id = "elsewhere".into();
    assert_eq!(
        app.ensure_workspace_terminal("p", &|| Ok(())).unwrap_err(),
        "binding.identity_changed"
    );
    assert_eq!(fake.creates.lock().unwrap().len(), 1);
}
#[test]
fn rejects_wrong_machine_and_project_relink() {
    let (app, fake) = fixture("remote-test");
    let target = app.ensure_workspace_terminal("p", &|| Ok(())).unwrap();
    assert_eq!(target.machine_id, "remote-test");
    app.db
        .lock()
        .unwrap()
        .execute("UPDATE projects SET machine_id='local'", [])
        .unwrap();
    assert!(app.ensure_workspace_terminal("p", &|| Ok(())).is_err());
    assert_eq!(fake.creates.lock().unwrap().len(), 1);
}
#[test]
fn definite_pre_create_failure_and_cancel_are_retryable() {
    let (app, fake) = fixture("local");
    assert!(
        app.ensure_workspace_terminal("p", &|| Err("cancelled".into()))
            .is_err()
    );
    *fake.fail_before.lock().unwrap() = true;
    assert!(app.ensure_workspace_terminal("p", &|| Ok(())).is_err());
    *fake.fail_before.lock().unwrap() = false;
    app.ensure_workspace_terminal("p", &|| Ok(())).unwrap();
    assert_eq!(fake.creates.lock().unwrap().len(), 1);
}

#[test]
fn disabled_execution_computer_cannot_create_or_reopen_shell() {
    let (app, fake) = fixture("remote-test");
    app.ensure_workspace_terminal("p", &|| Ok(())).unwrap();
    app.db
        .lock()
        .unwrap()
        .execute("UPDATE machines SET disabled=1 WHERE id='remote-test'", [])
        .unwrap();
    assert_eq!(
        app.ensure_workspace_terminal("p", &|| Ok(())).unwrap_err(),
        "machine.disabled"
    );
    assert_eq!(fake.creates.lock().unwrap().len(), 1);
}

#[test]
fn dedicated_shell_is_reserved_from_task_ownership_even_after_agent_launch() {
    let (app, _) = fixture("local");
    let shell = app.ensure_workspace_terminal("p", &|| Ok(())).unwrap();
    let target = crate::workflow::Target {
        machine_id: shell.machine_id,
        session: shell.session,
        workspace_id: shell.workspace_id,
        pane_id: shell.pane_id,
        terminal_id: shell.terminal_id,
        agent: "codex".into(),
        process_id: "new-process".into(),
        codex_resume_locks: vec![],
        title: "manual agent".into(),
        status: "idle".into(),
    };
    assert_eq!(
        crate::workflow::pane_owner(&app.db.lock().unwrap(), &target).unwrap(),
        Some("workspace-terminal".into())
    );
}
