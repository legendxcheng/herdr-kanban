use super::*;

#[test]
fn repair_rechecks_binding_and_epoch_after_target_observation() {
    for change in ["binding", "epoch"] {
        let (app, fake) = fixture();
        app.start_task("a", "original", &op()).unwrap();
        let session = saved(&app, "a");
        let candidate = target("replacement");
        fake.0
            .lock()
            .unwrap()
            .panes
            .insert(candidate.pane_id.clone(), (candidate.clone(), session));
        let db = app.db.clone();
        fake.0.lock().unwrap().observe = Some(Arc::new(move |phase| {
            if phase == "targets" {
                // Also proves external observations do not hold the database mutex.
                let db = db
                    .try_lock()
                    .expect("repair held SQLite across a remote call");
                match change {
                    "binding" => {
                        db.execute(
                            "UPDATE task_bindings SET state='reserved' WHERE task_id='a'",
                            [],
                        )
                        .unwrap();
                    }
                    _ => {
                        db.execute(
                            "UPDATE tasks SET execution_epoch=execution_epoch+1 WHERE id='a'",
                            [],
                        )
                        .unwrap();
                    }
                }
            }
        }));
        assert!(app.repair("a", &candidate.pane_id).is_err(), "{change}");
        assert_ne!(
            app.binding("a").unwrap().target.unwrap().pane_id,
            candidate.pane_id
        );
    }
}

#[test]
fn repair_rejects_process_replacement_after_target_selection() {
    let (app, fake) = fixture();
    app.start_task("a", "original", &op()).unwrap();
    let old = app.binding("a").unwrap().target;
    let candidate = target("replacement");
    fake.0.lock().unwrap().panes.insert(
        candidate.pane_id.clone(),
        (candidate.clone(), saved(&app, "a")),
    );
    let weak = Arc::downgrade(&fake);
    fake.0.lock().unwrap().observe = Some(Arc::new(move |phase| {
        if phase == "targets" {
            weak.upgrade()
                .unwrap()
                .0
                .lock()
                .unwrap()
                .panes
                .get_mut("replacement")
                .unwrap()
                .0
                .process_id = "another-process".into();
        }
    }));
    assert_eq!(
        app.repair("a", &candidate.pane_id).unwrap_err(),
        "binding.identity_changed"
    );
    assert_eq!(app.binding("a").unwrap().target, old);
}

#[test]
#[ignore = "read-only Herdr queries with an explicit source database, machine, workspace and pane; writes only an in-memory fixture"]
fn live_readonly_binding_recovery() {
    let source = rusqlite::Connection::open_with_flags(
        std::env::var("KANBAN_READONLY_DB").unwrap(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let mut machine =
        crate::machines::get(&source, &std::env::var("KANBAN_READONLY_MACHINE").unwrap()).unwrap();
    // An explicit, already installed helper can be selected without updating personal settings.
    if let Ok(name) = std::env::var("KANBAN_READONLY_HELPER_NAME") {
        machine.helper_name = Some(name);
    }
    let workspace = std::env::var("KANBAN_READONLY_WORKSPACE").unwrap();
    let pane = std::env::var("KANBAN_READONLY_PANE").unwrap();
    let mut cli = crate::herdr::Cli::from_env();
    cli.session = machine.session.clone();
    cli.remote = Some(machine.clone());
    let live = cli
        .targets(&workspace)
        .unwrap()
        .into_iter()
        .find(|t| t.pane_id == pane)
        .unwrap();
    let observed = cli.session_id(&live).unwrap();
    if std::env::var_os("KANBAN_READONLY_EXPECT_MISSING_SESSION").is_some() {
        assert!(
            observed.is_none(),
            "this acceptance requires the reported missing-ID case"
        );
    }
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    db.execute(
        "INSERT INTO machines(id,name,kind,session) VALUES (?1,'Read only probe','ssh',?2)",
        params![machine.id, machine.session],
    )
    .unwrap();
    db.execute("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at,herdr_session,workspace_id,sync_status,machine_id) VALUES ('probe','Probe','.','probe','now',?1,?2,'ready',?3)", params![machine.session,workspace,machine.id]).unwrap();
    db.execute_batch("INSERT INTO tasks(id,project_id,name,status,created_at,updated_at) VALUES ('probe','probe','Probe','in_progress','now','now');").unwrap();
    let wrong = op();
    db.execute("INSERT INTO task_sessions(task_id,codex_session_id,cwd,updated_at) VALUES ('probe',?1,'.','now')", [&wrong]).unwrap();
    let app = Workflow {
        db: Arc::new(Mutex::new(db)),
        herdr: Arc::new(cli),
    };
    app.repair("probe", &pane).unwrap();
    let permit = app.view_task("probe").unwrap();
    app.validate_view("probe", &permit).unwrap();
    assert!(permit.target.same_identity(&live));
    assert_eq!(saved(&app, "probe"), None);
    let count: i64 = app
        .db
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM dispatch_operations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    eprintln!(
        "Manual remote binding/view passed: pane={pane}, reported_session={observed:?}; stale resume pointer cleared; no dispatch"
    );
}
