use super::*;

#[test]
fn missing_session_preserves_pane_and_completion_can_be_retried() {
    let (app, fake) = fixture();
    app.start_task("a", "A", &op()).unwrap();
    fake.0.lock().unwrap().missing_id = true;
    assert!(app.complete_task("a", &op()).unwrap().warning.is_some());
    assert_eq!(status(&app, "a"), "done");
    assert!(app.binding("a").unwrap().target.is_some());
    assert!(fake.0.lock().unwrap().closed.is_empty());
    let completed: String = app
        .db
        .lock()
        .unwrap()
        .query_row("SELECT completed_at FROM tasks WHERE id='a'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(chrono::DateTime::parse_from_rfc3339(&completed).is_ok());
    // Simulate a completion on a previous day; retrying cleanup must not move it.
    app.db
        .lock()
        .unwrap()
        .execute(
            "UPDATE tasks SET completed_at='2026-01-01T00:00:00Z' WHERE id='a'",
            [],
        )
        .unwrap();
    fake.0.lock().unwrap().missing_id = false;
    app.complete_task("a", &op()).unwrap();
    assert!(app.binding("a").unwrap().target.is_none());
    let retained: String = app
        .db
        .lock()
        .unwrap()
        .query_row("SELECT completed_at FROM tasks WHERE id='a'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(retained, "2026-01-01T00:00:00Z");
    app.start_task("a", "", &op()).unwrap();
    let cleared: Option<String> = app
        .db
        .lock()
        .unwrap()
        .query_row("SELECT completed_at FROM tasks WHERE id='a'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(cleared.is_none());
    app.complete_task("a", &op()).unwrap();
    let recompleted: String = app
        .db
        .lock()
        .unwrap()
        .query_row("SELECT completed_at FROM tasks WHERE id='a'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_ne!(recompleted, retained);
}

#[test]
fn close_timeout_does_not_block_other_tasks_and_retry_does_not_close_again_when_absent() {
    let (app, fake) = fixture();
    app.start_task("a", "A", &op()).unwrap();
    fake.0.lock().unwrap().close_then_error = true;
    assert!(app.complete_task("a", &op()).unwrap().warning.is_some());
    assert!(app.start_task("a", "", &op()).is_err());
    app.start_task("b", "B", &op()).unwrap();
    app.complete_task("a", &op()).unwrap();
    assert_eq!(fake.0.lock().unwrap().closed.len(), 1);
    assert!(saved(&app, "a").is_some());
}

#[test]
fn failed_restart_cannot_hide_unfinished_completion_cleanup() {
    let (app, fake) = fixture();
    app.start_task("a", "A", &op()).unwrap();
    fake.0
        .lock()
        .unwrap()
        .panes
        .values_mut()
        .next()
        .unwrap()
        .0
        .process_id = "replacement".into();
    assert!(app.complete_task("a", &op()).unwrap().warning.is_some());
    assert!(app.start_task("a", "", &op()).is_err());
    assert_eq!(status(&app, "a"), "done");
    assert_eq!(
        crate::tasks::archive(&mut app.db.lock().unwrap(), "a").unwrap_err(),
        "archive.cleanup_required"
    );
    assert!(fake.0.lock().unwrap().closed.is_empty());
}

#[test]
fn replaced_process_is_never_closed() {
    let (app, fake) = fixture();
    app.start_task("a", "A", &op()).unwrap();
    fake.0
        .lock()
        .unwrap()
        .panes
        .values_mut()
        .next()
        .unwrap()
        .0
        .process_id = "replacement".into();
    assert!(app.complete_task("a", &op()).unwrap().warning.is_some());
    assert!(fake.0.lock().unwrap().closed.is_empty());
}
