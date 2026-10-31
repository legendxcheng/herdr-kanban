//! Task authoring and bounded, best-effort background naming, independent of development.
use crate::{
    images,
    models::{Task, TaskPriority},
};
use rusqlite::{Connection, OptionalExtension, params};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::{thread, time::Duration};

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

pub fn init(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS task_title_requests (
        task_id TEXT PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
        id TEXT NOT NULL UNIQUE, input TEXT NOT NULL, state TEXT NOT NULL,
        created_ms INTEGER NOT NULL, edit_version INTEGER NOT NULL DEFAULT 0, error TEXT NOT NULL DEFAULT ''
    );
    CREATE TRIGGER IF NOT EXISTS invalidate_task_title AFTER UPDATE OF name,prompt_json,archived_at ON tasks
    WHEN OLD.name IS NOT NEW.name OR OLD.prompt_json IS NOT NEW.prompt_json OR NEW.archived_at IS NOT NULL
    BEGIN
        UPDATE task_title_requests SET state='invalidated',edit_version=edit_version+1
        WHERE task_id=NEW.id AND state IN ('pending','running');
    END;
    INSERT OR IGNORE INTO schema_migrations(version) VALUES (11);")?;
    if !db.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=19)",
        [],
        |r| r.get::<_, bool>(0),
    )? {
        let tx = db.unchecked_transaction()?;
        tx.execute_batch("ALTER TABLE task_title_requests ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE task_title_requests ADD COLUMN next_attempt_ms INTEGER NOT NULL DEFAULT 0;
            UPDATE task_title_requests SET state='interrupted',error='title.interrupted' WHERE state IN ('pending','running');
            CREATE INDEX title_due ON task_title_requests(state,next_attempt_ms,created_ms);
            DROP TRIGGER IF EXISTS invalidate_task_title;
            CREATE TRIGGER invalidate_task_title AFTER UPDATE OF name,prompt_json,archived_at ON tasks
            WHEN OLD.name IS NOT NEW.name OR OLD.prompt_json IS NOT NEW.prompt_json OR NEW.archived_at IS NOT NULL
            BEGIN
                UPDATE task_title_requests SET state='invalidated',edit_version=edit_version+1
                WHERE task_id=NEW.id AND state IN ('pending','running','retry_wait','failed','interrupted');
            END;
            INSERT INTO schema_migrations(version) VALUES(19);")?;
        tx.commit()?;
    }
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TitleState {
    pub id: String,
    pub state: String,
    pub attempts: i64,
    pub next_attempt_ms: i64,
    pub error: String,
}

pub fn view(db: &Connection, task: &str) -> Result<Option<TitleState>, String> {
    db.query_row(
        "SELECT id,state,attempts,next_attempt_ms,error FROM task_title_requests WHERE task_id=?1",
        [task],
        |r| {
            Ok(TitleState {
                id: r.get(0)?,
                state: r.get(1)?,
                attempts: r.get(2)?,
                next_attempt_ms: r.get(3)?,
                error: r.get(4)?,
            })
        },
    )
    .optional()
    .map_err(err)
}

fn retry_delay(attempt: i64) -> i64 {
    match attempt {
        0 | 1 => 10_000,
        2 => 30_000,
        _ => 120_000,
    }
}

pub fn recover_at(db: &Connection, now: i64) -> rusqlite::Result<()> {
    db.execute("UPDATE task_title_requests SET
        state=CASE WHEN attempts<4 THEN 'retry_wait' ELSE 'failed' END,
        next_attempt_ms=?1+CASE WHEN attempts<=1 THEN 10000 WHEN attempts=2 THEN 30000 ELSE 120000 END,
        error='title.interrupted' WHERE state='running'", [now])?;
    Ok(())
}

fn retry_request(db: &Connection, task: &str, request: &str, now: i64) -> Result<(), String> {
    let tx = db.unchecked_transaction().map_err(err)?;
    crate::tasks::ensure_active(&tx, task)?;
    let current = view(&tx, task)?.ok_or("title.stale")?;
    // Concurrent clicks join an already queued request, without resetting its budget.
    if matches!(current.state.as_str(), "pending" | "running" | "retry_wait") {
        return Ok(());
    }
    if current.id != request || !matches!(current.state.as_str(), "failed" | "interrupted") {
        return Err("title.stale".into());
    }
    let json: String = tx
        .query_row("SELECT prompt_json FROM tasks WHERE id=?1", [task], |r| {
            r.get(0)
        })
        .map_err(err)?;
    let input = crate::title_prompt::summarize(&json, "zh-CN")?.input;
    if input.is_empty() {
        return Err("title.input".into());
    }
    tx.execute("UPDATE task_title_requests SET id=?2,input=?3,state='pending',attempts=0,next_attempt_ms=?4,created_ms=?4,error='',edit_version=edit_version+1 WHERE task_id=?1",
        params![task, uuid::Uuid::new_v4().to_string(), input, now]).map_err(err)?;
    tx.commit().map_err(err)
}

pub trait TitleGenerator: Send + Sync {
    fn generate(&self, project: &str, input: &str, stop: &AtomicBool) -> Result<String, String>;
}

pub struct NewTask {
    pub project_id: String,
    pub name: Option<String>,
    pub prompt_json: String,
    pub priority: TaskPriority,
    pub draft_id: Option<String>,
    pub locale: String,
}

pub struct TaskTitles {
    db: Arc<Mutex<Connection>>,
    stop: Arc<AtomicBool>,
    workers: Mutex<Vec<thread::JoinHandle<()>>>,
    available: bool,
}

impl TaskTitles {
    pub fn new(
        db: Arc<Mutex<Connection>>,
        generator: Arc<dyn TitleGenerator>,
        notify: Arc<dyn Fn(String) + Send + Sync>,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let mut workers = Vec::new();
        for index in 0..2 {
            let (db, generator, stop, notify) =
                (db.clone(), generator.clone(), stop.clone(), notify.clone());
            if let Ok(handle) = thread::Builder::new()
                .name(format!("task-title-{index}"))
                .spawn(move || {
                    while !stop.load(Ordering::Acquire) {
                        let job = db.lock().ok().and_then(|db| {
                            take_next_at(&db, chrono::Utc::now().timestamp_millis())
                                .ok()
                                .flatten()
                        });
                        if let Some(job) = job {
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    generator.generate(&job.project, &job.input, &stop)
                                }))
                                .unwrap_or_else(|_| Err("title.worker_failed".into()));
                            if let Ok(db) = db.lock() {
                                let result = if stop.load(Ordering::Acquire) {
                                    Err("title.cancelled".into())
                                } else {
                                    result
                                };
                                let _ = finish_at(
                                    &db,
                                    &job,
                                    result,
                                    chrono::Utc::now().timestamp_millis(),
                                );
                            }
                            if !stop.load(Ordering::Acquire) {
                                notify(job.project);
                            }
                        } else {
                            thread::sleep(Duration::from_millis(50));
                        }
                    }
                })
            {
                workers.push(handle);
            }
        }
        Self {
            db,
            stop,
            available: !workers.is_empty(),
            workers: Mutex::new(workers),
        }
    }

    pub fn create(&self, input: NewTask) -> Result<Task, String> {
        self.create_configured(input, Default::default())
    }

    pub fn retry(&self, task: &str, request: &str) -> Result<(), String> {
        if self.stop.load(Ordering::Acquire) || !self.available {
            return Err("title.worker_failed".into());
        }
        retry_request(
            &*self.db.lock().map_err(err)?,
            task,
            request,
            chrono::Utc::now().timestamp_millis(),
        )
    }

    pub fn last_codex_settings(&self) -> Result<crate::codex_settings::Settings, String> {
        crate::codex_settings::last(&*self.db.lock().map_err(err)?)
    }

    pub fn create_configured(
        &self,
        input: NewTask,
        settings: crate::codex_settings::Settings,
    ) -> Result<Task, String> {
        let supplied = input
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let summary = if supplied.is_none() {
            Some(crate::title_prompt::summarize(
                &input.prompt_json,
                &input.locale,
            )?)
        } else {
            None
        };
        let name = supplied.unwrap_or_else(|| &summary.as_ref().unwrap().fallback);
        let db = self.db.lock().map_err(err)?;
        let tx = db.unchecked_transaction().map_err(err)?;
        let mut task = images::save_task_in(
            &tx,
            None,
            &input.project_id,
            name,
            &input.prompt_json,
            input.priority,
            input.draft_id.as_deref(),
        )?;
        crate::codex_settings::save(&tx, &task.id, &settings, true)?;
        task.codex_settings = settings;
        if let Some(summary) = summary.filter(|s| !s.input.is_empty()) {
            let active = !self.stop.load(Ordering::Acquire) && self.available;
            tx.execute("INSERT INTO task_title_requests(task_id,id,input,state,created_ms,next_attempt_ms) VALUES(?1,?2,?3,?4,?5,?5)",
                params![task.id, uuid::Uuid::new_v4().to_string(), summary.input, if active { "pending" } else { "failed" }, chrono::Utc::now().timestamp_millis()]).map_err(err)?;
        }
        task.title_generation = view(&tx, &task.id)?;
        tx.commit().map_err(err)?;
        Ok(task)
    }

    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Release);
        let workers = self
            .workers
            .lock()
            .map(|mut workers| workers.drain(..).collect::<Vec<_>>())
            .unwrap_or_default();
        for worker in workers {
            let _ = worker.join();
        }
    }

    pub fn update(
        &self,
        id: &str,
        name: Option<&str>,
        json: &str,
        priority: TaskPriority,
        draft: Option<&str>,
    ) -> Result<(), String> {
        self.update_configured(id, name, json, priority, draft, None)
    }

    pub fn update_configured(
        &self,
        id: &str,
        name: Option<&str>,
        json: &str,
        priority: TaskPriority,
        draft: Option<&str>,
        settings: Option<&crate::codex_settings::Settings>,
    ) -> Result<(), String> {
        let db = self.db.lock().map_err(err)?;
        let tx = db.unchecked_transaction().map_err(err)?;
        let (project, current): (String, String) = tx
            .query_row("SELECT project_id,name FROM tasks WHERE id=?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map_err(|_| "task.missing")?;
        if name.is_some_and(|name| name.trim().is_empty()) {
            return Err("task.title_required".into());
        }
        if let Some(settings) = settings {
            crate::codex_settings::save(&tx, id, settings, false)?;
        }
        // An explicit rename also invalidates when its value equals the temporary name.
        if name.is_some() {
            tx.execute("UPDATE task_title_requests SET state='invalidated',edit_version=edit_version+1 WHERE task_id=?1 AND state IN ('pending','running','retry_wait','failed','interrupted')", [id]).map_err(err)?;
        }
        images::save_task_in(
            &tx,
            Some(id),
            &project,
            name.map(str::trim).unwrap_or(&current),
            json,
            priority,
            draft,
        )?;
        tx.commit().map_err(err)
    }
}
impl Drop for TaskTitles {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct Job {
    task: String,
    id: String,
    version: i64,
    attempt: i64,
    input: String,
    project: String,
}

fn take_next_at(db: &Connection, now: i64) -> Result<Option<Job>, String> {
    let tx = db.unchecked_transaction().map_err(err)?;
    let next = tx.query_row("SELECT r.task_id,r.id,r.edit_version,r.attempts+1,r.input,t.project_id FROM task_title_requests r JOIN tasks t ON t.id=r.task_id WHERE r.state IN ('pending','retry_wait') AND r.next_attempt_ms<=?1 AND r.attempts<4 AND t.archived_at IS NULL ORDER BY r.next_attempt_ms,r.created_ms,r.id LIMIT 1", [now], |r| Ok(Job {
        task: r.get(0)?, id: r.get(1)?, version: r.get(2)?, attempt: r.get(3)?, input: r.get(4)?, project: r.get(5)?,
    })).optional().map_err(err)?;
    if let Some(job) = &next {
        tx.execute(
            "UPDATE task_title_requests SET state='running',attempts=attempts+1 WHERE task_id=?1",
            [&job.task],
        )
        .map_err(err)?;
    }
    tx.commit().map_err(err)?;
    Ok(next)
}

fn finish_at(
    db: &Connection,
    job: &Job,
    result: Result<String, String>,
    now: i64,
) -> Result<(), String> {
    let result = result.and_then(|title| {
        let title = title.trim().trim_matches(['"', '\'', '“', '”', '`']).trim();
        if title.is_empty()
            || title
                .chars()
                .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
        {
            return Err("title.invalid_output".into());
        }
        Ok(title.chars().take(40).collect::<String>())
    });
    let tx = db.unchecked_transaction().map_err(err)?;
    let valid: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM task_title_requests r JOIN tasks t ON t.id=r.task_id WHERE r.task_id=?1 AND r.id=?2 AND r.edit_version=?3 AND r.attempts=?4 AND r.state='running' AND t.archived_at IS NULL)",
        params![job.task, job.id, job.version, job.attempt], |r| r.get(0)).map_err(err)?;
    if !valid {
        return Ok(());
    }
    match result {
        Ok(title) => {
            let changed = tx.execute("UPDATE task_title_requests SET state='succeeded',error='',next_attempt_ms=0 WHERE task_id=?1 AND state='running'", [&job.task]).map_err(err)?;
            if changed == 1 {
                tx.execute(
                    "UPDATE tasks SET name=?2,updated_at=?3 WHERE id=?1 AND archived_at IS NULL",
                    params![job.task, title, chrono::Utc::now().to_rfc3339()],
                )
                .map_err(err)?;
            }
        }
        Err(error) => {
            // Store only stable codes, never CLI stderr, prompts or credentials.
            let code = if (error.starts_with("title.") || error.starts_with("machine."))
                && error.len() <= 80
                && error
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b == b'.' || b == b'_')
            {
                error.as_str()
            } else {
                "title.generation_failed"
            };
            let retryable = matches!(
                code,
                "title.timeout"
                    | "machine.timeout"
                    | "title.cli_failed"
                    | "machine.cli_failed"
                    | "title.generation_failed"
                    | "title.invalid_output"
                    | "title.io"
                    | "machine.io"
                    | "title.cancelled"
                    | "title.network"
                    | "title.rate_limited"
                    | "title.service_unavailable"
                    | "machine.cancelled"
            );
            let retry = retryable && job.attempt < 4;
            tx.execute("UPDATE task_title_requests SET state=?2,error=?3,next_attempt_ms=?4 WHERE task_id=?1",
                params![job.task, if retry { "retry_wait" } else { "failed" }, code, if retry { now + retry_delay(job.attempt) } else { 0 }]).map_err(err)?;
        }
    }
    tx.commit().map_err(err)
}

#[cfg(test)]
#[path = "title_tests.rs"]
mod tests;
