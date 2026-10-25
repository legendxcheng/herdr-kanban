//! Herdr topology for automatic task allocation. Existing task bindings remain valid in any tab.
use super::{Cli, field};
use serde_json::Value;

impl Cli {
    pub(super) fn kanban_tab(&self, workspace: &str) -> Result<Option<String>, String> {
        let doc = self.json(&["tab", "list", "--workspace", workspace])?;
        let tabs = doc
            .pointer("/result/tabs")
            .and_then(Value::as_array)
            .ok_or("herdr.invalid_response")?;
        let mut found = None;
        for tab in tabs {
            if field(tab, "workspace_id")? != workspace {
                return Err("binding.identity_changed".into());
            }
            if tab.get("label").and_then(Value::as_str) == Some("kanban") {
                if found.is_some() {
                    return Err("flow.kanban_tab_ambiguous".into());
                }
                found = Some(field(tab, "tab_id")?.to_string());
            }
        }
        Ok(found)
    }

    pub(super) fn workspace_panes(&self, workspace: &str) -> Result<Vec<Value>, String> {
        let doc = self.json(&["pane", "list", "--workspace", workspace])?;
        let panes = doc
            .pointer("/result/panes")
            .and_then(Value::as_array)
            .ok_or("herdr.invalid_response")?;
        for pane in panes {
            if field(pane, "workspace_id")? != workspace {
                return Err("binding.identity_changed".into());
            }
        }
        Ok(panes.clone())
    }

    pub(super) fn split_anchor(
        &self,
        workspace: &str,
        tab: &str,
    ) -> Result<(String, &'static str), String> {
        let panes = self.workspace_panes(workspace)?;
        let candidates: Vec<_> = panes
            .iter()
            .filter(|p| p.get("tab_id").and_then(Value::as_str) == Some(tab))
            .collect();
        let first = candidates.first().ok_or("binding.missing")?;
        let doc = self.json(&["pane", "layout", "--pane", field(first, "pane_id")?])?;
        let layout = doc
            .pointer("/result/layout")
            .ok_or("herdr.invalid_response")?;
        if field(layout, "workspace_id")? != workspace || field(layout, "tab_id")? != tab {
            return Err("binding.identity_changed".into());
        }
        let visible = layout
            .get("panes")
            .and_then(Value::as_array)
            .ok_or("herdr.invalid_response")?;
        let mut best = None;
        for pane in visible {
            let id = field(pane, "pane_id")?;
            if !candidates
                .iter()
                .any(|p| p.get("pane_id").and_then(Value::as_str) == Some(id))
            {
                continue;
            }
            let w = pane
                .pointer("/rect/width")
                .and_then(Value::as_u64)
                .ok_or("herdr.invalid_response")?;
            let h = pane
                .pointer("/rect/height")
                .and_then(Value::as_u64)
                .ok_or("herdr.invalid_response")?;
            let area = w.saturating_mul(h);
            if best.as_ref().is_none_or(|(size, _, _)| area > *size) {
                best = Some((
                    area,
                    id.to_string(),
                    if w >= h.saturating_mul(2) {
                        "right"
                    } else {
                        "down"
                    },
                ));
            }
        }
        best.map(|(_, id, direction)| (id, direction))
            .ok_or_else(|| "binding.missing".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::Herdr;

    #[test]
    #[ignore = "requires an isolated Herdr workspace; creates and closes test tabs"]
    fn live_shared_tab_reuses_shell_and_rejects_duplicate_labels() {
        assert_eq!(std::env::var("HERDR_ENV").as_deref(), Ok("1"));
        let name = std::env::var("KANBAN_LIVE_WORKSPACE").unwrap();
        assert!(name.starts_with("kanban-acceptance-"));
        let cwd = std::env::var("KANBAN_LIVE_CWD").unwrap();
        let cli = Cli::from_env();
        let spaces = cli.workspaces().unwrap();
        let matches: Vec<_> = spaces.iter().filter(|w| w.name == name).collect();
        assert_eq!(matches.len(), 1);
        let workspace = &matches[0].id;
        assert!(cli.kanban_tab(workspace).unwrap().is_none());
        assert!(cli.available(workspace, &cwd).unwrap().is_empty());
        let first = cli
            .create_pane(workspace, &cwd, "test-a", &|| Ok(()))
            .unwrap();
        let tab = cli.kanban_tab(workspace).unwrap().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        loop {
            let available = cli.available(workspace, &cwd).unwrap();
            if !available.is_empty() {
                assert_eq!(available.len(), 1);
                assert_eq!(available[0].pane_id, first.pane_id);
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let second = cli
            .create_pane(workspace, &cwd, "test-b", &|| Ok(()))
            .unwrap();
        assert_ne!(first.pane_id, second.pane_id);
        assert_eq!(cli.kanban_tab(workspace).unwrap(), Some(tab.clone()));
        let panes = cli.workspace_panes(workspace).unwrap();
        assert_eq!(
            panes
                .iter()
                .filter(|p| p.get("tab_id").and_then(Value::as_str) == Some(&tab))
                .count(),
            2
        );
        let duplicate = cli
            .json(&[
                "tab",
                "create",
                "--workspace",
                workspace,
                "--cwd",
                &cwd,
                "--label",
                "kanban",
                "--no-focus",
            ])
            .unwrap();
        let duplicate_id = duplicate
            .pointer("/result/tab/tab_id")
            .and_then(Value::as_str)
            .unwrap();
        assert_eq!(
            cli.available(workspace, &cwd).unwrap_err(),
            "flow.kanban_tab_ambiguous"
        );
        assert_eq!(
            cli.create_pane(workspace, &cwd, "never", &|| Ok(()))
                .unwrap_err(),
            "flow.kanban_tab_ambiguous"
        );
        cli.json(&["tab", "close", duplicate_id]).unwrap();
        // These are only the two shell panes created above, with no agent or user task.
        cli.json(&["pane", "close", &first.pane_id]).unwrap();
        assert_eq!(cli.kanban_tab(workspace).unwrap(), Some(tab));
        cli.json(&["pane", "close", &second.pane_id]).unwrap();
        assert!(cli.kanban_tab(workspace).unwrap().is_none());
    }
}
