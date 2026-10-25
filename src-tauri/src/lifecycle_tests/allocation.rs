use super::*;

#[test]
fn lost_shared_tab_creation_is_quarantined_without_blocking_another_task() {
    let (app, fake) = fixture();
    let first = op();
    fake.0.lock().unwrap().lost_create_response = true;
    assert!(app.start_task("a", "A", &first).is_err());
    assert!(app.binding("a").unwrap().target.is_none());
    let path = std::env::temp_dir().join(format!("kanban-placement-{}.db", op()));
    app.db
        .lock()
        .unwrap()
        .execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])
        .unwrap();
    let conn = Connection::open(&path).unwrap();
    crate::db::init(&conn).unwrap();
    crate::db::recover_interrupted(&conn).unwrap();
    let reopened = Workflow {
        db: Arc::new(Mutex::new(conn)),
        herdr: app.herdr.clone(),
    };
    assert_eq!(
        reopened.recover_lifecycle("a", &first).unwrap_err(),
        "flow.close_before_reset"
    );
    fake.0.lock().unwrap().lost_create_response = false;
    reopened.start_task("b", "B", &op()).unwrap();
    assert_eq!(
        reopened.binding("b").unwrap().target.unwrap().pane_id,
        "created-2"
    );
    assert_eq!(fake.0.lock().unwrap().sent, vec!["B"]);
    fake.0.lock().unwrap().panes.remove("created-1");
    // B is still running; its persisted creation receipt excludes it from A's unknown result.
    reopened.recover_lifecycle("a", &first).unwrap();
    reopened.start_task("a", "A", &op()).unwrap();
    assert_eq!(fake.0.lock().unwrap().sent, vec!["B", "A"]);
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn creation_receipt_commit_failure_preserves_inventory_and_does_not_adopt_an_orphan() {
    let (app, fake) = fixture();
    let first = op();
    app.db.lock().unwrap().execute_batch("CREATE TRIGGER fail_receipt BEFORE UPDATE OF target_json ON pane_allocations BEGIN SELECT RAISE(FAIL,'disk failure'); END;").unwrap();
    assert!(app.start_task("a", "A", &first).is_err());
    assert!(fake.0.lock().unwrap().sent.is_empty());
    app.db
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_receipt")
        .unwrap();
    app.start_task("b", "B", &op()).unwrap();
    assert_eq!(
        app.binding("b").unwrap().target.unwrap().pane_id,
        "created-2"
    );
    assert_eq!(
        app.recover_lifecycle("a", &first).unwrap_err(),
        "flow.close_before_reset"
    );
    fake.0.lock().unwrap().panes.remove("created-1");
    app.recover_lifecycle("a", &first).unwrap();
    assert_eq!(fake.0.lock().unwrap().sent, vec!["B"]);
}

#[test]
fn reuses_unoccupied_idle_pane_with_fresh_session_but_skips_busy_targets() {
    let (app, fake) = fixture();
    let idle = target("idle");
    let mut busy = target("busy");
    busy.status = "working".into();
    fake.0.lock().unwrap().panes.extend([
        (idle.pane_id.clone(), (idle, Some(op()))),
        (busy.pane_id.clone(), (busy, None)),
    ]);
    app.start_task("a", "A", &op()).unwrap();
    app.start_task("b", "B", &op()).unwrap();
    let s = fake.0.lock().unwrap();
    assert_eq!(s.created, 1);
    assert_eq!(s.prepared, vec![None, None]);
    assert_eq!(app.binding("a").unwrap().target.unwrap().pane_id, "idle");
    assert_ne!(app.binding("b").unwrap().target.unwrap().pane_id, "idle");
}

#[test]
fn concurrent_projects_claim_distinct_panes() {
    let (app, fake) = fixture();
    let idle = target("idle");
    fake.0
        .lock()
        .unwrap()
        .panes
        .insert("idle".into(), (idle, None));
    std::thread::scope(|scope| {
        let a = scope.spawn(|| app.start_task("a", "A", &op()));
        let b = scope.spawn(|| app.start_task("b", "B", &op()));
        a.join().unwrap().unwrap();
        b.join().unwrap().unwrap();
    });
    assert_ne!(
        app.binding("a").unwrap().target.unwrap().pane_id,
        app.binding("b").unwrap().target.unwrap().pane_id
    );
    assert_eq!(fake.0.lock().unwrap().sent.len(), 2);
}
