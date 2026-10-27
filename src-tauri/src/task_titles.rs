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
    INSERT OR IGNORE INTO schema_migrations(version) VALUES (11);")
}

pub trait TitleGenerator: Send + Sync {
    fn generate(&self, input: &str, stop: &AtomicBool) -> Result<String, String>;
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
                        let job = db.lock().ok().and_then(|db| take_next(&db).ok().flatten());
                        if let Some((task, input, project)) = job {
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    generator.generate(&input, &stop)
                                }))
                                .unwrap_or_else(|_| Err("title.worker_failed".into()));
                            if let Ok(db) = db.lock() {
                                let _ = finish(&db, &task, result);
                            }
                            if !stop.load(Ordering::Acquire) {
                                notify(project);
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
            tx.execute("INSERT INTO task_title_requests(task_id,id,input,state,created_ms) VALUES(?1,?2,?3,?4,?5)",
                params![task.id, uuid::Uuid::new_v4().to_string(), summary.input, if active { "pending" } else { "failed" }, chrono::Utc::now().timestamp_millis()]).map_err(err)?;
        }
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
            tx.execute("UPDATE task_title_requests SET state='invalidated',edit_version=edit_version+1 WHERE task_id=?1 AND state IN ('pending','running')", [id]).map_err(err)?;
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

fn take_next(db: &Connection) -> Result<Option<(String, String, String)>, String> {
    db.execute("UPDATE task_title_requests SET state='failed',error='title.queue_timeout' WHERE state='pending' AND created_ms<?1", [chrono::Utc::now().timestamp_millis() - 60_000]).map_err(err)?;
    let next = db.query_row("SELECT r.task_id,r.input,t.project_id FROM task_title_requests r JOIN tasks t ON t.id=r.task_id WHERE r.state='pending' ORDER BY r.created_ms,r.id LIMIT 1", [], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional().map_err(err)?;
    if let Some((task, _, _)) = &next {
        db.execute(
            "UPDATE task_title_requests SET state='running' WHERE task_id=?1",
            [task],
        )
        .map_err(err)?;
    }
    Ok(next)
}

fn finish(db: &Connection, task: &str, result: Result<String, String>) -> Result<(), String> {
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
    match result {
        Ok(title) => {
            let changed = tx.execute("UPDATE task_title_requests SET state='succeeded' WHERE task_id=?1 AND state='running'", [task]).map_err(err)?;
            if changed == 1 {
                tx.execute(
                    "UPDATE tasks SET name=?2,updated_at=?3 WHERE id=?1 AND archived_at IS NULL",
                    params![task, title, chrono::Utc::now().to_rfc3339()],
                )
                .map_err(err)?;
            }
        }
        Err(_) => {
            tx.execute("UPDATE task_title_requests SET state='failed',error='title.generation_failed' WHERE task_id=?1 AND state='running'", [task]).map_err(err)?;
        }
    }
    tx.commit().map_err(err)
}

#[cfg(test)]
#[path = "title_tests.rs"]
mod tests;
