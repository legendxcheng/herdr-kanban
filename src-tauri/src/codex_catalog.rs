//! Read-only discovery in the same machine and working directory as task execution.
use crate::codex_settings::Settings;
use rusqlite::{Connection, OptionalExtension, params};
use std::{process::Command, sync::atomic::AtomicBool, time::Duration};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub efforts: Vec<String>,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Catalog {
    pub models: Vec<Model>,
    pub source: String,
    pub cached: bool,
    pub error: Option<String>,
}
impl Catalog {
    pub fn validate(&self, settings: &Settings) -> Result<(), String> {
        settings.validate()?;
        if !settings.explicit() {
            return Ok(());
        }
        let id = settings.model.as_deref().ok_or("model.choose_model")?;
        let model = self
            .models
            .iter()
            .find(|m| m.id == id)
            .ok_or("model.unavailable")?;
        if settings
            .effort
            .as_ref()
            .is_some_and(|e| !model.efforts.contains(e))
        {
            return Err("model.effort_unavailable".into());
        }
        Ok(())
    }
}

pub fn local(cwd: &str) -> Result<Catalog, String> {
    // `codex` is also the entry Herdr resolves in the pane. On Windows the fixed
    // wrapper name supports npm .cmd without interpolating any task data in cmd.
    #[cfg(windows)]
    let mut command = {
        let mut c = Command::new("cmd.exe");
        c.args(["/d", "/c", "codex", "debug", "models"]);
        c
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut c = Command::new("codex");
        c.args(["debug", "models"]);
        c
    };
    command.current_dir(cwd);
    let bytes = crate::title_process::run_with_limit(
        command,
        vec![],
        &AtomicBool::new(false),
        Duration::from_secs(15),
        8 * 1024 * 1024,
        false,
    )
    .map_err(|_| "model.catalog_unavailable")?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "model.catalog_invalid")?;
    let models = value["models"]
        .as_array()
        .ok_or("model.catalog_invalid")?
        .iter()
        .filter(|m| m["visibility"] == "list")
        .filter_map(|m| {
            let id = m["slug"].as_str()?.to_string();
            let name = m["display_name"].as_str().unwrap_or(&id).to_string();
            let efforts = m["supported_reasoning_levels"]
                .as_array()?
                .iter()
                .filter_map(|e| e["effort"].as_str().map(str::to_string))
                .collect();
            Some(Model { id, name, efforts })
        })
        .collect::<Vec<_>>();
    if models.is_empty() {
        return Err("model.catalog_invalid".into());
    }
    Ok(Catalog {
        models,
        source: "cli_catalog".into(),
        cached: false,
        error: None,
    })
}

pub fn cached(db: &Connection, machine: &str, cwd: &str) -> Result<Option<Catalog>, String> {
    let json: Option<String> = db
        .query_row(
            "SELECT catalog FROM codex_catalog_cache WHERE machine_id=?1 AND cwd=?2",
            params![machine, cwd],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    json.map(|j| serde_json::from_str(&j).map_err(|_| "model.catalog_invalid".into()))
        .transpose()
}

impl crate::workflow::Workflow {
    pub fn model_catalog(&self, project: &str, refresh: bool) -> Result<Catalog, String> {
        let (machine, cwd): (String, String) = self
            .db
            .lock()
            .map_err(|e| e.to_string())?
            .query_row(
                "SELECT machine_id,root_path FROM projects WHERE id=?1",
                [project],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| "workspace.missing")?;
        let cache = cached(&*self.db.lock().map_err(|e| e.to_string())?, &machine, &cwd)?;
        if !refresh {
            if let Some(mut cached) = cache.clone() {
                cached.cached = true;
                return Ok(cached);
            }
        }
        let result = self
            .for_project(project)
            .and_then(|app| app.herdr.codex_catalog(&cwd));
        match result {
            Ok(catalog) => {
                let json = serde_json::to_string(&catalog).map_err(|e| e.to_string())?;
                self.db.lock().map_err(|e| e.to_string())?.execute("INSERT INTO codex_catalog_cache(machine_id,cwd,catalog) VALUES(?1,?2,?3) ON CONFLICT(machine_id,cwd) DO UPDATE SET catalog=excluded.catalog", params![machine,cwd,json]).map_err(|e| e.to_string())?;
                Ok(catalog)
            }
            Err(error) => {
                if let Some(mut catalog) = cache {
                    catalog.cached = true;
                    catalog.error = Some(error);
                    Ok(catalog)
                } else {
                    Err(error)
                }
            }
        }
    }
}
