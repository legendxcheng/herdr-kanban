use super::*;

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
