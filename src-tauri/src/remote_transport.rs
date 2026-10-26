//! SSH carries a versioned JSON request on stdin; paths and prompts are never shell text.
use base64::Engine;
use serde::{Serialize, de::DeserializeOwned};
use std::{process::Command, sync::atomic::AtomicBool, time::Duration};

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
    let profile = saved_profiles()?
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

#[derive(serde::Deserialize)]
struct Envelope {
    version: u32,
    host_identity: String,
    result: serde_json::Value,
    error: Option<String>,
}

pub fn request<T: DeserializeOwned>(
    machine: &crate::machines::Machine,
    request: impl Serialize,
) -> Result<T, String> {
    let helper = machine
        .helper_name
        .as_deref()
        .ok_or("machine.setup_required")?;
    if helper.len() != 32 || !helper.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("machine.invalid_response".into());
    }
    let script = format!(
        "$ErrorActionPreference='Stop'; $ProgressPreference='SilentlyContinue'; [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); $OutputEncoding=[Console]::OutputEncoding; & (Join-Path $env:USERPROFILE '.herdr-kanban/helpers/{helper}.exe') --remote-helper; exit $LASTEXITCODE"
    );
    let input = serde_json::to_vec(&serde_json::json!({"version":1,"host_identity":machine.host_identity,"session":machine.session,"request":request})).map_err(|e|e.to_string())?;
    let bytes = run(
        powershell(
            machine
                .ssh_target
                .as_deref()
                .ok_or("machine.invalid_target")?,
            &script,
        )?,
        input,
        55,
    )?;
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
