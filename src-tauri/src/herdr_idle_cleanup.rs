//! Live eligibility and guarded close, executed on the machine owning the pane.
use super::{Cli, field};
use crate::{
    idle_cleanup::IdlePane,
    workflow::{Herdr, SendError},
};

#[cfg(all(test, windows))]
#[path = "herdr_idle_cleanup_tests.rs"]
mod tests;

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct CloseReply {
    pub error: Option<String>,
    pub uncertain: bool,
}

impl Cli {
    fn inspect_idle_cleanup(
        &self,
        workspace: &str,
        tab: &str,
        pane: &str,
    ) -> Result<Option<IdlePane>, String> {
        let (target, info) = self.inspect_pane(workspace, pane)?;
        if info.get("tab_id").and_then(serde_json::Value::as_str) != Some(tab)
            || !self.ready(&target, &info)
        {
            return Ok(None);
        }
        // Herdr does not expose a reliable shell command-completion state. Even
        // a shell with no children may be running Start-Sleep/Read-Host/builtins.
        // Never use the foreground group or screen text to authorize its closure.
        if target.agent != "codex" {
            return Ok(None);
        }
        let cwd = info
            .get("foreground_cwd")
            .or_else(|| info.get("cwd"))
            .and_then(serde_json::Value::as_str)
            .ok_or("herdr.invalid_response")?;
        Ok(Some(IdlePane {
            target,
            tab_id: tab.into(),
            cwd: cwd.into(),
        }))
    }
    pub(super) fn cleanup_panes(&self, workspace: &str) -> Result<Vec<IdlePane>, String> {
        if let Some(machine) = &self.remote {
            let mut panes: Vec<IdlePane> = crate::remote_transport::request(
                machine,
                serde_json::json!({"method":"idle_cleanup_panes","workspace":workspace}),
            )?;
            for pane in &mut panes {
                pane.target.machine_id = machine.id.clone();
            }
            return Ok(panes);
        }
        let Some(tab) = self.kanban_tab(workspace)? else {
            return Ok(vec![]);
        };
        let mut result = vec![];
        for pane in self.workspace_panes(workspace)? {
            if pane.get("tab_id").and_then(serde_json::Value::as_str) != Some(&tab) {
                continue;
            }
            match self.inspect_idle_cleanup(workspace, &tab, field(&pane, "pane_id")?) {
                Ok(Some(pane)) => result.push(pane),
                Ok(None) => {}
                Err(_) => {
                    // A disappearing/unreadable target is not authority to close it.
                    // Keep scanning only if the enclosing scope is still readable;
                    // connection failures and Tab ambiguity must reach the UI.
                    if self.kanban_tab(workspace)?.as_deref() != Some(&tab) {
                        return Err("cleanup.changed".into());
                    }
                    self.workspace_panes(workspace)?;
                }
            }
        }
        Ok(result)
    }
    pub(super) fn cleanup_close(
        &self,
        pane: &IdlePane,
        check: &dyn Fn() -> Result<(), String>,
    ) -> Result<(), SendError> {
        let certain = |message: String| SendError {
            message,
            uncertain: false,
        };
        if pane.target.machine_id != self.machine_id() || pane.target.session != self.session {
            return Err(certain("binding.identity_changed".into()));
        }
        if let Some(machine) = &self.remote {
            check().map_err(certain)?;
            let response: CloseReply = crate::remote_transport::request(
                machine,
                serde_json::json!({"method":"idle_cleanup_close","pane":pane}),
            )
            .map_err(|message| SendError {
                message,
                uncertain: true,
            })?;
            return match response.error {
                Some(message) => Err(SendError {
                    message,
                    uncertain: response.uncertain,
                }),
                None => Ok(()),
            };
        }
        let tab = self
            .kanban_tab(&pane.target.workspace_id)
            .map_err(certain)?;
        if tab.as_deref() != Some(&pane.tab_id) {
            return Err(certain("cleanup.changed".into()));
        }
        let live = self
            .inspect_idle_cleanup(
                &pane.target.workspace_id,
                &pane.tab_id,
                &pane.target.pane_id,
            )
            .map_err(certain)?;
        if !live.is_some_and(|p| p.target.same_identity(&pane.target)) {
            return Err(certain("cleanup.changed".into()));
        }
        check().map_err(certain)?;
        self.request(&["pane", "close", &pane.target.pane_id])
            .map(|_| ())
    }
}
