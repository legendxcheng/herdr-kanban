//! A real Herdr client hosted by an independent WezTerm window.
//! No terminal proxy, mouse decoding, SetParent, owner, or global topmost.
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
pub struct View {
    id: String,
    task: String,
    bounds: Mutex<Bounds>,
    stopped: AtomicBool,
    ready: AtomicBool,
    presented: AtomicBool,
    window_found: AtomicBool,
    status: Status,
    open_gate: Arc<Mutex<()>>,
}
impl View {
    fn stop(&self, message: &str) {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
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
}
impl NativeTerminalManager {
    pub fn select(
        &self,
        id: String,
        task: String,
        bounds: Bounds,
        status: Status,
    ) -> Result<Arc<View>, String> {
        bounds.validate()?;
        if id.is_empty() || task.is_empty() {
            return Err("terminal.stale".into());
        }
        let view = Arc::new(View {
            id,
            task,
            bounds: Mutex::new(bounds),
            stopped: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            presented: AtomicBool::new(false),
            window_found: AtomicBool::new(false),
            status,
            open_gate: self.open_gate.clone(),
        });
        if let Some(old) = self.slot.lock().map_err(err)?.replace(view.clone()) {
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
            if slot.as_ref().is_some_and(|v| v.task == task) {
                if let Some(view) = slot.take() {
                    view.stop("terminal.closed");
                }
            }
        }
    }
    pub fn close_all(&self) {
        if let Ok(mut slot) = self.slot.lock() {
            if let Some(view) = slot.take() {
                view.stop("terminal.closed");
            }
        }
    }
}
impl Drop for NativeTerminalManager {
    fn drop(&mut self) {
        self.close_all();
    }
}

// Keep ordinary WezTerm/Herdr settings and native mouse bindings. Only override
// window chrome needed for the independent window to occupy Kanban's slot.
fn client_command(executable: &Path, cli: &Cli, class: &str) -> Command {
    let mut command = Command::new(executable);
    command
        .args([
            "--config",
            "window_decorations=\"NONE\"",
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
        .args(["--session", &cli.session, "client"]);
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
pub fn open(view: Arc<View>, app: Arc<Workflow>, main_hwnd: isize) -> Result<(), String> {
    let result = open_inner(view.clone(), app, main_hwnd);
    if let Err(ref e) = result {
        view.stop(e);
    }
    result
}
fn open_inner(view: Arc<View>, app: Arc<Workflow>, main_hwnd: isize) -> Result<(), String> {
    // Focus is session-wide in Herdr. Serialize overlapping selections so a
    // slow obsolete open cannot focus its task after the newest selection.
    let _gate = view.open_gate.lock().map_err(err)?;
    view.check()?;
    let target = app.target(&view.task, true)?;
    let cli = Cli::from_env();
    if target.session != cli.session {
        return Err("binding.identity_changed".into());
    }
    view.check()?;
    cli.request(&["agent", "focus", &target.pane_id])
        .map_err(|e| e.message)?;
    view.check()?;
    let executable = wezterm_executable()?;
    let class = format!("kanban-terminal-{}", uuid::Uuid::new_v4());
    let config_dir = crate::db::db_path()
        .parent()
        .ok_or("terminal.window_timeout")?
        .join("terminal");
    std::fs::create_dir_all(&config_dir).map_err(err)?;
    let diagnostics = std::fs::File::create(config_dir.join("wezterm.log")).map_err(err)?;
    let child = client_command(&executable, &cli, &class)
        .stderr(diagnostics)
        .spawn()
        .map_err(|e| format!("terminal.wezterm_unavailable: {e}"))?;
    let mut process = windows::Process::new(child)?;
    let pid = process.id();
    let monitor = view.clone();
    thread::spawn(move || {
        let events = windows::WindowEvents::new(main_hwnd);
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut hwnd = 0;
        while !monitor.stopped.load(Ordering::Acquire) {
            if process.exited() {
                monitor.stop("terminal.disconnected");
                break;
            }
            if hwnd == 0 {
                hwnd = windows::find(pid, &class);
                if hwnd != 0 {
                    monitor.window_found.store(true, Ordering::Release);
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
                windows::sync(
                    main_hwnd,
                    hwnd,
                    *monitor.bounds.lock().unwrap(),
                    monitor.ready.load(Ordering::Acquire)
                        && monitor.presented.load(Ordering::Acquire),
                );
            }
            events.wait();
        }
        if hwnd != 0 {
            windows::hide(hwnd);
        }
        // Drop closes this WezTerm and its Herdr CLIENT only, never the server.
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    while !view.window_found.load(Ordering::Acquire) && Instant::now() < deadline {
        view.check()?;
        thread::sleep(Duration::from_millis(10));
    }
    view.check()?;
    if !view.window_found.load(Ordering::Acquire) {
        return Err("terminal.window_timeout".into());
    }
    // This means the native window is open. Herdr owns connection/error UI;
    // there is no invented terminal frame/ready handshake with its client.
    view.ready.store(true, Ordering::Release);
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
    fn launches_official_connect_only_client_without_proxy_or_input_overrides() {
        let cli = Cli {
            executable: "C:/Tools/Herdr/herdr.exe".into(),
            session: "test-session".into(),
        };
        let command = client_command(Path::new("wezterm-gui.exe"), &cli, "test-window");
        let args: Vec<_> = command
            .get_args()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            &args[args.len() - 4..],
            &[
                "C:/Tools/Herdr/herdr.exe",
                "--session",
                "test-session",
                "client"
            ]
        );
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
