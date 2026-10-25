//! Compact snapshot for the overview dialog. No prompts, bindings or Herdr I/O.
use rusqlite::{Connection, params};

#[derive(Debug, serde::Serialize, PartialEq)]
pub struct ProjectCounts {
    pub project_id: String,
    pub todo: u64,
    pub in_progress: u64,
    pub done: u64,
    pub today_done: u64,
}

pub fn read(
    db: &Connection,
    projects: &[String],
    start: &str,
    end: &str,
) -> Result<Vec<ProjectCounts>, String> {
    let from = chrono::DateTime::parse_from_rfc3339(start).map_err(|_| "overview.invalid_day")?;
    let to = chrono::DateTime::parse_from_rfc3339(end).map_err(|_| "overview.invalid_day")?;
    // Local calendar days can be shorter/longer than 24 hours at DST changes.
    if to <= from || (to - from).num_hours() > 26 {
        return Err("overview.invalid_day".into());
    }
    let mut stmt = db.prepare(
        "WITH projected AS (
            SELECT t.project_id, t.archived_at, t.completed_at,
                CASE WHEN o.kind='start' AND o.state IN ('pending','unknown') AND r.operation_id IS NOT NULL
                     THEN 'in_progress' ELSE CASE WHEN t.status IN ('done','in_progress') THEN t.status ELSE 'todo' END END AS column_status
            FROM tasks t
            LEFT JOIN lifecycle_operations o ON o.rowid=(SELECT MAX(h.rowid) FROM lifecycle_operations h WHERE h.task_id=t.id)
            LEFT JOIN start_requests r ON r.operation_id=o.id
            WHERE t.project_id IN (SELECT value FROM json_each(?1))
        )
        SELECT p.id,
            COALESCE(SUM(t.archived_at IS NULL AND t.column_status='todo'),0),
            COALESCE(SUM(t.archived_at IS NULL AND t.column_status='in_progress'),0),
            COALESCE(SUM(t.archived_at IS NULL AND t.column_status='done'),0),
            COALESCE(SUM(t.column_status='done' AND julianday(t.completed_at)>=julianday(?2) AND julianday(t.completed_at)<julianday(?3)),0)
        FROM projects p LEFT JOIN projected t ON t.project_id=p.id
        WHERE p.id IN (SELECT value FROM json_each(?1)) GROUP BY p.id"
    ).map_err(|e| e.to_string())?;
    let ids = serde_json::to_string(projects).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![ids, start, end], |r| {
            Ok(ProjectCounts {
                project_id: r.get(0)?,
                todo: r.get(1)?,
                in_progress: r.get(2)?,
                done: r.get(3)?,
                today_done: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "task_overview_tests.rs"]
mod tests;
