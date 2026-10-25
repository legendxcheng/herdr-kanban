//! A private, one-client transport adapter for Herdr's native UI.
//! It adds only the initial connection-scoped pane.focus. All other frames pass
//! unchanged; no terminal renderer, input translator, or public/global focus API.
use super::View;
use crate::herdr::Cli;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::windows::io::{AsRawHandle, FromRawHandle},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_NO_DATA, ERROR_PIPE_CONNECTED, ERROR_PIPE_LISTENING, INVALID_HANDLE_VALUE, LocalFree,
    },
    Security::{
        Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, SECURITY_ATTRIBUTES,
    },
    Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX},
    System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_NOWAIT, PIPE_REJECT_REMOTE_CLIENTS, PeekNamedPipe,
        SetNamedPipeHandleState,
    },
};
#[path = "native_client_wire.rs"]
mod wire;

pub struct Connection {
    pub endpoint: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Connection {
    pub fn start(cli: &Cli, view: Arc<View>, pane: String) -> Result<Self, String> {
        let upstream = cli.client_endpoint()?;
        let endpoint = format!("kanban-herdr-{}", uuid::Uuid::new_v4());
        let listener = Pipe::listen(&endpoint)?;
        let stop = Arc::new(AtomicBool::new(false));
        let cancelled = stop.clone();
        let worker = thread::spawn(move || {
            let result = relay(listener, &upstream, &view, &cancelled, pane);
            if let Err(error) = result {
                if !cancelled.load(Ordering::Acquire) && view.check().is_ok() {
                    view.stop(&error);
                }
            }
        });
        Ok(Self {
            endpoint,
            stop,
            worker: Some(worker),
        })
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn check(view: &View, stop: &AtomicBool, deadline: Instant) -> Result<(), String> {
    view.check()?;
    if stop.load(Ordering::Acquire) {
        return Err("terminal.stale".into());
    }
    if Instant::now() > deadline {
        return Err("terminal.client_focus_timeout".into());
    }
    Ok(())
}
fn relay(
    mut downstream: Pipe,
    upstream: &Path,
    view: &View,
    stop: &AtomicBool,
    pane: String,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !downstream.accept()? {
        check(view, stop, deadline)?;
        thread::sleep(Duration::from_millis(2));
    }
    let mut upstream = loop {
        check(view, stop, deadline)?;
        match Pipe::connect(upstream) {
            Ok(pipe) => break pipe,
            Err(error) if error.raw_os_error() == Some(231) => {
                thread::sleep(Duration::from_millis(2))
            }
            Err(error) => return Err(format!("terminal.client_connect: {error}")),
        }
    };
    let mut input = Vec::new();
    let mut output = Vec::new();
    let mut position = wire::Position::new(pane);
    let mut hello = false;
    loop {
        view.check()?;
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        if !position.ready {
            check(view, stop, deadline)?;
        }
        let received =
            downstream.read_available(&mut input)? | upstream.read_available(&mut output)?;
        while let Some(payload) = wire::next_frame(&mut input)? {
            if !hello {
                let (kind, doc) = wire::control(&payload)?.ok_or("terminal.client_protocol")?;
                if kind != "endpoint.hello.v1" || doc["generation"] != 1 {
                    return Err("terminal.client_protocol".into());
                }
                hello = true;
            }
            upstream.send(&wire::frame(&payload), view, stop)?;
        }
        while let Some(payload) = wire::next_frame(&mut output)? {
            let (request, forward) = if position.ready {
                (None, true)
            } else {
                position.receive(&payload)?
            };
            if let Some(request) = request {
                upstream.send(&request, view, stop)?;
            }
            if forward {
                downstream.send(&wire::frame(&payload), view, stop)?;
            }
            if position.ready {
                view.focused.store(true, Ordering::Release);
            }
        }
        if !received {
            thread::sleep(Duration::from_millis(2));
        }
    }
}
struct Pipe(File);
impl Pipe {
    fn listen(endpoint: &str) -> Result<Self, String> {
        let path: Vec<u16> = format!(r"\\.\pipe\{endpoint}")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // Only the creating user and SYSTEM; reject remote clients as well.
        let sddl: Vec<u16> = "D:P(A;;GA;;;OW)(A;;GA;;;SY)"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor = std::ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(error());
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let handle = unsafe {
            CreateNamedPipeW(
                path.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                65536,
                65536,
                0,
                &attributes,
            )
        };
        unsafe {
            LocalFree(descriptor);
        }
        if handle == INVALID_HANDLE_VALUE {
            return Err(error());
        }
        Ok(Self(unsafe { File::from_raw_handle(handle) }))
    }
    fn accept(&self) -> Result<bool, String> {
        if unsafe { ConnectNamedPipe(self.0.as_raw_handle(), std::ptr::null_mut()) } != 0 {
            return Ok(true);
        }
        match std::io::Error::last_os_error()
            .raw_os_error()
            .map(|e| e as u32)
        {
            Some(ERROR_PIPE_CONNECTED) => Ok(true),
            Some(ERROR_PIPE_LISTENING) => Ok(false),
            _ => Err(error()),
        }
    }
    fn connect(path: &Path) -> std::io::Result<Self> {
        let name = path.to_string_lossy();
        let path = if name.starts_with(r"\\.\pipe\") {
            name.into_owned()
        } else {
            format!(r"\\.\pipe\{name}")
        };
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let mode = PIPE_NOWAIT;
        if unsafe {
            SetNamedPipeHandleState(
                file.as_raw_handle(),
                &mode,
                std::ptr::null(),
                std::ptr::null(),
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self(file))
    }
    fn read_available(&mut self, buffer: &mut Vec<u8>) -> Result<bool, String> {
        let mut available = 0;
        if unsafe {
            PeekNamedPipe(
                self.0.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(error());
        }
        if available == 0 {
            return Ok(false);
        }
        let mut bytes = [0u8; 65536];
        let count = self
            .0
            .read(&mut bytes)
            .map_err(|e| format!("terminal.client_connect: {e}"))?;
        if count == 0 {
            return Err("terminal.disconnected".into());
        }
        buffer.extend_from_slice(&bytes[..count]);
        if buffer.len() > wire::MAX_FRAME + 65540 {
            return Err("terminal.client_protocol".into());
        }
        Ok(true)
    }
    fn send(&mut self, bytes: &[u8], view: &View, stop: &AtomicBool) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut remaining = bytes;
        while !remaining.is_empty() {
            check(view, stop, deadline)?;
            match self.0.write(remaining) {
                Ok(0) => thread::sleep(Duration::from_millis(2)),
                Ok(count) => remaining = &remaining[count..],
                Err(error)
                    if error.raw_os_error() == Some(ERROR_NO_DATA as i32)
                        || error.kind() == std::io::ErrorKind::WouldBlock =>
                {
                    thread::sleep(Duration::from_millis(2))
                }
                Err(error) => return Err(format!("terminal.client_connect: {error}")),
            }
        }
        Ok(())
    }
}
fn error() -> String {
    format!(
        "terminal.client_connect: {}",
        std::io::Error::last_os_error()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires an isolated running Herdr session and native WezTerm"]
    fn live_client_focus() {
        let cli = Cli {
            executable: std::env::var("KANBAN_HERDR_EXECUTABLE").unwrap().into(),
            session: std::env::var("KANBAN_HERDR_SESSION").unwrap(),
        };
        let manager = crate::native_terminal::NativeTerminalManager::default();
        let view = manager
            .select(
                "probe".into(),
                "probe".into(),
                crate::native_terminal::Bounds {
                    x: 0.,
                    y: 0.,
                    width: 100.,
                    height: 100.,
                    viewport_width: 100.,
                    viewport_height: 100.,
                    visible: true,
                },
                Arc::new(|event| eprintln!("status: {}", event.message)),
            )
            .unwrap();
        let connection = Connection::start(&cli, view.clone(), "w1:p1".into()).unwrap();
        let exe = crate::native_terminal::wezterm_executable().unwrap();
        let child = crate::native_terminal::client_command(
            &exe,
            &cli,
            "kanban-connection-test",
            &connection.endpoint,
        )
        .spawn()
        .unwrap();
        let _process = crate::native_terminal::windows::Process::new(child).unwrap();
        let deadline = Instant::now() + Duration::from_secs(18);
        while !view.focused.load(Ordering::Acquire) {
            check(&view, &AtomicBool::new(false), deadline).unwrap();
            thread::sleep(Duration::from_millis(10));
        }
        manager.close_all();
    }
}
