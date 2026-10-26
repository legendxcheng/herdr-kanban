//! Bounded CLI process execution and response handling.
use super::Cli;
use crate::workflow::SendError;
use serde_json::Value;
use std::{path::PathBuf, process::Command, time::Duration};

impl Cli {
    pub(crate) fn client_endpoint(&self) -> Result<std::path::PathBuf, String> {
        let bytes = self.output(&["status", "server"]).map_err(|e| e.message)?;
        let status = std::str::from_utf8(&bytes).map_err(|_| "herdr.invalid_response")?;
        let api = status
            .lines()
            .find_map(|line| line.strip_prefix("socket: "))
            .filter(|s| !s.trim().is_empty())
            .ok_or("herdr.invalid_response")?;
        let path = std::path::Path::new(api.trim());
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or("herdr.invalid_response")?;
        Ok(path.with_file_name(format!("{stem}-client.sock")))
    }
    pub fn from_env() -> Self {
        let installed = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("Programs/Herdr/bin/herdr.exe"));
        Self {
            remote: None,
            executable: std::env::var_os("KANBAN_HERDR_EXECUTABLE")
                .map(PathBuf::from)
                .or_else(|| installed.filter(|p| p.is_file()))
                .unwrap_or_else(|| "herdr".into()),
            session: std::env::var("KANBAN_HERDR_SESSION").unwrap_or_else(|_| "default".into()),
        }
    }
    pub fn command(&self) -> Command {
        let mut cmd = Command::new(&self.executable);
        if let Some(machine) = &self.remote {
            cmd.args(["--machine", machine.profile_id.as_deref().unwrap_or("")]);
        } else {
            cmd.args(["--session", &self.session]);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        cmd
    }
    pub(crate) fn output(&self, args: &[&str]) -> Result<Vec<u8>, SendError> {
        if let Some(machine) = &self.remote {
            // Execute against the frozen host/session, after its helper verifies identity.
            // An official profile may be edited while this operation is in flight.
            let response: crate::remote_helper::CliOutput = crate::remote_transport::request(
                machine,
                serde_json::json!({"method":"cli_output","args":args}),
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
                None => Ok(response.stdout),
            };
        }
        let mut command = self.command();
        command.args(args);
        let timeout = if args.starts_with(&["agent", "start"]) {
            40
        } else {
            12
        };
        crate::title_process::run_with_limit(
            command,
            Vec::new(),
            &std::sync::atomic::AtomicBool::new(false),
            Duration::from_secs(timeout),
            4 * 1024 * 1024,
            true,
        )
        .map_err(|error| {
            let uncertain = error != "title.unavailable";
            let message = error
                .replacen("title.cli_failed", "herdr.failed", 1)
                .replacen("title.", "herdr.", 1);
            SendError { message, uncertain }
        })
    }
    pub fn request(&self, args: &[&str]) -> Result<Value, SendError> {
        let stdout = self.output(args)?;
        // Unlike agent commands, pane send-text intentionally prints nothing
        // after its server acknowledgement. Only this command accepts silence.
        if (args.starts_with(&["pane", "send-text"]) || args.starts_with(&["pane", "close"]))
            && stdout.iter().all(u8::is_ascii_whitespace)
        {
            return Ok(Value::Null);
        }
        let doc: Value = serde_json::from_slice(&stdout).map_err(|_| SendError {
            message: "herdr.invalid_response".into(),
            uncertain: true,
        })?;
        if doc.get("ok") == Some(&Value::Bool(false))
            || doc.get("error").is_some_and(|e| !e.is_null())
        {
            return Err(SendError {
                message: "herdr.failed".into(),
                uncertain: true,
            });
        }
        Ok(doc)
    }
    pub(super) fn json(&self, args: &[&str]) -> Result<Value, String> {
        self.request(args).map_err(|e| e.message)
    }
}
