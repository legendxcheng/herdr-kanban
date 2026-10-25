//! Serialize only placement, not agent startup. Journal the pre-create inventory before side effects.
use crate::workflow::{PaneIdentity, Target, Workflow};
use rusqlite::{OptionalExtension, params};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, Weak},
};

pub(super) fn workspace_lock(session: &str, workspace: &str) -> Result<Arc<Mutex<()>>, String> {
    type Locks = HashMap<(String, String), Weak<Mutex<()>>>;
    static LOCKS: OnceLock<Mutex<Locks>> = OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(Default::default)
        .lock()
        .map_err(|e| e.to_string())?;
    locks.retain(|_, lock| lock.strong_count() > 0);
    let key = (session.to_string(), workspace.to_string());
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return Ok(lock);
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    Ok(lock)
}

impl Workflow {
    pub(super) fn record_allocation_intent(
        &self,
        task: &str,
        op: &str,
        workspace: &str,
    ) -> Result<(), String> {
        let Some(before) = self.herdr.pane_inventory(workspace)? else {
            return Ok(());
        };
        let db = self.db.lock().map_err(|e| e.to_string())?;
        super::ensure_operation(&db, task, op)?;
        db.execute("INSERT INTO pane_allocations(operation_id,session,workspace,before_json) VALUES (?1,?2,?3,?4)",
            params![op, self.herdr.session(), workspace, serde_json::to_string(&before).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub(super) fn record_allocation_target(
        &self,
        task: &str,
        op: &str,
        target: &Target,
    ) -> Result<(), String> {
        let db = self.db.lock().map_err(|e| e.to_string())?;
        super::ensure_operation(&db, task, op)?;
        db.execute(
            "UPDATE pane_allocations SET target_json=?2 WHERE operation_id=?1",
            params![
                op,
                serde_json::to_string(target).map_err(|e| e.to_string())?
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub(super) fn allocation_quarantines(&self, target: &Target) -> Result<bool, String> {
        let db = self.db.lock().map_err(|e| e.to_string())?;
        let mut stmt = db.prepare("SELECT before_json,target_json FROM pane_allocations WHERE session=?1 AND workspace=?2 AND resolved=0").map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![target.session, target.workspace_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (before, created) = row.map_err(|e| e.to_string())?;
            if let Some(created) = created {
                let created: Target = serde_json::from_str(&created).map_err(|e| e.to_string())?;
                if created.terminal_id == target.terminal_id {
                    return Ok(true);
                }
            } else {
                let before: Vec<PaneIdentity> =
                    serde_json::from_str(&before).map_err(|e| e.to_string())?;
                if !before.iter().any(|p| p.terminal_id == target.terminal_id) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// None means a legacy operation. Ambiguous new panes are never adopted or sent a prompt.
    pub(super) fn allocation_absent(
        &self,
        workspace: &str,
        op: &str,
    ) -> Result<Option<bool>, String> {
        let lock = workspace_lock(self.herdr.session(), workspace)?;
        let _guard = lock.lock().map_err(|e| e.to_string())?;
        let record = {
            let db = self.db.lock().map_err(|e| e.to_string())?;
            db.query_row("SELECT before_json,target_json FROM pane_allocations WHERE operation_id=?1 AND session=?2 AND workspace=?3",
                params![op,self.herdr.session(),workspace], |r| Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?))).optional().map_err(|e| e.to_string())?
        };
        let Some((before, created)) = record else {
            return Ok(None);
        };
        let live = self
            .herdr
            .pane_inventory(workspace)?
            .ok_or("herdr.invalid_response")?;
        if let Some(created) = created {
            let created: Target = serde_json::from_str(&created).map_err(|e| e.to_string())?;
            return Ok(Some(
                !live.iter().any(|p| p.terminal_id == created.terminal_id),
            ));
        }
        let before: Vec<PaneIdentity> = serde_json::from_str(&before).map_err(|e| e.to_string())?;
        let db = self.db.lock().map_err(|e| e.to_string())?;
        let mut stmt = db.prepare("SELECT target_json FROM pane_allocations WHERE session=?1 AND operation_id<>?2 AND target_json IS NOT NULL").map_err(|e| e.to_string())?;
        let known = stmt
            .query_map(params![self.herdr.session(), op], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .map(|r| {
                r.map_err(|e| e.to_string())
                    .and_then(|s| serde_json::from_str::<Target>(&s).map_err(|e| e.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(live.iter().all(|p| {
            before.iter().any(|b| b.terminal_id == p.terminal_id)
                || known.iter().any(|k| k.terminal_id == p.terminal_id)
        })))
    }
}
