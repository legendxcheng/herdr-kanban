//! Observe only this application's interactive authorization operation, never its input.
use std::{
    collections::HashMap,
    path::PathBuf,
    process::Child,
    sync::{Mutex, OnceLock},
};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct EnrollmentStatus {
    pub id: String,
    pub state: String,
    pub error: String,
}
struct Enrollment {
    child: Child,
    directory: PathBuf,
    status: EnrollmentStatus,
}
fn operations() -> &'static Mutex<HashMap<String, Enrollment>> {
    static OPERATIONS: OnceLock<Mutex<HashMap<String, Enrollment>>> = OnceLock::new();
    OPERATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}
pub fn track(id: String, child: Child, directory: PathBuf) -> Result<EnrollmentStatus, String> {
    let status = EnrollmentStatus {
        id: id.clone(),
        state: "waiting".into(),
        error: String::new(),
    };
    operations().lock().map_err(|e| e.to_string())?.insert(
        id,
        Enrollment {
            child,
            directory,
            status: status.clone(),
        },
    );
    let operation = status.id.clone();
    std::thread::spawn(move || {
        while read(&operation).is_ok_and(|s| s.state == "waiting") {
            std::thread::sleep(std::time::Duration::from_millis(750));
        }
    });
    Ok(status)
}
pub fn read(id: &str) -> Result<EnrollmentStatus, String> {
    let mut operations = operations().lock().map_err(|e| e.to_string())?;
    let op = operations.get_mut(id).ok_or("machine.enrollment_missing")?;
    if op.status.state != "waiting" {
        return Ok(op.status.clone());
    }
    let ended = op.child.try_wait().map_err(|e| e.to_string())?.is_some();
    let result = op.directory.join("result.json");
    if ended && result.metadata().is_ok_and(|m| m.len() < 8192) {
        if let Ok(value) = std::fs::read(&result).and_then(|bytes| {
            serde_json::from_slice::<EnrollmentStatus>(&bytes).map_err(std::io::Error::other)
        }) {
            if value.id == id && matches!(value.state.as_str(), "succeeded" | "failed") {
                op.status = value;
            }
        }
    }
    if ended && op.status.state == "waiting" {
        op.status.state = "cancelled".into();
        op.status.error = "machine.enrollment_cancelled".into();
    }
    // Remove only files created for this operation; no user SSH configuration is removed.
    if ended {
        for name in ["settings.json", "enroll.ps1", "result.json"] {
            let _ = std::fs::remove_file(op.directory.join(name));
        }
        let _ = std::fs::remove_dir(&op.directory);
    }
    Ok(op.status.clone())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::os::windows::process::CommandExt;

    fn finished(result: Option<(&str, &str)>) -> EnrollmentStatus {
        let id = uuid::Uuid::new_v4().to_string();
        let directory = std::env::temp_dir().join(format!("kanban-enrollment-test-{id}"));
        std::fs::create_dir(&directory).unwrap();
        if let Some((identity, state)) = result {
            let value = EnrollmentStatus {
                id: if identity == "same" {
                    id.clone()
                } else {
                    identity.into()
                },
                state: state.into(),
                error: if state == "failed" {
                    "machine.enrollment_failed".into()
                } else {
                    String::new()
                },
            };
            std::fs::write(
                directory.join("result.json"),
                serde_json::to_vec(&value).unwrap(),
            )
            .unwrap();
        }
        let child = std::process::Command::new("cmd")
            .args(["/C", "exit 0"])
            .creation_flags(0x08000000)
            .spawn()
            .unwrap();
        assert_eq!(
            track(id.clone(), child, directory.clone()).unwrap().state,
            "waiting"
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let status = read(&id).unwrap();
            if status.state != "waiting" {
                assert!(!directory.exists());
                return status;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    #[test]
    fn exited_terminal_requires_an_exact_operation_result() {
        assert_eq!(finished(None).state, "cancelled");
        assert_eq!(
            finished(Some(("other-operation", "succeeded"))).state,
            "cancelled"
        );
        assert_eq!(finished(Some(("same", "succeeded"))).state, "succeeded");
        assert_eq!(finished(Some(("same", "failed"))).state, "failed");
    }
}
