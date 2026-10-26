use super::*;

#[test]
fn deletion_is_atomic_and_late_dispatch_cannot_reclaim_the_pane() {
    for uncertain in [false, true] {
        let (app, external) = fixture();
        *external.uncertain.lock().unwrap() = uncertain;
        let db = app.db.clone();
        *external.after_send.lock().unwrap() = Some(Box::new(move || {
            let mut db = db.lock().unwrap();
            db.execute_batch("CREATE TRIGGER fail_delete BEFORE DELETE ON task_bindings BEGIN SELECT RAISE(ABORT,'disk failure'); END;").unwrap();
            assert!(crate::tasks::delete(&mut db, "a").is_err());
            assert_eq!(crate::tasks::list(&db, "p1").unwrap().len(), 1);
            assert!(binding(&db, "a").unwrap().target.is_some());
            db.execute_batch("DROP TRIGGER fail_delete").unwrap();
            crate::tasks::delete(&mut db, "a").unwrap();
            crate::tasks::delete(&mut db, "a").unwrap();
        }));
        assert!(app.dispatch("a", "w1:p1", "old", "old-op").is_err());
        let db = app.db.lock().unwrap();
        assert!(crate::tasks::list(&db, "p1").unwrap().is_empty());
        crate::db::recover_interrupted(&db).unwrap();
        let evidence = binding(&db, "a").unwrap();
        assert!(evidence.target.is_none());
        assert_eq!(evidence.operation.unwrap().state, "pending");
        let new = crate::tasks::create(&db, "p1", "A", crate::models::TaskPriority::Normal, "{}")
            .unwrap();
        assert_ne!(new.id, "a");
        drop(db);
        *external.after_send.lock().unwrap() = None;
        *external.uncertain.lock().unwrap() = false;
        app.dispatch(&new.id, "w1:p1", "new", "new-op").unwrap();
        assert!(app.resolve("a", "old-op", true, "late result").is_err());
        assert_eq!(app.binding(&new.id).unwrap().state, "valid");
        assert_eq!(*external.sent.lock().unwrap(), vec!["old", "new"]);
    }
}

#[test]
fn failed_archive_rolls_back_the_card_and_binding_then_retry_can_succeed() {
    let (app, external) = fixture();
    app.dispatch("a", "w1:p1", "Task A", "op-a").unwrap();
    let mut db = app.db.lock().unwrap();
    db.execute_batch("UPDATE tasks SET status='done' WHERE id='a';
        CREATE TRIGGER fail_archive BEFORE DELETE ON task_bindings BEGIN SELECT RAISE(ABORT,'disk failure'); END;").unwrap();
    assert!(crate::tasks::archive(&mut db, "a").is_err());
    assert_eq!(crate::tasks::list(&db, "p1").unwrap().len(), 1);
    assert_eq!(binding(&db, "a").unwrap().target.unwrap().pane_id, "w1:p1");
    db.execute_batch("DROP TRIGGER fail_archive").unwrap();
    crate::tasks::archive(&mut db, "a").unwrap();
    assert!(crate::tasks::list(&db, "p1").unwrap().is_empty());
    assert_eq!(*external.sent.lock().unwrap(), vec!["Task A"]);
}

#[test]
fn archive_and_dispatch_are_mutually_exclusive() {
    let (app, external) = fixture();
    app.db
        .lock()
        .unwrap()
        .execute("UPDATE tasks SET status='done' WHERE id='a'", [])
        .unwrap();
    let gate = std::sync::Barrier::new(2);
    let (archived, dispatched) = std::thread::scope(|scope| {
        let archive = scope.spawn(|| {
            gate.wait();
            crate::tasks::archive(&mut app.db.lock().unwrap(), "a")
        });
        let dispatch = scope.spawn(|| {
            gate.wait();
            app.dispatch("a", "w1:p1", "Once", "race")
        });
        (archive.join().unwrap(), dispatch.join().unwrap())
    });
    assert_ne!(archived.is_ok(), dispatched.is_ok());
    if archived.is_ok() {
        assert!(
            crate::tasks::list(&app.db.lock().unwrap(), "p1")
                .unwrap()
                .is_empty()
        );
        assert!(external.sent.lock().unwrap().is_empty());
    } else {
        assert_eq!(
            crate::tasks::list(&app.db.lock().unwrap(), "p1").unwrap()[0].status,
            crate::models::TaskStatus::InProgress
        );
        assert_eq!(*external.sent.lock().unwrap(), vec!["Once"]);
    }
}

#[test]
fn archive_releases_ownership_for_the_next_task_and_rejects_late_activation() {
    let (app, external) = fixture();
    app.dispatch("a", "w1:p1", "Task A", "op-a").unwrap();
    {
        let mut db = app.db.lock().unwrap();
        db.execute("UPDATE tasks SET status='done' WHERE id='a'", [])
            .unwrap();
        crate::tasks::archive(&mut db, "a").unwrap();
    }
    assert_eq!(app.repair("a", "w1:p1").unwrap_err(), "task.archived");
    assert_eq!(
        app.dispatch("a", "w1:p1", "late", "op-late").unwrap_err(),
        "task.archived"
    );
    assert_eq!(
        app.start_task("a", "late", &uuid::Uuid::new_v4().to_string())
            .unwrap_err(),
        "task.archived"
    );
    assert_eq!(app.target("a", true).unwrap_err(), "task.archived");
    app.dispatch("b", "w1:p1", "Task B", "op-b").unwrap();
    assert_eq!(*external.sent.lock().unwrap(), vec!["Task A", "Task B"]);
}

#[test]
fn closing_a_task_cancels_its_pending_attachment_before_spawning() {
    let (app, _) = fixture();
    app.dispatch("a", "w1:p1", "A", "op").unwrap();
    let manager = crate::native_terminal::NativeTerminalManager::default();
    let view = manager
        .select(
            "late".into(),
            "a".into(),
            crate::native_terminal::Bounds {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 600.0,
                viewport_width: 800.0,
                viewport_height: 600.0,
                visible: true,
            },
            Arc::new(|_| {}),
        )
        .unwrap();
    manager.close_task("a");
    assert_eq!(
        crate::native_terminal::open(view, Arc::new(app), 0).unwrap_err(),
        "terminal.stale"
    );
}
