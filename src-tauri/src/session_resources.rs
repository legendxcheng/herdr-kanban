//! Runtime resources outlive their task cards.
use crate::workflow::Target;
use rusqlite::{Connection, params};

#[derive(Debug, serde::Serialize)]
pub struct Resource {
    pub id: i64,
    pub task_id: String,
    pub target: Target,
    pub reason: String,
    pub state: String,
    pub error: String,
}

pub fn init(db: &Connection) -> rusqlite::Result<()> {
    let tx = db.unchecked_transaction()?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS session_resources(
        id INTEGER PRIMARY KEY, task_id TEXT NOT NULL, machine_id TEXT NOT NULL,
        session TEXT NOT NULL, terminal_id TEXT NOT NULL, process_id TEXT NOT NULL,
        target_json TEXT NOT NULL, reason TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'tracked',
        error TEXT NOT NULL DEFAULT '', runtime_json TEXT, session_id TEXT, cwd TEXT, epoch INTEGER NOT NULL DEFAULT 0,
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        UNIQUE(machine_id,session,terminal_id,process_id));
        CREATE TABLE IF NOT EXISTS session_operation_epochs(operation_id TEXT PRIMARY KEY,epoch INTEGER NOT NULL);
        CREATE TRIGGER IF NOT EXISTS resource_operation_epoch AFTER INSERT ON lifecycle_operations BEGIN
          INSERT OR IGNORE INTO session_operation_epochs SELECT NEW.id,execution_epoch FROM tasks WHERE id=NEW.task_id; END;")?;
    let capture = |source: &str, task: &str, machine: &str, reason: &str| {
        format!(
        "INSERT INTO session_resources(task_id,machine_id,session,terminal_id,process_id,target_json,reason,session_id,cwd,epoch)
         SELECT {task},{machine},json_extract({source},'$.session'),json_extract({source},'$.terminal_id'),json_extract({source},'$.process_id'),{source},'{reason}',
           (SELECT codex_session_id FROM task_sessions WHERE task_id={task}),
           (SELECT cwd FROM task_sessions WHERE task_id={task}),
           COALESCE((SELECT execution_epoch FROM tasks WHERE id={task}),0)
         WHERE {source} IS NOT NULL AND json_valid({source}) AND json_extract({source},'$.terminal_id') IS NOT NULL AND json_extract({source},'$.process_id') IS NOT NULL AND json_extract({source},'$.session') IS NOT NULL
         ON CONFLICT(machine_id,session,terminal_id,process_id) DO UPDATE SET
           target_json=excluded.target_json,task_id=excluded.task_id,reason=excluded.reason,state='tracked',epoch=excluded.epoch,
           session_id=COALESCE(excluded.session_id,session_resources.session_id),cwd=COALESCE(excluded.cwd,session_resources.cwd),updated_at=CURRENT_TIMESTAMP;")
    };
    for (name, event) in [("insert", "INSERT"), ("update", "UPDATE OF target_json")] {
        tx.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS resource_binding_{name} AFTER {event} ON task_bindings BEGIN {} END;",capture("NEW.target_json","NEW.task_id","NEW.machine_id","bound")))?;
    }
    tx.execute_batch("CREATE TRIGGER IF NOT EXISTS resource_binding_replace BEFORE UPDATE OF target_json ON task_bindings BEGIN
        UPDATE session_resources SET reason='detached',updated_at=CURRENT_TIMESTAMP WHERE task_id=OLD.task_id AND target_json=OLD.target_json AND target_json<>NEW.target_json AND state<>'closed'; END;
        CREATE TRIGGER IF NOT EXISTS resource_task_delete BEFORE DELETE ON tasks BEGIN
        UPDATE session_resources SET reason='deleted',session_id=COALESCE((SELECT codex_session_id FROM task_sessions WHERE task_id=OLD.id),session_id),
          cwd=COALESCE((SELECT cwd FROM task_sessions WHERE task_id=OLD.id),cwd),updated_at=CURRENT_TIMESTAMP WHERE task_id=OLD.id AND state<>'closed';
        END;
        CREATE TRIGGER IF NOT EXISTS resource_binding_detach BEFORE DELETE ON task_bindings BEGIN
        UPDATE session_resources SET reason=CASE WHEN reason='deleted' THEN reason ELSE 'detached' END,
          session_id=COALESCE((SELECT codex_session_id FROM task_sessions WHERE task_id=OLD.task_id),session_id),
          cwd=COALESCE((SELECT cwd FROM task_sessions WHERE task_id=OLD.task_id),cwd),updated_at=CURRENT_TIMESTAMP
          WHERE task_id=OLD.task_id AND state<>'closed'; END;")?;
    if !tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=22)",
        [],
        |r| r.get::<_, bool>(0),
    )? {
        // Historical records never constitute permission for automatic closure.
        let mut q=tx.prepare("SELECT task_id,machine_id,target_json FROM task_bindings
            UNION ALL SELECT task_id,machine_id,target_json FROM lifecycle_operations WHERE target_json IS NOT NULL
            UNION ALL SELECT task_id,machine_id,target_json FROM dispatch_operations
            UNION ALL SELECT COALESCE(o.task_id,''),a.machine_id,a.target_json FROM pane_allocations a LEFT JOIN lifecycle_operations o ON o.id=a.operation_id WHERE a.target_json IS NOT NULL
            UNION ALL SELECT r.task_id,COALESCE(json_extract(t.target_json,'$.machine_id'),'local'),t.target_json FROM retired_execution_targets t JOIN task_execution_resets r ON r.id=t.reset_id")?;
        let records = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(q);
        for (task, machine, json) in records {
            if let Ok(t) = serde_json::from_str::<Target>(&json) {
                tx.execute("INSERT OR IGNORE INTO session_resources(task_id,machine_id,session,terminal_id,process_id,target_json,reason) VALUES(?1,?2,?3,?4,?5,?6,'history')",params![task,machine,t.session,t.terminal_id,t.process_id,json])?;
            }
        }
        tx.execute("INSERT INTO schema_migrations(version) VALUES(22)", [])?;
    }
    tx.commit()
}

pub fn list(db: &Connection) -> Result<Vec<Resource>, String> {
    let mut q=db.prepare("SELECT id,task_id,target_json,reason,state,error FROM session_resources WHERE state<>'closed' ORDER BY id DESC").map_err(|e|e.to_string())?;
    q.query_map([], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get::<_, String>(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
        ))
    })
    .map_err(|e| e.to_string())?
    .map(|r| {
        let (id, task_id, json, reason, state, error) = r.map_err(|e| e.to_string())?;
        Ok(Resource {
            id,
            task_id,
            target: serde_json::from_str(&json).map_err(|e| e.to_string())?,
            reason,
            state,
            error,
        })
    })
    .collect()
}

/// Record a creation receipt even when its task was removed while Herdr replied.
pub fn record(db: &Connection, task: &str, target: &Target, reason: &str) -> Result<(), String> {
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1)",
            [task],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let reason = if reason == "created" && !exists {
        "deleted"
    } else {
        reason
    };
    db.execute("INSERT INTO session_resources(task_id,machine_id,session,terminal_id,process_id,target_json,reason,epoch)
      VALUES(?1,?2,?3,?4,?5,?6,?7,COALESCE((SELECT execution_epoch FROM tasks WHERE id=?1),-1)) ON CONFLICT(machine_id,session,terminal_id,process_id) DO UPDATE SET target_json=excluded.target_json",
      params![task,target.machine_id,target.session,target.terminal_id,target.process_id,serde_json::to_string(target).map_err(|e|e.to_string())?,reason]).map_err(|e|e.to_string())?;
    Ok(())
}

/// Operation-scoped epochs survive reset and deletion before an external reply.
pub fn record_created(
    db: &Connection,
    task: &str,
    operation: &str,
    target: &Target,
) -> Result<(), String> {
    record(db, task, target, "created")?;
    db.execute("UPDATE session_resources SET epoch=COALESCE((SELECT epoch FROM session_operation_epochs WHERE operation_id=?1),epoch) WHERE machine_id=?2 AND session=?3 AND terminal_id=?4 AND process_id=?5",params![operation,target.machine_id,target.session,target.terminal_id,target.process_id]).map_err(|e|e.to_string())?;
    Ok(())
}
pub fn save_runtime(
    db: &Connection,
    runtime: &crate::runtime_processes::Runtime,
) -> Result<(), String> {
    db.execute("UPDATE session_resources SET runtime_json=?4 WHERE machine_id=?1 AND session=?2 AND terminal_id=?3 AND process_id=?5",params![runtime.target.machine_id,runtime.target.session,runtime.target.terminal_id,serde_json::to_string(runtime).map_err(|e|e.to_string())?,runtime.target.process_id]).map_err(|e|e.to_string())?;
    Ok(())
}
pub fn released(db: &Connection, target: &Target) -> Result<(), String> {
    db.execute("UPDATE session_resources SET state='closed',error='' WHERE machine_id=?1 AND session=?2 AND terminal_id=?3 AND process_id=?4",params![target.machine_id,target.session,target.terminal_id,target.process_id]).map_err(|e|e.to_string())?;
    Ok(())
}

/// One bounded attempt per known deletion. Discovery alone never authorizes a close.
pub fn drain_deleted(app: &crate::workflow::Workflow) -> Result<(), String> {
    let resources = list(&*app.db.lock().map_err(|e| e.to_string())?)?;
    for resource in resources
        .into_iter()
        .filter(|r| r.reason == "deleted" && r.state == "tracked")
        .take(16)
    {
        app.check_running()?;
        let claimed = app
            .db
            .lock()
            .map_err(|e| e.to_string())?
            .execute(
                "UPDATE session_resources SET state='releasing' WHERE id=?1 AND state='tracked'",
                [resource.id],
            )
            .map_err(|e| e.to_string())?;
        if claimed != 1 {
            continue;
        }
        let (mut state, error) = match app.release_deleted_resource(resource.id) {
            Ok(batch) => batch
                .items
                .first()
                .map(|i| (i.state.clone(), i.error.clone()))
                .unwrap_or(("failed".into(), "cleanup.stale".into())),
            Err(error) => ("failed".into(), error),
        };
        if error == "session_cleanup.busy" {
            state = "tracked".into();
        }
        app.db
            .lock()
            .map_err(|e| e.to_string())?
            .execute(
                "UPDATE session_resources SET state=?2,error=?3 WHERE id=?1",
                params![resource.id, state, error],
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub fn start(app: std::sync::Arc<crate::workflow::Workflow>) {
    std::thread::spawn(move || {
        while app.check_running().is_ok() {
            if let Err(error) = drain_deleted(&app) {
                eprintln!("session resource cleanup: {error}");
            }
            for _ in 0..20 {
                if app.check_running().is_err() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deleting_a_card_keeps_its_runtime_resource_visible() {
        let mut db = Connection::open_in_memory().unwrap();
        crate::db::init(&db).unwrap();
        db.execute_batch("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at) VALUES('p','P','.','P','now');
          INSERT INTO tasks(id,project_id,name,status,created_at,updated_at) VALUES('t','p','T','in_progress','now','now');").unwrap();
        let target = Target {
            machine_id: "local".into(),
            session: "default".into(),
            workspace_id: "w1".into(),
            pane_id: "w1:p1".into(),
            terminal_id: "terminal".into(),
            agent: "codex".into(),
            process_id: "10:20".into(),
            codex_resume_locks: vec![],
            title: String::new(),
            status: "idle".into(),
        };
        db.execute("INSERT INTO task_bindings(task_id,machine_id,session,pane_id,target_json,state) VALUES('t','local','default','w1:p1',?1,'valid')",[serde_json::to_string(&target).unwrap()]).unwrap();
        crate::tasks::delete(&mut db, "t").unwrap();
        assert!(crate::tasks::list(&db, "p").unwrap().is_empty());
        let resources = list(&db).unwrap();
        assert_eq!(
            resources.len(),
            1,
            "deleting the card must not discard cleanup responsibility"
        );
        assert_eq!(resources[0].target, target);
        assert_eq!(resources[0].reason, "deleted");
    }
}
