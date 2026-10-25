//! Task/pane ownership and dispatch, shared by the Tauri command boundary.
use crate::models::DispatchRecord;
use rusqlite::{Connection, OptionalExtension, params};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Target {
    pub session: String,
    pub workspace_id: String,
    pub pane_id: String,
    pub terminal_id: String,
    pub agent: String,
    pub process_id: String,
    /// Observed only after this app starts a fresh, exact Codex resume process.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub codex_resume_locks: Vec<String>,
    pub title: String,
    pub status: String,
}

pub struct SendError {
    pub message: String,
    pub uncertain: bool,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PaneIdentity {
    pub pane_id: String,
    pub terminal_id: String,
}
pub(crate) fn ensure_project_target(
    db: &Connection,
    task: &str,
    target: &Target,
    session: &str,
) -> Result<(), String> {
    if target.session != session
        || crate::projects::task_identity(db, task, session)? != target.workspace_id
    {
        return Err("binding.identity_changed".into());
    }
    Ok(())
}

pub trait Herdr: Send + Sync {
    fn session(&self) -> &str {
        "default"
    }
    fn workspaces(&self) -> Result<Vec<crate::projects::Workspace>, String> {
        Err("workspace.unsupported".into())
    }
    fn create_workspace(
        &self,
        _name: &str,
        _cwd: &str,
    ) -> Result<crate::projects::Workspace, SendError> {
        Err(SendError {
            message: "workspace.unsupported".into(),
            uncertain: false,
        })
    }
    fn rename_workspace(&self, _id: &str, _name: &str) -> Result<(), String> {
        Err("workspace.unsupported".into())
    }
    fn move_workspace(
        &self,
        _id: &str,
        _before: Option<&str>,
    ) -> Result<Vec<crate::projects::Workspace>, String> {
        Err("workspace.unsupported".into())
    }
    fn available(&self, _workspace: &str, _cwd: &str) -> Result<Vec<Target>, String> {
        Err("flow.unsupported".into())
    }
    /// Live inventory for durable creation reconciliation. None keeps legacy adapters compatible.
    fn pane_inventory(&self, _workspace: &str) -> Result<Option<Vec<PaneIdentity>>, String> {
        Ok(None)
    }
    fn create_pane(
        &self,
        _workspace: &str,
        _cwd: &str,
        _label: &str,
        _check_current: &dyn Fn() -> Result<(), String>,
    ) -> Result<Target, String> {
        Err("flow.unsupported".into())
    }
    fn prepare_codex(
        &self,
        _target: &Target,
        _cwd: &str,
        _resume: Option<&str>,
        _name: &str,
        _check_current: &dyn Fn() -> Result<(), String>,
    ) -> Result<Target, String> {
        Err("flow.unsupported".into())
    }
    fn session_id(&self, _target: &Target) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn pane_exists(&self, _target: &Target) -> Result<bool, String> {
        Err("flow.unsupported".into())
    }
    fn close_pane(
        &self,
        _target: &Target,
        _check_current: &dyn Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        Err("flow.unsupported".into())
    }
    fn find_created(&self, _workspace: &str, _label: &str) -> Result<Option<Target>, String> {
        Err("flow.unsupported".into())
    }
    fn targets(&self, workspace: &str) -> Result<Vec<Target>, String>;
    fn validate(&self, workspace: &str, expected: &Target) -> Result<Target, String> {
        self.targets(workspace)?
            .into_iter()
            .find(|t| t.same_identity(expected))
            .ok_or_else(|| "binding.identity_changed".into())
    }
    fn send(&self, target: &Target, prompt: &str) -> Result<(), SendError>;
    /// Multi-command adapters also check between external side effects.
    fn send_checked(
        &self,
        target: &Target,
        prompt: &str,
        check_current: &dyn Fn() -> Result<(), String>,
    ) -> Result<(), SendError> {
        check_current().map_err(|message| SendError {
            message,
            uncertain: false,
        })?;
        self.send(target, prompt)
    }
}

pub struct Workflow {
    pub db: Arc<Mutex<Connection>>,
    pub herdr: Arc<dyn Herdr>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Operation {
    pub id: String,
    pub task_id: String,
    pub workspace: String,
    pub target: Target,
    pub prompt: String,
    pub created_at: String,
    pub state: String,
    pub had_binding: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Binding {
    pub state: String,
    pub target: Option<Target>,
    pub operation: Option<Operation>,
}

impl Target {
    pub fn same_identity(&self, other: &Self) -> bool {
        self.session == other.session
            && self.workspace_id == other.workspace_id
            && self.pane_id == other.pane_id
            && self.terminal_id == other.terminal_id
            && self.agent == other.agent
            && self.process_id == other.process_id
    }
}

fn operation_row(r: &rusqlite::Row) -> rusqlite::Result<Operation> {
    let json: String = r.get(3)?;
    let target = serde_json::from_str(&json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(Operation {
        id: r.get(0)?,
        task_id: r.get(1)?,
        workspace: r.get(2)?,
        target,
        prompt: r.get(4)?,
        created_at: r.get(5)?,
        state: r.get(6)?,
        had_binding: r.get(7)?,
    })
}
const OP_SELECT: &str = "SELECT id,task_id,workspace,target_json,prompt,created_at,state,had_binding FROM dispatch_operations";

pub fn binding(conn: &Connection, task: &str) -> Result<Binding, String> {
    let operation = conn
        .query_row(
            &format!("{OP_SELECT} WHERE task_id=?1 AND state IN ('pending','unknown')"),
            [task],
            operation_row,
        )
        .optional()
        .map_err(err)?;
    let stored: Option<(String, String)> = conn
        .query_row(
            "SELECT target_json,state FROM task_bindings WHERE task_id=?1",
            [task],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(err)?;
    let target = stored
        .as_ref()
        .map(|(json, _)| serde_json::from_str(json))
        .transpose()
        .map_err(err)?;
    let state = if let Some(ref op) = operation {
        op.state.clone()
    } else if let Some((_, ref state)) = stored {
        state.clone()
    } else {
        let legacy: bool = conn
            .query_row(
                "SELECT status='in_progress' OR dispatched_to IS NOT NULL FROM tasks WHERE id=?1",
                [task],
                |r| r.get(0),
            )
            .map_err(err)?;
        let saved: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM task_sessions WHERE task_id=?1)",
                [task],
                |r| r.get(0),
            )
            .map_err(err)?;
        if saved {
            "dormant".into()
        } else if legacy {
            "repair".into()
        } else {
            "unbound".into()
        }
    };
    Ok(Binding {
        state,
        target,
        operation,
    })
}

pub fn pane_owner(conn: &Connection, target: &Target) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT task_id FROM task_bindings WHERE session=?1 AND pane_id=?2",
        params![target.session, target.pane_id],
        |r| r.get(0),
    )
    .optional()
    .map_err(err)
}
fn ensure_available(conn: &Connection, target: &Target, task: &str) -> Result<(), String> {
    if pane_owner(conn, target)?
        .as_deref()
        .is_some_and(|owner| owner != task)
    {
        Err("binding.occupied".into())
    } else {
        Ok(())
    }
}

impl Workflow {
    pub fn repair(&self, task: &str, pane: &str) -> Result<(), String> {
        let workspace = self.workspace(task)?;
        let target = self
            .herdr
            .targets(&workspace)?
            .into_iter()
            .find(|t| t.pane_id == pane)
            .ok_or("binding.missing")?;
        let mut conn = self.db.lock().map_err(err)?;
        let tx = conn.transaction().map_err(err)?;
        crate::lifecycle::ensure_idle(&tx, task, None)?;
        ensure_project_target(&tx, task, &target, self.herdr.session())?;
        if binding(&tx, task)?.operation.is_some() {
            return Err("dispatch.verify_required".into());
        }
        ensure_available(&tx, &target, task)?;
        tx.execute("INSERT INTO task_bindings(task_id,session,pane_id,target_json,state) VALUES (?1,?2,?3,?4,'valid') ON CONFLICT(task_id) DO UPDATE SET session=excluded.session,pane_id=excluded.pane_id,target_json=excluded.target_json,state='valid'",
            params![task,target.session,pane,serde_json::to_string(&target).map_err(err)?]).map_err(err)?;
        tx.commit().map_err(err)
    }
    pub(crate) fn workspace(&self, task: &str) -> Result<String, String> {
        let db = self.db.lock().map_err(err)?;
        crate::tasks::ensure_active(&db, task)?;
        crate::projects::task_identity(&db, task, self.herdr.session())
    }
    pub fn binding(&self, task: &str) -> Result<Binding, String> {
        binding(&*self.db.lock().map_err(err)?, task)
    }
    pub fn target(&self, task: &str, require_developing: bool) -> Result<Target, String> {
        crate::lifecycle::ensure_idle(&*self.db.lock().map_err(err)?, task, None)?;
        let b = self.binding(task)?;
        if b.state != "valid" {
            return Err("binding.repair_required".into());
        }
        if require_developing {
            let status: String = self
                .db
                .lock()
                .map_err(err)?
                .query_row("SELECT status FROM tasks WHERE id=?1", [task], |r| r.get(0))
                .map_err(err)?;
            if status != "in_progress" {
                return Err("binding.not_developing".into());
            }
        }
        let target = b.target.ok_or("binding.repair_required")?;
        self.herdr.validate(&self.workspace(task)?, &target)
    }
    pub fn dispatch(
        &self,
        task: &str,
        pane: &str,
        prompt: &str,
        operation: &str,
    ) -> Result<DispatchRecord, String> {
        self.dispatch_inner(task, pane, prompt, operation, None)
    }
    pub(crate) fn dispatch_inner(
        &self,
        task: &str,
        pane: &str,
        prompt: &str,
        operation: &str,
        lifecycle: Option<&str>,
    ) -> Result<DispatchRecord, String> {
        if prompt.trim().is_empty() {
            return Err("dispatch.empty".into());
        }
        if operation.is_empty() {
            return Err("dispatch.operation_required".into());
        }
        let workspace = self.workspace(task)?;
        // A durable operation ID makes retries safe even after an app restart.
        if let Some(op) = self
            .db
            .lock()
            .map_err(err)?
            .query_row(
                &format!("{OP_SELECT} WHERE id=?1"),
                [operation],
                operation_row,
            )
            .optional()
            .map_err(err)?
        {
            if op.task_id != task || op.target.pane_id != pane || op.prompt != prompt {
                return Err("dispatch.operation_conflict".into());
            }
            return if op.state == "sent" {
                Ok(record(&op))
            } else {
                Err("dispatch.verify_required".into())
            };
        }
        let target = self
            .herdr
            .targets(&workspace)?
            .into_iter()
            .find(|t| t.pane_id == pane)
            .ok_or("binding.missing")?;
        let op;
        {
            let mut conn = self.db.lock().map_err(err)?;
            let tx = conn.transaction().map_err(err)?;
            crate::lifecycle::ensure_idle(&tx, task, lifecycle)?;
            ensure_project_target(&tx, task, &target, self.herdr.session())?;
            let b = binding(&tx, task)?;
            if b.operation.is_some() {
                return Err("dispatch.verify_required".into());
            }
            if b.state == "repair" {
                return Err("binding.repair_required".into());
            }
            if b.target
                .as_ref()
                .is_some_and(|old| !old.same_identity(&target))
            {
                return Err("binding.repair_required".into());
            }
            ensure_available(&tx, &target, task)?;
            op = Operation {
                id: operation.into(),
                task_id: task.into(),
                workspace,
                target: target.clone(),
                prompt: prompt.into(),
                created_at: chrono::Utc::now().to_rfc3339(),
                state: "pending".into(),
                had_binding: b.target.is_some(),
            };
            let json = serde_json::to_string(&target).map_err(err)?;
            tx.execute("INSERT OR IGNORE INTO task_working_directories(task_id,cwd) SELECT t.id,p.root_path FROM tasks t JOIN projects p ON p.id=t.project_id WHERE t.id=?1",[task]).map_err(err)?;
            tx.execute("INSERT INTO task_bindings(task_id,session,pane_id,target_json,state) VALUES (?1,?2,?3,?4,'reserved') ON CONFLICT(task_id) DO UPDATE SET state='reserved'",
                params![task,target.session,pane,json]).map_err(|_|"binding.occupied".to_string())?;
            tx.execute("INSERT INTO dispatch_operations(id,task_id,workspace,target_json,prompt,created_at,state,had_binding) VALUES (?1,?2,?3,?4,?5,?6,'pending',?7)",
                params![op.id,task,op.workspace,json,prompt,op.created_at,op.had_binding]).map_err(|_|"dispatch.verify_required".to_string())?;
            tx.commit().map_err(err)?;
        }
        crate::tasks::ensure_active(&*self.db.lock().map_err(err)?, task)?;
        match self.herdr.send_checked(&target, prompt, &|| {
            let db = self.db.lock().map_err(err)?;
            crate::tasks::ensure_active(&db, task)?;
            crate::images::verify_operation(&db, operation)
        }) {
            Ok(()) => match self.finish(&op, true, "") {
                Ok(()) => Ok(record(&op)),
                Err(_) => {
                    self.mark_unknown(&op.id);
                    Err("dispatch.verify_required".into())
                }
            },
            Err(error) if error.uncertain => {
                self.mark_unknown(&op.id);
                Err(if error.message == "dispatch.submit_verify_required" {
                    error.message
                } else {
                    format!("dispatch.verify_required: {}", error.message)
                })
            }
            Err(error) => {
                self.finish(&op, false, &error.message)?;
                Err(error.message)
            }
        }
    }
    fn mark_unknown(&self, id: &str) {
        if let Ok(conn) = self.db.lock() {
            let _ = conn.execute(
                "UPDATE dispatch_operations SET state='unknown' WHERE id=?1 AND state='pending' AND EXISTS(SELECT 1 FROM tasks WHERE id=dispatch_operations.task_id)",
                [id],
            );
        }
    }
    pub fn resolve(
        &self,
        task: &str,
        operation: &str,
        sent: bool,
        evidence: &str,
    ) -> Result<(), String> {
        if evidence.trim().len() < 3 {
            return Err("dispatch.evidence_required".into());
        }
        let op = self
            .binding(task)?
            .operation
            .ok_or("dispatch.operation_conflict")?;
        if op.id != operation || op.state != "unknown" {
            return Err("dispatch.operation_conflict".into());
        }
        // This records an explicit user's observation, never an inferred send result.
        self.finish(&op, sent, evidence.trim())
    }
    fn finish(&self, op: &Operation, sent: bool, evidence: &str) -> Result<(), String> {
        let mut conn = self.db.lock().map_err(err)?;
        let tx = conn.transaction().map_err(err)?;
        crate::tasks::ensure_active(&tx, &op.task_id)?;
        let changed = tx.execute("UPDATE dispatch_operations SET state=?1,evidence=?2 WHERE id=?3 AND state IN ('pending','unknown')",
            params![if sent {"sent"} else {"not_sent"},evidence,op.id]).map_err(err)?;
        if changed != 1 {
            return Err("dispatch.operation_conflict".into());
        }
        if sent {
            if tx.execute("UPDATE tasks SET status='in_progress',completed_at=NULL,prompt_snapshot=?1,dispatched_at=?2,dispatched_to=?3,updated_at=?2 WHERE id=?4",
                params![op.prompt,op.created_at,format!("{}/{}",op.workspace,op.target.pane_id),op.task_id]).map_err(err)? != 1 { return Err("binding.missing".into()); }
            tx.execute(
                "UPDATE task_bindings SET state='valid' WHERE task_id=?1",
                [&op.task_id],
            )
            .map_err(err)?;
        } else if op.had_binding {
            tx.execute(
                "UPDATE task_bindings SET state='valid' WHERE task_id=?1",
                [&op.task_id],
            )
            .map_err(err)?;
        } else {
            tx.execute("DELETE FROM task_bindings WHERE task_id=?1", [&op.task_id])
                .map_err(err)?;
        }
        tx.commit().map_err(err)
    }
}
fn record(op: &Operation) -> DispatchRecord {
    DispatchRecord {
        task_id: op.task_id.clone(),
        target_workspace: op.workspace.clone(),
        target_agent: op.target.pane_id.clone(),
        prompt_snapshot: op.prompt.clone(),
        dispatched_at: op.created_at.clone(),
    }
}

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
#[path = "workflow_tests/mod.rs"]
mod tests;
