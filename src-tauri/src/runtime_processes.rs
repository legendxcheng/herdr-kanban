//! Identity-pinned, read-only process evidence from the execution computer.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Process {
    pub pid: u32,
    pub birth: u64,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Runtime {
    pub target: crate::workflow::Target,
    pub processes: Vec<Process>,
    #[serde(default)]
    pub receipt_ids: Vec<String>,
}

#[cfg(windows)]
pub fn capture(target: &crate::workflow::Target) -> Result<Runtime, String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_NO_MORE_FILES, GetLastError, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::*,
    };
    let root: u32 = target
        .process_id
        .split(':')
        .next()
        .ok_or("binding.identity_unknown")?
        .parse()
        .map_err(|_| "binding.identity_unknown")?;
    let root_birth = crate::codex_launcher::birth(root).ok_or("binding.identity_unknown")?;
    let mut tree = Vec::new();
    unsafe {
        let handle = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if handle == INVALID_HANDLE_VALUE {
            return Err("binding.identity_unknown".into());
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(handle, &mut entry);
        while ok != 0 {
            tree.push((entry.th32ProcessID, entry.th32ParentProcessID));
            ok = Process32NextW(handle, &mut entry);
        }
        let error = GetLastError();
        CloseHandle(handle);
        if error != ERROR_NO_MORE_FILES {
            return Err("binding.identity_unknown".into());
        }
    }
    let mut ids = std::collections::HashSet::from([root]);
    loop {
        let before = ids.len();
        for (pid, parent) in &tree {
            if ids.contains(parent) {
                ids.insert(*pid);
            }
        }
        if before == ids.len() {
            break;
        }
    }
    let mut processes = Vec::new();
    for pid in ids {
        // A child that exited during the snapshot is already released. Access
        // denied is not absence and must prevent a false release confirmation.
        if let Some(birth) = crate::codex_launcher::birth(pid) {
            processes.push(Process { pid, birth });
        } else if !exited(&Process { pid, birth: 0 })? {
            return Err("binding.identity_unknown".into());
        }
    }
    if crate::codex_launcher::birth(root) != Some(root_birth) {
        return Err("binding.identity_changed".into());
    }
    Ok(Runtime {
        target: target.clone(),
        processes,
        receipt_ids: vec![uuid::Uuid::new_v4().to_string()],
    })
}

#[cfg(windows)]
fn exited(process: &Process) -> Result<bool, String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, GetLastError, WAIT_OBJECT_0},
        System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            WaitForSingleObject,
        },
    };
    unsafe {
        let h = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            process.pid,
        );
        if h.is_null() {
            return if GetLastError() == ERROR_INVALID_PARAMETER {
                Ok(true)
            } else {
                Err("binding.identity_unknown".into())
            };
        }
        let ended = WaitForSingleObject(h, 0) == WAIT_OBJECT_0;
        CloseHandle(h);
        if ended {
            return Ok(true);
        }
    }
    Ok(crate::codex_launcher::birth(process.pid)
        .is_some_and(|birth| birth != process.birth && process.birth != 0))
}

pub fn absent(runtime: &Runtime) -> Result<bool, String> {
    if runtime.processes.is_empty() {
        return Err("binding.identity_unknown".into());
    }
    for process in &runtime.processes {
        if !exited(process)? {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(not(windows))]
pub fn capture(_: &crate::workflow::Target) -> Result<Runtime, String> {
    Err("terminal.unsupported".into())
}
#[cfg(not(windows))]
fn exited(_: &Process) -> Result<bool, String> {
    Err("terminal.unsupported".into())
}

fn receipt_directory() -> std::path::PathBuf {
    crate::db::db_path().with_file_name("runtime-receipts")
}
fn receipt_path(root: &std::path::Path, id: &str) -> Result<std::path::PathBuf, String> {
    uuid::Uuid::parse_str(id).map_err(|_| "binding.identity_unknown")?;
    Ok(root.join(format!("{id}.json")))
}
fn merge(into: &mut Runtime, other: &Runtime) -> Result<(), String> {
    if !into.target.same_identity(&other.target) {
        return Err("binding.identity_changed".into());
    }
    for p in &other.processes {
        if !into
            .processes
            .iter()
            .any(|known| known.pid == p.pid && known.birth == p.birth)
        {
            into.processes.push(p.clone());
        }
    }
    Ok(())
}
/// The execution computer retains the last process snapshot before any close.
/// A lost SSH reply can still be reconciled without replaying that close.
pub fn evidence(runtime: &Runtime) -> Result<Runtime, String> {
    evidence_at(&receipt_directory(), runtime)
}
fn evidence_at(root: &std::path::Path, runtime: &Runtime) -> Result<Runtime, String> {
    let mut result = runtime.clone();
    for id in &runtime.receipt_ids {
        let path = receipt_path(root, id)?;
        match std::fs::read(path) {
            Ok(bytes) => {
                let stored: Runtime = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                merge(&mut result, &stored)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(result)
}
pub fn save_close_evidence(original: &Runtime, current: &Runtime) -> Result<(), String> {
    let root = receipt_directory();
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    save_close_evidence_at(&root, original, current)
}
fn save_close_evidence_at(
    root: &std::path::Path,
    original: &Runtime,
    current: &Runtime,
) -> Result<(), String> {
    let mut all = evidence_at(root, original)?;
    merge(&mut all, current)?;
    let id = original
        .receipt_ids
        .first()
        .ok_or("binding.identity_unknown")?;
    let path = receipt_path(root, id)?;
    let temp = root.join(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        file.write_all(&serde_json::to_vec(&all).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        std::fs::rename(&temp, &path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn close_time_children_are_durable_even_when_the_reply_is_lost() {
        let dir = std::env::temp_dir().join(format!("kanban-runtime-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let target = crate::workflow::Target {
            machine_id: "local".into(),
            session: "test".into(),
            workspace_id: "w".into(),
            pane_id: "p".into(),
            terminal_id: "t".into(),
            agent: "codex".into(),
            process_id: "1:2".into(),
            codex_resume_locks: vec![],
            title: String::new(),
            status: "working".into(),
        };
        let original = Runtime {
            target,
            processes: vec![Process { pid: 1, birth: 10 }],
            receipt_ids: vec![uuid::Uuid::new_v4().to_string()],
        };
        let mut closing = original.clone();
        closing.processes.push(Process { pid: 2, birth: 20 });
        save_close_evidence_at(&dir, &original, &closing).unwrap();
        let recovered = evidence_at(&dir, &original).unwrap();
        assert_eq!(recovered.processes.len(), 2);
        assert!(
            recovered
                .processes
                .iter()
                .any(|p| p.pid == 2 && p.birth == 20)
        );
        std::fs::remove_file(receipt_path(&dir, &original.receipt_ids[0]).unwrap()).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
}
