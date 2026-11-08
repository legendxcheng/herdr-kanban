use super::Cli;
use crate::{startup_prompt::Request, workflow::Target};

impl Cli {
    pub(crate) fn validate_startup_context(&self, request: &Request) -> Result<(), String> {
        let (live, _) = self.inspect_pane(&request.target.workspace_id, &request.target.pane_id)?;
        if live.terminal_id != request.target.terminal_id
            || live.session != request.target.session
            || live.process_id.split(':').next() != request.target.process_id.split(':').next()
            || !live.agent.is_empty()
        {
            return Err("binding.identity_changed".into());
        }
        Ok(())
    }
    pub(crate) fn stage_startup_prompt(&self, request: &Request) -> Result<String, String> {
        crate::codex_launcher::stage(request)
    }
    pub(crate) fn observe_startup_prompt(
        &self,
        request: &Request,
    ) -> Result<Option<Target>, String> {
        let Some(started) = crate::codex_launcher::started(request)? else {
            return Ok(None);
        };
        let (live, _) = self.inspect_pane(&request.target.workspace_id, &request.target.pane_id)?;
        let expected = &request.target;
        if live.terminal_id != expected.terminal_id
            || live.session != expected.session
            || live.process_id.split(':').next() != expected.process_id.split(':').next()
        {
            return Err("binding.identity_changed".into());
        }
        // Agent metadata may lag the native process. Status/idle is not a gate.
        if live.agent != "codex" {
            return Ok(None);
        }
        if live.process_id.rsplit(':').next() != Some(started.pid.to_string().as_str()) {
            return Err("binding.identity_changed".into());
        }
        Ok(Some(live))
    }
    pub(super) fn launch_startup_prompt(
        &self,
        request: &Request,
        check: &dyn Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        check()?;
        let command: String = (if let Some(machine) = &self.remote {
            crate::remote_transport::request(
                machine,
                serde_json::json!({"method":"stage_startup_prompt","request":request}),
            )
        } else {
            self.stage_startup_prompt(request)
        })
        .map_err(|error| format!("startup.not_launched:{error}"))?;
        check()?;
        let (live, _) = self.inspect_pane(&request.target.workspace_id, &request.target.pane_id)?;
        if !live.same_identity(&request.target)
            || !live.agent.is_empty()
            || !live
                .process_id
                .split_once(':')
                .is_some_and(|(shell, group)| shell == group)
        {
            return Err("binding.identity_changed".into());
        }
        check()?;
        self.output(&["pane", "run", &live.pane_id, &command])
            .map(|_| ())
            .map_err(|e| e.message)
    }
}
