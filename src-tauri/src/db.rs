use rusqlite::{Connection, Result};
use std::path::PathBuf;

/// Keep this guard alive until shutdown, so restart recovery cannot race a sender.
pub fn lock_database(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_extension("lock"))?;
    fs2::FileExt::try_lock_exclusive(&file)?;
    Ok(file)
}

pub fn db_path() -> PathBuf {
    // Allows isolated desktop acceptance runs without touching personal tasks.
    if let Some(path) = std::env::var_os("KANBAN_DATA_DIR") {
        return PathBuf::from(path).join("kanban.db");
    }
    let data_dir = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    data_dir.join("herdr-kanban").join("kanban.db")
}

pub fn init(conn: &Connection) -> Result<()> {
    init_before_machines(conn)?;
    crate::machines::init(conn)
}

pub(crate) fn init_before_machines(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=3000;")?;
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS projects (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            root_path TEXT NOT NULL,
            herdr_workspace TEXT NOT NULL,
            cs_task_id TEXT,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS tasks (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'todo',
            prompt_json TEXT NOT NULL DEFAULT '{}',
            prompt_snapshot TEXT,
            dispatched_at TEXT,
            dispatched_to TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_tasks_project ON tasks(project_id);
        CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
    ",
    )?;
    // migrate: silently ignored if column already exists
    let _ = conn.execute_batch("ALTER TABLE projects ADD COLUMN cs_task_id TEXT;");
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations(version INTEGER PRIMARY KEY);
        CREATE TABLE IF NOT EXISTS task_bindings (
            task_id TEXT PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
            session TEXT NOT NULL, pane_id TEXT NOT NULL, target_json TEXT NOT NULL,
            state TEXT NOT NULL CHECK(state IN ('valid','reserved')),
            UNIQUE(session,pane_id)
        );
        INSERT OR IGNORE INTO schema_migrations(version) VALUES (1);",
    )?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS dispatch_operations (
        id TEXT PRIMARY KEY, task_id TEXT NOT NULL, workspace TEXT NOT NULL,
        target_json TEXT NOT NULL, prompt TEXT NOT NULL, created_at TEXT NOT NULL,
        state TEXT NOT NULL CHECK(state IN ('pending','unknown','sent','not_sent')),
        had_binding INTEGER NOT NULL, evidence TEXT NOT NULL DEFAULT ''
    );
    CREATE UNIQUE INDEX IF NOT EXISTS one_pending_dispatch ON dispatch_operations(task_id) WHERE state IN ('pending','unknown');
    INSERT OR IGNORE INTO schema_migrations(version) VALUES (2);")?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS task_sessions (
        task_id TEXT PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
        codex_session_id TEXT NOT NULL, cwd TEXT NOT NULL, updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS lifecycle_operations (
        id TEXT PRIMARY KEY, task_id TEXT NOT NULL, kind TEXT NOT NULL,
        state TEXT NOT NULL, target_json TEXT, error TEXT NOT NULL DEFAULT '',
        created_at TEXT NOT NULL, evidence TEXT NOT NULL DEFAULT ''
    );
    CREATE UNIQUE INDEX IF NOT EXISTS one_active_lifecycle ON lifecycle_operations(task_id)
        WHERE state IN ('pending','unknown');
    INSERT OR IGNORE INTO schema_migrations(version) VALUES (3);",
    )?;
    let has_priority: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=4)",
        [],
        |row| row.get(0),
    )?;
    if !has_priority {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch("ALTER TABLE tasks ADD COLUMN priority TEXT NOT NULL DEFAULT 'normal' CHECK(priority IN ('normal','high','urgent'));
            INSERT INTO schema_migrations(version) VALUES (4);")?;
        tx.commit()?;
    }
    let has_archive: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=5)",
        [],
        |row| row.get(0),
    )?;
    if !has_archive {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(
            "ALTER TABLE tasks ADD COLUMN archived_at TEXT;
            INSERT INTO schema_migrations(version) VALUES (5);",
        )?;
        tx.commit()?;
    }
    if !conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=6)",
        [],
        |r| r.get::<_, bool>(0),
    )? {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch("ALTER TABLE projects ADD COLUMN herdr_session TEXT;
            ALTER TABLE projects ADD COLUMN workspace_id TEXT;
            ALTER TABLE projects ADD COLUMN sync_status TEXT NOT NULL DEFAULT 'unlinked';
            ALTER TABLE projects ADD COLUMN directory_source TEXT NOT NULL DEFAULT 'fallback';
            ALTER TABLE projects ADD COLUMN herdr_directory TEXT;
            CREATE UNIQUE INDEX project_workspace_identity ON projects(herdr_session,workspace_id) WHERE workspace_id IS NOT NULL;
            INSERT INTO schema_migrations(version) VALUES (6);")?;
        tx.commit()?;
    }
    conn.execute_batch("CREATE TABLE IF NOT EXISTS workspace_operations(
        id TEXT PRIMARY KEY, session TEXT NOT NULL, name TEXT NOT NULL, cwd TEXT NOT NULL,
        workspace_id TEXT, project_id TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('pending','unknown','succeeded','failed')),
        error TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS workspace_exclusions(session TEXT NOT NULL,workspace_id TEXT NOT NULL,PRIMARY KEY(session,workspace_id));
        CREATE TABLE IF NOT EXISTS task_working_directories(task_id TEXT PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,cwd TEXT NOT NULL);
        INSERT OR IGNORE INTO task_working_directories(task_id,cwd)
            SELECT t.id,COALESCE(s.cwd,p.root_path) FROM tasks t JOIN projects p ON p.id=t.project_id
            LEFT JOIN task_sessions s ON s.task_id=t.id WHERE EXISTS(SELECT 1 FROM task_bindings b WHERE b.task_id=t.id);")?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS start_requests (
        operation_id TEXT PRIMARY KEY REFERENCES lifecycle_operations(id),
        project_id TEXT NOT NULL, workspace TEXT NOT NULL, cwd TEXT NOT NULL,
        prompt_json TEXT NOT NULL, needs_prompt INTEGER NOT NULL,
        original_status TEXT NOT NULL, stage TEXT NOT NULL DEFAULT 'queued'
    );
    INSERT OR IGNORE INTO schema_migrations(version) VALUES (7);",
    )?;
    if !conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=8)",
        [],
        |r| r.get::<_, bool>(0),
    )? {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(
            "ALTER TABLE projects ADD COLUMN workspace_order INTEGER;
            INSERT INTO schema_migrations(version) VALUES (8);",
        )?;
        tx.commit()?;
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS pane_allocations (
        operation_id TEXT PRIMARY KEY, session TEXT NOT NULL, workspace TEXT NOT NULL,
        before_json TEXT NOT NULL, target_json TEXT, resolved INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX IF NOT EXISTS pane_allocation_workspace ON pane_allocations(session,workspace,resolved);",
    )?;
    crate::images::init(conn)?;
    if !conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=10)",
        [],
        |r| r.get::<_, bool>(0),
    )? {
        let tx = conn.unchecked_transaction()?;
        // Old updated_at values include edits and cleanup retries: do not backfill.
        tx.execute_batch(
            "ALTER TABLE tasks ADD COLUMN completed_at TEXT;
            CREATE INDEX lifecycle_task_history ON lifecycle_operations(task_id);
            INSERT INTO schema_migrations(version) VALUES (10);",
        )?;
        tx.commit()?;
    }
    crate::task_titles::init(conn)?;
    Ok(())
}

/// Only called once when opening the application, never during a live command.
pub fn recover_interrupted(conn: &Connection) -> Result<()> {
    conn.execute("UPDATE task_title_requests SET state='interrupted',error='title.interrupted' WHERE state IN ('pending','running')", [])?;
    conn.execute("DELETE FROM image_drafts", [])?;
    let _ = crate::images::release_abandoned_preparations(conn, true);
    // Reclamation cannot prevent opening otherwise usable task data.
    crate::images::schedule_collection(conn);
    conn.execute("UPDATE workspace_operations SET state='unknown',error='workspace.interrupted' WHERE state='pending'", [])?;
    conn.execute("UPDATE lifecycle_operations SET state='unknown',error='flow.interrupted' WHERE state='pending' AND EXISTS(SELECT 1 FROM tasks WHERE id=lifecycle_operations.task_id)", [])?;
    conn.execute(
        "UPDATE dispatch_operations SET state='unknown' WHERE state='pending' AND EXISTS(SELECT 1 FROM tasks WHERE id=dispatch_operations.task_id)",
        [],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn another_instance_cannot_recover_a_live_databases_dispatches() {
        let path = std::env::temp_dir().join(format!("kanban-{}.db", uuid::Uuid::new_v4()));
        let first = super::lock_database(&path).unwrap();
        assert!(super::lock_database(&path).is_err());
        drop(first);
        let restarted = super::lock_database(&path).unwrap();
        drop(restarted);
        std::fs::remove_file(path.with_extension("lock")).unwrap();
    }
}
