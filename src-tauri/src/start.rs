//! Durable acceptance is local; the worker and its progress outlive any viewing page.
use crate::{
    lifecycle::{self, Flow},
    workflow::Workflow,
};
use rusqlite::{OptionalExtension, params};
use std::sync::Arc;

#[derive(Clone, serde::Serialize)]
pub struct StartChanged {
    pub project_id: String,
    pub task_id: String,
    pub operation_id: String,
}
pub type Notify = Arc<dyn Fn(StartChanged) + Send + Sync>;

struct Request {
    workspace: String,
    cwd: String,
    prompt_json: String,
    needs_prompt: bool,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl Workflow {
    pub fn submit_start(
        self: &Arc<Self>,
        task: &str,
        op: &str,
        notify: Notify,
    ) -> Result<Flow, String> {
        let (request, event, accepted) = {
            let mut db = self.db.lock().map_err(err)?;
            let tx = db.transaction().map_err(err)?;
            crate::tasks::ensure_active(&tx, task)?;
            let existing: Option<(String, String)> = tx
                .query_row(
                    "SELECT task_id,kind FROM lifecycle_operations WHERE id=?1",
                    [op],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(err)?;
            if let Some((owner, kind)) = existing {
                if owner != task || kind != "start" {
                    return Err("dispatch.operation_conflict".into());
                }
                let flow = lifecycle::view(&tx, task)?;
                if flow.operation_id.as_deref() != Some(op) {
                    return Err("dispatch.operation_conflict".into());
                }
                return Ok(flow);
            }
            let workspace = crate::projects::task_identity(&tx, task, self.herdr.session())?;
            lifecycle::begin_flow_in(&tx, task, op, "start")?;
            let (project_id, cwd, prompt_json, original_status, saved): (String, String, String, String, bool) = tx.query_row(
                "SELECT t.project_id,COALESCE(s.cwd,d.cwd,p.root_path),t.prompt_json,t.status,s.codex_session_id IS NOT NULL
                 FROM tasks t JOIN projects p ON p.id=t.project_id LEFT JOIN task_sessions s ON s.task_id=t.id
                 LEFT JOIN task_working_directories d ON d.task_id=t.id WHERE t.id=?1", [task],
                |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(err)?;
            let needs_prompt = !saved && crate::workflow::binding(&tx, task)?.target.is_none();
            if needs_prompt {
                crate::images::pin_prompt(&tx, task, op, &prompt_json)?;
            }
            tx.execute("INSERT INTO start_requests(operation_id,project_id,workspace,cwd,prompt_json,needs_prompt,original_status)
                VALUES (?1,?2,?3,?4,?5,?6,?7)", params![op,project_id,workspace,cwd,prompt_json,needs_prompt,original_status]).map_err(err)?;
            let accepted = lifecycle::view(&tx, task)?;
            tx.commit().map_err(err)?;
            (
                Request {
                    workspace,
                    cwd,
                    prompt_json,
                    needs_prompt,
                },
                StartChanged {
                    project_id,
                    task_id: task.into(),
                    operation_id: op.into(),
                },
                accepted,
            )
        };
        let worker = self.clone();
        let changed = event.clone();
        let reporter = notify.clone();
        // A thread is owned by the application, not by an invoke future or WebView.
        if let Err(error) = std::thread::Builder::new().name("kanban-start".into()).spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let progress = |stage: &str| -> Result<(), String> {
                    worker.start_stage(&changed.operation_id, stage)?;
                    reporter(changed.clone());
                    Ok(())
                };
                let prompt = if request.needs_prompt {
                    progress("reading_prompt")?;
                    let resolved = {
                        let db = worker.db.lock().map_err(err)?;
                        crate::images::resolve_prompt(&db, &request.prompt_json)
                    };
                    match resolved {
                        Ok(text) => text,
                        Err(error) => {
                            worker.fail_unstarted(&changed.task_id, &changed.operation_id, &error)?;
                            return Err(error);
                        }
                    }
                } else { String::new() };
                worker.run_start(&changed.task_id, &prompt, &changed.operation_id,
                    Some((request.workspace, request.cwd)), &progress).map(|_| ())
            }));
            // If an unexpected error left the operation pending, retain its evidence.
            let error = match result {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(error),
                Err(_) => Some("flow.worker_failed".into()),
            };
            if let Some(error) = error {
                if let Ok(db) = worker.db.lock() {
                    let _ = db.execute("UPDATE lifecycle_operations SET state='unknown',error=?2 WHERE id=?1 AND state='pending' AND EXISTS(SELECT 1 FROM tasks WHERE id=lifecycle_operations.task_id)",
                        params![changed.operation_id, error]);
                }
            }
            reporter(changed);
        }) {
            self.fail_unstarted(task, op, &format!("flow.worker_failed: {error}"))?;
            notify(event);
            return Err(format!("flow.worker_failed: {error}"));
        }
        notify(event);
        Ok(accepted)
    }

    fn start_stage(&self, op: &str, stage: &str) -> Result<(), String> {
        let db = self.db.lock().map_err(err)?;
        let updated = db
            .execute(
                "UPDATE start_requests SET stage=?2 WHERE operation_id=?1
            AND EXISTS(SELECT 1 FROM lifecycle_operations o JOIN tasks t ON t.id=o.task_id WHERE o.id=?1 AND o.state='pending')",
                params![op, stage],
            )
            .map_err(err)?;
        if updated != 1 {
            return Err("dispatch.operation_conflict".into());
        }
        Ok(())
    }
}
