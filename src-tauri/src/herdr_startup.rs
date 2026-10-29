//! Local startup is explicit, shared, bounded, and independent of task dispatch.
use crate::herdr::Cli;
use serde_json::Value;
use std::{
    fs::File,
    io::Read,
    path::PathBuf,
    process::{Child, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Default)]
struct Attempt {
    result: Option<Result<(), String>>,
    child: Option<Child>,
    handoff_pending: bool,
}

pub struct LocalHerdrStartup {
    cli: Cli,
    log: PathBuf,
    attempt: Mutex<Attempt>,
    generation: AtomicU64,
}

impl LocalHerdrStartup {
    pub fn new(cli: Cli, log: PathBuf) -> Self {
        Self {
            cli,
            log,
            attempt: Mutex::new(Attempt::default()),
            generation: AtomicU64::new(0),
        }
    }

    pub fn ensure(&self, retry: bool) -> Result<(), String> {
        let observed = self.generation.load(Ordering::Acquire);
        let mut attempt = self.attempt.lock().map_err(|_| "herdr.startup_failed")?;
        if let Some(result) = &attempt.result {
            if !retry || observed != self.generation.load(Ordering::Acquire) {
                return result.clone();
            }
        }
        let result = self.run(&mut attempt, Duration::from_secs(20));
        attempt.result = Some(result.clone());
        self.generation.fetch_add(1, Ordering::Release);
        result
    }

    fn probe(&self, timeout: Duration) -> Result<bool, String> {
        if self.cli.executable.is_absolute() && !self.cli.executable.is_file() {
            return Err("herdr.unavailable".into());
        }
        let mut command = self.cli.command();
        command.args(["status", "--json"]);
        let bytes = crate::title_process::run_with_limit(
            command,
            Vec::new(),
            &AtomicBool::new(false),
            timeout,
            64 * 1024,
            true,
        )
        .map_err(|e| {
            e.replacen("title.cli_failed", "herdr.failed", 1)
                .replacen("title.", "herdr.", 1)
        })?;
        status_running(&bytes, &self.cli.session)
    }

    fn run(&self, attempt: &mut Attempt, timeout: Duration) -> Result<(), String> {
        if self.cli.remote.is_some() {
            return Err("herdr.invalid_response".into());
        }
        let deadline = Instant::now() + timeout;
        let mut launched = attempt.handoff_pending;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("herdr.startup_timeout".into());
            }
            match self.probe(remaining.min(Duration::from_secs(2))) {
                Ok(true) => {
                    attempt.handoff_pending = false;
                    if let Some(mut child) = attempt.child.take() {
                        // Reap the launcher on exit; never kill the persistent server.
                        std::thread::spawn(move || {
                            let _ = child.wait();
                        });
                    }
                    return Ok(());
                }
                Ok(false) => {}
                Err(error) if error == "herdr.timeout" && (launched || attempt.child.is_some()) => {
                    continue;
                }
                Err(error) => return Err(error),
            }
            if let Some(child) = attempt.child.as_mut() {
                if let Some(exit) = child
                    .try_wait()
                    .map_err(|e| format!("herdr.startup_failed: {e}"))?
                {
                    attempt.child = None;
                    if !exit.success() {
                        return Err(format!("herdr.startup_failed: {}", self.diagnostic()));
                    }
                    // Some launchers detach their server and exit before it is ready.
                    // Preserve the pending handoff across timeout/retry as well.
                    attempt.handoff_pending = true;
                    launched = true;
                }
            }
            if attempt.child.is_none() && !launched {
                // Only an explicit not_running status permits a launch. Do not use
                // title_process here: its job kills descendants when the CLI exits.
                let log =
                    File::create(&self.log).map_err(|e| format!("herdr.startup_failed: {e}"))?;
                let mut command = self.cli.command();
                command
                    .arg("server")
                    .stdin(Stdio::null())
                    .stdout(
                        log.try_clone()
                            .map_err(|e| format!("herdr.startup_failed: {e}"))?,
                    )
                    .stderr(log);
                attempt.child = Some(
                    command
                        .spawn()
                        .map_err(|e| format!("herdr.startup_failed: {e}"))?,
                );
                launched = true;
            }
            std::thread::sleep(
                Duration::from_millis(100).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }

    fn diagnostic(&self) -> String {
        let mut bytes = Vec::new();
        if let Ok(file) = File::open(&self.log) {
            let _ = file.take(4096).read_to_end(&mut bytes);
        }
        String::from_utf8_lossy(&bytes).chars().take(800).collect()
    }
}

fn status_running(bytes: &[u8], session: &str) -> Result<bool, String> {
    let doc: Value = serde_json::from_slice(bytes).map_err(|_| "herdr.invalid_response")?;
    // Herdr reports the default session as explicit JSON null. Missing fields
    // must still fail, and this alias must never match a named session.
    let matches_session = |side: &Value| match side.get("session") {
        Some(Value::String(name)) => name == session,
        Some(Value::Null) => session == "default",
        _ => false,
    };
    if !matches_session(&doc["client"]) || !matches_session(&doc["server"]) {
        return Err("herdr.invalid_response".into());
    }
    match doc["server"]["running"].as_bool() {
        Some(true)
            if doc["server"]["compatible"] == false
                || doc["server"]["endpoint_compatible"] == false =>
        {
            Err("herdr.incompatible".into())
        }
        Some(true) => Ok(true),
        Some(false) if doc["server"]["status"] == "not_running" => Ok(false),
        _ => Err("herdr.invalid_response".into()),
    }
}

#[tauri::command]
pub async fn ensure_local_herdr(
    startup: tauri::State<'_, std::sync::Arc<LocalHerdrStartup>>,
    retry: bool,
) -> Result<(), String> {
    let startup = startup.inner().clone();
    tauri::async_runtime::spawn_blocking(move || startup.ensure(retry))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
#[path = "herdr_startup_tests.rs"]
mod tests;
