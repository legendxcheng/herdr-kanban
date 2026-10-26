use super::*;

#[derive(serde::Serialize)]
pub struct WorkspaceCandidate {
    #[serde(flatten)]
    pub workspace: Workspace,
    pub project_id: Option<String>,
    pub shown: bool,
    pub blocked: bool,
}
#[derive(serde::Serialize)]
pub struct WorkspaceChoices {
    pub machine_id: String,
    pub session: String,
    pub workspaces: Vec<WorkspaceCandidate>,
}

impl ProjectService {
    pub fn choices(&self) -> Result<WorkspaceChoices, String> {
        let _gate = self.gate.lock().map_err(err)?;
        let snapshot = self.store_snapshot(self.herdr.workspaces()?)?;
        let db = self.db.lock().map_err(err)?;
        let projects = list(&db)?;
        let selected = crate::project_visibility::selected(&db)?;
        Ok(WorkspaceChoices {
            machine_id: snapshot.machine_id,
            session: snapshot.session.clone(),
            workspaces: snapshot
                .workspaces
                .into_iter()
                .map(|workspace| {
                    let project = projects.iter().find(|p| {
                        p.machine_id == self.herdr.machine_id()
                            && p.herdr_session.as_deref() == Some(&snapshot.session)
                            && p.workspace_id.as_deref() == Some(&workspace.id)
                    });
                    let blocked = snapshot.operations.iter().any(|op| {
                        op.state != "failed"
                            && (op.workspace_id.as_deref() == Some(&workspace.id)
                                || workspace.name == format!("kanban-create-{}", op.id))
                    });
                    WorkspaceCandidate {
                        shown: project.is_some_and(|p| selected.contains(&p.id)),
                        project_id: project.map(|p| p.id.clone()),
                        workspace,
                        blocked,
                    }
                })
                .collect(),
        })
    }

    pub fn add_existing(&self, session: &str, ids: &[String]) -> Result<(), String> {
        let _gate = self.gate.lock().map_err(err)?;
        if session != self.herdr.session() {
            return Err("workspace.session_mismatch".into());
        }
        if ids.is_empty() || ids.len() > 1000 {
            return Err("workspace.invalid_selection".into());
        }
        let live = self.herdr.workspaces()?;
        // Validate the entire request before any visibility changes. No SSH under DB lock.
        if ids.iter().any(|id| !live.iter().any(|w| &w.id == id)) {
            return Err("workspace.missing".into());
        }
        let mut db = self.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        let pending = operations(&tx, self.herdr.machine_id(), session)?;
        for (order, workspace) in live.iter().enumerate().filter(|(_, w)| ids.contains(&w.id)) {
            if pending.iter().any(|op| {
                op.state != "failed"
                    && (op.workspace_id.as_deref() == Some(&workspace.id)
                        || workspace.name == format!("kanban-create-{}", op.id))
            }) {
                return Err("workspace.verify_required".into());
            }
            let existing = list(&tx)?.into_iter().find(|p| {
                p.machine_id == self.herdr.machine_id()
                    && p.herdr_session.as_deref() == Some(session)
                    && p.workspace_id.as_deref() == Some(&workspace.id)
            });
            let (root, source) = directory_state(existing.as_ref(), workspace.directory.as_deref());
            let id = existing
                .map(|p| p.id)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            tx.execute("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at,herdr_session,workspace_id,sync_status,directory_source,herdr_directory,workspace_order,machine_id)
                VALUES(?1,?2,?3,?2,?4,?5,?6,'ready',?7,?8,?9,?10)
                ON CONFLICT(id) DO UPDATE SET name=excluded.name,herdr_workspace=excluded.herdr_workspace,root_path=excluded.root_path,sync_status='ready',directory_source=excluded.directory_source,herdr_directory=excluded.herdr_directory,workspace_order=excluded.workspace_order",
                params![id,workspace.name,root,chrono::Utc::now().to_rfc3339(),session,workspace.id,source,workspace.directory,order as i64,self.herdr.machine_id()]).map_err(err)?;
            tx.execute("DELETE FROM workspace_exclusions WHERE machine_id=?1 AND session=?2 AND workspace_id=?3", params![self.herdr.machine_id(),session,workspace.id]).map_err(err)?;
            crate::project_visibility::show(&tx, &id, true)?;
        }
        tx.commit().map_err(err)
    }
}
