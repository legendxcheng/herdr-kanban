//! Durable execution identities. Names and SSH aliases are display/connection data.
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

pub const LOCAL_ID: &str = "local";
pub fn local_id() -> String {
    LOCAL_ID.into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Machine {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub profile_id: Option<String>,
    pub ssh_target: Option<String>,
    pub session: String,
    pub platform: Option<String>,
    pub disabled: bool,
    pub helper_name: Option<String>,
    pub host_identity: Option<String>,
}

pub fn init(db: &Connection) -> rusqlite::Result<()> {
    if db.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=12)",
        [],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(());
    }
    let tx = db.unchecked_transaction()?;
    tx.execute_batch("CREATE TABLE machines(
        id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('local','ssh')),
        profile_id TEXT, ssh_target TEXT, session TEXT NOT NULL, platform TEXT,
        disabled INTEGER NOT NULL DEFAULT 0, helper_name TEXT, host_identity TEXT);
        INSERT INTO machines(id,name,kind,session) VALUES('local','Local','local','default');
        ALTER TABLE projects ADD COLUMN machine_id TEXT NOT NULL DEFAULT 'local';
        DROP INDEX project_workspace_identity;
        CREATE UNIQUE INDEX project_workspace_identity ON projects(machine_id,herdr_session,workspace_id) WHERE workspace_id IS NOT NULL;
        CREATE TABLE machine_task_bindings (
            task_id TEXT PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
            machine_id TEXT NOT NULL DEFAULT 'local', session TEXT NOT NULL, pane_id TEXT NOT NULL,
            target_json TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('valid','reserved')),
            UNIQUE(machine_id,session,pane_id));
        INSERT INTO machine_task_bindings(task_id,session,pane_id,target_json,state) SELECT task_id,session,pane_id,target_json,state FROM task_bindings;
        DROP TABLE task_bindings;
        ALTER TABLE machine_task_bindings RENAME TO task_bindings;
        CREATE TABLE machine_workspace_exclusions(machine_id TEXT NOT NULL DEFAULT 'local',session TEXT NOT NULL,workspace_id TEXT NOT NULL,PRIMARY KEY(machine_id,session,workspace_id));
        INSERT INTO machine_workspace_exclusions(session,workspace_id) SELECT session,workspace_id FROM workspace_exclusions;
        DROP TABLE workspace_exclusions;
        ALTER TABLE machine_workspace_exclusions RENAME TO workspace_exclusions;")?;
    for table in [
        "dispatch_operations",
        "lifecycle_operations",
        "task_sessions",
        "start_requests",
        "pane_allocations",
        "workspace_operations",
        "task_working_directories",
    ] {
        tx.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN machine_id TEXT NOT NULL DEFAULT 'local';"
        ))?;
    }
    // Capture immutable ownership in the same transaction as each intent/snapshot.
    // Project reassignment is not supported; these records survive task deletion.
    for table in [
        "dispatch_operations",
        "lifecycle_operations",
        "task_sessions",
        "task_working_directories",
    ] {
        tx.execute_batch(&format!("CREATE TRIGGER {table}_machine_snapshot AFTER INSERT ON {table}
            BEGIN UPDATE {table} SET machine_id=COALESCE((SELECT p.machine_id FROM projects p JOIN tasks t ON t.project_id=p.id WHERE t.id=NEW.task_id),'local') WHERE rowid=NEW.rowid; END;"))?;
    }
    tx.execute_batch("CREATE TRIGGER start_machine_snapshot AFTER INSERT ON start_requests
        BEGIN UPDATE start_requests SET machine_id=(SELECT machine_id FROM projects WHERE id=NEW.project_id) WHERE operation_id=NEW.operation_id; END;
        CREATE TRIGGER project_machine_immutable BEFORE UPDATE OF machine_id ON projects
        WHEN OLD.machine_id<>NEW.machine_id AND EXISTS(SELECT 1 FROM tasks WHERE project_id=OLD.id)
        BEGIN SELECT RAISE(ABORT,'machine.project_in_use'); END;")?;
    tx.execute_batch("DROP INDEX pane_allocation_workspace;
        CREATE INDEX pane_allocation_workspace ON pane_allocations(machine_id,session,workspace,resolved);
        INSERT INTO schema_migrations(version) VALUES(12);")?;
    tx.commit()
}

pub fn list(db: &Connection) -> Result<Vec<Machine>, String> {
    let mut query = db.prepare("SELECT id,name,kind,profile_id,ssh_target,session,platform,disabled,helper_name,host_identity FROM machines ORDER BY kind='local' DESC,rowid").map_err(|e| e.to_string())?;
    query
        .query_map([], |r| {
            Ok(Machine {
                id: r.get(0)?,
                name: r.get(1)?,
                kind: r.get(2)?,
                profile_id: r.get(3)?,
                ssh_target: r.get(4)?,
                session: r.get(5)?,
                platform: r.get(6)?,
                disabled: r.get(7)?,
                helper_name: r.get(8)?,
                host_identity: r.get(9)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())
}

/// Early schema-12 databases predate connection metadata. Never infer those
/// columns from the old version marker, or overwrite already verified metadata.
pub fn migrate_connection_metadata(db: &Connection) -> rusqlite::Result<()> {
    if db.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=14)",
        [],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(());
    }
    let tx = db.unchecked_transaction()?;
    let columns = {
        let mut query = tx.prepare("PRAGMA table_info(machines)")?;
        query
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for column in ["helper_name", "host_identity"] {
        if !columns.iter().any(|name| name == column) {
            tx.execute_batch(&format!("ALTER TABLE machines ADD COLUMN {column} TEXT;"))?;
        }
    }
    tx.execute("INSERT INTO schema_migrations(version) VALUES(14)", [])?;
    tx.commit()
}

pub fn get(db: &Connection, id: &str) -> Result<Machine, String> {
    list(db)?
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| "machine.missing".into())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedMachine {
    pub id: String,
    pub label: String,
    pub target: String,
    pub session: String,
    pub enabled: bool,
}

pub fn import_profile(
    db: &Connection,
    profile: &SavedMachine,
    platform: &str,
) -> Result<Machine, String> {
    if !profile.enabled {
        return Err("machine.disabled".into());
    }
    let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
    let existing = list(&tx)?
        .into_iter()
        .find(|m| m.profile_id.as_deref() == Some(&profile.id));
    if existing.as_ref().is_some_and(|m| {
        m.ssh_target.as_deref() != Some(&profile.target) || m.session != profile.session
    }) {
        return Err("machine.identity_changed".into());
    }
    let id = existing
        .map(|m| m.id)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    tx.execute("INSERT INTO machines(id,name,kind,profile_id,ssh_target,session,platform) VALUES(?1,?2,'ssh',?3,?4,?5,?6)
        ON CONFLICT(id) DO UPDATE SET disabled=0,platform=excluded.platform",
        rusqlite::params![id,profile.label,profile.id,profile.target,profile.session,platform]).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    get(db, &id)
}

pub fn disable(db: &Connection, id: &str) -> Result<(), String> {
    if id == LOCAL_ID {
        return Err("machine.local_required".into());
    }
    if db
        .execute("UPDATE machines SET disabled=1 WHERE id=?1", [id])
        .map_err(|e| e.to_string())?
        != 1
    {
        return Err("machine.missing".into());
    }
    Ok(())
}

pub fn rename(db: &Connection, id: &str, name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("machine.invalid_input".into());
    }
    if db
        .execute(
            "UPDATE machines SET name=?2 WHERE id=?1",
            rusqlite::params![id, name.trim()],
        )
        .map_err(|e| e.to_string())?
        != 1
    {
        return Err("machine.missing".into());
    }
    Ok(())
}

pub fn project_machine(db: &Connection, project: &str) -> Result<String, String> {
    db.query_row(
        "SELECT machine_id FROM projects WHERE id=?1",
        [project],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "workspace.project_missing".into())
}

pub fn ensure_enabled(db: &Connection, id: &str) -> Result<(), String> {
    let disabled: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM machines WHERE id=?1 AND disabled=1)",
            [id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if disabled {
        Err("machine.disabled".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn early_machine_schema_upgrades_missing_connection_columns_without_losing_identity() {
        for missing in [
            vec!["helper_name", "host_identity"],
            vec!["helper_name"],
            vec!["host_identity"],
            vec![],
        ] {
            let db = Connection::open_in_memory().unwrap();
            crate::db::init(&db).unwrap();
            let profile = SavedMachine {
                id: "existing-profile".into(),
                label: "devpc".into(),
                target: "devpc".into(),
                session: "default".into(),
                enabled: true,
            };
            let machine = import_profile(&db, &profile, "windows").unwrap();
            db.execute("UPDATE machines SET helper_name='installed-helper',host_identity='verified-host',disabled=1 WHERE id=?1", [&machine.id]).unwrap();
            db.execute("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at,machine_id) VALUES('retained','kept','C:/kept','kept','then',?1)", [&machine.id]).unwrap();
            db.execute_batch("INSERT INTO tasks(id,project_id,name,status,prompt_json,created_at,updated_at) VALUES('kept-task','retained','kept task','todo','{}','then','then');
                INSERT INTO project_visibility(project_id,visible) VALUES('retained',0);
                DELETE FROM schema_migrations WHERE version=14;").unwrap();
            for column in &missing {
                db.execute_batch(&format!("ALTER TABLE machines DROP COLUMN {column}"))
                    .unwrap();
            }
            crate::db::init(&db).unwrap();
            crate::db::init(&db).unwrap();
            let restored = get(&db, &machine.id)
                .expect("upgraded machine list must support the Connect query");
            assert!(restored.disabled);
            assert_eq!(restored.profile_id, Some(profile.id.clone()));
            assert_eq!(
                restored.helper_name.as_deref(),
                (!missing.contains(&"helper_name")).then_some("installed-helper")
            );
            assert_eq!(
                restored.host_identity.as_deref(),
                (!missing.contains(&"host_identity")).then_some("verified-host")
            );
            assert_eq!(project_machine(&db, "retained").unwrap(), machine.id);
            assert_eq!(
                db.query_row("SELECT name FROM tasks WHERE id='kept-task'", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
                "kept task"
            );
            assert!(
                !crate::project_visibility::selected(&db)
                    .unwrap()
                    .contains("retained")
            );
            assert_eq!(
                import_profile(&db, &profile, "windows").unwrap().id,
                machine.id
            );
            assert!(
                db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=14)",
                    [],
                    |r| r.get::<_, bool>(0)
                )
                .unwrap()
            );
        }
    }
    #[test]
    fn legacy_database_keeps_history_and_targets_when_upgraded_and_reopened() {
        let file = std::env::temp_dir().join(format!(
            "kanban-machine-upgrade-{}.db",
            uuid::Uuid::new_v4()
        ));
        let db = Connection::open(&file).unwrap();
        crate::db::init_before_machines(&db).unwrap();
        db.execute_batch("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at,herdr_session,workspace_id,sync_status) VALUES('p','legacy','C:/legacy','legacy','then','default','w1','ready');
        INSERT INTO tasks(id,project_id,name,status,prompt_json,created_at,updated_at) VALUES('t','p','old task','in_progress','{}','then','then');
        INSERT INTO task_sessions(task_id,codex_session_id,cwd,updated_at) VALUES('t','old-session','C:/legacy','then');
        INSERT INTO workspace_exclusions(session,workspace_id) VALUES('default','w2');
        INSERT INTO dispatch_operations(id,task_id,workspace,target_json,prompt,state,created_at,had_binding) VALUES('op','t','w1','{}','never resend','unknown','then',1);").unwrap();
        let target=serde_json::json!({"session":"default","workspace_id":"w1","pane_id":"w1:p1","terminal_id":"term","agent":"codex","process_id":"1:2","title":"","status":"idle"}).to_string();
        db.execute("INSERT INTO task_bindings(task_id,session,pane_id,target_json,state) VALUES('t','default','w1:p1',?1,'reserved')",[&target]).unwrap();
        db.execute(
            "UPDATE dispatch_operations SET target_json=?1 WHERE id='op'",
            [&target],
        )
        .unwrap();
        crate::db::init(&db).unwrap();
        drop(db);
        let db = Connection::open(&file).unwrap();
        crate::db::init(&db).unwrap();
        assert_eq!(get(&db, LOCAL_ID).unwrap().kind, "local");
        assert_eq!(project_machine(&db, "p").unwrap(), LOCAL_ID);
        let binding = crate::workflow::binding(&db, "t").unwrap();
        assert_eq!(binding.target.unwrap().machine_id, LOCAL_ID);
        let session: (String, String) = db
            .query_row(
                "SELECT codex_session_id,machine_id FROM task_sessions WHERE task_id='t'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(session, ("old-session".into(), LOCAL_ID.into()));
        let operation: (String, String, String) = db
            .query_row(
                "SELECT state,prompt,machine_id FROM dispatch_operations WHERE id='op'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            operation,
            ("unknown".into(), "never resend".into(), LOCAL_ID.into())
        );
        let excluded:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM workspace_exclusions WHERE machine_id='local' AND workspace_id='w2')",[],|r|r.get(0)).unwrap();
        assert!(excluded);
        drop(db);
        std::fs::remove_file(file).unwrap();
    }
    #[test]
    fn importing_a_saved_profile_is_idempotent_and_removal_keeps_identity() {
        let db = Connection::open_in_memory().unwrap();
        crate::db::init(&db).unwrap();
        let profile = SavedMachine {
            id: "profile-1".into(),
            label: "devpc".into(),
            target: "devpc".into(),
            session: "default".into(),
            enabled: true,
        };
        let first = import_profile(&db, &profile, "windows").unwrap();
        let mut renamed = profile.clone();
        renamed.label = "Development PC".into();
        let second = import_profile(&db, &renamed, "windows").unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(second.name, "devpc");
        rename(&db, &first.id, "My development PC").unwrap();
        disable(&db, &first.id).unwrap();
        assert!(get(&db, &first.id).unwrap().disabled);
        assert_eq!(
            import_profile(&db, &profile, "windows").unwrap().id,
            first.id
        );
        assert!(!get(&db, &first.id).unwrap().disabled);
        assert_eq!(get(&db, &first.id).unwrap().name, "My development PC");
        renamed.target = "another-computer".into();
        assert!(
            import_profile(&db, &renamed, "windows")
                .unwrap_err()
                .contains("identity_changed")
        );
    }
}
