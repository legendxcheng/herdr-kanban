use super::*;

fn setup_reorder() -> (ProjectService, Arc<FakeHerdr>, Vec<Project>) {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    let herdr = Arc::new(FakeHerdr::default());
    *herdr.spaces.lock().unwrap() = ["a", "b", "c", "d"].map(navigation_workspace).to_vec();
    let service = ProjectService::new(Arc::new(Mutex::new(db)), herdr.clone());
    let projects = service.sync().unwrap().projects;
    (service, herdr, projects)
}
fn names(projects: &[Project]) -> Vec<&str> {
    projects.iter().map(|p| p.name.as_str()).collect()
}

#[test]
fn drag_order_changes_herdr_and_durable_navigation_with_task_history_intact() {
    let (service, herdr, projects) = setup_reorder();
    let task = crate::tasks::create(
        &service.db.lock().unwrap(),
        &projects[2].id,
        "Keep prompt",
        crate::models::TaskPriority::Normal,
        "{\"keep\":true}",
    )
    .unwrap();
    let moved = service
        .move_project(&projects[2].id, Some(&projects[0].id))
        .unwrap();
    assert_eq!(names(&moved.projects), ["c", "a", "b", "d"]);
    assert_eq!(
        herdr
            .spaces
            .lock()
            .unwrap()
            .iter()
            .map(|w| w.id.clone())
            .collect::<Vec<_>>(),
        ["c", "a", "b", "d"]
    );
    assert_eq!(moved.projects[0].id, projects[2].id);
    assert_eq!(
        names(&visible_list(&service.db.lock().unwrap()).unwrap()),
        ["c", "a", "b", "d"]
    );
    assert_eq!(
        crate::tasks::list(&service.db.lock().unwrap(), &projects[2].id).unwrap()[0].prompt_json,
        task.prompt_json
    );
    assert_eq!(
        names(
            &service
                .move_project(&projects[2].id, None)
                .unwrap()
                .projects
        ),
        ["a", "b", "d", "c"]
    );
    // External Herdr changes remain authoritative after a Kanban drag.
    herdr.spaces.lock().unwrap().rotate_left(1);
    assert_eq!(
        names(&service.sync().unwrap().projects),
        ["b", "d", "c", "a"]
    );
}

#[test]
fn moving_visible_workspace_keeps_excluded_workspaces_and_unknown_outcomes_are_only_read_back() {
    let (service, herdr, projects) = setup_reorder();
    service.remove(&projects[1].id).unwrap();
    *herdr.lose_response.lock().unwrap() = true;
    assert_eq!(
        service
            .move_project(&projects[3].id, Some(&projects[0].id))
            .err()
            .unwrap(),
        "herdr.timeout"
    );
    assert_eq!(herdr.moves.lock().unwrap().len(), 1);
    assert_eq!(
        names(&visible_list(&service.db.lock().unwrap()).unwrap()),
        ["d", "a", "c"]
    );
    assert_eq!(
        herdr
            .spaces
            .lock()
            .unwrap()
            .iter()
            .map(|w| w.id.clone())
            .collect::<Vec<_>>(),
        ["d", "a", "b", "c"]
    );
    assert_eq!(names(&service.sync().unwrap().projects), ["d", "a", "c"]);
    assert_eq!(herdr.moves.lock().unwrap().len(), 1);
}

#[test]
fn disappeared_targets_session_mismatches_and_offline_requests_never_move_another_workspace() {
    let (service, herdr, projects) = setup_reorder();
    herdr.spaces.lock().unwrap().remove(0);
    assert!(
        service
            .move_project(&projects[2].id, Some(&projects[0].id))
            .is_err()
    );
    assert_eq!(
        names(&visible_list(&service.db.lock().unwrap()).unwrap()),
        ["b", "c", "d"]
    );
    assert!(
        service
            .move_project(&projects[2].id, Some(&projects[2].id))
            .is_err()
    );
    service
        .db
        .lock()
        .unwrap()
        .execute(
            "UPDATE projects SET herdr_session='other' WHERE id=?1",
            [&projects[2].id],
        )
        .unwrap();
    assert_eq!(
        service.move_project(&projects[2].id, None).err().unwrap(),
        "workspace.session_mismatch"
    );
    *herdr.offline.lock().unwrap() = true;
    assert!(service.move_project(&projects[1].id, None).is_err());
    assert!(herdr.moves.lock().unwrap().is_empty());
}

#[test]
fn local_commit_failure_recovers_actual_order_on_next_sync_without_resending() {
    let (service, herdr, projects) = setup_reorder();
    service.db.lock().unwrap().execute_batch("CREATE TRIGGER fail_reorder BEFORE UPDATE OF workspace_order ON projects BEGIN SELECT RAISE(ABORT,'disk failure'); END;").unwrap();
    assert!(
        service
            .move_project(&projects[3].id, Some(&projects[0].id))
            .is_err()
    );
    assert_eq!(
        names(&visible_list(&service.db.lock().unwrap()).unwrap()),
        ["a", "b", "c", "d"]
    );
    service
        .db
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_reorder")
        .unwrap();
    assert_eq!(
        names(&service.sync().unwrap().projects),
        ["d", "a", "b", "c"]
    );
    assert_eq!(herdr.moves.lock().unwrap().len(), 1);
}

#[test]
fn dragged_order_survives_database_reopen_and_offline_sync() {
    let path = std::env::temp_dir().join(format!("kanban-reorder-{}.db", uuid::Uuid::new_v4()));
    let db = Connection::open(&path).unwrap();
    crate::db::init(&db).unwrap();
    let herdr = Arc::new(FakeHerdr::default());
    *herdr.spaces.lock().unwrap() = ["a", "b"].map(navigation_workspace).to_vec();
    let service = ProjectService::new(Arc::new(Mutex::new(db)), herdr.clone());
    let projects = service.sync().unwrap().projects;
    service
        .move_project(&projects[1].id, Some(&projects[0].id))
        .unwrap();
    drop(service);
    let db = Connection::open(&path).unwrap();
    crate::db::init(&db).unwrap();
    let service = ProjectService::new(Arc::new(Mutex::new(db)), herdr.clone());
    *herdr.offline.lock().unwrap() = true;
    assert!(service.sync().is_err());
    assert_eq!(
        names(&visible_list(&service.db.lock().unwrap()).unwrap()),
        ["b", "a"]
    );
    assert_eq!(herdr.moves.lock().unwrap().len(), 1);
    drop(service);
    std::fs::remove_file(path).unwrap();
}
