//! Bounded CLI process execution and response handling.
use super::Cli;
use crate::workflow::SendError;
use serde_json::Value;
use std::{
    io::Read,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

impl Cli {
    pub fn from_env() -> Self {
        let installed = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("Programs/Herdr/bin/herdr.exe"));
        Self {
            executable: std::env::var_os("KANBAN_HERDR_EXECUTABLE")
                .map(PathBuf::from)
                .or_else(|| installed.filter(|p| p.is_file()))
                .unwrap_or_else(|| "herdr".into()),
            session: std::env::var("KANBAN_HERDR_SESSION").unwrap_or_else(|_| "default".into()),
        }
    }
    pub fn command(&self) -> Command {
        let mut cmd = Command::new(&self.executable);
        cmd.args(["--session", &self.session]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        cmd
    }
    pub(super) fn output(&self, args: &[&str]) -> Result<Vec<u8>, SendError> {
        let mut child = self
            .command()
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SendError {
                message: format!("herdr.unavailable: {e}"),
                uncertain: false,
            })?;
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let read = |stream: Box<dyn Read + Send>| {
            thread::spawn(move || {
                let mut bytes = Vec::new();
                let _ = stream.take(4 * 1024 * 1024).read_to_end(&mut bytes);
                bytes
            })
        };
        let out = read(Box::new(stdout));
        let errors = read(Box::new(stderr));
        let start = Instant::now();
        let timeout = if args.starts_with(&["agent", "start"]) {
            40
        } else {
            12
        };
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) if start.elapsed() < Duration::from_secs(timeout) => {
                    thread::sleep(Duration::from_millis(20))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(());
                }
            }
        };
        let stdout = out.join().unwrap_or_default();
        let stderr = errors.join().unwrap_or_default();
        let status = status.map_err(|_| SendError {
            message: "herdr.timeout".into(),
            uncertain: true,
        })?;
        if !status.success() {
            return Err(SendError {
                message: format!(
                    "herdr.failed: {}",
                    String::from_utf8_lossy(&stderr)
                        .chars()
                        .take(1000)
                        .collect::<String>()
                ),
                uncertain: true,
            });
        }
        Ok(stdout)
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
