//! Real Win32 regression: exercise sync with owned helper windows in the Z order.
use super::*;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

unsafe extern "system" fn record_position(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_WINDOWPOSCHANGING && (*(lp as *const WINDOWPOS)).flags & SWP_NOZORDER == 0 {
            let count = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const AtomicUsize;
            if !count.is_null() {
                (*count).fetch_add(1, Ordering::Relaxed);
            }
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

struct Gui {
    hwnd: HWND,
    helper: HWND,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    reorders: Arc<AtomicUsize>,
}
impl Gui {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let reorders = Arc::new(AtomicUsize::new(0));
        let counter = reorders.clone();
        let thread = thread::spawn(move || unsafe {
            let window = Window::new(std::ptr::null_mut(), true);
            SetWindowLongPtrW(window.0, GWLP_USERDATA, Arc::as_ptr(&counter) as isize);
            SetWindowLongPtrW(
                window.0,
                GWLP_WNDPROC,
                record_position as *const () as isize,
            );
            let helper = Window::new(window.0, false);
            let nested = Window::new(helper.0, false);
            tx.send((window.0 as isize, helper.0 as isize)).unwrap();
            while !stopped.load(Ordering::Acquire) {
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                thread::sleep(Duration::from_millis(2));
            }
            drop(nested);
            drop(helper);
            drop(window);
        });
        let (hwnd, helper) = rx.recv().unwrap();
        Self {
            hwnd: hwnd as HWND,
            helper: helper as HWND,
            stop,
            thread: Some(thread),
            reorders,
        }
    }
}
impl Drop for Gui {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
    }
}

struct Window(HWND);
impl Window {
    unsafe fn new(owner: HWND, visible: bool) -> Self {
        unsafe {
            let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class.as_ptr(),
                class.as_ptr(),
                WS_POPUP,
                40,
                40,
                240,
                180,
                owner,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            assert!(!hwnd.is_null());
            if visible {
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
            Self(hwnd)
        }
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.0);
        }
    }
}
unsafe fn above(top: HWND, bottom: HWND) -> bool {
    unsafe {
        let mut cursor = bottom;
        for _ in 0..4096 {
            cursor = GetWindow(cursor, GW_HWNDPREV);
            if cursor == top {
                return true;
            }
            if cursor.is_null() {
                return false;
            }
        }
        panic!("window traversal did not terminate")
    }
}
unsafe fn activate(hwnd: HWND) {
    unsafe {
        let current = windows_sys::Win32::System::Threading::GetCurrentThreadId();
        let foreground = GetWindowThreadProcessId(GetForegroundWindow(), std::ptr::null_mut());
        let target = GetWindowThreadProcessId(hwnd, std::ptr::null_mut());
        let attach_foreground = current != foreground
            && foreground != 0
            && windows_sys::Win32::System::Threading::AttachThreadInput(current, foreground, 1)
                != 0;
        let attach_target = target != current
            && target != foreground
            && windows_sys::Win32::System::Threading::AttachThreadInput(current, target, 1) != 0;
        let result = SetForegroundWindow(hwnd);
        if attach_target {
            windows_sys::Win32::System::Threading::AttachThreadInput(current, target, 0);
        }
        if attach_foreground {
            windows_sys::Win32::System::Threading::AttachThreadInput(current, foreground, 0);
        }
        assert_ne!(result, 0, "test could not activate its target window");
        thread::sleep(Duration::from_millis(50));
        assert_eq!(GetForegroundWindow(), hwnd);
    }
}
fn bounds() -> Bounds {
    Bounds {
        x: 80.0,
        y: 0.0,
        width: 160.0,
        height: 180.0,
        viewport_width: 240.0,
        viewport_height: 180.0,
        visible: true,
    }
}
unsafe fn settle(host: HWND, terminal: HWND, bounds: Bounds) {
    let mut syncer = WindowSync::default();
    for _ in 0..40 {
        syncer.sync(host as isize, terminal as isize, bounds, true);
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[ignore = "creates real desktop windows; run explicitly with one test thread"]
fn owned_helpers_do_not_trap_viewer_behind_host() {
    unsafe {
        let foreground = GetForegroundWindow();
        let terminal_gui = Gui::new();
        let host_gui = Gui::new();
        let terminal = &terminal_gui.hwnd;
        let host = &host_gui.hwnd;
        let external = Window::new(std::ptr::null_mut(), true);
        let flags = SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE;
        SetWindowPos(*host, external.0, 0, 0, 0, 0, flags);
        SetWindowPos(*terminal, *host, 0, 0, 0, 0, flags);
        SetWindowPos(
            terminal_gui.helper,
            GetWindow(*host, GW_HWNDPREV),
            0,
            0,
            0,
            0,
            flags | SWP_NOOWNERZORDER,
        );
        assert!(above(*host, *terminal));
        assert!(above(host_gui.helper, *host));
        settle(*host, *terminal, bounds());
        assert!(
            above(*terminal, *host),
            "visible terminal remains covered by host with hidden owned helpers"
        );
        SetWindowPos(
            host_gui.helper,
            *terminal,
            0,
            0,
            0,
            0,
            flags | SWP_NOOWNERZORDER,
        );
        thread::sleep(Duration::from_millis(20));
        let reorders = terminal_gui.reorders.load(Ordering::Relaxed);
        let mut syncer = WindowSync::default();
        for _ in 0..30 {
            syncer.sync(*host as isize, *terminal as isize, bounds(), true);
            assert!(
                above(*terminal, *host),
                "subsequent sync put viewer back behind its host"
            );
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            terminal_gui.reorders.load(Ordering::Relaxed),
            reorders,
            "stable visible order must not reorder repeatedly around owned helpers"
        );
        assert!(
            above(external.0, *terminal),
            "viewer overtook an unrelated window"
        );
        assert_eq!(GetForegroundWindow(), foreground, "sync stole focus");
        assert_eq!(
            GetWindowLongPtrW(*terminal, GWL_EXSTYLE) & WS_EX_TOPMOST as isize,
            0
        );
        assert!(GetWindow(*terminal, GW_OWNER).is_null());

        let dialog = Window::new(*host, true);
        SetWindowPos(dialog.0, external.0, 0, 0, 0, 0, flags | SWP_NOOWNERZORDER);
        settle(*host, *terminal, bounds());
        assert!(
            above(dialog.0, *terminal),
            "viewer covered a visible host dialog"
        );
        assert!(above(*terminal, *host));
        drop(dialog);

        let mut hidden = bounds();
        hidden.visible = false;
        settle(*host, *terminal, hidden);
        assert_eq!(IsWindowVisible(*terminal), 0);
        settle(*host, *terminal, bounds());
        assert_ne!(IsWindowVisible(*terminal), 0);
        assert!(above(*terminal, *host));
        ShowWindow(*host, SW_SHOWMINNOACTIVE);
        settle(*host, *terminal, bounds());
        assert_eq!(IsWindowVisible(*terminal), 0);
        ShowWindow(*host, SW_SHOWNOACTIVATE);
        settle(*host, *terminal, bounds());
        assert_ne!(IsWindowVisible(*terminal), 0);
        assert!(above(*terminal, *host));
    }
}

#[test]
#[ignore = "launches an isolated real WezTerm; run explicitly with one test thread"]
fn wezterm_recovers_from_occlusion_with_owned_helpers() {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    let class = format!("kanban-zorder-test-{}", uuid::Uuid::new_v4());
    let mut command = Command::new(super::super::wezterm_executable().unwrap());
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
            &class,
            "--position=-32000,-32000",
            "--",
            "cmd.exe",
            "/k",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x08000000);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("WEZTERM_") {
            command.env_remove(key);
        }
    }
    let process = Process::new(command.spawn().unwrap()).unwrap();
    let mut terminal = 0;
    for _ in 0..100 {
        terminal = find(process.id(), &class);
        if terminal != 0 {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert_ne!(terminal, 0);
    unsafe {
        let terminal = terminal as HWND;
        let host = Gui::new();
        let external = Window::new(std::ptr::null_mut(), true);
        let flags = SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE;
        SetWindowPos(host.hwnd, external.0, 0, 0, 0, 0, flags);
        SetWindowPos(
            terminal,
            host.hwnd,
            120,
            40,
            160,
            180,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        thread::sleep(Duration::from_millis(100));
        let foreground = GetForegroundWindow();
        for _ in 0..10 {
            SetWindowPos(terminal, host.hwnd, 0, 0, 0, 0, flags);
            settle(host.hwnd, terminal, bounds());
            assert!(
                above(terminal, host.hwnd),
                "real WezTerm remains hidden behind host"
            );
            assert!(
                above(external.0, terminal),
                "real WezTerm overtook external window"
            );
            assert_eq!(GetForegroundWindow(), foreground);
        }
        // A background-only SetWindowPos probe misses this Windows restriction:
        // the host now actually owns the foreground, while WezTerm does not.
        let mut passed = true;
        for _ in 0..12 {
            activate(terminal);
            settle(host.hwnd, terminal, bounds());
            passed &= above(terminal, host.hwnd) && GetForegroundWindow() == terminal;
            activate(host.hwnd);
            // A non-activating external window can be between the foreground
            // host and its viewer. Recovery must not lower the entire app.
            SetWindowPos(external.0, host.hwnd, 0, 0, 0, 0, flags);
            settle(host.hwnd, terminal, bounds());
            passed &= above(terminal, host.hwnd)
                && above(host.hwnd, external.0)
                && GetForegroundWindow() == host.hwnd;
            activate(external.0);
            settle(host.hwnd, terminal, bounds());
            passed &= above(external.0, terminal)
                && above(terminal, host.hwnd)
                && GetForegroundWindow() == external.0;
        }
        // A topmost external window still covers both, even with host focus.
        SetWindowPos(external.0, HWND_TOPMOST, 0, 0, 0, 0, flags);
        activate(host.hwnd);
        settle(host.hwnd, terminal, bounds());
        passed &= above(external.0, terminal)
            && above(terminal, host.hwnd)
            && GetForegroundWindow() == host.hwnd
            && GetWindowLongPtrW(terminal, GWL_EXSTYLE) & WS_EX_TOPMOST as isize == 0;
        // Restore the user's previous foreground before reporting a failure.
        activate(foreground);
        assert!(
            passed,
            "foreground cycle failed: viewer covered, external order changed, or keyboard focus stolen"
        );
    }
}
