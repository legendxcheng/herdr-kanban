use super::*;

#[test]
fn directory_sync_does_not_change_a_saved_conversations_working_directory() {
    let (app, herdr) = fixture();
    app.start_task("a", "hello", &uuid::Uuid::new_v4().to_string())
        .unwrap();
    app.db
        .lock()
        .unwrap()
        .execute(
            "UPDATE projects SET root_path='changed-directory' WHERE id='p1'",
            [],
        )
        .unwrap();
    app.complete_task("a", &uuid::Uuid::new_v4().to_string())
        .unwrap();
    app.start_task("a", "must not resend", &uuid::Uuid::new_v4().to_string())
        .unwrap();
    assert_eq!(herdr.0.lock().unwrap().directories, vec![".", "."]);
}

#[test]
fn completes_and_resumes_exact_conversation_after_application_reopen_without_replaying() {
    let (app, fake) = fixture();
    app.start_task("a", "original task", &op()).unwrap();
    let session = saved(&app, "a").unwrap();
    assert_eq!(status(&app, "a"), "in_progress");
    assert!(app.complete_task("a", &op()).unwrap().warning.is_none());
    assert_eq!(status(&app, "a"), "done");
    assert!(app.binding("a").unwrap().target.is_none());
    assert_eq!(app.binding("a").unwrap().state, "dormant");
    let path = std::env::temp_dir().join(format!("kanban-flow-{}.db", op()));
    app.db
        .lock()
        .unwrap()
        .execute("VACUUM main INTO ?1", [path.to_str().unwrap()])
        .unwrap();
    drop(app);
    let persisted = Connection::open(&path).unwrap();
    crate::db::init(&persisted).unwrap();
    crate::db::recover_interrupted(&persisted).unwrap();
    let reopened = Workflow {
        db: Arc::new(Mutex::new(persisted)),
        herdr: fake.clone(),
    };
    reopened.start_task("a", "must not send", &op()).unwrap();
    assert_eq!(saved(&reopened, "a"), Some(session.clone()));
    let s = fake.0.lock().unwrap();
    assert_eq!(s.prepared, vec![None, Some(session)]);
    assert_eq!(s.sent, vec!["original task"]);
    assert_eq!(s.created, 2);
    assert_eq!(s.closed.len(), 1);
    drop(s);
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn repeated_drop_and_same_operation_never_repeat_first_prompt() {
    let (app, fake) = fixture();
    let id = op();
    app.start_task("a", "A", &id).unwrap();
    app.start_task("a", "A", &id).unwrap();
    app.start_task("a", "A", &op()).unwrap();
    assert_eq!(fake.0.lock().unwrap().prepared.len(), 1);
    assert_eq!(fake.0.lock().unwrap().sent, vec!["A"]);
    assert!(app.start_task("b", "B", &id).is_err());
}

#[test]
fn incorrect_resumed_identity_does_not_overwrite_saved_session() {
    let (app, fake) = fixture();
    app.start_task("a", "A", &op()).unwrap();
    app.complete_task("a", &op()).unwrap();
    let original = saved(&app, "a");
    fake.0.lock().unwrap().mismatch = true;
    assert!(app.start_task("a", "", &op()).is_err());
    assert_eq!(saved(&app, "a"), original);
    assert_eq!(fake.0.lock().unwrap().sent.len(), 1);
}

#[test]
fn uncertain_prompt_is_reconciled_through_existing_dispatch_evidence_without_resend() {
    let (app, fake) = fixture();
    fake.0.lock().unwrap().send_error = true;
    let id = op();
    assert!(app.start_task("a", "A", &id).is_err());
    assert!(app.start_task("a", "A", &op()).is_err());
    assert!(app.recover_lifecycle("a", &id).is_err());
    app.resolve(
        "a",
        &format!("{id}:dispatch"),
        true,
        "Observed task A in pane",
    )
    .unwrap();
    assert_eq!(
        view(&app.db.lock().unwrap(), "a").unwrap().state.as_deref(),
        Some("succeeded")
    );
    assert_eq!(status(&app, "a"), "in_progress");
    assert_eq!(fake.0.lock().unwrap().sent, vec!["A"]);
}

#[test]
fn received_prompt_with_missing_ack_finishes_start_without_replay() {
    let (app, fake) = fixture();
    {
        let mut s = fake.0.lock().unwrap();
        s.send_error = true;
        s.receipt_available = true;
    }
    app.start_task("a", "exact prompt", &op()).unwrap();
    assert_eq!(status(&app, "a"), "in_progress");
    assert_eq!(app.binding("a").unwrap().state, "valid");
    assert!(saved(&app, "a").is_some());
    assert_eq!(
        view(&app.db.lock().unwrap(), "a").unwrap().state.as_deref(),
        Some("succeeded")
    );
    assert_eq!(fake.0.lock().unwrap().sent, vec!["exact prompt"]);
}

#[test]
fn early_receipt_is_rechecked_and_committed_with_session_and_never_replayed() {
    for replace_after_probe in [false, true] {
        let (app, fake) = fixture();
        {
            let mut state = fake.0.lock().unwrap();
            state.early_receipt = true;
            state.receipt_available = true;
            if replace_after_probe {
                let weak = Arc::downgrade(&fake);
                state.observe = Some(Arc::new(move |phase| {
                    if phase == "early_receipt" {
                        weak.upgrade()
                            .unwrap()
                            .0
                            .lock()
                            .unwrap()
                            .panes
                            .values_mut()
                            .next()
                            .unwrap()
                            .0
                            .process_id = "replaced".into();
                    }
                }));
            }
        }
        let id = op();
        let result = app.start_task("a", "测试", &id);
        assert_eq!(result.is_ok(), !replace_after_probe);
        assert_eq!(fake.0.lock().unwrap().sent, vec!["测试"]);
        assert_eq!(fake.0.lock().unwrap().receipt_checks, 1);
        if replace_after_probe {
            assert_eq!(app.binding("a").unwrap().state, "unknown");
        } else {
            assert_eq!(status(&app, "a"), "in_progress");
            assert!(saved(&app, "a").is_some());
            let evidence: String = app
                .db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT evidence FROM dispatch_operations WHERE id=?1",
                    [format!("{id}:dispatch")],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(evidence.contains("Exact user message"));
        }
    }
}

#[test]
fn delayed_receipt_after_restart_recovers_dispatch_session_and_start_together() {
    let (app, fake) = fixture();
    {
        let mut s = fake.0.lock().unwrap();
        s.send_error = true;
        s.missing_id = true;
    }
    let id = op();
    assert!(app.start_task("a", "delayed", &id).is_err());
    assert!(saved(&app, "a").is_none());
    crate::db::recover_interrupted(&app.db.lock().unwrap()).unwrap();
    {
        let mut s = fake.0.lock().unwrap();
        s.missing_id = false;
        s.receipt_available = true;
    }
    app.recover_lifecycle("a", &id).unwrap();
    assert_eq!(status(&app, "a"), "in_progress");
    assert!(saved(&app, "a").is_some());
    assert!(app.binding("a").unwrap().operation.is_none());
    assert_eq!(
        view(&app.db.lock().unwrap(), "a").unwrap().state.as_deref(),
        Some("succeeded")
    );
    assert_eq!(fake.0.lock().unwrap().sent, vec!["delayed"]);
}

#[test]
fn receipt_recovery_rolls_back_all_state_when_session_persistence_fails() {
    let (app, fake) = fixture();
    {
        let mut s = fake.0.lock().unwrap();
        s.send_error = true;
        s.missing_id = true;
    }
    let id = op();
    assert!(app.start_task("a", "once", &id).is_err());
    {
        let mut s = fake.0.lock().unwrap();
        s.missing_id = false;
        s.receipt_available = true;
    }
    app.db.lock().unwrap().execute_batch("CREATE TRIGGER fail_receipt BEFORE INSERT ON task_sessions BEGIN SELECT RAISE(ABORT,'disk failure'); END;").unwrap();
    assert!(app.recover_lifecycle("a", &id).is_err());
    assert_eq!(status(&app, "a"), "todo");
    assert_eq!(app.binding("a").unwrap().state, "unknown");
    assert!(saved(&app, "a").is_none());
    assert_eq!(
        view(&app.db.lock().unwrap(), "a").unwrap().state.as_deref(),
        Some("unknown")
    );
    app.db
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_receipt")
        .unwrap();
    app.recover_lifecycle("a", &id).unwrap();
    assert_eq!(fake.0.lock().unwrap().sent, vec!["once"]);
}

#[test]
fn receipt_does_not_adopt_a_replaced_process_or_switched_session() {
    for replace_process in [true, false] {
        let (app, fake) = fixture();
        fake.0.lock().unwrap().send_error = true;
        let id = op();
        assert!(app.start_task("a", "once", &id).is_err());
        let original_session = saved(&app, "a");
        {
            let mut s = fake.0.lock().unwrap();
            s.receipt_available = true;
            let (target, session) = s.panes.values_mut().next().unwrap();
            if replace_process {
                target.process_id = "new-process".into();
            } else {
                *session = Some(uuid::Uuid::new_v4().to_string());
            }
        }
        assert!(app.recover_lifecycle("a", &id).is_err());
        assert_eq!(status(&app, "a"), "todo");
        assert_eq!(saved(&app, "a"), original_session);
        assert_eq!(fake.0.lock().unwrap().sent, vec!["once"]);
    }
}

#[test]
fn manual_sent_confirmation_also_captures_late_session_and_finishes_start() {
    let (app, fake) = fixture();
    {
        let mut s = fake.0.lock().unwrap();
        s.send_error = true;
        s.missing_id = true;
    }
    let id = op();
    assert!(app.start_task("a", "once", &id).is_err());
    fake.0.lock().unwrap().missing_id = false;
    app.resolve(
        "a",
        &format!("{id}:dispatch"),
        true,
        "User observed the exact prompt",
    )
    .unwrap();
    assert!(saved(&app, "a").is_some());
    assert_eq!(
        view(&app.db.lock().unwrap(), "a").unwrap().state.as_deref(),
        Some("succeeded")
    );
    assert_eq!(fake.0.lock().unwrap().sent, vec!["once"]);
}

#[test]
#[ignore = "Read-only real Herdr queries; requires an explicit copied receipt-acceptance.db and task id"]
fn live_receipt_recovers_copied_task_without_input() {
    let path = std::path::PathBuf::from(
        std::env::var_os("KANBAN_RECEIPT_SNAPSHOT").expect("snapshot required"),
    );
    assert_eq!(path.file_name().unwrap(), "receipt-acceptance.db");
    let task = std::env::var("KANBAN_RECEIPT_TASK").expect("task required");
    let db = Connection::open(&path).unwrap();
    crate::db::init(&db).unwrap();
    let app = Workflow {
        db: Arc::new(Mutex::new(db)),
        herdr: Arc::new(crate::herdr::Cli::from_env()),
    };
    let original = view(&app.db.lock().unwrap(), &task).unwrap();
    assert_eq!(original.state.as_deref(), Some("unknown"));
    app.recover_lifecycle(&task, original.operation_id.as_deref().unwrap())
        .unwrap();
    assert_eq!(app.binding(&task).unwrap().state, "valid");
    assert!(app.binding(&task).unwrap().operation.is_none());
    assert!(saved(&app, &task).is_some());
    assert_eq!(status(&app, &task), "in_progress");
    drop(app);
    let reopened = Connection::open(&path).unwrap();
    crate::db::recover_interrupted(&reopened).unwrap();
    assert_eq!(
        view(&reopened, &task).unwrap().state.as_deref(),
        Some("succeeded")
    );
}

#[test]
fn failed_start_requires_pane_absence_before_releasing_reservation() {
    let (app, fake) = fixture();
    fake.0.lock().unwrap().prepare_error = true;
    let id = op();
    assert!(app.start_task("a", "A", &id).is_err());
    assert!(app.recover_lifecycle("a", &id).is_err());
    assert!(ensure_idle(&app.db.lock().unwrap(), "a", None).is_err());
    fake.0.lock().unwrap().panes.clear();
    app.recover_lifecycle("a", &id).unwrap();
    fake.0.lock().unwrap().prepare_error = false;
    app.start_task("a", "A", &op()).unwrap();
    assert_eq!(fake.0.lock().unwrap().sent, vec!["A"]);
}

#[test]
fn pane_busy_releases_rejected_start_without_closing_or_sending() {
    let (app, fake) = fixture();
    fake.0.lock().unwrap().prepare_busy = true;
    let id = op();
    assert!(app.start_task("a", "A", &id).is_err());
    assert_eq!(
        view(&app.db.lock().unwrap(), "a").unwrap().state.as_deref(),
        Some("failed")
    );
    assert!(app.binding("a").unwrap().target.is_none());
    {
        let s = fake.0.lock().unwrap();
        assert!(s.sent.is_empty());
        assert!(s.closed.is_empty());
        assert_eq!(s.panes.len(), 1);
    }
    fake.0.lock().unwrap().prepare_busy = false;
    app.start_task("a", "A", &op()).unwrap();
    assert_eq!(fake.0.lock().unwrap().sent, vec!["A"]);
}

#[test]
fn pane_busy_legacy_unknown_can_be_checked_without_removing_pane() {
    let (app, fake) = fixture();
    fake.0.lock().unwrap().prepare_error = true;
    let id = op();
    assert!(app.start_task("a", "A", &id).is_err());
    app.end_flow(
        &id,
        "unknown",
        r#"herdr.failed: {"error":{"code":"agent_pane_busy"},"id":"cli:agent:start"}"#,
    )
    .unwrap();
    app.recover_lifecycle("a", &id).unwrap();
    assert_eq!(
        view(&app.db.lock().unwrap(), "a").unwrap().state.as_deref(),
        Some("failed")
    );
    assert!(app.binding("a").unwrap().target.is_none());
    let s = fake.0.lock().unwrap();
    assert_eq!(s.panes.len(), 1);
    assert!(s.closed.is_empty());
    assert!(s.sent.is_empty());
}

#[test]
fn pane_busy_keeps_saved_session_for_explicit_resume_retry() {
    let (app, fake) = fixture();
    app.start_task("a", "A", &op()).unwrap();
    app.complete_task("a", &op()).unwrap();
    let session = saved(&app, "a").unwrap();
    fake.0.lock().unwrap().prepare_busy = true;
    assert!(app.start_task("a", "ignored", &op()).is_err());
    assert_eq!(saved(&app, "a").as_deref(), Some(session.as_str()));
    fake.0.lock().unwrap().prepare_busy = false;
    app.start_task("a", "ignored", &op()).unwrap();
    let s = fake.0.lock().unwrap();
    assert_eq!(s.sent, vec!["A"]);
    assert_eq!(s.prepared.last(), Some(&Some(session)));
}

#[test]
fn pane_busy_cannot_release_an_operation_that_has_dispatched() {
    let (app, fake) = fixture();
    fake.0.lock().unwrap().send_error = true;
    let id = op();
    assert!(app.start_task("a", "A", &id).is_err());
    let error = r#"herdr.failed: {"error":{"code":"agent_pane_busy"},"id":"cli:agent:start"}"#;
    app.end_flow(&id, "unknown", error).unwrap();
    assert!(app.release_rejected_start("a", &id, error).is_err());
    assert!(app.binding("a").unwrap().target.is_some());
    assert_eq!(
        view(&app.db.lock().unwrap(), "a").unwrap().state.as_deref(),
        Some("unknown")
    );
    assert_eq!(fake.0.lock().unwrap().sent, vec!["A"]);
}

#[test]
fn restart_marks_pending_launch_unknown_and_never_automatically_replays_it() {
    let (app, fake) = fixture();
    let id = op();
    app.begin_flow("a", &id, "start").unwrap();
    crate::db::recover_interrupted(&app.db.lock().unwrap()).unwrap();
    assert!(app.start_task("a", "A", &op()).is_err());
    app.recover_lifecycle("a", &id).unwrap();
    app.start_task("a", "A", &op()).unwrap();
    assert_eq!(fake.0.lock().unwrap().sent, vec!["A"]);
}

#[test]
fn no_prompt_fails_without_creating_and_legacy_tasks_require_repair() {
    let (app, fake) = fixture();
    assert!(app.start_task("a", "", &op()).is_err());
    app.db
        .lock()
        .unwrap()
        .execute("UPDATE tasks SET dispatched_to='old/pane' WHERE id='b'", [])
        .unwrap();
    assert!(app.start_task("b", "B", &op()).is_err());
    assert_eq!(fake.0.lock().unwrap().created, 0);
}

#[test]
fn resume_commit_failure_recovers_by_exact_session_without_sending() {
    let (app, fake) = fixture();
    app.start_task("a", "A", &op()).unwrap();
    app.complete_task("a", &op()).unwrap();
    app.db.lock().unwrap().execute_batch("CREATE TRIGGER fail_resume BEFORE UPDATE OF status ON tasks WHEN new.status='in_progress' BEGIN SELECT RAISE(FAIL,'disk failure'); END;").unwrap();
    let id = op();
    assert!(app.start_task("a", "", &id).is_err());
    assert_eq!(status(&app, "a"), "done");
    app.db
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_resume")
        .unwrap();
    crate::db::recover_interrupted(&app.db.lock().unwrap()).unwrap();
    app.recover_lifecycle("a", &id).unwrap();
    assert_eq!(status(&app, "a"), "in_progress");
    assert_eq!(fake.0.lock().unwrap().sent, vec!["A"]);
}

#[test]
fn late_recovery_cannot_release_a_newer_operations_binding() {
    let (app, _) = fixture();
    let old = op();
    app.begin_flow("a", &old, "start").unwrap();
    app.end_flow(&old, "failed", "").unwrap();
    let new = op();
    app.begin_flow("a", &new, "start").unwrap();
    app.reserve_target("a", &new, &target("new-pane")).unwrap();
    assert!(app.reserve_target("a", &old, &target("old-pane")).is_err());
    assert!(app.activate("a", &old, &target("old-pane")).is_err());
    assert_eq!(
        app.binding("a").unwrap().target.unwrap().pane_id,
        "new-pane"
    );
}
