use super::*;
use crate::workflow::{Herdr, SendError};
use rusqlite::Connection;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Fake {
    panes: Mutex<Vec<IdlePane>>,
    closed: Mutex<Vec<String>>,
    uncertain: Mutex<bool>,
}
impl Herdr for Fake {
    fn session_id(&self, _: &Target) -> Result<Option<String>, String> {
        Ok(Some("00000000-0000-4000-8000-000000000041".into()))
    }
    fn close_idle_pane(
        &self,
        pane: &IdlePane,
        check: &dyn Fn() -> Result<(), String>,
    ) -> Result<(), SendError> {
        let live = self.idle_panes(&pane.target.workspace_id).unwrap();
        if !live
            .iter()
            .any(|p| p.tab_id == pane.tab_id && p.target.same_identity(&pane.target))
        {
            return Err(SendError {
                message: "cleanup.changed".into(),
                uncertain: false,
            });
        }
        check().map_err(|message| SendError {
            message,
            uncertain: false,
        })?;
        self.closed
            .lock()
            .unwrap()
            .push(pane.target.pane_id.clone());
        if *self.uncertain.lock().unwrap() {
            return Err(SendError {
                message: "disconnected".into(),
                uncertain: true,
            });
        }
        self.panes
            .lock()
            .unwrap()
            .retain(|p| !p.target.same_identity(&pane.target));
        Ok(())
    }
    fn pane_inventory(
        &self,
        _: &str,
    ) -> Result<Option<Vec<crate::workflow::PaneIdentity>>, String> {
        Ok(Some(
            self.panes
                .lock()
                .unwrap()
                .iter()
                .map(|p| crate::workflow::PaneIdentity {
                    pane_id: p.target.pane_id.clone(),
                    terminal_id: p.target.terminal_id.clone(),
                })
                .collect(),
        ))
    }
    fn idle_panes(&self, workspace: &str) -> Result<Vec<IdlePane>, String> {
        Ok(self
            .panes
            .lock()
            .unwrap()
            .iter()
            .filter(|p| {
                p.target.workspace_id == workspace
                    && matches!(p.target.status.as_str(), "idle" | "done")
            })
            .cloned()
            .collect())
    }
    fn targets(&self, _: &str) -> Result<Vec<Target>, String> {
        Ok(vec![])
    }
    fn send(&self, _: &Target, _: &str) -> Result<(), SendError> {
        panic!("cleanup must not dispatch")
    }
}
#[test]
fn preview_reports_database_errors_instead_of_claiming_no_idle_sessions() {
    let (app, fake) = fixture();
    fake.panes.lock().unwrap().push(pane("candidate"));
    app.db
        .lock()
        .unwrap()
        .execute("DROP TABLE pane_allocations", [])
        .unwrap();
    assert!(app.preview_idle_cleanup("p").is_err());
    assert!(fake.closed.lock().unwrap().is_empty());
}
#[test]
fn late_inprogress_binding_protects_a_previously_unbound_candidate() {
    let (app, fake) = fixture();
    let orphan = pane("orphan");
    fake.panes.lock().unwrap().push(orphan.clone());
    let preview = app.preview_idle_cleanup("p").unwrap();
    app.db.lock().unwrap().execute("INSERT INTO task_bindings(task_id,session,pane_id,target_json,state) VALUES('task','default','orphan',?1,'valid')",[serde_json::to_string(&orphan.target).unwrap()]).unwrap();
    let report = app.execute_idle_cleanup("p", &preview.id).unwrap();
    assert_eq!((report.closed, report.skipped), (0, 1));
    assert!(fake.closed.lock().unwrap().is_empty());
}
#[test]
fn uncertain_closure_is_never_replayed_and_can_be_verified_after_database_reopen() {
    let (app, fake) = fixture();
    fake.panes.lock().unwrap().push(pane("orphan"));
    *fake.uncertain.lock().unwrap() = true;
    let preview = app.preview_idle_cleanup("p").unwrap();
    let report = app.execute_idle_cleanup("p", &preview.id).unwrap();
    assert_eq!((report.closed, report.failed), (0, 1));
    assert_eq!(report.items[0].state, "unknown");
    assert_eq!(
        app.preview_idle_cleanup("p").unwrap().unresolved,
        vec![preview.id.clone()]
    );
    app.execute_idle_cleanup("p", &preview.id).unwrap();
    assert_eq!(fake.closed.lock().unwrap().len(), 1);
    assert_eq!(app.preview_idle_cleanup("p").unwrap().count, 0);
    let path = std::env::temp_dir().join(format!("kanban-cleanup-{}.db", uuid::Uuid::new_v4()));
    app.db
        .lock()
        .unwrap()
        .execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])
        .unwrap();
    let reopened = Connection::open(&path).unwrap();
    crate::db::init(&reopened).unwrap();
    crate::db::recover_interrupted(&reopened).unwrap();
    let app = Workflow {
        db: Arc::new(Mutex::new(reopened)),
        herdr: fake.clone(),
    };
    fake.panes.lock().unwrap().clear();
    let verified = app.idle_cleanup_result("p", &preview.id).unwrap();
    assert_eq!((verified.closed, verified.failed), (1, 0));
    assert_eq!(fake.closed.lock().unwrap().len(), 1);
    drop(app);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn confirmed_cleanup_keeps_its_snapshot_rechecks_idle_and_is_idempotent() {
    let (app, fake) = fixture();
    fake.panes
        .lock()
        .unwrap()
        .extend([pane("idle"), pane("changed")]);
    let preview = app.preview_idle_cleanup("p").unwrap();
    fake.panes.lock().unwrap()[1].target.status = "working".into();
    fake.panes.lock().unwrap().push(pane("later"));
    let report = app.execute_idle_cleanup("p", &preview.id).unwrap();
    assert_eq!((report.closed, report.skipped, report.failed), (1, 1, 0));
    assert_eq!(*fake.closed.lock().unwrap(), vec!["idle"]);
    let repeated = app.execute_idle_cleanup("p", &preview.id).unwrap();
    assert_eq!(repeated.closed, 1);
    assert_eq!(fake.closed.lock().unwrap().len(), 1);
    assert_eq!(fake.panes.lock().unwrap().len(), 2);
}
#[test]
fn non_inprogress_binding_preserves_conversation_and_task_while_releasing_closed_pane() {
    let (app, fake) = fixture();
    let old = pane("old");
    {
        let db = app.db.lock().unwrap();
        db.execute(
            "UPDATE tasks SET status='done',completed_at='yesterday' WHERE id='task'",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO task_bindings(task_id,session,pane_id,target_json,state) VALUES('task','default','old',?1,'valid')",[serde_json::to_string(&old.target).unwrap()]).unwrap();
    }
    fake.panes.lock().unwrap().push(old);
    let preview = app.preview_idle_cleanup("p").unwrap();
    assert_eq!(preview.count, 1);
    assert_eq!(
        app.execute_idle_cleanup("p", &preview.id).unwrap().closed,
        1
    );
    assert_eq!(app.binding("task").unwrap().state, "dormant");
    let tasks = crate::tasks::list(&app.db.lock().unwrap(), "p").unwrap();
    assert_eq!(tasks[0].status, crate::models::TaskStatus::Done);
    assert_eq!(
        crate::lifecycle::view(&app.db.lock().unwrap(), "task")
            .unwrap()
            .session_id
            .as_deref(),
        Some("00000000-0000-4000-8000-000000000041")
    );
}
#[test]
fn uncertain_bound_cleanup_blocks_restart_and_keeps_its_binding() {
    let (app, fake) = fixture();
    let old = pane("old");
    {
        let db = app.db.lock().unwrap();
        db.execute("UPDATE tasks SET status='done' WHERE id='task'", [])
            .unwrap();
        db.execute("INSERT INTO task_bindings(task_id,session,pane_id,target_json,state) VALUES('task','default','old',?1,'valid')", [serde_json::to_string(&old.target).unwrap()]).unwrap();
    }
    fake.panes.lock().unwrap().push(old);
    *fake.uncertain.lock().unwrap() = true;
    let preview = app.preview_idle_cleanup("p").unwrap();
    assert_eq!(
        app.execute_idle_cleanup("p", &preview.id).unwrap().failed,
        1
    );
    assert_eq!(
        app.start_task("task", "must not send", &uuid::Uuid::new_v4().to_string())
            .unwrap_err(),
        "cleanup.pending"
    );
    assert!(app.binding("task").unwrap().target.is_some());
    assert_eq!(fake.closed.lock().unwrap().len(), 1);
}
#[test]
fn reserved_binding_and_unknown_lifecycle_remain_protected_even_when_task_is_todo() {
    for reserved in [true, false] {
        let (app, fake) = fixture();
        let candidate = pane("candidate");
        {
            let db = app.db.lock().unwrap();
            db.execute("UPDATE tasks SET status='todo'", []).unwrap();
            let json = serde_json::to_string(&candidate.target).unwrap();
            if reserved {
                db.execute("INSERT INTO task_bindings(task_id,session,pane_id,target_json,state) VALUES('task','default','candidate',?1,'reserved')",[json]).unwrap();
            } else {
                db.execute("INSERT INTO lifecycle_operations(id,task_id,kind,state,target_json,created_at) VALUES('op','task','start','unknown',?1,'now')",[json]).unwrap();
            }
        }
        fake.panes.lock().unwrap().push(candidate);
        assert_eq!(app.preview_idle_cleanup("p").unwrap().count, 0);
    }
}
#[test]
fn preview_is_not_authority_after_target_moves_tabs_or_changes_terminal() {
    for replaced in [true, false] {
        let (app, fake) = fixture();
        fake.panes.lock().unwrap().push(pane("candidate"));
        let preview = app.preview_idle_cleanup("p").unwrap();
        if replaced {
            fake.panes.lock().unwrap()[0].target.terminal_id = "replacement".into();
        } else {
            fake.panes.lock().unwrap()[0].tab_id = "other-tab".into();
        }
        let result = app.execute_idle_cleanup("p", &preview.id).unwrap();
        assert_eq!((result.closed, result.skipped), (0, 1));
        assert!(fake.closed.lock().unwrap().is_empty());
    }
}
fn pane(name: &str) -> IdlePane {
    IdlePane {
        target: Target {
            machine_id: "local".into(),
            session: "default".into(),
            workspace_id: "w1".into(),
            pane_id: name.into(),
            terminal_id: format!("term-{name}"),
            agent: "codex".into(),
            process_id: format!("pid-{name}"),
            codex_resume_locks: vec![],
            title: name.into(),
            status: "idle".into(),
        },
        tab_id: "w1:t1".into(),
        cwd: ".".into(),
    }
}
fn fixture() -> (Workflow, Arc<Fake>) {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    db.execute_batch("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at,herdr_session,workspace_id,sync_status) VALUES('p','P','.','P','now','default','w1','ready');
        INSERT INTO tasks(id,project_id,name,status,created_at,updated_at) VALUES('task','p','Task','in_progress','now','now');").unwrap();
    let fake = Arc::new(Fake::default());
    (
        Workflow {
            db: Arc::new(Mutex::new(db)),
            herdr: fake.clone(),
        },
        fake,
    )
}
#[test]
fn preview_excludes_inprogress_idle_codex_and_does_not_close_anything() {
    let (app, fake) = fixture();
    let bound = pane("bound");
    app.db.lock().unwrap().execute("INSERT INTO task_bindings(task_id,session,pane_id,target_json,state) VALUES('task','default','bound',?1,'valid')", [serde_json::to_string(&bound.target).unwrap()]).unwrap();
    fake.panes.lock().unwrap().extend([bound, pane("orphan")]);
    let preview = app.preview_idle_cleanup("p").unwrap();
    assert_eq!(preview.count, 1);
    assert!(fake.closed.lock().unwrap().is_empty());
    assert!(app.binding("task").unwrap().target.is_some());
}
