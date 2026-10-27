//! Project/workspace application boundary. All external mutations are recoverable.
use crate::{models::Project, workflow::Herdr};
use rusqlite::{Connection, OptionalExtension, params};
use std::sync::{Arc, Mutex};
#[path = "project_selection.rs"]
mod selection;
pub use selection::WorkspaceChoices;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub directory: Option<String>,
}
#[derive(serde::Serialize)]
pub struct Snapshot {
    pub machine_id: String,
    pub session: String,
    pub projects: Vec<Project>,
    pub workspaces: Vec<Workspace>,
    pub operations: Vec<CreateOperation>,
}
#[derive(Clone, serde::Serialize)]
pub struct CreateOperation {
    pub machine_id: String,
    pub id: String,
    pub session: String,
    pub name: String,
    pub cwd: String,
    pub workspace_id: Option<String>,
    pub project_id: String,
    pub state: String,
    pub error: String,
}
pub struct ProjectService {
    pub db: Arc<Mutex<Connection>>,
    herdr: Arc<dyn Herdr>,
    gate: Arc<Mutex<()>>,
    gates: Arc<Mutex<std::collections::HashMap<String, Arc<Mutex<()>>>>>,
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub fn list(conn: &Connection) -> Result<Vec<Project>, String> {
    let mut stmt = conn.prepare("SELECT id,name,root_path,herdr_workspace,cs_task_id,created_at,herdr_session,workspace_id,sync_status,directory_source,herdr_directory,machine_id FROM projects ORDER BY workspace_order IS NULL,workspace_order,created_at,id").map_err(err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Project {
                machine_id: r.get(11)?,
                id: r.get(0)?,
                name: r.get(1)?,
                root_path: r.get(2)?,
                herdr_workspace: r.get(3)?,
                cs_task_id: r.get(4)?,
                created_at: r.get(5)?,
                herdr_session: r.get(6)?,
                workspace_id: r.get(7)?,
                sync_status: r.get(8)?,
                directory_source: r.get(9)?,
                herdr_directory: r.get(10)?,
            })
        })
        .map_err(err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(err)
}
/// Navigation hides confirmed missing identities; recovery still uses the full list.
pub fn visible_list(conn: &Connection) -> Result<Vec<Project>, String> {
    let selected = crate::project_visibility::selected(conn)?;
    Ok(list(conn)?
        .into_iter()
        .filter(|p| {
            selected.contains(&p.id) && (p.workspace_id.is_none() || p.sync_status != "missing")
        })
        .collect())
}
impl ProjectService {
    fn valid_fallback(&self, path: Option<&str>) -> Result<bool, String> {
        match path.filter(|p| !p.is_empty()) {
            Some(path) => self.herdr.resources().directory_exists(path),
            None => Ok(true),
        }
    }

    pub fn new(db: Arc<Mutex<Connection>>, herdr: Arc<dyn Herdr>) -> Self {
        let gate = Arc::new(Mutex::new(()));
        let gates = Arc::new(Mutex::new(std::collections::HashMap::from([(
            herdr.machine_id().to_string(),
            gate.clone(),
        )])));
        Self {
            db,
            herdr,
            gate,
            gates,
        }
    }
    pub fn for_machine(&self, id: &str) -> Result<Self, String> {
        let herdr = crate::machine_routing::adapter(&self.db, &self.herdr, id)?;
        let gate = self.machine_gate(id)?;
        Ok(Self {
            db: self.db.clone(),
            herdr,
            gate,
            gates: self.gates.clone(),
        })
    }
    fn machine_gate(&self, id: &str) -> Result<Arc<Mutex<()>>, String> {
        Ok(self
            .gates
            .lock()
            .map_err(err)?
            .entry(id.into())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone())
    }
    pub fn for_project(&self, id: &str) -> Result<Self, String> {
        let machine = {
            let db = self.db.lock().map_err(err)?;
            crate::machines::project_machine(&db, id)?
        };
        self.for_machine(&machine)
    }
    pub fn for_operation(&self, id: &str) -> Result<Self, String> {
        let machine: String = self
            .db
            .lock()
            .map_err(err)?
            .query_row(
                "SELECT machine_id FROM workspace_operations WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(err)?;
        self.for_machine(&machine)
    }
    pub fn sync(&self) -> Result<Snapshot, String> {
        let _gate = self.gate.lock().map_err(err)?;
        let workspaces = self.herdr.workspaces()?;
        self.store_snapshot(workspaces)
    }
    /// Move one stable identity before another, leaving unlisted/excluded workspaces intact.
    pub fn move_project(&self, id: &str, before: Option<&str>) -> Result<Snapshot, String> {
        let _gate = self.gate.lock().map_err(err)?;
        let project = self.get(id)?;
        let workspace = identity_on(&project, self.herdr.machine_id(), self.herdr.session())?;
        let anchor = before
            .map(|id| {
                let p = self.get(id)?;
                identity_on(&p, self.herdr.machine_id(), self.herdr.session())
            })
            .transpose()?;
        if anchor.as_deref() == Some(&workspace) {
            return Err("workspace.invalid_input".into());
        }
        let live = self.herdr.workspaces()?;
        if !live.iter().any(|w| w.id == workspace)
            || anchor
                .as_ref()
                .is_some_and(|id| !live.iter().any(|w| &w.id == id))
        {
            self.store_snapshot(live)?;
            return Err("workspace.missing".into());
        }
        match self.herdr.move_workspace(&workspace, anchor.as_deref()) {
            Ok(workspaces) => self.store_snapshot(workspaces),
            Err(error) => {
                // A timeout may follow a successful move. Read actual state, never resend.
                if let Ok(workspaces) = self.herdr.workspaces() {
                    let _ = self.store_snapshot(workspaces);
                }
                Err(error)
            }
        }
    }
    fn store_snapshot(&self, workspaces: Vec<Workspace>) -> Result<Snapshot, String> {
        let mut db = self.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        let existing: Vec<_> = list(&tx)?
            .into_iter()
            .filter(|p| p.machine_id == self.herdr.machine_id())
            .collect();
        let pending = operations(&tx, self.herdr.machine_id(), self.herdr.session())?;
        tx.execute("UPDATE projects SET sync_status='missing' WHERE herdr_session=?1 AND machine_id=?2 AND workspace_id IS NOT NULL",params![self.herdr.session(),self.herdr.machine_id()]).map_err(err)?;
        for (order, ws) in workspaces.iter().enumerate() {
            // Presence is independent of whether a pending creation can be imported.
            tx.execute("UPDATE projects SET sync_status='ready',workspace_order=?3 WHERE herdr_session=?1 AND workspace_id=?2 AND machine_id=?4", params![self.herdr.session(),ws.id,order as i64,self.herdr.machine_id()]).map_err(err)?;
            if pending.iter().any(|op| {
                op.workspace_id.as_deref() == Some(&ws.id)
                    || ws.name == format!("kanban-create-{}", op.id)
            }) {
                continue;
            }
            let excluded:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM workspace_exclusions WHERE session=?1 AND workspace_id=?2 AND machine_id=?3)",params![self.herdr.session(),ws.id,self.herdr.machine_id()],|r|r.get(0)).map_err(err)?;
            if excluded {
                continue;
            }
            let mut candidates = Vec::new();
            for p in existing.iter().filter(|p| p.workspace_id.is_none()) {
                let targets = legacy_targets(&tx, &p.id)?;
                let identity_match = targets.as_ref().is_some_and(|items| {
                    items.iter().any(|t| {
                        t.workspace_id == ws.id
                            && t.machine_id == self.herdr.machine_id()
                            && t.session == self.herdr.session()
                    })
                });
                if p.herdr_workspace == ws.name || identity_match {
                    candidates.push((p, targets));
                }
            }
            if !existing.iter().any(|p| {
                p.machine_id == self.herdr.machine_id()
                    && p.herdr_session.as_deref() == Some(self.herdr.session())
                    && p.workspace_id.as_deref() == Some(&ws.id)
            }) && !candidates.is_empty()
            {
                if candidates.len() != 1 {
                    continue;
                }
                let (candidate, targets) = &candidates[0];
                let proven = targets.as_ref().is_some_and(|items| {
                    if items.is_empty() {
                        workspaces.iter().filter(|w| w.name == ws.name).count() == 1
                            && ws
                                .directory
                                .as_ref()
                                .is_some_and(|d| same_directory(d, &candidate.root_path))
                    } else {
                        items.iter().all(|t| {
                            t.machine_id == self.herdr.machine_id()
                                && t.session == self.herdr.session()
                                && t.workspace_id == ws.id
                        })
                    }
                });
                if !proven {
                    continue;
                }
                tx.execute("UPDATE projects SET herdr_session=?2,workspace_id=?3,sync_status='ready' WHERE id=?1", params![candidate.id,self.herdr.session(),ws.id]).map_err(err)?;
            }
            let current = list(&tx)?.into_iter().find(|p| {
                p.machine_id == self.herdr.machine_id()
                    && p.herdr_session.as_deref() == Some(self.herdr.session())
                    && p.workspace_id.as_deref() == Some(&ws.id)
            });
            let (root, source) = directory_state(current.as_ref(), ws.directory.as_deref());
            tx.execute("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at,herdr_session,workspace_id,sync_status,directory_source,herdr_directory,workspace_order,machine_id)
                VALUES (?1,?2,?3,?2,?4,?5,?6,'ready',?7,?8,?9,?10)
                ON CONFLICT DO UPDATE SET name=excluded.name,herdr_workspace=excluded.herdr_workspace,
                sync_status='ready',root_path=excluded.root_path,directory_source=excluded.directory_source,herdr_directory=excluded.herdr_directory,workspace_order=excluded.workspace_order",
                params![uuid::Uuid::new_v4().to_string(),ws.name,root,chrono::Utc::now().to_rfc3339(),self.herdr.session(),ws.id,source,ws.directory,order as i64,self.herdr.machine_id()]).map_err(err)?;
        }
        tx.commit().map_err(err)?;
        Ok(Snapshot {
            machine_id: self.herdr.machine_id().into(),
            session: self.herdr.session().into(),
            projects: visible_list(&db)?
                .into_iter()
                .filter(|p| p.machine_id == self.herdr.machine_id())
                .collect(),
            workspaces,
            operations: operations(&db, self.herdr.machine_id(), self.herdr.session())?,
        })
    }
    pub fn operations(&self) -> Result<Vec<CreateOperation>, String> {
        operations(
            &*self.db.lock().map_err(err)?,
            self.herdr.machine_id(),
            self.herdr.session(),
        )
    }
    pub fn update(&self, id: &str, name: &str, fallback: Option<&str>) -> Result<Project, String> {
        let _gate = self.gate.lock().map_err(err)?;
        if name.trim().is_empty() {
            return Err("workspace.invalid_input".into());
        }
        if !self.valid_fallback(fallback)? {
            return Err("workspace.invalid_input".into());
        }
        let p = self.get(id)?;
        let wid = identity_on(&p, self.herdr.machine_id(), self.herdr.session())?;
        // Name changes do not alter task targets, and need not block running work.
        if p.name != name.trim() {
            self.herdr.rename_workspace(&wid, name.trim())?;
        }
        let actual = self
            .herdr
            .workspaces()?
            .into_iter()
            .find(|w| w.id == wid)
            .ok_or("workspace.missing")?;
        if actual.name != name.trim() {
            return Err("workspace.verify_required".into());
        }
        let (root, source) = match fallback {
            Some("") => (
                actual
                    .directory
                    .clone()
                    .ok_or("workspace.directory_missing")?,
                "herdr",
            ),
            Some(d) => (
                d.into(),
                if actual
                    .directory
                    .as_ref()
                    .is_some_and(|root| same_directory(root, d))
                {
                    "herdr"
                } else if actual.directory.is_some() {
                    "conflict"
                } else {
                    "fallback"
                },
            ),
            None => (p.root_path, p.directory_source.as_str()),
        };
        self.db.lock().map_err(err)?.execute("UPDATE projects SET name=?2,herdr_workspace=?2,root_path=?3,directory_source=?4,herdr_directory=?5,sync_status='ready' WHERE id=?1",params![id,actual.name,root,source,actual.directory]).map_err(err)?;
        self.get(id)
    }
    pub fn get(&self, id: &str) -> Result<Project, String> {
        list(&*self.db.lock().map_err(err)?)?
            .into_iter()
            .find(|p| p.id == id && p.machine_id == self.herdr.machine_id())
            .ok_or("workspace.project_missing".into())
    }
    pub fn link(
        &self,
        project: Option<&str>,
        workspace: &str,
        fallback: Option<&str>,
    ) -> Result<Project, String> {
        let _gate = self.gate.lock().map_err(err)?;
        if !self.valid_fallback(fallback)? {
            return Err("workspace.invalid_input".into());
        }
        let ws = self
            .herdr
            .workspaces()?
            .into_iter()
            .find(|w| w.id == workspace)
            .ok_or("workspace.missing")?;
        let mut db = self.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        if operations(&tx, self.herdr.machine_id(), self.herdr.session())?
            .iter()
            .any(|o| {
                o.state != "failed"
                    && (o.workspace_id.as_deref() == Some(workspace)
                        || ws.name == format!("kanban-create-{}", o.id))
            })
        {
            return Err("workspace.verify_required".into());
        }
        let id = project
            .map(str::to_owned)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        if project.is_some() {
            ensure_editable(&tx, &id, true)?;
            if crate::machines::project_machine(&tx, &id)? != self.herdr.machine_id() {
                return Err("machine.identity_changed".into());
            }
        }
        if list(&tx)?.iter().any(|p| {
            p.id != id
                && p.machine_id == self.herdr.machine_id()
                && p.herdr_session.as_deref() == Some(self.herdr.session())
                && p.workspace_id.as_deref() == Some(workspace)
        }) {
            return Err("workspace.occupied".into());
        }
        let root = ws.directory.as_deref().or(fallback).unwrap_or("");
        let source = if ws.directory.is_some() {
            "herdr"
        } else if root.is_empty() {
            "missing"
        } else {
            "fallback"
        };
        if project.is_some() {
            tx.execute("UPDATE projects SET name=?2,herdr_workspace=?2,herdr_session=?3,workspace_id=?4,root_path=?5,herdr_directory=?6,directory_source=?7,sync_status='ready' WHERE id=?1",params![id,ws.name,self.herdr.session(),workspace,root,ws.directory,source]).map_err(err)?;
        } else {
            tx.execute("INSERT INTO projects(id,name,herdr_workspace,herdr_session,workspace_id,root_path,herdr_directory,directory_source,sync_status,created_at,machine_id) VALUES (?1,?2,?2,?3,?4,?5,?6,?7,'ready',?8,?9)",params![id,ws.name,self.herdr.session(),workspace,root,ws.directory,source,chrono::Utc::now().to_rfc3339(),self.herdr.machine_id()]).map_err(err)?;
        }
        tx.execute(
            "DELETE FROM workspace_exclusions WHERE session=?1 AND workspace_id=?2 AND machine_id=?3",
            params![self.herdr.session(), workspace, self.herdr.machine_id()],
        )
        .map_err(err)?;
        crate::project_visibility::show(&tx, &id, true)?;
        tx.commit().map_err(err)?;
        drop(db);
        self.get(&id)
    }
    pub fn remove(&self, id: &str) -> Result<Vec<String>, String> {
        let _gate = self.gate.lock().map_err(err)?;
        let mut db = self.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        ensure_editable(&tx, id, false)?;
        let p = list(&tx)?
            .into_iter()
            .find(|p| p.id == id)
            .ok_or("workspace.project_missing")?;
        if let (Some(session), Some(wid)) = (p.herdr_session, p.workspace_id) {
            tx.execute(
                "INSERT OR IGNORE INTO workspace_exclusions(session,workspace_id,machine_id) VALUES (?1,?2,?3)",
                params![session, wid, p.machine_id],
            )
            .map_err(err)?;
        }
        let tasks = crate::tasks::list(&tx, id)?
            .into_iter()
            .map(|t| t.id)
            .collect();
        tx.execute("DELETE FROM projects WHERE id=?1", [id])
            .map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(tasks)
    }
    pub fn create(&self, id: &str, name: &str, cwd: &str) -> Result<Project, String> {
        let _gate = self.gate.lock().map_err(err)?;
        self.ensure_operation_active(id)?;
        uuid::Uuid::parse_str(id).map_err(|_| "workspace.invalid_operation")?;
        if let Some(op) = self.operation(id)? {
            if op.state != "failed" && (op.name != name.trim() || op.cwd != cwd) {
                return Err("workspace.operation_mismatch".into());
            }
            if op.state != "failed" {
                return self.recover_inner(op, None);
            }
            self.db
                .lock()
                .map_err(err)?
                .execute(
                    "DELETE FROM workspace_operations WHERE id=?1 AND state='failed'",
                    [id],
                )
                .map_err(err)?;
        }
        if name.trim().is_empty() || !self.herdr.resources().directory_exists(cwd)? {
            return Err("workspace.invalid_input".into());
        }
        self.db.lock().map_err(err)?.execute("INSERT INTO workspace_operations(id,session,name,cwd,project_id,state,created_at,machine_id) VALUES (?1,?2,?3,?4,?1,'pending',?5,?6)",params![id,self.herdr.session(),name.trim(),cwd,chrono::Utc::now().to_rfc3339(),self.herdr.machine_id()]).map_err(err)?;
        match self
            .herdr
            .create_workspace(&format!("kanban-create-{id}"), cwd)
        {
            Ok(ws) => {
                self.db.lock().map_err(err)?.execute("UPDATE workspace_operations SET workspace_id=?2,state='unknown' WHERE id=?1",params![id,ws.id]).map_err(err)?;
                self.recover_inner(
                    self.operation(id)?.ok_or("workspace.operation_missing")?,
                    None,
                )
            }
            Err(e) => {
                self.db
                    .lock()
                    .map_err(err)?
                    .execute(
                        "UPDATE workspace_operations SET state=?2,error=?3 WHERE id=?1",
                        params![
                            id,
                            if e.uncertain { "unknown" } else { "failed" },
                            e.message
                        ],
                    )
                    .map_err(err)?;
                Err(e.message)
            }
        }
    }
    pub fn recover(&self, id: &str, workspace_id: Option<&str>) -> Result<Project, String> {
        let _gate = self.gate.lock().map_err(err)?;
        self.ensure_operation_active(id)?;
        let op = self.operation(id)?.ok_or("workspace.operation_missing")?;
        self.recover_inner(op, workspace_id)
    }
    fn ensure_operation_active(&self, id: &str) -> Result<(), String> {
        let dismissed: bool = self
            .db
            .lock()
            .map_err(err)?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workspace_operation_dismissals WHERE operation_id=?1)",
                [id],
                |r| r.get(0),
            )
            .map_err(err)?;
        if dismissed {
            return Err("workspace.operation_dismissed".into());
        }
        Ok(())
    }

    /// Local-only: resolve the machine from the record without opening an adapter.
    /// Share the creation/recovery gate, then recheck the outcome under the DB lock.
    pub fn dismiss_operation(&self, id: &str) -> Result<(), String> {
        let machine: String = self
            .db
            .lock()
            .map_err(err)?
            .query_row(
                "SELECT machine_id FROM workspace_operations WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?
            .ok_or("workspace.operation_missing")?;
        let gate = self.machine_gate(&machine)?;
        let _gate = gate.lock().map_err(err)?;
        let mut db = self.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        let state: String = tx
            .query_row(
                "SELECT state FROM workspace_operations WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(err)?;
        if !matches!(state.as_str(), "failed" | "unknown") {
            return Err("workspace.dismiss_unavailable".into());
        }
        tx.execute("INSERT OR IGNORE INTO workspace_operation_dismissals(operation_id,dismissed_at) VALUES(?1,?2)",
            params![id, chrono::Utc::now().to_rfc3339()]).map_err(err)?;
        tx.commit().map_err(err)
    }

    fn operation(&self, id: &str) -> Result<Option<CreateOperation>, String> {
        let db = self.db.lock().map_err(err)?;
        db.query_row("SELECT id,session,name,cwd,workspace_id,project_id,state,error,machine_id FROM workspace_operations WHERE id=?1 AND session=?2 AND machine_id=?3",params![id,self.herdr.session(),self.herdr.machine_id()],operation_row).optional().map_err(err)
    }
    fn recover_inner(
        &self,
        mut op: CreateOperation,
        selected: Option<&str>,
    ) -> Result<Project, String> {
        if op.state == "succeeded" {
            return list(&*self.db.lock().map_err(err)?)?
                .into_iter()
                .find(|p| p.id == op.project_id)
                .ok_or("workspace.project_removed".into());
        }
        let spaces = self.herdr.workspaces()?;
        let marker = format!("kanban-create-{}", op.id);
        let matches: Vec<_> = spaces
            .iter()
            .filter(|w| match op.workspace_id.as_deref() {
                Some(id) => w.id == id,
                None => w.name == marker,
            })
            .collect();
        let ws = if matches.len() == 1 {
            matches[0]
        } else if op.workspace_id.is_none() {
            spaces
                .iter()
                .find(|w| Some(w.id.as_str()) == selected)
                .ok_or("workspace.verify_required")?
        } else {
            return Err("workspace.verify_required".into());
        };
        // Persist the exact identity before rename, so a lost rename response is readable on retry.
        {
            let db = self.db.lock().map_err(err)?;
            if operations(&db, self.herdr.machine_id(), self.herdr.session())?
                .iter()
                .any(|other| {
                    other.id != op.id
                        && (other.workspace_id.as_deref() == Some(&ws.id)
                            || ws.name == format!("kanban-create-{}", other.id))
                        && other.state != "failed"
                })
            {
                return Err("workspace.occupied".into());
            }
            if let Some(existing) = list(&db)?.into_iter().find(|p| {
                p.machine_id == op.machine_id
                    && p.herdr_session.as_deref() == Some(&op.session)
                    && p.workspace_id.as_deref() == Some(&ws.id)
                    && p.id != op.project_id
            }) {
                if selected != Some(ws.id.as_str()) {
                    return Err("workspace.occupied".into());
                }
                // Explicit recovery adopts the existing Project in place, preserving all tasks.
                op.project_id = existing.id;
            }
            db.execute(
                "UPDATE workspace_operations SET workspace_id=?2,project_id=?3,state='unknown' WHERE id=?1",
                params![op.id, ws.id,op.project_id],
            )
            .map_err(err)?;
        }
        if ws.name != op.name {
            self.herdr.rename_workspace(&ws.id, &op.name)?;
        }
        let actual = self
            .herdr
            .workspaces()?
            .into_iter()
            .find(|w| w.id == ws.id && w.name == op.name)
            .ok_or("workspace.verify_required")?;
        let mut db = self.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        tx.execute("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at,herdr_session,workspace_id,sync_status,directory_source,herdr_directory,machine_id) VALUES (?1,?2,?3,?2,?4,?5,?6,'ready',?7,?8,?9) ON CONFLICT(id) DO UPDATE SET name=excluded.name,herdr_workspace=excluded.herdr_workspace,sync_status='ready'",params![op.project_id,actual.name,actual.directory.as_deref().unwrap_or(&op.cwd),chrono::Utc::now().to_rfc3339(),op.session,actual.id,if actual.directory.is_some(){"herdr"}else{"fallback"},actual.directory,op.machine_id]).map_err(err)?;
        tx.execute(
            "UPDATE workspace_operations SET state='succeeded',error='' WHERE id=?1",
            [&op.id],
        )
        .map_err(err)?;
        crate::project_visibility::show(&tx, &op.project_id, false)?;
        tx.commit().map_err(err)?;
        list(&db)?
            .into_iter()
            .find(|p| p.id == op.project_id)
            .ok_or("workspace.project_missing".into())
    }
}
fn legacy_targets(
    db: &Connection,
    project: &str,
) -> Result<Option<Vec<crate::workflow::Target>>, String> {
    let mut stmt=db.prepare("SELECT b.target_json FROM task_bindings b JOIN tasks t ON t.id=b.task_id WHERE t.project_id=?1
        UNION ALL SELECT o.target_json FROM lifecycle_operations o JOIN tasks t ON t.id=o.task_id WHERE t.project_id=?1 AND o.state IN ('pending','unknown') AND o.target_json IS NOT NULL
        UNION ALL SELECT o.target_json FROM dispatch_operations o JOIN tasks t ON t.id=o.task_id WHERE t.project_id=?1 AND o.state IN ('pending','unknown')").map_err(err)?;
    let json = stmt
        .query_map([project], |r| r.get::<_, String>(0))
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    Ok(json
        .iter()
        .map(|s| serde_json::from_str(s))
        .collect::<Result<Vec<_>, _>>()
        .ok())
}
pub fn identity_on(p: &Project, machine: &str, session: &str) -> Result<String, String> {
    if p.machine_id != machine {
        return Err("machine.identity_changed".into());
    }
    if p.herdr_session.as_deref() != Some(session) {
        return Err("workspace.session_mismatch".into());
    }
    p.workspace_id.clone().ok_or("workspace.unlinked".into())
}
pub fn project_identity_on(
    db: &Connection,
    id: &str,
    machine: &str,
    session: &str,
) -> Result<String, String> {
    let (owner, workspace, machine_id): (Option<String>, Option<String>, String) = db
        .query_row(
            "SELECT herdr_session,workspace_id,machine_id FROM projects WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(err)?;
    if machine_id != machine {
        return Err("machine.identity_changed".into());
    }
    let workspace = workspace.ok_or("workspace.unlinked")?;
    if owner.as_deref() != Some(session) {
        return Err("workspace.session_mismatch".into());
    }
    Ok(workspace)
}
pub fn task_identity_on(
    db: &Connection,
    id: &str,
    machine: &str,
    session: &str,
) -> Result<String, String> {
    let project: String = db
        .query_row("SELECT project_id FROM tasks WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .map_err(err)?;
    project_identity_on(db, &project, machine, session)
}
fn ensure_editable(db: &Connection, id: &str, relink: bool) -> Result<(), String> {
    let active:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM tasks t WHERE t.project_id=?1 AND (
        EXISTS(SELECT 1 FROM lifecycle_operations o WHERE o.task_id=t.id AND o.state IN ('pending','unknown')) OR
        EXISTS(SELECT 1 FROM dispatch_operations o WHERE o.task_id=t.id AND o.state IN ('pending','unknown'))))",[id],|r|r.get(0)).map_err(err)?;
    if active {
        return Err("workspace.verify_required".into());
    }
    if relink {
        let bound:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM task_bindings b JOIN tasks t ON t.id=b.task_id WHERE t.project_id=?1)",[id],|r|r.get(0)).map_err(err)?;
        if bound {
            return Err("workspace.active_binding".into());
        }
        if !list(db)?.iter().any(|p| p.id == id) {
            return Err("workspace.project_missing".into());
        }
    }
    Ok(())
}
fn operation_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<CreateOperation> {
    Ok(CreateOperation {
        machine_id: r.get(8)?,
        id: r.get(0)?,
        session: r.get(1)?,
        name: r.get(2)?,
        cwd: r.get(3)?,
        workspace_id: r.get(4)?,
        project_id: r.get(5)?,
        state: r.get(6)?,
        error: r.get(7)?,
    })
}
pub(crate) fn operations(
    db: &Connection,
    machine: &str,
    session: &str,
) -> Result<Vec<CreateOperation>, String> {
    let mut stmt=db.prepare("SELECT id,session,name,cwd,workspace_id,project_id,state,error,machine_id FROM workspace_operations WHERE session=?1 AND machine_id=?2 AND state<>'succeeded' AND NOT EXISTS(SELECT 1 FROM workspace_operation_dismissals d WHERE d.operation_id=workspace_operations.id) ORDER BY created_at").map_err(err)?;
    let rows = stmt
        .query_map(params![session, machine], operation_row)
        .map_err(err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(err)
}
fn directory_state(current: Option<&Project>, remote: Option<&str>) -> (String, &'static str) {
    if let Some(p) = current.filter(|p| !p.root_path.is_empty()) {
        if remote.is_some_and(|d| same_directory(&p.root_path, d)) {
            return (remote.unwrap().into(), "herdr");
        }
        if matches!(p.directory_source.as_str(), "fallback" | "conflict") {
            return (
                p.root_path.clone(),
                if remote.is_some() {
                    "conflict"
                } else {
                    "fallback"
                },
            );
        }
        if remote.is_none() {
            return (p.root_path.clone(), "cached");
        }
    }
    (
        remote.unwrap_or("").into(),
        if remote.is_some() { "herdr" } else { "missing" },
    )
}
fn same_directory(a: &str, b: &str) -> bool {
    let normalize = |p: &str| p.replace('\\', "/").trim_end_matches('/').to_lowercase();
    normalize(a) == normalize(b)
}

#[cfg(test)]
#[path = "projects_tests.rs"]
mod tests;
