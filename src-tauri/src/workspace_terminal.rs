//! A durable, non-Agent shell owned by a Workspace. Viewing never sends shell input.
use crate::workflow::Workflow;
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Target {
    pub machine_id: String,
    pub session: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub terminal_id: String,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub fn init(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS workspace_terminals (
        machine_id TEXT NOT NULL, session TEXT NOT NULL, workspace_id TEXT NOT NULL,
        operation_id TEXT NOT NULL, cwd TEXT NOT NULL, target_json TEXT,
        state TEXT NOT NULL CHECK(state IN ('creating','ready','failed')),
        PRIMARY KEY(machine_id,session,workspace_id));
        INSERT OR IGNORE INTO schema_migrations(version) VALUES(18);",
    )
}

/// A dedicated shell is not a task candidate, even if the user launches an Agent in it.
pub(crate) fn owns(db: &Connection, target: &crate::workflow::Target) -> Result<bool, String> {
    db.query_row("SELECT EXISTS(SELECT 1 FROM workspace_terminals WHERE machine_id=?1 AND session=?2
        AND (json_extract(target_json,'$.terminal_id')=?3 OR json_extract(target_json,'$.pane_id')=?4))",
        params![target.machine_id,target.session,target.terminal_id,target.pane_id], |r|r.get(0)).map_err(err)
}

impl Workflow {
    /// Caller has already routed the Workflow to the Project's execution computer.
    /// The gate covers discovery, durable intent, creation and receipt persistence.
    pub fn ensure_workspace_terminal(
        &self,
        project: &str,
        check: &dyn Fn() -> Result<(), String>,
    ) -> Result<Target, String> {
        let workspace = self.shell_project(project)?.0;
        let gate = crate::lifecycle::pane_allocation::workspace_lock(
            self.herdr.machine_id(),
            self.herdr.session(),
            &workspace,
        )?;
        let _guard = gate.lock().map_err(err)?;
        check()?;
        let (current, cwd) = self.shell_project(project)?;
        if current != workspace {
            return Err("binding.identity_changed".into());
        }
        let record = self.db.lock().map_err(err)?.query_row(
            "SELECT operation_id,target_json,state FROM workspace_terminals WHERE machine_id=?1 AND session=?2 AND workspace_id=?3",
            params![self.herdr.machine_id(),self.herdr.session(),workspace],
            |r| Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,String>(2)?))
        ).optional().map_err(err)?;
        if let Some((operation, saved, state)) = record {
            if let Some(saved) = saved {
                let target: Target = serde_json::from_str(&saved).map_err(err)?;
                self.check_shell_scope(project, &target)?;
                // Full-session inventory distinguishes a moved terminal from a closed one.
                if self.herdr.workspace_terminal_live(&target)? {
                    check()?;
                    self.check_shell_scope(project, &target)?;
                    return Ok(target);
                }
            } else if state == "creating" {
                // A lost response is not permission to replay creation, even if a read
                // sees no pane yet: the original remote request may still be running.
                let target = self
                    .herdr
                    .find_workspace_terminal(&workspace, &operation)?
                    .ok_or("workspace_terminal.unknown")?;
                self.save_shell(project, &operation, &target)?;
                check()?;
                return Ok(target);
            }
        }
        if !self.herdr.resources().directory_exists(&cwd)? {
            return Err("flow.directory_missing".into());
        }
        check()?;
        if self.shell_project(project)? != (workspace.clone(), cwd.clone()) {
            return Err("binding.identity_changed".into());
        }
        let operation = uuid::Uuid::new_v4().to_string();
        self.db.lock().map_err(err)?.execute(
            "INSERT INTO workspace_terminals(machine_id,session,workspace_id,operation_id,cwd,state) VALUES(?1,?2,?3,?4,?5,'creating')
             ON CONFLICT(machine_id,session,workspace_id) DO UPDATE SET operation_id=excluded.operation_id,cwd=excluded.cwd,target_json=NULL,state='creating'",
            params![self.herdr.machine_id(),self.herdr.session(),workspace,operation,cwd]).map_err(err)?;
        let still_current = || {
            check()?;
            if self.shell_project(project)? != (workspace.clone(), cwd.clone()) {
                return Err("binding.identity_changed".into());
            }
            Ok(())
        };
        match self
            .herdr
            .create_workspace_terminal(&workspace, &cwd, &operation, &still_current)
        {
            Ok(target) => {
                // Always persist the receipt, including when the view was switched meanwhile.
                self.save_shell(project, &operation, &target)?;
                check()?;
                Ok(target)
            }
            Err(error) => {
                if !error.uncertain {
                    self.db
                        .lock()
                        .map_err(err)?
                        .execute(
                            "UPDATE workspace_terminals SET state='failed' WHERE operation_id=?1",
                            [&operation],
                        )
                        .map_err(err)?;
                }
                Err(error.message)
            }
        }
    }

    fn shell_project(&self, project: &str) -> Result<(String, String), String> {
        let db = self.db.lock().map_err(err)?;
        crate::machines::ensure_enabled(&db, self.herdr.machine_id())?;
        let workspace = crate::projects::project_identity_on(
            &db,
            project,
            self.herdr.machine_id(),
            self.herdr.session(),
        )?;
        let cwd = db
            .query_row(
                "SELECT root_path FROM projects WHERE id=?1",
                [project],
                |r| r.get(0),
            )
            .map_err(err)?;
        Ok((workspace, cwd))
    }

    pub fn check_shell_scope(&self, project: &str, target: &Target) -> Result<(), String> {
        if target.machine_id != self.herdr.machine_id()
            || target.session != self.herdr.session()
            || self.shell_project(project)?.0 != target.workspace_id
        {
            return Err("binding.identity_changed".into());
        }
        Ok(())
    }

    pub fn saved_workspace_terminal(&self, project: &str) -> Result<Target, String> {
        let (workspace, _) = self.shell_project(project)?;
        let json: String = self.db.lock().map_err(err)?.query_row(
            "SELECT target_json FROM workspace_terminals WHERE machine_id=?1 AND session=?2 AND workspace_id=?3 AND state='ready'",
            params![self.herdr.machine_id(), self.herdr.session(), workspace], |r| r.get(0)).map_err(err)?;
        let target = serde_json::from_str(&json).map_err(err)?;
        self.check_shell_scope(project, &target)?;
        Ok(target)
    }

    fn save_shell(&self, project: &str, operation: &str, target: &Target) -> Result<(), String> {
        self.check_shell_scope(project, target)?;
        let changed = self.db.lock().map_err(err)?.execute(
            "UPDATE workspace_terminals SET target_json=?2,state='ready' WHERE operation_id=?1 AND machine_id=?3 AND session=?4 AND workspace_id=?5",
            params![operation,serde_json::to_string(target).map_err(err)?,target.machine_id,target.session,target.workspace_id]).map_err(err)?;
        if changed != 1 {
            return Err("binding.identity_changed".into());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "workspace_terminal_tests.rs"]
mod tests;
