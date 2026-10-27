//! Confirmed, identity-pinned cleanup of idle panes in one Workspace's kanban Tab.
use crate::workflow::{Target, Workflow};
use rusqlite::{Connection, params};

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub fn init(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS idle_cleanup_batches(
        id TEXT PRIMARY KEY, project_id TEXT NOT NULL, machine_id TEXT NOT NULL,
        session TEXT NOT NULL, workspace TEXT NOT NULL, state TEXT NOT NULL, created_at TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS idle_cleanup_items(
        batch_id TEXT NOT NULL REFERENCES idle_cleanup_batches(id), ordinal INTEGER NOT NULL,
        candidate_json TEXT NOT NULL, state TEXT NOT NULL, error TEXT NOT NULL DEFAULT '',
        PRIMARY KEY(batch_id,ordinal));")
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct IdlePane {
    pub target: Target,
    pub tab_id: String,
    pub cwd: String,
}
#[derive(Debug, serde::Serialize)]
pub struct Preview {
    pub id: String,
    pub count: usize,
    pub unresolved: Vec<String>,
}
#[derive(Debug, serde::Serialize)]
pub struct Report {
    pub id: String,
    pub closed: usize,
    pub skipped: usize,
    pub failed: usize,
    pub items: Vec<ItemResult>,
}
#[derive(Debug, serde::Serialize)]
pub struct ItemResult {
    pub pane_id: String,
    pub state: String,
    pub error: String,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct Owner {
    task: String,
    binding: String,
    cwd: String,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct Candidate {
    pane: IdlePane,
    owner: Option<Owner>,
}

pub(crate) fn ensure_unclaimed(
    db: &Connection,
    target: &Target,
    except: Option<&str>,
) -> Result<(), String> {
    let busy:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM idle_cleanup_items i JOIN idle_cleanup_batches b ON b.id=i.batch_id
        WHERE i.state IN ('claimed','closing','unknown') AND b.machine_id=?1 AND b.session=?2
        AND (json_extract(i.candidate_json,'$.pane.target.terminal_id')=?3 OR json_extract(i.candidate_json,'$.pane.target.pane_id')=?4)
        AND (?5 IS NULL OR b.id<>?5))",params![target.machine_id,target.session,target.terminal_id,target.pane_id,except],|r|r.get(0)).map_err(err)?;
    if busy {
        Err("cleanup.pending".into())
    } else {
        Ok(())
    }
}

pub(crate) fn ensure_task_unclaimed(db: &Connection, task: &str) -> Result<(), String> {
    // The cleanup snapshot owns the task guard. Do not parse legacy binding
    // payloads just to check whether an unrelated task mutation is permitted.
    let busy: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM idle_cleanup_items WHERE state IN ('claimed','closing','unknown') AND json_extract(candidate_json,'$.owner.task')=?1)", [task], |r| r.get(0)).map_err(err)?;
    if busy {
        Err("cleanup.pending".into())
    } else {
        Ok(())
    }
}
pub(crate) fn recover(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("UPDATE idle_cleanup_items SET state='unknown',error='cleanup.unknown' WHERE state='closing';
        UPDATE idle_cleanup_items SET state='failed',error='cleanup.interrupted' WHERE state='claimed';
        UPDATE idle_cleanup_items SET state='skipped',error='cleanup.interrupted' WHERE state='ready' AND batch_id IN (SELECT id FROM idle_cleanup_batches WHERE state='running');
        UPDATE idle_cleanup_batches SET state='done' WHERE state='running';")
}

fn eligible_owner(db: &Connection, pane: &IdlePane) -> Result<Option<Owner>, String> {
    let t = &pane.target;
    if crate::lifecycle::pane_allocation::quarantines(db, t)? {
        return Err("cleanup.protected".into());
    }
    let unresolved: bool = db.query_row("SELECT EXISTS(
        SELECT 1 FROM lifecycle_operations WHERE machine_id=?1 AND state IN ('pending','unknown')
        AND json_extract(target_json,'$.session')=?2 AND (json_extract(target_json,'$.terminal_id')=?3 OR json_extract(target_json,'$.pane_id')=?4)
        UNION ALL SELECT 1 FROM dispatch_operations WHERE machine_id=?1 AND state IN ('pending','unknown')
        AND json_extract(target_json,'$.session')=?2 AND (json_extract(target_json,'$.terminal_id')=?3 OR json_extract(target_json,'$.pane_id')=?4))",
        params![t.machine_id,t.session,t.terminal_id,t.pane_id], |r|r.get(0)).map_err(err)?;
    if unresolved {
        return Err("cleanup.protected".into());
    }
    let mut stmt = db.prepare("SELECT b.task_id,b.target_json,b.state,t.status,COALESCE(w.cwd,s.cwd,p.root_path)
        FROM task_bindings b JOIN tasks t ON t.id=b.task_id JOIN projects p ON p.id=t.project_id
        LEFT JOIN task_working_directories w ON w.task_id=t.id LEFT JOIN task_sessions s ON s.task_id=t.id
        WHERE b.machine_id=?1 AND b.session=?2 AND (b.pane_id=?3 OR json_extract(b.target_json,'$.terminal_id')=?4)").map_err(err)?;
    let rows = stmt
        .query_map(
            params![t.machine_id, t.session, t.pane_id, t.terminal_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            },
        )
        .map_err(err)?;
    let mut result = None;
    for row in rows {
        let (task, binding, state, status, cwd) = row.map_err(err)?;
        let bound: Target = serde_json::from_str(&binding).map_err(err)?;
        if result.is_some()
            || status == "in_progress"
            || state != "valid"
            || !bound.same_identity(t)
            || t.agent != "codex"
        {
            return Err("cleanup.protected".into());
        }
        let pending: bool=db.query_row("SELECT EXISTS(SELECT 1 FROM lifecycle_operations WHERE task_id=?1 AND state IN ('pending','unknown') UNION ALL SELECT 1 FROM dispatch_operations WHERE task_id=?1 AND state IN ('pending','unknown'))",[&task],|r|r.get(0)).map_err(err)?;
        if pending {
            return Err("cleanup.protected".into());
        }
        result = Some(Owner { task, binding, cwd });
    }
    Ok(result)
}
impl Workflow {
    pub fn execute_idle_cleanup(&self, project: &str, id: &str) -> Result<Report, String> {
        let workspace = self.cleanup_workspace(project)?;
        let lock = crate::lifecycle::pane_allocation::workspace_lock(
            self.herdr.machine_id(),
            self.herdr.session(),
            &workspace,
        )?;
        let _guard = lock.lock().map_err(err)?;
        let run = {
            let db = self.db.lock().map_err(err)?;
            let (state,created): (String,String)=db.query_row("SELECT state,created_at FROM idle_cleanup_batches WHERE id=?1 AND project_id=?2 AND machine_id=?3 AND session=?4 AND workspace=?5",
                params![id,project,self.herdr.machine_id(),self.herdr.session(),workspace],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|"cleanup.stale")?;
            if state == "preview" {
                let age = chrono::Utc::now().signed_duration_since(
                    chrono::DateTime::parse_from_rfc3339(&created).map_err(err)?,
                );
                if age > chrono::Duration::minutes(10) {
                    return Err("cleanup.stale".into());
                }
                db.execute(
                    "UPDATE idle_cleanup_batches SET state='running' WHERE id=?1",
                    [id],
                )
                .map_err(err)?;
                true
            } else {
                false
            }
        };
        if run {
            for (ordinal, candidate, state, _) in self.cleanup_items(id)? {
                if state != "ready" {
                    continue;
                }
                if let Err(error) = self.cleanup_one(project, id, ordinal, &candidate) {
                    // Only an item durably marked 'closing' may have reached Herdr.
                    let db = self.db.lock().map_err(err)?;
                    db.execute("UPDATE idle_cleanup_items SET error=CASE WHEN state='closing' THEN 'cleanup.unknown' ELSE ?3 END,
                        state=CASE WHEN state='closing' THEN 'unknown' ELSE 'skipped' END WHERE batch_id=?1 AND ordinal=?2 AND state IN ('ready','claimed','closing')",params![id,ordinal,error]).map_err(err)?;
                }
            }
            self.db
                .lock()
                .map_err(err)?
                .execute(
                    "UPDATE idle_cleanup_batches SET state='done' WHERE id=?1",
                    [id],
                )
                .map_err(err)?;
        }
        self.idle_cleanup_result(project, id)
    }
    fn cleanup_items(&self, id: &str) -> Result<Vec<(i64, Candidate, String, String)>, String> {
        let db = self.db.lock().map_err(err)?;
        let mut stmt=db.prepare("SELECT ordinal,candidate_json,state,error FROM idle_cleanup_items WHERE batch_id=?1 ORDER BY ordinal").map_err(err)?;
        stmt.query_map([id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(err)?
        .map(|r| {
            let (n, json, state, error) = r.map_err(err)?;
            Ok((n, serde_json::from_str(&json).map_err(err)?, state, error))
        })
        .collect()
    }
    fn check_cleanup_candidate(
        &self,
        db: &Connection,
        project: &str,
        id: &str,
        c: &Candidate,
    ) -> Result<(), String> {
        crate::machines::ensure_enabled(db, self.herdr.machine_id())?;
        if crate::projects::project_identity_on(
            db,
            project,
            self.herdr.machine_id(),
            self.herdr.session(),
        )? != c.pane.target.workspace_id
        {
            return Err("cleanup.stale".into());
        }
        ensure_unclaimed(db, &c.pane.target, Some(id))?;
        if eligible_owner(db, &c.pane)? != c.owner {
            return Err("cleanup.changed".into());
        }
        Ok(())
    }
    fn cleanup_one(&self, project: &str, id: &str, n: i64, c: &Candidate) -> Result<(), String> {
        // Capture a recoverable conversation before releasing a non-InProgress binding.
        let mut exact = c.pane.clone();
        let saved = if let Some(owner) = &c.owner {
            let bound: Target = serde_json::from_str(&owner.binding).map_err(err)?;
            exact.target.codex_resume_locks = bound.codex_resume_locks;
            Some(
                self.herdr
                    .session_id(&exact.target)?
                    .ok_or("flow.session_missing")?,
            )
        } else {
            None
        };
        {
            let mut db = self.db.lock().map_err(err)?;
            let tx = db.transaction().map_err(err)?;
            self.check_cleanup_candidate(&tx, project, id, c)?;
            if let (Some(owner), Some(session)) = (&c.owner, &saved) {
                uuid::Uuid::parse_str(session).map_err(|_| "flow.session_invalid")?;
                tx.execute("INSERT INTO task_sessions(task_id,codex_session_id,cwd,updated_at,machine_id) VALUES(?1,?2,?3,?4,?5)
                    ON CONFLICT(task_id) DO UPDATE SET codex_session_id=excluded.codex_session_id,cwd=excluded.cwd,updated_at=excluded.updated_at,machine_id=excluded.machine_id",
                    params![owner.task,session,owner.cwd,chrono::Utc::now().to_rfc3339(),c.pane.target.machine_id]).map_err(err)?;
            }
            tx.execute(
                "UPDATE idle_cleanup_items SET state='claimed' WHERE batch_id=?1 AND ordinal=?2",
                params![id, n],
            )
            .map_err(err)?;
            tx.commit().map_err(err)?;
        }
        let closed=self.herdr.close_idle_pane(&exact,&|| {
            let mut db=self.db.lock().map_err(err)?;
            let tx=db.transaction().map_err(err)?;
            self.check_cleanup_candidate(&tx,project,id,c)?;
            tx.execute("UPDATE idle_cleanup_items SET state='closing' WHERE batch_id=?1 AND ordinal=?2 AND state IN ('claimed','closing')",params![id,n]).map_err(err)?;
            tx.commit().map_err(err)
        });
        match closed {
            Ok(()) => {
                if self.cleanup_absent(&c.pane.target)? {
                    self.finish_cleanup_item(id, n, c)
                } else {
                    Err("cleanup.unknown".into())
                }
            }
            Err(error) => {
                let state = if error.uncertain {
                    "unknown"
                } else {
                    "skipped"
                };
                let message = if error.uncertain {
                    "cleanup.unknown"
                } else {
                    &error.message
                };
                self.db.lock().map_err(err)?.execute("UPDATE idle_cleanup_items SET state=?3,error=?4 WHERE batch_id=?1 AND ordinal=?2",params![id,n,state,message]).map_err(err)?;
                Ok(())
            }
        }
    }
    fn cleanup_absent(&self, target: &Target) -> Result<bool, String> {
        self.herdr.cleanup_target_absent(target)
    }
    fn finish_cleanup_item(&self, id: &str, n: i64, c: &Candidate) -> Result<(), String> {
        let mut db = self.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        if let Some(owner) = &c.owner {
            tx.execute("DELETE FROM task_bindings WHERE task_id=?1 AND target_json=?2 AND state='valid' AND EXISTS(SELECT 1 FROM tasks WHERE id=?1 AND status<>'in_progress')",params![owner.task,owner.binding]).map_err(err)?;
        }
        tx.execute("UPDATE idle_cleanup_items SET state='closed',error='' WHERE batch_id=?1 AND ordinal=?2",params![id,n]).map_err(err)?;
        tx.commit().map_err(err)
    }
    pub fn idle_cleanup_result(&self, project: &str, id: &str) -> Result<Report, String> {
        // Reading results is allowed offline. Reconciliation is read-only towards Herdr.
        let db = self.db.lock().map_err(err)?;
        let matches:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM idle_cleanup_batches WHERE id=?1 AND project_id=?2 AND machine_id=?3 AND session=?4)",params![id,project,self.herdr.machine_id(),self.herdr.session()],|r|r.get(0)).map_err(err)?;
        if !matches {
            return Err("cleanup.stale".into());
        }
        drop(db);
        let mut report = Report {
            id: id.into(),
            closed: 0,
            skipped: 0,
            failed: 0,
            items: vec![],
        };
        for (n, c, mut state, mut error) in self.cleanup_items(id)? {
            if state == "unknown"
                && self.cleanup_workspace(project).is_ok()
                && self.cleanup_absent(&c.pane.target).unwrap_or(false)
            {
                self.finish_cleanup_item(id, n, &c)?;
                state = "closed".into();
                error.clear();
            }
            match state.as_str() {
                "closed" => report.closed += 1,
                "skipped" => report.skipped += 1,
                _ => report.failed += 1,
            }
            report.items.push(ItemResult {
                pane_id: c.pane.target.pane_id,
                state,
                error,
            });
        }
        Ok(report)
    }
    fn cleanup_workspace(&self, project: &str) -> Result<String, String> {
        let db = self.db.lock().map_err(err)?;
        crate::machines::ensure_enabled(&db, self.herdr.machine_id())?;
        crate::projects::project_identity_on(
            &db,
            project,
            self.herdr.machine_id(),
            self.herdr.session(),
        )
    }
    pub fn preview_idle_cleanup(&self, project: &str) -> Result<Preview, String> {
        let workspace = self.cleanup_workspace(project)?;
        let panes = self.herdr.idle_panes(&workspace)?;
        let id = uuid::Uuid::new_v4().to_string();
        let mut db = self.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        if crate::projects::project_identity_on(
            &tx,
            project,
            self.herdr.machine_id(),
            self.herdr.session(),
        )? != workspace
        {
            return Err("cleanup.stale".into());
        }
        tx.execute(
            "INSERT INTO idle_cleanup_batches VALUES(?1,?2,?3,?4,?5,'preview',?6)",
            params![
                id,
                project,
                self.herdr.machine_id(),
                self.herdr.session(),
                workspace,
                chrono::Utc::now().to_rfc3339()
            ],
        )
        .map_err(err)?;
        let mut count = 0;
        for pane in panes {
            if pane.target.machine_id != self.herdr.machine_id()
                || pane.target.session != self.herdr.session()
                || pane.target.workspace_id != workspace
            {
                return Err("binding.identity_changed".into());
            }
            match ensure_unclaimed(&tx, &pane.target, None) {
                Ok(()) => {}
                Err(error) if error == "cleanup.pending" => continue,
                Err(error) => return Err(error),
            }
            let owner = match eligible_owner(&tx, &pane) {
                Ok(owner) => owner,
                Err(error) if error == "cleanup.protected" => continue,
                Err(error) => return Err(error),
            };
            let candidate = Candidate { pane, owner };
            tx.execute("INSERT INTO idle_cleanup_items(batch_id,ordinal,candidate_json,state) VALUES(?1,?2,?3,'ready')",params![id,count,serde_json::to_string(&candidate).map_err(err)?]).map_err(err)?;
            count += 1;
        }
        tx.commit().map_err(err)?;
        let unresolved=db.prepare("SELECT DISTINCT b.id FROM idle_cleanup_batches b JOIN idle_cleanup_items i ON i.batch_id=b.id
            WHERE b.project_id=?1 AND i.state IN ('closing','unknown','claimed') ORDER BY b.rowid")
            .map_err(err)?.query_map([project],|r|r.get::<_,String>(0)).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
        Ok(Preview {
            id,
            count,
            unresolved,
        })
    }
}
#[cfg(test)]
#[path = "idle_cleanup_tests.rs"]
mod tests;

#[tauri::command]
pub async fn preview_idle_cleanup(
    app: tauri::State<'_, std::sync::Arc<Workflow>>,
    project_id: String,
) -> Result<Preview, String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _scope = crate::remote_control::Scope::enter();
        app.for_project(&project_id)?
            .preview_idle_cleanup(&project_id)
    })
    .await
    .map_err(err)?
}
#[tauri::command]
pub async fn execute_idle_cleanup(
    app: tauri::State<'_, std::sync::Arc<Workflow>>,
    project_id: String,
    preview_id: String,
) -> Result<Report, String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _scope = crate::remote_control::Scope::enter();
        app.for_project(&project_id)?
            .execute_idle_cleanup(&project_id, &preview_id)
    })
    .await
    .map_err(err)?
}
#[tauri::command]
pub async fn idle_cleanup_result(
    app: tauri::State<'_, std::sync::Arc<Workflow>>,
    project_id: String,
    preview_id: String,
) -> Result<Report, String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _scope = crate::remote_control::Scope::enter();
        app.for_project(&project_id)?
            .idle_cleanup_result(&project_id, &preview_id)
    })
    .await
    .map_err(err)?
}
