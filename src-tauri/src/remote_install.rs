//! Install a content-addressed helper; never replace a running executable or Herdr server.
use crate::{machines::Machine, remote_transport as transport};
use base64::Engine;
use sha2::{Digest, Sha256};

/// Start, search and viewing must all upgrade before using new helper methods.
/// No database lock is held during upload; recheck identity before publishing.
pub(crate) fn ensure_current(
    db: &std::sync::Arc<std::sync::Mutex<rusqlite::Connection>>,
    expected: &Machine,
) -> Result<Machine, String> {
    ensure_current_with_budget(
        db,
        expected,
        &transport::Budget {
            stop: &std::sync::atomic::AtomicBool::new(false),
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(300),
        },
    )
}

pub(crate) fn ensure_current_with_budget(
    db: &std::sync::Arc<std::sync::Mutex<rusqlite::Connection>>,
    expected: &Machine,
    budget: &transport::Budget<'_>,
) -> Result<Machine, String> {
    static SETUP: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let read = || {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let current = crate::machines::get(&conn, &expected.id)?;
        if current.disabled {
            return Err("machine.disabled".to_string());
        }
        if identity(&current) != identity(expected) {
            return Err("machine.identity_changed".into());
        }
        Ok(current)
    };
    budget.check()?;
    let hash = helper_hash()?;
    let current = read()?;
    if current.helper_name.as_deref() == Some(hash) {
        return Ok(current);
    }
    let _setup = loop {
        budget.check()?;
        match SETUP.try_lock() {
            Ok(lock) => break lock,
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(std::time::Duration::from_millis(25))
            }
            Err(_) => return Err("machine.setup_failed".into()),
        }
    };
    let current = read()?;
    if current.helper_name.as_deref() == Some(hash) {
        return Ok(current);
    }
    let installed = install_with_budget(
        &current,
        &std::env::current_exe().map_err(|e| e.to_string())?,
        budget,
    )?;
    let conn = db.lock().map_err(|e| e.to_string())?;
    let latest = crate::machines::get(&conn, &current.id)?;
    if latest.disabled || identity(&latest) != identity(&current) {
        return Err("machine.identity_changed".into());
    }
    conn.execute(
        "UPDATE machines SET helper_name=?2 WHERE id=?1",
        rusqlite::params![current.id, installed.helper_name],
    )
    .map_err(|e| e.to_string())?;
    Ok(installed)
}

fn identity(machine: &Machine) -> serde_json::Value {
    serde_json::json!([
        machine.id,
        machine.profile_id,
        machine.ssh_target,
        machine.session,
        machine.host_identity
    ])
}

fn helper_hash() -> Result<&'static str, String> {
    static HASH: std::sync::OnceLock<Result<String, String>> = std::sync::OnceLock::new();
    HASH.get_or_init(|| {
        let mut file = std::fs::File::open(std::env::current_exe().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        std::io::copy(&mut file, &mut hash).map_err(|e| e.to_string())?;
        Ok(format!("{:x}", hash.finalize())[..32].into())
    })
    .as_deref()
    .map_err(Clone::clone)
}

// Only small installation metadata uses the encoded command. PowerShell 5's
// Console.In can block under Windows SSH even when native helper stdin works.
fn metadata_command(
    target: &str,
    script: &str,
    request: serde_json::Value,
) -> Result<std::process::Command, String> {
    let encoded = base64::engine::general_purpose::STANDARD
        .encode(serde_json::to_vec(&request).map_err(|_| "machine.invalid_request")?);
    transport::powershell(
        target,
        &format!(
            "$request=[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{encoded}')) | ConvertFrom-Json\n{script}"
        ),
    )
}

const BOOTSTRAP: &str = r#"
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
if (-not [Environment]::Is64BitOperatingSystem -or $env:OS -ne 'Windows_NT') { throw 'machine.platform_unsupported' }
$root=Join-Path $env:USERPROFILE '.herdr-kanban'
$identityFile=Join-Path $root 'identity'
if ($request.expected -and (-not (Test-Path -LiteralPath $identityFile) -or [IO.File]::ReadAllText($identityFile).Trim() -ne $request.expected)) { throw 'machine.identity_changed' }
New-Item -ItemType Directory -Force (Join-Path $root 'helpers') | Out-Null
try {
 $stream=[IO.File]::Open($identityFile,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::Read)
 try { $bytes=[Text.Encoding]::UTF8.GetBytes([Guid]::NewGuid().ToString()); $stream.Write($bytes,0,$bytes.Length) } finally { $stream.Dispose() }
} catch [IO.IOException] { if (-not (Test-Path -LiteralPath $identityFile)) { throw } }
$identity=[IO.File]::ReadAllText($identityFile).Trim()
if ($identity -notmatch '^[a-fA-F0-9-]{36}$') { throw 'machine.identity_changed' }
@{identity=$identity; platform='windows'} | ConvertTo-Json -Compress
"#;

const VERIFY: &str = r#"
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$root=Join-Path $env:USERPROFILE '.herdr-kanban'
if ([IO.File]::ReadAllText((Join-Path $root 'identity')).Trim() -ne $request.identity) { throw 'machine.identity_changed' }
if ($request.hash -notmatch '^[a-f0-9]{64}$' -or $request.upload -notmatch '^[a-f0-9]{32}$') { throw 'machine.invalid_request' }
$temp=Join-Path $root ('helpers/'+$request.upload+'.upload')
$dest=Join-Path $root ('helpers/'+$request.hash.Substring(0,32)+'.exe')
if ((Get-FileHash -LiteralPath $temp -Algorithm SHA256).Hash.ToLowerInvariant() -ne $request.hash) { throw 'machine.hash_mismatch' }
if (Test-Path -LiteralPath $dest) {
 if ((Get-FileHash -LiteralPath $dest -Algorithm SHA256).Hash.ToLowerInvariant() -ne $request.hash) { throw 'machine.hash_mismatch' }
 Remove-Item -LiteralPath $temp
} else { Move-Item -LiteralPath $temp -Destination $dest }
Write-Output 'verified'
"#;

pub fn install(machine: &Machine, executable: &std::path::Path) -> Result<Machine, String> {
    install_with_budget(
        machine,
        executable,
        &transport::Budget {
            stop: &std::sync::atomic::AtomicBool::new(false),
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(300),
        },
    )
}

fn install_with_budget(
    machine: &Machine,
    executable: &std::path::Path,
    budget: &transport::Budget<'_>,
) -> Result<Machine, String> {
    budget.verify_profile(machine)?;
    let target = machine
        .ssh_target
        .as_deref()
        .ok_or("machine.invalid_target")?;
    let bootstrap = budget.run(
        metadata_command(
            target,
            BOOTSTRAP,
            serde_json::json!({"expected":machine.host_identity}),
        )?,
        Vec::new(),
        30,
    )?;
    let response: serde_json::Value =
        serde_json::from_slice(&bootstrap).map_err(|_| "machine.invalid_response")?;
    let identity = response["identity"]
        .as_str()
        .ok_or("machine.invalid_response")?;
    let mut source = std::fs::File::open(executable).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    std::io::copy(&mut source, &mut hash).map_err(|e| e.to_string())?;
    let hash = format!("{:x}", hash.finalize());
    let upload = uuid::Uuid::new_v4().simple().to_string();
    let mut copy = std::process::Command::new("scp");
    copy.args(["-q", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "--"])
        .arg(executable)
        .arg(format!("{target}:.herdr-kanban/helpers/{upload}.upload"));
    budget.run(copy, Vec::new(), 180)?;
    budget.run(
        metadata_command(
            target,
            VERIFY,
            serde_json::json!({"identity":identity,"hash":hash,"upload":upload}),
        )?,
        Vec::new(),
        45,
    )?;
    let mut installed = machine.clone();
    installed.helper_name = Some(hash[..32].into());
    installed.host_identity = Some(identity.into());
    installed.platform = Some("windows".into());
    let hello: serde_json::Value = transport::request_with_budget(
        &installed,
        serde_json::json!({"method":"preflight"}),
        budget,
    )?;
    if hello["platform"] != "windows" || hello["architecture"] != "x86_64" {
        return Err("machine.platform_unsupported".into());
    }
    Ok(installed)
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(windows)]
    fn installation_metadata_arrives_without_reading_ssh_stdin_or_evaluating_data() {
        let expected = serde_json::json!({"expected":"quotes ' 中文 $(throw 'must remain data')"});
        let command = super::metadata_command("fixture", "[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); $request | ConvertTo-Json -Compress", expected.clone()).unwrap();
        let mut local = std::process::Command::new("powershell.exe");
        local
            .args(["-NoProfile", "-NonInteractive", "-EncodedCommand"])
            .arg(command.get_args().last().unwrap());
        let output = crate::title_process::run(
            local,
            vec![],
            &std::sync::atomic::AtomicBool::new(false),
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output).unwrap(),
            expected
        );
    }
    #[test]
    #[ignore = "requires explicit Windows SSH profile and built helper artifact"]
    fn install_and_query_real_windows_remote() {
        let profile_id = std::env::var("KANBAN_ACCEPTANCE_MACHINE").expect("explicit profile");
        let executable = std::env::var("KANBAN_ACCEPTANCE_HELPER").expect("built helper");
        let profile = crate::remote_transport::saved_profiles()
            .unwrap()
            .into_iter()
            .find(|p| p.id == profile_id)
            .unwrap();
        let db = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::init(&db).unwrap();
        let machine = crate::machines::import_profile(&db, &profile, "windows").unwrap();
        let installed = super::install(&machine, std::path::Path::new(&executable)).unwrap();
        let spaces: serde_json::Value = crate::remote_transport::request(
            &installed,
            serde_json::json!({"method":"workspaces"}),
        )
        .unwrap();
        assert!(spaces.is_array());
        let mut frozen = installed.clone();
        frozen.profile_id = Some("changed-after-adapter-was-created".into());
        let mut cli = crate::herdr::Cli::from_env();
        cli.session = frozen.session.clone();
        cli.remote = Some(frozen);
        let output = cli
            .output(&["workspace", "list"])
            .map_err(|e| e.message)
            .unwrap();
        assert!(serde_json::from_slice::<serde_json::Value>(&output).is_ok());
        if let Ok(data) = std::env::var("KANBAN_DATA_DIR") {
            if data.contains("ssh-multi-machine") && data.contains("acceptance") {
                std::fs::write(
                    std::path::Path::new(&data).join("machine.json"),
                    serde_json::to_vec_pretty(&installed).unwrap(),
                )
                .unwrap();
            }
        }
        println!(
            "remote-helper verified: {} workspaces; identity {}",
            spaces.as_array().unwrap().len(),
            installed.host_identity.as_deref().unwrap()
        );
    }
}
