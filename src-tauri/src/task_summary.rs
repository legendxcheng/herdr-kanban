//! Read-only, local snapshot for the cross-workspace board. Never loads prompts or contacts Herdr.
use crate::models::{TaskPriority, TaskStatus};
use rusqlite::Connection;

#[derive(Debug, serde::Serialize)]
pub struct TaskSummary {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    pub created_at: String,
}

pub fn read(db: &Connection, project_ids: &[String]) -> Result<Vec<TaskSummary>, String> {
    // Keep this scope identical to projects::visible_list, including offline cached projects.
    // Latest start projection matches task_overview and the regular task list's Flow view.
    let mut stmt = db.prepare(
        "SELECT t.id,t.project_id,t.name,
            CASE WHEN o.kind='start' AND o.state IN ('pending','unknown') AND r.operation_id IS NOT NULL
                 THEN 'in_progress' ELSE t.status END,
            t.priority,t.created_at
         FROM tasks t
         JOIN projects p ON p.id=t.project_id
         JOIN project_visibility v ON v.project_id=p.id AND v.visible=1
         LEFT JOIN lifecycle_operations o ON o.rowid=(SELECT MAX(h.rowid) FROM lifecycle_operations h WHERE h.task_id=t.id)
         LEFT JOIN start_requests r ON r.operation_id=o.id
         WHERE t.archived_at IS NULL AND (p.workspace_id IS NULL OR p.sync_status!='missing')
           AND p.id IN (SELECT value FROM json_each(?1))
         ORDER BY CASE t.priority WHEN 'urgent' THEN 0 WHEN 'high' THEN 1 ELSE 2 END,t.created_at,t.id"
    ).map_err(|e| e.to_string())?;
    let ids = serde_json::to_string(project_ids).map_err(|e| e.to_string())?;
    stmt.query_map([ids], |r| {
        let priority: String = r.get(4)?;
        Ok(TaskSummary {
            id: r.get(0)?,
            project_id: r.get(1)?,
            name: r.get(2)?,
            status: TaskStatus::from_str(&r.get::<_, String>(3)?),
            priority: match priority.as_str() {
                "urgent" => TaskPriority::Urgent,
                "high" => TaskPriority::High,
                _ => TaskPriority::Normal,
            },
            created_at: r.get(5)?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "task_summary_tests.rs"]
mod tests;
