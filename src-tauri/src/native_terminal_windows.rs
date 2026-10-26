use super::Bounds;
use std::{
    os::windows::io::AsRawHandle,
    process::Child,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::*, Gdi::ClientToScreen},
    System::JobObjects::*,
    UI::{Accessibility::*, WindowsAndMessaging::*},
};

#[cfg(test)]
#[path = "native_terminal_windows_tests.rs"]
mod tests;

/// Run on the dedicated monitor thread, including its own message pump. Main
/// window location events wake it during Windows' modal drag/resize loop; it
/// must not depend on WebView IPC or Tauri's main-thread event dispatch.
pub struct WindowEvents(Vec<HWINEVENTHOOK>);
unsafe extern "system" fn window_event(
    _: HWINEVENTHOOK,
    _: u32,
    _: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    // Delivery wakes MsgWaitForMultipleObjectsEx. Read the latest state once
    // after draining events rather than replaying obsolete move coordinates.
}
impl WindowEvents {
    pub fn new(main: isize) -> Self {
        unsafe {
            let mut pid = 0;
            GetWindowThreadProcessId(main as _, &mut pid);
            Self(vec![
                SetWinEventHook(
                    EVENT_OBJECT_LOCATIONCHANGE,
                    EVENT_OBJECT_LOCATIONCHANGE,
                    std::ptr::null_mut(),
                    Some(window_event),
                    pid,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                ),
                SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    std::ptr::null_mut(),
                    Some(window_event),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                ),
            ])
        }
    }
    pub fn wait(&self) {
        unsafe {
            // The timeout also catches bounds IPC, minimize/restore, and hook
            // registration failure. Never block the monitor on a GUI thread.
            MsgWaitForMultipleObjectsEx(0, std::ptr::null(), 16, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
            let mut message = MSG::default();
            for _ in 0..64 {
                if PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) == 0 {
                    break;
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}
impl Drop for WindowEvents {
    fn drop(&mut self) {
        for hook in &self.0 {
            if !hook.is_null() {
                unsafe {
                    UnhookWinEvent(*hook);
                }
            }
        }
    }
}

pub struct Process {
    child: Child,
    job: isize,
}
impl Process {
    pub fn new(mut child: Child) -> Result<Self, String> {
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
                return Err("terminal.process_isolation".into());
            }
            Ok(Self {
                child,
                job: job as isize,
            })
        }
    }
    pub fn id(&self) -> u32 {
        self.child.id()
    }
    pub fn exited(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.job as _);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Search<'a> {
    pid: u32,
    class: &'a str,
    hwnd: isize,
}
unsafe extern "system" fn enumerate(hwnd: HWND, data: LPARAM) -> i32 {
    unsafe {
        let search = &mut *(data as *mut Search<'_>);
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == search.pid {
            let mut class = [0u16; 256];
            let len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
            if len > 0 && String::from_utf16_lossy(&class[..len as usize]) == search.class {
                search.hwnd = hwnd as isize;
                return 0;
            }
        }
        1
    }
}
pub fn find(pid: u32, class: &str) -> isize {
    let mut search = Search {
        pid,
        class,
        hwnd: 0,
    };
    unsafe {
        EnumWindows(Some(enumerate), &mut search as *mut _ as isize);
    }
    search.hwnd
}
pub fn valid(hwnd: isize, expected_pid: u32) -> bool {
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd as _, &mut pid);
        IsWindow(hwnd as _) != 0 && pid == expected_pid
    }
}
pub fn hide(hwnd: isize) {
    unsafe {
        ShowWindowAsync(hwnd as _, SW_HIDE);
    }
}
struct Anchor {
    main: HWND,
    terminal: HWND,
    preceding: HWND,
    found: bool,
    ignore_main_owned: bool,
}
unsafe extern "system" fn stacking_window(hwnd: HWND, data: LPARAM) -> i32 {
    unsafe {
        let anchor = &mut *(data as *mut Anchor);
        if hwnd == anchor.main {
            anchor.found = true;
            return 0;
        }
        // IME/TSF and parked viewers participate in the raw Z order even when
        // hidden. Never insert a viewer after its own owned popup: Windows may
        // move the owner's group as well. Visible host dialogs remain anchors
        // so the viewer stays below them.
        let owner = GetAncestor(hwnd, GA_ROOTOWNER);
        if hwnd == anchor.terminal
            || (IsWindowVisible(hwnd) != 0
                && owner != anchor.terminal
                && (!anchor.ignore_main_owned || owner != anchor.main))
        {
            anchor.preceding = hwnd;
        }
        1
    }
}
fn preceding_window(main: HWND, terminal: HWND, ignore_main_owned: bool) -> Option<HWND> {
    let mut anchor = Anchor {
        main,
        terminal,
        preceding: HWND_TOP,
        found: false,
        ignore_main_owned,
    };
    unsafe {
        EnumWindows(Some(stacking_window), &mut anchor as *mut _ as isize);
    }
    anchor.found.then_some(anchor.preceding)
}

/// Async SetWindowPos only queues a request. Re-read the actual order each
/// monitor tick; allow three attempts 50ms apart, then cool down for one second.
/// Position/size following is independent of Z-order retries.
#[derive(Default)]
pub struct WindowSync {
    attempts: u8,
    retry_at: Option<Instant>,
    foreground: isize,
    last_report: Option<Instant>,
}
impl WindowSync {
    fn reorder_due(&mut self, needed: bool, foreground: HWND) -> bool {
        let now = Instant::now();
        if !needed || self.foreground != foreground as isize {
            self.attempts = 0;
            self.retry_at = None;
        }
        self.foreground = foreground as isize;
        if !needed || self.retry_at.is_some_and(|at| now < at) {
            return false;
        }
        self.attempts += 1;
        let delay = if self.attempts >= 3 {
            self.attempts = 0;
            self.report("stacking has not converged; retrying after cooldown");
            Duration::from_secs(1)
        } else {
            Duration::from_millis(50)
        };
        self.retry_at = Some(now + delay);
        true
    }
    fn report(&mut self, message: &str) {
        let now = Instant::now();
        if self
            .last_report
            .is_none_or(|at| now.duration_since(at) >= Duration::from_secs(5))
        {
            eprintln!("Kanban native window: {message}");
            self.last_report = Some(now);
        }
    }
    fn check_position(&mut self, result: i32) {
        if result == 0 {
            self.report(&format!(
                "SetWindowPos failed: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    pub fn sync(&mut self, main: isize, terminal: isize, bounds: Bounds, ready: bool) {
        unsafe {
            let main = main as HWND;
            let terminal = terminal as HWND;
            let foreground = GetForegroundWindow();
            let mut cloaked = 0u32;
            DwmGetWindowAttribute(main, DWMWA_CLOAKED as _, &mut cloaked as *mut _ as _, 4);
            let visible = ready
                && bounds.visible
                && bounds.width >= 2.0
                && bounds.height >= 2.0
                && IsWindowVisible(main) != 0
                && IsIconic(main) == 0
                && cloaked == 0;
            if !visible {
                self.reorder_due(false, foreground);
                if IsWindowVisible(terminal) != 0 {
                    ShowWindowAsync(terminal, SW_HIDE);
                }
            }
            // Keep following behind other applications and while a React overlay
            // is open. Minimized windows have meaningless offscreen coordinates.
            if !ready || IsIconic(main) != 0 || bounds.width < 2.0 || bounds.height < 2.0 {
                return;
            }
            let mut client = RECT::default();
            let mut origin = POINT::default();
            if GetClientRect(main, &mut client) == 0 || ClientToScreen(main, &mut origin) == 0 {
                return;
            }
            let sx = f64::from(client.right) / bounds.viewport_width;
            let sy = f64::from(client.bottom) / bounds.viewport_height;
            let x = origin.x + (bounds.x * sx).round() as i32;
            let y = origin.y + (bounds.y * sy).round() as i32;
            let width = (bounds.width * sx).round() as i32;
            let height = (bounds.height * sy).round() as i32;
            let mut actual = RECT::default();
            if GetWindowRect(terminal, &mut actual) == 0 {
                return;
            }
            let moving = actual.left != x || actual.top != y;
            let sizing =
                actual.right - actual.left != width || actual.bottom - actual.top != height;
            let showing = visible && IsWindowVisible(terminal) == 0;
            // Keep the pair adjacent in the normal Z order. An unrelated foreground
            // window covers both naturally; losing focus must never hide the viewer.
            let Some(mut preceding) = preceding_window(main, terminal, false) else {
                return;
            };
            let reorder = self.reorder_due(visible && preceding != terminal, foreground);
            let terminal_active = foreground == terminal
                || (!foreground.is_null() && GetAncestor(foreground, GA_ROOTOWNER) == terminal);
            // Windows can accept a background viewer's HWND_TOP request without
            // putting it above the foreground host. First bring the viewer just
            // below the host, then on a later observed tick lower the host below
            // it. Do not lower the host until adjacency is confirmed: otherwise
            // an intervening application would cover the whole Kanban window.
            // Ignore both windows' owned helpers when checking this adjacency.
            let main_active = foreground == main;
            let lower_main = reorder
                && (terminal_active
                    || (main_active && preceding_window(terminal, main, true) == Some(main)));
            if lower_main {
                self.check_position(SetWindowPos(
                    main,
                    terminal,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE
                        | SWP_NOSIZE
                        | SWP_NOACTIVATE
                        | SWP_ASYNCWINDOWPOS
                        | SWP_NOOWNERZORDER,
                ));
            }
            let reorder = reorder && !lower_main;
            if main_active && reorder {
                preceding = main;
            }
            // Inserting inside the topmost band could accidentally promote WezTerm.
            if preceding.is_null()
                || GetWindowLongPtrW(preceding, GWL_EXSTYLE) & WS_EX_TOPMOST as isize != 0
            {
                preceding = HWND_TOP;
            }
            if moving || sizing || showing || reorder {
                self.check_position(SetWindowPos(
                    terminal,
                    preceding,
                    x,
                    y,
                    width,
                    height,
                    SWP_NOACTIVATE
                        | SWP_ASYNCWINDOWPOS
                        | SWP_NOOWNERZORDER
                        | if showing { SWP_SHOWWINDOW } else { 0 }
                        | if moving { 0 } else { SWP_NOMOVE }
                        | if sizing { 0 } else { SWP_NOSIZE }
                        | if reorder { 0 } else { SWP_NOZORDER },
                ));
            }
        }
    }
}
