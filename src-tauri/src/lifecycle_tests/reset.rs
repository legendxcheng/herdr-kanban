use super::*;

fn reset(app: &Workflow, task: &str, request: &str) -> Result<bool, String> {
    let mut db = app.db.lock().unwrap();
    let flow = view(&db, task)?;
    let binding = binding(&db, task)?;
    let epoch = crate::task_reset::epoch(&db, task)?;
    crate::task_reset::reset(
        &mut db,
        task,
        epoch,
        flow.operation_id.as_deref(),
        binding.operation.as_ref().map(|o| o.id.as_str()),
        request,
    )
}

#[test]
fn reset_preserves_task_content_and_history_but_next_start_uses_a_new_session() {
    let (app, fake) = fixture();
    set_prompt(&app, "a", "original prompt");
    let original = crate::tasks::list(&app.db.lock().unwrap(), "p1").unwrap()[0].clone();
    let first = op();
    fake.0.lock().unwrap().send_error = true;
    assert!(app.start_task("a", "original prompt", &first).is_err());
    let session = saved(&app, "a").unwrap();
    let old_target = app.binding("a").unwrap().target.unwrap();
    let request = op();
    assert!(reset(&app, "a", &request).unwrap());
    {
        let db = app.db.lock().unwrap();
        crate::db::init(&db).unwrap();
        crate::db::recover_interrupted(&db).unwrap();
        let task = crate::tasks::list(&db, "p1").unwrap().remove(0);
        assert_eq!(task.id, original.id);
        assert_eq!(task.name, original.name);
        assert_eq!(task.prompt_json, original.prompt_json);
        assert_eq!(task.created_at, original.created_at);
        assert_eq!(task.priority, original.priority);
        assert_eq!(task.codex_settings, original.codex_settings);
        assert_eq!(task.status, crate::models::TaskStatus::Todo);
        assert_eq!(task.execution_epoch, 1);
        assert!(task.codex_settings_editable);
        assert!(task.flow.unwrap().operation_id.is_none());
        let (owner, state, prompt): (String, String, String) = db
            .query_row(
                "SELECT task_id,state,prompt FROM dispatch_operations WHERE id=?1",
                [format!("{first}:dispatch")],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(owner, format!("retired:{request}"));
        assert_eq!(state, "unknown");
        assert_eq!(prompt, "original prompt");
        assert!(crate::task_reset::quarantined(&db, &old_target).unwrap());
    }
    assert!(app.repair("a", &old_target.pane_id).is_err());
    assert!(
        app.dispatch("b", &old_target.pane_id, "must not send", "blocked")
            .is_err()
    );
    assert!(app.recover_lifecycle("a", &first).is_err());
    assert!(
        app.resolve("a", &format!("{first}:dispatch"), true, "late confirmation")
            .is_err()
    );
    fake.0.lock().unwrap().send_error = false;
    set_prompt(&app, "a", "revised prompt");
    app.start_task("a", "revised prompt", &op()).unwrap();
    assert_ne!(saved(&app, "a"), Some(session));
    assert_eq!(fake.0.lock().unwrap().prepared, vec![None, None]);
    assert_eq!(
        fake.0.lock().unwrap().sent,
        vec!["original prompt", "revised prompt"]
    );
    assert!(fake.0.lock().unwrap().closed.is_empty());
    // Retrying an old successful reset cannot touch the new running execution.
    assert!(
        !crate::task_reset::reset(
            &mut app.db.lock().unwrap(),
            "a",
            0,
            Some(&first),
            Some(&format!("{first}:dispatch")),
            &request
        )
        .unwrap()
    );
    assert_eq!(status(&app, "a"), "in_progress");
}

#[test]
fn reset_is_atomic_rejects_stale_confirmation_and_does_not_reset_pending_work() {
    let (app, fake) = fixture();
    let first = op();
    assert!(reset(&app, "a", &op()).is_err());
    app.begin_flow("a", &first, "start").unwrap();
    assert_eq!(reset(&app, "a", &op()).unwrap_err(), "reset.running");
    app.end_flow(&first, "failed", "test failure").unwrap();
    {
        let mut db = app.db.lock().unwrap();
        assert_eq!(
            crate::task_reset::reset(&mut db, "a", 0, Some("stale"), None, &op()).unwrap_err(),
            "reset.stale"
        );
        db.execute_batch("CREATE TRIGGER reset_fail BEFORE UPDATE OF status ON tasks BEGIN SELECT RAISE(ABORT,'disk failure'); END;").unwrap();
    }
    assert!(reset(&app, "a", &op()).is_err());
    {
        let db = app.db.lock().unwrap();
        assert_eq!(view(&db, "a").unwrap().operation_id, Some(first));
        assert_eq!(crate::task_reset::epoch(&db, "a").unwrap(), 0);
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM task_execution_resets", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        db.execute_batch("DROP TRIGGER reset_fail").unwrap();
    }
    assert!(reset(&app, "a", &op()).unwrap());
    assert!(fake.0.lock().unwrap().prepared.is_empty());
    assert!(fake.0.lock().unwrap().sent.is_empty());
}

#[test]
fn retired_workers_cannot_save_sessions_dispatch_activate_or_restore_errors() {
    for phase in ["prepared", "session", "sent"] {
        let (app, fake) = fixture();
        let db = app.db.clone();
        fake.0.lock().unwrap().observe = Some(Arc::new(move |at| {
            if at != phase {
                return;
            }
            let mut db = db.lock().unwrap();
            // Simulate an interrupted/uncertain operation with a late external reply.
            db.execute(
                "UPDATE lifecycle_operations SET state='unknown' WHERE task_id='a'",
                [],
            )
            .unwrap();
            db.execute(
                "UPDATE dispatch_operations SET state='unknown' WHERE task_id='a'",
                [],
            )
            .unwrap();
            let flow = view(&db, "a").unwrap();
            let binding = binding(&db, "a").unwrap();
            crate::task_reset::reset(
                &mut db,
                "a",
                0,
                flow.operation_id.as_deref(),
                binding.operation.as_ref().map(|o| o.id.as_str()),
                &op(),
            )
            .unwrap();
        }));
        assert!(app.start_task("a", "old prompt", &op()).is_err(), "{phase}");
        assert_eq!(status(&app, "a"), "todo", "{phase}");
        assert!(saved(&app, "a").is_none(), "{phase}");
        assert!(app.binding("a").unwrap().target.is_none(), "{phase}");
        assert!(
            view(&app.db.lock().unwrap(), "a")
                .unwrap()
                .operation_id
                .is_none(),
            "{phase}"
        );
        assert_eq!(
            fake.0.lock().unwrap().sent.len(),
            usize::from(phase == "sent")
        );
    }
}

#[test]
fn reset_survives_database_reopen_and_does_not_recover_retired_unknown_operations() {
    let (app, fake) = fixture();
    fake.0.lock().unwrap().prepare_error = true;
    assert!(app.start_task("a", "prompt", &op()).is_err());
    reset(&app, "a", &op()).unwrap();
    let path = std::env::temp_dir().join(format!("kanban-reset-{}.db", op()));
    app.db
        .lock()
        .unwrap()
        .execute("VACUUM main INTO ?1", [path.to_str().unwrap()])
        .unwrap();
    drop(app);
    let db = Connection::open(&path).unwrap();
    crate::db::init(&db).unwrap();
    crate::db::recover_interrupted(&db).unwrap();
    assert_eq!(crate::task_reset::epoch(&db, "a").unwrap(), 1);
    assert!(view(&db, "a").unwrap().operation_id.is_none());
    assert_eq!(binding(&db, "a").unwrap().state, "unbound");
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn reset_after_lost_pane_creation_quarantines_unknown_pane_but_allows_a_proven_new_pane() {
    let (app, fake) = fixture();
    fake.0.lock().unwrap().lost_create_response = true;
    assert!(app.start_task("a", "first", &op()).is_err());
    reset(&app, "a", &op()).unwrap();
    assert_eq!(app.repair("a", "created-1").unwrap_err(), "binding.occupied");
    assert_eq!(app.dispatch("b", "created-1", "must not send", "blocked").unwrap_err(), "binding.occupied");
    fake.0.lock().unwrap().lost_create_response = false;
    app.start_task("a", "new prompt", &op()).unwrap();
    assert_eq!(app.binding("a").unwrap().target.unwrap().pane_id, "created-2");
    assert_eq!(fake.0.lock().unwrap().sent, vec!["new prompt"]);
    assert_eq!(app.repair("b", "created-1").unwrap_err(), "binding.occupied");
}

#[test]
fn reset_keeps_image_ownership_references_and_settings_editable_without_changing_preferences() {
    let (app, _) = fixture();
    let settings = crate::codex_settings::Settings { model: Some("test-model".into()), effort: Some("high".into()) };
    let document = serde_json::json!({"type":"doc","content":[
        {"type":"paragraph","content":[{"type":"mention","attrs":{"kind":"skill","label":"kanban-dev","ref":".agents/skills/kanban-dev/SKILL.md"}}]},
        {"type":"promptImage","attrs":{"imageId":"image-one","name":"reference.png"}}
    ]}).to_string();
    let flow = op();
    {
        let db = app.db.lock().unwrap();
        crate::codex_settings::save(&db,"a",&settings,true).unwrap();
        crate::tasks::update(&db,"a","Title",&document,crate::models::TaskPriority::Urgent).unwrap();
        db.execute_batch("INSERT INTO prompt_images VALUES('image-one','p1','reference.png','image/png',4,1,1,'digest');
            INSERT INTO task_image_refs VALUES('a','image-one');").unwrap();
        db.execute("INSERT INTO lifecycle_operations(id,task_id,kind,state,created_at) VALUES(?1,'a','start','failed','now')",[&flow]).unwrap();
        db.execute("INSERT INTO operation_image_refs VALUES(?1,'image-one')",[&flow]).unwrap();
    }
    reset(&app,"a",&op()).unwrap();
    let db = app.db.lock().unwrap();
    let task=crate::tasks::list(&db,"p1").unwrap().remove(0);
    assert_eq!(task.prompt_json,document);
    assert_eq!(task.priority,crate::models::TaskPriority::Urgent);
    assert_eq!(task.codex_settings,settings);
    assert!(task.codex_settings_editable);
    assert_eq!(crate::codex_settings::last(&db).unwrap(),settings);
    assert_eq!(db.query_row("SELECT COUNT(*) FROM task_image_refs WHERE task_id='a'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(db.query_row("SELECT COUNT(*) FROM operation_image_refs WHERE operation_id=?1",[&flow],|r|r.get::<_,i64>(0)).unwrap(),1);
}
