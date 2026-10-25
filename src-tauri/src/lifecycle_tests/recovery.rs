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
    app.recover_lifecycle("a", &id).unwrap();
    assert_eq!(status(&app, "a"), "in_progress");
    assert_eq!(fake.0.lock().unwrap().sent, vec!["A"]);
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
