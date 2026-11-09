//! Read-only live discovery: pane ownership alone is not proof of process identity.
use crate::workflow::{Target, Workflow};

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Discovery {
    pub targets: Vec<Target>,
    pub failures: Vec<DiscoveryFailure>,
    #[serde(default)]
    pub age_ms: u64,
}

pub type Discoveries = std::collections::HashMap<String, Result<Discovery, String>>;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DiscoveryFailure {
    pub pane_id: String,
    pub error: String,
}

#[derive(Debug, serde::Serialize)]
pub struct AgentSnapshot {
    pub agents: Vec<AgentChoice>,
    pub failures: Vec<DiscoveryFailure>,
    pub age_ms: u64,
    pub diagnostics: Vec<crate::agent_diagnostics::Diagnostic>,
}

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
        machine: String,
        live: Mutex<Result<Vec<Target>, String>>,
        failures: Mutex<Vec<DiscoveryFailure>>,
        db: Arc<Mutex<Connection>>,
    }
    impl Herdr for FakeHerdr {
        fn machine_id(&self) -> &str {
            &self.machine
        }
        fn targets(&self, _: &str) -> Result<Vec<Target>, String> {
            assert!(
                self.db.try_lock().is_ok(),
                "external reads must not hold SQLite"
            );
            self.live.lock().unwrap().clone()
        }
        fn discover_agents(&self, workspace: &str) -> Result<Discovery, String> {
            Ok(Discovery {
                targets: self.targets(workspace)?,
                failures: self.failures.lock().unwrap().clone(),
                age_ms: 0,
            })
        }
        fn send(&self, _: &Target, _: &str) -> Result<(), SendError> {
            panic!("status reads must never send")
        }
    }
    fn fixture() -> (Workflow, Arc<FakeHerdr>, Target) {
        fixture_on("local")
    }
    fn fixture_on(machine: &str) -> (Workflow, Arc<FakeHerdr>, Target) {
        let db = Connection::open_in_memory().unwrap();
        crate::db::init(&db).unwrap();
        if machine != "local" {
            db.execute(
                "INSERT INTO machines(id,name,kind,session) VALUES(?1,'Remote','ssh','default')",
                [machine],
            )
            .unwrap();
        }
        db.execute_batch("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at,herdr_session,workspace_id,sync_status)
            VALUES ('p','P','.','P','now','default','w1','ready');
            ").unwrap();
        db.execute("UPDATE projects SET machine_id=?1 WHERE id='p'", [machine])
            .unwrap();
        db.execute_batch("INSERT INTO tasks(id,project_id,name,status,created_at,updated_at) VALUES ('a','p','A','in_progress','now','now');").unwrap();
        let target = Target {
            machine_id: machine.into(),
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
        db.execute("INSERT INTO task_bindings(task_id,session,pane_id,target_json,state,machine_id) VALUES ('a','default','w1:p1',?1,'valid',?2)",
            params![serde_json::to_string(&target).unwrap(),machine]).unwrap();
        let db = Arc::new(Mutex::new(db));
        let fake = Arc::new(FakeHerdr {
            machine: machine.into(),
            live: Mutex::new(Ok(vec![target.clone()])),
            failures: Mutex::new(vec![]),
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
    fn non_task_agents_keep_ownership_without_breaking_workspace_status() {
        for machine in ["local", "remote-xc"] {
            for retired in [false, true] {
                let (app, fake, task) = fixture_on(machine);
                let other = Target {
                    pane_id: "w1:p2".into(),
                    terminal_id: "term-other".into(),
                    process_id: "300:400".into(),
                    status: "done".into(),
                    ..task.clone()
                };
                {
                    let db = app.db.lock().unwrap();
                    let json = serde_json::to_string(&other).unwrap();
                    if retired {
                        db.execute("INSERT INTO task_execution_resets(id,task_id,epoch,retired_task_id,snapshot_json,created_at) VALUES('reset','a',0,'old','{}','now')", []).unwrap();
                        db.execute("INSERT INTO retired_execution_targets(reset_id,target_json) VALUES('reset',?1)", [&json]).unwrap();
                    } else {
                        db.execute("INSERT INTO workspace_terminals(machine_id,session,workspace_id,operation_id,cwd,target_json,state) VALUES(?1,'default','w1','op','.',?2,'ready')", params![machine,json]).unwrap();
                    }
                }
                for status in ["idle", "working", "blocked", "done"] {
                    *fake.live.lock().unwrap() = Ok(vec![
                        other.clone(),
                        Target {
                            status: status.into(),
                            ..task.clone()
                        },
                    ]);
                    let snapshot = crate::agent_diagnostics::snapshot(&app, "p")
                        .expect("a protected non-task agent must not fail the whole workspace");
                    assert!(snapshot.failures.is_empty());
                    let task_agent = snapshot
                        .agents
                        .iter()
                        .find(|a| a.pane_id == task.pane_id)
                        .unwrap();
                    assert_eq!(task_agent.verified_task_id.as_deref(), Some("a"));
                    assert_eq!(task_agent.status, status);
                    let protected = snapshot
                        .agents
                        .iter()
                        .find(|a| a.pane_id == other.pane_id)
                        .unwrap();
                    assert_eq!(
                        protected.occupied_by.as_deref(),
                        Some(if retired {
                            "retired-execution"
                        } else {
                            "workspace-terminal"
                        })
                    );
                    assert!(protected.verified_task_id.is_none());
                    let candidates = app.list_agents("p").unwrap();
                    assert!(
                        candidates
                            .iter()
                            .find(|a| a.pane_id == other.pane_id)
                            .unwrap()
                            .occupied_by
                            .is_some()
                    );
                    assert_eq!(app.binding("a").unwrap().target.as_ref(), Some(&task));
                }
            }
        }
    }
    #[test]
    fn missing_real_task_is_classified_instead_of_becoming_external_error() {
        let (app, _, _) = fixture();
        {
            let db = app.db.lock().unwrap();
            db.execute_batch("PRAGMA foreign_keys=OFF; DELETE FROM tasks WHERE id='a';")
                .unwrap();
        }
        let diagnostic = crate::agent_diagnostics::snapshot(&app, "p").unwrap_err();
        assert_eq!(diagnostic.stage, "task_lookup");
        assert_eq!(diagnostic.code, "status.database_row_missing");
    }
    #[test]
    fn corrupt_real_binding_has_a_specific_safe_error() {
        let (app, _, _) = fixture();
        app.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE task_bindings SET target_json='private broken json' WHERE task_id='a'",
                [],
            )
            .unwrap();
        let diagnostic = crate::agent_diagnostics::snapshot(&app, "p").unwrap_err();
        assert_eq!(diagnostic.stage, "binding");
        assert_eq!(diagnostic.code, "status.binding_read_failed");
        assert!(
            !serde_json::to_string(&diagnostic)
                .unwrap()
                .contains("private")
        );
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
    #[test]
    fn background_snapshot_keeps_per_pane_failure_distinct_from_missing_agent() {
        let (app, fake, target) = fixture();
        *fake.live.lock().unwrap() = Ok(vec![]);
        *fake.failures.lock().unwrap() = vec![DiscoveryFailure {
            pane_id: target.pane_id.clone(),
            error: "herdr.timeout".into(),
        }];
        let snapshot = app.agent_snapshot("p").unwrap();
        assert!(snapshot.agents.is_empty());
        assert_eq!(snapshot.failures.len(), 1);
        assert_eq!(snapshot.failures[0].pane_id, "w1:p1");
        fake.failures.lock().unwrap().clear();
        assert!(app.agent_snapshot("p").unwrap().failures.is_empty());
        *fake.live.lock().unwrap() = Ok(vec![Target {
            status: "done".into(),
            ..target
        }]);
        let snapshot = app.agent_snapshot("p").unwrap();
        assert_eq!(snapshot.agents[0].verified_task_id.as_deref(), Some("a"));
        assert_eq!(snapshot.agents[0].status, "done");
    }

    #[test]
    fn diagnostic_snapshot_preserves_batch_and_pane_scope_without_database_changes() {
        let (app, fake, target) = fixture();
        *fake.live.lock().unwrap() = Err(crate::agent_diagnostics::staged(
            "agent_list",
            "herdr.timeout".into(),
        ));
        let error = crate::agent_diagnostics::snapshot(&app, "p").unwrap_err();
        assert_eq!(error.stage, "agent_list");
        assert_eq!(error.code, "herdr.timeout");
        assert_eq!(error.workspace_id, "w1");
        assert!(error.pane_id.is_none());
        *fake.live.lock().unwrap() = Ok(vec![]);
        *fake.failures.lock().unwrap() = vec![DiscoveryFailure {
            pane_id: target.pane_id.clone(),
            error: crate::agent_diagnostics::staged(
                "process_info",
                "herdr.failed: private stderr".into(),
            ),
        }];
        let snapshot = crate::agent_diagnostics::snapshot(&app, "p").unwrap();
        assert_eq!(snapshot.failures[0].error, "herdr.failed");
        assert_eq!(snapshot.diagnostics[0].stage, "process_info");
        assert_eq!(snapshot.diagnostics[0].task_id.as_deref(), Some("a"));
        assert!(
            !serde_json::to_string(&snapshot)
                .unwrap()
                .contains("private stderr")
        );
        fake.failures.lock().unwrap().clear();
        *fake.live.lock().unwrap() = Ok(vec![target]);
        assert!(
            crate::agent_diagnostics::snapshot(&app, "p")
                .unwrap()
                .diagnostics
                .is_empty()
        );
        assert_eq!(app.binding("a").unwrap().state, "valid");
    }
    #[test]
    fn batched_snapshots_preserve_ownership_and_per_project_errors() {
        let (app, _, _) = fixture();
        let results = crate::agent_diagnostics::snapshots(&app, vec!["p".into(), "missing".into(), "p".into()]);
        assert_eq!(results.len(), 2);
        assert!(results["p"].error.is_none());
        assert_eq!(results["p"].snapshot.as_ref().unwrap().agents.len(), app.agent_snapshot("p").unwrap().agents.len());
        assert!(results["missing"].snapshot.is_none());
        assert!(results["missing"].error.is_some());
    }
}

impl Workflow {
    pub fn agent_snapshot(&self, project: &str) -> Result<AgentSnapshot, String> {
        let workspace = self.agent_workspace(project).map_err(|e| {
            crate::agent_diagnostics::read_error(
                "project_identity",
                e,
                "status.project_read_failed",
            )
        })?;
        let discovery = self.herdr.discover_agents(&workspace)?;
        self.agent_snapshot_from_discovery(project, &workspace, discovery)
    }
    pub(crate) fn agent_snapshot_from_discovery(&self, project: &str, workspace: &str, discovery: Discovery) -> Result<AgentSnapshot, String> {
        Ok(AgentSnapshot {
            agents: self.assign_agents(project, workspace, discovery.targets)?,
            failures: discovery.failures,
            age_ms: discovery.age_ms,
            diagnostics: Vec::new(),
        })
    }
    pub fn list_agents(&self, project: &str) -> Result<Vec<AgentChoice>, String> {
        let workspace = self.agent_workspace(project)?;
        let targets = self.herdr.targets(&workspace)?;
        self.assign_agents(project, &workspace, targets)
    }
    pub(crate) fn agent_workspace(&self, project: &str) -> Result<String, String> {
        crate::projects::project_identity_on(
            &*self.db.lock().map_err(|e| e.to_string())?,
            project,
            self.herdr.machine_id(),
            self.herdr.session(),
        )
    }
    fn assign_agents(
        &self,
        project: &str,
        workspace: &str,
        targets: Vec<Target>,
    ) -> Result<Vec<AgentChoice>, String> {
        // External queries have finished; recheck ownership before publishing.
        let db = self.db.lock().map_err(|e| {
            crate::agent_diagnostics::read_error("ownership", e, "status.database_lock_failed")
        })?;
        if crate::projects::project_identity_on(
            &db,
            project,
            self.herdr.machine_id(),
            self.herdr.session(),
        )
        .map_err(|e| {
            crate::agent_diagnostics::read_error(
                "project_identity",
                e,
                "status.project_read_failed",
            )
        })? != workspace
        {
            return Err("binding.identity_changed".into());
        }
        targets.into_iter().map(|target| {
            let owner = crate::workflow::pane_ownership(&db, &target).map_err(|e|
                crate::agent_diagnostics::read_error("ownership", e, "status.ownership_read_failed"))?;
            let verified_task_id = if let Some(crate::workflow::PaneOwner::Task(task_id)) = owner.as_ref() {
                let task_is_current: bool = db.query_row(
                    "SELECT project_id=?2 AND archived_at IS NULL AND NOT EXISTS(
                        SELECT 1 FROM lifecycle_operations WHERE task_id=?1 AND state IN ('pending','unknown'))
                     FROM tasks WHERE id=?1", [task_id.as_str(), project], |r| r.get(0),
                ).map_err(|e| crate::agent_diagnostics::read_error("task_lookup", e, "status.database_query_failed"))?;
                let binding = crate::workflow::binding(&db, task_id).map_err(|e|
                    crate::agent_diagnostics::read_error("binding", e, "status.binding_read_failed"))?;
                (task_is_current && binding.state == "valid"
                    && target.session == self.herdr.session() && target.workspace_id == workspace
                    && binding.target.as_ref().is_some_and(|expected| expected.same_identity(&target)))
                    .then(|| task_id.clone())
            } else { None };
            Ok(AgentChoice {
                name: target.agent.clone(), status: target.status.clone(),
                pane_id: target.pane_id.clone(), title: target.title.clone(),
                occupied_by: owner.map(|owner| owner.occupied_by().to_owned()), verified_task_id, target,
            })
        }).collect()
    }
}
