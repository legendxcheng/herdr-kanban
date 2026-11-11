use super::*;

#[test]
fn viewing_distinguishes_delayed_session_identity_from_a_different_conversation() {
    for remote in [false, true] {
        let (app, fake) = fixture_on(remote);
        app.start_task("a", "original", &op()).unwrap();
        let permission = app.view_task("a").unwrap();
        let original = saved(&app, "a");
        fake.0.lock().unwrap().missing_id = true;
        assert_eq!(app.view_task("a").unwrap_err(), "flow.session_unverified");
        assert_eq!(
            app.validate_view("a", &permission).unwrap_err(),
            "flow.session_unverified"
        );
        fake.0.lock().unwrap().missing_id = false;
        app.view_task("a").unwrap();
        fake.0.lock().unwrap().panes.values_mut().next().unwrap().1 = Some(op());
        assert_eq!(app.view_task("a").unwrap_err(), "flow.session_mismatch");
        assert_eq!(saved(&app, "a"), original);
        assert_eq!(fake.0.lock().unwrap().sent, vec!["original"]);
    }
}

#[test]
fn repair_rejects_wrong_or_unidentified_sessions_before_changing_the_binding() {
    for remote in [false, true] {
        let (app, fake) = fixture_on(remote);
        app.start_task("a", "original", &op()).unwrap();
        let original = app.binding("a").unwrap().target;
        let session = saved(&app, "a");
        let mut candidate = target("replacement");
        candidate.machine_id = app.herdr.machine_id().into();
        for (id, error) in [
            (Some(op()), "flow.session_mismatch"),
            (None, "flow.session_unverified"),
        ] {
            fake.0
                .lock()
                .unwrap()
                .panes
                .insert(candidate.pane_id.clone(), (candidate.clone(), id));
            assert_eq!(app.repair("a", &candidate.pane_id).unwrap_err(), error);
            assert_eq!(app.binding("a").unwrap().target, original);
            assert_eq!(saved(&app, "a"), session);
        }
        fake.0.lock().unwrap().panes.insert(
            candidate.pane_id.clone(),
            (candidate.clone(), session.clone()),
        );
        app.repair("a", &candidate.pane_id).unwrap();
        assert!(app.view_task("a").unwrap().target.same_identity(&candidate));
        assert_eq!(saved(&app, "a"), session);
        let state = fake.0.lock().unwrap();
        assert_eq!(state.sent, vec!["original"]);
        assert!(state.closed.is_empty());
        assert_eq!(state.created, 1);
    }
}

#[test]
fn repair_rechecks_local_session_and_binding_after_remote_observation() {
    for change in ["session", "binding", "epoch"] {
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
            if phase == "session" {
                // Also proves external observations do not hold the database mutex.
                let db = db
                    .try_lock()
                    .expect("repair held SQLite across a remote call");
                match change {
                    "session" => {
                        db.execute(
                            "UPDATE task_sessions SET codex_session_id=?1 WHERE task_id='a'",
                            [op()],
                        )
                        .unwrap();
                    }
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
fn repair_rejects_process_replacement_during_session_observation() {
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
        if phase == "session" {
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
    let session = cli
        .view_identity(&live)
        .unwrap()
        .expect("explicit live conversation");
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
    assert_eq!(
        app.repair("probe", &pane).unwrap_err(),
        "flow.session_mismatch"
    );
    assert!(app.binding("probe").unwrap().target.is_none());
    app.db
        .lock()
        .unwrap()
        .execute(
            "UPDATE task_sessions SET codex_session_id=?1 WHERE task_id='probe'",
            [&session],
        )
        .unwrap();
    app.repair("probe", &pane).unwrap();
    let permit = app.view_task("probe").unwrap();
    app.validate_view("probe", &permit).unwrap();
    assert!(permit.target.same_identity(&live));
    assert_eq!(saved(&app, "probe"), Some(session.clone()));
    let count: i64 = app
        .db
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM dispatch_operations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    eprintln!(
        "Read-only remote repair/view passed: pane={pane}, session={session}; rejected wrong session; no dispatch"
    );
}
