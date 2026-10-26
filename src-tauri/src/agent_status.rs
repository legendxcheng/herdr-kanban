//! Read-only live discovery: pane ownership alone is not proof of process identity.
use crate::workflow::{Target, Workflow};

#[derive(Debug, serde::Serialize)]
pub struct AgentChoice {
    pub name: String,
    pub status: String,
    pub pane_id: String,
    pub title: String,
    pub occupied_by: Option<String>,
    pub verified_task_id: Option<String>,
    pub target: Target,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::{Herdr, SendError};
    use rusqlite::{Connection, params};
    use std::sync::{Arc, Mutex};

    struct FakeHerdr {
        live: Mutex<Result<Vec<Target>, String>>,
        db: Arc<Mutex<Connection>>,
    }
    impl Herdr for FakeHerdr {
        fn targets(&self, _: &str) -> Result<Vec<Target>, String> {
            assert!(
                self.db.try_lock().is_ok(),
                "external reads must not hold SQLite"
            );
            self.live.lock().unwrap().clone()
        }
        fn send(&self, _: &Target, _: &str) -> Result<(), SendError> {
            panic!("status reads must never send")
        }
    }
    fn fixture() -> (Workflow, Arc<FakeHerdr>, Target) {
        let db = Connection::open_in_memory().unwrap();
        crate::db::init(&db).unwrap();
        db.execute_batch("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at,herdr_session,workspace_id,sync_status)
            VALUES ('p','P','.','P','now','default','w1','ready');
            INSERT INTO tasks(id,project_id,name,status,created_at,updated_at) VALUES ('a','p','A','in_progress','now','now');").unwrap();
        let target = Target {
            machine_id: crate::machines::local_id(),
            session: "default".into(),
            workspace_id: "w1".into(),
            pane_id: "w1:p1".into(),
            terminal_id: "term-a".into(),
            agent: "codex".into(),
            process_id: "100:200".into(),
            codex_resume_locks: Vec::new(),
            title: "A".into(),
            status: "idle".into(),
        };
        db.execute("INSERT INTO task_bindings(task_id,session,pane_id,target_json,state) VALUES ('a','default','w1:p1',?1,'valid')",
            [serde_json::to_string(&target).unwrap()]).unwrap();
        let db = Arc::new(Mutex::new(db));
        let fake = Arc::new(FakeHerdr {
            live: Mutex::new(Ok(vec![target.clone()])),
            db: db.clone(),
        });
        (
            Workflow {
                db,
                herdr: fake.clone(),
            },
            fake,
            target,
        )
    }
    #[test]
    fn live_status_is_read_without_changing_task_or_binding() {
        let (app, fake, target) = fixture();
        for status in [
            "working",
            "blocked",
            "done",
            "idle",
            "unknown",
            "future-state",
        ] {
            let mut live = target.clone();
            live.status = status.into();
            live.title = "renamed".into();
            *fake.live.lock().unwrap() = Ok(vec![live]);
            let choices = app.list_agents("p").unwrap();
            assert_eq!(choices[0].verified_task_id.as_deref(), Some("a"));
            assert_eq!(choices[0].status, status);
        }
        let db = app.db.lock().unwrap();
        assert_eq!(
            crate::workflow::binding(&db, "a").unwrap().target.unwrap(),
            target
        );
        let state: (String, String) = db
            .query_row(
                "SELECT status,updated_at FROM tasks WHERE id='a'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, ("in_progress".into(), "now".into()));
    }
    #[test]
    fn replaced_identity_is_not_assigned_to_the_old_task() {
        let (app, fake, target) = fixture();
        for field in [
            "session",
            "workspace_id",
            "pane_id",
            "terminal_id",
            "agent",
            "process_id",
        ] {
            let mut json = serde_json::to_value(&target).unwrap();
            json[field] = "changed".into();
            *fake.live.lock().unwrap() = Ok(vec![serde_json::from_value(json).unwrap()]);
            assert!(
                app.list_agents("p").unwrap()[0].verified_task_id.is_none(),
                "{field}"
            );
        }
    }
    #[test]
    fn reserved_unknown_archived_and_lifecycle_pending_are_not_live_task_status() {
        let (app, _, target) = fixture();
        for state in ["pending", "unknown"] {
            app.db.lock().unwrap().execute("INSERT INTO lifecycle_operations(id,task_id,kind,state,error,created_at) VALUES (?1,'a','start',?2,'','now')", params![state,state]).unwrap();
            assert!(app.list_agents("p").unwrap()[0].verified_task_id.is_none());
            app.db
                .lock()
                .unwrap()
                .execute("DELETE FROM lifecycle_operations", [])
                .unwrap();
            app.db.lock().unwrap().execute("INSERT INTO dispatch_operations(id,task_id,workspace,target_json,prompt,created_at,state,had_binding) VALUES (?1,'a','w1',?2,'','now',?3,1)", params![state,serde_json::to_string(&target).unwrap(),state]).unwrap();
            assert!(app.list_agents("p").unwrap()[0].verified_task_id.is_none());
            app.db
                .lock()
                .unwrap()
                .execute("DELETE FROM dispatch_operations", [])
                .unwrap();
        }
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE task_bindings SET state='reserved'", [])
            .unwrap();
        let choice = app.list_agents("p").unwrap().remove(0);
        assert_eq!(choice.occupied_by.as_deref(), Some("a"));
        assert!(choice.verified_task_id.is_none());
        app.db
            .lock()
            .unwrap()
            .execute_batch(
                "UPDATE task_bindings SET state='valid'; UPDATE tasks SET archived_at='now'",
            )
            .unwrap();
        assert!(app.list_agents("p").unwrap()[0].verified_task_id.is_none());
    }
    #[test]
    fn offline_is_an_error_and_missing_agent_is_not_replaced_by_saved_status() {
        let (app, fake, _) = fixture();
        *fake.live.lock().unwrap() = Err("offline".into());
        assert_eq!(app.list_agents("p").unwrap_err(), "offline");
        *fake.live.lock().unwrap() = Ok(vec![]);
        assert!(app.list_agents("p").unwrap().is_empty());
        assert_eq!(app.binding("a").unwrap().state, "valid");
    }
}

impl Workflow {
    pub fn list_agents(&self, project: &str) -> Result<Vec<AgentChoice>, String> {
        let workspace = crate::projects::project_identity_on(
            &*self.db.lock().map_err(|e| e.to_string())?,
            project,
            self.herdr.machine_id(),
            self.herdr.session(),
        )?;
        // Never hold SQLite while waiting for Herdr.
        let targets = self.herdr.targets(&workspace)?;
        let db = self.db.lock().map_err(|e| e.to_string())?;
        if crate::projects::project_identity_on(
            &db,
            project,
            self.herdr.machine_id(),
            self.herdr.session(),
        )? != workspace
        {
            return Err("binding.identity_changed".into());
        }
        targets.into_iter().map(|target| {
            let owner = crate::workflow::pane_owner(&db, &target)?;
            let verified_task_id = if let Some(task_id) = owner.as_ref() {
                let task_is_current: bool = db.query_row(
                    "SELECT project_id=?2 AND archived_at IS NULL AND NOT EXISTS(
                        SELECT 1 FROM lifecycle_operations WHERE task_id=?1 AND state IN ('pending','unknown'))
                     FROM tasks WHERE id=?1", [task_id.as_str(), project], |r| r.get(0),
                ).map_err(|e| e.to_string())?;
                let binding = crate::workflow::binding(&db, task_id)?;
                (task_is_current && binding.state == "valid"
                    && target.session == self.herdr.session() && target.workspace_id == workspace
                    && binding.target.as_ref().is_some_and(|expected| expected.same_identity(&target)))
                    .then(|| task_id.clone())
            } else { None };
            Ok(AgentChoice {
                name: target.agent.clone(), status: target.status.clone(),
                pane_id: target.pane_id.clone(), title: target.title.clone(),
                occupied_by: owner, verified_task_id, target,
            })
        }).collect()
    }
}
