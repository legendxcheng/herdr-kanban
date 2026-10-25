use super::*;

fn fixture(db: &Connection) {
    crate::db::init(db).unwrap();
    db.execute_batch(
        "INSERT INTO projects(id,name,root_path,herdr_workspace,created_at) VALUES
        ('p','Current','.','current','now'),('q','Other','.','other','now');",
    )
    .unwrap();
}

fn task(db: &Connection, id: &str) {
    db.execute("INSERT INTO tasks(id,project_id,name,status,prompt_json,prompt_snapshot,created_at,updated_at)
        VALUES (?1,'p',?1,'done','original prompt','original snapshot','now','now')", [id]).unwrap();
}

fn batch(db: &mut Connection, ids: &[&str], action: CompletedTaskAction) -> BatchTaskResult {
    remove_completed(
        db,
        "p",
        &ids.iter().map(|id| (*id).into()).collect::<Vec<_>>(),
        action,
    )
}

fn exists(db: &Connection, table: &str, column: &str, id: &str) -> bool {
    db.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE {column}=?1)"),
        [id],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn batch_rechecks_project_status_and_archive_scope_and_uses_only_snapshot_ids() {
    for action in [CompletedTaskAction::Archive, CompletedTaskAction::Delete] {
        let mut db = Connection::open_in_memory().unwrap();
        fixture(&db);
        for id in [
            "first",
            "todo",
            "active",
            "archived",
            "foreign",
            "newly-completed",
            "last",
        ] {
            task(&db, id);
        }
        db.execute_batch(
            "UPDATE tasks SET status='todo' WHERE id='todo';
            UPDATE tasks SET status='in_progress' WHERE id='active';
            UPDATE tasks SET archived_at='earlier' WHERE id='archived';
            UPDATE tasks SET project_id='q' WHERE id='foreign';",
        )
        .unwrap();
        assert!(batch(&mut db, &[], action).succeeded.is_empty());
        let result = batch(
            &mut db,
            &[
                "first", "todo", "active", "archived", "foreign", "missing", "first", "last",
            ],
            action,
        );
        assert_eq!(result.succeeded, ["first", "last"]);
        assert_eq!(result.skipped.len(), 5);
        assert!(result.failed.is_empty());
        assert!(
            result
                .skipped
                .iter()
                .all(|issue| issue.error == "batch.task_changed")
        );
        for id in ["todo", "active", "archived", "foreign", "newly-completed"] {
            assert!(exists(&db, "tasks", "id", id));
            let stamp: Option<String> = db
                .query_row("SELECT archived_at FROM tasks WHERE id=?1", [id], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(
                stamp.as_deref(),
                if id == "archived" {
                    Some("earlier")
                } else {
                    None
                }
            );
        }
        let repeated = batch(&mut db, &["first", "last"], action);
        assert!(repeated.succeeded.is_empty());
        assert_eq!(repeated.skipped.len(), 2);
    }
}

#[test]
fn batch_archive_skips_guards_rolls_back_failed_item_and_preserves_history_on_reopen() {
    let path = std::env::temp_dir().join(format!("kanban-batch-{}.db", uuid::Uuid::new_v4()));
    let mut db = Connection::open(&path).unwrap();
    fixture(&db);
    for id in ["first", "dispatch", "closing", "cleanup", "failed", "last"] {
        task(&db, id);
        db.execute("INSERT INTO task_bindings(task_id,session,pane_id,target_json,state) VALUES (?1,'default',?1,'{}','valid')", [id]).unwrap();
        db.execute("INSERT INTO task_sessions(task_id,codex_session_id,cwd,updated_at) VALUES (?1,?1,'.','now')", [id]).unwrap();
    }
    db.execute_batch("INSERT INTO dispatch_operations(id,task_id,workspace,target_json,prompt,created_at,state,had_binding)
        VALUES ('dispatch-op','dispatch','current','{}','sent once','now','unknown',1);
        INSERT INTO lifecycle_operations(id,task_id,kind,state,created_at) VALUES
        ('closing-op','closing','complete','unknown','now'),
        ('cleanup-op','cleanup','complete','failed','now'),
        ('later-start','cleanup','start','failed','now');
        CREATE TRIGGER fail_one_archive BEFORE DELETE ON task_bindings WHEN OLD.task_id='failed'
        BEGIN SELECT RAISE(ABORT,'simulated archive write failure'); END;").unwrap();
    let result = batch(
        &mut db,
        &["first", "dispatch", "closing", "cleanup", "failed", "last"],
        CompletedTaskAction::Archive,
    );
    assert_eq!(result.succeeded, ["first", "last"]);
    assert_eq!(
        result
            .skipped
            .iter()
            .map(|issue| issue.error.as_str())
            .collect::<Vec<_>>(),
        [
            "dispatch.verify_required",
            "flow.verify_required",
            "archive.cleanup_required"
        ]
    );
    assert_eq!(result.failed.len(), 1);
    assert_eq!(result.failed[0].id, "failed");
    assert!(
        result.failed[0]
            .error
            .contains("simulated archive write failure")
    );
    drop(db);
    let db = Connection::open(&path).unwrap();
    crate::db::init(&db).unwrap();
    for id in ["first", "last"] {
        let retained: (String, String, bool) = db
            .query_row(
                "SELECT prompt_json,prompt_snapshot,archived_at IS NOT NULL FROM tasks WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            retained,
            ("original prompt".into(), "original snapshot".into(), true)
        );
        assert!(exists(&db, "task_sessions", "task_id", id));
        assert!(!exists(&db, "task_bindings", "task_id", id));
    }
    for id in ["dispatch", "closing", "cleanup", "failed"] {
        let active: bool = db
            .query_row(
                "SELECT archived_at IS NULL FROM tasks WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(active);
        assert!(exists(&db, "task_bindings", "task_id", id));
    }
    assert!(exists(&db, "dispatch_operations", "id", "dispatch-op"));
    assert!(exists(&db, "lifecycle_operations", "id", "cleanup-op"));
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn batch_delete_unresolved_tasks_is_local_cascades_and_continues_after_write_failure() {
    let mut db = Connection::open_in_memory().unwrap();
    fixture(&db);
    for id in ["unresolved", "failed", "last"] {
        task(&db, id);
        db.execute("INSERT INTO task_sessions(task_id,codex_session_id,cwd,updated_at) VALUES (?1,?1,'.','now')", [id]).unwrap();
        db.execute("INSERT INTO task_bindings(task_id,session,pane_id,target_json,state) VALUES (?1,'default',?1,'corrupt target','valid')", [id]).unwrap();
    }
    db.execute_batch("INSERT INTO lifecycle_operations(id,task_id,kind,state,target_json,created_at)
        VALUES ('close','unresolved','complete','unknown','corrupt target','now');
        INSERT INTO dispatch_operations(id,task_id,workspace,target_json,prompt,created_at,state,had_binding)
        VALUES ('send','unresolved','current','corrupt target','once','now','unknown',1);
        INSERT INTO task_working_directories(task_id,cwd) VALUES ('unresolved','.');
        CREATE TRIGGER fail_one_delete BEFORE DELETE ON tasks WHEN OLD.id='failed'
        BEGIN SELECT RAISE(ABORT,'simulated delete write failure'); END;").unwrap();
    let result = batch(
        &mut db,
        &["unresolved", "failed", "last"],
        CompletedTaskAction::Delete,
    );
    assert_eq!(result.succeeded, ["unresolved", "last"]);
    assert!(result.skipped.is_empty());
    assert_eq!(result.failed.len(), 1);
    assert_eq!(result.failed[0].id, "failed");
    for table in [
        "tasks",
        "task_sessions",
        "task_bindings",
        "task_working_directories",
    ] {
        assert!(!exists(
            &db,
            table,
            if table == "tasks" { "id" } else { "task_id" },
            "unresolved"
        ));
    }
    for table in ["tasks", "task_sessions", "task_bindings"] {
        assert!(exists(
            &db,
            table,
            if table == "tasks" { "id" } else { "task_id" },
            "failed"
        ));
    }
    assert!(exists(&db, "lifecycle_operations", "id", "close"));
    assert!(exists(&db, "dispatch_operations", "id", "send"));
    db.execute_batch("DROP TRIGGER fail_one_delete").unwrap();
    assert_eq!(
        batch(&mut db, &["failed"], CompletedTaskAction::Delete).succeeded,
        ["failed"]
    );
}

#[test]
fn batch_does_not_remove_done_tasks_already_projected_into_background_start() {
    for state in ["pending", "unknown"] {
        for action in [CompletedTaskAction::Archive, CompletedTaskAction::Delete] {
            let mut db = Connection::open_in_memory().unwrap();
            fixture(&db);
            task(&db, "starting");
            task(&db, "last");
            db.execute("INSERT INTO lifecycle_operations(id,task_id,kind,state,created_at) VALUES ('start','starting','start',?1,'now')", [state]).unwrap();
            db.execute_batch("INSERT INTO start_requests(operation_id,project_id,workspace,cwd,prompt_json,needs_prompt,original_status,stage)
                VALUES ('start','p','current','.','{}',0,'done','queued');").unwrap();
            let result = batch(&mut db, &["starting", "last"], action);
            assert_eq!(result.succeeded, ["last"]);
            assert_eq!(result.skipped.len(), 1);
            assert_eq!(result.skipped[0].id, "starting");
            assert!(exists(&db, "tasks", "id", "starting"));
            assert!(exists(&db, "start_requests", "operation_id", "start"));
        }
    }
}
