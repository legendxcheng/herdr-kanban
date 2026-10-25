//! Conservative evidence for an input-free resume. argv survives an in-process
//! /resume switch, so later lock observations must match the startup evidence.
//! Unknown platform/API results fail closed; normal hook reports don't use this.

#[cfg(not(windows))]
pub(super) fn session_locks(_: u64) -> Option<Vec<String>> {
    None
}

#[cfg(windows)]
pub(super) fn session_locks(pid: u64) -> Option<Vec<String>> {
    let mut ids: Vec<_> = windows::sessions(pid)?.into_iter().collect();
    ids.sort();
    Some(ids)
}

pub(super) fn owns_session(pid: u64, expected: &str, startup: &[String]) -> bool {
    session_locks(pid).is_some_and(|ids| matches_startup_locks(&ids, expected, startup))
}

fn matches_startup_locks(ids: &[String], expected: &str, startup: &[String]) -> bool {
    if startup.is_empty() {
        // Older bindings did not record the auto-review helper's lock.
        return ids.len() == 1 && ids[0] == expected;
    }
    // A newly launched automatic-review process can hold the resumed thread
    // and an auxiliary thread. Never infer that an arbitrary second lock is safe:
    // every ID must still match the evidence captured at our own process startup.
    ids == startup && startup.len() <= 2 && startup.iter().any(|id| id == expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_locks_keep_the_startup_helper_but_reject_a_switched_conversation() {
        let root = "root".to_string();
        let helper = "helper".to_string();
        let startup = vec![helper.clone(), root.clone()];
        assert!(matches_startup_locks(&startup, &root, &startup));
        assert!(!matches_startup_locks(
            &["other".into(), root.clone()],
            &root,
            &startup
        ));
        assert!(!matches_startup_locks(
            &[helper.clone(), "other".into(), root.clone()],
            &root,
            &startup
        ));
        assert!(!matches_startup_locks(&startup, "other", &startup));
        assert!(!matches_startup_locks(&startup, &root, &[]));
        assert!(matches_startup_locks(&[root.clone()], &root, &[]));
    }
}

#[cfg(windows)]
mod windows {
    use std::{
        collections::HashSet, ffi::c_void, mem::size_of, os::windows::io::AsRawHandle,
        path::PathBuf,
    };

    type Handle = *mut c_void;
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Entry {
        handle: Handle,
        handle_count: usize,
        pointer_count: usize,
        access: u32,
        object_type: u32,
        attributes: u32,
        reserved: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn CloseHandle(handle: Handle) -> i32;
        fn GetCurrentProcess() -> Handle;
        fn DuplicateHandle(
            source: Handle,
            handle: Handle,
            target: Handle,
            result: *mut Handle,
            access: u32,
            inherit: i32,
            options: u32,
        ) -> i32;
        fn GetFileType(handle: Handle) -> u32;
        fn GetFinalPathNameByHandleW(handle: Handle, path: *mut u16, size: u32, flags: u32) -> u32;
        fn GetProcessHandleCount(process: Handle, count: *mut u32) -> i32;
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtQueryInformationProcess(
            process: Handle,
            class: u32,
            buffer: *mut c_void,
            size: u32,
            returned: *mut u32,
        ) -> i32;
    }
    struct Owned(Handle);
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    fn handles(process: Handle) -> Option<Vec<Entry>> {
        // Bound allocation and work even if the process has an abnormal handle table.
        let mut count = 0;
        if unsafe { GetProcessHandleCount(process, &mut count) } == 0 || count > 8192 {
            return None;
        }
        let header = 2 * size_of::<usize>();
        let capacity = header + 8192 * size_of::<Entry>();
        // usize allocation provides alignment for the native header and entries.
        let mut buffer = vec![0usize; capacity.div_ceil(size_of::<usize>())];
        let mut returned = 0;
        if unsafe {
            NtQueryInformationProcess(
                process,
                51,
                buffer.as_mut_ptr().cast(),
                capacity as u32,
                &mut returned,
            )
        } != 0
        {
            return None;
        }
        let count = buffer[0];
        if count > 8192 || header.checked_add(count.checked_mul(size_of::<Entry>())?)? > capacity {
            return None;
        }
        let mut entries = Vec::with_capacity(count);
        for index in 0..count {
            // Native bounds and alignment were checked above. Entry is Copy and
            // the buffer lives for the whole snapshot traversal.
            let entry = unsafe {
                buffer
                    .as_ptr()
                    .cast::<u8>()
                    .add(header + index * size_of::<Entry>())
                    .cast::<Entry>()
                    .read()
            };
            entries.push(entry);
        }
        Some(entries)
    }

    pub(super) fn sessions(pid: u64) -> Option<HashSet<String>> {
        let pid = u32::try_from(pid).ok()?;
        // Derive the File object type from our own known file handle. Type indices
        // vary across Windows boots; unrelated handles may not be duplicable.
        let own_file = std::fs::File::open(std::env::current_exe().ok()?).ok()?;
        let file_type = handles(unsafe { GetCurrentProcess() })?
            .iter()
            .find(|entry| entry.handle == own_file.as_raw_handle())?
            .object_type;
        // Query limited to this exact foreground process; no machine-wide scan.
        let process = Owned(unsafe { OpenProcess(0x0400 | 0x0040, 0, pid) });
        if process.0.is_null() {
            return None;
        }
        let mut ids = HashSet::new();
        let mut path = vec![0u16; 32768];
        for entry in handles(process.0)? {
            if entry.object_type != file_type {
                continue;
            }
            let mut duplicated = std::ptr::null_mut();
            if unsafe {
                DuplicateHandle(
                    process.0,
                    entry.handle,
                    GetCurrentProcess(),
                    &mut duplicated,
                    0,
                    0,
                    2,
                )
            } == 0
            {
                // A closing/replaced handle makes this observation uncertain.
                return None;
            }
            let file = Owned(duplicated);
            if unsafe { GetFileType(file.0) } != 1 {
                continue;
            }
            let size = unsafe {
                // FILE_NAME_OPENED avoids normalizing network paths.
                GetFinalPathNameByHandleW(file.0, path.as_mut_ptr(), path.len() as u32, 8)
            } as usize;
            if size == 0 || size >= path.len() {
                return None;
            }
            let path = PathBuf::from(String::from_utf16(&path[..size]).ok()?);
            if path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                != Some("thread-writer-locks")
            {
                continue;
            }
            let name = path.file_name()?.to_str()?;
            let Some(id) = name.strip_suffix(".lock") else {
                continue;
            };
            if uuid::Uuid::parse_str(id).is_ok() {
                ids.insert(id.to_owned());
            }
            if ids.len() > 2 {
                break;
            }
        }
        Some(ids)
    }
}
