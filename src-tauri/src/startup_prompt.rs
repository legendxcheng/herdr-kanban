//! Durable first input, before a single launch; recovery only observes that launch.
use crate::{
    codex_settings::Settings,
    lifecycle::Outcome,
    workflow::{Target, Workflow},
};
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Request {
    pub operation: String,
    pub target: Target,
    pub cwd: String,
    pub prompt: String,
    pub settings: Settings,
    pub epoch: i64,
}
pub(crate) fn init(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS startup_prompt_operations (
        operation_id TEXT PRIMARY KEY REFERENCES lifecycle_operations(id), request_json TEXT NOT NULL);")
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
impl Workflow {
    pub(crate) fn has_startup_prompt(&self, operation: &str) -> Result<bool, String> {
        self.db
            .lock()
            .map_err(err)?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM startup_prompt_operations WHERE operation_id=?1)",
                [operation],
                |r| r.get(0),
            )
            .map_err(err)
    }
    fn check_startup(&self, task: &str, request: &Request) -> Result<(), String> {
        self.check_running()?;
        let db = self.db.lock().map_err(err)?;
        crate::task_reset::check_epoch(&db, task, request.epoch)?;
        crate::lifecycle::ensure_operation(&db, task, &request.operation)?;
        crate::machines::ensure_enabled(&db, self.herdr.machine_id())?;
        crate::workflow::ensure_project_target(&db, task, &request.target, self.herdr.session())
    }
    pub(crate) fn start_with_prompt(
        &self,
        task: &str,
        operation: &str,
        target: &Target,
        cwd: &str,
        prompt: &str,
        settings: &Settings,
        progress: &dyn Fn(&str) -> Result<(), String>,
    ) -> Result<Outcome, String> {
        let target = self.herdr.startup_shell(target, &|| {
            self.check_running()?;
            crate::lifecycle::ensure_operation(&*self.db.lock().map_err(err)?, task, operation)
        })?;
        self.reserve_target(task, operation, &target)?;
        let epoch = crate::task_reset::epoch(&*self.db.lock().map_err(err)?, task)?;
        let request = Request {
            operation: operation.into(),
            target: target.clone(),
            cwd: cwd.into(),
            prompt: prompt.into(),
            settings: settings.clone(),
            epoch,
        };
        let database = self
            .db
            .lock()
            .map_err(err)?
            .path()
            .filter(|p| !p.is_empty())
            .map(std::path::PathBuf::from);
        let _trace = crate::dispatch_trace::Scope::enter(operation, &target, database);
        self.check_startup(task, &request)?;
        {
            let mut db = self.db.lock().map_err(err)?;
            let tx = db.transaction().map_err(err)?;
            crate::task_reset::check_epoch(&tx, task, epoch)?;
            crate::lifecycle::ensure_operation(&tx, task, operation)?;
            crate::machines::ensure_enabled(&tx, self.herdr.machine_id())?;
            // Unknown from the outset: a crash cannot tell whether the next external call ran.
            tx.execute(
                "INSERT INTO startup_prompt_operations VALUES (?1,?2)",
                params![operation, serde_json::to_string(&request).map_err(err)?],
            )
            .map_err(err)?;
            tx.execute("INSERT INTO dispatch_operations(id,task_id,workspace,target_json,prompt,created_at,state,had_binding) VALUES (?1,?2,?3,?4,?5,?6,'unknown',1)", params![format!("{operation}:dispatch"),task,target.workspace_id,serde_json::to_string(&target).map_err(err)?,prompt,chrono::Utc::now().to_rfc3339()]).map_err(err)?;
            tx.commit().map_err(err)?;
        }
        crate::dispatch_trace::record("startup_intent", "persisted", None);
        // Even a timeout may have started Codex. Never issue this call from recovery.
        crate::dispatch_trace::record("startup_request", "attempt", None);
        let check_write = || {
            self.check_startup(task, &request)?;
            crate::images::verify_operation(&*self.db.lock().map_err(err)?, operation)
        };
        check_write()?;
        let launch_error = self.herdr.launch_prompt(&request, &check_write).err();
        if let Some(error) = &launch_error {
            crate::dispatch_trace::record("startup_request", "error", Some(error));
            if error.starts_with("startup.not_launched:") {
                return Err(error.clone());
            }
        }
        progress("confirming_startup_prompt")?;
        let deadline = std::time::Instant::now() + self.herdr.startup_receipt_wait();
        loop {
            self.check_startup(task, &request)?;
            match self.reconcile_startup_prompt(task, operation, &|| self.check_running()) {
                Ok(()) => {
                    crate::dispatch_trace::record("startup_receipt", "committed", None);
                    return Ok(Outcome::default());
                }
                Err(error) if error.starts_with("startup.not_launched:") => return Err(error),
                Err(error) if std::time::Instant::now() >= deadline => {
                    return Err(launch_error.unwrap_or(error));
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(250)),
            }
        }
    }
    pub(crate) fn reconcile_startup_prompt(
        &self,
        task: &str,
        operation: &str,
        cancelled: &dyn Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        cancelled()?;
        let request: Request = {
            let db = self.db.lock().map_err(err)?;
            let json: String = db
                .query_row(
                    "SELECT request_json FROM startup_prompt_operations WHERE operation_id=?1",
                    [operation],
                    |r| r.get(0),
                )
                .map_err(err)?;
            serde_json::from_str(&json).map_err(err)?
        };
        self.check_startup(task, &request)?;
        let live = self
            .herdr
            .find_prompt_launch(&request)?
            .ok_or("startup.awaiting_process")?;
        crate::dispatch_trace::record("startup_process", "identified", None);
        // The adapter proves the PID. All previously acquired context stays immutable.
        if live.machine_id != request.target.machine_id
            || live.session != request.target.session
            || live.workspace_id != request.target.workspace_id
            || live.pane_id != request.target.pane_id
            || live.terminal_id != request.target.terminal_id
            || live.agent != "codex"
        {
            return Err("binding.identity_changed".into());
        }
        crate::session_resources::record_created(&*self.db.lock().map_err(err)?,task,operation,&live)?;
        cancelled()?;
        self.check_startup(task, &request)?;
        {
            let mut db = self.db.lock().map_err(err)?;
            let tx = db.transaction().map_err(err)?;
            crate::task_reset::check_epoch(&tx, task, request.epoch)?;
            crate::lifecycle::ensure_operation(&tx, task, operation)?;
            crate::machines::ensure_enabled(&tx, self.herdr.machine_id())?;
            let old: Option<String> = tx
                .query_row(
                    "SELECT target_json FROM task_bindings WHERE task_id=?1",
                    [task],
                    |r| r.get(0),
                )
                .optional()
                .map_err(err)?;
            let old: Target = serde_json::from_str(&old.ok_or("binding.missing")?).map_err(err)?;
            if !old.same_identity(&request.target) && !old.same_identity(&live) {
                return Err("binding.identity_changed".into());
            }
            let json = serde_json::to_string(&live).map_err(err)?;
            tx.execute(
                "UPDATE task_bindings SET target_json=?2 WHERE task_id=?1",
                params![task, json],
            )
            .map_err(err)?;
            tx.execute(
                "UPDATE lifecycle_operations SET target_json=?2 WHERE id=?1",
                params![operation, json],
            )
            .map_err(err)?;
            tx.execute(
                "UPDATE dispatch_operations SET target_json=?2 WHERE id=?1 AND state='unknown'",
                params![format!("{operation}:dispatch"), json],
            )
            .map_err(err)?;
            cancelled()?;
            tx.commit().map_err(err)?;
        }
        let known = crate::lifecycle::view(&*self.db.lock().map_err(err)?, task)?.session_id;
        self.save_session(task, operation, &live, &request.cwd, known.as_deref())?;
        self.check_startup(task, &request)?;
        let dispatch = self
            .binding(task)?
            .operation
            .ok_or("dispatch.operation_conflict")?;
        if dispatch.id != format!("{operation}:dispatch") {
            return Err("dispatch.operation_conflict".into());
        }
        self.confirm_received_dispatch(&dispatch, &|| {
            cancelled()?;
            self.check_running()
        })
    }
    pub(crate) fn fail_startup_not_launched(
        &self,
        task: &str,
        operation: &str,
        error: &str,
        cancelled: &dyn Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        cancelled()?;
        self.check_running()?;
        let mut db = self.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        crate::machines::ensure_enabled(&tx, self.herdr.machine_id())?;
        crate::lifecycle::ensure_operation(&tx, task, operation)?;
        tx.execute("UPDATE dispatch_operations SET state='not_sent',evidence=?2 WHERE id=?1 AND task_id=?3 AND state='unknown'",params![format!("{operation}:dispatch"),error,task]).map_err(err)?;
        tx.execute(
            "DELETE FROM task_bindings WHERE task_id=?1 AND state='reserved'",
            [task],
        )
        .map_err(err)?;
        tx.execute(
            "UPDATE lifecycle_operations SET state='failed',error=?2 WHERE id=?1",
            params![operation, error],
        )
        .map_err(err)?;
        tx.execute(
            "UPDATE pane_allocations SET resolved=1 WHERE operation_id=?1",
            [operation],
        )
        .map_err(err)?;
        cancelled()?;
        tx.commit().map_err(err)
    }
}
