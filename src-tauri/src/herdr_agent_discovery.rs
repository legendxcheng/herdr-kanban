//! Background status reads retain per-pane failures instead of losing them as missing agents.
use super::*;
use crate::agent_status::{Discovery, DiscoveryFailure};

impl Cli {
    pub(super) fn discover_agent_statuses(&self, workspace: &str) -> Result<Discovery, String> {
        if let Some(machine) = &self.remote {
            let mut discovery: Discovery = crate::remote_transport::request(
                machine,
                serde_json::json!({"method":"agent_status_targets", "workspace":workspace}),
            )?;
            for target in &mut discovery.targets {
                target.machine_id = machine.id.clone();
            }
            return Ok(discovery);
        }
        let spaces = self.status_json(&["workspace", "list"])?;
        let matches = spaces
            .pointer("/result/workspaces")
            .and_then(Value::as_array)
            .ok_or("herdr.invalid_response")?
            .iter()
            .filter(|w| w["workspace_id"] == workspace)
            .count();
        if matches != 1 {
            return Err("binding.workspace_ambiguous".into());
        }
        let sampled = std::time::Instant::now();
        let doc = self.status_json(&["agent", "list"])?;
        let agents = doc
            .pointer("/result/agents")
            .and_then(Value::as_array)
            .ok_or("herdr.invalid_response")?;
        let agents: Vec<_> = agents
            .iter()
            .filter(|a| a["workspace_id"] == workspace)
            .map(|a| Ok((field(a, "pane_id")?, a)))
            .collect::<Result<_, String>>()?;
        let mut discovery = Discovery::default();
        // Each status read has a short deadline. Keep process checks sequential
        // within a Workspace; project-level concurrency is already bounded by the caller.
        for (pane, agent) in agents {
            match self.observe_status(workspace, agent) {
                Ok(target) => discovery.targets.push(target),
                Err(error) => discovery.failures.push(DiscoveryFailure {
                    pane_id: pane.into(),
                    error,
                }),
            }
        }
        discovery.age_ms = sampled.elapsed().as_millis().min(u64::MAX as u128) as u64;
        Ok(discovery)
    }

    fn status_target(&self, workspace: &str, agent: &Value) -> Result<Target, String> {
        let info =
            self.status_json(&["pane", "process-info", "--pane", field(agent, "pane_id")?])?;
        self.target_from_process_info(workspace, agent, &info)
    }

    fn observe_status(&self, workspace: &str, agent: &Value) -> Result<Target, String> {
        if let Ok(target) = self.status_target(workspace, agent) {
            if matches!(
                target.status.as_str(),
                "working" | "blocked" | "done" | "idle"
            ) {
                return Ok(target);
            }
        }
        let doc = self.status_json(&["agent", "get", field(agent, "pane_id")?])?;
        let current = doc
            .pointer("/result/agent")
            .ok_or("herdr.invalid_response")?;
        if ["workspace_id", "pane_id", "terminal_id", "agent"]
            .iter()
            .any(|key| current[*key] != agent[*key])
        {
            return Err("binding.identity_changed".into());
        }
        self.status_target(workspace, current)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn background_discovery_recovers_unknown_list_entry_without_opening_or_focusing() {
        for (status, terminal, failure, expected) in [
            ("idle", "term", false, Some("idle")),
            ("working", "term", false, Some("working")),
            ("done", "term", false, Some("done")),
            ("unknown", "term", false, Some("unknown")),
            ("idle", "replacement", false, None),
            ("idle", "term", true, None),
        ] {
            background_fixture(status, terminal, failure, expected);
        }
    }

    fn background_fixture(
        status: &str,
        terminal: &str,
        fail_process: bool,
        expected: Option<&str>,
    ) {
        let fixture =
            Fixture(std::env::temp_dir().join(format!("kanban-status-{}", uuid::Uuid::new_v4())));
        std::fs::create_dir(&fixture.0).unwrap();
        let executable = fixture.0.join("herdr.cmd");
        let script = concat!(
            "@echo off\r\n",
            "if \"%~3 %~4\"==\"workspace list\" (\r\necho {\"result\":{\"workspaces\":[{\"workspace_id\":\"w1\"}]}}\r\nexit /b 0\r\n)\r\n",
            "if \"%~3 %~4\"==\"agent list\" (\r\necho {\"result\":{\"agents\":[{\"workspace_id\":\"w1\",\"pane_id\":\"w1:p1\",\"terminal_id\":\"term\",\"agent\":\"codex\",\"agent_status\":\"unknown\"}]}}\r\nexit /b 0\r\n)\r\n",
            "if \"%~3 %~4\"==\"agent get\" (\r\necho {\"result\":{\"agent\":{\"workspace_id\":\"w1\",\"pane_id\":\"w1:p1\",\"terminal_id\":\"term\",\"agent\":\"codex\",\"agent_status\":\"idle\"}}}\r\nexit /b 0\r\n)\r\n",
            "if \"%~3 %~4\"==\"pane process-info\" (\r\necho {\"result\":{\"process_info\":{\"shell_pid\":1,\"foreground_process_group_id\":2}}}\r\nexit /b 0\r\n)\r\n",
            "echo unexpected>>\"%~dp0writes\"\r\nexit /b 3\r\n",
        );
        let script = script.replace(
            "\"agent_status\":\"idle\"",
            &format!("\"agent_status\":\"{status}\""),
        );
        // Replace only the single-agent response, leaving the list identity intact.
        let script = script.replace("\"agent\":{\"workspace_id\":\"w1\",\"pane_id\":\"w1:p1\",\"terminal_id\":\"term\"",
            &format!("\"agent\":{{\"workspace_id\":\"w1\",\"pane_id\":\"w1:p1\",\"terminal_id\":\"{terminal}\""));
        let script = if fail_process {
            script.replace(
                "\"foreground_process_group_id\":2",
                "\"foreground_process_group_id\":0",
            )
        } else {
            script
        };
        std::fs::write(&executable, script).unwrap();
        let cli = Cli {
            executable,
            session: "default".into(),
            remote: None,
        };
        let result = cli.discover_agents("w1").unwrap();
        if let Some(expected) = expected {
            assert_eq!(result.targets[0].status, expected);
            assert!(result.failures.is_empty());
        } else {
            assert!(result.targets.is_empty());
            assert_eq!(result.failures.len(), 1);
            assert_eq!(
                result.failures[0].error,
                if fail_process {
                    "binding.identity_unknown"
                } else {
                    "binding.identity_changed"
                }
            );
        }
        assert!(!fixture.0.join("writes").exists());
    }

    #[test]
    fn slow_pane_does_not_expire_healthy_background_observations() {
        let fixture = Fixture(
            std::env::temp_dir().join(format!("kanban-slow-status-{}", uuid::Uuid::new_v4())),
        );
        std::fs::create_dir(&fixture.0).unwrap();
        let executable = fixture.0.join("herdr.cmd");
        let bad = serde_json::json!({"workspace_id":"w1","pane_id":"w1:p1","terminal_id":"bad","agent":"codex","agent_status":"unknown"});
        let good = serde_json::json!({"workspace_id":"w1","pane_id":"w1:p2","terminal_id":"good","agent":"codex","agent_status":"idle"});
        let list = serde_json::json!({"result":{"agents":[bad.clone(),good]}});
        let get = serde_json::json!({"result":{"agent":bad}});
        let script = format!(
            concat!(
                "@echo off\r\n",
                "if \"%~3 %~4\"==\"workspace list\" (\r\necho {{\"result\":{{\"workspaces\":[{{\"workspace_id\":\"w1\"}}]}}}}\r\nexit /b 0\r\n)\r\n",
                "if \"%~3 %~4\"==\"agent list\" (\r\necho {list}\r\nexit /b 0\r\n)\r\n",
                "if \"%~3 %~4\"==\"agent get\" (\r\necho {get}\r\nexit /b 0\r\n)\r\n",
                "if \"%~3 %~4\"==\"pane process-info\" (\r\n",
                "if \"%~6\"==\"w1:p1\" ping -n 5 127.0.0.1 >nul\r\n",
                "echo {{\"result\":{{\"process_info\":{{\"shell_pid\":1,\"foreground_process_group_id\":2}}}}}}\r\nexit /b 0\r\n)\r\nexit /b 3\r\n"
            ),
            list = list,
            get = get
        );
        std::fs::write(&executable, script).unwrap();
        let cli = Cli {
            executable,
            session: "default".into(),
            remote: None,
        };
        let result = cli.discover_agents("w1").unwrap();
        assert_eq!(result.targets.len(), 1);
        assert_eq!(result.targets[0].pane_id, "w1:p2");
        assert_eq!(result.targets[0].status, "idle");
        assert_eq!(result.failures.len(), 1);
        assert_eq!(result.failures[0].pane_id, "w1:p1");
        assert!(
            result.age_ms < 5_000,
            "healthy sample aged {}ms",
            result.age_ms
        );
    }

    #[test]
    #[ignore = "Read-only live local status acceptance; requires HERDR_ENV=1 and the running Kanban database"]
    fn live_background_task_statuses_without_viewer() {
        assert_eq!(std::env::var("HERDR_ENV").as_deref(), Ok("1"));
        let path =
            PathBuf::from(std::env::var_os("APPDATA").unwrap()).join("herdr-kanban/kanban.db");
        let db =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        let projects = db.prepare("SELECT DISTINCT p.id FROM projects p JOIN tasks t ON t.project_id=p.id WHERE p.machine_id='local' AND p.herdr_session='default' AND t.status='in_progress' AND t.archived_at IS NULL")
            .unwrap().query_map([], |row| row.get::<_, String>(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        let mut cli = Cli::from_env();
        cli.session = "default".into();
        let app = crate::workflow::Workflow {
            db: std::sync::Arc::new(std::sync::Mutex::new(db)),
            herdr: std::sync::Arc::new(cli),
        };
        let mut verified = 0;
        for project in projects {
            let snapshot = match app.agent_snapshot(&project) {
                Ok(snapshot) => snapshot,
                Err(error) if error == "binding.workspace_ambiguous" => {
                    println!("project={project} unavailable={error}");
                    continue;
                }
                Err(error) => panic!("project={project}: {error}"),
            };
            for agent in snapshot
                .agents
                .iter()
                .filter(|a| a.verified_task_id.is_some())
            {
                println!("verified pane={} status={}", agent.pane_id, agent.status);
                if matches!(
                    agent.status.as_str(),
                    "working" | "idle" | "done" | "blocked"
                ) {
                    verified += 1;
                }
            }
            println!(
                "project={project} query_failures={}",
                snapshot.failures.len()
            );
        }
        assert!(verified > 0, "No live bound task has a classified state");
    }
}
