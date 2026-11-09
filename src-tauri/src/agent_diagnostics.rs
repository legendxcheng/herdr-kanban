//! Status-only diagnostics. Never persist CLI stderr, prompts, paths or environment values.
use crate::{agent_status::AgentSnapshot, workflow::Workflow};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    pub at: String,
    pub project_id: String,
    pub machine_id: String,
    pub workspace_id: String,
    pub session: String,
    pub pane_id: Option<String>,
    pub task_id: Option<String>,
    pub stage: String,
    pub code: String,
    pub elapsed_ms: u64,
}

pub fn staged(stage: &str, error: String) -> String {
    if error.starts_with("status_query|") {
        error
    } else {
        format!("status_query|{stage}|{error}")
    }
}

pub fn error_parts(error: &str, fallback: &str) -> (String, String) {
    let (stage, error) = error
        .strip_prefix("status_query|")
        .and_then(|s| s.split_once('|'))
        .unwrap_or((fallback, error));
    let code = error.split(':').next().unwrap_or("");
    let code = if code.len() <= 100
        && code.contains('.')
        && code
            .bytes()
            .all(|c| c.is_ascii_lowercase() || b"._".contains(&c))
    {
        code
    } else {
        "status.external_error"
    };
    (stage.to_owned(), code.to_owned())
}

pub fn failure(project: &str, stage: &str, error: &str) -> Diagnostic {
    let (stage, code) = error_parts(error, stage);
    Diagnostic {
        at: chrono::Utc::now().to_rfc3339(),
        project_id: project.into(),
        machine_id: String::new(),
        workspace_id: String::new(),
        session: String::new(),
        pane_id: None,
        task_id: None,
        stage,
        code,
        elapsed_ms: 0,
    }
}

/// Classify local read failures without putting database contents or JSON into logs.
pub fn read_error(stage: &str, error: impl std::fmt::Display, fallback: &str) -> String {
    let error = error.to_string();
    let (_, code) = error_parts(&error, stage);
    let code = if code != "status.external_error" {
        code.as_str()
    } else if error == rusqlite::Error::QueryReturnedNoRows.to_string() {
        "status.database_row_missing"
    } else {
        fallback
    };
    staged(stage, code.to_owned())
}

struct Context {
    base: Diagnostic,
    path: Option<PathBuf>,
    tasks: HashMap<String, String>,
}

impl Context {
    fn read(app: &Workflow, project: &str) -> Result<Self, String> {
        let db = app.db.lock().map_err(|e| e.to_string())?;
        let mut base = failure(project, "project", "status.external_error");
        (base.machine_id, base.workspace_id, base.session) = db.query_row(
            "SELECT machine_id,COALESCE(workspace_id,''),herdr_session FROM projects WHERE id=?1",
            [project], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e| e.to_string())?;
        let tasks = db.prepare("SELECT b.pane_id,t.id FROM task_bindings b JOIN tasks t ON t.id=b.task_id WHERE t.project_id=?1 AND t.archived_at IS NULL")
            .map_err(|e| e.to_string())?.query_map([project], |r| Ok((r.get(0)?,r.get(1)?)))
            .map_err(|e| e.to_string())?.collect::<Result<HashMap<_,_>,_>>().map_err(|e| e.to_string())?;
        let path = db
            .path()
            .filter(|p| !p.is_empty())
            .map(|p| PathBuf::from(p).with_file_name("agent-status.jsonl"));
        Ok(Self { base, path, tasks })
    }
    fn diagnostic(
        &self,
        error: &str,
        stage: &str,
        pane: Option<&str>,
        start: Instant,
    ) -> Diagnostic {
        let (stage, code) = error_parts(error, stage);
        Diagnostic {
            at: chrono::Utc::now().to_rfc3339(),
            stage,
            code,
            pane_id: pane.map(str::to_owned),
            task_id: pane.and_then(|p| self.tasks.get(p).cloned()),
            elapsed_ms: start.elapsed().as_millis() as u64,
            ..self.base.clone()
        }
    }
}

pub fn snapshot(app: &Workflow, project: &str) -> Result<AgentSnapshot, Diagnostic> {
    let start = Instant::now();
    let context = Context::read(app, project).map_err(|e| failure(project, "project", &e))?;
    let result = app
        .for_status_project(project)
        .and_then(|app| app.agent_snapshot(project));
    finish(context, result, start)
}

fn finish(context: Context, result: Result<AgentSnapshot, String>, start: Instant) -> Result<AgentSnapshot, Diagnostic> {
    match result {
        Err(error) => {
            let diagnostic = context.diagnostic(&error, "snapshot", None, start);
            record(context.path.as_deref(), &diagnostic, false);
            Err(diagnostic)
        }
        Ok(mut snapshot) => {
            record(
                context.path.as_deref(),
                &context.diagnostic("status.ok", "snapshot", None, start),
                true,
            );
            for item in &mut snapshot.failures {
                let diagnostic =
                    context.diagnostic(&item.error, "process_identity", Some(&item.pane_id), start);
                item.error = diagnostic.code.clone();
                record(context.path.as_deref(), &diagnostic, false);
                snapshot.diagnostics.push(diagnostic);
            }
            for agent in &snapshot.agents {
                if agent.verified_task_id.is_some()
                    && !snapshot.failures.iter().any(|f| f.pane_id == agent.pane_id)
                {
                    record(
                        context.path.as_deref(),
                        &context.diagnostic("status.ok", "snapshot", Some(&agent.pane_id), start),
                        true,
                    );
                }
            }
            Ok(snapshot)
        }
    }
}

#[derive(Serialize)]
pub struct SnapshotResult {
    pub snapshot: Option<AgentSnapshot>,
    pub error: Option<Diagnostic>,
}
impl From<Result<AgentSnapshot, Diagnostic>> for SnapshotResult {
    fn from(value: Result<AgentSnapshot, Diagnostic>) -> Self {
        match value {
            Ok(snapshot) => Self { snapshot: Some(snapshot), error: None },
            Err(error) => Self { snapshot: None, error: Some(error) },
        }
    }
}

/// One discovery per machine, with independent project errors and final ownership checks.
pub fn snapshots(app: &Workflow, projects: Vec<String>) -> HashMap<String, SnapshotResult> {
    let mut groups = HashMap::<String, Vec<(String, Context, Instant)>>::new();
    let mut output = HashMap::new();
    for project in projects.into_iter().collect::<std::collections::BTreeSet<_>>() {
        let start = Instant::now();
        match Context::read(app, &project) {
            Ok(context) => groups.entry(context.base.machine_id.clone()).or_default().push((project, context, start)),
            Err(error) => { output.insert(project.clone(), Err(failure(&project, "project", &error)).into()); }
        }
    }
    for (machine, entries) in groups {
        let routed = app.for_status_machine(&machine);
        let mut valid = Vec::new();
        for (project, context, start) in entries {
            match routed.as_ref().map_err(Clone::clone).and_then(|app| app.agent_workspace(&project)) {
                Ok(workspace) => valid.push((project, workspace, context, start)),
                Err(error) => { output.insert(project, finish(context, Err(error), start).into()); }
            }
        }
        if valid.is_empty() { continue; }
        let routed = routed.unwrap();
        let workspaces = valid.iter().map(|(_, workspace, _, _)| workspace.clone()).collect::<Vec<_>>();
        let discovered = routed.herdr.discover_agent_batch(&workspaces);
        for (project, workspace, context, start) in valid {
            let result = discovered.as_ref().map_err(Clone::clone)
                .and_then(|items| items.get(&workspace).cloned().ok_or_else(|| "herdr.invalid_response".to_string())?)
                .and_then(|discovery| routed.agent_snapshot_from_discovery(&project, &workspace, discovery));
            output.insert(project, finish(context, result, start).into());
        }
    }
    output
}

#[derive(Default)]
struct Journal {
    failures: HashMap<(PathBuf, String, Option<String>), (String, Instant)>,
}
impl Journal {
    fn event(
        &mut self,
        path: &std::path::Path,
        d: &Diagnostic,
        success: bool,
        now: Instant,
    ) -> Option<&'static str> {
        let key = (path.to_owned(), d.project_id.clone(), d.pane_id.clone());
        if success {
            return self.failures.remove(&key).map(|_| "recovered");
        }
        let fingerprint = format!("{}|{}", d.stage, d.code);
        if self.failures.get(&key).is_some_and(|(old, at)| {
            old == &fingerprint && now.duration_since(*at) < Duration::from_secs(60)
        }) {
            return None;
        }
        // Bound retained identities even when projects are removed while offline.
        if self.failures.len() >= 4096 {
            if let Some(oldest) = self
                .failures
                .iter()
                .min_by_key(|(_, (_, at))| *at)
                .map(|(key, _)| key.clone())
            {
                self.failures.remove(&oldest);
            }
        }
        self.failures.insert(key, (fingerprint, now));
        Some("failed")
    }
}

fn record(path: Option<&std::path::Path>, diagnostic: &Diagnostic, success: bool) {
    let Some(path) = path else { return };
    static JOURNAL: OnceLock<Mutex<Journal>> = OnceLock::new();
    let Ok(mut journal) = JOURNAL.get_or_init(Default::default).lock() else {
        return;
    };
    let Some(event) = journal.event(path, diagnostic, success, Instant::now()) else {
        return;
    };
    let mut row = serde_json::to_value(diagnostic).unwrap_or_default();
    row["event"] = event.into();
    // Existing bounded writer truncates at 2 MiB; diagnostics never fail a status read.
    let _ = crate::start_timing::append(path, &row);
}

#[tauri::command]
pub fn open_agent_status_logs(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let path = crate::db::db_path();
    let directory = path.parent().ok_or("status.log_directory_unavailable")?;
    app.opener()
        .open_path(directory.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stages_preserve_codes_but_never_external_output() {
        assert_eq!(
            error_parts(
                &staged("process_info", "herdr.failed: secret command output".into()),
                "snapshot"
            ),
            ("process_info".into(), "herdr.failed".into())
        );
        assert_eq!(
            error_parts("password from external error", "transport").1,
            "status.external_error"
        );
        assert_eq!(
            error_parts(
                &staged("transport", staged("agent_list", "herdr.timeout".into())),
                "snapshot"
            )
            .0,
            "agent_list"
        );
    }
    #[test]
    fn bounded_file_records_failure_and_recovery_without_success_poll_noise() {
        let directory =
            std::env::temp_dir().join(format!("kanban-status-log-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("agent-status.jsonl");
        std::fs::write(&path, vec![b' '; 2 * 1024 * 1024 + 1]).unwrap();
        let d = failure("p", "process_info", "herdr.timeout");
        record(Some(&path), &d, false);
        record(Some(&path), &d, false);
        let ok = failure("p", "snapshot", "status.ok");
        record(Some(&path), &ok, true);
        record(Some(&path), &ok, true);
        let text = std::fs::read_to_string(&path).unwrap();
        let rows: Vec<serde_json::Value> = text
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["event"], "failed");
        assert_eq!(rows[0]["code"], "herdr.timeout");
        assert_eq!(rows[1]["event"], "recovered");
        assert!(text.len() < 4096);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn failure_throttle_is_per_project_and_pane_and_recovery_is_once() {
        let mut journal = Journal::default();
        let path = std::path::Path::new("agent-status.jsonl");
        let now = Instant::now();
        let mut d = failure("p1", "process_info", "herdr.timeout");
        d.pane_id = Some("w1:p1".into());
        assert_eq!(journal.event(path, &d, false, now), Some("failed"));
        assert_eq!(
            journal.event(path, &d, false, now + Duration::from_secs(3)),
            None
        );
        assert_eq!(
            journal.event(path, &d, false, now + Duration::from_secs(60)),
            Some("failed")
        );
        d.pane_id = Some("w1:p2".into());
        assert_eq!(journal.event(path, &d, false, now), Some("failed"));
        assert_eq!(journal.event(path, &d, true, now), Some("recovered"));
        assert_eq!(journal.event(path, &d, true, now), None);
        d.pane_id = Some("w1:p1".into());
        d.code = "herdr.unavailable".into();
        assert_eq!(
            journal.event(path, &d, false, now + Duration::from_secs(61)),
            Some("failed")
        );
    }
}
