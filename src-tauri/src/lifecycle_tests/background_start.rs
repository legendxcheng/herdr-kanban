use super::*;

#[test]
fn ssh_configured_start_and_resume_do_not_require_a_catalog() {
    let (app, fake) = fixture_on(true);
    let app = Arc::new(app);
    fake.0.lock().unwrap().catalog_error = true;
    let settings = crate::codex_settings::Settings {
        model: Some("test-model".into()),
        effort: Some("high".into()),
    };
    crate::codex_settings::save(&app.db.lock().unwrap(), "a", &settings, false).unwrap();
    set_prompt(&app, "a", "send once");
    app.submit_start("a", &op(), Arc::new(|_| {})).unwrap();
    assert_eq!(await_start(&app, "a").state.as_deref(), Some("succeeded"));
    let session = saved(&app, "a");
    assert!(session.is_some());
    app.complete_task("a", &op()).unwrap();
    app.submit_start("a", &op(), Arc::new(|_| {})).unwrap();
    assert_eq!(await_start(&app, "a").state.as_deref(), Some("succeeded"));
    assert_eq!(saved(&app, "a"), session);
    let state = fake.0.lock().unwrap();
    assert!(state.catalog_directories.is_empty());
    assert_eq!(state.settings, vec![settings.clone(), settings]);
    assert_eq!(state.sent, vec!["send once"]);
}

#[test]
fn ssh_catalog_bypass_preserves_start_failure_without_sending_or_fallback() {
    let (app, fake) = fixture_on(true);
    let settings = crate::codex_settings::Settings {
        model: Some("test-model".into()),
        effort: None,
    };
    crate::codex_settings::save(&app.db.lock().unwrap(), "a", &settings, false).unwrap();
    {
        let mut state = fake.0.lock().unwrap();
        state.catalog_error = true;
        state.prepare_error = true;
    }
    assert_eq!(
        app.start_task("a", "never send", &op()).unwrap_err(),
        "timeout"
    );
    assert_eq!(
        view(&app.db.lock().unwrap(), "a").unwrap().state.as_deref(),
        Some("unknown")
    );
    let state = fake.0.lock().unwrap();
    assert!(state.catalog_directories.is_empty());
    assert_eq!(state.settings, vec![settings]);
    assert!(state.sent.is_empty());
    assert_eq!(state.created, 1);
}

#[test]
fn ssh_catalog_bypass_still_rejects_malformed_settings_before_pane_creation() {
    let (app, fake) = fixture_on(true);
    app.db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO task_codex_settings(task_id,model) VALUES ('a','bad model')",
            [],
        )
        .unwrap();
    assert_eq!(
        app.start_task("a", "never send", &op()).unwrap_err(),
        "model.invalid_settings"
    );
    let state = fake.0.lock().unwrap();
    assert!(state.catalog_directories.is_empty());
    assert_eq!(state.created, 0);
    assert!(state.sent.is_empty());
}

#[test]
fn catalog_cache_is_directory_scoped_and_offline_choices_cannot_start_unvalidated() {
    let (app, fake) = fixture();
    app.db
        .lock()
        .unwrap()
        .execute("UPDATE projects SET root_path='other' WHERE id='p2'", [])
        .unwrap();
    assert!(!app.model_catalog("p1", false).unwrap().cached);
    fake.0.lock().unwrap().catalog_error = true;
    assert!(app.model_catalog("p1", false).unwrap().cached);
    let offline = app.model_catalog("p1", true).unwrap();
    assert!(offline.cached);
    assert_eq!(offline.error.as_deref(), Some("model.catalog_unavailable"));
    assert!(app.model_catalog("p2", false).is_err());
    assert_eq!(
        fake.0.lock().unwrap().catalog_directories,
        vec![".", ".", "other"]
    );
    let selected = crate::codex_settings::Settings {
        model: Some("test-model".into()),
        effort: Some("high".into()),
    };
    crate::codex_settings::save(&app.db.lock().unwrap(), "a", &selected, false).unwrap();
    assert_eq!(
        app.start_task("a", "do not send", &op()).unwrap_err(),
        "model.catalog_unavailable"
    );
    assert_eq!(fake.0.lock().unwrap().created, 0);
    assert!(fake.0.lock().unwrap().sent.is_empty());
    // A known invalid pair is rejected even when the directory is cached.
    let invalid = crate::codex_settings::Settings {
        effort: Some("ultra".into()),
        ..selected
    };
    assert_eq!(
        crate::codex_settings::save(&app.db.lock().unwrap(), "a", &invalid, false).unwrap_err(),
        "model.effort_unavailable"
    );
}

#[test]
fn configured_task_starts_and_resumes_with_same_settings_without_resending() {
    let (app, fake) = fixture();
    let settings = crate::codex_settings::Settings {
        model: Some("test-model".into()),
        effort: Some("high".into()),
    };
    crate::codex_settings::save(&app.db.lock().unwrap(), "a", &settings, false).unwrap();
    app.start_task("a", "first prompt", &op()).unwrap();
    assert_eq!(fake.0.lock().unwrap().settings, vec![settings.clone()]);
    let session = saved(&app, "a");
    app.complete_task("a", &op()).unwrap();
    app.start_task("a", "must not resend", &op()).unwrap();
    assert_eq!(saved(&app, "a"), session);
    assert_eq!(
        fake.0.lock().unwrap().settings,
        vec![settings.clone(), settings]
    );
    assert_eq!(fake.0.lock().unwrap().sent, vec!["first prompt"]);
}

#[test]
fn invalid_model_fails_before_pane_allocation_and_can_be_corrected() {
    let (app, fake) = fixture();
    let invalid = crate::codex_settings::Settings {
        model: Some("missing-model".into()),
        effort: None,
    };
    crate::codex_settings::save(&app.db.lock().unwrap(), "a", &invalid, false).unwrap();
    assert_eq!(
        app.start_task("a", "first", &op()).unwrap_err(),
        "model.unavailable"
    );
    assert_eq!(fake.0.lock().unwrap().created, 0);
    assert!(fake.0.lock().unwrap().sent.is_empty());
    assert!(crate::tasks::list(&app.db.lock().unwrap(), "p1").unwrap()[0].codex_settings_editable);
    crate::codex_settings::save(&app.db.lock().unwrap(), "a", &Default::default(), false).unwrap();
    app.start_task("a", "first", &op()).unwrap();
}

#[test]
fn accepted_configuration_is_frozen_and_unresolved_start_cannot_be_edited() {
    let (app, fake) = fixture();
    let app = Arc::new(app);
    set_prompt(&app, "a", "once");
    let selected = crate::codex_settings::Settings {
        model: Some("test-model".into()),
        effort: Some("high".into()),
    };
    crate::codex_settings::save(&app.db.lock().unwrap(), "a", &selected, false).unwrap();
    let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    fake.0.lock().unwrap().prepare_gate = Some(gate.clone());
    let operation = op();
    app.submit_start("a", &operation, Arc::new(|_| {})).unwrap();
    assert_eq!(
        crate::codex_settings::save(&app.db.lock().unwrap(), "a", &Default::default(), false)
            .unwrap_err(),
        "model.settings_locked"
    );
    app.db
        .lock()
        .unwrap()
        .execute(
            "UPDATE task_codex_settings SET effort='low' WHERE task_id='a'",
            [],
        )
        .unwrap();
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    assert_eq!(await_start(&app, "a").state.as_deref(), Some("succeeded"));
    assert_eq!(fake.0.lock().unwrap().settings, vec![selected]);
    app.submit_start("a", &operation, Arc::new(|_| {})).unwrap();
    assert_eq!(fake.0.lock().unwrap().sent, vec!["once"]);
}

#[test]
fn explicit_configuration_cannot_be_dispatched_to_an_existing_agent() {
    let (app, fake) = fixture();
    fake.0
        .lock()
        .unwrap()
        .panes
        .insert("existing".into(), (target("existing"), None));
    let selected = crate::codex_settings::Settings {
        model: Some("test-model".into()),
        effort: None,
    };
    crate::codex_settings::save(&app.db.lock().unwrap(), "a", &selected, false).unwrap();
    assert_eq!(
        app.dispatch("a", "existing", "must not send", &op())
            .unwrap_err(),
        "model.automatic_start_required"
    );
    assert!(fake.0.lock().unwrap().sent.is_empty());
}

#[test]
fn configured_task_can_continue_in_its_retained_process_but_not_a_repaired_external_agent() {
    let (app, fake) = fixture();
    let selected = crate::codex_settings::Settings {
        model: Some("test-model".into()),
        effort: Some("high".into()),
    };
    crate::codex_settings::save(&app.db.lock().unwrap(), "a", &selected, false).unwrap();
    app.start_task("a", "first", &op()).unwrap();
    let pane = app.binding("a").unwrap().target.unwrap().pane_id;
    app.db
        .lock()
        .unwrap()
        .execute("UPDATE tasks SET status='todo' WHERE id='a'", [])
        .unwrap();
    app.dispatch("a", &pane, "follow up", &op()).unwrap();
    assert_eq!(fake.0.lock().unwrap().sent, vec!["first", "follow up"]);
    app.complete_task("a", &op()).unwrap();
    fake.0
        .lock()
        .unwrap()
        .panes
        .insert("external".into(), (target("external"), None));
    app.repair("a", "external").unwrap();
    assert_eq!(
        app.dispatch("a", "external", "do not send", &op())
            .unwrap_err(),
        "model.automatic_start_required"
    );
    assert_eq!(fake.0.lock().unwrap().sent.len(), 2);
}

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
        "inventory",
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
            if phase == "inventory" { 0 } else { 1 },
            "{phase}"
        );
        assert_eq!(
            s.prepared.len(),
            if ["inventory", "created", "preparing"].contains(&phase) {
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
