//! Plain terminal identity is independent of the foreground command and Agent status.
use super::{Cli, field};
use crate::{
    workflow::{Herdr, SendError},
    workspace_terminal::Target,
};
use serde_json::Value;

fn target(cli: &Cli, pane: &Value) -> Result<Target, String> {
    Ok(Target {
        machine_id: cli.machine_id().into(),
        session: cli.session.clone(),
        workspace_id: field(pane, "workspace_id")?.into(),
        tab_id: field(pane, "tab_id")?.into(),
        pane_id: field(pane, "pane_id")?.into(),
        terminal_id: field(pane, "terminal_id")?.into(),
    })
}
fn label(operation: &str) -> String {
    format!("kanban-terminal-{operation}")
}

impl Cli {
    pub(crate) fn shell_live(&self, expected: &Target) -> Result<bool, String> {
        if expected.machine_id != self.machine_id() || expected.session != self.session {
            return Err("binding.identity_changed".into());
        }
        self.resolve_workspace_id(&expected.workspace_id)?;
        let doc = self.json(&["pane", "list"])?;
        let panes = doc
            .pointer("/result/panes")
            .and_then(Value::as_array)
            .ok_or("herdr.invalid_response")?;
        let mut found = false;
        for pane in panes {
            let live = target(self, pane)?;
            if live.terminal_id == expected.terminal_id {
                if live != *expected || found {
                    return Err("binding.identity_changed".into());
                }
                found = true;
            } else if live.pane_id == expected.pane_id {
                return Err("binding.identity_changed".into());
            }
        }
        Ok(found)
    }

    pub(crate) fn shell_find(
        &self,
        workspace: &str,
        operation: &str,
    ) -> Result<Option<Target>, String> {
        self.resolve_workspace_id(workspace)?;
        let doc = self.json(&["tab", "list", "--workspace", workspace])?;
        let tabs = doc
            .pointer("/result/tabs")
            .and_then(Value::as_array)
            .ok_or("herdr.invalid_response")?;
        let matches: Vec<_> = tabs
            .iter()
            .filter(|tab| tab["label"].as_str() == Some(label(operation).as_str()))
            .collect();
        if matches.len() > 1 {
            return Err("workspace_terminal.ambiguous".into());
        }
        let Some(tab) = matches.first() else {
            return Ok(None);
        };
        if field(tab, "workspace_id")? != workspace {
            return Err("binding.identity_changed".into());
        }
        let tab_id = field(tab, "tab_id")?;
        let panes = self.workspace_panes(workspace)?;
        let matches: Vec<_> = panes
            .iter()
            .filter(|pane| pane["tab_id"].as_str() == Some(tab_id))
            .collect();
        if matches.len() != 1 {
            return Err("workspace_terminal.ambiguous".into());
        }
        Ok(Some(target(self, matches[0])?))
    }

    pub(crate) fn shell_create(
        &self,
        workspace: &str,
        cwd: &str,
        operation: &str,
        check: &dyn Fn() -> Result<(), String>,
    ) -> Result<Target, SendError> {
        check().map_err(|message| SendError {
            message,
            uncertain: false,
        })?;
        let doc = self.request(&[
            "tab",
            "create",
            "--workspace",
            workspace,
            "--cwd",
            cwd,
            "--label",
            &label(operation),
            "--no-focus",
        ])?;
        let result = (|| {
            let pane = doc
                .pointer("/result/root_pane")
                .ok_or("herdr.invalid_response")?;
            let target = target(self, pane)?;
            if target.workspace_id != workspace {
                return Err("binding.identity_changed".into());
            }
            Ok(target)
        })();
        result.map_err(|message| SendError {
            message,
            uncertain: true,
        })
    }
}
