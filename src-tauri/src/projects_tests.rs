use super::*;
use crate::workflow::{Herdr, SendError, Target};
use std::sync::{Arc, Mutex};

#[path = "project_reorder_tests.rs"]
mod reorder;

#[derive(Default)]
struct FakeHerdr {
    machine_name: Option<String>,
    spaces: Mutex<Vec<Workspace>>,
    lose_response: Mutex<bool>,
    offline: Mutex<bool>,
    reject_create: Mutex<bool>,
    session_name: Option<String>,
    moves: Mutex<Vec<(String, Option<String>)>>,
}

#[test]
fn machines_with_the_same_workspace_identity_keep_separate_projects_and_presence() {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    let db = Arc::new(Mutex::new(db));
    let local = Arc::new(FakeHerdr::default());
    let remote = Arc::new(FakeHerdr {
        machine_name: Some("devpc".into()),
        ..Default::default()
    });
    *local.spaces.lock().unwrap() = vec![navigation_workspace("w1")];
    *remote.spaces.lock().unwrap() = vec![navigation_workspace("w1")];
    let local_service = ProjectService::new(db.clone(), local.clone());
    let remote_service = ProjectService::new(db.clone(), remote.clone());
    let local_id = local_service.sync().unwrap().projects[0].id.clone();
    remote_service.sync().unwrap();
    let projects = visible_list(&db.lock().unwrap()).unwrap();
    assert_eq!(projects.len(), 2);
    assert!(
        projects
            .iter()
            .any(|p| p.id == local_id && p.machine_id == "local")
    );
    assert!(
        projects
            .iter()
            .any(|p| p.id != local_id && p.machine_id == "devpc")
    );
    let created = remote_service
        .create(&uuid::Uuid::new_v4().to_string(), "Remote only", ".")
        .unwrap();
    assert_eq!(created.machine_id, "devpc");
    remote.spaces.lock().unwrap().clear();
    remote_service.sync().unwrap();
    assert_eq!(visible_list(&db.lock().unwrap()).unwrap()[0].id, local_id);
}

fn navigation_workspace(id: &str) -> Workspace {
    Workspace {
        id: id.into(),
        name: id.into(),
        directory: Some(".".into()),
    }
}

#[test]
fn navigation_cache_survives_upgrade_reopen_and_offline_without_resurrecting_closed_projects() {
    let path = std::env::temp_dir().join(format!("kanban-navigation-{}.db", uuid::Uuid::new_v4()));
    let db = Connection::open(&path).unwrap();
    crate::db::init(&db).unwrap();
    // Recreate the previous schema and an unlinked legacy project before upgrading.
    db.execute_batch("ALTER TABLE projects DROP COLUMN workspace_order; DELETE FROM schema_migrations WHERE version=8;
        INSERT INTO projects(id,name,root_path,herdr_workspace,created_at) VALUES ('legacy','Legacy','.','Legacy','old');").unwrap();
    crate::db::init(&db).unwrap();
    let herdr = Arc::new(FakeHerdr::default());
    *herdr.spaces.lock().unwrap() = vec![
        navigation_workspace("a"),
        navigation_workspace("b"),
        navigation_workspace("c"),
    ];
    let service = ProjectService::new(Arc::new(Mutex::new(db)), herdr.clone());
    service.sync().unwrap();
    *herdr.spaces.lock().unwrap() = vec![navigation_workspace("c"), navigation_workspace("a")];
    let expected = service
        .sync()
        .unwrap()
        .projects
        .iter()
        .map(|p| p.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        service
            .sync()
            .unwrap()
            .projects
            .iter()
            .map(|p| p.name.clone())
            .collect::<Vec<_>>(),
        vec!["c", "a", "Legacy"]
    );
    drop(service);
    let db = Connection::open(&path).unwrap();
    crate::db::init(&db).unwrap();
    let service = ProjectService::new(Arc::new(Mutex::new(db)), herdr.clone());
    assert_eq!(
        visible_list(&service.db.lock().unwrap())
            .unwrap()
            .iter()
            .map(|p| p.id.clone())
            .collect::<Vec<_>>(),
        expected
    );
    *herdr.offline.lock().unwrap() = true;
    assert!(service.sync().is_err());
    assert_eq!(
        visible_list(&service.db.lock().unwrap())
            .unwrap()
            .iter()
            .map(|p| p.id.clone())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(list(&service.db.lock().unwrap()).unwrap().len(), 4);
    *herdr.offline.lock().unwrap() = false;
    herdr.spaces.lock().unwrap().clear();
    assert_eq!(service.sync().unwrap().projects[0].id, "legacy");
    assert_eq!(visible_list(&service.db.lock().unwrap()).unwrap().len(), 1);
    drop(service);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn navigation_failed_sync_transaction_keeps_last_order_and_visibility() {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    let herdr = Arc::new(FakeHerdr::default());
    *herdr.spaces.lock().unwrap() = vec![navigation_workspace("a"), navigation_workspace("b")];
    let service = ProjectService::new(Arc::new(Mutex::new(db)), herdr.clone());
    let expected = service
        .sync()
        .unwrap()
        .projects
        .iter()
        .map(|p| p.id.clone())
        .collect::<Vec<_>>();
    service.db.lock().unwrap().execute_batch("CREATE TRIGGER fail_navigation BEFORE UPDATE OF workspace_order ON projects BEGIN SELECT RAISE(ABORT,'disk failure'); END;").unwrap();
    *herdr.spaces.lock().unwrap() = vec![navigation_workspace("b")];
    assert!(service.sync().is_err());
    let cached = visible_list(&service.db.lock().unwrap()).unwrap();
    assert_eq!(
        cached.iter().map(|p| p.id.clone()).collect::<Vec<_>>(),
        expected
    );
    assert!(cached.iter().all(|p| p.sync_status == "ready"));
}

#[test]
fn navigation_online_project_with_unresolved_creation_is_not_mistaken_for_closed() {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    let herdr = Arc::new(FakeHerdr::default());
    *herdr.spaces.lock().unwrap() = vec![navigation_workspace("a")];
    let service = ProjectService::new(Arc::new(Mutex::new(db)), herdr.clone());
    let project = service.sync().unwrap().projects.remove(0);
    service.db.lock().unwrap().execute("INSERT INTO workspace_operations(id,session,name,cwd,project_id,workspace_id,state,created_at) VALUES ('pending','default','a','.',?1,'a','unknown','now')",[&project.id]).unwrap();
    let snapshot = service.sync().unwrap();
    assert_eq!(snapshot.projects[0].id, project.id);
    assert_eq!(snapshot.projects[0].sync_status, "ready");
    assert_eq!(snapshot.operations[0].state, "unknown");
    herdr.spaces.lock().unwrap().clear();
    assert!(service.sync().unwrap().projects.is_empty());
    assert_eq!(service.operations().unwrap()[0].state, "unknown");
}

#[test]
fn navigation_follows_herdr_reordering_and_insertion_without_changing_project_ids() {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    let herdr = Arc::new(FakeHerdr::default());
    *herdr.spaces.lock().unwrap() = vec![navigation_workspace("a"), navigation_workspace("b")];
    let service = ProjectService::new(Arc::new(Mutex::new(db)), herdr.clone());
    let first = service.sync().unwrap();
    *herdr.spaces.lock().unwrap() = vec![
        navigation_workspace("b"),
        navigation_workspace("c"),
        navigation_workspace("a"),
    ];
    let next = service.sync().unwrap();
    assert_eq!(
        next.projects
            .iter()
            .map(|p| p.workspace_id.as_deref().unwrap())
            .collect::<Vec<_>>(),
        vec!["b", "c", "a"]
    );
    assert_eq!(next.projects[0].id, first.projects[1].id);
    assert_eq!(next.projects[2].id, first.projects[0].id);
    assert_eq!(
        service
            .sync()
            .unwrap()
            .projects
            .iter()
            .map(|p| p.id.clone())
            .collect::<Vec<_>>(),
        next.projects
            .iter()
            .map(|p| p.id.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn navigation_hides_closed_workspace_and_restores_only_the_same_identity_with_history() {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    let herdr = Arc::new(FakeHerdr::default());
    *herdr.spaces.lock().unwrap() = vec![navigation_workspace("a"), navigation_workspace("b")];
    let service = ProjectService::new(Arc::new(Mutex::new(db)), herdr.clone());
    let first = service.sync().unwrap();
    let project = &first.projects[0];
    let task = crate::tasks::create(
        &service.db.lock().unwrap(),
        &project.id,
        "Keep prompt",
        crate::models::TaskPriority::High,
        "{\"keep\":true}",
    )
    .unwrap();
    service.db.lock().unwrap().execute("INSERT INTO task_sessions(task_id,codex_session_id,cwd,updated_at) VALUES (?1,'saved-session','.','now')", [&task.id]).unwrap();
    herdr.spaces.lock().unwrap().remove(0);
    assert_eq!(
        service
            .sync()
            .unwrap()
            .projects
            .iter()
            .map(|p| p.id.clone())
            .collect::<Vec<_>>(),
        vec![first.projects[1].id.clone()]
    );
    assert_eq!(service.get(&project.id).unwrap().sync_status, "missing");
    herdr.spaces.lock().unwrap().push(Workspace {
        id: "new-a".into(),
        name: "a".into(),
        directory: Some(".".into()),
    });
    let new = service
        .sync()
        .unwrap()
        .projects
        .into_iter()
        .find(|p| p.name == "a")
        .unwrap();
    assert_ne!(new.id, project.id);
    assert!(
        crate::tasks::list(&service.db.lock().unwrap(), &new.id)
            .unwrap()
            .is_empty()
    );
    herdr.spaces.lock().unwrap().push(navigation_workspace("a"));
    assert!(
        service
            .sync()
            .unwrap()
            .projects
            .iter()
            .any(|p| p.id == project.id)
    );
    let tasks = crate::tasks::list(&service.db.lock().unwrap(), &project.id).unwrap();
    assert_eq!(tasks[0].id, task.id);
    assert_eq!(tasks[0].prompt_json, "{\"keep\":true}");
    assert_eq!(
        service
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT codex_session_id FROM task_sessions WHERE task_id=?1",
                [&task.id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "saved-session"
    );
}
impl Herdr for FakeHerdr {
    fn machine_id(&self) -> &str {
        self.machine_name.as_deref().unwrap_or("local")
    }
    fn session(&self) -> &str {
        self.session_name.as_deref().unwrap_or("default")
    }
    fn workspaces(&self) -> Result<Vec<Workspace>, String> {
        if *self.offline.lock().unwrap() {
            Err("herdr.unavailable".into())
        } else {
            Ok(self.spaces.lock().unwrap().clone())
        }
    }
    fn targets(&self, _: &str) -> Result<Vec<Target>, String> {
        Ok(vec![])
    }
    fn move_workspace(&self, id: &str, before: Option<&str>) -> Result<Vec<Workspace>, String> {
        self.moves
            .lock()
            .unwrap()
            .push((id.into(), before.map(str::to_owned)));
        let mut spaces = self.spaces.lock().unwrap();
        let index = spaces
            .iter()
            .position(|w| w.id == id)
            .ok_or("workspace.missing")?;
        let moved = spaces.remove(index);
        let index = before
            .map(|id| {
                spaces
                    .iter()
                    .position(|w| w.id == id)
                    .ok_or("workspace.missing")
            })
            .transpose()?
            .unwrap_or(spaces.len());
        spaces.insert(index, moved);
        if *self.lose_response.lock().unwrap() {
            Err("herdr.timeout".into())
        } else {
            Ok(spaces.clone())
        }
    }
    fn send(&self, _: &Target, _: &str) -> Result<(), SendError> {
        unreachable!()
    }
    fn create_workspace(&self, name: &str, cwd: &str) -> Result<Workspace, SendError> {
        if *self.reject_create.lock().unwrap() {
            return Err(SendError {
                message: "cannot spawn".into(),
                uncertain: false,
            });
        }
        let mut spaces = self.spaces.lock().unwrap();
        let ws = Workspace {
            id: format!("w{}", spaces.len() + 1),
            name: name.into(),
            directory: Some(cwd.into()),
        };
        spaces.push(ws.clone());
        if *self.lose_response.lock().unwrap() {
            Err(SendError {
                message: "herdr.timeout".into(),
                uncertain: true,
            })
        } else {
            Ok(ws)
        }
    }
    fn rename_workspace(&self, id: &str, name: &str) -> Result<(), String> {
        self.spaces
            .lock()
            .unwrap()
            .iter_mut()
            .find(|w| w.id == id)
            .ok_or("missing")?
            .name = name.into();
        Ok(())
    }
}
#[test]
fn imports_workspace_once_and_follows_rename_by_identity() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    let herdr = Arc::new(FakeHerdr::default());
    herdr.spaces.lock().unwrap().push(Workspace {
        id: "w1".into(),
        name: "Alpha".into(),
        directory: Some("E:/Alpha".into()),
    });
    let service = ProjectService::new(Arc::new(Mutex::new(db)), herdr.clone());
    let first = service.sync().unwrap();
    assert_eq!(first.projects.len(), 1);
    assert_eq!(first.projects[0].root_path, "E:/Alpha");
    herdr.spaces.lock().unwrap()[0].name = "Renamed".into();
    let second = service.sync().unwrap();
    assert_eq!(second.projects.len(), 1);
    assert_eq!(second.projects[0].id, first.projects[0].id);
    assert_eq!(second.projects[0].name, "Renamed");
    assert_eq!(second.projects[0].workspace_id.as_deref(), Some("w1"));
}
#[test]
fn upgrades_unique_legacy_project_without_losing_tasks() {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    db.execute_batch("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at) VALUES ('old','My board','e:/alpha/','Alpha','now');").unwrap();
    let task = crate::tasks::create(
        &db,
        "old",
        "Keep me",
        crate::models::TaskPriority::High,
        "{}",
    )
    .unwrap();
    let herdr = Arc::new(FakeHerdr::default());
    herdr.spaces.lock().unwrap().push(Workspace {
        id: "w1".into(),
        name: "Alpha".into(),
        directory: Some("E:/Alpha".into()),
    });
    let service = ProjectService::new(Arc::new(Mutex::new(db)), herdr);
    let snapshot = service.sync().unwrap();
    assert_eq!(snapshot.projects.len(), 1);
    assert_eq!(snapshot.projects[0].id, "old");
    assert_eq!(snapshot.projects[0].directory_source, "herdr");
    assert_eq!(snapshot.projects[0].workspace_id.as_deref(), Some("w1"));
    assert_eq!(
        crate::tasks::list(&service.db.lock().unwrap(), "old").unwrap()[0].id,
        task.id
    );
}
#[test]
fn lost_create_response_recovers_after_restart_without_creating_twice() {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    let db = Arc::new(Mutex::new(db));
    let herdr = Arc::new(FakeHerdr::default());
    *herdr.lose_response.lock().unwrap() = true;
    let op = uuid::Uuid::new_v4().to_string();
    let service = ProjectService::new(db.clone(), herdr.clone());
    assert!(service.create(&op, "New project", ".").is_err());
    drop(service);
    crate::db::recover_interrupted(&db.lock().unwrap()).unwrap();
    let service = ProjectService::new(db, herdr.clone());
    let result = service.recover(&op, None).unwrap();
    assert_eq!(result.name, "New project");
    assert_eq!(herdr.spaces.lock().unwrap().len(), 1);
    assert_eq!(
        service.create(&op, "New project", ".").unwrap().id,
        result.id
    );
    assert_eq!(service.sync().unwrap().projects.len(), 1);
}
#[test]
fn sync_does_not_import_an_unresolved_create_or_erase_a_missing_project() {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    let herdr = Arc::new(FakeHerdr::default());
    let service = ProjectService::new(Arc::new(Mutex::new(db)), herdr.clone());
    *herdr.lose_response.lock().unwrap() = true;
    let op = uuid::Uuid::new_v4().to_string();
    assert!(service.create(&op, "New project", ".").is_err());
    assert!(service.sync().unwrap().projects.is_empty());
    let project = service.recover(&op, None).unwrap();
    herdr.spaces.lock().unwrap().clear();
    let snapshot = service.sync().unwrap();
    assert!(snapshot.projects.is_empty());
    assert_eq!(service.get(&project.id).unwrap().sync_status, "missing");
}
#[test]
fn rename_updates_herdr_and_removal_stays_local_until_explicitly_linked() {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    let herdr = Arc::new(FakeHerdr::default());
    let service = ProjectService::new(Arc::new(Mutex::new(db)), herdr.clone());
    let p = service
        .create(&uuid::Uuid::new_v4().to_string(), "Alpha", ".")
        .unwrap();
    service.update(&p.id, "Beta", None).unwrap();
    assert_eq!(herdr.spaces.lock().unwrap()[0].name, "Beta");
    service.remove(&p.id).unwrap();
    assert!(service.sync().unwrap().projects.is_empty());
    assert_eq!(herdr.spaces.lock().unwrap().len(), 1);
    let linked = service
        .link(None, p.workspace_id.as_deref().unwrap(), None)
        .unwrap();
    assert_eq!(linked.name, "Beta");
    assert_eq!(service.sync().unwrap().projects.len(), 1);
}
#[test]
fn legacy_binding_identity_wins_over_a_stale_name_even_during_recovery() {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    db.execute_batch("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at) VALUES ('old','Old','E:/Alpha','Old name','now');").unwrap();
    let task = crate::tasks::create(
        &db,
        "old",
        "Keep",
        crate::models::TaskPriority::Normal,
        "{}",
    )
    .unwrap();
    let target = Target {
        machine_id: crate::machines::local_id(),
        session: "default".into(),
        workspace_id: "w1".into(),
        pane_id: "w1:p1".into(),
        terminal_id: "t".into(),
        agent: "codex".into(),
        process_id: "1".into(),
        codex_resume_locks: Vec::new(),
        title: "".into(),
        status: "idle".into(),
    };
    db.execute("INSERT INTO task_bindings(task_id,session,pane_id,target_json,state) VALUES (?1,'default','w1:p1',?2,'valid')",params![task.id,serde_json::to_string(&target).unwrap()]).unwrap();
    db.execute("INSERT INTO lifecycle_operations(id,task_id,kind,state,target_json,created_at) VALUES ('old-op',?1,'complete','unknown',?2,'now')",params![task.id,serde_json::to_string(&target).unwrap()]).unwrap();
    let herdr = Arc::new(FakeHerdr::default());
    herdr.spaces.lock().unwrap().push(Workspace {
        id: "w1".into(),
        name: "Renamed".into(),
        directory: Some("E:/Alpha".into()),
    });
    let service = ProjectService::new(Arc::new(Mutex::new(db)), herdr);
    let snapshot = service.sync().unwrap();
    assert_eq!(snapshot.projects.len(), 1);
    assert_eq!(snapshot.projects[0].id, "old");
    assert_eq!(snapshot.projects[0].directory_source, "herdr");
    assert_eq!(snapshot.projects[0].name, "Renamed");
}
#[test]
fn directory_fallback_is_explicit_and_offline_does_not_erase_cached_projects() {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    let herdr = Arc::new(FakeHerdr::default());
    herdr.spaces.lock().unwrap().push(Workspace {
        id: "w1".into(),
        name: "Alpha".into(),
        directory: None,
    });
    let service = ProjectService::new(Arc::new(Mutex::new(db)), herdr.clone());
    let p = service.sync().unwrap().projects.remove(0);
    assert_eq!(p.directory_source, "missing");
    service.update(&p.id, "Alpha", Some(".")).unwrap();
    herdr.spaces.lock().unwrap()[0].directory = Some("E:/Authoritative".into());
    let snapshot = service.sync().unwrap();
    assert_eq!(snapshot.projects[0].directory_source, "conflict");
    assert_eq!(snapshot.projects[0].root_path, ".");
    service.update(&p.id, "Alpha", Some("")).unwrap();
    assert_eq!(service.get(&p.id).unwrap().root_path, "E:/Authoritative");
    herdr.spaces.lock().unwrap()[0].directory = None;
    assert_eq!(
        service.sync().unwrap().projects[0].directory_source,
        "cached"
    );
    *herdr.offline.lock().unwrap() = true;
    assert!(service.sync().is_err());
    assert_eq!(service.get(&p.id).unwrap().root_path, "E:/Authoritative");
}
#[test]
fn duplicate_legacy_projects_are_never_merged_and_explicit_link_is_unique() {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    db.execute_batch("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at) VALUES ('a','A','.','Alpha','now'),('b','B','.','Alpha','now');").unwrap();
    let herdr = Arc::new(FakeHerdr::default());
    herdr.spaces.lock().unwrap().push(Workspace {
        id: "w1".into(),
        name: "Alpha".into(),
        directory: Some(".".into()),
    });
    let service = ProjectService::new(Arc::new(Mutex::new(db)), herdr);
    let snapshot = service.sync().unwrap();
    assert_eq!(snapshot.projects.len(), 2);
    assert!(snapshot.projects.iter().all(|p| p.workspace_id.is_none()));
    service.link(Some("a"), "w1", None).unwrap();
    assert_eq!(
        service.link(Some("b"), "w1", None).unwrap_err(),
        "workspace.occupied"
    );
    assert!(service.get("b").unwrap().workspace_id.is_none());
}
#[test]
fn creation_commit_failure_is_recoverable_and_duplicate_submits_create_once() {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    db.execute_batch("CREATE TRIGGER reject_project BEFORE INSERT ON projects BEGIN SELECT RAISE(ABORT,'disk full'); END;").unwrap();
    let herdr = Arc::new(FakeHerdr::default());
    let service = Arc::new(ProjectService::new(Arc::new(Mutex::new(db)), herdr.clone()));
    let op = uuid::Uuid::new_v4().to_string();
    assert!(service.create(&op, "Alpha", ".").is_err());
    assert_eq!(service.operations().unwrap()[0].state, "unknown");
    service
        .db
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_project;")
        .unwrap();
    let workers: Vec<_> = (0..3)
        .map(|_| {
            let service = service.clone();
            let op = op.clone();
            std::thread::spawn(move || service.create(&op, "Alpha", ".").unwrap())
        })
        .collect();
    let results: Vec<_> = workers.into_iter().map(|w| w.join().unwrap().id).collect();
    assert!(results.iter().all(|id| id == &results[0]));
    assert_eq!(herdr.spaces.lock().unwrap().len(), 1);
}
#[test]
fn known_pre_send_failure_can_retry_without_leaving_a_dead_operation() {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    let herdr = Arc::new(FakeHerdr::default());
    let service = ProjectService::new(Arc::new(Mutex::new(db)), herdr.clone());
    let op = uuid::Uuid::new_v4().to_string();
    *herdr.reject_create.lock().unwrap() = true;
    assert!(service.create(&op, "Alpha", ".").is_err());
    assert_eq!(service.operations().unwrap()[0].state, "failed");
    *herdr.reject_create.lock().unwrap() = false;
    service.create(&op, "Alpha", ".").unwrap();
    assert!(service.operations().unwrap().is_empty());
    assert_eq!(herdr.spaces.lock().unwrap().len(), 1);
}
#[test]
fn equal_ids_in_other_sessions_do_not_change_an_existing_projects_identity() {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    let db = Arc::new(Mutex::new(db));
    let first = Arc::new(FakeHerdr::default());
    let second = Arc::new(FakeHerdr {
        session_name: Some("other".into()),
        ..Default::default()
    });
    first.spaces.lock().unwrap().push(Workspace {
        id: "w1".into(),
        name: "First".into(),
        directory: None,
    });
    second.spaces.lock().unwrap().push(Workspace {
        id: "w1".into(),
        name: "Second".into(),
        directory: None,
    });
    let service = ProjectService::new(db.clone(), first);
    let old = service.sync().unwrap().projects.remove(0);
    let other = ProjectService::new(db, second);
    let snapshot = other.sync().unwrap();
    assert_eq!(snapshot.projects.len(), 2);
    assert_eq!(
        other.update(&old.id, "Wrong", None).unwrap_err(),
        "workspace.session_mismatch"
    );
    assert_eq!(service.get(&old.id).unwrap().name, "First");
}
#[test]
fn unfinished_task_operations_block_relink_and_delete_but_not_other_projects() {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    let herdr = Arc::new(FakeHerdr::default());
    let service = ProjectService::new(Arc::new(Mutex::new(db)), herdr.clone());
    let p = service
        .create(&uuid::Uuid::new_v4().to_string(), "Alpha", ".")
        .unwrap();
    let task = crate::tasks::create(
        &service.db.lock().unwrap(),
        &p.id,
        "Keep",
        crate::models::TaskPriority::Normal,
        "{}",
    )
    .unwrap();
    service.db.lock().unwrap().execute("INSERT INTO lifecycle_operations(id,task_id,kind,state,created_at) VALUES ('pending',?1,'start','unknown','now')",[&task.id]).unwrap();
    assert_eq!(
        service.remove(&p.id).unwrap_err(),
        "workspace.verify_required"
    );
    assert_eq!(
        service.link(Some(&p.id), "w1", None).unwrap_err(),
        "workspace.verify_required"
    );
    service
        .create(&uuid::Uuid::new_v4().to_string(), "Other", ".")
        .unwrap();
    assert_eq!(service.sync().unwrap().projects.len(), 2);
}
#[test]
fn persisted_creation_recovers_after_database_reopen() {
    let path = std::env::temp_dir().join(format!("kanban-projects-{}.db", uuid::Uuid::new_v4()));
    let db = Connection::open(&path).unwrap();
    crate::db::init(&db).unwrap();
    let herdr = Arc::new(FakeHerdr::default());
    *herdr.lose_response.lock().unwrap() = true;
    let op = uuid::Uuid::new_v4().to_string();
    {
        let service = ProjectService::new(Arc::new(Mutex::new(db)), herdr.clone());
        assert!(service.create(&op, "Persisted", ".").is_err());
    }
    let db = Connection::open(&path).unwrap();
    crate::db::init(&db).unwrap();
    crate::db::recover_interrupted(&db).unwrap();
    {
        let service = ProjectService::new(Arc::new(Mutex::new(db)), herdr.clone());
        assert_eq!(service.recover(&op, None).unwrap().name, "Persisted");
    }
    assert_eq!(herdr.spaces.lock().unwrap().len(), 1);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn renamed_unconfirmed_workspace_can_be_explicitly_adopted_after_sync() {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    let herdr = Arc::new(FakeHerdr::default());
    let service = ProjectService::new(Arc::new(Mutex::new(db)), herdr.clone());
    let op = uuid::Uuid::new_v4().to_string();
    *herdr.lose_response.lock().unwrap() = true;
    assert!(service.create(&op, "Alpha", ".").is_err());
    herdr.rename_workspace("w1", "Externally renamed").unwrap();
    let imported = service.sync().unwrap().projects.remove(0);
    let task = crate::tasks::create(
        &service.db.lock().unwrap(),
        &imported.id,
        "Keep",
        crate::models::TaskPriority::Normal,
        "{}",
    )
    .unwrap();
    let recovered = service.recover(&op, Some("w1")).unwrap();
    assert_eq!(recovered.id, imported.id);
    assert_eq!(
        crate::tasks::list(&service.db.lock().unwrap(), &recovered.id).unwrap()[0].id,
        task.id
    );
    assert!(service.operations().unwrap().is_empty());
    assert_eq!(herdr.spaces.lock().unwrap().len(), 1);
}

#[test]
fn explicit_recovery_disambiguates_markers_without_claiming_another_operation() {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    let herdr = Arc::new(FakeHerdr::default());
    let service = ProjectService::new(Arc::new(Mutex::new(db)), herdr.clone());
    *herdr.lose_response.lock().unwrap() = true;
    let first = uuid::Uuid::new_v4().to_string();
    let second = uuid::Uuid::new_v4().to_string();
    assert!(service.create(&first, "First", ".").is_err());
    assert!(service.create(&second, "Second", ".").is_err());
    herdr.spaces.lock().unwrap().push(Workspace {
        id: "w3".into(),
        name: format!("kanban-create-{first}"),
        directory: Some(".".into()),
    });
    assert_eq!(
        service.recover(&first, None).unwrap_err(),
        "workspace.verify_required"
    );
    assert_eq!(
        service.recover(&first, Some("w2")).unwrap_err(),
        "workspace.occupied"
    );
    assert_eq!(
        service
            .recover(&first, Some("w3"))
            .unwrap()
            .workspace_id
            .as_deref(),
        Some("w3")
    );
    assert_eq!(
        service
            .recover(&second, None)
            .unwrap()
            .workspace_id
            .as_deref(),
        Some("w2")
    );
    assert_eq!(herdr.spaces.lock().unwrap().len(), 3);
}

#[test]
#[ignore = "creates a dedicated workspace; requires HERDR_ENV=1 and KANBAN_WORKSPACE_ACCEPTANCE_DIR"]
fn live_workspace_creation_directory_rename_and_local_removal() {
    assert_eq!(std::env::var("HERDR_ENV").unwrap(), "1");
    let cwd = std::env::var("KANBAN_WORKSPACE_ACCEPTANCE_DIR").unwrap();
    assert!(std::path::Path::new(&cwd).is_dir());
    let cli = Arc::new(crate::herdr::Cli::from_env());
    let op = uuid::Uuid::new_v4().to_string();
    let name = format!("kanban-acceptance-workspace-{op}");
    let db = Connection::open(std::path::Path::new(&cwd).join("live-projects.db")).unwrap();
    crate::db::init(&db).unwrap();
    let service = ProjectService::new(Arc::new(Mutex::new(db)), cli.clone());
    let project = service.create(&op, &name, &cwd).unwrap();
    let id = project.workspace_id.as_ref().unwrap();
    // Record the exact owned resource before any assertion, for safe explicit cleanup.
    std::fs::write(std::path::Path::new(&cwd).join("workspace-id.txt"), id).unwrap();
    assert!(same_directory(&project.root_path, &cwd));
    let renamed = format!("{name}-renamed");
    service.update(&project.id, &renamed, None).unwrap();
    assert_eq!(
        cli.workspaces()
            .unwrap()
            .iter()
            .find(|w| &w.id == id)
            .unwrap()
            .name,
        renamed
    );
    cli.rename_workspace(id, &name).unwrap();
    assert_eq!(
        service
            .sync()
            .unwrap()
            .projects
            .iter()
            .find(|p| p.id == project.id)
            .unwrap()
            .name,
        name
    );
    // The persisted directory may lag the online workspace. Wait only for bounded readback.
    let mut authoritative = None;
    for _ in 0..20 {
        authoritative = cli
            .workspaces()
            .unwrap()
            .into_iter()
            .find(|w| &w.id == id)
            .unwrap()
            .directory;
        if authoritative.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    assert!(same_directory(authoritative.as_deref().unwrap(), &cwd));
    service.remove(&project.id).unwrap();
    assert!(cli.workspaces().unwrap().iter().any(|w| &w.id == id));
    assert!(
        !service
            .sync()
            .unwrap()
            .projects
            .iter()
            .any(|p| p.id == project.id)
    );
    let restored = service.link(None, id, None).unwrap();
    assert!(same_directory(&restored.root_path, &cwd));
    assert_eq!(restored.directory_source, "herdr");
}
