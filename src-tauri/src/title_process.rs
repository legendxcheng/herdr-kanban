//! Bounded process I/O with cancellation and descendant cleanup.
use std::{
    io::{Read, Write},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const OUTPUT_LIMIT: usize = 64 * 1024;

pub fn run(
    mut command: Command,
    input: Vec<u8>,
    stop: &AtomicBool,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + timeout;
    if stop.load(Ordering::Acquire) {
        return Err("title.cancelled".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "title.unavailable")?;
    let mut guard = Process::new(child)?;
    let overflow = Arc::new(AtomicBool::new(false));
    fn reader(
        stream: impl Read + Send + 'static,
        overflow: Arc<AtomicBool>,
    ) -> mpsc::Receiver<Vec<u8>> {
        let (send, receive) = mpsc::channel();
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stream.take(OUTPUT_LIMIT as u64 + 1).read_to_end(&mut bytes);
            if bytes.len() > OUTPUT_LIMIT {
                overflow.store(true, Ordering::Release);
            }
            let _ = send.send(bytes);
        });
        receive
    }
    let stdout = reader(
        guard.child.stdout.take().ok_or("title.io")?,
        overflow.clone(),
    );
    let stderr = reader(
        guard.child.stderr.take().ok_or("title.io")?,
        overflow.clone(),
    );
    let mut stdin = guard.child.stdin.take().ok_or("title.io")?;
    let (written, write_result) = mpsc::channel();
    thread::spawn(move || {
        let _ = written.send(stdin.write_all(&input));
    });
    let status = loop {
        if stop.load(Ordering::Acquire) {
            return Err("title.cancelled".into());
        }
        if overflow.load(Ordering::Acquire) {
            return Err("title.output_limit".into());
        }
        if Instant::now() >= deadline {
            return Err("title.timeout".into());
        }
        if let Some(status) = guard.child.try_wait().map_err(|_| "title.io")? {
            break status;
        }
        thread::sleep(Duration::from_millis(10));
    };
    // Close descendant pipe owners before collecting readers, even on normal exit.
    drop(guard);
    let remaining = || {
        deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_secs(1))
    };
    let out = stdout.recv_timeout(remaining()).map_err(|_| "title.io")?;
    let errors = stderr.recv_timeout(remaining()).map_err(|_| "title.io")?;
    if out.len() > OUTPUT_LIMIT || errors.len() > OUTPUT_LIMIT {
        return Err("title.output_limit".into());
    }
    if !status.success() {
        #[cfg(test)]
        eprintln!("CLI diagnostic: {}", String::from_utf8_lossy(&errors));
        return Err("title.cli_failed".into());
    }
    write_result
        .recv_timeout(remaining())
        .map_err(|_| "title.io")?
        .map_err(|_| "title.io")?;
    Ok(out)
}

struct Process {
    child: Child,
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}
impl Process {
    fn new(mut child: Child) -> Result<Self, String> {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::*};
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if job.is_null()
                    || SetInformationJobObject(
                        job,
                        JobObjectExtendedLimitInformation,
                        &info as *const _ as _,
                        std::mem::size_of_val(&info) as u32,
                    ) == 0
                    || AssignProcessToJobObject(job, child.as_raw_handle() as _) == 0
                {
                    let _ = child.kill();
                    let _ = child.wait();
                    if !job.is_null() {
                        CloseHandle(job);
                    }
                    return Err("title.process_isolation".into());
                }
                Ok(Self { child, job })
            }
        }
        #[cfg(not(windows))]
        {
            Ok(Self { child })
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.job);
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
#[path = "title_process_tests.rs"]
mod tests;
