use super::*;

#[test]
fn first_prompt_is_carried_by_one_launch_without_terminal_submission() {
    let (app, fake) = fixture();
    let id = op();
    app.start_task("a", "中文\n\n\t$skill", &id).unwrap();
    let state = fake.0.lock().unwrap();
    assert_eq!(state.launch_prompts, ["中文\n\n\t$skill"]);
    assert_eq!(state.created, 1);
    assert!(state.input_requests.is_empty());
    drop(state);
    assert_eq!(status(&app, "a"), "in_progress");
}

#[test]
fn intent_is_durable_before_launch_and_status_changes_never_submit_again() {
    for status in ["working", "idle", "unknown", "done"] {
        let (app, fake) = fixture();
        let db = app.db.clone();
        let weak = Arc::downgrade(&fake);
        fake.0.lock().unwrap().observe = Some(Arc::new(move |phase| {
            if phase == "launching" {
                let db = db.lock().unwrap();
                let row: (String, String) = db
                    .query_row("SELECT prompt,state FROM dispatch_operations", [], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })
                    .unwrap();
                assert_eq!(row, ("frozen".into(), "unknown".into()));
            }
            if phase == "prepared" {
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
                    .status = status.into();
            }
        }));
        let id = op();
        app.start_task("a", "frozen", &id).unwrap();
        app.start_task("a", "frozen", &id).unwrap();
        let state = fake.0.lock().unwrap();
        assert_eq!(state.launch_prompts, ["frozen"]);
        assert!(state.input_requests.is_empty());
    }
}

#[test]
fn lost_launch_response_with_receipt_succeeds_even_while_working() {
    let (app, fake) = fixture();
    {
        let mut s = fake.0.lock().unwrap();
        s.send_error = true;
        s.receipt_available = true;
    }
    app.start_task("a", "once", &op()).unwrap();
    assert_eq!(status(&app, "a"), "in_progress");
    assert_eq!(fake.0.lock().unwrap().launch_prompts, ["once"]);
}

#[test]
fn lost_response_survives_real_database_reopen_and_late_receipt_without_relaunch() {
    let (app, fake) = fixture();
    fake.0.lock().unwrap().send_error = true;
    let id = op();
    assert!(app.start_task("a", "frozen", &id).is_err());
    assert!(
        !view(&app.db.lock().unwrap(), "a")
            .unwrap()
            .can_continue_first_prompt
    );
    assert!(app.continue_first_prompt("a", &id, 0).is_err());
    let file = std::env::temp_dir().join(format!("startup-{}.db", op()));
    app.db
        .lock()
        .unwrap()
        .execute("VACUUM INTO ?1", [file.to_str().unwrap()])
        .unwrap();
    drop(app);
    let conn = Connection::open(&file).unwrap();
    crate::db::init(&conn).unwrap();
    crate::db::recover_interrupted(&conn).unwrap();
    let app = Workflow {
        db: Arc::new(Mutex::new(conn)),
        herdr: fake.clone(),
    };
    assert!(app.start_task("a", "edited", &op()).is_err());
    fake.0.lock().unwrap().receipt_available = true;
    app.recover_lifecycle("a", &id).unwrap();
    assert_eq!(status(&app, "a"), "in_progress");
    assert_eq!(fake.0.lock().unwrap().launch_prompts, ["frozen"]);
    drop(app);
    std::fs::remove_file(file).unwrap();
}

#[test]
fn receipt_commit_failure_is_recoverable_and_does_not_reexecute() {
    let (app, fake) = fixture();
    app.db.lock().unwrap().execute_batch("CREATE TRIGGER fail_start BEFORE UPDATE OF status ON tasks BEGIN SELECT RAISE(ABORT,'disk failed'); END;").unwrap();
    let id = op();
    assert!(app.start_task("a", "accepted", &id).is_err());
    assert_eq!(
        app.binding("a").unwrap().operation.unwrap().state,
        "unknown"
    );
    app.db
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_start")
        .unwrap();
    app.recover_lifecycle("a", &id).unwrap();
    assert_eq!(status(&app, "a"), "in_progress");
    assert_eq!(fake.0.lock().unwrap().launch_prompts, ["accepted"]);
}

#[test]
fn failure_to_persist_intent_never_calls_launch() {
    let (app, fake) = fixture();
    app.db.lock().unwrap().execute_batch("CREATE TRIGGER fail_intent BEFORE INSERT ON dispatch_operations BEGIN SELECT RAISE(ABORT,'disk failed'); END;").unwrap();
    assert!(app.start_task("a", "never", &op()).is_err());
    assert!(fake.0.lock().unwrap().launch_prompts.is_empty());
}

#[test]
fn durable_not_launched_evidence_after_reopen_releases_only_the_failed_attempt() {
    let (app, fake) = fixture();
    fake.0.lock().unwrap().prepare_error = true;
    let id = op();
    assert!(app.start_task("a", "never executed", &id).is_err());
    let file = std::env::temp_dir().join(format!("startup-failure-{}.db", op()));
    app.db
        .lock()
        .unwrap()
        .execute("VACUUM INTO ?1", [file.to_str().unwrap()])
        .unwrap();
    drop(app);
    let conn = Connection::open(&file).unwrap();
    crate::db::init(&conn).unwrap();
    crate::db::recover_interrupted(&conn).unwrap();
    let app = Workflow {
        db: Arc::new(Mutex::new(conn)),
        herdr: fake.clone(),
    };
    fake.0.lock().unwrap().startup_failure =
        Some("startup.not_launched:startup.native_codex_missing".into());
    app.recover_lifecycle("a", &id).unwrap();
    assert_eq!(
        view(&app.db.lock().unwrap(), "a").unwrap().state.as_deref(),
        Some("failed")
    );
    assert!(app.binding("a").unwrap().target.is_none());
    assert!(fake.0.lock().unwrap().launch_prompts.is_empty());
    {
        let mut s = fake.0.lock().unwrap();
        s.prepare_error = false;
        s.startup_failure = None;
    }
    app.start_task("a", "retry", &op()).unwrap();
    assert_eq!(fake.0.lock().unwrap().launch_prompts, ["retry"]);
    drop(app);
    std::fs::remove_file(file).unwrap();
}
