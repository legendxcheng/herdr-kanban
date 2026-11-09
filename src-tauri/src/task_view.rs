//! Viewing a known terminal does not attest that its prompt was received.
use crate::workflow::{Target, Workflow};
use rusqlite::{Connection, params};

#[derive(Clone, Debug)]
pub struct Authorization {
    pub target: Target,
    pub read_only: bool,
    epoch: i64,
    operation: Option<String>,
    session_id: Option<String>,
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub(crate) fn observe_identity(
    herdr: &(impl crate::workflow::Herdr + ?Sized),
    target: &Target,
) -> Result<Option<String>, String> {
    herdr.validate(&target.workspace_id, target)?;
    let session = if target.agent == "codex" {
        herdr.session_id(target)?
    } else {
        None
    };
    herdr.validate(&target.workspace_id, target)?;
    Ok(session)
}

fn local(db: &Connection, task: &str) -> Result<Authorization, String> {
    let epoch = crate::task_reset::epoch(db, task)?;
    crate::idle_cleanup::ensure_task_unclaimed(db, task)?;
    let binding = crate::workflow::binding(db, task)?;
    let flow = crate::lifecycle::view(db, task)?;
    if flow.state.as_deref() == Some("pending") || binding.state == "pending" {
        return Err("flow.busy".into());
    }
    if flow.state.as_deref() == Some("unknown") && flow.kind.as_deref() != Some("start") {
        return Err("flow.retry_cleanup".into());
    }
    let target = binding.target.ok_or("binding.repair_required")?;
    crate::machines::ensure_enabled(db, &target.machine_id)?;
    crate::workflow::ensure_project_target(db, task, &target, &target.session)?;
    crate::workflow::ensure_available(db, &target, task)?;
    let unknown_start = flow.state.as_deref() == Some("unknown");
    let read_only = binding.state == "unknown" || unknown_start;
    let operation = if read_only {
        if target.agent != "codex"
            || [
                &target.machine_id,
                &target.session,
                &target.workspace_id,
                &target.pane_id,
                &target.terminal_id,
                &target.process_id,
            ]
            .iter()
            .any(|s| s.trim().is_empty())
        {
            return Err("binding.identity_changed".into());
        }
        if unknown_start {
            let id = flow
                .operation_id
                .as_ref()
                .ok_or("dispatch.operation_conflict")?;
            let json: Option<String> = db.query_row(
                "SELECT target_json FROM lifecycle_operations WHERE id=?1 AND task_id=?2 AND kind='start' AND state='unknown'",
                params![id, task], |r| r.get(0),
            ).map_err(err)?;
            let original: Target =
                serde_json::from_str(&json.ok_or("binding.identity_changed")?).map_err(err)?;
            if !original.same_identity(&target)
                || binding
                    .operation
                    .as_ref()
                    .is_some_and(|op| op.id != format!("{id}:dispatch"))
            {
                return Err("binding.identity_changed".into());
            }
        }
        if let Some(op) = binding.operation.as_ref() {
            if op.state != "unknown" || !op.target.same_identity(&target) {
                return Err("binding.identity_changed".into());
            }
            Some(op.id.clone())
        } else {
            flow.operation_id.clone()
        }
    } else {
        crate::lifecycle::ensure_idle(db, task, None)?;
        let developing: bool = db
            .query_row(
                "SELECT status='in_progress' FROM tasks WHERE id=?1",
                [task],
                |r| r.get(0),
            )
            .map_err(err)?;
        if binding.state != "valid" || !developing {
            return Err("binding.not_developing".into());
        }
        None
    };
    if read_only && operation.is_none() {
        return Err("dispatch.operation_conflict".into());
    }
    Ok(Authorization {
        target,
        read_only,
        epoch,
        operation,
        session_id: flow.session_id,
    })
}

impl Workflow {
    /// Local checks run on every native input frame, so a reset/deletion revokes the lease.
    pub fn check_view(&self, task: &str, expected: &Authorization) -> Result<(), String> {
        let current = local(&*self.db.lock().map_err(err)?, task)?;
        if current.epoch != expected.epoch
            || current.operation != expected.operation
            || current.read_only != expected.read_only
            || !current.target.same_identity(&expected.target)
            || current
                .session_id
                .as_ref()
                .is_some_and(|id| expected.session_id.as_ref() != Some(id))
        {
            return Err("terminal.stale".into());
        }
        Ok(())
    }
    pub fn view_task(&self, task: &str) -> Result<Authorization, String> {
        let mut authorization = local(&*self.db.lock().map_err(err)?, task)?;
        if self.herdr.machine_id() != authorization.target.machine_id
            || self.herdr.session() != authorization.target.session
        {
            return Err("machine.identity_changed".into());
        }
        let session = self.herdr.view_identity(&authorization.target)?;
        if authorization
            .session_id
            .as_ref()
            .is_some_and(|expected| session.as_ref() != Some(expected))
        {
            return Err("flow.session_mismatch".into());
        }
        if authorization.read_only {
            uuid::Uuid::parse_str(session.as_deref().ok_or("flow.session_missing")?)
                .map_err(|_| "flow.session_invalid")?;
        }
        authorization.session_id = session;
        self.check_view(task, &authorization)?;
        Ok(authorization)
    }
    pub fn validate_view(&self, task: &str, expected: &Authorization) -> Result<(), String> {
        self.check_view(task, expected)?;
        let session = self.herdr.view_identity(&expected.target)?;
        if expected.session_id.is_some() && session != expected.session_id {
            return Err("flow.session_mismatch".into());
        }
        self.check_view(task, expected)
    }
}
