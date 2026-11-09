//! Archive only after the task's exact pane has been closed or confirmed absent.
use super::*;
use crate::tasks::{BatchTaskIssue, BatchTaskResult};

impl Workflow {
    /// A project scope is supplied by batch callers. False means its snapshot
    /// item is no longer eligible (single-item repeated archive is successful).
    pub fn archive_task(&self, task: &str, project: Option<&str>) -> Result<bool, String> {
        let op = uuid::Uuid::new_v4().to_string();
        {
            let mut db = self.db.lock().map_err(err)?;
            let tx = db.transaction().map_err(err)?;
            let row: Option<(String, String, bool)> = tx
                .query_row(
                    "SELECT project_id,status,archived_at IS NOT NULL FROM tasks WHERE id=?1",
                    [task],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(err)?;
            let Some((owner, status, archived)) = row else {
                return if project.is_some() {
                    Ok(false)
                } else {
                    Err("task.missing".into())
                };
            };
            if project.is_some_and(|expected| owner != expected)
                || (project.is_some() && (archived || status != "done"))
            {
                return Ok(false);
            }
            if archived {
                return Ok(true);
            }
            if status != "done" {
                return Err("archive.only_done".into());
            }
            // With no binding, archive locally; there is no remote target to
            // close. Existing unresolved-operation guards still apply.
            let has_binding: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM task_bindings WHERE task_id=?1)",
                    [task],
                    |r| r.get(0),
                )
                .map_err(err)?;
            if !has_binding {
                crate::tasks::archive_in_transaction(&tx, task)?;
                tx.commit().map_err(err)?;
                return Ok(true);
            }
            // Eligibility and the completion claim share a transaction. A
            // concurrent start/dispatch cannot turn this into closing new work.
            begin_flow_in(&tx, task, &op, "complete")?;
            tx.commit().map_err(err)?;
        }
        let routed = match self.for_task(task) {
            Ok(app) => app,
            Err(error) => {
                self.end_flow(&op, "failed", &error)?;
                return Err(error);
            }
        };
        let result = routed.complete_started(task, &op, true)?;
        match result.warning {
            Some(error) => Err(error),
            None => Ok(true),
        }
    }

    pub fn archive_completed_tasks(&self, project: &str, ids: &[String]) -> BatchTaskResult {
        let mut result = BatchTaskResult::default();
        let mut seen = std::collections::HashSet::new();
        for task in ids {
            if !seen.insert(task) {
                continue;
            }
            match self.archive_task(task, Some(project)) {
                Ok(true) => result.succeeded.push(task.clone()),
                Ok(false) => result.skipped.push(BatchTaskIssue {
                    id: task.clone(),
                    error: "batch.task_changed".into(),
                }),
                Err(error) => {
                    let issue = BatchTaskIssue {
                        id: task.clone(),
                        error,
                    };
                    if matches!(
                        issue.error.as_str(),
                        "flow.verify_required"
                            | "dispatch.verify_required"
                            | "archive.cleanup_required"
                    ) {
                        result.skipped.push(issue);
                    } else {
                        result.failed.push(issue);
                    }
                }
            }
        }
        result
    }
}
