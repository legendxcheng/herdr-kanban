use super::*;
use crate::project_visibility as visibility;

fn setup() -> (ProjectService, Arc<FakeHerdr>) {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    let herdr = Arc::new(FakeHerdr::default());
    *herdr.spaces.lock().unwrap() = ["a", "b"].map(navigation_workspace).to_vec();
    (
        ProjectService::new(Arc::new(Mutex::new(db)), herdr.clone()),
        herdr,
    )
}

#[test]
fn discovery_is_not_selection_and_hidden_history_survives_readding() {
    let (service, herdr) = setup();
    assert!(service.sync().unwrap().projects.is_empty());
    assert!(
        !visibility::state(&service.db.lock().unwrap())
            .unwrap()
            .configured
    );
    let choices = service.choices().unwrap();
    assert_eq!(choices.workspaces.len(), 2);
    assert!(choices.workspaces.iter().all(|w| !w.shown));
    service
        .add_existing("default", &["a".into(), "b".into()])
        .unwrap();
    let a = service.sync().unwrap().projects.remove(0);
    let task = crate::tasks::create(
        &service.db.lock().unwrap(),
        &a.id,
        "Keep",
        crate::models::TaskPriority::Normal,
        "{\"keep\":true}",
    )
    .unwrap();
    {
        let db = service.db.lock().unwrap();
        db.execute("INSERT INTO task_sessions(task_id,codex_session_id,cwd,updated_at) VALUES(?1,'exact-session','.','now')", [&task.id]).unwrap();
        db.execute("INSERT INTO lifecycle_operations(id,task_id,kind,state,created_at) VALUES('op',?1,'start','unknown','now')", [&task.id]).unwrap();
        visibility::hide(&db, &a.id).unwrap();
        visibility::hide(&db, &a.id).unwrap();
        assert_eq!(
            visibility::state(&db).unwrap().recovery_projects[0].id,
            a.id
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM workspace_exclusions", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(service.sync().unwrap().projects.len(), 1);
    assert_eq!(herdr.spaces.lock().unwrap().len(), 2);
    service
        .add_existing("default", &["a".into(), "a".into()])
        .unwrap();
    assert_eq!(service.sync().unwrap().projects[0].id, a.id);
    let db = service.db.lock().unwrap();
    assert_eq!(
        crate::tasks::list(&db, &a.id).unwrap()[0].prompt_json,
        "{\"keep\":true}"
    );
    assert_eq!(
        db.query_row(
            "SELECT codex_session_id FROM task_sessions WHERE task_id=?1",
            [&task.id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "exact-session"
    );
    assert_eq!(
        db.query_row(
            "SELECT state FROM lifecycle_operations WHERE id='op'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "unknown"
    );
}

#[test]
fn bulk_add_is_atomic_for_missing_pending_and_database_failure() {
    let (service, _) = setup();
    service.sync().unwrap();
    assert!(
        service
            .add_existing("default", &["a".into(), "missing".into()])
            .is_err()
    );
    assert!(
        visible_list(&service.db.lock().unwrap())
            .unwrap()
            .is_empty()
    );
    service.db.lock().unwrap().execute_batch("CREATE TRIGGER fail_b BEFORE INSERT ON project_visibility WHEN (SELECT workspace_id FROM projects WHERE id=NEW.project_id)='b' BEGIN SELECT RAISE(ABORT,'disk failure'); END;").unwrap();
    assert!(
        service
            .add_existing("default", &["a".into(), "b".into()])
            .is_err()
    );
    assert!(
        visible_list(&service.db.lock().unwrap())
            .unwrap()
            .is_empty()
    );
    service.db.lock().unwrap().execute_batch("DROP TRIGGER fail_b; INSERT INTO workspace_operations(id,session,name,cwd,project_id,workspace_id,state,created_at) VALUES('pending','default','b','.','pending','b','unknown','now');").unwrap();
    assert_eq!(
        service
            .add_existing("default", &["a".into(), "b".into()])
            .unwrap_err(),
        "workspace.verify_required"
    );
    assert!(
        visible_list(&service.db.lock().unwrap())
            .unwrap()
            .is_empty()
    );
    assert!(
        service
            .choices()
            .unwrap()
            .workspaces
            .iter()
            .find(|w| w.workspace.id == "b")
            .unwrap()
            .blocked
    );
    assert!(
        service
            .add_existing("different-session", &["a".into()])
            .is_err()
    );
}

#[test]
fn offline_hide_persists_and_failed_hide_keeps_selection() {
    let (service, herdr) = setup();
    let projects = select_discovered(&service).projects;
    *herdr.offline.lock().unwrap() = true;
    {
        let db = service.db.lock().unwrap();
        visibility::hide(&db, &projects[0].id).unwrap();
        db.execute_batch("CREATE TRIGGER reject_hide BEFORE UPDATE ON project_visibility BEGIN SELECT RAISE(ABORT,'disk failure'); END;").unwrap();
        assert!(visibility::hide(&db, &projects[1].id).is_err());
        assert_eq!(visible_list(&db).unwrap()[0].id, projects[1].id);
    }
    assert!(service.sync().is_err());
    assert!(service.add_existing("default", &["a".into()]).is_err());
    assert_eq!(visible_list(&service.db.lock().unwrap()).unwrap().len(), 1);
}

#[test]
fn create_recovery_shows_once_and_never_overrides_later_hiding() {
    let (service, herdr) = setup();
    *herdr.lose_response.lock().unwrap() = true;
    let operation = uuid::Uuid::new_v4().to_string();
    assert!(service.create(&operation, "Created", ".").is_err());
    let created = service.recover(&operation, None).unwrap();
    assert_eq!(
        visible_list(&service.db.lock().unwrap()).unwrap()[0].id,
        created.id
    );
    visibility::hide(&service.db.lock().unwrap(), &created.id).unwrap();
    service.recover(&operation, None).unwrap();
    service.create(&operation, "Created", ".").unwrap();
    assert!(service.sync().unwrap().projects.is_empty());
    assert_eq!(herdr.spaces.lock().unwrap().len(), 3);
}

#[test]
fn selection_and_exclusions_are_scoped_to_machine_and_session() {
    let (local, _) = setup();
    let remote_herdr = Arc::new(FakeHerdr {
        machine_name: Some("remote".into()),
        ..Default::default()
    });
    *remote_herdr.spaces.lock().unwrap() = vec![navigation_workspace("a")];
    let remote = ProjectService::new(local.db.clone(), remote_herdr.clone());
    local.sync().unwrap();
    remote.sync().unwrap();
    local.add_existing("default", &["a".into()]).unwrap();
    assert!(remote.sync().unwrap().projects.is_empty());
    remote.add_existing("default", &["a".into()]).unwrap();
    let r = remote.sync().unwrap().projects.remove(0);
    visibility::hide(&local.db.lock().unwrap(), &r.id).unwrap();
    assert_eq!(local.sync().unwrap().projects.len(), 1);
    assert!(remote.sync().unwrap().projects.is_empty());
    remote.remove(&r.id).unwrap();
    assert_eq!(remote.choices().unwrap().workspaces.len(), 1);
    remote.add_existing("default", &["a".into()]).unwrap();
    assert_ne!(remote.sync().unwrap().projects[0].id, r.id);
    assert_eq!(local.sync().unwrap().projects.len(), 1);
    assert_eq!(remote_herdr.spaces.lock().unwrap().len(), 1);
}

#[test]
fn schema_twelve_upgrade_keeps_records_and_persists_explicit_empty_selection() {
    let file = std::env::temp_dir().join(format!("kanban-selection-{}.db", uuid::Uuid::new_v4()));
    let db = Connection::open(&file).unwrap();
    crate::db::init_before_machines(&db).unwrap();
    crate::machines::init(&db).unwrap();
    db.execute_batch("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at) VALUES('old','Legacy','.','Legacy','now');").unwrap();
    let task = crate::tasks::create(
        &db,
        "old",
        "Keep",
        crate::models::TaskPriority::Normal,
        "{}",
    )
    .unwrap();
    crate::db::init(&db).unwrap();
    crate::db::init(&db).unwrap();
    assert!(visible_list(&db).unwrap().is_empty());
    assert!(!visibility::state(&db).unwrap().configured);
    visibility::show(&db, "old", true).unwrap();
    visibility::hide(&db, "old").unwrap();
    drop(db);
    let db = Connection::open(&file).unwrap();
    crate::db::init(&db).unwrap();
    assert!(visibility::state(&db).unwrap().configured);
    assert!(visible_list(&db).unwrap().is_empty());
    assert_eq!(crate::tasks::list(&db, "old").unwrap()[0].id, task.id);
    drop(db);
    std::fs::remove_file(file).unwrap();
}
