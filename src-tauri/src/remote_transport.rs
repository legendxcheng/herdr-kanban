//! SSH carries a versioned JSON request on stdin; paths and prompts are never shell text.
use base64::Engine;
use serde::{Serialize, de::DeserializeOwned};
use std::{
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

/// One deadline covers setup and the request; cancellation never resends work.
pub(crate) struct Budget<'a> {
    pub stop: &'a AtomicBool,
    pub deadline: Instant,
}
impl Budget<'_> {
    pub fn check(&self) -> Result<(), String> {
        if self.stop.load(Ordering::Acquire) {
            return Err("machine.cancelled".into());
        }
        if Instant::now() >= self.deadline {
            return Err("machine.timeout".into());
        }
        Ok(())
    }
    pub fn run(&self, command: Command, input: Vec<u8>, seconds: u64) -> Result<Vec<u8>, String> {
        self.check()?;
        crate::title_process::run_with_limit(
            command,
            input,
            self.stop,
            self.deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_secs(seconds)),
            20 * 1024 * 1024,
            false,
        )
        .map_err(|e| e.replacen("title.", "machine.", 1))
    }
    pub fn verify_profile(&self, machine: &crate::machines::Machine) -> Result<(), String> {
        if machine.disabled {
            return Err("machine.disabled".into());
        }
        let mut command = Command::new(crate::herdr::Cli::from_env().executable);
        command.args(["machine", "list", "--json"]);
        let profiles = serde_json::from_slice(&self.run(command, Vec::new(), 45)?)
            .map_err(|_| "machine.invalid_response")?;
        match_profile(machine, profiles)
    }
}

pub fn run(command: Command, input: Vec<u8>, seconds: u64) -> Result<Vec<u8>, String> {
    crate::title_process::run_with_limit(
        command,
        input,
        &AtomicBool::new(false),
        Duration::from_secs(seconds),
        20 * 1024 * 1024,
        true,
    )
    .map_err(|e| e.replacen("title.", "machine.", 1))
}

pub fn alias(target: &str) -> Result<(), String> {
    if target.is_empty()
        || target.starts_with('-')
        || !target
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-@:".contains(&b))
    {
        return Err("machine.invalid_target".into());
    }
    Ok(())
}

pub fn ssh(target: &str) -> Result<Command, String> {
    alias(target)?;
    let mut command = Command::new("ssh");
    command.args([
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=10",
        "-o",
        "ServerAliveInterval=5",
        "-o",
        "ServerAliveCountMax=2",
        "--",
        target,
    ]);
    Ok(command)
}

pub fn powershell(target: &str, script: &str) -> Result<Command, String> {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let mut command = ssh(target)?;
    command.args([
        "powershell",
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-EncodedCommand",
        &encoded,
    ]);
    Ok(command)
}

pub fn herdr(args: &[&str]) -> Result<Vec<u8>, String> {
    let mut command = Command::new(crate::herdr::Cli::from_env().executable);
    command.args(args);
    run(command, Vec::new(), 45)
}

pub fn saved_profiles() -> Result<Vec<crate::machines::SavedMachine>, String> {
    serde_json::from_slice(&herdr(&["machine", "list", "--json"])?)
        .map_err(|_| "machine.invalid_response".into())
}

pub fn verify_profile(machine: &crate::machines::Machine) -> Result<(), String> {
    if machine.disabled {
        return Err("machine.disabled".into());
    }
    match_profile(machine, saved_profiles()?)
}

fn match_profile(
    machine: &crate::machines::Machine,
    profiles: Vec<crate::machines::SavedMachine>,
) -> Result<(), String> {
    if machine.disabled {
        return Err("machine.disabled".into());
    }
    let profile = profiles
        .into_iter()
        .find(|p| Some(&p.id) == machine.profile_id.as_ref())
        .ok_or("machine.profile_missing")?;
    if !profile.enabled
        || Some(&profile.target) != machine.ssh_target.as_ref()
        || profile.session != machine.session
    {
        return Err("machine.identity_changed".into());
    }
    Ok(())
}

/// Dedicated one-shot transport, independent of task-control and search queues.
pub(crate) fn request_with_budget<T: DeserializeOwned>(
    machine: &crate::machines::Machine,
    request: impl Serialize,
    budget: &Budget<'_>,
) -> Result<T, String> {
    let input = serde_json::to_vec(&serde_json::json!({"version":1,"host_identity":machine.host_identity,"session":machine.session,"request":request})).map_err(|_| "machine.invalid_request")?;
    let bytes = budget.run(helper_command(machine, false)?, input, 55)?;
    let response: Envelope =
        serde_json::from_slice(&bytes).map_err(|_| "machine.invalid_response")?;
    if response.version != 1 || Some(&response.host_identity) != machine.host_identity.as_ref() {
        return Err("machine.identity_changed".into());
    }
    if let Some(error) = response.error {
        return Err(error);
    }
    serde_json::from_value(response.result).map_err(|_| "machine.invalid_response".into())
}

#[derive(serde::Deserialize)]
struct Envelope {
    version: u32,
    host_identity: String,
    result: serde_json::Value,
    error: Option<String>,
}

/// Direct inherited handles work for both console/debug and GUI/release helpers.
/// PowerShell does not wait for a GUI executable invoked without a pipeline,
/// so it can exit successfully before that helper writes its JSON response.
pub fn helper_command(
    machine: &crate::machines::Machine,
    streaming: bool,
) -> Result<Command, String> {
    helper_command_mode(machine, if streaming { " --search-stream-v1" } else { "" })
}

pub(crate) fn control_command(machine: &crate::machines::Machine) -> Result<Command, String> {
    helper_command_mode(machine, " --control-stream-v1")
}

fn helper_command_mode(machine: &crate::machines::Machine, mode: &str) -> Result<Command, String> {
    let helper = machine
        .helper_name
        .as_deref()
        .ok_or("machine.setup_required")?;
    if helper.len() != 32 || !helper.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("machine.invalid_response".into());
    }
    let mut command = ssh(machine
        .ssh_target
        .as_deref()
        .ok_or("machine.invalid_target")?)?;
    command.args([
        "cmd",
        "/d",
        "/s",
        "/c",
        &format!(
            "\"\"%USERPROFILE%\\.herdr-kanban\\helpers\\{helper}.exe\" --remote-helper{mode}\""
        ),
    ]);
    Ok(command)
}

pub fn request<T: DeserializeOwned>(
    machine: &crate::machines::Machine,
    request: impl Serialize,
) -> Result<T, String> {
    let request = serde_json::to_value(request).map_err(|e| e.to_string())?;
    if let Some(result) = crate::remote_control::request(machine, request.clone()) {
        return serde_json::from_value(result?).map_err(|_| "machine.invalid_response".into());
    }
    let input = serde_json::to_vec(&serde_json::json!({"version":1,"host_identity":machine.host_identity,"session":machine.session,"request":request})).map_err(|e|e.to_string())?;
    let bytes = run(helper_command(machine, false)?, input, 55)?;
    let response: Envelope =
        serde_json::from_slice(&bytes).map_err(|_| "machine.invalid_response")?;
    if response.version != 1 || Some(&response.host_identity) != machine.host_identity.as_ref() {
        return Err("machine.identity_changed".into());
    }
    if let Some(error) = response.error {
        return Err(error);
    }
    serde_json::from_value(response.result).map_err(|_| "machine.invalid_response".into())
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn title_transport_budget_cancels_processes_and_bounds_the_total_deadline() {
        use super::*;
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let worker = std::thread::spawn(move || {
            let budget = Budget {
                stop: &signal,
                deadline: Instant::now() + Duration::from_secs(30),
            };
            let mut command = Command::new("powershell.exe");
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep 30",
            ]);
            budget.run(command, vec![], 55)
        });
        std::thread::sleep(Duration::from_millis(200));
        let start = Instant::now();
        stop.store(true, Ordering::Release);
        assert_eq!(worker.join().unwrap().unwrap_err(), "machine.cancelled");
        assert!(start.elapsed() < Duration::from_secs(3));
        let stop = AtomicBool::new(false);
        let budget = Budget {
            stop: &stop,
            deadline: Instant::now() + Duration::from_millis(100),
        };
        let mut command = Command::new("powershell.exe");
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Sleep 30",
        ]);
        assert_eq!(
            budget.run(command, vec![], 55).unwrap_err(),
            "machine.timeout"
        );
    }
    #[test]
    #[ignore = "requires an explicitly selected SSH machine and its deployed release GUI helper"]
    fn release_gui_helper_returns_preflight_envelope() {
        use sha2::{Digest, Sha256};
        let machine_file =
            std::env::var("KANBAN_ACCEPTANCE_MACHINE_FILE").expect("explicit machine fixture");
        let executable =
            std::env::var("KANBAN_ACCEPTANCE_HELPER").expect("deployed release executable");
        let bytes = std::fs::read(executable).unwrap();
        // A debug console executable hides this failure. Require a PE GUI image.
        let pe = u32::from_le_bytes(bytes[0x3c..0x40].try_into().unwrap()) as usize;
        assert_eq!(&bytes[pe..pe + 4], b"PE\0\0");
        let subsystem = u16::from_le_bytes(bytes[pe + 24 + 68..pe + 24 + 70].try_into().unwrap());
        assert_eq!(subsystem, 2, "regression requires Windows GUI subsystem");
        let mut machine: crate::machines::Machine =
            serde_json::from_slice(&std::fs::read(machine_file).unwrap()).unwrap();
        machine.helper_name = Some(format!("{:x}", Sha256::digest(&bytes))[..32].into());
        let response: serde_json::Value =
            super::request(&machine, serde_json::json!({"method":"preflight"})).unwrap();
        assert_eq!(response["platform"], "windows");
        assert_eq!(response["architecture"], "x86_64");
    }
}
