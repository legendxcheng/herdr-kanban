use super::*;

#[test]
fn legacy_proven_unsent_survives_sqlite_file_reopen_without_automatic_input() {
    let (app, fake) = fixture();
    fake.0.lock().unwrap().before_input_error = Some("flow.not_idle".into());
    let id = op();
    app.start_task("a", "frozen once", &id).unwrap_err();
    let path =
        std::env::temp_dir().join(format!("kanban-first-reopen-{}.db", uuid::Uuid::new_v4()));
    {
        let db = app.db.lock().unwrap();
        db.execute(
            "UPDATE dispatch_operations SET evidence='flow.not_idle'",
            [],
        )
        .unwrap();
        db.execute("VACUUM INTO ?1", [path.to_str().unwrap()])
            .unwrap();
    }
    drop(app);
    let conn = Connection::open(&path).unwrap();
    crate::db::init(&conn).unwrap();
    crate::db::recover_interrupted(&conn).unwrap();
    let app = Workflow {
        db: Arc::new(Mutex::new(conn)),
        herdr: fake.clone(),
    };
    assert!(
        view(&app.db.lock().unwrap(), "a")
            .unwrap()
            .can_continue_first_prompt
    );
    assert!(fake.0.lock().unwrap().sent.is_empty());
    fake.0.lock().unwrap().before_input_error = None;
    app.continue_first_prompt("a", &id, 0).unwrap();
    assert_eq!(fake.0.lock().unwrap().sent, ["frozen once"]);
    drop(app);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn newly_discovered_session_is_pinned_before_continuation_notification() {
    let (app, fake) = fixture();
    fake.0.lock().unwrap().before_input_error = Some("flow.not_idle".into());
    fake.0.lock().unwrap().missing_id = true;
    let id = op();
    app.start_task("a", "once", &id).unwrap_err();
    fake.0.lock().unwrap().missing_id = false;
    fake.0.lock().unwrap().before_input_error = None;
    let pane = app.binding("a").unwrap().target.unwrap().pane_id;
    let error = app
        .continue_first_prompt_notifying("a", &id, 0, &|_| {
            fake.0.lock().unwrap().panes.get_mut(&pane).unwrap().1 =
                Some(uuid::Uuid::new_v4().to_string());
        })
        .unwrap_err();
    assert_eq!(error, "flow.session_mismatch");
    assert!(fake.0.lock().unwrap().sent.is_empty());
    assert!(
        !view(&app.db.lock().unwrap(), "a")
            .unwrap()
            .can_continue_first_prompt
    );
}

#[test]
fn workflow_start_survives_idle_working_idle_before_first_input() {
    let (app, fake) = fixture();
    {
        let mut state = fake.0.lock().unwrap();
        state.receipt_available = true;
        // prepare_codex returns idle; the actual submit adapter then sees these observations.
        state.first_prompt_states = Some([("working", false), ("unknown", true), ("idle", false)].into_iter().map(|(status,pending)| serde_json::json!({"agent_status":status,"interactive_ready":true,"launch_pending":pending,"state_change_seq":3})).collect());
    }
    let id = op();
    app.start_task("a", "中文\n\n\t$skill", &id).unwrap();
    assert_eq!(status(&app, "a"), "in_progress");
    assert_eq!(
        view(&app.db.lock().unwrap(), "a").unwrap().state.as_deref(),
        Some("succeeded")
    );
    let state = fake.0.lock().unwrap();
    assert_eq!(state.created, 1);
    assert_eq!(state.prepared.len(), 1);
    assert_eq!(state.sent, ["中文\n\n\t$skill"]);
    assert_eq!(
        state
            .input_requests
            .iter()
            .filter(|r| r[1] == "send-text")
            .count(),
        1
    );
    assert_eq!(
        state
            .input_requests
            .iter()
            .filter(|r| r.last().is_some_and(|s| s == "enter"))
            .count(),
        1
    );
}

#[test]
fn session_appearing_after_unsent_start_can_continue_but_known_session_switch_cannot() {
    for known in [false, true] {
        let (app, fake) = fixture();
        fake.0.lock().unwrap().before_input_error = Some("flow.not_idle".into());
        fake.0.lock().unwrap().missing_id = !known;
        let id = op();
        app.start_task("a", "original", &id).unwrap_err();
        fake.0.lock().unwrap().before_input_error = None;
        fake.0.lock().unwrap().missing_id = false;
        if known {
            let target = app.binding("a").unwrap().target.unwrap();
            fake.0
                .lock()
                .unwrap()
                .panes
                .get_mut(&target.pane_id)
                .unwrap()
                .1 = Some(uuid::Uuid::new_v4().to_string());
            assert_eq!(
                app.continue_first_prompt("a", &id, 0).unwrap_err(),
                "flow.session_mismatch"
            );
            assert!(fake.0.lock().unwrap().sent.is_empty());
        } else {
            app.continue_first_prompt("a", &id, 0).unwrap();
            assert_eq!(fake.0.lock().unwrap().sent, ["original"]);
        }
    }
}

#[test]
fn concurrent_continuations_claim_one_durable_attempt() {
    let (app, fake) = fixture();
    fake.0.lock().unwrap().before_input_error = Some("flow.not_idle".into());
    let id = op();
    app.start_task("a", "once", &id).unwrap_err();
    fake.0.lock().unwrap().before_input_error = None;
    let app = Arc::new(app);
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let (app, id, barrier) = (app.clone(), id.clone(), barrier.clone());
            std::thread::spawn(move || {
                barrier.wait();
                app.continue_first_prompt("a", &id, 0).is_ok()
            })
        })
        .collect();
    barrier.wait();
    assert_eq!(
        handles
            .into_iter()
            .map(|h| usize::from(h.join().unwrap()))
            .sum::<usize>(),
        1
    );
    assert_eq!(fake.0.lock().unwrap().sent, ["once"]);
    assert_eq!(
        app.db
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM dispatch_operations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn known_unsent_first_prompt_is_not_queued_as_receipt_recovery() {
    let (app, fake) = fixture();
    let app = Arc::new(app);
    fake.0.lock().unwrap().before_input_error = Some("flow.not_idle".into());
    let id = op();
    assert_eq!(
        app.start_task("a", "frozen original", &id).unwrap_err(),
        "flow.not_idle"
    );
    let flow = serde_json::to_value(view(&app.db.lock().unwrap(), "a").unwrap()).unwrap();
    assert_eq!(flow["can_continue_first_prompt"], true);
    let recovery = crate::task_recovery::Recovery::new(app.clone(), Arc::new(|_| {}));
    assert!(recovery.poll(0).unwrap().is_empty());
    assert!(fake.0.lock().unwrap().sent.is_empty());
}

#[test]
fn explicit_continuation_keeps_original_process_and_prompt_and_sends_once() {
    let (app, fake) = fixture();
    fake.0.lock().unwrap().before_input_error = Some("flow.not_idle".into());
    let id = op();
    app.start_task("a", "frozen original", &id).unwrap_err();
    let target = app.binding("a").unwrap().target.unwrap();
    fake.0.lock().unwrap().before_input_error = None;
    let flow = app.continue_first_prompt("a", &id, 0).unwrap();
    assert_eq!(flow.state.as_deref(), Some("succeeded"));
    assert!(
        app.binding("a")
            .unwrap()
            .target
            .unwrap()
            .same_identity(&target)
    );
    assert!(app.continue_first_prompt("a", &id, 0).is_err());
    let s = fake.0.lock().unwrap();
    assert_eq!(s.created, 1);
    assert_eq!(s.prepared.len(), 1);
    assert_eq!(s.sent, vec!["frozen original"]);
    assert!(s.closed.is_empty());
}

#[test]
fn continuation_rejects_unknown_manual_decisions_and_replaced_targets() {
    for failure in ["unknown", "manual", "replaced", "epoch"] {
        let (app, fake) = fixture();
        fake.0.lock().unwrap().before_input_error = Some("flow.not_idle".into());
        let id = op();
        app.start_task("a", "original", &id).unwrap_err();
        let target = app.binding("a").unwrap().target.unwrap();
        match failure {
            "unknown" => {
                app.db
                    .lock()
                    .unwrap()
                    .execute("UPDATE dispatch_operations SET state='unknown'", [])
                    .unwrap();
            }
            "manual" => {
                app.db
                    .lock()
                    .unwrap()
                    .execute(
                        "UPDATE dispatch_operations SET evidence='manual: not sent'",
                        [],
                    )
                    .unwrap();
            }
            "replaced" => {
                fake.0
                    .lock()
                    .unwrap()
                    .panes
                    .get_mut(&target.pane_id)
                    .unwrap()
                    .0
                    .process_id = "replacement".into();
            }
            _ => {}
        }
        fake.0.lock().unwrap().before_input_error = None;
        assert!(
            app.continue_first_prompt("a", &id, if failure == "epoch" { 1 } else { 0 })
                .is_err(),
            "{failure}"
        );
        assert!(fake.0.lock().unwrap().sent.is_empty(), "{failure}");
    }
}

#[test]
fn repeated_unsent_attempts_keep_audit_and_late_receipt_never_replays() {
    let (app, fake) = fixture();
    fake.0.lock().unwrap().before_input_error = Some("flow.not_idle".into());
    let original = op();
    app.start_task("a", "original", &original).unwrap_err();
    app.continue_first_prompt("a", &original, 0).unwrap_err();
    let next = view(&app.db.lock().unwrap(), "a").unwrap();
    assert!(next.can_continue_first_prompt);
    assert_ne!(next.operation_id.as_deref(), Some(original.as_str()));
    assert!(app.continue_first_prompt("a", &original, 0).is_err());
    fake.0.lock().unwrap().before_input_error = None;
    fake.0.lock().unwrap().send_error = true;
    app.continue_first_prompt("a", next.operation_id.as_deref().unwrap(), 0)
        .unwrap_err();
    let unknown = view(&app.db.lock().unwrap(), "a").unwrap();
    assert!(!unknown.can_continue_first_prompt);
    fake.0.lock().unwrap().receipt_available = true;
    app.recover_lifecycle("a", unknown.operation_id.as_deref().unwrap())
        .unwrap();
    assert_eq!(status(&app, "a"), "in_progress");
    assert_eq!(fake.0.lock().unwrap().sent, ["original"]);
    assert_eq!(
        app.db
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM dispatch_operations WHERE state='not_sent'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
}

#[test]
fn shutdown_revokes_waiting_first_submission_and_future_continuations() {
    let (app, fake) = fixture();
    let app = Arc::new(app);
    let weak = Arc::downgrade(&app);
    fake.0.lock().unwrap().observe = Some(Arc::new(move |phase| {
        if phase == "prepared" {
            weak.upgrade().unwrap().stop_starts();
        }
    }));
    let id = op();
    assert!(app.start_task("a", "once", &id).is_err());
    assert!(fake.0.lock().unwrap().sent.is_empty());
    assert!(app.continue_first_prompt("a", &id, 0).is_err());
    assert_ne!(status(&app, "a"), "in_progress");
}

#[test]
fn interrupted_claim_is_unknown_on_reopen_and_cannot_be_continued() {
    let (app, fake) = fixture();
    fake.0.lock().unwrap().before_input_error = Some("flow.not_idle".into());
    let id = op();
    app.start_task("a", "once", &id).unwrap_err();
    // Crash just after claiming a durable write, before any definitive result.
    app.db
        .lock()
        .unwrap()
        .execute("UPDATE dispatch_operations SET state='pending'", [])
        .unwrap();
    crate::db::recover_interrupted(&app.db.lock().unwrap()).unwrap();
    assert!(
        !view(&app.db.lock().unwrap(), "a")
            .unwrap()
            .can_continue_first_prompt
    );
    assert!(app.continue_first_prompt("a", &id, 0).is_err());
    assert!(fake.0.lock().unwrap().sent.is_empty());
}

#[test]
fn manual_not_sent_confirmation_still_resolves_unknown_first_dispatch_without_sending() {
    let (app, fake) = fixture();
    fake.0.lock().unwrap().send_error = true;
    let id = op();
    app.start_task("a", "once", &id).unwrap_err();
    app.resolve("a", &format!("{id}:dispatch"), false, "")
        .unwrap();
    assert!(app.binding("a").unwrap().operation.is_none());
    assert!(
        !view(&app.db.lock().unwrap(), "a")
            .unwrap()
            .can_continue_first_prompt
    );
    assert_eq!(fake.0.lock().unwrap().sent, ["once"]);
}
