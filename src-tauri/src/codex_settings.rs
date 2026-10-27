//! Task-owned startup choices; defaults are intentionally not resolved into a model.
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub model: Option<String>,
    pub effort: Option<String>,
}

impl Settings {
    pub fn explicit(&self) -> bool {
        self.model.is_some() || self.effort.is_some()
    }
    pub fn validate(&self) -> Result<(), String> {
        for value in [&self.model, &self.effort].into_iter().flatten() {
            if value.is_empty()
                || value.len() > 160
                || !value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-._/".contains(&c))
            {
                return Err("model.invalid_settings".into());
            }
        }
        Ok(())
    }
    pub fn args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(model) = &self.model {
            args.extend(["--model".into(), model.clone()]);
        }
        if let Some(effort) = &self.effort {
            args.extend(["-c".into(), format!("model_reasoning_effort=\"{effort}\"")]);
        }
        args
    }
}

pub fn init(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS task_codex_settings (
        task_id TEXT PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE, model TEXT, effort TEXT);
        CREATE TABLE IF NOT EXISTS last_codex_settings (
        id INTEGER PRIMARY KEY CHECK(id=1), model TEXT, effort TEXT);
        CREATE TABLE IF NOT EXISTS start_codex_settings (
        operation_id TEXT PRIMARY KEY REFERENCES lifecycle_operations(id), model TEXT, effort TEXT);
        CREATE TABLE IF NOT EXISTS codex_catalog_cache (
        machine_id TEXT NOT NULL, cwd TEXT NOT NULL, catalog TEXT NOT NULL, PRIMARY KEY(machine_id,cwd));
        INSERT OR IGNORE INTO schema_migrations(version) VALUES(15);")
}

pub fn read(db: &Connection, task: &str) -> Result<Settings, String> {
    db.query_row(
        "SELECT model,effort FROM task_codex_settings WHERE task_id=?1",
        [task],
        |r| {
            Ok(Settings {
                model: r.get(0)?,
                effort: r.get(1)?,
            })
        },
    )
    .optional()
    .map(|v| v.unwrap_or_default())
    .map_err(|e| e.to_string())
}

pub fn last(db: &Connection) -> Result<Settings, String> {
    db.query_row(
        "SELECT model,effort FROM last_codex_settings WHERE id=1",
        [],
        |r| {
            Ok(Settings {
                model: r.get(0)?,
                effort: r.get(1)?,
            })
        },
    )
    .optional()
    .map(|v| v.unwrap_or_default())
    .map_err(|e| e.to_string())
}

pub fn editable(db: &Connection, task: &str) -> Result<bool, String> {
    db.query_row("SELECT status='todo' AND archived_at IS NULL
        AND NOT EXISTS(SELECT 1 FROM task_sessions WHERE task_id=t.id)
        AND NOT EXISTS(SELECT 1 FROM task_bindings WHERE task_id=t.id)
        AND NOT EXISTS(SELECT 1 FROM lifecycle_operations WHERE task_id=t.id AND state IN ('pending','unknown'))
        AND NOT EXISTS(SELECT 1 FROM dispatch_operations WHERE task_id=t.id AND state IN ('pending','unknown','sent'))
        FROM tasks t WHERE id=?1", [task], |r| r.get(0)).map_err(|e| e.to_string())
}

pub fn save(
    db: &Connection,
    task: &str,
    settings: &Settings,
    remember: bool,
) -> Result<(), String> {
    settings.validate()?;
    if read(db, task)? != *settings && !editable(db, task)? {
        return Err("model.settings_locked".into());
    }
    if settings.explicit() && (remember || read(db, task)? != *settings) {
        let (machine,cwd): (String,String) = db.query_row("SELECT p.machine_id,p.root_path FROM projects p JOIN tasks t ON t.project_id=p.id WHERE t.id=?1", [task], |r| Ok((r.get(0)?,r.get(1)?))).map_err(|e| e.to_string())?;
        if let Some(catalog) = crate::codex_catalog::cached(db, &machine, &cwd)? {
            catalog.validate(settings)?;
        }
    }
    db.execute(
        "INSERT INTO task_codex_settings(task_id,model,effort) VALUES(?1,?2,?3)
        ON CONFLICT(task_id) DO UPDATE SET model=excluded.model,effort=excluded.effort",
        params![task, settings.model, settings.effort],
    )
    .map_err(|e| e.to_string())?;
    if remember {
        db.execute(
            "INSERT INTO last_codex_settings(id,model,effort) VALUES(1,?1,?2)
            ON CONFLICT(id) DO UPDATE SET model=excluded.model,effort=excluded.effort",
            params![settings.model, settings.effort],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn freeze(db: &Connection, task: &str, operation: &str) -> Result<Settings, String> {
    let settings = read(db, task)?;
    db.execute(
        "INSERT OR IGNORE INTO start_codex_settings(operation_id,model,effort) VALUES(?1,?2,?3)",
        params![operation, settings.model, settings.effort],
    )
    .map_err(|e| e.to_string())?;
    db.query_row(
        "SELECT model,effort FROM start_codex_settings WHERE operation_id=?1",
        [operation],
        |r| {
            Ok(Settings {
                model: r.get(0)?,
                effort: r.get(1)?,
            })
        },
    )
    .map_err(|e| e.to_string())
}
