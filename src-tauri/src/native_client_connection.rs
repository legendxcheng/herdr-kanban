//! A private, one-client transport adapter for Herdr's native UI.
//! It adds connection-scoped pane.zoom On to show the selected task alone.
//! A no-focus split is re-zoomed while that task remains selected. Other frames pass
//! unchanged; no terminal renderer, input translator, or public/global focus API.
#[cfg(test)]
use super::View;
use super::session::Session;
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
    #[cfg(test)]
    pub fn start(cli: &Cli, view: Arc<View>, pane: String) -> Result<Self, String> {
        Self::start_session(cli, Session::new(view, pane))
    }
    pub fn start_session(cli: &Cli, session: Arc<Session>) -> Result<Self, String> {
        let upstream = if let Some(machine) = &cli.remote {
            Endpoint::Remote(machine.clone())
        } else {
            Endpoint::Local(cli.client_endpoint()?)
        };
        let endpoint = format!("kanban-herdr-{}", uuid::Uuid::new_v4());
        let listener = Pipe::listen(&endpoint)?;
        let stop = Arc::new(AtomicBool::new(false));
        let cancelled = stop.clone();
        let worker = thread::spawn(move || {
            let result = relay(listener, &upstream, &session, &cancelled);
            if let Err(error) = result {
                if !cancelled.load(Ordering::Acquire) && session.check().is_ok() {
                    session.stop(&error);
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
fn check(view: &Session, stop: &AtomicBool, deadline: Instant) -> Result<(), String> {
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
    upstream: &Endpoint,
    view: &Session,
    stop: &AtomicBool,
) -> Result<(), String> {
    let mut deadline = Instant::now() + Duration::from_secs(15);
    while !downstream.accept()? {
        check(view, stop, deadline)?;
        thread::sleep(Duration::from_millis(2));
    }
    let mut upstream = loop {
        check(view, stop, deadline)?;
        match upstream {
            Endpoint::Remote(machine) => {
                break Upstream::Remote(crate::native_ssh::Stream::connect(machine)?);
            }
            Endpoint::Local(path) => match Pipe::connect(path) {
                Ok(pipe) => break Upstream::Local(pipe),
                Err(error) if error.raw_os_error() == Some(231) => {
                    thread::sleep(Duration::from_millis(2))
                }
                Err(error) => return Err(format!("terminal.client_connect: {error}")),
            },
        }
    };
    let mut input = Vec::new();
    let mut output = Vec::new();
    let (first, pane) = view.selected();
    let mut selection_id = first.id.clone();
    let mut position = wire::Position::new(pane);
    let mut hello = false;
    loop {
        view.check()?;
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        let (selected, pane) = view.selected();
        let active = selected.check().is_ok();
        if selection_id != selected.id {
            selection_id = selected.id.clone();
            deadline = Instant::now() + Duration::from_secs(15);
            selected.focused.store(false, Ordering::Release);
            if let Some(request) = position.retarget(pane.clone()) {
                upstream.send(&request, view, stop)?;
            }
        }
        if active && !position.ready {
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
            let current = view.selected().0;
            let input_ready = active
                && current.id == selection_id
                && selected.check().is_ok()
                && position.ready
                && selected.ready.load(Ordering::Acquire)
                && selected.presented.load(Ordering::Acquire);
            if wire::forward_input(&payload, &position.input_pane, input_ready)? {
                upstream.send(&wire::frame(&payload), view, stop)?;
            }
        }
        while let Some(payload) = wire::next_frame(&mut output)? {
            if !active {
                if position.parked_receive(&payload)? {
                    downstream.send(&wire::frame(&payload), view, stop)?;
                }
                continue;
            }
            let was_ready = position.ready;
            let (request, forward) = position.receive(&payload)?;
            if let Some(request) = request {
                if was_ready {
                    deadline = Instant::now() + Duration::from_secs(15);
                }
                selected.focused.store(false, Ordering::Release);
                upstream.send(&request, view, stop)?;
            }
            if forward {
                downstream.send(&wire::frame(&payload), view, stop)?;
            }
            if position.ready {
                selected.focused.store(true, Ordering::Release);
            }
        }
        if !received {
            thread::sleep(Duration::from_millis(2));
        }
    }
}
struct Pipe(File);
enum Endpoint {
    Local(std::path::PathBuf),
    Remote(crate::machines::Machine),
}
enum Upstream {
    Local(Pipe),
    Remote(crate::native_ssh::Stream),
}
impl Upstream {
    fn read_available(&mut self, buffer: &mut Vec<u8>) -> Result<bool, String> {
        match self {
            Self::Local(pipe) => pipe.read_available(buffer),
            Self::Remote(stream) => stream.read_available(buffer),
        }
    }
    fn send(&mut self, bytes: &[u8], view: &Session, stop: &AtomicBool) -> Result<(), String> {
        match self {
            Self::Local(pipe) => pipe.send(bytes, view, stop),
            Self::Remote(stream) => {
                let deadline = Instant::now() + Duration::from_secs(5);
                stream.send(bytes, || check(view, stop, deadline))
            }
        }
    }
}
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
    fn send(&mut self, bytes: &[u8], view: &Session, stop: &AtomicBool) -> Result<(), String> {
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
    #[ignore = "requires explicit remote acceptance fixture and native WezTerm"]
    fn live_remote_native_client_switch_and_reconnect() {
        let data = std::env::var("KANBAN_DATA_DIR").unwrap();
        assert!(data.contains("ssh-multi-machine") && data.contains("acceptance"));
        let machine: crate::machines::Machine =
            serde_json::from_slice(&std::fs::read(Path::new(&data).join("machine.json")).unwrap())
                .unwrap();
        let fixture: serde_json::Value =
            serde_json::from_slice(&std::fs::read(Path::new(&data).join("fixture.json")).unwrap())
                .unwrap();
        let workspace = fixture["workspace"].as_str().unwrap();
        let mut cli = Cli::from_env();
        cli.session = machine.session.clone();
        cli.remote = Some(machine);
        let spaces = cli
            .request(&["workspace", "list"])
            .unwrap_or_else(|e| panic!("{}", e.message));
        assert!(
            spaces["result"]["workspaces"]
                .as_array()
                .unwrap()
                .iter()
                .any(|w| w["workspace_id"] == workspace
                    && w["label"]
                        .as_str()
                        .unwrap_or("")
                        .starts_with("kanban-acceptance-"))
        );
        let panes = cli
            .request(&["pane", "list", "--workspace", workspace])
            .unwrap_or_else(|e| panic!("{}", e.message));
        let ids: Vec<_> = panes["result"]["panes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["pane_id"].as_str().unwrap().to_string())
            .collect();
        assert!(ids.len() >= 2);
        let manager = crate::native_terminal::NativeTerminalManager::default();
        for pane in [&ids[0], &ids[1], &ids[0], &ids[1]] {
            let id = uuid::Uuid::new_v4().to_string();
            let view = manager
                .select(
                    id.clone(),
                    id,
                    crate::native_terminal::Bounds {
                        x: 0.,
                        y: 0.,
                        width: 100.,
                        height: 100.,
                        viewport_width: 100.,
                        viewport_height: 100.,
                        visible: true,
                    },
                    Arc::new(|event| eprintln!("status {}", event.message)),
                )
                .unwrap();
            let profile = crate::native_terminal::profile::Profile::prepare_machine(
                &Path::new(&data).join("terminal"),
                &cli,
            )
            .unwrap();
            let connection = Connection::start(&cli, view.clone(), pane.clone()).unwrap();
            let mut command = crate::native_terminal::client_command(
                &crate::native_terminal::wezterm_executable().unwrap(),
                &cli,
                &format!("kanban-acceptance-{}", uuid::Uuid::new_v4()),
                &connection.endpoint,
            );
            profile.apply(&mut command);
            let process =
                crate::native_terminal::windows::Process::new(command.spawn().unwrap()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(25);
            while !view.focused.load(Ordering::Acquire) {
                view.check().unwrap();
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(20));
            }
            println!("native remote focus+zoom confirmed: {pane}");
            manager.close_all();
            drop(connection);
            drop(process);
            drop(profile);
        }
        let after = cli
            .request(&["pane", "list", "--workspace", workspace])
            .unwrap_or_else(|e| panic!("{}", e.message));
        assert_eq!(
            after["result"]["panes"].as_array().unwrap().len(),
            ids.len()
        );
    }
    #[test]
    #[ignore = "requires a fresh isolated kanban-acceptance session with one shell pane and native WezTerm"]
    fn live_client_zoom() {
        let cli = Cli {
            remote: None,
            executable: std::env::var("KANBAN_HERDR_EXECUTABLE").unwrap().into(),
            session: std::env::var("KANBAN_HERDR_SESSION").unwrap(),
        };
        assert!(cli.session.starts_with("kanban-acceptance-"));
        let request = |args: &[&str]| {
            cli.request(args)
                .unwrap_or_else(|e| panic!("{}", e.message))
        };
        let profile = crate::native_terminal::profile::Profile::prepare(
            &crate::db::db_path().parent().unwrap().join("terminal"),
            &cli.session,
        )
        .unwrap();
        let initial = request(&["pane", "layout", "--pane", "w1:p1"]);
        assert_eq!(
            initial["result"]["layout"]["panes"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let manager = crate::native_terminal::NativeTerminalManager::default();
        let open = |pane: &str| {
            let id = uuid::Uuid::new_v4().to_string();
            let view = manager
                .select(
                    id.clone(),
                    id,
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
            let connection = Connection::start(&cli, view.clone(), pane.into()).unwrap();
            let exe = crate::native_terminal::wezterm_executable().unwrap();
            let mut command = crate::native_terminal::client_command(
                &exe,
                &cli,
                "kanban-zoom-test",
                &connection.endpoint,
            );
            profile.apply(&mut command);
            let child = command.spawn().unwrap();
            let process = crate::native_terminal::windows::Process::new(child).unwrap();
            (view, connection, process)
        };
        let wait = |view: &View| {
            let deadline = Instant::now() + Duration::from_secs(18);
            while !view.focused.load(Ordering::Acquire) {
                view.check().unwrap();
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(10));
            }
        };
        let layout =
            |pane: &str| request(&["pane", "layout", "--pane", pane])["result"]["layout"].clone();
        let (view, connection, process) = open("w1:p1");
        wait(&view);
        assert_eq!(layout("w1:p1")["zoomed"], false); // single-pane no-op succeeds
        let split = request(&[
            "pane",
            "split",
            "w1:p1",
            "--direction",
            "right",
            "--no-focus",
        ]);
        eprintln!("split: {split}");
        let deadline = Instant::now() + Duration::from_secs(18);
        loop {
            let current = layout("w1:p1");
            if current["zoomed"] == true {
                assert_eq!(current["focused_pane_id"], "w1:p1");
                break;
            }
            view.check().unwrap();
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(30));
        }
        wait(&view);
        manager.close_all();
        drop(connection);
        drop(process);
        // Reconnect, switch both directions, and reconnect an already zoomed pane.
        for pane in ["w1:p2", "w1:p1", "w1:p1", "w1:p2"] {
            let (view, connection, process) = open(pane);
            wait(&view);
            let current = layout(pane);
            assert_eq!(current["zoomed"], true);
            assert_eq!(current["focused_pane_id"], pane);
            manager.close_all();
            drop(connection);
            drop(process);
        }
        // A superseded connection cannot win the final selection.
        let (_old, old_connection, old_process) = open("w1:p1");
        let (view, connection, process) = open("w1:p2");
        drop(old_connection);
        drop(old_process);
        wait(&view);
        assert_eq!(layout("w1:p2")["focused_pane_id"], "w1:p2");
        manager.close_all();
        drop(connection);
        drop(process);
        let panes = request(&["pane", "list"]);
        assert_eq!(panes["result"]["panes"].as_array().unwrap().len(), 2);
        eprintln!(
            "single pane, background split, repeated zoom, reconnect, switching, cancellation and pane survival passed"
        );
    }

    #[test]
    #[ignore = "requires an isolated running Herdr session and native WezTerm"]
    fn live_client_focus() {
        let cli = Cli {
            remote: None,
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
        let profile = crate::native_terminal::profile::Profile::prepare(
            &crate::db::db_path().parent().unwrap().join("terminal"),
            &cli.session,
        )
        .unwrap();
        let mut command = crate::native_terminal::client_command(
            &exe,
            &cli,
            "kanban-connection-test",
            &connection.endpoint,
        );
        profile.apply(&mut command);
        let child = command.spawn().unwrap();
        let _process = crate::native_terminal::windows::Process::new(child).unwrap();
        let deadline = Instant::now() + Duration::from_secs(18);
        while !view.focused.load(Ordering::Acquire) {
            view.check().unwrap();
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        manager.close_all();
    }
}
