//! A real Herdr client hosted by an independent WezTerm window.
//! No terminal rendering, mouse decoding, SetParent, owner, or global topmost.
use crate::{herdr::Cli, workflow::Workflow};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, serde::Serialize)]
pub struct Output {
    pub kind: &'static str,
    pub message: String,
}
#[derive(Clone, Copy, serde::Deserialize, Debug, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub viewport_width: f64,
    pub viewport_height: f64,
    pub visible: bool,
}
impl Bounds {
    pub fn validate(&self) -> Result<(), String> {
        if [
            self.x,
            self.y,
            self.width,
            self.height,
            self.viewport_width,
            self.viewport_height,
        ]
        .iter()
        .any(|n| !n.is_finite())
            || self.x < 0.0
            || self.y < 0.0
            || self.width < 0.0
            || self.height < 0.0
            || !(1.0..=32768.0).contains(&self.viewport_width)
            || !(1.0..=32768.0).contains(&self.viewport_height)
            || self.x + self.width > self.viewport_width + 2.0
            || self.y + self.height > self.viewport_height + 2.0
        {
            return Err("terminal.size".into());
        }
        Ok(())
    }
}

pub type Status = Arc<dyn Fn(Output) + Send + Sync>;
#[derive(Clone, PartialEq)]
pub enum Owner {
    Task(String),
    Workspace(String),
}
pub struct View {
    id: String,
    owner: Owner,
    bounds: Mutex<Bounds>,
    stopped: AtomicBool,
    ready: AtomicBool,
    presented: AtomicBool,
    focused: AtomicBool,
    status: Status,
    open_gate: Arc<Mutex<()>>,
    font: Arc<Mutex<font::Preferences>>,
    session: Mutex<std::sync::Weak<session::Session>>,
    task_lease: Mutex<Option<TaskLease>>,
    resume_only: AtomicBool,
}
#[derive(Clone)]
struct TaskLease {
    app: Arc<Workflow>,
    task: String,
    authorization: crate::task_view::Authorization,
}
impl View {
    pub fn resume_only(&self) {
        self.resume_only.store(true, Ordering::Release);
    }
    fn read_only(&self) -> Result<bool, String> {
        self.check()?;
        let lease = self.task_lease.lock().map_err(err)?;
        if let Some(lease) = lease.as_ref() {
            lease.app.check_view(&lease.task, &lease.authorization)?;
            Ok(lease.authorization.read_only)
        } else {
            Ok(false)
        }
    }
    fn stop(&self, message: &str) {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(session) = self.session.lock().ok().and_then(|s| s.upgrade()) {
            session.pause(&self.id);
        }
        (self.status)(Output {
            kind: "closed",
            message: message.into(),
        });
    }
    fn check(&self) -> Result<(), String> {
        if self.stopped.load(Ordering::Acquire) {
            Err("terminal.stale".into())
        } else {
            Ok(())
        }
    }
}
#[derive(Default)]
pub struct NativeTerminalManager {
    slot: Mutex<Option<Arc<View>>>,
    open_gate: Arc<Mutex<()>>,
    font: Arc<Mutex<font::Preferences>>,
    sessions: Mutex<std::collections::HashMap<String, Arc<session::Session>>>,
    disabled: Mutex<std::collections::HashSet<String>>,
    stopped: AtomicBool,
}
impl NativeTerminalManager {
    pub fn select(
        &self,
        id: String,
        task: String,
        bounds: Bounds,
        status: Status,
    ) -> Result<Arc<View>, String> {
        self.select_owner(id, Owner::Task(task), bounds, status)
    }
    pub fn select_owner(
        &self,
        id: String,
        owner: Owner,
        bounds: Bounds,
        status: Status,
    ) -> Result<Arc<View>, String> {
        if self.stopped.load(Ordering::Acquire) {
            return Err("terminal.closed".into());
        }
        bounds.validate()?;
        let owner_id = match &owner {
            Owner::Task(id) | Owner::Workspace(id) => id,
        };
        if id.is_empty() || owner_id.is_empty() {
            return Err("terminal.stale".into());
        }
        let view = Arc::new(View {
            id,
            owner,
            bounds: Mutex::new(bounds),
            stopped: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            presented: AtomicBool::new(false),
            focused: AtomicBool::new(false),
            status,
            open_gate: self.open_gate.clone(),
            font: self.font.clone(),
            session: Mutex::new(std::sync::Weak::new()),
            task_lease: Mutex::new(None),
            resume_only: AtomicBool::new(false),
        });
        let mut slot = self.slot.lock().map_err(err)?;
        if self.stopped.load(Ordering::Acquire) {
            return Err("terminal.closed".into());
        }
        if let Some(old) = slot.replace(view.clone()) {
            old.stop("terminal.switched");
        }
        Ok(view)
    }
    pub fn bounds(&self, id: &str, bounds: Bounds) -> Result<(), String> {
        bounds.validate()?;
        let slot = self.slot.lock().map_err(err)?;
        let view = slot
            .as_ref()
            .filter(|v| v.id == id && !v.stopped.load(Ordering::Acquire))
            .ok_or("terminal.stale")?;
        *view.bounds.lock().map_err(err)? = bounds;
        view.presented.store(true, Ordering::Release);
        Ok(())
    }
    pub fn close(&self, id: &str) {
        if let Ok(mut slot) = self.slot.lock() {
            if slot.as_ref().is_some_and(|v| v.id == id) {
                if let Some(view) = slot.take() {
                    view.stop("terminal.closed");
                }
            }
        }
    }
    pub fn close_task(&self, task: &str) {
        if let Ok(mut slot) = self.slot.lock() {
            if slot
                .as_ref()
                .is_some_and(|v| v.owner == Owner::Task(task.into()))
            {
                if let Some(view) = slot.take() {
                    view.stop("terminal.closed");
                }
            }
        }
        let views = self
            .sessions
            .lock()
            .map(|sessions| {
                sessions
                    .values()
                    .map(|s| s.selected().0)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for view in views {
            if view.owner == Owner::Task(task.into()) {
                view.stop("terminal.closed");
            }
        }
    }
    pub fn close_all(&self) {
        let view = self.slot.lock().ok().and_then(|mut slot| slot.take());
        if let Some(view) = view {
            view.stop("terminal.closed");
        }
        let sessions = self
            .sessions
            .lock()
            .map(|mut sessions| std::mem::take(&mut *sessions))
            .unwrap_or_default();
        for session in sessions.values() {
            session.stop("terminal.closed");
        }
    }

    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        self.close_all();
    }

    pub fn disable_machine(&self, machine: &str) {
        self.disabled.lock().unwrap().insert(machine.into());
        let retired = {
            let mut sessions = self.sessions.lock().unwrap();
            let keys: Vec<_> = sessions
                .iter()
                .filter(|(_, s)| s.machine.as_deref() == Some(machine))
                .map(|(k, _)| k.clone())
                .collect();
            keys.into_iter()
                .filter_map(|k| sessions.remove(&k))
                .collect::<Vec<_>>()
        };
        for session in retired {
            session.stop("machine.disabled");
        }
    }

    pub fn enable_machine(&self, machine: &str) {
        self.disabled.lock().unwrap().remove(machine);
    }

    // Called under open_gate after fresh task/Workspace authorization.
    fn retain(
        &self,
        cli: &Cli,
        view: Arc<View>,
        pane: String,
    ) -> Result<(Arc<session::Session>, bool), String> {
        if self.stopped.load(Ordering::Acquire) {
            return Err("terminal.closed".into());
        }
        if let Some(machine) = &cli.remote {
            if machine.disabled || self.disabled.lock().map_err(err)?.contains(&machine.id) {
                return Err("machine.disabled".into());
            }
        }
        view.check()?;
        let key = session::key(cli);
        let mut sessions = self.sessions.lock().map_err(err)?;
        if self.stopped.load(Ordering::Acquire) {
            return Err("terminal.closed".into());
        }
        if cli
            .remote
            .as_ref()
            .is_some_and(|m| self.disabled.lock().unwrap().contains(&m.id))
        {
            return Err("machine.disabled".into());
        }
        let mut retired = Vec::new();
        sessions.retain(|old_key, session| {
            if session.stopped.load(Ordering::Acquire) {
                return false;
            }
            if cli
                .remote
                .as_ref()
                .is_some_and(|m| session.machine.as_ref() == Some(&m.id))
                && old_key != &key
            {
                retired.push((session.clone(), "machine.identity_changed"));
                return false;
            }
            true
        });
        let (session, fresh) = if let Some(session) = sessions.get(&key) {
            (session.clone(), false)
        } else {
            // Only local viewers retain the old bounded resource policy.
            if cli.remote.is_none()
                && sessions.values().filter(|s| s.machine.is_none()).count() >= 3
            {
                if let Some(oldest) = sessions
                    .iter()
                    .filter(|(_, s)| s.machine.is_none())
                    .min_by_key(|(_, s)| *s.touched.lock().unwrap())
                    .map(|(key, _)| key.clone())
                {
                    if let Some(old) = sessions.remove(&oldest) {
                        retired.push((old, "terminal.closed"));
                    }
                }
            }
            let session = session::Session::for_client(view.clone(), pane.clone(), cli);
            sessions.insert(key, session.clone());
            (session, true)
        };
        drop(sessions);
        for (old, reason) in retired {
            old.stop(reason);
        }
        if !fresh {
            session.select(view, pane)?;
        }
        session.check()?;
        Ok((session, fresh))
    }

    pub fn open(
        &self,
        view: Arc<View>,
        app: Arc<Workflow>,
        main_hwnd: isize,
    ) -> Result<(), String> {
        let started = Instant::now();
        let result = (|| {
            use crate::workflow::Herdr;
            view.check()?;
            let routed;
            let shell;
            let target;
            let authorization;
            let mut cli;
            let _shell_connection;
            match &view.owner {
                Owner::Workspace(project) => {
                    routed = app.for_project(project)?;
                    _shell_connection = Some(crate::remote_control::Scope::enter());
                    shell = Some(if view.resume_only.load(Ordering::Acquire) {
                        routed.saved_workspace_terminal(project)?
                    } else {
                        routed.ensure_workspace_terminal(project, &|| view.check())?
                    });
                    target = None;
                    authorization = None;
                    cli = routed.herdr.terminal_client()?;
                    if !cli.workspace_terminal_live(shell.as_ref().unwrap())? {
                        return Err("workspace_terminal.missing".into());
                    }
                }
                Owner::Task(task) => {
                    _shell_connection = None;
                    routed = app.for_task(task)?;
                    let permission = routed.view_task(task)?;
                    target = Some(permission.target.clone());
                    *view.task_lease.lock().map_err(err)? = Some(TaskLease {
                        app: routed.clone(),
                        task: task.clone(),
                        authorization: permission.clone(),
                    });
                    authorization = Some(permission);
                    shell = None;
                    cli = app.herdr.terminal_client()?;
                }
            }
            if let Some(target) = &target {
                if target.machine_id != crate::machines::LOCAL_ID {
                    let machine =
                        crate::machines::get(&*app.db.lock().map_err(err)?, &target.machine_id)?;
                    if machine.disabled {
                        return Err("machine.disabled".into());
                    }
                    // view_task already checked the target and viewing authorization.
                    // Normal viewing does not require Codex session reporting.
                    // Setup may have published a newer helper under its own hash.
                    cli.remote = Some(crate::machines::get(
                        &*app.db.lock().map_err(err)?,
                        &target.machine_id,
                    )?);
                    cli.session = target.session.clone();
                } else {
                    if cli.session != target.session {
                        return Err("binding.identity_changed".into());
                    }
                    cli.validate(&target.workspace_id, &target)?;
                }
            }
            let check_target = || -> Result<(), String> {
                match (&view.owner, &target, &shell) {
                    (Owner::Task(task), Some(_), _) => {
                        routed.check_view(task, authorization.as_ref().ok_or("terminal.stale")?)
                    }
                    (Owner::Workspace(project), _, Some(shell))
                        if routed.saved_workspace_terminal(project)? == *shell =>
                    {
                        Ok(())
                    }
                    _ => Err("binding.identity_changed".into()),
                }
            };
            let pane_id = target
                .as_ref()
                .map(|t| &t.pane_id)
                .or_else(|| shell.as_ref().map(|t| &t.pane_id))
                .ok_or("terminal.stale")?;
            view.check()?;
            check_target()?;
            session::record(
                "authorized",
                &view.id,
                cli.remote.as_ref().map(|m| m.id.as_str()),
                None,
                started.elapsed(),
            );
            let _gate = view.open_gate.lock().map_err(err)?;
            view.check()?;
            check_target()?;
            let (session, fresh) = self.retain(&cli, view.clone(), pane_id.clone())?;
            session::record(
                if fresh { "created" } else { "reused" },
                &view.id,
                session.machine.as_deref(),
                Some(&session.id),
                started.elapsed(),
            );
            if fresh {
                if let Err(error) = launch(session.clone(), &cli, main_hwnd) {
                    session.stop(&error);
                    return Err(error);
                }
            }
            let deadline = Instant::now() + Duration::from_secs(15);
            while !(session.window_found.load(Ordering::Acquire)
                && view.focused.load(Ordering::Acquire))
            {
                view.check()?;
                session.check()?;
                if Instant::now() >= deadline {
                    session.stop("terminal.client_focus_timeout");
                    return Err("terminal.client_focus_timeout".into());
                }
                thread::sleep(Duration::from_millis(10));
            }
            view.check()?;
            check_target()?;
            if let Some(shell) = &shell {
                if !cli.workspace_terminal_live(shell)? {
                    return Err("workspace_terminal.missing".into());
                }
                view.check()?;
                check_target()?;
            }
            view.ready.store(true, Ordering::Release);
            session::record(
                "ready",
                &view.id,
                session.machine.as_deref(),
                Some(&session.id),
                started.elapsed(),
            );
            (view.status)(Output {
                kind: "mode",
                message: if authorization.as_ref().is_some_and(|a| a.read_only) {
                    "read_only"
                } else {
                    "interactive"
                }
                .into(),
            });
            if let Some(lease) = view.task_lease.lock().map_err(err)?.clone() {
                let weak = Arc::downgrade(&view);
                thread::spawn(move || {
                    while let Some(view) = weak.upgrade() {
                        if view.check().is_err() {
                            break;
                        }
                        if let Err(error) =
                            lease.app.validate_view(&lease.task, &lease.authorization)
                        {
                            view.stop(&error);
                            break;
                        }
                        drop(view);
                        thread::sleep(Duration::from_secs(1));
                    }
                });
            }
            Ok(())
        })();
        if let Err(ref error) = result {
            view.stop(error);
        }
        result
    }
}
impl Drop for NativeTerminalManager {
    fn drop(&mut self) {
        self.close_all();
    }
}

// Keep native input and the user's settings, with a per-view hidden sidebar.
#[cfg(test)]
fn client_command(executable: &Path, cli: &Cli, class: &str, endpoint: &str) -> Command {
    client_command_with_font(
        executable,
        cli,
        class,
        endpoint,
        "window_decorations=\"NONE\"",
    )
}

fn client_command_with_font(
    executable: &Path,
    cli: &Cli,
    class: &str,
    endpoint: &str,
    font_override: &str,
) -> Command {
    let mut command = Command::new(executable);
    command
        .args([
            "--config",
            font_override,
            "--config",
            "enable_tab_bar=false",
            "start",
            "--always-new-process",
            "--no-auto-connect",
            "--class",
            class,
            "--position=-32000,-32000",
            "--",
        ])
        .arg(&cli.executable)
        .arg("client")
        // --session overrides the client socket override in Herdr. The environment
        // keeps the real session/config identity while using our private connection.
        .env("HERDR_SESSION", &cli.session)
        .env("HERDR_CLIENT_SOCKET_PATH", endpoint)
        // The identity matches the copied chrome preferences in the view's
        // private state directory; it does not enable remote mode.
        .env(
            "HERDR_REMOTE_PREFERENCES_IDENTITY",
            serde_json::to_string(&profile::machine_identity(cli)).unwrap(),
        );
    // `client` connects only to an existing server. Never auto-start a server
    // (and therefore Agents) inside the viewer's kill-on-close Windows Job.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("WEZTERM_") {
            command.env_remove(key);
        }
    }
    for key in [
        "HERDR_ENV",
        "HERDR_SOCKET_PATH",
        "HERDR_PANE_ID",
        "HERDR_TAB_ID",
        "HERDR_WORKSPACE_ID",
    ] {
        command.env_remove(key);
    }
    command.stdin(Stdio::null()).stdout(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}
fn launch(session: Arc<session::Session>, cli: &Cli, main_hwnd: isize) -> Result<(), String> {
    let view = session.selected().0;
    let connection = connection::Connection::start_session(cli, session.clone())?;
    let executable = wezterm_executable()?;
    let class = format!("kanban-terminal-{}", uuid::Uuid::new_v4());
    let config_dir = crate::db::db_path()
        .parent()
        .ok_or("terminal.window_timeout")?
        .join("terminal");
    std::fs::create_dir_all(&config_dir).map_err(err)?;
    let profile = profile::Profile::prepare_machine(&config_dir, cli)?;
    *session.profile.lock().map_err(err)? = Some(profile.directory().to_path_buf());
    let diagnostics = std::fs::File::create(config_dir.join("wezterm.log")).map_err(err)?;
    let font_override = {
        let mut font = view.font.lock().map_err(err)?;
        view.check()?;
        font.activate(&session.id, &config_dir, profile.directory())?
    };
    let mut command = client_command_with_font(
        &executable,
        cli,
        &class,
        &connection.endpoint,
        &font_override,
    );
    profile.apply(&mut command);
    let child = command
        .stderr(diagnostics)
        .spawn()
        .map_err(|e| format!("terminal.wezterm_unavailable: {e}"))?;
    let mut process = windows::Process::new(child)?;
    let pid = process.id();
    let monitor = session.clone();
    thread::spawn(move || {
        let _connection = connection;
        let events = windows::WindowEvents::new(main_hwnd);
        let mut window_sync = windows::WindowSync::default();
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut hwnd = 0;
        while !monitor.stopped.load(Ordering::Acquire) {
            let selected = monitor.selected().0;
            monitor.expire_if_idle();
            let active = selected.check().is_ok();
            if monitor.check().is_err() {
                break;
            }
            if let Ok(mut font) = selected.font.lock() {
                font.poll(&monitor.id);
            }
            if process.exited() {
                monitor.stop("terminal.disconnected");
                break;
            }
            if hwnd == 0 {
                hwnd = windows::find(pid, &class);
                if hwnd != 0 {
                    monitor.window_found.store(true, Ordering::Release);
                    monitor.hwnd.store(hwnd, Ordering::Release);
                }
                if hwnd == 0 && Instant::now() > deadline {
                    monitor.stop("terminal.window_timeout");
                    break;
                }
            }
            if hwnd != 0 {
                if !windows::valid(hwnd, pid) {
                    monitor.stop("terminal.disconnected");
                    break;
                }
                window_sync.sync(
                    main_hwnd,
                    hwnd,
                    *selected.bounds.lock().unwrap(),
                    active
                        && selected.ready.load(Ordering::Acquire)
                        && selected.focused.load(Ordering::Acquire)
                        && selected.presented.load(Ordering::Acquire),
                );
            }
            events.wait();
        }
        if hwnd != 0 {
            windows::hide(hwnd);
        }
        monitor.hwnd.store(0, Ordering::Release);
        // Drop closes this WezTerm and its Herdr CLIENT only, never the server.
        drop(process);
        drop(profile);
    });
    Ok(())
}

fn wezterm_executable() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("KANBAN_WEZTERM_EXECUTABLE") {
        let mut path = PathBuf::from(path);
        if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("wezterm.exe"))
        {
            path.set_file_name("wezterm-gui.exe");
        }
        return path
            .is_file()
            .then_some(path)
            .ok_or_else(|| "terminal.wezterm_unavailable".into());
    }
    let mut dirs: Vec<_> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    for key in ["ProgramFiles", "LOCALAPPDATA"] {
        if let Some(root) = std::env::var_os(key) {
            dirs.push(PathBuf::from(root).join(if key == "LOCALAPPDATA" {
                "Programs/WezTerm"
            } else {
                "WezTerm"
            }));
        }
    }
    dirs.into_iter()
        .map(|p| p.join("wezterm-gui.exe"))
        .find(|p| p.is_file())
        .ok_or_else(|| "terminal.wezterm_unavailable".into())
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[path = "native_client_profile.rs"]
mod profile;

#[path = "native_font.rs"]
mod font;

#[path = "native_view_session.rs"]
mod session;

#[cfg(windows)]
#[path = "native_client_connection.rs"]
mod connection;
#[cfg(windows)]
#[path = "native_terminal_windows.rs"]
mod windows;

#[cfg(test)]
mod tests {
    use super::*;
    fn bounds() -> Bounds {
        Bounds {
            x: 800.0,
            y: 48.0,
            width: 400.0,
            height: 700.0,
            viewport_width: 1200.0,
            viewport_height: 800.0,
            visible: true,
        }
    }
    #[test]
    fn four_remote_clients_survive_idle_and_switching_until_machine_disable_or_shutdown() {
        let native = NativeTerminalManager::default();
        let mut retained = Vec::new();
        for n in 0..4 {
            let machine = crate::machines::Machine {
                id: format!("machine-{n}"),
                name: "same display name".into(),
                kind: "ssh".into(),
                profile_id: Some(format!("profile-{n}")),
                ssh_target: Some(format!("host-{n}")),
                session: "default".into(),
                platform: None,
                disabled: false,
                helper_name: Some("helper".into()),
                host_identity: Some(format!("host-identity-{n}")),
            };
            let cli = Cli {
                executable: "herdr".into(),
                session: "default".into(),
                remote: Some(machine),
            };
            let view = native
                .select(
                    format!("view-{n}"),
                    format!("task-{n}"),
                    bounds(),
                    Arc::new(|_| {}),
                )
                .unwrap();
            let (session, fresh) = native.retain(&cli, view, "pane".into()).unwrap();
            assert!(fresh);
            native.close(&format!("view-{n}"));
            *session.touched.lock().unwrap() = Instant::now() - Duration::from_secs(3600);
            session.expire_if_idle();
            retained.push((cli, session));
        }
        for (n, (cli, old)) in retained.iter().enumerate() {
            let view = native
                .select(
                    format!("return-{n}"),
                    "another-task".into(),
                    bounds(),
                    Arc::new(|_| {}),
                )
                .unwrap();
            let (session, fresh) = native.retain(cli, view, "other-pane".into()).unwrap();
            assert!(!fresh, "return must reuse even the first of four machines");
            assert_eq!(old.id, session.id);
            assert!(session.check().is_ok());
        }
        native.disable_machine("machine-0");
        assert!(retained[0].1.check().is_err());
        assert!(retained[1].1.check().is_ok());
        let view = native
            .select(
                "late".into(),
                "late-task".into(),
                bounds(),
                Arc::new(|_| {}),
            )
            .unwrap();
        assert!(native.retain(&retained[0].0, view, "pane".into()).is_err());
        native.shutdown();
        assert!(retained.iter().all(|(_, s)| s.check().is_err()));
        assert!(
            native
                .select(
                    "late-exit".into(),
                    "task".into(),
                    bounds(),
                    Arc::new(|_| {})
                )
                .is_err()
        );
    }
    #[test]
    fn completing_a_task_releases_its_view_but_preserves_the_client_for_other_tasks() {
        let native = NativeTerminalManager::default();
        let first = native
            .select("a".into(), "task-a".into(), bounds(), Arc::new(|_| {}))
            .unwrap();
        let session = session::Session::new(first.clone(), "pane-a".into());
        native
            .sessions
            .lock()
            .unwrap()
            .insert("machine".into(), session.clone());
        native.close_task("task-a");
        assert!(first.check().is_err());
        assert!(
            session.check().is_ok(),
            "task completion must not close the retained machine client"
        );
        let next = native
            .select("b".into(), "task-b".into(), bounds(), Arc::new(|_| {}))
            .unwrap();
        session.select(next.clone(), "pane-b".into()).unwrap();
        assert!(next.check().is_ok());
        native.close_all();
        assert!(session.check().is_err());
    }
    #[test]
    fn old_close_and_geometry_cannot_change_new_connection() {
        let native = NativeTerminalManager::default();

        let first = native
            .select("a".into(), "task-a".into(), bounds(), Arc::new(|_| {}))
            .unwrap();
        let second = native
            .select("b".into(), "task-b".into(), bounds(), Arc::new(|_| {}))
            .unwrap();
        assert!(first.stopped.load(Ordering::Acquire));
        native.close("a");
        native.close_task("task-a");
        assert!(!second.stopped.load(Ordering::Acquire));
        assert!(native.bounds("a", bounds()).is_err());
        native.bounds("b", bounds()).unwrap();
        native.close_task("task-b");
        assert!(second.stopped.load(Ordering::Acquire));
    }
    #[test]
    fn task_cleanup_cannot_close_workspace_view_and_stale_close_is_ignored() {
        let native = NativeTerminalManager::default();
        let shell = native
            .select_owner(
                "shell".into(),
                Owner::Workspace("same-id".into()),
                bounds(),
                Arc::new(|_| {}),
            )
            .unwrap();
        native.close_task("same-id");
        assert!(!shell.stopped.load(Ordering::Acquire));
        let task = native
            .select("task".into(), "same-id".into(), bounds(), Arc::new(|_| {}))
            .unwrap();
        assert!(shell.stopped.load(Ordering::Acquire));
        native.close("shell");
        assert!(!task.stopped.load(Ordering::Acquire));
    }
    #[test]
    fn launches_official_client_with_font_hooks_and_normal_user_config() {
        let cli = Cli {
            remote: None,
            executable: "C:/Tools/Herdr/herdr.exe".into(),
            session: "test-session".into(),
        };
        let base = std::env::temp_dir().join(format!("kanban-command-{}", uuid::Uuid::new_v4()));
        let profile = profile::Profile::fixture(&base, &cli.session);
        let font_override = font::Preferences::default()
            .activate("test-window", &base, profile.directory())
            .unwrap();
        let mut command = client_command_with_font(
            Path::new("wezterm-gui.exe"),
            &cli,
            "test-window",
            "private-test-pipe",
            &font_override,
        );
        profile.apply(&mut command);
        let args: Vec<_> = command
            .get_args()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            &args[args.len() - 2..],
            &["C:/Tools/Herdr/herdr.exe", "client"]
        );
        assert!(!args.iter().any(|a| a == "--session" || a == "focus"));
        assert!(args.contains(&font_override));
        for (key, expected) in [
            ("HERDR_SESSION", "test-session"),
            ("HERDR_CLIENT_SOCKET_PATH", "private-test-pipe"),
        ] {
            assert!(
                command
                    .get_envs()
                    .any(|(name, value)| name == key
                        && value == Some(std::ffi::OsStr::new(expected)))
            );
        }
        assert!(!args.iter().any(|arg| arg.contains("bridge")
            || arg.contains("mouse_bindings")
            || arg == "--config-file"));
        for key in [
            "HERDR_ENV",
            "HERDR_SOCKET_PATH",
            "HERDR_PANE_ID",
            "HERDR_TAB_ID",
            "HERDR_WORKSPACE_ID",
        ] {
            assert!(
                command
                    .get_envs()
                    .any(|(name, value)| name == key && value.is_none())
            );
        }
        assert!(
            command
                .get_envs()
                .any(|(key, value)| key == "HERDR_CONFIG_PATH" && value.is_some())
        );
        assert!(
            command
                .get_envs()
                .any(|(key, value)| key == "XDG_STATE_HOME" && value.is_some())
        );
        drop(profile);
        std::fs::remove_dir(&base).unwrap();
    }
    #[test]
    fn rejects_nonfinite_and_offscreen_webview_rectangles() {
        let mut rect = bounds();
        rect.x = f64::NAN;
        assert!(rect.validate().is_err());
        rect = bounds();
        rect.width = 2000.0;
        assert!(rect.validate().is_err());
        rect = bounds();
        rect.visible = false;
        rect.width = 0.0;
        assert!(rect.validate().is_ok());
    }
}
