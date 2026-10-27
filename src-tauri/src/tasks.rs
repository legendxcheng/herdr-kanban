//! Task persistence shared by IPC commands and workflow guards.

use crate::models::{Task, TaskPriority, TaskStatus};
use rusqlite::{Connection, OptionalExtension, params};

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

pub fn list(db: &Connection, project: &str) -> Result<Vec<Task>, String> {
    list_scope(db, &serde_json::to_string(&[project]).map_err(err)?, false)
}

/// Complete task views in a single local scope, with the same visibility rules as navigation.
pub fn list_visible(db: &Connection, projects: &[String]) -> Result<Vec<Task>, String> {
    list_scope(db, &serde_json::to_string(projects).map_err(err)?, true)
}

fn list_scope(db: &Connection, scope: &str, visible_only: bool) -> Result<Vec<Task>, String> {
    let mut stmt = db.prepare("SELECT t.id,t.project_id,t.name,t.status,t.prompt_json,t.prompt_snapshot,
        t.dispatched_at,t.dispatched_to,t.created_at,t.updated_at,t.priority FROM tasks t
        WHERE t.project_id IN (SELECT value FROM json_each(?1)) AND t.archived_at IS NULL
        AND (?2=0 OR EXISTS (SELECT 1 FROM projects p JOIN project_visibility v ON v.project_id=p.id
            WHERE p.id=t.project_id AND v.visible=1 AND (p.workspace_id IS NULL OR p.sync_status!='missing')))
        ORDER BY CASE t.priority WHEN 'urgent' THEN 0 WHEN 'high' THEN 1 ELSE 2 END, t.created_at, t.id").map_err(err)?;
    let rows = stmt
        .query_map(params![scope, visible_only], |row| {
            let priority: String = row.get(10)?;
            Ok(Task {
                codex_settings: Default::default(),
                codex_settings_editable: false,
                id: row.get(0)?,
                project_id: row.get(1)?,
                name: row.get(2)?,
                status: TaskStatus::from_str(&row.get::<_, String>(3)?),
                prompt_json: row.get(4)?,
                prompt_snapshot: row.get(5)?,
                dispatched_at: row.get(6)?,
                dispatched_to: row.get(7)?,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
                priority: match priority.as_str() {
                    "urgent" => TaskPriority::Urgent,
                    "high" => TaskPriority::High,
                    _ => TaskPriority::Normal,
                },
                binding: None,
                flow: None,
                archived_at: None,
                cleanup_required: false,
            })
        })
        .map_err(err)?;
    let mut tasks = rows.collect::<Result<Vec<_>, _>>().map_err(err)?;
    for task in &mut tasks {
        task.codex_settings = crate::codex_settings::read(db, &task.id)?;
        task.codex_settings_editable = crate::codex_settings::editable(db, &task.id)?;
        task.binding = Some(crate::workflow::binding(db, &task.id)?);
        task.flow = Some(crate::lifecycle::view(db, &task.id)?);
        task.cleanup_required = cleanup_required(db, &task.id)?;
    }
    Ok(tasks)
}

pub fn create(
    db: &Connection,
    project: &str,
    name: &str,
    priority: TaskPriority,
    prompt: &str,
) -> Result<Task, String> {
    let now = chrono::Utc::now().to_rfc3339();
    let task = Task {
        codex_settings: Default::default(),
        codex_settings_editable: true,
        id: uuid::Uuid::new_v4().to_string(),
        project_id: project.into(),
        name: name.into(),
        status: TaskStatus::Todo,
        priority,
        prompt_json: prompt.into(),
        prompt_snapshot: None,
        dispatched_at: None,
        dispatched_to: None,
        created_at: now.clone(),
        updated_at: now,
        binding: None,
        flow: None,
        archived_at: None,
        cleanup_required: false,
    };
    db.execute("INSERT INTO tasks(id,project_id,name,status,prompt_json,created_at,updated_at,priority) VALUES (?1,?2,?3,?4,?5,?6,?6,?7)",
        params![task.id, project, name, task.status.as_str(), task.prompt_json, task.created_at, priority.as_str()]).map_err(err)?;
    Ok(task)
}

pub fn update(
    db: &Connection,
    id: &str,
    name: &str,
    prompt: &str,
    priority: TaskPriority,
) -> Result<(), String> {
    ensure_active(db, id)?;
    db.execute(
        "UPDATE tasks SET name=?1,prompt_json=?2,priority=?3,updated_at=?4 WHERE id=?5",
        params![
            name,
            prompt,
            priority.as_str(),
            chrono::Utc::now().to_rfc3339(),
            id
        ],
    )
    .map_err(err)?;
    Ok(())
}

/// Call under the same database lock/transaction as the protected mutation.
pub fn ensure_active(db: &Connection, id: &str) -> Result<(), String> {
    let archived: bool = db
        .query_row(
            "SELECT archived_at IS NOT NULL FROM tasks WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?
        .ok_or("task.missing")?;
    if archived {
        return Err("task.archived".into());
    }
    Ok(())
}

/// Task absence revokes its workers. Keep operation evidence, including unknown sends;
/// cascading foreign keys release bindings, saved sessions and working directories.
pub fn delete(db: &mut Connection, id: &str) -> Result<(), String> {
    let tx = db.transaction().map_err(err)?;
    tx.execute("DELETE FROM tasks WHERE id=?1", [id])
        .map_err(err)?;
    tx.commit().map_err(err)?;
    crate::images::schedule_collection(db);
    Ok(())
}

pub fn archive(db: &mut Connection, id: &str) -> Result<(), String> {
    let tx = db.transaction().map_err(err)?;
    archive_in_transaction(&tx, id)?;
    tx.commit().map_err(err)
}

fn archive_in_transaction(tx: &Connection, id: &str) -> Result<(), String> {
    let (status, archived): (String, bool) = tx
        .query_row(
            "SELECT status,archived_at IS NOT NULL FROM tasks WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(err)?
        .ok_or("task.missing")?;
    if archived {
        return Ok(());
    }
    if status != "done" {
        return Err("archive.only_done".into());
    }
    crate::lifecycle::ensure_idle(tx, id, None)?;
    let pending: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM dispatch_operations WHERE task_id=?1 AND state IN ('pending','unknown'))", [id], |r| r.get(0)).map_err(err)?;
    if pending {
        return Err("dispatch.verify_required".into());
    }
    if cleanup_required(tx, id)? {
        return Err("archive.cleanup_required".into());
    }
    tx.execute(
        "UPDATE tasks SET archived_at=?2,updated_at=?2 WHERE id=?1",
        params![id, chrono::Utc::now().to_rfc3339()],
    )
    .map_err(err)?;
    tx.execute("DELETE FROM task_bindings WHERE task_id=?1", [id])
        .map_err(err)?;
    Ok(())
}

#[derive(Clone, Copy)]
pub enum CompletedTaskAction {
    Archive,
    Delete,
}

#[derive(Default, serde::Serialize)]
pub struct BatchTaskResult {
    pub succeeded: Vec<String>,
    pub skipped: Vec<BatchTaskIssue>,
    pub failed: Vec<BatchTaskIssue>,
}

#[derive(serde::Serialize)]
pub struct BatchTaskIssue {
    pub id: String,
    pub error: String,
}

/// The caller holds the database lock. Each task has its own transaction so a
/// failed write cannot roll back the other tasks or bypass their scope checks.
pub fn remove_completed(
    db: &mut Connection,
    project: &str,
    ids: &[String],
    action: CompletedTaskAction,
) -> BatchTaskResult {
    let mut result = BatchTaskResult::default();
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        if !seen.insert(id) {
            continue;
        }
        match remove_completed_item(db, project, id, action) {
            Ok(true) => result.succeeded.push(id.clone()),
            Ok(false) => result.skipped.push(BatchTaskIssue {
                id: id.clone(),
                error: "batch.task_changed".into(),
            }),
            Err(error) => {
                let blocked = matches!(
                    error.as_str(),
                    "flow.verify_required"
                        | "dispatch.verify_required"
                        | "archive.cleanup_required"
                );
                let issue = BatchTaskIssue {
                    id: id.clone(),
                    error,
                };
                if blocked {
                    result.skipped.push(issue);
                } else {
                    result.failed.push(issue);
                }
            }
        }
    }
    result
}

fn remove_completed_item(
    db: &mut Connection,
    project: &str,
    id: &str,
    action: CompletedTaskAction,
) -> Result<bool, String> {
    let tx = db.transaction().map_err(err)?;
    let in_scope: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1 AND project_id=?2 AND status='done' AND archived_at IS NULL)",
        params![id, project], |row| row.get(0),
    ).map_err(err)?;
    if !in_scope {
        return Ok(false);
    }
    // An accepted background start projects a done task into in_progress before
    // its persisted status changes. Mirror that projection under this transaction.
    let starting: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM lifecycle_operations o JOIN start_requests r ON r.operation_id=o.id
         WHERE o.task_id=?1 AND o.kind='start' AND o.state IN ('pending','unknown'))",
        [id], |row| row.get(0),
    ).map_err(err)?;
    if starting {
        return Ok(false);
    }
    match action {
        CompletedTaskAction::Archive => archive_in_transaction(&tx, id)?,
        CompletedTaskAction::Delete => {
            tx.execute("DELETE FROM tasks WHERE id=?1", [id])
                .map_err(err)?;
        }
    }
    tx.commit().map_err(err)?;
    Ok(true)
}

// A later failed start must not conceal a previous unfinished completion.
fn cleanup_required(db: &Connection, id: &str) -> Result<bool, String> {
    db.query_row("SELECT state IN ('failed','unknown') FROM lifecycle_operations WHERE task_id=?1 AND kind='complete' ORDER BY rowid DESC LIMIT 1",
        [id], |row| row.get(0)).optional().map(|value| value.unwrap_or(false)).map_err(err)
}

#[cfg(test)]
#[path = "completed_task_tests.rs"]
mod completed_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TaskPriority;
    use rusqlite::Connection;

    #[test]
    fn delete_unresolved_task_is_local_persistent_and_idempotent() {
        let path = std::env::temp_dir().join(format!("kanban-delete-{}.db", uuid::Uuid::new_v4()));
        let mut db = Connection::open(&path).unwrap();
        crate::db::init(&db).unwrap();
        db.execute_batch("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at) VALUES ('p','P','.','workspace','now');").unwrap();
        let task = create(&db, "p", "异常任务", TaskPriority::Normal, "{}").unwrap();
        let other = create(&db, "p", "保留任务", TaskPriority::Normal, "{}").unwrap();
        db.execute("INSERT INTO lifecycle_operations(id,task_id,kind,state,error,created_at) VALUES ('lost',?1,'start','unknown','agent_not_found','now')", [&task.id]).unwrap();
        delete(&mut db, &task.id).unwrap();
        delete(&mut db, &task.id).unwrap();
        assert_eq!(
            list(&db, "p")
                .unwrap()
                .iter()
                .map(|t| &t.id)
                .collect::<Vec<_>>(),
            vec![&other.id]
        );
        drop(db);
        let db = Connection::open(&path).unwrap();
        crate::db::init(&db).unwrap();
        crate::db::recover_interrupted(&db).unwrap();
        assert_eq!(list(&db, "p").unwrap().len(), 1);
        assert_eq!(ensure_active(&db, &task.id), Err("task.missing".into()));
        assert_eq!(
            crate::lifecycle::view(&db, &task.id)
                .unwrap()
                .state
                .as_deref(),
            Some("unknown")
        );
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn created_prompt_and_mentions_survive_database_reopen() {
        let path =
            std::env::temp_dir().join(format!("kanban-task-form-{}.db", uuid::Uuid::new_v4()));
        let prompt = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"检查表单"},{"type":"mention","attrs":{"kind":"file","label":"src/App.tsx","ref":"E:/fixture/src/App.tsx"}},{"type":"mention","attrs":{"kind":"skill","label":"review","ref":"E:/fixture/SKILL.md"}}]}]}"#;
        let id;
        {
            let db = Connection::open(&path).unwrap();
            crate::db::init(&db).unwrap();
            db.execute_batch("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at) VALUES ('p','Project','.','workspace','now');").unwrap();
            let task = create(&db, "p", "完整任务", TaskPriority::Urgent, prompt).unwrap();
            id = task.id;
            assert_eq!(task.prompt_json, prompt);
        }
        {
            let db = Connection::open(&path).unwrap();
            crate::db::init(&db).unwrap();
            let tasks = list(&db, "p").unwrap();
            assert_eq!(tasks.len(), 1);
            assert_eq!(tasks[0].id, id);
            assert_eq!(tasks[0].name, "完整任务");
            assert_eq!(tasks[0].priority, TaskPriority::Urgent);
            assert_eq!(tasks[0].status, TaskStatus::Todo);
            assert_eq!(tasks[0].prompt_json, prompt);
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn upgrading_an_old_database_preserves_data_and_survives_reopen() {
        let path = std::env::temp_dir().join(format!("kanban-cards-{}.db", uuid::Uuid::new_v4()));
        {
            let mut db = Connection::open(&path).unwrap();
            db.execute_batch("CREATE TABLE projects(id TEXT PRIMARY KEY,name TEXT NOT NULL,root_path TEXT NOT NULL,herdr_workspace TEXT NOT NULL,created_at TEXT NOT NULL);
                CREATE TABLE tasks(id TEXT PRIMARY KEY,project_id TEXT NOT NULL,name TEXT NOT NULL,status TEXT NOT NULL DEFAULT 'todo',prompt_json TEXT NOT NULL DEFAULT '{}',prompt_snapshot TEXT,dispatched_at TEXT,dispatched_to TEXT,created_at TEXT NOT NULL,updated_at TEXT NOT NULL);
                INSERT INTO projects VALUES ('p','Project','.','workspace','now');
                INSERT INTO tasks(id,project_id,name,status,prompt_json,prompt_snapshot,created_at,updated_at) VALUES ('old','p','Existing','done','original prompt','snapshot','2020','2020');").unwrap();
            crate::db::init(&db).unwrap();
            crate::db::init(&db).unwrap();
            let old = list(&db, "p").unwrap().remove(0);
            assert_eq!(old.priority, TaskPriority::Normal);
            assert!(old.archived_at.is_none());
            assert_eq!(old.prompt_json, "original prompt");
            assert_eq!(old.prompt_snapshot.as_deref(), Some("snapshot"));
            create(&db, "p", "New urgent", TaskPriority::Urgent, "{}").unwrap();
            archive(&mut db, "old").unwrap();
        }
        {
            let db = Connection::open(&path).unwrap();
            crate::db::init(&db).unwrap();
            let active = list(&db, "p").unwrap();
            assert_eq!(active.len(), 1);
            assert_eq!(active[0].priority, TaskPriority::Urgent);
            let retained: (String,String,bool) = db.query_row("SELECT prompt_json,prompt_snapshot,archived_at IS NOT NULL FROM tasks WHERE id='old'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
            assert_eq!(
                retained,
                ("original prompt".into(), "snapshot".into(), true)
            );
            assert!(serde_json::from_str::<TaskPriority>("\"critical\"").is_err());
            assert!(
                db.execute("UPDATE tasks SET priority='critical'", [])
                    .is_err()
            );
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn archive_keeps_pending_dispatch_and_unfinished_cleanup_visible() {
        let mut db = Connection::open_in_memory().unwrap();
        crate::db::init(&db).unwrap();
        db.execute_batch("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at) VALUES ('p','Project','.','workspace','now');").unwrap();
        let task = create(&db, "p", "Unresolved", TaskPriority::High, "{}").unwrap();
        db.execute("UPDATE tasks SET status='done' WHERE id=?1", [&task.id])
            .unwrap();
        for state in ["pending", "unknown"] {
            db.execute("INSERT INTO dispatch_operations(id,task_id,workspace,target_json,prompt,created_at,state,had_binding) VALUES ('op',?1,'workspace','{}','prompt','now',?2,0)", params![task.id, state]).unwrap();
            assert_eq!(
                archive(&mut db, &task.id).unwrap_err(),
                "dispatch.verify_required"
            );
            db.execute("DELETE FROM dispatch_operations", []).unwrap();
        }
        for (state, reason) in [
            ("pending", "flow.verify_required"),
            ("unknown", "flow.verify_required"),
            ("failed", "archive.cleanup_required"),
        ] {
            db.execute("INSERT INTO lifecycle_operations(id,task_id,kind,state,created_at,error) VALUES ('flow',?1,'complete',?2,'now','cleanup failed')", params![task.id, state]).unwrap();
            assert_eq!(archive(&mut db, &task.id).unwrap_err(), reason);
            db.execute("DELETE FROM lifecycle_operations", []).unwrap();
        }
        assert_eq!(list(&db, "p").unwrap().len(), 1);
    }

    #[test]
    fn only_finished_tasks_can_be_archived_and_repeated_archive_is_safe() {
        let mut db = Connection::open_in_memory().unwrap();
        crate::db::init(&db).unwrap();
        db.execute_batch("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at) VALUES ('p','Project','.','workspace','now');").unwrap();
        let task = create(&db, "p", "Keep history", TaskPriority::Urgent, "{}").unwrap();
        assert_eq!(archive(&mut db, &task.id).unwrap_err(), "archive.only_done");
        db.execute("UPDATE tasks SET status='done' WHERE id=?1", [&task.id])
            .unwrap();
        archive(&mut db, &task.id).unwrap();
        archive(&mut db, &task.id).unwrap();
        assert!(list(&db, "p").unwrap().is_empty());
        assert_eq!(
            update(&db, &task.id, "late edit", "{}", TaskPriority::High).unwrap_err(),
            "task.archived"
        );
    }

    #[test]
    fn tasks_keep_priority_and_list_most_urgent_first() {
        let db = Connection::open_in_memory().unwrap();
        crate::db::init(&db).unwrap();
        db.execute_batch("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at) VALUES ('p','Project','.','workspace','now');").unwrap();
        create(&db, "p", "General", TaskPriority::Normal, "{}").unwrap();
        let important = create(&db, "p", "Important", TaskPriority::High, "{}").unwrap();
        create(&db, "p", "Urgent", TaskPriority::Urgent, "{}").unwrap();
        assert_eq!(
            list(&db, "p")
                .unwrap()
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>(),
            ["Urgent", "Important", "General"]
        );
        update(&db, &important.id, "Changed", "{}", TaskPriority::Normal).unwrap();
        let tasks = list(&db, "p").unwrap();
        assert_eq!(
            tasks.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["Urgent", "General", "Changed"]
        );
        assert_eq!(tasks[2].priority, TaskPriority::Normal);
    }
}
