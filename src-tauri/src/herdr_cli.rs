//! Bounded CLI process execution and response handling.
use super::Cli;
use crate::workflow::SendError;
use serde_json::Value;
use std::{process::Command, time::Duration};

impl Cli {
    pub(crate) fn client_endpoint(&self) -> Result<std::path::PathBuf, String> {
        let path = self.api_endpoint()?;
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or("herdr.invalid_response")?;
        Ok(path.with_file_name(format!("{stem}-client.sock")))
    }
    pub(super) fn api_endpoint(&self) -> Result<std::path::PathBuf, String> {
        let bytes = self.output(&["status", "server"]).map_err(|e| e.message)?;
        let status = std::str::from_utf8(&bytes).map_err(|_| "herdr.invalid_response")?;
        let api = status
            .lines()
            .find_map(|line| line.strip_prefix("socket: "))
            .filter(|s| !s.trim().is_empty())
            .ok_or("herdr.invalid_response")?;
        Ok(std::path::PathBuf::from(api.trim()))
    }
    pub fn from_env() -> Self {
        Self {
            remote: None,
            executable: super::executable::from_env(),
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
        let mut command = self.command_for_args(args)?;
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
    fn command_for_args(&self, args: &[&str]) -> Result<Command, SendError> {
        // Windows refuses CR/LF arguments to .cmd/.bat. Keep the wrapper as the
        // authority for discovery, then send literal API arguments to the native
        // CLI it reports, pinned to the very same socket. Do not forward --session:
        // Herdr would recompute the endpoint using the parent process's config.
        #[cfg(windows)]
        if self
            .executable
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"))
            && args.iter().any(|arg| arg.contains(['\r', '\n']))
            && args
                .first()
                .is_some_and(|arg| matches!(*arg, "workspace" | "pane" | "agent" | "tab"))
        {
            let invalid = || SendError {
                message: "herdr.invalid_response".into(),
                uncertain: false,
            };
            let bytes = self.output(&["status", "--json"]).map_err(|mut error| {
                error.uncertain = false; // Only a read has run; no payload was sent.
                error
            })?;
            let status: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
            let binary = std::path::Path::new(
                status
                    .pointer("/client/binary")
                    .and_then(Value::as_str)
                    .ok_or_else(invalid)?,
            );
            let socket = status
                .pointer("/server/socket")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(invalid)?;
            if !binary.is_absolute()
                || !binary.is_file()
                || !binary
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
            {
                return Err(invalid());
            }
            if status.pointer("/server/running").and_then(Value::as_bool) != Some(true) {
                return Err(SendError {
                    message: "herdr.failed".into(),
                    uncertain: false,
                });
            }
            let mut command = Command::new(binary);
            command
                .env("HERDR_SESSION", &self.session)
                .env("HERDR_SOCKET_PATH", socket);
            return Ok(command);
        }
        Ok(self.command())
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
