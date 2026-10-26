//! Resolve ownership under a short database lock, then freeze the adapter for the operation.
use crate::{
    machines,
    workflow::{Herdr, Workflow},
};
use std::sync::Arc;

pub fn adapter(
    db: &Arc<std::sync::Mutex<rusqlite::Connection>>,
    current: &Arc<dyn Herdr>,
    id: &str,
) -> Result<Arc<dyn Herdr>, String> {
    if id == current.machine_id() {
        return Ok(current.clone());
    }
    let machine = {
        let conn = db.lock().map_err(|e| e.to_string())?;
        machines::get(&conn, id)?
    };
    if machine.disabled {
        return Err("machine.disabled".into());
    }
    let mut cli = crate::herdr::Cli::from_env();
    if machine.kind == "ssh" {
        let machine = crate::remote_install::ensure_current(db, &machine)?;
        crate::remote_transport::verify_profile(&machine)?;
        let _: serde_json::Value =
            crate::remote_transport::request(&machine, serde_json::json!({"method":"hello"}))?;
        cli.session = machine.session.clone();
        cli.remote = Some(machine);
    }
    Ok(Arc::new(cli))
}

impl Workflow {
    pub fn for_project(&self, project: &str) -> Result<Arc<Self>, String> {
        let id = {
            let db = self.db.lock().map_err(|e| e.to_string())?;
            machines::project_machine(&db, project)?
        };
        Ok(Arc::new(Self {
            db: self.db.clone(),
            herdr: adapter(&self.db, &self.herdr, &id)?,
        }))
    }
    pub fn for_task(&self, task: &str) -> Result<Arc<Self>, String> {
        let project: String = self
            .db
            .lock()
            .map_err(|e| e.to_string())?
            .query_row("SELECT project_id FROM tasks WHERE id=?1", [task], |r| {
                r.get(0)
            })
            .map_err(|_| "task.missing")?;
        self.for_project(&project)
    }
    pub fn for_optional_project(&self, project: Option<&str>) -> Result<Arc<Self>, String> {
        match project {
            Some(project) => self.for_project(project),
            None => Ok(Arc::new(Self {
                db: self.db.clone(),
                herdr: adapter(&self.db, &self.herdr, machines::LOCAL_ID)?,
            })),
        }
    }
}
