//! Per-step, best-effort dispatch diagnostics. Never record prompts or command arguments.
use crate::workflow::Target;
use serde_json::json;
use std::{cell::RefCell, marker::PhantomData, path::PathBuf, rc::Rc, time::Instant};

thread_local! {
    static ACTIVE: RefCell<Option<Trace>> = const { RefCell::new(None) };
}

struct Trace {
    operation: String,
    target: Target,
    start: Instant,
    path: Option<PathBuf>,
}

pub(crate) struct Scope(Option<Trace>, PhantomData<Rc<()>>);

impl Scope {
    pub fn enter(operation: &str, target: &Target, database: Option<PathBuf>) -> Self {
        let trace = Trace {
            operation: operation.into(),
            target: target.clone(),
            start: Instant::now(),
            path: database.map(|p| p.with_file_name("dispatch-input.jsonl")),
        };
        Self(ACTIVE.with(|slot| slot.replace(Some(trace))), PhantomData)
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        ACTIVE.with(|slot| slot.replace(self.0.take()));
    }
}

pub(crate) fn record(stage: &str, outcome: &str, error: Option<&str>) {
    ACTIVE.with(|slot| {
        let trace = slot.borrow();
        let Some(trace) = trace.as_ref() else { return };
        let Some(path) = &trace.path else { return };
        // External CLI errors can contain user content. Only admit stable error codes.
        let code = error.map(|e| {
            let code = e.split(':').next().unwrap_or("");
            if code.len() <= 80
                && code.contains('.')
                && code
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || b"._".contains(&c))
            {
                code
            } else {
                "external.error"
            }
        });
        let row = json!({"at":chrono::Utc::now().to_rfc3339(), "operation_id":trace.operation,
            "machine_id":trace.target.machine_id,"session":trace.target.session,
            "pane_id":trace.target.pane_id,"terminal_id":trace.target.terminal_id,
            "process_id":trace.target.process_id,"stage":stage,"outcome":outcome,
            "error":code,"elapsed_ms":trace.start.elapsed().as_millis()});
        let _ = crate::start_timing::append(path, &row);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trace_is_immediate_scoped_and_excludes_prompt_and_external_error_body() {
        let directory = std::env::temp_dir().join(format!("kanban-trace-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("dispatch-input.jsonl");
        let target = Target {
            machine_id: "remote".into(),
            session: "session".into(),
            workspace_id: "w1".into(),
            pane_id: "w1:p1".into(),
            terminal_id: "terminal".into(),
            agent: "codex".into(),
            process_id: "1:2".into(),
            codex_resume_locks: vec![],
            title: "PRIVATE TITLE".into(),
            status: "idle".into(),
        };
        {
            let _scope = Scope::enter("operation", &target, Some(directory.join("kanban.db")));
            record("text", "requested", None);
            record("text", "failed", Some("herdr.failed: PRIVATE PROMPT"));
            assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 2);
        }
        record("enter", "requested", None);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("PRIVATE"));
        let rows: Vec<serde_json::Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1]["error"], "herdr.failed");
        assert_eq!(rows[0]["operation_id"], "operation");
        assert_eq!(rows[0]["machine_id"], "remote");
        assert_eq!(rows[0]["terminal_id"], "terminal");
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
