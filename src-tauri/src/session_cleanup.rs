//! Durable global cleanup. Preview is not authority to close a replacement target.
use crate::{
    runtime_processes::Runtime,
    workflow::{Target, Workflow},
};
use rusqlite::{Connection, OptionalExtension, params};
use std::sync::Arc;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Scope {
    pub machine: String,
    pub workspace: String,
    pub error: String,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Item {
    pub id: String,
    pub target: Target,
    pub machine: String,
    pub workspace: String,
    pub state: String,
    pub error: String,
    pub warning: String,
}
#[derive(Debug, serde::Serialize)]
pub struct Batch {
    pub id: String,
    pub state: String,
    pub items: Vec<Item>,
    pub scopes: Vec<Scope>,
}

pub fn init(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS session_cleanup_batches(id TEXT PRIMARY KEY,created_at INTEGER NOT NULL,state TEXT NOT NULL,scopes_json TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS session_cleanup_items(id TEXT PRIMARY KEY,batch_id TEXT NOT NULL REFERENCES session_cleanup_batches(id),target_json TEXT NOT NULL,
        machine TEXT NOT NULL,workspace TEXT NOT NULL,state TEXT NOT NULL,error TEXT NOT NULL DEFAULT '',runtime_json TEXT,owner_task TEXT,warning TEXT NOT NULL DEFAULT '');
      CREATE INDEX IF NOT EXISTS session_cleanup_batch ON session_cleanup_items(batch_id);
      CREATE TABLE IF NOT EXISTS session_cleanup_idle_releases(batch_id TEXT NOT NULL,ordinal INTEGER NOT NULL,item_id TEXT NOT NULL,PRIMARY KEY(batch_id,ordinal));")
}
pub fn recover(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("UPDATE session_resources SET state='unknown',error='session_cleanup.interrupted' WHERE state='releasing';
      UPDATE session_cleanup_items SET state='unknown',error='session_cleanup.interrupted' WHERE state IN ('claimed','closing');
      UPDATE session_cleanup_items SET state='skipped',error='session_cleanup.interrupted' WHERE state='ready' AND batch_id IN (SELECT id FROM session_cleanup_batches WHERE state='running');
      UPDATE session_cleanup_batches SET state='done' WHERE state IN ('running','retrying');")
}
pub fn ensure_unclaimed(db: &Connection, t: &Target, own: Option<&str>) -> Result<(), String> {
    let busy:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM session_cleanup_items WHERE state IN ('claimed','closing','unknown') AND (?1 IS NULL OR id<>?1)
      AND COALESCE(json_extract(target_json,'$.machine_id'),'local')=?2 AND json_extract(target_json,'$.session')=?3
      AND (json_extract(target_json,'$.terminal_id')=?4 OR json_extract(target_json,'$.pane_id')=?5))",
      params![own,t.machine_id,t.session,t.terminal_id,t.pane_id],|r|r.get(0)).map_err(err)?;
    if busy {
        Err("cleanup.pending".into())
    } else {
        Ok(())
    }
}
pub fn ensure_task_unclaimed(db: &Connection, task: &str) -> Result<(), String> {
    let busy:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM session_cleanup_items WHERE state IN ('claimed','closing','unknown') AND (owner_task=?1 OR EXISTS(
      SELECT 1 FROM lifecycle_operations o WHERE o.task_id=?1 AND o.machine_id=COALESCE(json_extract(session_cleanup_items.target_json,'$.machine_id'),'local')
       AND json_extract(o.target_json,'$.session')=json_extract(session_cleanup_items.target_json,'$.session') AND json_extract(o.target_json,'$.terminal_id')=json_extract(session_cleanup_items.target_json,'$.terminal_id')
      UNION ALL SELECT 1 FROM dispatch_operations o WHERE o.task_id=?1 AND o.machine_id=COALESCE(json_extract(session_cleanup_items.target_json,'$.machine_id'),'local')
       AND json_extract(o.target_json,'$.session')=json_extract(session_cleanup_items.target_json,'$.session') AND json_extract(o.target_json,'$.terminal_id')=json_extract(session_cleanup_items.target_json,'$.terminal_id'))))",[task],|r|r.get(0)).map_err(err)?;
    if busy {
        Err("cleanup.pending".into())
    } else {
        Ok(())
    }
}

impl Workflow {
    fn cleanup_check(
        &self,
        db: &Connection,
        t: &Target,
        own: Option<&str>,
    ) -> Result<Option<String>, String> {
        self.check_running()?;
        crate::machines::ensure_enabled(db, &t.machine_id)?;
        ensure_unclaimed(db, t, own)?;
        let idle_running:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM idle_cleanup_items i JOIN idle_cleanup_batches b ON b.id=i.batch_id WHERE b.state='running' AND i.state IN ('claimed','closing','unknown') AND b.machine_id=?1 AND b.session=?2 AND (json_extract(i.candidate_json,'$.pane.target.terminal_id')=?3 OR json_extract(i.candidate_json,'$.pane.target.pane_id')=?4))",params![t.machine_id,t.session,t.terminal_id,t.pane_id],|r|r.get(0)).map_err(err)?;
        if idle_running {
            return Err("session_cleanup.busy".into());
        }
        let mut q=db.prepare("SELECT b.task_id,t.status,b.target_json FROM task_bindings b JOIN tasks t ON t.id=b.task_id
            WHERE b.machine_id=?1 AND b.session=?2 AND (b.pane_id=?3 OR json_extract(b.target_json,'$.terminal_id')=?4)").map_err(err)?;
        let owners = q
            .query_map(
                params![t.machine_id, t.session, t.pane_id, t.terminal_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        if owners.iter().any(|(_, state, _)| state == "in_progress") {
            return Err("session_cleanup.in_progress".into());
        }
        if owners.len() > 1 {
            return Err("binding.identity_changed".into());
        }
        let mut q=db.prepare("SELECT task_id FROM lifecycle_operations WHERE machine_id=?1 AND target_json IS NOT NULL
          AND json_extract(target_json,'$.session')=?2 AND (json_extract(target_json,'$.terminal_id')=?3 OR json_extract(target_json,'$.pane_id')=?4)
          UNION SELECT task_id FROM dispatch_operations WHERE machine_id=?1 AND json_extract(target_json,'$.session')=?2
          AND (json_extract(target_json,'$.terminal_id')=?3 OR json_extract(target_json,'$.pane_id')=?4)").map_err(err)?;
        let tasks = q
            .query_map(
                params![t.machine_id, t.session, t.terminal_id, t.pane_id],
                |r| r.get::<_, String>(0),
            )
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        if tasks
            .iter()
            .chain(owners.iter().map(|(id, _, _)| id))
            .any(|task| crate::session_activity::busy(self, task))
        {
            return Err("session_cleanup.busy".into());
        }
        Ok(owners.first().and_then(|(id, _, json)| {
            serde_json::from_str::<Target>(json)
                .ok()
                .filter(|bound| bound.same_identity(t))
                .map(|_| id.clone())
        }))
    }
    fn new_cleanup_batch(
        &self,
        items: &[(Target, String, String)],
        scopes: &[Scope],
    ) -> Result<Batch, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let mut db = self.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        tx.execute(
            "INSERT INTO session_cleanup_batches VALUES(?1,?2,'preview',?3)",
            params![
                id,
                chrono::Utc::now().timestamp(),
                serde_json::to_string(scopes).map_err(err)?
            ],
        )
        .map_err(err)?;
        for (target, machine, workspace) in items {
            let (state, error, owner) = match self.cleanup_check(&tx, target, None) {
                Ok(owner) => ("ready", String::new(), owner),
                Err(error) => ("protected", error, None),
            };
            crate::session_resources::record(
                &tx,
                owner.as_deref().unwrap_or(""),
                target,
                "discovered",
            )?;
            tx.execute("INSERT INTO session_cleanup_items(id,batch_id,target_json,machine,workspace,state,error,owner_task) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![uuid::Uuid::new_v4().to_string(),id,serde_json::to_string(target).map_err(err)?,machine,workspace,state,error,owner]).map_err(err)?;
        }
        tx.commit().map_err(err)?;
        drop(db);
        self.cleanup_result(&id, false)
    }
    pub fn preview_session_cleanup(&self) -> Result<Batch, String> {
        let machines = crate::machines::list(&*self.db.lock().map_err(err)?)?;
        let mut items = Vec::new();
        let mut scopes = Vec::new();
        for machine in machines {
            let _scope = crate::remote_control::Scope::enter();
            self.check_running()?;
            if machine.disabled {
                scopes.push(Scope {
                    machine: machine.name,
                    workspace: String::new(),
                    error: "machine.disabled".into(),
                });
                continue;
            }
            let result = (|| {
                let adapter = crate::machine_routing::adapter(&self.db, &self.herdr, &machine.id)?;
                let workspaces = adapter.workspaces()?;
                let ids = workspaces.iter().map(|w| w.id.clone()).collect::<Vec<_>>();
                let mut discoveries = adapter.discover_agent_batch(&ids)?;
                for workspace in workspaces {
                    match discoveries
                        .remove(&workspace.id)
                        .unwrap_or_else(|| Err("herdr.invalid_response".into()))
                    {
                        Ok(discovery) => {
                            for failure in discovery.failures {
                                scopes.push(Scope {
                                    machine: machine.name.clone(),
                                    workspace: workspace.name.clone(),
                                    error: format!("{}: {}", failure.pane_id, failure.error),
                                });
                            }
                            for target in discovery.targets {
                                if target.agent != "shell" && !target.agent.is_empty() {
                                    if target.machine_id != machine.id
                                        || target.session != adapter.session()
                                        || target.workspace_id != workspace.id
                                    {
                                        return Err("binding.identity_changed".into());
                                    }
                                    items.push((
                                        target,
                                        machine.name.clone(),
                                        workspace.name.clone(),
                                    ));
                                }
                            }
                        }
                        Err(error) => scopes.push(Scope {
                            machine: machine.name.clone(),
                            workspace: workspace.name,
                            error,
                        }),
                    }
                }
                Ok::<_, String>(())
            })();
            if let Err(error) = result {
                scopes.push(Scope {
                    machine: machine.name,
                    workspace: String::new(),
                    error,
                });
            }
        }
        self.new_cleanup_batch(&items, &scopes)
    }
    pub fn cleanup_result(&self, id: &str, verify: bool) -> Result<Batch, String> {
        let (state, scopes, items) = {
            let db = self.db.lock().map_err(err)?;
            let (state, scopes): (String, String) = db
                .query_row(
                    "SELECT state,scopes_json FROM session_cleanup_batches WHERE id=?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(err)?;
            let mut q=db.prepare("SELECT id,target_json,machine,workspace,state,error,warning FROM session_cleanup_items WHERE batch_id=?1 ORDER BY rowid").map_err(err)?;
            let rows = q
                .query_map([id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, String>(6)?,
                    ))
                })
                .map_err(err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(err)?;
            let items = rows
                .into_iter()
                .map(|(id, json, machine, workspace, state, error, warning)| {
                    Ok(Item {
                        id,
                        target: serde_json::from_str(&json).map_err(err)?,
                        machine,
                        workspace,
                        state,
                        error,
                        warning,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            (
                state,
                serde_json::from_str::<Vec<Scope>>(&scopes).map_err(err)?,
                items,
            )
        };
        if verify && state == "done" {
            for item in items.iter().filter(|i| i.state == "unknown") {
                if let Err(error) = self.verify_cleanup_item(item) {
                    self.verification_error(item, &error)?;
                }
            }
            return self.cleanup_result(id, false);
        }
        Ok(Batch {
            id: id.into(),
            state,
            items,
            scopes,
        })
    }
    pub fn cleanup_history(&self) -> Result<Vec<Batch>, String> {
        let ids = {
            let db = self.db.lock().map_err(err)?;
            let mut q=db.prepare("SELECT id FROM session_cleanup_batches WHERE state<>'preview' ORDER BY created_at DESC,rowid DESC LIMIT 20").map_err(err)?;
            q.query_map([], |r| r.get::<_, String>(0))
                .map_err(err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(err)?
        };
        ids.iter()
            .map(|id| self.cleanup_result(id, false))
            .collect()
    }
    fn item_state(&self, item: &Item, state: &str, error: &str) -> Result<(), String> {
        self.db
            .lock()
            .map_err(err)?
            .execute(
                "UPDATE session_cleanup_items SET state=?2,error=?3 WHERE id=?1",
                params![item.id, state, error],
            )
            .map_err(err)?;
        Ok(())
    }
    fn verification_error(&self, item: &Item, error: &str) -> Result<(), String> {
        self.db.lock().map_err(err)?.execute("UPDATE session_cleanup_items SET error=?2 WHERE id=?1 AND state='unknown' AND batch_id IN (SELECT id FROM session_cleanup_batches WHERE state='done')",params![item.id,error]).map_err(err)?;
        Ok(())
    }
    fn finish_session_cleanup_item(&self, item: &Item, state: &str) -> Result<(), String> {
        let mut db = self.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        let owner = self.cleanup_check(&tx, &item.target, Some(&item.id))?;
        if let Some(task) = owner {
            // Keep the original unknown facts under a retired execution. This
            // revokes old workers without claiming their inputs were delivered.
            let retired = format!("retired:cleanup:{}", item.id);
            for table in ["lifecycle_operations", "dispatch_operations"] {
                tx.execute(&format!("UPDATE {table} SET task_id=?2 WHERE task_id=?1 AND state IN ('pending','unknown')"),params![task,retired]).map_err(err)?;
            }
            tx.execute(
                "UPDATE tasks SET execution_epoch=execution_epoch+1 WHERE id=?1",
                [&task],
            )
            .map_err(err)?;
            tx.execute("DELETE FROM task_bindings WHERE task_id=?1 AND json_extract(target_json,'$.terminal_id')=?2 AND json_extract(target_json,'$.process_id')=?3", params![task,item.target.terminal_id,item.target.process_id])
                .map_err(err)?;
        }
        tx.execute("UPDATE workspace_terminals SET state='failed',target_json=NULL WHERE machine_id=?1 AND session=?2 AND json_extract(target_json,'$.terminal_id')=?3",params![item.target.machine_id,item.target.session,item.target.terminal_id]).map_err(err)?;
        for table in ["lifecycle_operations", "dispatch_operations"] {
            tx.execute(&format!("UPDATE {table} SET task_id=?1 WHERE state IN ('pending','unknown') AND machine_id=?2 AND json_extract(target_json,'$.session')=?3 AND json_extract(target_json,'$.terminal_id')=?4"),params![format!("retired:cleanup:{}",item.id),item.target.machine_id,item.target.session,item.target.terminal_id]).map_err(err)?;
        }
        let error = if state == "absent" {
            "session_cleanup.already_absent"
        } else {
            ""
        };
        tx.execute("UPDATE session_resources SET state=?4,error=?5,updated_at=CURRENT_TIMESTAMP WHERE machine_id=?1 AND session=?2 AND terminal_id=?3 AND process_id=?6",params![item.target.machine_id,item.target.session,item.target.terminal_id,state,error,item.target.process_id]).map_err(err)?;
        tx.execute(
            "UPDATE session_cleanup_items SET state=?2,error=?3 WHERE id=?1",
            params![item.id, state, error],
        )
        .map_err(err)?;
        tx.commit().map_err(err)
    }
    fn verify_cleanup_item(&self, item: &Item) -> Result<(), String> {
        let _scope = crate::remote_control::Scope::enter();
        let runtime: Option<String> = self
            .db
            .lock()
            .map_err(err)?
            .query_row(
                "SELECT runtime_json FROM session_cleanup_items WHERE id=?1",
                [&item.id],
                |r| r.get(0),
            )
            .map_err(err)?;
        let adapter =
            crate::machine_routing::adapter(&self.db, &self.herdr, &item.target.machine_id)?;
        if let Some(json) = runtime {
            let runtime: Runtime = serde_json::from_str(&json).map_err(err)?;
            if adapter.runtime_absent(&runtime)? {
                return self.finish_session_cleanup_item(item, "closed");
            }
        } else if adapter.cleanup_target_absent(&item.target)? {
            return self.finish_session_cleanup_item(item, "absent");
        }
        if let Err(error) = adapter.validate(&item.target.workspace_id, &item.target) {
            if error == "binding.identity_changed" {
                self.db.lock().map_err(err)?.execute("UPDATE session_cleanup_items SET state='skipped',error=?2 WHERE id=?1 AND state='unknown'",params![item.id,error]).map_err(err)?;
            }
        }
        Ok(())
    }
    fn release_cleanup_item(&self, item: &Item) -> Result<(), String> {
        let _scope = crate::remote_control::Scope::enter();
        let target = &item.target;
        let gate = crate::lifecycle::pane_allocation::workspace_lock(
            &target.machine_id,
            &target.session,
            &target.workspace_id,
        )?;
        let _guard = gate.lock().map_err(err)?;
        let adapter = crate::machine_routing::adapter(&self.db, &self.herdr, &target.machine_id)?;
        {
            let mut db = self.db.lock().map_err(err)?;
            let tx = db.transaction().map_err(err)?;
            let owner = self.cleanup_check(&tx, target, Some(&item.id))?;
            if tx.execute("UPDATE session_cleanup_items SET state='claimed',owner_task=?2 WHERE id=?1 AND state='ready'",params![item.id,owner]).map_err(err)?!=1{return Err("cleanup.stale".into());}
            tx.commit().map_err(err)?;
        }
        if adapter.cleanup_target_absent(target)? {
            let stored:Option<String>=self.db.lock().map_err(err)?.query_row("SELECT runtime_json FROM session_resources WHERE machine_id=?1 AND session=?2 AND terminal_id=?3 AND process_id=?4 AND runtime_json IS NOT NULL ORDER BY id DESC LIMIT 1",params![target.machine_id,target.session,target.terminal_id,target.process_id],|r|r.get(0)).optional().map_err(err)?;
            if let Some(json) = stored {
                let runtime: Runtime = serde_json::from_str(&json).map_err(err)?;
                self.db
                    .lock()
                    .map_err(err)?
                    .execute(
                        "UPDATE session_cleanup_items SET runtime_json=?2 WHERE id=?1",
                        params![item.id, json],
                    )
                    .map_err(err)?;
                if adapter.runtime_absent(&runtime)? {
                    return self.finish_session_cleanup_item(item, "closed");
                }
                return self.item_state(item, "unknown", "session_cleanup.awaiting_exit");
            }
            // An old PID without a birth stamp is not proof of process release.
            self.finish_session_cleanup_item(item, "absent")?;
            return Ok(());
        }
        let mut runtime = adapter.capture_runtime(target)?;
        let prior: Option<String> = self
            .db
            .lock()
            .map_err(err)?
            .query_row(
                "SELECT runtime_json FROM session_cleanup_items WHERE id=?1",
                [&item.id],
                |r| r.get(0),
            )
            .map_err(err)?;
        if let Some(json) = prior {
            let old: Runtime = serde_json::from_str(&json).map_err(err)?;
            for receipt in old.receipt_ids {
                if !runtime.receipt_ids.contains(&receipt) {
                    runtime.receipt_ids.push(receipt);
                }
            }
            for process in old.processes {
                if !runtime
                    .processes
                    .iter()
                    .any(|p| p.pid == process.pid && p.birth == process.birth)
                {
                    runtime.processes.push(process);
                }
            }
        }
        if !runtime.target.same_identity(target) {
            return Err("binding.identity_changed".into());
        }
        // Session discovery is best-effort for cleanup, never a close prerequisite.
        let session = adapter
            .session_id(target)
            .ok()
            .flatten()
            .filter(|s| uuid::Uuid::parse_str(s).is_ok());
        if let Some(session) = session {
            let db = self.db.lock().map_err(err)?;
            let owner = self.cleanup_check(&db, target, Some(&item.id))?;
            db.execute("UPDATE session_resources SET session_id=?4 WHERE machine_id=?1 AND session=?2 AND terminal_id=?3",params![target.machine_id,target.session,target.terminal_id,session]).map_err(err)?;
            if let Some(task) = owner {
                db.execute("INSERT INTO task_sessions(task_id,codex_session_id,cwd,updated_at,machine_id)
                      SELECT t.id,?2,COALESCE(d.cwd,p.root_path),?3,?4 FROM tasks t JOIN projects p ON p.id=t.project_id LEFT JOIN task_working_directories d ON d.task_id=t.id WHERE t.id=?1
                      ON CONFLICT(task_id) DO UPDATE SET codex_session_id=excluded.codex_session_id,cwd=excluded.cwd,updated_at=excluded.updated_at",params![task,session,chrono::Utc::now().to_rfc3339(),target.machine_id]).map_err(err)?;
            }
        } else {
            self.db.lock().map_err(err)?.execute("UPDATE session_cleanup_items SET warning='session_cleanup.history_unavailable' WHERE id=?1",[&item.id]).map_err(err)?;
        }
        crate::session_resources::save_runtime(&*self.db.lock().map_err(err)?, &runtime)?;
        self.db
            .lock()
            .map_err(err)?
            .execute(
                "UPDATE session_cleanup_items SET runtime_json=?2 WHERE id=?1",
                params![item.id, serde_json::to_string(&runtime).map_err(err)?],
            )
            .map_err(err)?;
        let result = adapter.close_runtime(&runtime, &|| {
            let mut db = self.db.lock().map_err(err)?;
            let tx = db.transaction().map_err(err)?;
            self.cleanup_check(&tx, target, Some(&item.id))?;
            tx.execute("INSERT OR IGNORE INTO session_cleanup_idle_releases SELECT i.batch_id,i.ordinal,?1 FROM idle_cleanup_items i JOIN idle_cleanup_batches b ON b.id=i.batch_id WHERE i.state='unknown' AND b.state<>'running' AND b.machine_id=?2 AND b.session=?3 AND json_extract(i.candidate_json,'$.pane.target.terminal_id')=?4",params![item.id,target.machine_id,target.session,target.terminal_id]).map_err(err)?;
            tx.execute(
                "UPDATE session_cleanup_items SET state='closing' WHERE id=?1",
                [&item.id],
            )
            .map_err(err)?;
            tx.commit().map_err(err)
        });
        match result {
            Ok(()) => {
                self.item_state(item, "unknown", "session_cleanup.awaiting_exit")?;
                for _ in 0..20 {
                    if adapter.runtime_absent(&runtime)? {
                        return self.finish_session_cleanup_item(item, "closed");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Ok(())
            }
            Err(e) => self.item_state(
                item,
                if e.uncertain { "unknown" } else { "failed" },
                &e.message,
            ),
        }
    }
    pub fn execute_session_cleanup(&self, id: &str) -> Result<Batch, String> {
        {
            let db = self.db.lock().map_err(err)?;
            let (state, created): (String, i64) = db
                .query_row(
                    "SELECT state,created_at FROM session_cleanup_batches WHERE id=?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(err)?;
            if state != "preview" {
                drop(db);
                return self.cleanup_result(id, false);
            }
            if chrono::Utc::now().timestamp() - created > 600 {
                return Err("cleanup.stale".into());
            }
            if db.execute("UPDATE session_cleanup_batches SET state='running' WHERE id=?1 AND state='preview'",[id]).map_err(err)?!=1{return Err("cleanup.stale".into());}
        }
        for item in self
            .cleanup_result(id, false)?
            .items
            .iter()
            .filter(|i| i.state == "ready")
        {
            if let Err(error) = self.release_cleanup_item(item) {
                let state: String = self
                    .db
                    .lock()
                    .map_err(err)?
                    .query_row(
                        "SELECT state FROM session_cleanup_items WHERE id=?1",
                        [&item.id],
                        |r| r.get(0),
                    )
                    .map_err(err)?;
                let next = if matches!(state.as_str(), "closing" | "unknown") {
                    "unknown"
                } else if matches!(
                    error.as_str(),
                    "session_cleanup.in_progress"
                        | "session_cleanup.busy"
                        | "binding.identity_changed"
                        | "cleanup.pending"
                ) {
                    "skipped"
                } else {
                    "failed"
                };
                self.item_state(item, next, &error)?;
            }
        }
        self.db
            .lock()
            .map_err(err)?
            .execute(
                "UPDATE session_cleanup_batches SET state='done' WHERE id=?1",
                [id],
            )
            .map_err(err)?;
        self.cleanup_result(id, false)
    }
    pub fn retry_session_cleanup(&self, id: &str) -> Result<Batch, String> {
        let batch = self.cleanup_result(id, true)?;
        if batch.state != "done" {
            return Err("cleanup.pending".into());
        }
        if self
            .db
            .lock()
            .map_err(err)?
            .execute(
                "UPDATE session_cleanup_batches SET state='retrying' WHERE id=?1 AND state='done'",
                [id],
            )
            .map_err(err)?
            != 1
        {
            return Err("cleanup.pending".into());
        }
        for item in batch.items.iter().filter(|i| i.state == "unknown") {
            let _scope = crate::remote_control::Scope::enter();
            let result = (|| {
                let adapter = crate::machine_routing::adapter(
                    &self.db,
                    &self.herdr,
                    &item.target.machine_id,
                )?;
                let current = adapter.capture_runtime(&item.target)?;
                if !current.target.same_identity(&item.target) {
                    return Err("binding.identity_changed".into());
                }
                let old: Option<String> = self
                    .db
                    .lock()
                    .map_err(err)?
                    .query_row(
                        "SELECT runtime_json FROM session_cleanup_items WHERE id=?1",
                        [&item.id],
                        |r| r.get(0),
                    )
                    .map_err(err)?;
                if let Some(json) = old {
                    let old: Runtime = serde_json::from_str(&json).map_err(err)?;
                    let root = item
                        .target
                        .process_id
                        .split(':')
                        .next()
                        .and_then(|s| s.parse::<u32>().ok());
                    if old
                        .processes
                        .iter()
                        .filter(|p| Some(p.pid) == root)
                        .any(|p| {
                            !current
                                .processes
                                .iter()
                                .any(|n| n.pid == p.pid && n.birth == p.birth)
                        })
                    {
                        return Err("binding.identity_changed".into());
                    }
                }
                let mut db = self.db.lock().map_err(err)?;
                let tx = db.transaction().map_err(err)?;
                self.cleanup_check(&tx, &item.target, Some(&item.id))?;
                tx.execute("UPDATE session_cleanup_items SET state='ready',error='' WHERE id=?1 AND state='unknown'",[&item.id]).map_err(err)?;
                tx.commit().map_err(err)
            })();
            if let Err(error) = result {
                self.item_state(
                    item,
                    if error == "binding.identity_changed" {
                        "skipped"
                    } else {
                        "unknown"
                    },
                    &error,
                )?;
            }
        }
        {
            let db = self.db.lock().map_err(err)?;
            db.execute("UPDATE session_cleanup_batches SET state='preview',created_at=?2 WHERE id=?1 AND state='retrying'",params![id,chrono::Utc::now().timestamp()]).map_err(err)?;
        }
        self.execute_session_cleanup(id)
    }
    pub fn preview_session_resource(&self, id: i64) -> Result<Batch, String> {
        let resource = crate::session_resources::list(&*self.db.lock().map_err(err)?)?
            .into_iter()
            .find(|r| r.id == id)
            .ok_or("cleanup.stale")?;
        let target = &resource.target;
        let previous:Option<String>=self.db.lock().map_err(err)?.query_row("SELECT batch_id FROM session_cleanup_items WHERE state IN ('unknown','claimed','closing') AND COALESCE(json_extract(target_json,'$.machine_id'),'local')=?1 AND json_extract(target_json,'$.session')=?2 AND json_extract(target_json,'$.terminal_id')=?3 ORDER BY rowid DESC LIMIT 1",params![target.machine_id,target.session,target.terminal_id],|r|r.get(0)).optional().map_err(err)?;
        if let Some(id) = previous {
            return self.cleanup_result(&id, true);
        }
        self.new_cleanup_batch(
            &[(
                target.clone(),
                target.machine_id.clone(),
                target.workspace_id.clone(),
            )],
            &[],
        )
    }
    pub fn release_deleted_resource(&self, resource_id: i64) -> Result<Batch, String> {
        let resource = crate::session_resources::list(&*self.db.lock().map_err(err)?)?
            .into_iter()
            .find(|r| r.id == resource_id)
            .ok_or("cleanup.stale")?;
        if resource.reason != "deleted" {
            return Err("cleanup.protected".into());
        }
        let batch = self.new_cleanup_batch(
            &[(
                resource.target.clone(),
                resource.target.machine_id.clone(),
                resource.target.workspace_id.clone(),
            )],
            &[],
        )?;
        self.execute_session_cleanup(&batch.id)
    }
}

#[tauri::command]
pub async fn preview_session_cleanup(
    app: tauri::State<'_, Arc<Workflow>>,
) -> Result<Batch, String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _scope = crate::remote_control::Scope::enter();
        app.preview_session_cleanup()
    })
    .await
    .map_err(err)?
}
#[tauri::command]
pub async fn execute_session_cleanup(
    app: tauri::State<'_, Arc<Workflow>>,
    native: tauri::State<'_, Arc<crate::native_terminal::NativeTerminalManager>>,
    batch_id: String,
) -> Result<Batch, String> {
    let app = app.inner().clone();
    let native = native.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = app.execute_session_cleanup(&batch_id)?;
        release_views(&native, &result);
        Ok(result)
    })
    .await
    .map_err(err)?
}
#[tauri::command]
pub async fn session_cleanup_result(
    app: tauri::State<'_, Arc<Workflow>>,
    native: tauri::State<'_, Arc<crate::native_terminal::NativeTerminalManager>>,
    batch_id: String,
) -> Result<Batch, String> {
    let app = app.inner().clone();
    let native = native.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = app.cleanup_result(&batch_id, true)?;
        release_views(&native, &result);
        Ok(result)
    })
    .await
    .map_err(err)?
}
#[tauri::command]
pub fn session_cleanup_history(app: tauri::State<'_, Arc<Workflow>>) -> Result<Vec<Batch>, String> {
    app.cleanup_history()
}
#[tauri::command]
pub fn session_resource_list(
    state: tauri::State<'_, crate::commands::DbState>,
) -> Result<Vec<crate::session_resources::Resource>, String> {
    crate::session_resources::list(&*state.0.lock().map_err(err)?)
}

#[cfg(test)]
#[path = "session_cleanup_tests.rs"]
mod tests;

#[tauri::command]
pub async fn retry_session_cleanup(
    app: tauri::State<'_, Arc<Workflow>>,
    native: tauri::State<'_, Arc<crate::native_terminal::NativeTerminalManager>>,
    batch_id: String,
) -> Result<Batch, String> {
    let app = app.inner().clone();
    let native = native.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = app.retry_session_cleanup(&batch_id)?;
        release_views(&native, &result);
        Ok(result)
    })
    .await
    .map_err(err)?
}
fn release_views(native: &crate::native_terminal::NativeTerminalManager, batch: &Batch) {
    native.close_released(
        &batch
            .items
            .iter()
            .filter(|i| matches!(i.state.as_str(), "closed" | "absent"))
            .map(|i| i.target.clone())
            .collect::<Vec<_>>(),
    );
}

#[tauri::command]
pub async fn preview_session_resource(
    app: tauri::State<'_, Arc<Workflow>>,
    resource_id: i64,
) -> Result<Batch, String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || app.preview_session_resource(resource_id))
        .await
        .map_err(err)?
}
