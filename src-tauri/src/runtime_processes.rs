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
