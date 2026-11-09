use super::*;

fn done_with_pane(app: &Workflow, task: &str) {
    app.start_task(task, "archive fixture", &op()).unwrap();
    app.db
        .lock()
        .unwrap()
        .execute(
            "UPDATE tasks SET status='done',completed_at='2026-01-01T00:00:00Z' WHERE id=?1",
            [task],
        )
        .unwrap();
}
fn archived(app: &Workflow, task: &str) -> bool {
    app.db
        .lock()
        .unwrap()
        .query_row(
            "SELECT archived_at IS NOT NULL FROM tasks WHERE id=?1",
            [task],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn archive_closes_exact_pane_keeps_session_and_other_tasks_on_both_machines() {
    for remote in [false, true] {
        let (app, fake) = fixture_on(remote);
        done_with_pane(&app, "a");
        app.start_task("b", "other", &op()).unwrap();
        let pane = app.binding("a").unwrap().target.unwrap();
        let session = saved(&app, "a");
        let sent = fake.0.lock().unwrap().sent.len();
        assert!(app.archive_task("a", None).unwrap());
        assert!(archived(&app, "a"));
        assert_eq!(saved(&app, "a"), session);
        assert_eq!(fake.0.lock().unwrap().closed, [pane.pane_id]);
        assert_eq!(fake.0.lock().unwrap().sent.len(), sent);
        assert_eq!(fake.0.lock().unwrap().panes.len(), 1);
        assert!(app.archive_task("a", None).unwrap());
        assert_eq!(fake.0.lock().unwrap().closed.len(), 1);
        let completed: String = app
            .db
            .lock()
            .unwrap()
            .query_row("SELECT completed_at FROM tasks WHERE id='a'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(completed, "2026-01-01T00:00:00Z");
    }
}

#[test]
fn archive_close_unknown_retains_task_and_reconciles_absence_without_repeat_close() {
    let (app, fake) = fixture();
    done_with_pane(&app, "a");
    fake.0.lock().unwrap().close_then_error = true;
    assert_eq!(app.archive_task("a", None).unwrap_err(), "timeout");
    assert!(!archived(&app, "a"));
    assert!(app.binding("a").unwrap().target.is_some());
    assert_eq!(
        view(&app.db.lock().unwrap(), "a").unwrap().state.as_deref(),
        Some("unknown")
    );
    assert!(app.archive_task("a", None).unwrap());
    assert!(archived(&app, "a"));
    assert_eq!(fake.0.lock().unwrap().closed.len(), 1);
}

#[test]
fn archive_missing_session_or_changed_identity_preserves_pane_and_can_retry() {
    let (app, fake) = fixture();
    done_with_pane(&app, "a");
    fake.0.lock().unwrap().missing_id = true;
    assert_eq!(
        app.archive_task("a", None).unwrap_err(),
        "flow.session_missing"
    );
    assert!(!archived(&app, "a"));
    assert!(fake.0.lock().unwrap().closed.is_empty());
    fake.0.lock().unwrap().missing_id = false;
    let old = fake
        .0
        .lock()
        .unwrap()
        .panes
        .values()
        .next()
        .unwrap()
        .0
        .process_id
        .clone();
    fake.0
        .lock()
        .unwrap()
        .panes
        .values_mut()
        .next()
        .unwrap()
        .0
        .process_id = "replacement".into();
    assert!(app.archive_task("a", None).is_err());
    assert!(!archived(&app, "a"));
    assert!(fake.0.lock().unwrap().closed.is_empty());
    fake.0
        .lock()
        .unwrap()
        .panes
        .values_mut()
        .next()
        .unwrap()
        .0
        .process_id = old;
    assert!(app.archive_task("a", None).unwrap());
}

#[test]
fn archive_write_failure_rolls_back_archive_and_binding_release_after_close() {
    let (app, fake) = fixture();
    done_with_pane(&app, "a");
    app.db.lock().unwrap().execute_batch("CREATE TRIGGER reject_archive BEFORE UPDATE OF archived_at ON tasks BEGIN SELECT RAISE(ABORT,'archive disk failure'); END;").unwrap();
    assert!(
        app.archive_task("a", None)
            .unwrap_err()
            .contains("archive disk failure")
    );
    assert!(!archived(&app, "a"));
    assert!(app.binding("a").unwrap().target.is_some());
    assert_eq!(fake.0.lock().unwrap().closed.len(), 1);
    app.db
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_archive")
        .unwrap();
    app.archive_task("a", None).unwrap();
    assert!(archived(&app, "a"));
    assert_eq!(fake.0.lock().unwrap().closed.len(), 1);
}

#[test]
fn batch_archive_uses_snapshot_rechecks_scope_and_continues_after_close_failure() {
    let (app, fake) = fixture();
    done_with_pane(&app, "a");
    app.db
        .lock()
        .unwrap()
        .execute_batch("UPDATE tasks SET project_id='p1',status='done' WHERE id='b';")
        .unwrap();
    fake.0.lock().unwrap().missing_id = true;
    let ids = vec!["a".into(), "missing".into(), "a".into(), "b".into()];
    let result = app.archive_completed_tasks("p1", &ids);
    assert_eq!(result.succeeded, ["b"]);
    assert_eq!(result.failed.len(), 1);
    assert_eq!(result.failed[0].id, "a");
    assert_eq!(result.skipped.len(), 1);
    assert!(!archived(&app, "a"));
    assert!(
        app.archive_completed_tasks("p2", &["a".into()])
            .succeeded
            .is_empty()
    );
    assert!(fake.0.lock().unwrap().closed.is_empty());
}

#[test]
fn archive_claim_blocks_restart_and_dispatch_during_session_read() {
    let (app, fake) = fixture();
    let app = Arc::new(app);
    done_with_pane(&app, "a");
    let weak = Arc::downgrade(&app);
    fake.0.lock().unwrap().observe = Some(Arc::new(move |phase| {
        if phase == "session" {
            let app = weak.upgrade().unwrap();
            assert_eq!(
                app.start_task("a", "do not start", &op()).unwrap_err(),
                "flow.verify_required"
            );
            assert!(crate::lifecycle::ensure_idle(&app.db.lock().unwrap(), "a", None).is_err());
            assert!(!archived(&app, "a"));
        }
    }));
    app.archive_task("a", None).unwrap();
    assert!(archived(&app, "a"));
}

#[test]
fn pending_start_or_dispatch_never_closes_an_archive_candidate() {
    for kind in ["start", "dispatch"] {
        let (app, fake) = fixture();
        done_with_pane(&app, "a");
        if kind == "start" {
            app.db.lock().unwrap().execute_batch("INSERT INTO lifecycle_operations(id,task_id,kind,state,created_at) VALUES ('pending-start','a','start','pending','now');").unwrap();
        } else {
            app.db.lock().unwrap().execute_batch("INSERT INTO dispatch_operations(id,task_id,workspace,target_json,prompt,created_at,state,had_binding) VALUES ('pending-dispatch','a','workspace','{}','once','now','unknown',1);").unwrap();
        }
        assert!(app.archive_task("a", None).is_err());
        assert!(!archived(&app, "a"));
        assert!(fake.0.lock().unwrap().closed.is_empty());
    }
}

#[test]
fn disconnected_archive_preserves_binding_until_explicit_retry() {
    let (app, fake) = fixture();
    done_with_pane(&app, "a");
    fake.0.lock().unwrap().lookup_error = Some("machine.disconnected".into());
    assert_eq!(
        app.archive_task("a", None).unwrap_err(),
        "machine.disconnected"
    );
    assert!(!archived(&app, "a"));
    assert!(app.binding("a").unwrap().target.is_some());
    assert!(fake.0.lock().unwrap().closed.is_empty());
    fake.0.lock().unwrap().lookup_error = None;
    app.archive_task("a", None).unwrap();
    assert!(archived(&app, "a"));
}

#[test]
fn unbound_archive_is_local_but_unknown_start_and_dispatch_still_block() {
    let (app, fake) = fixture();
    app.db
        .lock()
        .unwrap()
        .execute_batch("UPDATE tasks SET status='done';")
        .unwrap();
    fake.0.lock().unwrap().lookup_error = Some("offline".into());
    app.archive_task("a", None).unwrap();
    app.db.lock().unwrap().execute_batch("INSERT INTO lifecycle_operations(id,task_id,kind,state,created_at) VALUES ('starting','b','start','unknown','now');").unwrap();
    assert_eq!(
        app.archive_task("b", None).unwrap_err(),
        "flow.verify_required"
    );
    assert!(!archived(&app, "b"));
    assert!(fake.0.lock().unwrap().closed.is_empty());
}
