use super::*;

#[test]
fn accepts_and_persists_start_without_waiting_for_herdr() {
    let (app, fake) = fixture();
    let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    fake.0.lock().unwrap().prepare_gate = Some(gate.clone());
    app.db
        .lock()
        .unwrap()
        .execute(
            "UPDATE tasks SET prompt_json=?1 WHERE id='a'",
            [r#"{"type":"doc","content":[{"type":"text","text":"snapshot"}]}"#],
        )
        .unwrap();
    let app = Arc::new(app);
    let operation = op();
    let accepted = app.submit_start("a", &operation, Arc::new(|_| {})).unwrap();
    let tasks = crate::tasks::list(&app.db.lock().unwrap(), "p1").unwrap();
    let duplicate = app.submit_start("a", &operation, Arc::new(|_| {})).unwrap();
    let other = crate::tasks::update(
        &app.db.lock().unwrap(),
        "b",
        "still editable",
        "{}",
        crate::models::TaskPriority::Normal,
    );
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    assert_eq!(accepted.operation_id.as_deref(), Some(operation.as_str()));
    assert_eq!(duplicate.operation_id, accepted.operation_id);
    assert_eq!(tasks[0].status, crate::models::TaskStatus::Todo);
    assert_eq!(
        tasks[0].flow.as_ref().unwrap().requested_status.as_deref(),
        Some("in_progress")
    );
    other.unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while view(&app.db.lock().unwrap(), "a").unwrap().state.as_deref() == Some("pending") {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(status(&app, "a"), "in_progress");
    assert_eq!(fake.0.lock().unwrap().sent, vec!["snapshot"]);
}

#[test]
fn asynchronous_start_uses_accepted_content_and_directory_then_resumes_without_reading_prompt() {
    let (app, fake) = fixture();
    let app = Arc::new(app);
    set_prompt(&app, "a", "accepted content");
    let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    fake.0.lock().unwrap().prepare_gate = Some(gate.clone());
    app.submit_start("a", &op(), Arc::new(|_| {})).unwrap();
    set_prompt(&app, "a", "later edit");
    app.db
        .lock()
        .unwrap()
        .execute("UPDATE projects SET root_path='changed' WHERE id='p1'", [])
        .unwrap();
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    assert_eq!(await_start(&app, "a").state.as_deref(), Some("succeeded"));
    assert_eq!(fake.0.lock().unwrap().sent, vec!["accepted content"]);
    assert_eq!(fake.0.lock().unwrap().directories, vec!["."]);
    let session = saved(&app, "a");
    app.complete_task("a", &op()).unwrap();
    app.db
        .lock()
        .unwrap()
        .execute(
            "UPDATE tasks SET prompt_json='invalid json' WHERE id='a'",
            [],
        )
        .unwrap();
    app.submit_start("a", &op(), Arc::new(|_| {})).unwrap();
    assert_eq!(await_start(&app, "a").state.as_deref(), Some("succeeded"));
    assert_eq!(saved(&app, "a"), session);
    assert_eq!(fake.0.lock().unwrap().sent, vec!["accepted content"]);
}

#[test]
fn invalid_content_fails_locally_and_next_valid_request_can_start() {
    let (app, fake) = fixture();
    let app = Arc::new(app);
    app.db
        .lock()
        .unwrap()
        .execute(
            "UPDATE tasks SET prompt_json='invalid json' WHERE id='a'",
            [],
        )
        .unwrap();
    let id = op();
    app.submit_start("a", &id, Arc::new(|_| {})).unwrap();
    let failed = await_start(&app, "a");
    assert_eq!(failed.state.as_deref(), Some("failed"));
    assert!(failed.requested_status.is_none());
    assert_eq!(status(&app, "a"), "todo");
    assert_eq!(fake.0.lock().unwrap().created, 0);
    set_prompt(&app, "a", "fixed");
    app.db
        .lock()
        .unwrap()
        .execute(
            "UPDATE projects SET root_path='updated-directory' WHERE id='p1'",
            [],
        )
        .unwrap();
    // Retrying the old ID reports the original result instead of sending changed content.
    assert_eq!(
        app.submit_start("a", &id, Arc::new(|_| {}))
            .unwrap()
            .state
            .as_deref(),
        Some("failed")
    );
    app.submit_start("a", &op(), Arc::new(|_| {})).unwrap();
    assert_eq!(await_start(&app, "a").state.as_deref(), Some("succeeded"));
    assert_eq!(fake.0.lock().unwrap().sent, vec!["fixed"]);
    assert_eq!(
        fake.0.lock().unwrap().directories,
        vec!["updated-directory"]
    );
}

#[test]
fn deletion_during_external_start_steps_prevents_later_work_and_resurrection() {
    for phase in [
        "available",
        "created",
        "preparing",
        "prepared",
        "session",
        "sent",
    ] {
        let (app, fake) = fixture();
        let db = app.db.clone();
        fake.0.lock().unwrap().observe = Some(Arc::new(move |observed| {
            if observed == phase {
                crate::tasks::delete(&mut db.lock().unwrap(), "a").unwrap();
            }
        }));
        assert!(app.start_task("a", "A", &op()).is_err());
        assert!(
            crate::tasks::list(&app.db.lock().unwrap(), "p1")
                .unwrap()
                .is_empty()
        );
        assert!(saved(&app, "a").is_none());
        let s = fake.0.lock().unwrap();
        assert_eq!(
            s.created,
            if phase == "available" { 0 } else { 1 },
            "{phase}"
        );
        assert_eq!(
            s.prepared.len(),
            if ["available", "created", "preparing"].contains(&phase) {
                0
            } else {
                1
            },
            "{phase}"
        );
        assert_eq!(s.sent.len(), if phase == "sent" { 1 } else { 0 }, "{phase}");
        assert!(s.closed.is_empty());
        drop(s);
        // A late worker must not rewrite the evidence left at deletion.
        assert_eq!(
            view(&app.db.lock().unwrap(), "a").unwrap().state.as_deref(),
            Some("pending"),
            "{phase}"
        );
        fake.0.lock().unwrap().observe = None;
        app.start_task("b", "B", &op()).unwrap();
        assert_eq!(status(&app, "b"), "in_progress");
    }
}

#[test]
fn deleted_background_start_cannot_be_recovered_or_resubmitted() {
    let (app, fake) = fixture();
    let app = Arc::new(app);
    set_prompt(&app, "a", "A");
    let db = app.db.clone();
    fake.0.lock().unwrap().observe = Some(Arc::new(move |phase| {
        if phase == "preparing" {
            crate::tasks::delete(&mut db.lock().unwrap(), "a").unwrap();
        }
    }));
    let (tx, rx) = std::sync::mpsc::channel();
    let db = app.db.clone();
    let id = op();
    app.submit_start(
        "a",
        &id,
        Arc::new(move |_| {
            if crate::tasks::ensure_active(&db.lock().unwrap(), "a").is_err() {
                let _ = tx.send(());
            }
        }),
    )
    .unwrap();
    rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap();
    assert_eq!(
        view(&app.db.lock().unwrap(), "a").unwrap().state.as_deref(),
        Some("pending")
    );
    crate::db::recover_interrupted(&app.db.lock().unwrap()).unwrap();
    assert_eq!(
        view(&app.db.lock().unwrap(), "a").unwrap().state.as_deref(),
        Some("pending")
    );
    assert!(app.submit_start("a", &id, Arc::new(|_| {})).is_err());
    assert!(app.recover_lifecycle("a", &id).is_err());
    assert!(fake.0.lock().unwrap().prepared.is_empty());
    assert!(fake.0.lock().unwrap().sent.is_empty());
}

#[test]
fn asynchronous_unknown_start_preserves_ownership_and_does_not_block_other_tasks() {
    let (app, fake) = fixture();
    let app = Arc::new(app);
    set_prompt(&app, "a", "first");
    set_prompt(&app, "b", "second");
    fake.0.lock().unwrap().prepare_error = true;
    let id = op();
    app.submit_start("a", &id, Arc::new(|_| {})).unwrap();
    let unknown = await_start(&app, "a");
    assert_eq!(unknown.state.as_deref(), Some("unknown"));
    assert_eq!(unknown.requested_status.as_deref(), Some("in_progress"));
    assert!(app.submit_start("a", &op(), Arc::new(|_| {})).is_err());
    assert_eq!(
        app.submit_start("a", &id, Arc::new(|_| {}))
            .unwrap()
            .state
            .as_deref(),
        Some("unknown")
    );
    fake.0.lock().unwrap().prepare_error = false;
    app.submit_start("b", &op(), Arc::new(|_| {})).unwrap();
    assert_eq!(await_start(&app, "b").state.as_deref(), Some("succeeded"));
    assert_eq!(fake.0.lock().unwrap().sent, vec!["second"]);
    let pane = app.binding("a").unwrap().target.unwrap().pane_id;
    fake.0.lock().unwrap().panes.remove(&pane);
    app.recover_lifecycle("a", &id).unwrap();
    app.submit_start("a", &op(), Arc::new(|_| {})).unwrap();
    assert_eq!(await_start(&app, "a").state.as_deref(), Some("succeeded"));
    assert_eq!(fake.0.lock().unwrap().sent, vec!["second", "first"]);
}

#[test]
fn background_operation_survives_database_reopen_without_replay() {
    let (app, fake) = fixture();
    let app = Arc::new(app);
    set_prompt(&app, "a", "only once");
    let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    fake.0.lock().unwrap().prepare_gate = Some(gate.clone());
    let id = op();
    app.submit_start("a", &id, Arc::new(|_| {})).unwrap();
    let path = std::env::temp_dir().join(format!("kanban-background-{}.db", op()));
    app.db
        .lock()
        .unwrap()
        .execute("VACUUM main INTO ?1", [path.to_str().unwrap()])
        .unwrap();
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    await_start(&app, "a");
    drop(app);
    let db = Connection::open(&path).unwrap();
    crate::db::init(&db).unwrap();
    crate::db::recover_interrupted(&db).unwrap();
    let app = Arc::new(Workflow {
        db: Arc::new(Mutex::new(db)),
        herdr: fake.clone(),
    });
    assert_eq!(
        app.submit_start("a", &id, Arc::new(|_| {}))
            .unwrap()
            .state
            .as_deref(),
        Some("unknown")
    );
    let tasks = crate::tasks::list(&app.db.lock().unwrap(), "p1").unwrap();
    assert_eq!(
        tasks[0].flow.as_ref().unwrap().requested_status.as_deref(),
        Some("in_progress")
    );
    assert_eq!(fake.0.lock().unwrap().sent, vec!["only once"]);
    assert!(app.submit_start("a", &op(), Arc::new(|_| {})).is_err());
    drop(app);
    std::fs::remove_file(path).unwrap();
}
