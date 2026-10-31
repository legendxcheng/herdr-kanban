//! The external CLI boundary. Every request is bounded; arguments never use a shell.
use crate::workflow::{Herdr, SendError, Target};
#[path = "herdr_cli.rs"]
mod cli;
pub(crate) use cli::agent_start_rejected_busy;
#[cfg(all(test, windows))]
#[path = "agent_start_tests.rs"]
mod agent_start_tests;
#[path = "codex_process.rs"]
mod codex_process;
#[path = "herdr_executable.rs"]
mod executable;
#[cfg(test)]
#[path = "session_isolation_tests.rs"]
mod session_isolation_tests;
#[path = "shared_tab.rs"]
mod shared_tab;
#[path = "herdr_socket.rs"]
mod socket;
#[path = "herdr_workspace.rs"]
mod workspace_metadata;
use workspace_metadata::workspace_directories;
#[path = "herdr_codex.rs"]
mod codex;
#[path = "codex_receipt.rs"]
mod codex_receipt;
#[cfg(any(windows, test))]
#[path = "herdr_codex_submit.rs"]
mod codex_submit;
#[path = "herdr_idle_cleanup.rs"]
mod idle_cleanup;
#[path = "herdr_workspace_terminal.rs"]
mod workspace_terminal;
#[path = "herdr_agent_discovery.rs"]
mod agent_discovery;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone)]
pub struct Cli {
    pub executable: PathBuf,
    pub session: String,
    pub remote: Option<crate::machines::Machine>,
}

impl Cli {
    fn check_target(&self, target: &Target) -> Result<(), SendError> {
        // Revalidate immediately before sending, after the ownership reservation.
        let doc = self
            .json(&["agent", "get", &target.pane_id])
            .map_err(|message| SendError {
                message,
                uncertain: false,
            })?;
        let live = self
            .live_target(
                &target.workspace_id,
                doc.pointer("/result/agent").ok_or(SendError {
                    message: "binding.missing".into(),
                    uncertain: false,
                })?,
            )
            .map_err(|message| SendError {
                message,
                uncertain: false,
            })?;
        if !live.same_identity(target) {
            return Err(SendError {
                message: "binding.identity_changed".into(),
                uncertain: false,
            });
        }
        Ok(())
    }

    fn resolve_workspace_id(&self, workspace: &str) -> Result<String, String> {
        let doc = self.json(&["workspace", "list"])?;
        let spaces = doc
            .pointer("/result/workspaces")
            .and_then(Value::as_array)
            .ok_or("herdr.invalid_response")?;
        let matches: Vec<_> = spaces
            .iter()
            .filter(|w| w.get("workspace_id").and_then(Value::as_str) == Some(workspace))
            .collect();
        if matches.len() != 1 {
            return Err("binding.workspace_ambiguous".into());
        }
        Ok(field(matches[0], "workspace_id")?.to_string())
    }
    pub fn live_target(&self, workspace_id: &str, agent: &Value) -> Result<Target, String> {
        if field(agent, "workspace_id")? != workspace_id {
            return Err("binding.identity_changed".into());
        }
        let pane = field(agent, "pane_id")?;
        let info = self.json(&["pane", "process-info", "--pane", pane])?;
        self.target_from_process_info(workspace_id, agent, &info)
    }
    fn target_from_process_info(
        &self,
        workspace_id: &str,
        agent: &Value,
        info: &Value,
    ) -> Result<Target, String> {
        if field(agent, "workspace_id")? != workspace_id {
            return Err("binding.identity_changed".into());
        }
        let pane = field(agent, "pane_id")?;
        let group = info
            .pointer("/result/process_info/foreground_process_group_id")
            .and_then(Value::as_u64)
            .ok_or("binding.identity_unknown")?;
        let shell = info
            .pointer("/result/process_info/shell_pid")
            .and_then(Value::as_u64)
            .ok_or("binding.identity_unknown")?;
        if group == 0 {
            return Err("binding.identity_unknown".into());
        }
        Ok(Target {
            machine_id: self.machine_id().into(),
            session: self.session.clone(),
            workspace_id: workspace_id.into(),
            pane_id: pane.into(),
            terminal_id: field(agent, "terminal_id")?.into(),
            agent: field(agent, "agent")?.into(),
            process_id: format!("{shell}:{group}"),
            codex_resume_locks: Vec::new(),
            title: agent
                .get("terminal_title_stripped")
                .and_then(Value::as_str)
                .unwrap_or("")
                .into(),
            status: agent
                .get("agent_status")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .into(),
        })
    }
}
fn field<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "herdr.invalid_response".into())
}
impl Herdr for Cli {
    fn workspace_terminal_live(
        &self,
        target: &crate::workspace_terminal::Target,
    ) -> Result<bool, String> {
        self.shell_live(target)
    }
    fn find_workspace_terminal(
        &self,
        workspace: &str,
        operation: &str,
    ) -> Result<Option<crate::workspace_terminal::Target>, String> {
        self.shell_find(workspace, operation)
    }
    fn create_workspace_terminal(
        &self,
        workspace: &str,
        cwd: &str,
        operation: &str,
        check: &dyn Fn() -> Result<(), String>,
    ) -> Result<crate::workspace_terminal::Target, SendError> {
        self.shell_create(workspace, cwd, operation, check)
    }
    fn machine_id(&self) -> &str {
        self.remote
            .as_ref()
            .map(|m| m.id.as_str())
            .unwrap_or(crate::machines::LOCAL_ID)
    }
    fn resources(&self) -> &dyn crate::execution::Resources {
        match &self.remote {
            Some(machine) => machine,
            None => &crate::execution::LOCAL,
        }
    }
    fn terminal_client(&self) -> Result<Cli, String> {
        Ok(self.clone())
    }
    fn prompt_receipt(
        &self,
        target: &Target,
        prompt: &str,
        after: &str,
        expected_session: Option<&str>,
    ) -> Result<Option<crate::workflow::PromptReceipt>, String> {
        if let Some(machine) = &self.remote {
            return crate::remote_transport::request(
                machine,
                serde_json::json!({"method":"prompt_receipt","target":target,"prompt":prompt,"after":after,"expected":expected_session}),
            );
        }
        self.read_prompt_receipt(target, prompt, after, expected_session)
    }
    fn session(&self) -> &str {
        &self.session
    }
    fn workspaces(&self) -> Result<Vec<crate::projects::Workspace>, String> {
        if let Some(machine) = &self.remote {
            return crate::remote_transport::request(
                machine,
                serde_json::json!({"method":"workspaces"}),
            );
        }
        let doc = self.json(&["workspace", "list"])?;
        self.parse_workspaces(&doc)
    }
    fn move_workspace(
        &self,
        id: &str,
        before: Option<&str>,
    ) -> Result<Vec<crate::projects::Workspace>, String> {
        if let Some(machine) = &self.remote {
            return crate::remote_transport::request(
                machine,
                serde_json::json!({"method":"move_workspace","id":id,"before":before}),
            );
        }
        let doc = self.socket_request(
            "workspace.move_block",
            serde_json::json!({
                "workspace_ids": [id], "before_workspace_id": before
            }),
        )?;
        self.parse_workspace_move(&doc, id, before)
    }
    fn create_workspace(
        &self,
        name: &str,
        cwd: &str,
    ) -> Result<crate::projects::Workspace, SendError> {
        let doc = self.request(&[
            "workspace",
            "create",
            "--label",
            name,
            "--cwd",
            cwd,
            "--no-focus",
        ])?;
        let parse = || {
            let w = doc
                .pointer("/result/workspace")
                .ok_or("herdr.invalid_response")?;
            Ok(crate::projects::Workspace {
                id: field(w, "workspace_id")?.into(),
                name: field(w, "label")?.into(),
                directory: Some(cwd.into()),
            })
        };
        parse().map_err(|message: String| SendError {
            message,
            uncertain: true,
        })
    }
    fn rename_workspace(&self, id: &str, name: &str) -> Result<(), String> {
        self.json(&["workspace", "rename", id, name]).map(|_| ())
    }
    fn available(&self, workspace: &str, cwd: &str) -> Result<Vec<Target>, String> {
        if let Some(machine) = &self.remote {
            let mut targets: Vec<Target> = crate::remote_transport::request(
                machine,
                serde_json::json!({"method":"available_targets","workspace":workspace,"cwd":cwd}),
            )?;
            for target in &mut targets {
                target.machine_id = machine.id.clone();
            }
            return Ok(targets);
        }
        let workspace_id = self.resolve_workspace_id(workspace)?;
        let Some(tab) = self.kanban_tab(&workspace_id)? else {
            return Ok(Vec::new());
        };
        let doc = self.json(&["pane", "list", "--workspace", &workspace_id])?;
        let panes = doc
            .pointer("/result/panes")
            .and_then(Value::as_array)
            .ok_or("herdr.invalid_response")?;
        let mut targets = Vec::new();
        for pane in panes {
            if pane.get("tab_id").and_then(Value::as_str) != Some(tab.as_str()) {
                continue;
            }
            let id = field(pane, "pane_id")?;
            if let Ok((target, info)) = self.inspect_pane(&workspace_id, id) {
                if info.get("tab_id").and_then(Value::as_str) != Some(tab.as_str()) {
                    continue;
                }
                let dir = info
                    .get("foreground_cwd")
                    .and_then(Value::as_str)
                    .or_else(|| info.get("cwd").and_then(Value::as_str));
                let directory_matches = if let Some(dir) = dir {
                    if let Some(machine) = &self.remote {
                        crate::remote_transport::request(
                            machine,
                            serde_json::json!({"method":"same_directory","a":dir,"b":cwd}),
                        )?
                    } else {
                        same_directory(dir, cwd)
                    }
                } else {
                    false
                };
                if directory_matches && self.ready(&target, &info) {
                    targets.push(target);
                }
            }
        }
        targets.sort_by(|a, b| {
            (a.agent.as_str(), a.pane_id.as_str()).cmp(&(b.agent.as_str(), b.pane_id.as_str()))
        });
        Ok(targets)
    }
    fn idle_panes(&self, workspace: &str) -> Result<Vec<crate::idle_cleanup::IdlePane>, String> {
        self.cleanup_panes(workspace)
    }
    fn cleanup_target_absent(&self, target: &Target) -> Result<bool, String> {
        // Scan the whole session: moving a pane to another Workspace is not closure.
        let doc = self.json(&["pane", "list"])?;
        let panes = doc
            .pointer("/result/panes")
            .and_then(Value::as_array)
            .ok_or("herdr.invalid_response")?;
        for pane in panes {
            if field(pane, "terminal_id")? == target.terminal_id {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn close_idle_pane(
        &self,
        pane: &crate::idle_cleanup::IdlePane,
        check: &dyn Fn() -> Result<(), String>,
    ) -> Result<(), SendError> {
        self.cleanup_close(pane, check)
    }
    fn pane_inventory(
        &self,
        workspace: &str,
    ) -> Result<Option<Vec<crate::workflow::PaneIdentity>>, String> {
        let panes = self.workspace_panes(workspace)?;
        panes
            .iter()
            .map(|pane| {
                Ok(crate::workflow::PaneIdentity {
                    pane_id: field(pane, "pane_id")?.into(),
                    terminal_id: field(pane, "terminal_id")?.into(),
                })
            })
            .collect::<Result<Vec<_>, String>>()
            .map(Some)
    }
    fn create_pane(
        &self,
        workspace: &str,
        cwd: &str,
        label: &str,
        check_current: &dyn Fn() -> Result<(), String>,
    ) -> Result<Target, String> {
        check_current()?;
        if !self.resources().directory_exists(cwd)? {
            return Err("flow.directory_missing".into());
        }
        let id = self.resolve_workspace_id(workspace)?;
        check_current()?;
        let doc;
        let pane = if let Some(tab) = self.kanban_tab(&id)? {
            let (anchor, direction) = self.split_anchor(&id, &tab)?;
            check_current()?;
            doc = self.json(&[
                "pane",
                "split",
                "--pane",
                &anchor,
                "--direction",
                direction,
                "--cwd",
                cwd,
                "--no-focus",
            ])?;
            doc.pointer("/result/pane")
        } else {
            check_current()?;
            doc = self.json(&[
                "tab",
                "create",
                "--workspace",
                &id,
                "--cwd",
                cwd,
                "--label",
                "kanban",
                "--no-focus",
            ])?;
            doc.pointer("/result/root_pane")
        }
        .ok_or("herdr.invalid_response")?;
        if field(pane, "workspace_id")? != id {
            return Err("binding.identity_changed".into());
        }
        let pane_id = field(pane, "pane_id")?;
        // The pane identity is recorded even when shell initialization has not finished.
        Ok(Target {
            machine_id: self.machine_id().into(),
            session: self.session.clone(),
            workspace_id: id,
            pane_id: pane_id.into(),
            terminal_id: field(pane, "terminal_id")?.into(),
            agent: String::new(),
            process_id: String::new(),
            codex_resume_locks: Vec::new(),
            title: label.into(),
            status: "unknown".into(),
        })
    }
    fn prepare_codex(
        &self,
        target: &Target,
        cwd: &str,
        resume: Option<&str>,
        name: &str,
        check_current: &dyn Fn() -> Result<(), String>,
    ) -> Result<Target, String> {
        self.prepare_codex_session(
            target,
            cwd,
            resume,
            name,
            &Default::default(),
            check_current,
        )
    }
    fn prepare_codex_configured(
        &self,
        target: &Target,
        cwd: &str,
        resume: Option<&str>,
        name: &str,
        settings: &crate::codex_settings::Settings,
        check_current: &dyn Fn() -> Result<(), String>,
    ) -> Result<Target, String> {
        self.prepare_codex_session(target, cwd, resume, name, settings, check_current)
    }
    fn codex_catalog(&self, cwd: &str) -> Result<crate::codex_catalog::Catalog, String> {
        if let Some(machine) = &self.remote {
            return crate::remote_transport::request(
                machine,
                serde_json::json!({"method":"codex_catalog","cwd":cwd}),
            );
        }
        crate::codex_catalog::local(cwd)
    }
    fn session_id(&self, target: &Target) -> Result<Option<String>, String> {
        if let Some(machine) = &self.remote {
            return crate::remote_transport::request(
                machine,
                serde_json::json!({"method":"session_id","target":target}),
            );
        }
        self.read_codex_session(target)
    }
    fn pane_exists(&self, target: &Target) -> Result<bool, String> {
        if target.session != self.session {
            return Err("binding.identity_changed".into());
        }
        let doc = self.json(&["pane", "list", "--workspace", &target.workspace_id])?;
        let panes = doc
            .pointer("/result/panes")
            .and_then(Value::as_array)
            .ok_or("herdr.invalid_response")?;
        // Missing IDs are definitive only after a successful list response, never from a CLI error.
        Ok(panes
            .iter()
            .any(|p| p.get("pane_id").and_then(Value::as_str) == Some(&target.pane_id)))
    }
    fn close_pane(
        &self,
        target: &Target,
        check_current: &dyn Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        check_current()?;
        self.check_target(target).map_err(|e| e.message)?;
        check_current()?;
        self.json(&["pane", "close", &target.pane_id])?;
        Ok(())
    }
    fn find_created(&self, workspace: &str, label: &str) -> Result<Option<Target>, String> {
        let id = self.resolve_workspace_id(workspace)?;
        let doc = self.json(&["tab", "list", "--workspace", &id])?;
        let tabs = doc
            .pointer("/result/tabs")
            .and_then(Value::as_array)
            .ok_or("herdr.invalid_response")?;
        let matches: Vec<_> = tabs
            .iter()
            .filter(|t| t.get("label").and_then(Value::as_str) == Some(label))
            .collect();
        if matches.is_empty() {
            return Ok(None);
        }
        // Do not reset an ambiguous creation; ask for explicit inspection of the labelled tab.
        Err("flow.close_before_reset".into())
    }
    fn validate(&self, workspace: &str, expected: &Target) -> Result<Target, String> {
        if self.resolve_workspace_id(workspace)? != expected.workspace_id {
            return Err("binding.workspace_ambiguous".into());
        }
        let doc = self.json(&["agent", "get", &expected.pane_id])?;
        let mut live = self.live_target(
            &expected.workspace_id,
            doc.pointer("/result/agent").ok_or("binding.missing")?,
        )?;
        if live.same_identity(expected) {
            live.codex_resume_locks = expected.codex_resume_locks.clone();
            Ok(live)
        } else {
            Err("binding.identity_changed".into())
        }
    }
    fn targets(&self, workspace: &str) -> Result<Vec<Target>, String> {
        if let Some(machine) = &self.remote {
            let mut targets: Vec<Target> = crate::remote_transport::request(
                machine,
                serde_json::json!({"method":"targets","workspace":workspace}),
            )?;
            for target in &mut targets {
                target.machine_id = machine.id.clone();
            }
            return Ok(targets);
        }
        let id = self.resolve_workspace_id(workspace)?;
        let doc = self.json(&["agent", "list"])?;
        let agents = doc
            .pointer("/result/agents")
            .and_then(Value::as_array)
            .ok_or("herdr.invalid_response")?;
        Ok(agents
            .iter()
            .filter(|a| a.get("workspace_id").and_then(Value::as_str) == Some(id.as_str()))
            .filter_map(|a| self.live_target(&id, a).ok())
            .collect())
    }
    fn discover_agents(&self, workspace: &str) -> Result<crate::agent_status::Discovery, String> {
        self.discover_agent_statuses(workspace)
    }
    fn send(&self, target: &Target, prompt: &str) -> Result<(), SendError> {
        self.send_checked(target, prompt, &|| Ok(()))
    }
    fn send_checked(
        &self,
        target: &Target,
        prompt: &str,
        check_current: &dyn Fn() -> Result<(), String>,
    ) -> Result<(), SendError> {
        self.send_prompt_checked(target, prompt, check_current)
    }
    fn send_checking_receipt(
        &self,
        target: &Target,
        prompt: &str,
        check_current: &dyn Fn() -> Result<(), String>,
        received: &dyn Fn() -> Result<bool, String>,
    ) -> Result<(), SendError> {
        if self.remote.is_some() {
            self.send_prompt_with_receipt(target, prompt, check_current, received)
        } else {
            self.send_prompt_checked(target, prompt, check_current)
        }
    }
}

fn same_directory(a: &str, b: &str) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}
