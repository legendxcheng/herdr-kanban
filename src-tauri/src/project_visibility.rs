//! Local navigation preferences, independent of Herdr presence and task ownership.
use crate::{models::Project, projects};
use rusqlite::{Connection, params};
use std::collections::HashSet;

pub fn init(db: &Connection) -> rusqlite::Result<()> {
    if db.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=13)",
        [],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(());
    }
    let tx = db.unchecked_transaction()?;
    tx.execute_batch("CREATE TABLE project_visibility(project_id TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE, visible INTEGER NOT NULL CHECK(visible IN (0,1)));
        CREATE TABLE navigation_preferences(id INTEGER PRIMARY KEY CHECK(id=1), configured INTEGER NOT NULL DEFAULT 0);
        INSERT INTO navigation_preferences(id) VALUES(1);
        INSERT INTO schema_migrations(version) VALUES(13);")?;
    tx.commit()
}

pub fn selected(db: &Connection) -> Result<HashSet<String>, String> {
    let mut query = db
        .prepare("SELECT project_id FROM project_visibility WHERE visible=1")
        .map_err(|e| e.to_string())?;
    query
        .query_map([], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())
}

// Called inside the caller's transaction. Creation uses INSERT OR IGNORE so later
// recovery never overwrites an explicit user decision on an adopted Project.
pub fn show(db: &Connection, id: &str, explicit: bool) -> Result<(), String> {
    let sql = if explicit {
        "INSERT INTO project_visibility(project_id,visible) VALUES(?1,1) ON CONFLICT(project_id) DO UPDATE SET visible=1"
    } else {
        "INSERT OR IGNORE INTO project_visibility(project_id,visible) VALUES(?1,1)"
    };
    db.execute(sql, [id]).map_err(|e| e.to_string())?;
    db.execute(
        "UPDATE navigation_preferences SET configured=1 WHERE id=1",
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn hide(db: &Connection, id: &str) -> Result<(), String> {
    let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
    if !tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
            [id],
            |r| r.get::<_, bool>(0),
        )
        .map_err(|e| e.to_string())?
    {
        return Err("workspace.project_missing".into());
    }
    tx.execute("INSERT INTO project_visibility(project_id,visible) VALUES(?1,0) ON CONFLICT(project_id) DO UPDATE SET visible=0", [id]).map_err(|e| e.to_string())?;
    tx.execute(
        "UPDATE navigation_preferences SET configured=1 WHERE id=1",
        [],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
pub struct NavigationState {
    pub configured: bool,
    pub recovery_projects: Vec<Project>,
}

pub fn state(db: &Connection) -> Result<NavigationState, String> {
    let visible: HashSet<_> = projects::visible_list(db)?
        .into_iter()
        .map(|p| p.id)
        .collect();
    let mut recovery_projects = Vec::new();
    for project in projects::list(db)? {
        if visible.contains(&project.id) {
            continue;
        }
        let pending = db.query_row("SELECT EXISTS(SELECT 1 FROM tasks t WHERE t.project_id=?1 AND (
            EXISTS(SELECT 1 FROM dispatch_operations o WHERE o.task_id=t.id AND o.state IN ('pending','unknown')) OR
            EXISTS(SELECT 1 FROM lifecycle_operations o WHERE o.task_id=t.id AND o.state IN ('pending','unknown'))))", params![project.id], |r|r.get::<_,bool>(0)).map_err(|e| e.to_string())?;
        if pending {
            recovery_projects.push(project);
        }
    }
    Ok(NavigationState {
        configured: db
            .query_row(
                "SELECT configured FROM navigation_preferences WHERE id=1",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?,
        recovery_projects,
    })
}
