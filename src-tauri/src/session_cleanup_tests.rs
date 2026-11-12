use super::*;
use crate::workflow::{Herdr, SendError};
use std::sync::Mutex;
#[derive(Default)]
struct Fake {
    targets: Mutex<Vec<Target>>,
    closed: Mutex<Vec<String>>,
    uncertain: Mutex<bool>,
    retain: Mutex<bool>,
}
impl Herdr for Fake {
    fn workspaces(&self) -> Result<Vec<crate::projects::Workspace>, String> {
        Ok(vec![crate::projects::Workspace {
            id: "w1".into(),
            name: "Hidden workspace".into(),
            directory: Some(".".into()),
        }])
    }
    fn targets(&self, _: &str) -> Result<Vec<Target>, String> {
        Ok(self.targets.lock().unwrap().clone())
    }
    fn capture_runtime(&self, t: &Target) -> Result<Runtime, String> {
        self.validate(&t.workspace_id, t)?;
        Ok(Runtime {
            target: t.clone(),
            processes: vec![],
        })
    }
    fn cleanup_target_absent(&self, t: &Target) -> Result<bool, String> {
        Ok(!self
            .targets
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.terminal_id == t.terminal_id))
    }
    fn close_runtime(
        &self,
        r: &Runtime,
        check: &dyn Fn() -> Result<(), String>,
    ) -> Result<(), SendError> {
        check().map_err(|message| SendError {
            message,
            uncertain: false,
        })?;
        self.closed.lock().unwrap().push(r.target.pane_id.clone());
        if !*self.retain.lock().unwrap() {self.targets.lock().unwrap().retain(|p|p.terminal_id!=r.target.terminal_id);}
        if *self.uncertain.lock().unwrap() {
            Err(SendError {
                message: "lost response".into(),
                uncertain: true,
            })
        } else {
            Ok(())
        }
    }
    fn send(&self, _: &Target, _: &str) -> Result<(), SendError> {
        panic!("cleanup never sends a prompt")
    }
}
fn target(id: &str) -> Target {
    Target {
        machine_id: "local".into(),
        session: "default".into(),
        workspace_id: "w1".into(),
        pane_id: id.into(),
        terminal_id: format!("term-{id}"),
        agent: "codex".into(),
        process_id: format!("pid-{id}"),
        codex_resume_locks: vec![],
        title: id.into(),
        status: "working".into(),
    }
}
fn fixture() -> (Workflow, Arc<Fake>) {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    db.execute_batch("INSERT INTO projects(id,name,root_path,herdr_workspace,herdr_session,workspace_id,sync_status,created_at) VALUES('p','P','.','P','default','w1','ready','now');
 INSERT INTO tasks(id,project_id,name,status,created_at,updated_at) VALUES('t','p','T','in_progress','now','now');").unwrap();
    let fake = Arc::new(Fake::default());
    (
        Workflow {
            db: Arc::new(Mutex::new(db)),
            herdr: fake.clone(),
        },
        fake,
    )
}
fn bind(app: &Workflow, t: &Target) {
    app.db.lock().unwrap().execute("INSERT INTO task_bindings(task_id,machine_id,session,pane_id,target_json,state) VALUES('t','local','default',?1,?2,'valid')",params![t.pane_id,serde_json::to_string(t).unwrap()]).unwrap();
}
#[test]
fn global_confirmation_closes_working_unbound_agents_but_preserves_inprogress_and_shells() {
    let (app, fake) = fixture();
    let bound = target("bound");
    bind(&app, &bound);
    let mut shell = target("shell");
    shell.agent = "shell".into();
    fake.targets
        .lock()
        .unwrap()
        .extend([bound, target("orphan"), shell]);
    let batch = app.preview_session_cleanup().unwrap();
    assert_eq!(batch.items.len(), 2);
    assert!(fake.closed.lock().unwrap().is_empty());
    fake.targets.lock().unwrap().push(target("later"));
    let report = app.execute_session_cleanup(&batch.id).unwrap();
    assert_eq!(
        report.items.iter().filter(|i| i.state == "closed").count(),
        1
    );
    assert_eq!(*fake.closed.lock().unwrap(), ["orphan"]);
    assert!(app.binding("t").unwrap().target.is_some());
    app.execute_session_cleanup(&batch.id).unwrap();
    assert_eq!(fake.closed.lock().unwrap().len(), 1);
}
#[test]
fn binding_after_preview_protects_the_target() {
    let (app, fake) = fixture();
    let t = target("new-binding");
    fake.targets.lock().unwrap().push(t.clone());
    let batch = app.preview_session_cleanup().unwrap();
    bind(&app, &t);
    let report = app.execute_session_cleanup(&batch.id).unwrap();
    assert_eq!(report.items[0].state, "skipped");
    assert!(fake.closed.lock().unwrap().is_empty());
}
#[test]
fn deleted_resource_is_closed_and_remains_queryable_until_confirmed() {
    let (app, fake) = fixture();
    let t = target("deleted");
    bind(&app, &t);
    fake.targets.lock().unwrap().push(t);
    crate::tasks::delete(&mut app.db.lock().unwrap(), "t").unwrap();
    let resource = crate::session_resources::list(&app.db.lock().unwrap())
        .unwrap()
        .remove(0);
    let report = app.release_deleted_resource(resource.id).unwrap();
    assert_eq!(report.items[0].state, "closed");
    assert!(
        crate::session_resources::list(&app.db.lock().unwrap())
            .unwrap()
            .is_empty()
    );
}
#[test]
fn lost_close_response_is_verified_without_a_second_close() {
    let (app, fake) = fixture();
    fake.targets.lock().unwrap().push(target("lost"));
    *fake.uncertain.lock().unwrap() = true;
    let batch = app.preview_session_cleanup().unwrap();
    assert_eq!(
        app.execute_session_cleanup(&batch.id).unwrap().items[0].state,
        "unknown"
    );
    crate::db::recover_interrupted(&app.db.lock().unwrap()).unwrap();
    assert_eq!(
        app.cleanup_result(&batch.id, true).unwrap().items[0].state,
        "closed"
    );
    assert_eq!(fake.closed.lock().unwrap().len(), 1);
}

#[test]
fn historical_unknown_does_not_protect_todo_and_is_retired_without_faking_delivery() {
    let (app, fake) = fixture();
    let t = target("old");
    bind(&app, &t);
    fake.targets.lock().unwrap().push(t.clone());
    {
        let db = app.db.lock().unwrap();
        db.execute("UPDATE tasks SET status='todo'", []).unwrap();
        db.execute("INSERT INTO dispatch_operations(id,task_id,workspace,target_json,prompt,created_at,state,had_binding) VALUES('old-op','t','w1',?1,'original prompt','now','unknown',1)",[serde_json::to_string(&t).unwrap()]).unwrap();
    }
    let batch = app.preview_session_cleanup().unwrap();
    assert_eq!(batch.items[0].state, "ready");
    assert_eq!(
        app.execute_session_cleanup(&batch.id).unwrap().items[0].state,
        "closed"
    );
    let db = app.db.lock().unwrap();
    let (owner, state, prompt): (String, String, String) = db
        .query_row(
            "SELECT task_id,state,prompt FROM dispatch_operations WHERE id='old-op'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert!(owner.starts_with("retired:cleanup:"));
    assert_eq!(state, "unknown");
    assert_eq!(prompt, "original prompt");
    assert!(crate::workflow::binding(&db, "t").unwrap().target.is_none());
    assert_eq!(crate::tasks::list(&db, "p").unwrap().len(), 1);
}
#[test]
fn active_writer_and_expired_preview_never_close() {
    let (app, fake) = fixture();
    let t = target("active");
    bind(&app, &t);
    fake.targets.lock().unwrap().push(t);
    app.db
        .lock()
        .unwrap()
        .execute("UPDATE tasks SET status='todo'", [])
        .unwrap();
    let activity = crate::session_activity::enter(&app, "t");
    let batch = app.preview_session_cleanup().unwrap();
    assert_eq!(batch.items[0].error, "session_cleanup.busy");
    drop(activity);
    let batch = app.preview_session_cleanup().unwrap();
    app.db
        .lock()
        .unwrap()
        .execute(
            "UPDATE session_cleanup_batches SET created_at=0 WHERE id=?1",
            [&batch.id],
        )
        .unwrap();
    assert_eq!(
        app.execute_session_cleanup(&batch.id).unwrap_err(),
        "cleanup.stale"
    );
    assert!(fake.closed.lock().unwrap().is_empty());
}
#[test]
fn deleted_card_worker_does_not_close_discovered_or_reset_resources() {
    let (app, fake) = fixture();
    let t = target("delete-worker");
    bind(&app, &t);
    fake.targets.lock().unwrap().extend([t, target("orphan")]);
    app.preview_session_cleanup().unwrap();
    crate::tasks::delete(&mut app.db.lock().unwrap(), "t").unwrap();
    crate::session_resources::drain_deleted(&app).unwrap();
    crate::session_resources::drain_deleted(&app).unwrap();
    assert_eq!(*fake.closed.lock().unwrap(), ["delete-worker"]);
    assert_eq!(fake.targets.lock().unwrap().len(), 1);
}
#[test]
fn replaced_identity_and_disabled_machine_are_reported() {
    let (app, fake) = fixture();
    fake.targets.lock().unwrap().push(target("replaced"));
    app.db.lock().unwrap().execute("INSERT INTO machines(id,name,kind,session,disabled) VALUES('disabled','Disabled SSH','ssh','default',1)",[]).unwrap();
    let batch = app.preview_session_cleanup().unwrap();
    assert_eq!(batch.scopes[0].error, "machine.disabled");
    fake.targets.lock().unwrap()[0].process_id = "new-pid".into();
    let result = app.execute_session_cleanup(&batch.id).unwrap();
    assert_eq!(result.items[0].state, "skipped");
    assert!(fake.closed.lock().unwrap().is_empty());
}
#[test]
fn interrupted_batch_survives_real_sqlite_reopen_without_replaying_close() {
    let (mut app, fake) = fixture();
    fake.targets.lock().unwrap().push(target("reopen"));
    *fake.uncertain.lock().unwrap() = true;
    let path = std::env::temp_dir().join(format!("kanban-cleanup-{}.db", uuid::Uuid::new_v4()));
    app.db
        .lock()
        .unwrap()
        .execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])
        .unwrap();
    app.db = Arc::new(Mutex::new(Connection::open(&path).unwrap()));
    crate::db::init(&app.db.lock().unwrap()).unwrap();
    let batch = app.preview_session_cleanup().unwrap();
    app.execute_session_cleanup(&batch.id).unwrap();
    drop(app);
    let db = Connection::open(&path).unwrap();
    crate::db::init(&db).unwrap();
    crate::db::recover_interrupted(&db).unwrap();
    let app = Workflow {
        db: Arc::new(Mutex::new(db)),
        herdr: fake.clone(),
    };
    assert_eq!(
        app.cleanup_result(&batch.id, true).unwrap().items[0].state,
        "closed"
    );
    assert_eq!(fake.closed.lock().unwrap().len(), 1);
    drop(app);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn agent_in_a_workspace_terminal_is_cleaned_and_locator_is_reset() {
    let (app, fake) = fixture();
    let t = target("ordinary-agent");
    fake.targets.lock().unwrap().push(t.clone());
    app.db.lock().unwrap().execute("INSERT INTO workspace_terminals VALUES('local','default','w1','terminal-operation','.',?1,'ready')",[serde_json::to_string(&t).unwrap()]).unwrap();
    let batch = app.preview_session_cleanup().unwrap();
    assert_eq!(
        app.execute_session_cleanup(&batch.id).unwrap().items[0].state,
        "closed"
    );
    let state: String = app
        .db
        .lock()
        .unwrap()
        .query_row("SELECT state FROM workspace_terminals", [], |r| r.get(0))
        .unwrap();
    assert_eq!(state, "failed");
}

#[test]
fn cleanup_claim_blocks_task_writes_until_result_is_verified(){
 let (app,fake)=fixture();let t=target("claim");bind(&app,&t);fake.targets.lock().unwrap().push(t);
 app.db.lock().unwrap().execute("UPDATE tasks SET status='todo'",[]).unwrap();
 assert!(crate::idle_cleanup::ensure_task_unclaimed(&app.db.lock().unwrap(),"t").is_ok());
 *fake.uncertain.lock().unwrap()=true;let batch=app.preview_session_cleanup().unwrap();app.execute_session_cleanup(&batch.id).unwrap();
 assert_eq!(crate::idle_cleanup::ensure_task_unclaimed(&app.db.lock().unwrap(),"t").unwrap_err(),"cleanup.pending");
 app.cleanup_result(&batch.id,true).unwrap();assert!(crate::idle_cleanup::ensure_task_unclaimed(&app.db.lock().unwrap(),"t").is_ok());
}

#[test]
fn unknown_live_target_requires_explicit_verified_retry(){
 let (app,fake)=fixture();fake.targets.lock().unwrap().push(target("still-alive"));*fake.uncertain.lock().unwrap()=true;*fake.retain.lock().unwrap()=true;
 let batch=app.preview_session_cleanup().unwrap();app.execute_session_cleanup(&batch.id).unwrap();
 assert_eq!(app.cleanup_result(&batch.id,true).unwrap().items[0].state,"unknown");assert_eq!(fake.closed.lock().unwrap().len(),1);
 *fake.uncertain.lock().unwrap()=false;*fake.retain.lock().unwrap()=false;
 let result=app.retry_session_cleanup(&batch.id).unwrap();assert_eq!(result.items[0].state,"closed");assert_eq!(fake.closed.lock().unwrap().len(),2);
 assert_eq!(result.items[0].warning,"session_cleanup.history_unavailable");
}
#[test]
fn historical_idle_cleanup_unknown_is_not_permanent_protection(){
 let (app,fake)=fixture();let t=target("legacy-cleanup");fake.targets.lock().unwrap().push(t.clone());
 let c=serde_json::json!({"pane":{"target":t,"tab_id":"tab","cwd":"."},"owner":null});
 {let db=app.db.lock().unwrap();db.execute("INSERT INTO idle_cleanup_batches VALUES('legacy','p','local','default','w1','done','now')",[]).unwrap();db.execute("INSERT INTO idle_cleanup_items VALUES('legacy',0,?1,'unknown','lost')",[c.to_string()]).unwrap();}
 let batch=app.preview_session_cleanup().unwrap();assert_eq!(batch.items[0].state,"ready");assert_eq!(app.execute_session_cleanup(&batch.id).unwrap().items[0].state,"closed");
 let db=app.db.lock().unwrap();assert!(crate::idle_cleanup::ensure_unclaimed(&db,&t,None).is_ok());let state:String=db.query_row("SELECT state FROM idle_cleanup_items WHERE batch_id='legacy'",[],|r|r.get(0)).unwrap();assert_eq!(state,"unknown");
}
