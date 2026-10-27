use super::*;

fn seed(service: &ProjectService, id: &str, state: &str, machine: &str) {
    service.db.lock().unwrap().execute(
        "INSERT INTO workspace_operations(id,session,name,cwd,project_id,workspace_id,state,error,created_at,machine_id)
         VALUES(?1,'default','Attempt','.',?1,'w1',?2,'workspace.occupied','now',?3)",
        params![id, state, machine],
    ).unwrap();
}

fn fixture() -> (ProjectService, Arc<FakeHerdr>) {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    let herdr = Arc::new(FakeHerdr::default());
    (
        ProjectService::new(Arc::new(Mutex::new(db)), herdr.clone()),
        herdr,
    )
}

#[test]
fn dismiss_conflict_preserves_project_tasks_and_external_workspace() {
    let (service, herdr) = fixture();
    herdr
        .spaces
        .lock()
        .unwrap()
        .push(navigation_workspace("w1"));
    let project = select_discovered(&service).projects.remove(0);
    let task = crate::tasks::create(
        &service.db.lock().unwrap(),
        &project.id,
        "Keep",
        crate::models::TaskPriority::Normal,
        "{}",
    )
    .unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    seed(&service, &id, "unknown", "local");
    assert_eq!(
        service.recover(&id, None).unwrap_err(),
        "workspace.occupied"
    );
    service.dismiss_operation(&id).unwrap();
    service.dismiss_operation(&id).unwrap();
    let snapshot = service.sync().unwrap();
    assert!(snapshot.operations.is_empty());
    assert_eq!(snapshot.projects[0].id, project.id);
    assert_eq!(
        crate::tasks::list(&service.db.lock().unwrap(), &project.id).unwrap()[0].id,
        task.id
    );
    assert_eq!(herdr.spaces.lock().unwrap()[0].name, "w1");
    assert_eq!(
        service.recover(&id, Some("w1")).unwrap_err(),
        "workspace.operation_dismissed"
    );
    assert_eq!(
        service.create(&id, "Attempt", ".").unwrap_err(),
        "workspace.operation_dismissed"
    );
    assert_eq!(service.operation(&id).unwrap().unwrap().state, "unknown");
}

#[test]
fn dismiss_failed_and_remote_offline_records_without_resolving_machine() {
    let (service, herdr) = fixture();
    *herdr.offline.lock().unwrap() = true;
    for machine in ["local", "disabled-or-missing-remote"] {
        let id = uuid::Uuid::new_v4().to_string();
        seed(&service, &id, "failed", machine);
        service.dismiss_operation(&id).unwrap();
        let db = service.db.lock().unwrap();
        assert!(operations(&db, machine, "default").unwrap().is_empty());
        assert_eq!(
            db.query_row(
                "SELECT state,error FROM workspace_operations WHERE id=?1",
                [&id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            )
            .unwrap(),
            ("failed".into(), "workspace.occupied".into())
        );
    }
    assert!(herdr.spaces.lock().unwrap().is_empty());
}

#[test]
fn dismiss_rechecks_state_after_concurrent_recovery_and_rejects_pending() {
    let (service, _) = fixture();
    seed(&service, "pending", "pending", "local");
    assert_eq!(
        service.dismiss_operation("pending").unwrap_err(),
        "workspace.dismiss_unavailable"
    );
    seed(&service, "race", "unknown", "local");
    let service = Arc::new(service);
    let guard = service.gate.lock().unwrap();
    let worker = service.clone();
    let thread = std::thread::spawn(move || worker.dismiss_operation("race"));
    // Model the successful recovery commit while it still owns the shared gate.
    service
        .db
        .lock()
        .unwrap()
        .execute(
            "UPDATE workspace_operations SET state='succeeded' WHERE id='race'",
            [],
        )
        .unwrap();
    drop(guard);
    assert_eq!(
        thread.join().unwrap().unwrap_err(),
        "workspace.dismiss_unavailable"
    );
    assert_eq!(
        service
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM workspace_operation_dismissals",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn dismiss_failure_rolls_back_and_allows_retry() {
    let (service, _) = fixture();
    seed(&service, "fail", "failed", "local");
    service.db.lock().unwrap().execute_batch("CREATE TRIGGER reject_dismiss BEFORE INSERT ON workspace_operation_dismissals BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    assert!(service.dismiss_operation("fail").is_err());
    assert_eq!(service.operations().unwrap().len(), 1);
    service
        .db
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_dismiss;")
        .unwrap();
    service.dismiss_operation("fail").unwrap();
    assert!(service.operations().unwrap().is_empty());
}

#[test]
fn dismiss_upgrade_and_restart_preserve_evidence_and_do_not_resurrect() {
    let path = std::env::temp_dir().join(format!("kanban-dismiss-{}.db", uuid::Uuid::new_v4()));
    let id = uuid::Uuid::new_v4().to_string();
    {
        let db = Connection::open(&path).unwrap();
        crate::db::init(&db).unwrap();
        db.execute_batch("DROP TABLE workspace_operation_dismissals; DELETE FROM schema_migrations WHERE version=17;").unwrap();
        let service = ProjectService::new(Arc::new(Mutex::new(db)), Arc::new(FakeHerdr::default()));
        seed(&service, &id, "failed", "local");
        crate::db::init(&service.db.lock().unwrap()).unwrap();
        service.dismiss_operation(&id).unwrap();
    }
    {
        let db = Connection::open(&path).unwrap();
        crate::db::init(&db).unwrap();
        crate::db::recover_interrupted(&db).unwrap();
        let service = ProjectService::new(Arc::new(Mutex::new(db)), Arc::new(FakeHerdr::default()));
        assert!(service.operations().unwrap().is_empty());
        assert_eq!(
            service.create(&id, "Attempt", ".").unwrap_err(),
            "workspace.operation_dismissed"
        );
        assert_eq!(
            service.operation(&id).unwrap().unwrap().error,
            "workspace.occupied"
        );
    }
    std::fs::remove_file(path).unwrap();
}
