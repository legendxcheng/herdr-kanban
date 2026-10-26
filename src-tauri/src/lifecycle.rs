//! Durable task lifecycle. External calls never hold the SQLite mutex.
use crate::workflow::{Target, Workflow, binding, pane_owner};
use rusqlite::{Connection, OptionalExtension, params};
#[path = "pane_allocation.rs"]
mod pane_allocation;

#[cfg(all(test, windows))]
#[path = "delete_cli_tests.rs"]
mod delete_cli_tests;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Flow {
    pub session_id: Option<String>,
    pub operation_id: Option<String>,
    pub kind: Option<String>,
    pub state: Option<String>,
    pub error: String,
    pub pane_id: Option<String>,
    pub stage: Option<String>,
    pub requested_status: Option<String>,
}

#[cfg(test)]
#[path = "lifecycle_tests/mod.rs"]
mod tests;

#[derive(Default, Debug, serde::Serialize)]
pub struct Outcome {
    pub warning: Option<String>,
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub fn ensure_idle(db: &Connection, task: &str, own: Option<&str>) -> Result<(), String> {
    crate::tasks::ensure_active(db, task)?;
    let active: Option<String> = db.query_row(
        "SELECT id FROM lifecycle_operations WHERE task_id=?1 AND state IN ('pending','unknown')",
        [task], |r| r.get(0)).optional().map_err(err)?;
    if active.as_deref().is_some_and(|id| Some(id) != own) {
        return Err("flow.verify_required".into());
    }
    Ok(())
}

fn ensure_operation(db: &Connection, task: &str, id: &str) -> Result<(), String> {
    ensure_idle(db, task, Some(id))?;
    let active: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM lifecycle_operations WHERE id=?1 AND task_id=?2 AND state IN ('pending','unknown'))",params![id,task],|r|r.get(0)).map_err(err)?;
    if !active {
        return Err("dispatch.operation_conflict".into());
    }
    Ok(())
}
pub fn view(db: &Connection, task: &str) -> Result<Flow, String> {
    let session_id = db
        .query_row(
            "SELECT codex_session_id FROM task_sessions WHERE task_id=?1",
            [task],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?;
    let mut flow = Flow {
        session_id,
        operation_id: None,
        kind: None,
        state: None,
        error: String::new(),
        pane_id: None,
        stage: None,
        requested_status: None,
    };
    let op: Option<(String,String,String,String,Option<String>)> = db.query_row(
        "SELECT id,kind,state,error,target_json FROM lifecycle_operations WHERE task_id=?1 ORDER BY rowid DESC LIMIT 1",
        [task], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(err)?;
    if let Some((id, kind, state, error, json)) = op {
        flow.stage = db
            .query_row(
                "SELECT stage FROM start_requests WHERE operation_id=?1",
                [&id],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        if flow.stage.is_some() && state == "succeeded" {
            flow.stage = Some("ready".into());
        }
        if flow.stage.is_some() && matches!(state.as_str(), "pending" | "unknown") {
            flow.requested_status = Some("in_progress".into());
        }
        flow.operation_id = Some(id);
        flow.kind = Some(kind);
        flow.state = Some(state);
        flow.error = error;
        flow.pane_id = json
            .map(|s| serde_json::from_str::<Target>(&s))
            .transpose()
            .map_err(err)?
            .map(|t| t.pane_id);
    }
    Ok(flow)
}

pub(crate) fn begin_flow_in(
    db: &Connection,
    task: &str,
    id: &str,
    kind: &str,
) -> Result<bool, String> {
    if uuid::Uuid::parse_str(id).is_err() {
        return Err("dispatch.operation_required".into());
    }
    crate::tasks::ensure_active(db, task)?;
    let old: Option<(String, String, String)> = db
        .query_row(
            "SELECT task_id,kind,state FROM lifecycle_operations WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(err)?;
    if let Some((owner, old_kind, state)) = old {
        if owner != task || old_kind != kind {
            return Err("dispatch.operation_conflict".into());
        }
        return if state == "succeeded" {
            Ok(false)
        } else {
            Err("flow.verify_required".into())
        };
    }
    // A close interrupted by process death can be retried after rechecking the exact pane.
    if kind == "complete" {
        db.execute("UPDATE lifecycle_operations SET state='failed' WHERE task_id=?1 AND kind='complete' AND state='unknown'",[task]).map_err(err)?;
    }
    ensure_idle(db, task, None)?;
    if binding(db, task)?.operation.is_some() {
        return Err("dispatch.verify_required".into());
    }
    db.execute("INSERT INTO lifecycle_operations(id,task_id,kind,state,created_at) VALUES (?1,?2,?3,'pending',?4)",params![id,task,kind,now()]).map_err(err)?;
    if kind == "start" {
        db.execute("INSERT OR IGNORE INTO task_working_directories(task_id,cwd) SELECT t.id,COALESCE(s.cwd,p.root_path) FROM tasks t JOIN projects p ON p.id=t.project_id LEFT JOIN task_sessions s ON s.task_id=t.id WHERE t.id=?1",[task]).map_err(err)?;
    }
    Ok(true)
}

impl Workflow {
    fn check_flow(&self, task: &str, op: &str) -> Result<(), String> {
        let db = self.db.lock().map_err(err)?;
        crate::machines::ensure_enabled(&db, self.herdr.machine_id())?;
        ensure_operation(&db, task, op)
    }
    fn project_context(&self, task: &str) -> Result<(String, String, String), String> {
        let db = self.db.lock().map_err(err)?;
        let workspace = crate::projects::task_identity_on(
            &db,
            task,
            self.herdr.machine_id(),
            self.herdr.session(),
        )?;
        db.query_row(
            "SELECT COALESCE(s.cwd,d.cwd,p.root_path),t.status FROM tasks t JOIN projects p ON p.id=t.project_id LEFT JOIN task_sessions s ON s.task_id=t.id LEFT JOIN task_working_directories d ON d.task_id=t.id WHERE t.id=?1",
            [task], |r|Ok((workspace,r.get(0)?,r.get(1)?))).map_err(err)
    }
    fn begin_flow(&self, task: &str, id: &str, kind: &str) -> Result<bool, String> {
        let mut db = self.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        let started = begin_flow_in(&tx, task, id, kind)?;
        tx.commit().map_err(err)?;
        Ok(started)
    }
    pub(crate) fn end_flow(&self, id: &str, state: &str, error: &str) -> Result<(), String> {
        self.db
            .lock()
            .map_err(err)?
            .execute(
                "UPDATE lifecycle_operations SET state=?2,error=?3 WHERE id=?1 AND EXISTS(SELECT 1 FROM tasks WHERE id=lifecycle_operations.task_id)",
                params![id, state, error],
            )
            .map_err(err)?;
        Ok(())
    }
    /// A known pre-external failure must not freeze an unused working directory for a retry.
    pub(crate) fn fail_unstarted(&self, task: &str, op: &str, error: &str) -> Result<(), String> {
        let mut db = self.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        crate::tasks::ensure_active(&tx, task)?;
        let updated = tx.execute("UPDATE lifecycle_operations SET state='failed',error=?3 WHERE id=?1 AND task_id=?2 AND state='pending'",
            params![op, task, error]).map_err(err)?;
        if updated != 1 {
            return Err("dispatch.operation_conflict".into());
        }
        tx.execute("DELETE FROM task_working_directories WHERE task_id=?1 AND NOT EXISTS(SELECT 1 FROM task_bindings WHERE task_id=?1) AND NOT EXISTS(SELECT 1 FROM task_sessions WHERE task_id=?1)", [task]).map_err(err)?;
        tx.commit().map_err(err)
    }
    fn reserve_target(&self, task: &str, op: &str, target: &Target) -> Result<(), String> {
        let mut db = self.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        ensure_operation(&tx, task, op)?;
        crate::workflow::ensure_project_target(&tx, task, target, self.herdr.session())?;
        if pane_owner(&tx, target)?.is_some_and(|owner| owner != task) {
            return Err("binding.occupied".into());
        }
        let json = serde_json::to_string(target).map_err(err)?;
        tx.execute("INSERT INTO task_bindings(task_id,session,pane_id,target_json,state,machine_id) VALUES (?1,?2,?3,?4,'reserved',COALESCE(json_extract(?4,'$.machine_id'),'local')) ON CONFLICT(task_id) DO UPDATE SET machine_id=excluded.machine_id,session=excluded.session,pane_id=excluded.pane_id,target_json=excluded.target_json,state='reserved'", params![task,target.session,target.pane_id,json]).map_err(err)?;
        tx.execute(
            "UPDATE lifecycle_operations SET target_json=?2 WHERE id=?1",
            params![op, json],
        )
        .map_err(err)?;
        // The binding now owns this known pane; only unbound/unknown creation needs quarantine.
        tx.execute(
            "UPDATE pane_allocations SET resolved=1 WHERE operation_id=?1",
            [op],
        )
        .map_err(err)?;
        tx.commit().map_err(err)
    }
    fn save_session(
        &self,
        task: &str,
        target: &Target,
        cwd: &str,
        expected: Option<&str>,
    ) -> Result<Option<String>, String> {
        crate::tasks::ensure_active(&*self.db.lock().map_err(err)?, task)?;
        let id = self.herdr.session_id(target)?;
        if expected.is_some() && id.as_deref() != expected {
            return Err("flow.session_mismatch".into());
        }
        if let Some(ref id) = id {
            if uuid::Uuid::parse_str(id).is_err() {
                return Err("flow.session_invalid".into());
            }
            let db = self.db.lock().map_err(err)?;
            crate::tasks::ensure_active(&db, task)?;
            db.execute("INSERT INTO task_sessions(task_id,codex_session_id,cwd,updated_at) VALUES (?1,?2,?3,?4) ON CONFLICT(task_id) DO UPDATE SET codex_session_id=excluded.codex_session_id,cwd=excluded.cwd,updated_at=excluded.updated_at",params![task,id,cwd,now()]).map_err(err)?;
        }
        Ok(id)
    }
    fn activate(&self, task: &str, op: &str, target: &Target) -> Result<(), String> {
        let mut db = self.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        ensure_operation(&tx, task, op)?;
        tx.execute(
            "UPDATE task_bindings SET state='valid',target_json=?2 WHERE task_id=?1",
            params![task, serde_json::to_string(target).map_err(err)?],
        )
        .map_err(err)?;
        tx.execute(
            "UPDATE tasks SET status='in_progress',completed_at=NULL,updated_at=?2 WHERE id=?1",
            params![task, now()],
        )
        .map_err(err)?;
        tx.execute(
            "UPDATE lifecycle_operations SET state='succeeded',error='' WHERE id=?1",
            [op],
        )
        .map_err(err)?;
        tx.commit().map_err(err)
    }
    #[cfg(test)]
    pub fn start_task(&self, task: &str, prompt: &str, op: &str) -> Result<Outcome, String> {
        if !self.begin_flow(task, op, "start")? {
            return Ok(Outcome::default());
        }
        self.run_start(task, prompt, op, None, &|_| Ok(()))
    }
    pub(crate) fn run_start(
        &self,
        task: &str,
        prompt: &str,
        op: &str,
        context: Option<(String, String)>,
        progress: &dyn Fn(&str) -> Result<(), String>,
    ) -> Result<Outcome, String> {
        let mut touched = false;
        let result = (|| {
            self.check_flow(task, op)?;
            let (workspace, cwd) = match context {
                Some(context) => context,
                None => {
                    let (workspace, cwd, _) = self.project_context(task)?;
                    (workspace, cwd)
                }
            };
            progress("preparing_window")?;
            let b = self.binding(task)?;
            let saved = view(&*self.db.lock().map_err(err)?, task)?.session_id;
            // A retained task pane (e.g. completion without a session ID) still owns its conversation.
            if let Some(target) = b.target {
                if self.herdr.pane_exists(&target)? {
                    let live = self.herdr.validate(&workspace, &target)?;
                    if live.agent != "codex" {
                        return Err("flow.codex_required".into());
                    }
                    self.save_session(task, &live, &cwd, None)?;
                    self.activate(task, op, &live)?;
                    return Ok(Outcome::default());
                }
                if saved.is_none() {
                    return Err("flow.session_missing".into());
                }
                self.db
                    .lock()
                    .map_err(err)?
                    .execute("DELETE FROM task_bindings WHERE task_id=?1", [task])
                    .map_err(err)?;
            } else if b.state == "repair" && saved.is_none() {
                return Err("binding.repair_required".into());
            }
            if saved.is_none() && prompt.trim().is_empty() {
                return Err("dispatch.empty".into());
            }
            let allocation_lock = pane_allocation::workspace_lock(
                self.herdr.machine_id(),
                self.herdr.session(),
                &workspace,
            )?;
            let allocation_guard = allocation_lock.lock().map_err(err)?;
            self.check_flow(task, op)?;
            let mut selected = None;
            for candidate in self.herdr.available(&workspace, &cwd)? {
                if self.allocation_quarantines(&candidate)? {
                    continue;
                }
                match self.reserve_target(task, op, &candidate) {
                    Ok(()) => {
                        selected = Some(candidate);
                        break;
                    }
                    Err(e) if e == "binding.occupied" => continue,
                    Err(e) => return Err(e),
                }
            }
            let target = match selected {
                Some(target) => target,
                None => {
                    // Persist the inventory before creation; shared tabs cannot identify a task.
                    self.check_flow(task, op)?;
                    self.record_allocation_intent(task, op, &workspace)?;
                    touched = true;
                    let target = self.herdr.create_pane(
                        &workspace,
                        &cwd,
                        &format!("kanban-{op}"),
                        &|| self.check_flow(task, op),
                    )?;
                    self.record_allocation_target(task, op, &target)?;
                    self.reserve_target(task, op, &target)?;
                    target
                }
            };
            drop(allocation_guard);
            touched = true;
            progress("starting_agent")?;
            self.check_flow(task, op)?;
            let live = self.herdr.prepare_codex(
                &target,
                &cwd,
                saved.as_deref(),
                &format!("kb-{}", &op.replace('-', "")[..24]),
                &|| self.check_flow(task, op),
            )?;
            self.reserve_target(task, op, &live)?;
            self.save_session(task, &live, &cwd, saved.as_deref())?;
            self.db
                .lock()
                .map_err(err)?
                .execute(
                    "UPDATE task_bindings SET state='valid' WHERE task_id=?1",
                    [task],
                )
                .map_err(err)?;
            if saved.is_none() {
                progress("dispatching")?;
                self.dispatch_inner(
                    task,
                    &live.pane_id,
                    prompt,
                    &format!("{op}:dispatch"),
                    Some(op),
                )?;
                // Some integrations only report an ID after the first prompt.
                let _ = self.save_session(task, &live, &cwd, None);
            }
            self.activate(task, op, &live)?;
            Ok(Outcome::default())
        })();
        if let Err(ref e) = result {
            if touched {
                self.end_flow(op, "unknown", e)?;
            } else {
                self.fail_unstarted(task, op, e)?;
            }
        }
        result
    }
    pub fn complete_task(&self, task: &str, op: &str) -> Result<Outcome, String> {
        let (workspace, cwd, _) = self.project_context(task)?;
        if !self.begin_flow(task, op, "complete")? {
            return Ok(Outcome::default());
        }
        // Completion is a business decision, independent from external cleanup success.
        self.db
            .lock()
            .map_err(err)?
            .execute(
                "UPDATE tasks SET status='done',completed_at=CASE WHEN status='done' THEN completed_at ELSE ?2 END,updated_at=?2 WHERE id=?1",
                params![task, now()],
            )
            .map_err(err)?;
        let mut close_attempted = false;
        let result = (|| {
            if let Some(target) = self.binding(task)?.target {
                self.reserve_target(task, op, &target)?;
                if self.herdr.pane_exists(&target)? {
                    let live = self.herdr.validate(&workspace, &target)?;
                    if live.agent != "codex" {
                        return Err("flow.codex_required".into());
                    }
                    // Refresh from this exact live occupant; an old saved ID is not sufficient.
                    self.save_session(task, &live, &cwd, None)?
                        .ok_or("flow.session_missing")?;
                    close_attempted = true;
                    self.check_flow(task, op)?;
                    self.herdr
                        .close_pane(&live, &|| self.check_flow(task, op))?;
                    if self.herdr.pane_exists(&live)? {
                        return Err("flow.close_pending".into());
                    }
                }
                self.db
                    .lock()
                    .map_err(err)?
                    .execute("DELETE FROM task_bindings WHERE task_id=?1", [task])
                    .map_err(err)?;
            }
            Ok::<_, String>(())
        })();
        match result {
            Ok(()) => {
                self.end_flow(op, "succeeded", "")?;
                Ok(Outcome::default())
            }
            Err(error) => {
                // Keep ownership, but allow a deliberate cleanup retry or reconnect to this task.
                self.db
                    .lock()
                    .map_err(err)?
                    .execute(
                        "UPDATE task_bindings SET state='valid' WHERE task_id=?1",
                        [task],
                    )
                    .map_err(err)?;
                self.end_flow(
                    op,
                    if close_attempted { "unknown" } else { "failed" },
                    &error,
                )?;
                Ok(Outcome {
                    warning: Some(error),
                })
            }
        }
    }
    /// Reconcile without sending, starting or closing anything. An unresolved launch may only
    /// be released after its exact pane is gone, or its already-confirmed dispatch is committed.
    pub fn recover_lifecycle(&self, task: &str, op: &str) -> Result<(), String> {
        let (workspace, _, _) = self.project_context(task)?;
        let flow = view(&*self.db.lock().map_err(err)?, task)?;
        if flow.operation_id.as_deref() != Some(op) || flow.state.as_deref() != Some("unknown") {
            return Err("dispatch.operation_conflict".into());
        }
        if flow.kind.as_deref() == Some("complete") {
            return Err("flow.retry_cleanup".into());
        }
        let b = self.binding(task)?;
        if let Some(dispatch) = b.operation.as_ref() {
            if dispatch.id != format!("{op}:dispatch") || dispatch.state != "unknown" {
                return Err("dispatch.operation_conflict".into());
            }
            return self.confirm_received_dispatch(dispatch);
        }
        let sent: bool = self
            .db
            .lock()
            .map_err(err)?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM dispatch_operations WHERE id=?1 AND state='sent')",
                [format!("{op}:dispatch")],
                |r| r.get(0),
            )
            .map_err(err)?;
        if sent {
            if let Some(target) = b.target.as_ref() {
                let (_, cwd, _) = self.project_context(task)?;
                if self
                    .save_session(task, target, &cwd, flow.session_id.as_deref())?
                    .is_none()
                {
                    return Err("flow.session_missing".into());
                }
            } else {
                return Err("binding.missing".into());
            }
            return self.end_flow(op, "succeeded", "");
        }
        let prior: bool = self
            .db
            .lock()
            .map_err(err)?
            .query_row(
                "SELECT dispatched_at IS NOT NULL FROM tasks WHERE id=?1",
                [task],
                |r| r.get(0),
            )
            .map_err(err)?;
        if prior {
            if let (Some(expected), Some(old)) = (flow.session_id, b.target.as_ref()) {
                if let Some(live) = self.herdr.targets(&workspace)?.into_iter().find(|t| {
                    t.pane_id == old.pane_id
                        && t.terminal_id == old.terminal_id
                        && t.session == old.session
                }) {
                    if self.herdr.session_id(&live)?.as_deref() == Some(expected.as_str()) {
                        self.reserve_target(task, op, &live)?;
                        return self.activate(task, op, &live);
                    }
                }
            }
        }
        let target = match b.target {
            Some(t) => Some(t),
            None => match self.allocation_absent(&workspace, op)? {
                Some(false) => return Err("flow.close_before_reset".into()),
                Some(true) => None,
                None => self
                    .herdr
                    .find_created(&workspace, &format!("kanban-{op}"))?,
            },
        };
        if let Some(target) = target {
            if self.herdr.pane_exists(&target)? {
                return Err("flow.close_before_reset".into());
            }
        }
        let mut db = self.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        ensure_operation(&tx, task, op)?;
        tx.execute(
            "UPDATE pane_allocations SET resolved=1 WHERE operation_id=?1",
            [op],
        )
        .map_err(err)?;
        tx.execute("DELETE FROM task_bindings WHERE task_id=?1", [task])
            .map_err(err)?;
        // A session created before an unsent first prompt must not make the next start skip it.
        let prior: bool = tx
            .query_row(
                "SELECT dispatched_at IS NOT NULL FROM tasks WHERE id=?1",
                [task],
                |r| r.get(0),
            )
            .map_err(err)?;
        if !prior {
            tx.execute("DELETE FROM task_sessions WHERE task_id=?1", [task])
                .map_err(err)?;
        }
        tx.execute("UPDATE lifecycle_operations SET state='failed',error='',evidence='Exact launch pane absent; no confirmed prompt' WHERE id=?1",[op]).map_err(err)?;
        tx.commit().map_err(err)
    }
}
