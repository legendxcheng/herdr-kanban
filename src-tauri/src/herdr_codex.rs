//! Codex startup, exact session recovery and guarded prompt submission.
use super::{Cli, codex_process, field};
use crate::workflow::{Herdr, SendError, Target};
use serde_json::Value;
use std::{
    thread,
    time::{Duration, Instant},
};

impl Cli {
    pub(crate) fn startup_session_locks(
        &self,
        target: &Target,
    ) -> Result<Option<Vec<String>>, String> {
        if let Some(machine) = &self.remote {
            return crate::remote_transport::request(
                machine,
                serde_json::json!({"method":"session_locks","target":target}),
            );
        }
        self.check_target(target).map_err(|e| e.message)?;
        #[cfg(windows)]
        let locks = target
            .process_id
            .rsplit(':')
            .next()
            .and_then(|s| s.parse().ok())
            .and_then(codex_process::session_locks);
        #[cfg(not(windows))]
        let locks = None;
        self.check_target(target).map_err(|e| e.message)?;
        Ok(locks)
    }
    pub(super) fn prepare_codex_session(
        &self,
        target: &Target,
        cwd: &str,
        resume: Option<&str>,
        name: &str,
        settings: &crate::codex_settings::Settings,
        check_current: &dyn Fn() -> Result<(), String>,
    ) -> Result<Target, String> {
        check_current()?;
        settings.validate()?;
        if let Some(id) = resume {
            uuid::Uuid::parse_str(id).map_err(|_| "flow.session_invalid")?;
        }
        if target.session != self.session {
            return Err("binding.identity_changed".into());
        }
        let (live, info) = self.inspect_pane(&target.workspace_id, &target.pane_id)?;
        if live.terminal_id != target.terminal_id
            || live.agent != target.agent
            || (!target.process_id.is_empty() && !live.same_identity(target))
        {
            return Err("binding.identity_changed".into());
        }
        if live.agent == "codex" {
            if !self.ready(&live, &info) {
                return Err("flow.not_idle".into());
            }
            self.check_target(&live).map_err(|e| e.message)?;
            check_current()?;
            self.json(&["agent", "send-keys", &live.pane_id, "ctrl+u"])?;
            self.send_checked(&live, "/quit", check_current)
                .map_err(|e| e.message)?;
        } else if !live.agent.is_empty() {
            return Err("flow.not_idle".into());
        }
        let mut args = vec![
            "agent",
            "start",
            name,
            "--kind",
            "codex",
            "--pane",
            &target.pane_id,
            "--",
            // Hooks must inherit this pane's environment, including on resume.
            // A shared app-server may retain another pane's HERDR_PANE_ID.
            "--no-daemon",
            "--approve-for-me",
        ];
        if let Some(id) = resume {
            args.extend(["resume", id]);
        }
        args.extend(["-C", cwd]);
        let model_args = settings.args();
        args.extend(model_args.iter().map(String::as_str));
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut ready_shell: Option<Target> = None;
        let mut rejection = None;
        loop {
            check_current()?;
            let (shell, info) = self.inspect_pane(&target.workspace_id, &target.pane_id)?;
            // During exit the Agent registry may still describe the old Codex
            // even though process inspection already reports its parent shell.
            let exiting_agent = shell.agent == live.agent
                && (shell.same_identity(&live)
                    || shell
                        .process_id
                        .split_once(':')
                        .is_some_and(|(pid, group)| pid == group));
            if shell.terminal_id != target.terminal_id
                || (!target.process_id.is_empty()
                    && shell.process_id.split(':').next() != target.process_id.split(':').next())
                || (!shell.agent.is_empty() && !exiting_agent)
            {
                return Err("binding.identity_changed".into());
            }
            if Instant::now() >= deadline {
                return Err(rejection.unwrap_or_else(|| "flow.not_idle".into()));
            }
            if shell.agent.is_empty() && self.ready(&shell, &info) {
                if ready_shell
                    .as_ref()
                    .is_some_and(|old| !old.same_identity(&shell))
                {
                    return Err("binding.identity_changed".into());
                }
                ready_shell = Some(shell);
                check_current()?;
                match self.json(&args) {
                    Ok(_) => break,
                    // Windows process inspection can see the shell before Herdr's
                    // cached Agent state catches up. Only an explicit rejection
                    // is safe to retry; never repeat an uncertain launch.
                    Err(error) if super::agent_start_rejected_busy(&error) => {
                        rejection = Some(error);
                    }
                    Err(error) => return Err(error),
                }
            }
            thread::sleep(Duration::from_millis(150));
        }
        // agent.start acknowledges the launch, not necessarily interactive readiness.
        // In particular, a reused pane may still be detecting the new Codex process.
        let (mut live, info) = self.wait_started_codex(
            ready_shell.as_ref().ok_or("binding.identity_unknown")?,
            check_current,
        )?;
        if let Some(expected) = resume {
            let deadline = Instant::now() + Duration::from_secs(5);
            #[cfg(windows)]
            let needs_startup_locks = codex_session_id(&info)?.as_deref() != Some(expected);
            loop {
                check_current()?;
                // Capture only our own newly started process, before UI input is enabled.
                // A running binding must retain this snapshot, never recapture it.
                #[cfg(windows)]
                if needs_startup_locks && live.codex_resume_locks.is_empty() {
                    if let Some(ids) = self.startup_session_locks(&live)? {
                        if ids.len() == 2 && ids.iter().any(|id| id == expected) {
                            self.check_target(&live).map_err(|e| e.message)?;
                            live.codex_resume_locks = ids;
                        }
                    }
                    // Herdr can report interactive readiness before Codex has
                    // finished opening the resumed and auto-review threads.
                    if live.codex_resume_locks.is_empty() {
                        if Instant::now() >= deadline {
                            return Err("flow.session_mismatch".into());
                        }
                        thread::sleep(Duration::from_millis(150));
                        continue;
                    }
                }
                if self.session_id(&live)?.as_deref() == Some(expected) {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err("flow.session_mismatch".into());
                }
                thread::sleep(Duration::from_millis(150));
            }
        }
        Ok(live)
    }
    fn wait_started_codex(
        &self,
        shell: &Target,
        check_current: &dyn Fn() -> Result<(), String>,
    ) -> Result<(Target, Value), String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut observed: Option<Target> = None;
        loop {
            check_current()?;
            match self.inspect_pane(&shell.workspace_id, &shell.pane_id) {
                Ok((live, info)) => {
                    if live.terminal_id != shell.terminal_id
                        || live.process_id.split(':').next() != shell.process_id.split(':').next()
                        || observed
                            .as_ref()
                            .is_some_and(|old| !old.same_identity(&live))
                        || (!live.agent.is_empty() && live.agent != "codex")
                    {
                        return Err("binding.identity_changed".into());
                    }
                    if live.agent == "codex" && live.process_id != shell.process_id {
                        observed.get_or_insert_with(|| live.clone());
                        if self.ready(&live, &info) {
                            check_current()?;
                            return Ok((live, info));
                        }
                    }
                }
                Err(error) if error == "binding.identity_unknown" => {}
                Err(error) => return Err(error),
            }
            if Instant::now() >= deadline {
                return Err("flow.not_idle".into());
            }
            thread::sleep(Duration::from_millis(150));
        }
    }
    pub(super) fn read_codex_session(&self, target: &Target) -> Result<Option<String>, String> {
        self.check_target(target).map_err(|e| e.message)?;
        let doc = self.json(&["agent", "get", &target.pane_id])?;
        let agent = doc
            .pointer("/result/agent")
            .ok_or("herdr.invalid_response")?;
        let live = self.live_target(&target.workspace_id, agent)?;
        if !live.same_identity(target) {
            return Err("binding.identity_changed".into());
        }
        if let Some(id) = codex_session_id(agent)? {
            return Ok(Some(id));
        }
        // Codex defers the resume SessionStart hook until the first new input.
        // Before then, accept only our exact, independently observed resume
        // command and unique session write lock in the same live foreground process.
        // Never use a saved ID or another pane's report as evidence.
        if agent.get("interactive_ready").and_then(Value::as_bool) != Some(true) {
            return Ok(None);
        }
        let info = self.json(&["pane", "process-info", "--pane", &target.pane_id])?;
        let id = codex_resume_session(&info, target)?;
        self.check_target(target).map_err(|e| e.message)?;
        Ok(id)
    }
    pub(super) fn send_prompt_checked(
        &self,
        target: &Target,
        prompt: &str,
        check_current: &dyn Fn() -> Result<(), String>,
    ) -> Result<(), SendError> {
        self.send_prompt_with_receipt(target, prompt, check_current, &|| Ok(false))
    }
    pub(super) fn send_prompt_with_receipt(
        &self,
        target: &Target,
        prompt: &str,
        check_current: &dyn Fn() -> Result<(), String>,
        received: &dyn Fn() -> Result<bool, String>,
    ) -> Result<(), SendError> {
        let check = || {
            check_current().map_err(|message| SendError {
                message,
                uncertain: false,
            })
        };
        check()?;
        // Revalidate immediately before sending, after the ownership reservation.
        self.check_target(target)?;
        #[cfg(windows)]
        if target.agent == "codex" {
            return self.submit_windows_codex(target, prompt, check_current, received);
        }
        check()?;
        self.request(&["agent", "prompt", &target.pane_id, prompt])
            .map(|_| ())
    }
    pub(super) fn inspect_pane(
        &self,
        workspace: &str,
        pane: &str,
    ) -> Result<(Target, Value), String> {
        let doc = self.json(&["pane", "get", pane])?;
        let mut info = doc
            .pointer("/result/pane")
            .ok_or("herdr.invalid_response")?
            .clone();
        if info
            .get("agent")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
        {
            let doc = self.json(&["agent", "get", pane])?;
            let current = doc
                .pointer("/result/agent")
                .filter(|v| v.is_object())
                .ok_or("herdr.invalid_response")?;
            for key in ["workspace_id", "pane_id", "terminal_id"] {
                if field(current, key)? != field(&info, key)? {
                    return Err("binding.identity_changed".into());
                }
            }
            info = current.clone();
            match info.get("agent") {
                Some(Value::String(agent)) if !agent.is_empty() => {
                    return Ok((self.live_target(workspace, &info)?, info));
                }
                None | Some(Value::Null) | Some(Value::String(_)) => {
                    // The old Agent can disappear between pane.get and agent.get.
                    // Treat absence as a candidate shell, then require the process
                    // check below to prove its foreground group equals its shell PID.
                }
                _ => return Err("herdr.invalid_response".into()),
            }
        }
        if field(&info, "workspace_id")? != workspace {
            return Err("binding.identity_changed".into());
        }
        let process = self.json(&["pane", "process-info", "--pane", pane])?;
        let p = process
            .pointer("/result/process_info")
            .ok_or("herdr.invalid_response")?;
        let shell = p.get("shell_pid").and_then(Value::as_u64).unwrap_or(0);
        let group = p
            .get("foreground_process_group_id")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let shell_ready = shell != 0 && shell == group;
        Ok((
            Target {
                machine_id: self.machine_id().into(),
                session: self.session.clone(),
                workspace_id: workspace.into(),
                pane_id: pane.into(),
                terminal_id: field(&info, "terminal_id")?.into(),
                agent: String::new(),
                process_id: format!("{shell}:{group}"),
                codex_resume_locks: Vec::new(),
                title: String::new(),
                status: if shell_ready { "idle" } else { "unknown" }.into(),
            },
            info,
        ))
    }
    pub(super) fn ready(&self, target: &Target, info: &Value) -> bool {
        if target.agent.is_empty() {
            return target.status == "idle";
        }
        target.agent == "codex"
            && matches!(target.status.as_str(), "idle" | "done")
            && info.get("interactive_ready").and_then(Value::as_bool) == Some(true)
            && info.get("launch_pending").and_then(Value::as_bool) != Some(true)
    }
}

fn codex_session_id(agent: &Value) -> Result<Option<String>, String> {
    let Some(session) = agent.get("agent_session").filter(|s| !s.is_null()) else {
        return Ok(None);
    };
    if session.get("agent").and_then(Value::as_str) != Some("codex")
        || session.get("kind").and_then(Value::as_str) != Some("id")
    {
        return Ok(None);
    };
    let id = field(session, "value")?;
    uuid::Uuid::parse_str(id).map_err(|_| "flow.session_invalid")?;
    Ok(Some(id.into()))
}

fn codex_resume_session(doc: &Value, target: &Target) -> Result<Option<String>, String> {
    let info = doc
        .pointer("/result/process_info")
        .ok_or("herdr.invalid_response")?;
    let shell = info["shell_pid"]
        .as_u64()
        .ok_or("binding.identity_unknown")?;
    let group = info["foreground_process_group_id"]
        .as_u64()
        .ok_or("binding.identity_unknown")?;
    if target.agent != "codex"
        || info["pane_id"].as_str() != Some(&target.pane_id)
        || group == 0
        || format!("{shell}:{group}") != target.process_id
    {
        return Err("binding.identity_changed".into());
    }
    let processes = info["foreground_processes"]
        .as_array()
        .ok_or("herdr.invalid_response")?;
    let candidates: Vec<_> = processes
        .iter()
        .filter(|p| p["pid"].as_u64() == Some(group))
        .collect();
    let [process] = candidates.as_slice() else {
        return Ok(None);
    };
    let Some(argv) = process["argv"]
        .as_array()
        .and_then(|a| a.iter().map(Value::as_str).collect::<Option<Vec<_>>>())
    else {
        return Ok(None);
    };
    let Some(id) = codex_resume_command_session(&argv)? else {
        return Ok(None);
    };
    if codex_process::owns_session(group, id, &target.codex_resume_locks) {
        Ok(Some(id.into()))
    } else {
        Err("flow.session_mismatch".into())
    }
}

fn codex_resume_command_session<'a>(argv: &[&'a str]) -> Result<Option<&'a str>, String> {
    let argv = match argv {
        [node, script, rest @ ..]
            if node.rsplit(['/', '\\']).next().is_some_and(|name| {
                matches!(name.to_ascii_lowercase().as_str(), "node" | "node.exe")
            }) && script
                .replace('\\', "/")
                .to_ascii_lowercase()
                .ends_with("/node_modules/@openai/codex/bin/codex.js") =>
        {
            // Validate the same exact launch arguments as the native binary.
            let mut native = vec!["codex"];
            native.extend_from_slice(rest);
            return codex_resume_command_session(&native);
        }
        other => other,
    };
    // Accept only launches we issue, including panes started before automatic
    // review became the default. Unknown flags must not weaken resume evidence.
    let (exe, id, cwd, rest) = match argv {
        [
            exe,
            "--no-daemon",
            "--approve-for-me",
            "resume",
            id,
            "-C",
            cwd,
            rest @ ..,
        ]
        | [exe, "--no-daemon", "resume", id, "-C", cwd, rest @ ..] => (*exe, *id, *cwd, rest),
        _ => return Ok(None),
    };
    let (model, effort) = match rest {
        [] => (None, None),
        ["--model", model] => (Some(*model), None),
        ["-c", effort] => (None, Some(*effort)),
        ["--model", model, "-c", effort] => (Some(*model), Some(*effort)),
        _ => return Ok(None),
    };
    let effort = match effort {
        Some(value) => match value
            .strip_prefix("model_reasoning_effort=\"")
            .and_then(|v| v.strip_suffix('"'))
        {
            Some(effort) => Some(effort.to_string()),
            None => return Ok(None),
        },
        None => None,
    };
    if (crate::codex_settings::Settings {
        model: model.map(str::to_string),
        effort,
    })
    .validate()
    .is_err()
    {
        return Ok(None);
    }
    let name = exe.rsplit(['/', '\\']).next().unwrap_or("");
    if !matches!(name.to_ascii_lowercase().as_str(), "codex" | "codex.exe") || cwd.is_empty() {
        return Ok(None);
    }
    uuid::Uuid::parse_str(id).map_err(|_| "flow.session_invalid")?;
    Ok(Some(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_evidence_accepts_automatic_review_and_existing_legacy_panes() {
        let id = "22222222-2222-4222-8222-222222222222";
        for argv in [
            vec![
                "codex",
                "--no-daemon",
                "--approve-for-me",
                "resume",
                id,
                "-C",
                "E:/project",
                "--model",
                "test-model",
                "-c",
                "model_reasoning_effort=\"high\"",
            ],
            vec![
                "C:\\Program Files\\Codex\\codex.exe",
                "--no-daemon",
                "--approve-for-me",
                "resume",
                id,
                "-C",
                "E:\\Project with spaces",
            ],
            vec!["codex", "--no-daemon", "resume", id, "-C", "E:/project"],
            vec![
                "C:/Program Files/nodejs/node.exe",
                "C:/Users/test/npm/node_modules/@openai/codex/bin/codex.js",
                "--no-daemon",
                "--approve-for-me",
                "resume",
                id,
                "-C",
                "C:/remote project",
            ],
        ] {
            assert_eq!(codex_resume_command_session(&argv).unwrap(), Some(id));
        }
    }

    #[test]
    fn resume_evidence_rejects_unknown_launches_and_invalid_sessions() {
        let id = "22222222-2222-4222-8222-222222222222";
        for argv in [
            vec!["codex", "resume", id, "-C", "E:/project"],
            vec!["other.exe", "--no-daemon", "resume", id, "-C", "E:/project"],
            vec![
                "node.exe",
                "C:/other/codex.js",
                "--no-daemon",
                "resume",
                id,
                "-C",
                "E:/project",
            ],
            vec![
                "node.exe",
                "C:/npm/node_modules/@openai/codex/bin/codex.js",
                "--no-daemon",
                "resume",
                id,
                "-C",
                "E:/project",
                "unexpected prompt",
            ],
            vec!["codex", "--no-daemon", "resume", id, "-C", ""],
            vec![
                "codex",
                "--no-daemon",
                "--approve-for-me",
                "resume",
                id,
                "-C",
                "E:/project",
                "unexpected prompt",
            ],
            vec![
                "codex",
                "--no-daemon",
                "--dangerously-bypass-approvals-and-sandbox",
                "resume",
                id,
                "-C",
                "E:/project",
            ],
        ] {
            assert_eq!(codex_resume_command_session(&argv).unwrap(), None);
        }
        assert_eq!(
            codex_resume_command_session(&[
                "codex",
                "--no-daemon",
                "--approve-for-me",
                "resume",
                "invalid",
                "-C",
                "E:/project",
            ])
            .unwrap_err(),
            "flow.session_invalid"
        );
    }
}
