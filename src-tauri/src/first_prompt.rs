//! A proven pre-input failure can continue only through an explicit task-owned attempt.
use crate::workflow::Target;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

type ClosedDatabases = HashMap<usize, Weak<Mutex<Connection>>>;
fn closed() -> &'static Mutex<ClosedDatabases> {
    static CLOSED: OnceLock<Mutex<ClosedDatabases>> = OnceLock::new();
    CLOSED.get_or_init(Default::default)
}

pub(crate) struct Unsent {
    pub target: Target,
    pub prompt: String,
}

/// Positive durable evidence, never absence of logs or receipts. Legacy not_idle
/// was emitted before text; manual decisions and post-text errors are ineligible.
pub(crate) fn unsent(db: &Connection, task: &str, op: &str) -> Result<Option<Unsent>, String> {
    let row: Option<(String,String,String)> = db.query_row(
        "SELECT d.target_json,d.prompt,b.target_json FROM lifecycle_operations l
         JOIN tasks t ON t.id=l.task_id JOIN dispatch_operations d ON d.id=l.id||':dispatch' AND d.task_id=t.id
         JOIN task_bindings b ON b.task_id=t.id
         WHERE l.id=?1 AND t.id=?2 AND l.kind='start' AND l.state='unknown'
         AND t.archived_at IS NULL AND t.dispatched_at IS NULL AND b.state='valid'
         AND NOT EXISTS(SELECT 1 FROM startup_prompt_operations s WHERE s.operation_id=l.id)
         AND l.rowid=(SELECT MAX(rowid) FROM lifecycle_operations WHERE task_id=t.id)
         AND d.state='not_sent' AND (d.evidence='flow.not_idle' OR d.evidence LIKE 'first_prompt.not_written:%')
         AND d.evidence NOT IN ('first_prompt.not_written:flow.first_prompt_activity','first_prompt.not_written:flow.session_mismatch')
         AND NOT EXISTS(SELECT 1 FROM dispatch_operations other WHERE other.task_id=t.id AND other.state IN ('pending','unknown','sent'))
         AND NOT EXISTS(SELECT 1 FROM dispatch_operations newer WHERE newer.task_id=t.id AND newer.rowid>d.rowid)",
        params![op,task], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|e| e.to_string())?;
    let Some((json, prompt, binding)) = row else {
        return Ok(None);
    };
    let target: Target = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let binding: Target = serde_json::from_str(&binding).map_err(|e| e.to_string())?;
    if target.agent != "codex"
        || !target.same_identity(&binding)
        || prompt.trim().is_empty()
        || [
            &target.machine_id,
            &target.session,
            &target.workspace_id,
            &target.pane_id,
            &target.terminal_id,
            &target.process_id,
        ]
        .iter()
        .any(|s| s.is_empty())
    {
        return Ok(None);
    }
    Ok(Some(Unsent { target, prompt }))
}

pub(crate) fn check_pending(
    db: &Connection,
    task: &str,
    op: &str,
    epoch: i64,
) -> Result<(), String> {
    crate::task_reset::check_epoch(db, task, epoch)?;
    let pending: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM lifecycle_operations WHERE id=?1 AND task_id=?2 AND kind='start' AND state='pending')", params![op,task], |r| r.get(0)).map_err(|e| e.to_string())?;
    if pending {
        Ok(())
    } else {
        Err("dispatch.operation_conflict".into())
    }
}

impl crate::workflow::Workflow {
    /// Revoke active startup writers when the owning desktop closes. Reopening
    /// may reconcile uncertain writes, but never resumes an old write automatically.
    pub fn stop_starts(&self) {
        if let Ok(mut entries) = closed().lock() {
            entries.retain(|_, db| db.strong_count() > 0);
            entries.insert(Arc::as_ptr(&self.db) as usize, Arc::downgrade(&self.db));
        }
        if let Ok(db) = self.db.lock() {
            let _ = db.execute("UPDATE lifecycle_operations SET state='unknown',error='flow.interrupted' WHERE kind='start' AND state='pending'",[]);
        }
    }
    pub(crate) fn check_running(&self) -> Result<(), String> {
        let entries = closed().lock().map_err(|e| e.to_string())?;
        if entries
            .get(&(Arc::as_ptr(&self.db) as usize))
            .is_some_and(|db| db.strong_count() > 0)
        {
            Err("flow.interrupted".into())
        } else {
            Ok(())
        }
    }
    #[cfg(test)]
    pub fn continue_first_prompt(
        &self,
        task: &str,
        previous: &str,
        epoch: i64,
    ) -> Result<crate::lifecycle::Flow, String> {
        self.continue_first_prompt_notifying(task, previous, epoch, &|_| {})
    }
    pub(crate) fn continue_first_prompt_notifying(
        &self,
        task: &str,
        previous: &str,
        epoch: i64,
        notify: &dyn Fn(crate::start::StartChanged),
    ) -> Result<crate::lifecycle::Flow, String> {
        let gate = crate::task_recovery::gate(self, task)?;
        let _guard = gate.lock().map_err(|e| e.to_string())?;
        self.check_running()?;
        let pending = {
            let db = self.db.lock().map_err(|e| e.to_string())?;
            crate::task_reset::check_epoch(&db, task, epoch)?;
            unsent(&db, task, previous)?.ok_or("flow.first_prompt_ineligible")?
        };
        self.herdr
            .validate(&pending.target.workspace_id, &pending.target)?;
        let expected =
            crate::lifecycle::view(&*self.db.lock().map_err(|e| e.to_string())?, task)?.session_id;
        let current = self.herdr.session_id(&pending.target)?;
        if expected
            .as_ref()
            .is_some_and(|id| current.as_ref() != Some(id))
        {
            return Err("flow.session_mismatch".into());
        }
        // A session discovered during this preflight is already known identity;
        // do not let a later observation silently replace it before the first input.
        let expected = expected.or(current);
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let (operation, cwd) = {
            let mut db = self.db.lock().map_err(|e| e.to_string())?;
            let tx = db.transaction().map_err(|e| e.to_string())?;
            self.check_running()?;
            crate::task_reset::check_epoch(&tx, task, epoch)?;
            crate::lifecycle::ensure_operation(&tx, task, previous)?;
            crate::machines::ensure_enabled(&tx, self.herdr.machine_id())?;
            crate::workflow::ensure_project_target(
                &tx,
                task,
                &pending.target,
                self.herdr.session(),
            )?;
            crate::workflow::ensure_available(&tx, &pending.target, task)?;
            let current = unsent(&tx, task, previous)?.ok_or("flow.first_prompt_ineligible")?;
            if !current.target.same_identity(&pending.target) || current.prompt != pending.prompt {
                return Err("dispatch.operation_conflict".into());
            }
            let json = serde_json::to_string(&pending.target).map_err(|e| e.to_string())?;
            let evidence = serde_json::json!({"source":"continue_first_prompt","previous_operation":previous,"epoch":epoch}).to_string();
            tx.execute(
                "UPDATE lifecycle_operations SET state='failed' WHERE id=?1 AND state='unknown'",
                [previous],
            )
            .map_err(|e| e.to_string())?;
            tx.execute("INSERT INTO lifecycle_operations(id,task_id,kind,state,target_json,created_at,evidence,machine_id) VALUES(?1,?2,'start','pending',?3,?4,?5,?6)", params![id,task,json,now,evidence,pending.target.machine_id]).map_err(|e| e.to_string())?;
            tx.execute("INSERT INTO start_codex_settings(operation_id,model,effort) SELECT ?1,model,effort FROM start_codex_settings WHERE operation_id=?2", params![id,previous]).map_err(|e| e.to_string())?;
            tx.execute("INSERT INTO operation_image_refs(operation_id,image_id) SELECT DISTINCT ?1,image_id FROM operation_image_refs WHERE operation_id IN (?2,?3)", params![id,previous,format!("{previous}:dispatch")]).map_err(|e| e.to_string())?;
            let cwd: String = tx.query_row("SELECT COALESCE(d.cwd,p.root_path) FROM tasks t JOIN projects p ON p.id=t.project_id LEFT JOIN task_working_directories d ON d.task_id=t.id WHERE t.id=?1",[task],|r|r.get(0)).map_err(|e| e.to_string())?;
            if let Some(session) = &expected {
                tx.execute("INSERT INTO task_sessions(task_id,codex_session_id,cwd,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(task_id) DO UPDATE SET codex_session_id=excluded.codex_session_id,cwd=excluded.cwd,updated_at=excluded.updated_at",params![task,session,cwd,now]).map_err(|e| e.to_string())?;
            }
            tx.execute("INSERT INTO start_requests(operation_id,project_id,workspace,cwd,prompt_json,needs_prompt,original_status,stage) SELECT ?1,project_id,?2,?3,prompt_json,1,status,'waiting_first_prompt' FROM tasks WHERE id=?4",params![id,pending.target.workspace_id,cwd,task]).map_err(|e| e.to_string())?;
            let operation = crate::workflow::Operation {
                id: format!("{id}:dispatch"),
                task_id: task.into(),
                workspace: pending.target.workspace_id.clone(),
                target: pending.target.clone(),
                prompt: pending.prompt,
                created_at: now,
                state: "pending".into(),
                had_binding: true,
            };
            tx.execute("INSERT INTO dispatch_operations(id,task_id,workspace,target_json,prompt,created_at,state,had_binding,machine_id) VALUES(?1,?2,?3,?4,?5,?6,'pending',1,?7)",params![operation.id,task,operation.workspace,json,operation.prompt,operation.created_at,operation.target.machine_id]).map_err(|e| e.to_string())?;
            tx.execute(
                "UPDATE task_bindings SET state='reserved' WHERE task_id=?1",
                [task],
            )
            .map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            (operation, cwd)
        };
        let project_id = self
            .db
            .lock()
            .map_err(|e| e.to_string())?
            .query_row("SELECT project_id FROM tasks WHERE id=?1", [task], |r| {
                r.get::<_, String>(0)
            })
            .map_err(|e| e.to_string())?;
        if self.check_running().is_ok() {
            notify(crate::start::StartChanged {
                project_id,
                task_id: task.into(),
                operation_id: id.clone(),
            });
        }
        let _control = crate::remote_control::Scope::enter_if_absent();
        let result = self
            .send_reserved(&operation, Some(&id), epoch)
            .and_then(|_| {
                self.save_session(task, &id, &operation.target, &cwd, expected.as_deref())?;
                self.activate(task, &id, &operation.target)
            });
        if let Err(error) = result {
            // A retired or cancelled execution must not be resurrected by this result.
            let db = self.db.lock().map_err(|e| e.to_string())?;
            if crate::task_reset::check_epoch(&db, task, epoch).is_ok() {
                db.execute("UPDATE lifecycle_operations SET state='unknown',error=?2 WHERE id=?1 AND task_id=?3 AND state='pending'",params![id,error,task]).map_err(|e| e.to_string())?;
            }
            return Err(error);
        }
        crate::lifecycle::view(&*self.db.lock().map_err(|e| e.to_string())?, task)
    }
}
