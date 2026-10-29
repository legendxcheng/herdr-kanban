//! Retire an execution without deleting its task or asserting that external work stopped.
use crate::workflow::Target;
use rusqlite::{Connection, OptionalExtension, params};

#[cfg(test)]
#[path = "reset_acceptance.rs"]
mod acceptance;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub fn init(db: &Connection) -> rusqlite::Result<()> {
    if !db.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=16)",
        [],
        |r| r.get::<_, bool>(0),
    )? {
        let tx = db.unchecked_transaction()?;
        tx.execute_batch("ALTER TABLE tasks ADD COLUMN execution_epoch INTEGER NOT NULL DEFAULT 0;
            CREATE TABLE task_execution_resets (
                id TEXT PRIMARY KEY, task_id TEXT NOT NULL, epoch INTEGER NOT NULL,
                retired_task_id TEXT NOT NULL UNIQUE, snapshot_json TEXT NOT NULL, created_at TEXT NOT NULL,
                UNIQUE(task_id,epoch));
            CREATE TABLE retired_execution_targets (
                reset_id TEXT NOT NULL REFERENCES task_execution_resets(id), target_json TEXT NOT NULL);
            INSERT INTO schema_migrations(version) VALUES(16);")?;
        tx.commit()?;
    }
    Ok(())
}

pub fn epoch(db: &Connection, task: &str) -> Result<i64, String> {
    crate::tasks::ensure_active(db, task)?;
    db.query_row(
        "SELECT execution_epoch FROM tasks WHERE id=?1",
        [task],
        |r| r.get(0),
    )
    .map_err(err)
}

pub fn check_epoch(db: &Connection, task: &str, expected: i64) -> Result<(), String> {
    if epoch(db, task)? != expected {
        return Err("reset.stale".into());
    }
    Ok(())
}

/// All writers after external I/O must still own a current dispatch operation.
pub fn check_dispatch(db: &Connection, task: &str, operation: &str) -> Result<(), String> {
    crate::tasks::ensure_active(db, task)?;
    let current: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM dispatch_operations WHERE id=?1 AND task_id=?2 AND state IN ('pending','unknown'))", params![operation,task], |r| r.get(0)).map_err(err)?;
    if !current {
        return Err("dispatch.operation_conflict".into());
    }
    Ok(())
}

pub fn quarantined(db: &Connection, target: &Target) -> Result<bool, String> {
    // A pane ID can be reused, but the retired terminal must never be reused automatically.
    db.query_row("SELECT EXISTS(SELECT 1 FROM retired_execution_targets
        WHERE COALESCE(json_extract(target_json,'$.machine_id'),'local')=?1
        AND json_extract(target_json,'$.session')=?2 AND json_extract(target_json,'$.terminal_id')=?3)",
        params![target.machine_id,target.session,target.terminal_id], |r| r.get(0)).map_err(err)
}

pub fn reset(
    db: &mut Connection,
    task: &str,
    expected_epoch: i64,
    flow_id: Option<&str>,
    dispatch_id: Option<&str>,
    request: &str,
) -> Result<bool, String> {
    uuid::Uuid::parse_str(request).map_err(|_| "dispatch.operation_required")?;
    let tx = db.transaction().map_err(err)?;
    // A retry of a committed request never resets a subsequent execution.
    let previous: Option<(String, i64)> = tx
        .query_row(
            "SELECT task_id,epoch FROM task_execution_resets WHERE id=?1",
            [request],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(err)?;
    if let Some((owner, old_epoch)) = previous {
        return if owner == task && old_epoch == expected_epoch {
            Ok(false)
        } else {
            Err("reset.stale".into())
        };
    }
    check_epoch(&tx, task, expected_epoch)?;
    let flow = crate::lifecycle::view(&tx, task)?;
    let binding = crate::workflow::binding(&tx, task)?;
    if flow.operation_id.as_deref() != flow_id
        || binding.operation.as_ref().map(|op| op.id.as_str()) != dispatch_id
    {
        return Err("reset.stale".into());
    }
    if flow.state.as_deref() == Some("pending")
        || binding
            .operation
            .as_ref()
            .is_some_and(|op| op.state == "pending")
    {
        return Err("reset.running".into());
    }
    if !matches!(flow.state.as_deref(), Some("failed" | "unknown")) && binding.state != "unknown" {
        return Err("reset.not_failed".into());
    }
    let snapshot: String = tx.query_row("SELECT json_object('status',status,'prompt_snapshot',prompt_snapshot,'dispatched_at',dispatched_at,'dispatched_to',dispatched_to,'completed_at',completed_at,
        'session',(SELECT json_object('id',codex_session_id,'cwd',cwd) FROM task_sessions WHERE task_id=tasks.id),
        'cwd',(SELECT cwd FROM task_working_directories WHERE task_id=tasks.id)) FROM tasks WHERE id=?1", [task], |r| r.get(0)).map_err(err)?;
    let retired = format!("retired:{request}");
    tx.execute("INSERT INTO task_execution_resets(id,task_id,epoch,retired_task_id,snapshot_json,created_at) VALUES(?1,?2,?3,?4,?5,?6)", params![request,task,expected_epoch,retired,snapshot,chrono::Utc::now().to_rfc3339()]).map_err(err)?;
    tx.execute("INSERT INTO retired_execution_targets(reset_id,target_json)
        SELECT ?1,target_json FROM task_bindings WHERE task_id=?2
        UNION SELECT ?1,target_json FROM lifecycle_operations WHERE task_id=?2 AND target_json IS NOT NULL
        UNION SELECT ?1,target_json FROM dispatch_operations WHERE task_id=?2", params![request,task]).map_err(err)?;
    // Operation IDs, states, prompts and evidence remain intact. The retired scope has
    // no live task, revoking old workers and excluding history from current projections.
    tx.execute(
        "UPDATE lifecycle_operations SET task_id=?2 WHERE task_id=?1",
        params![task, retired],
    )
    .map_err(err)?;
    tx.execute(
        "UPDATE dispatch_operations SET task_id=?2 WHERE task_id=?1",
        params![task, retired],
    )
    .map_err(err)?;
    tx.execute(
        "UPDATE image_prompt_snapshots SET task_id=?2 WHERE task_id=?1",
        params![task, retired],
    )
    .map_err(err)?;
    tx.execute("DELETE FROM task_bindings WHERE task_id=?1", [task])
        .map_err(err)?;
    tx.execute("DELETE FROM task_sessions WHERE task_id=?1", [task])
        .map_err(err)?;
    tx.execute(
        "DELETE FROM task_working_directories WHERE task_id=?1",
        [task],
    )
    .map_err(err)?;
    tx.execute("UPDATE tasks SET status='todo',execution_epoch=execution_epoch+1,prompt_snapshot=NULL,dispatched_at=NULL,dispatched_to=NULL,completed_at=NULL,updated_at=?2 WHERE id=?1", params![task,chrono::Utc::now().to_rfc3339()]).map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(true)
}
