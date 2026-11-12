use super::*;

#[test]
fn completed_task_can_bind_after_failed_resume_without_replaying() {
    for remote in [false, true] {
        for same_pane in [false, true] {
            let (app, fake) = fixture_on(remote);
            app.start_task("a", "original", &op()).unwrap();
            app.complete_task("a", &op()).unwrap();
            fake.0.lock().unwrap().mismatch = true;
            let resume = op();
            assert_eq!(app.start_task("a", "", &resume).unwrap_err(), "flow.session_mismatch");
            assert_eq!(view(&app.db.lock().unwrap(), "a").unwrap().state.as_deref(), Some("unknown"));
            let mut selected = if same_pane { app.binding("a").unwrap().target.unwrap() } else { target("manual") };
            selected.machine_id = app.herdr.machine_id().into();
            fake.0.lock().unwrap().panes.insert(selected.pane_id.clone(), (selected.clone(), None));
            let old_epoch = crate::task_reset::epoch(&app.db.lock().unwrap(), "a").unwrap();
            app.repair("a", &selected.pane_id).unwrap();
            assert_eq!(status(&app, "a"), "in_progress");
            assert_eq!(saved(&app, "a"), None);
            let permit = app.view_task("a").unwrap();
            assert!(!permit.read_only);
            assert!(permit.target.same_identity(&selected));
            assert!(app.recover_lifecycle("a", &resume).is_err());
            assert!(app.activate("a", &resume, &selected).is_err());
            app.end_flow(&resume, "unknown", "late worker error").unwrap();
            assert!(crate::task_reset::check_epoch(&app.db.lock().unwrap(), "a", old_epoch).is_err());
            let db = app.db.lock().unwrap();
            let old: (String, String) = db.query_row("SELECT state,error FROM lifecycle_operations WHERE id=?1", [&resume], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
            assert_eq!(old, ("failed".into(), "flow.session_mismatch".into()));
            assert_eq!(view(&db, "a").unwrap().state.as_deref(), Some("succeeded"));
            drop(db);
            let path = std::env::temp_dir().join(format!("kanban-resume-binding-{}.db", op()));
            app.db.lock().unwrap().execute("VACUUM INTO ?1", [path.to_str().unwrap()]).unwrap();
            let conn = Connection::open(&path).unwrap();
            crate::db::init(&conn).unwrap();
            crate::db::recover_interrupted(&conn).unwrap();
            let reopened = Workflow { db: Arc::new(Mutex::new(conn)), herdr: fake.clone() };
            reopened.validate_view("a", &reopened.view_task("a").unwrap()).unwrap();
            drop(reopened);
            std::fs::remove_file(path).unwrap();
            let s = fake.0.lock().unwrap();
            assert_eq!(s.sent, vec!["original"]);
            assert_eq!(s.created, 2);
            assert_eq!(s.closed.len(), 1);
        }
    }
}

#[test]
fn asynchronous_resume_repair_is_atomic_and_rechecks_the_operation() {
    for failure in ["database", "operation", "occupied", "none"] {
        let (app, fake) = fixture();
        let app = Arc::new(app);
        app.start_task("a", "original", &op()).unwrap();
        app.complete_task("a", &op()).unwrap();
        fake.0.lock().unwrap().mismatch = true;
        let resume = op();
        app.submit_start("a", &resume, Arc::new(|_| {})).unwrap();
        let flow = await_start(&app, "a");
        assert_eq!(flow.state.as_deref(), Some("unknown"));
        assert!(flow.can_repair_resume);
        let selected = target("manual");
        fake.0.lock().unwrap().panes.insert("manual".into(), (selected.clone(), None));
        let original = app.binding("a").unwrap().target;
        let session = saved(&app, "a");
        match failure {
            "database" => { app.db.lock().unwrap().execute_batch("CREATE TRIGGER reject_repair BEFORE UPDATE OF status ON tasks WHEN new.status='in_progress' BEGIN SELECT RAISE(ABORT,'disk failure'); END;").unwrap(); }
            "operation" => {
                let db = app.db.clone();
                let resume = resume.clone();
                fake.0.lock().unwrap().observe = Some(Arc::new(move |phase| {
                    if phase == "targets" { db.lock().unwrap().execute("UPDATE lifecycle_operations SET state='pending' WHERE id=?1", [&resume]).unwrap(); }
                }));
            }
            "occupied" => { app.reserve_target("b", &{
                let id = op(); app.begin_flow("b", &id, "start").unwrap(); id
            }, &selected).unwrap(); }
            _ => {}
        }
        let result = app.repair("a", "manual");
        if failure == "none" {
            result.unwrap();
            assert_eq!(status(&app, "a"), "in_progress");
            assert!(!app.view_task("a").unwrap().read_only);
        } else {
            assert!(result.is_err(), "{failure}");
            assert_eq!(app.binding("a").unwrap().target, original);
            assert_eq!(saved(&app, "a"), session);
            assert_eq!(status(&app, "a"), "done");
        }
        assert_eq!(fake.0.lock().unwrap().sent, vec!["original"]);
    }
}

#[test]
fn manual_repair_does_not_bypass_pending_resume_or_unknown_prompt() {
    for scenario in ["pending_resume", "unknown_prompt", "unknown_complete"] {
        let (app, fake) = fixture();
        if scenario == "unknown_prompt" {
            fake.0.lock().unwrap().send_error = true;
            assert!(app.start_task("a", "original", &op()).is_err());
        } else {
            app.start_task("a", "original", &op()).unwrap();
            if scenario == "unknown_complete" {
                fake.0.lock().unwrap().close_error = true;
                app.complete_task("a", &op()).unwrap();
            } else {
                app.complete_task("a", &op()).unwrap();
                app.begin_flow("a", &op(), "start").unwrap();
            }
        }
        assert!(!view(&app.db.lock().unwrap(), "a").unwrap().can_repair_resume);
        assert_eq!(app.repair("a", "manual").unwrap_err(), "flow.verify_required");
    }
}

#[test]
fn manual_choice_opens_without_session_evidence_and_survives_database_reopen() {
    for remote in [false, true] {
        for reported in [None, Some(op())] {
            let (app, fake) = fixture_on(remote);
            app.start_task("a", "original", &op()).unwrap();
            let mut selected = target("manual");
            selected.machine_id = app.herdr.machine_id().into();
            fake.0
                .lock()
                .unwrap()
                .panes
                .insert(selected.pane_id.clone(), (selected.clone(), reported));
            fake.0.lock().unwrap().observe = Some(Arc::new(|phase| {
                assert_ne!(
                    phase, "session",
                    "manual binding/view must not depend on session reporting"
                );
            }));
            app.repair("a", &selected.pane_id).unwrap();
            assert_eq!(
                saved(&app, "a"),
                None,
                "old automatic resume pointer must not follow a manual choice"
            );
            let permit = app.view_task("a").unwrap();
            assert!(!permit.read_only);
            app.validate_view("a", &permit).unwrap();
            assert!(permit.target.same_identity(&selected));

            let path = std::env::temp_dir().join(format!("kanban-manual-binding-{}.db", op()));
            app.db
                .lock()
                .unwrap()
                .execute("VACUUM INTO ?1", [path.to_str().unwrap()])
                .unwrap();
            let conn = Connection::open(&path).unwrap();
            crate::db::init(&conn).unwrap();
            let reopened = Workflow {
                db: Arc::new(Mutex::new(conn)),
                herdr: fake.clone(),
            };
            reopened
                .validate_view("a", &reopened.view_task("a").unwrap())
                .unwrap();
            drop(reopened);
            std::fs::remove_file(path).unwrap();
            let s = fake.0.lock().unwrap();
            assert_eq!(s.sent, vec!["original"]);
            assert!(s.closed.is_empty());
            assert_eq!(s.created, 1);
        }
    }
}

#[test]
fn viewing_a_valid_terminal_does_not_require_codex_session_reporting() {
    let (app, fake) = fixture();
    app.start_task("a", "original", &op()).unwrap();
    fake.0.lock().unwrap().observe = Some(Arc::new(|phase| {
        assert_ne!(phase, "session");
    }));
    fake.0.lock().unwrap().missing_id = true;
    let permit = app.view_task("a").unwrap();
    app.validate_view("a", &permit).unwrap();
    fake.0
        .lock()
        .unwrap()
        .panes
        .values_mut()
        .next()
        .unwrap()
        .0
        .process_id = "replaced".into();
    assert!(app.validate_view("a", &permit).is_err());
}

#[test]
fn manually_bound_session_is_collected_when_completing_and_resumes_without_replaying() {
    let (app, fake) = fixture();
    app.start_task("a", "original", &op()).unwrap();
    let chosen = op();
    fake.0
        .lock()
        .unwrap()
        .panes
        .insert("manual".into(), (target("manual"), Some(chosen.clone())));
    app.repair("a", "manual").unwrap();
    assert_eq!(saved(&app, "a"), None);
    app.complete_task("a", &op()).unwrap();
    assert_eq!(saved(&app, "a"), Some(chosen.clone()));
    app.start_task("a", "must not resend", &op()).unwrap();
    let s = fake.0.lock().unwrap();
    assert_eq!(s.prepared.last(), Some(&Some(chosen)));
    assert_eq!(s.sent, vec!["original"]);
}
