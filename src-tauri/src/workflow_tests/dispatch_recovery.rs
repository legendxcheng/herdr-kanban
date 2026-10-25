use super::*;

#[test]
fn unconfirmed_submission_stays_unknown_without_blocking_another_pane_or_replaying() {
    let (app, external) = fixture();
    *external.uncertain.lock().unwrap() = true;
    *external.send_error_message.lock().unwrap() = Some("dispatch.submit_verify_required".into());
    assert_eq!(
        app.dispatch("a", "w1:p1", "Once", "op-1").unwrap_err(),
        "dispatch.submit_verify_required"
    );
    assert_eq!(app.binding("a").unwrap().state, "unknown");
    assert!(app.dispatch("a", "w1:p1", "Once", "op-1").is_err());
    assert!(app.dispatch("a", "w1:p1", "Once", "op-2").is_err());
    *external.uncertain.lock().unwrap() = false;
    app.dispatch("b", "w1:p2", "Other task", "other").unwrap();
    assert_eq!(*external.sent.lock().unwrap(), vec!["Once", "Other task"]);
}

#[test]
fn a_known_unsent_failure_releases_only_the_new_reservation() {
    let (app, external) = fixture();
    *external.unavailable.lock().unwrap() = true;
    assert!(app.dispatch("a", "w1:p1", "First", "failed").is_err());
    assert_eq!(app.binding("a").unwrap().state, "unbound");
    *external.unavailable.lock().unwrap() = false;
    app.dispatch("b", "w1:p1", "B", "succeeds").unwrap();
    *external.unavailable.lock().unwrap() = true;
    assert!(app.dispatch("b", "w1:p1", "Again", "failed-again").is_err());
    assert_eq!(app.binding("b").unwrap().state, "valid");
    assert_eq!(*external.sent.lock().unwrap(), vec!["B"]);
}

#[test]
fn a_successful_send_with_failed_commit_remains_recoverable_without_replay() {
    let (app, external) = fixture();
    app.db.lock().unwrap().execute_batch("CREATE TRIGGER fail_finish BEFORE UPDATE OF status ON tasks BEGIN SELECT RAISE(ABORT,'disk failure'); END;").unwrap();
    assert_eq!(
        app.dispatch("a", "w1:p1", "Once", "op").unwrap_err(),
        "dispatch.verify_required"
    );
    assert_eq!(app.binding("a").unwrap().state, "unknown");
    assert!(app.dispatch("b", "w1:p1", "Never", "other").is_err());
    app.db
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_finish")
        .unwrap();
    app.resolve("a", "op", true, "Observed Once in target pane")
        .unwrap();
    assert_eq!(app.binding("a").unwrap().state, "valid");
    assert_eq!(*external.sent.lock().unwrap(), vec!["Once"]);
}

#[test]
fn repeating_the_same_operation_does_not_resend_but_a_new_dispatch_can_continue() {
    let (app, external) = fixture();
    app.dispatch("a", "w1:p1", "First", "op-1").unwrap();
    app.dispatch("a", "w1:p1", "First", "op-1").unwrap();
    app.dispatch("a", "w1:p1", "Continue", "op-2").unwrap();
    assert_eq!(*external.sent.lock().unwrap(), vec!["First", "Continue"]);
}

#[test]
fn uncertain_send_requires_evidence_and_recovery_never_replays_the_prompt() {
    let (app, external) = fixture();
    *external.uncertain.lock().unwrap() = true;
    assert!(app.dispatch("a", "w1:p1", "Once", "op-1").is_err());
    assert_eq!(app.binding("a").unwrap().state, "unknown");
    assert!(app.dispatch("a", "w1:p1", "Once", "op-2").is_err());
    assert!(app.resolve("a", "op-1", true, "").is_err());
    app.resolve("a", "op-1", true, "Confirmed in the target terminal")
        .unwrap();
    assert_eq!(app.binding("a").unwrap().state, "valid");
    assert_eq!(*external.sent.lock().unwrap(), vec!["Once"]);
}

#[test]
fn interrupted_operations_survive_reopen_and_can_be_resolved_without_resending() {
    let (app, external) = fixture();
    *external.uncertain.lock().unwrap() = true;
    assert!(app.dispatch("a", "w1:p1", "Once", "op-1").is_err());
    // Simulate a crash between send and result persistence, then application startup.
    app.db
        .lock()
        .unwrap()
        .execute(
            "UPDATE dispatch_operations SET state='pending' WHERE id='op-1'",
            [],
        )
        .unwrap();
    crate::db::init(&app.db.lock().unwrap()).unwrap();
    crate::db::recover_interrupted(&app.db.lock().unwrap()).unwrap();
    let reopened = Workflow {
        db: app.db.clone(),
        herdr: app.herdr.clone(),
    };
    assert_eq!(reopened.binding("a").unwrap().state, "unknown");
    reopened
        .resolve("a", "op-1", false, "Target inspected; prompt absent")
        .unwrap();
    assert_eq!(reopened.binding("a").unwrap().state, "unbound");
    assert_eq!(*external.sent.lock().unwrap(), vec!["Once"]);
}
