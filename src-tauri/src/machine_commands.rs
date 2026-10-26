use crate::{
    commands::DbState,
    machines::{self, Machine, SavedMachine},
    remote_transport as transport,
};
use tauri::State;

#[tauri::command]
pub fn enroll_machine(
    target: String,
    session: Option<String>,
) -> Result<crate::machine_enrollment::EnrollmentStatus, String> {
    transport::alias(&target)?;
    let session = session.unwrap_or_else(|| "default".into());
    if session.is_empty()
        || !session
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
    {
        return Err("machine.invalid_input".into());
    }
    use sha2::{Digest, Sha256};
    let key_id = format!("{:x}", Sha256::digest(target.as_bytes()));
    let operation_id = uuid::Uuid::new_v4().to_string();
    let dir = std::env::temp_dir().join(format!("herdr-kanban-enroll-{operation_id}"));
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let script = dir.join("enroll.ps1");
    let settings = dir.join("settings.json");
    std::fs::write(
        &script,
        include_str!("../../scripts/enroll-windows-machine.ps1"),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(&settings,serde_json::to_vec(&serde_json::json!({"target":target,"session":session,"key_id":&key_id[..16],"herdr":crate::herdr::Cli::from_env().executable,"operation_id":operation_id,"result":dir.join("result.json")})).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    let mut command = std::process::Command::new("powershell");
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script)
        .arg("-Settings")
        .arg(settings);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x00000010);
    }
    let child = command
        .spawn()
        .map_err(|e| format!("machine.enrollment_failed: {e}"))?;
    crate::machine_enrollment::track(operation_id, child, dir)
}

#[tauri::command]
pub fn enrollment_status(
    operation_id: String,
) -> Result<crate::machine_enrollment::EnrollmentStatus, String> {
    crate::machine_enrollment::read(&operation_id)
}

#[tauri::command]
pub fn list_machines(db: State<'_, DbState>) -> Result<Vec<Machine>, String> {
    machines::list(&*db.0.lock().map_err(|e| e.to_string())?)
}

#[tauri::command]
pub async fn saved_machines() -> Result<Vec<SavedMachine>, String> {
    tauri::async_runtime::spawn_blocking(transport::saved_profiles)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn connect_machine(
    db: State<'_, DbState>,
    profile_id: String,
) -> Result<Machine, String> {
    let db = db.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Serialize setup only; normal Local and remote task operations remain independent.
        static SETUP: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _setup = SETUP.lock().map_err(|e| e.to_string())?;
        let profile = transport::saved_profiles()?
            .into_iter()
            .find(|p| p.id == profile_id)
            .ok_or("machine.profile_missing")?;
        let machine = {
            let conn = db.lock().map_err(|e| e.to_string())?;
            let machine = machines::import_profile(&conn, &profile, "windows")?;
            // A saved profile is not yet a verified execution environment.
            conn.execute("UPDATE machines SET disabled=1 WHERE id=?1", [&machine.id]).map_err(|e|e.to_string())?;
            machine
        };
        let installed = crate::remote_install::install(
            &machine,
            &std::env::current_exe().map_err(|e| e.to_string())?,
        )?;
        let conn = db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE machines SET helper_name=?2,host_identity=?3,platform=?4,disabled=0 WHERE id=?1",
            rusqlite::params![
                installed.id,
                installed.helper_name,
                installed.host_identity,
                installed.platform
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(installed)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn rename_machine(db: State<'_, DbState>, id: String, name: String) -> Result<(), String> {
    machines::rename(&*db.0.lock().map_err(|e| e.to_string())?, &id, &name)
}

#[tauri::command]
pub fn remove_machine(
    db: State<'_, DbState>,
    native: State<'_, std::sync::Arc<crate::native_terminal::NativeTerminalManager>>,
    id: String,
) -> Result<(), String> {
    let tasks = {
        let db = db.0.lock().map_err(|e| e.to_string())?;
        machines::disable(&db, &id)?;
        let mut query=db.prepare("SELECT t.id FROM tasks t JOIN projects p ON p.id=t.project_id WHERE p.machine_id=?1").map_err(|e|e.to_string())?;
        query
            .query_map([&id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };
    for task in tasks {
        native.close_task(&task);
    }
    Ok(())
}

#[tauri::command]
pub async fn machine_status(
    db: State<'_, DbState>,
    id: String,
) -> Result<serde_json::Value, String> {
    let machine = machines::get(&*db.0.lock().map_err(|e| e.to_string())?, &id)?;
    if machine.kind == "local" {
        return Ok(serde_json::json!({"status":"reachable"}));
    }
    tauri::async_runtime::spawn_blocking(move || {
        transport::verify_profile(&machine)?;
        let _: serde_json::Value =
            transport::request(&machine, serde_json::json!({"method":"hello"}))?;
        Ok(serde_json::json!({"status":"reachable"}))
    })
    .await
    .map_err(|e| e.to_string())?
}
