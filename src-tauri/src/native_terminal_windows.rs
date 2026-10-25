use super::Bounds;
use std::{os::windows::io::AsRawHandle, process::Child};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::*, Gdi::ClientToScreen},
    System::JobObjects::*,
    UI::{Accessibility::*, WindowsAndMessaging::*},
};

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
pub fn sync(main: isize, terminal: isize, bounds: Bounds, ready: bool) {
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
        let sizing = actual.right - actual.left != width || actual.bottom - actual.top != height;
        let showing = visible && IsWindowVisible(terminal) == 0;
        // Keep the pair adjacent in the normal Z order. An unrelated foreground
        // window covers both naturally; losing focus must never hide the viewer.
        let mut preceding = GetWindow(main, GW_HWNDPREV);
        let reorder = preceding != terminal;
        if visible && foreground == terminal && reorder {
            SetWindowPos(
                main,
                terminal,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
            );
        }
        let reorder = visible && reorder && foreground != terminal;
        // Inserting inside the topmost band could accidentally promote WezTerm.
        if preceding.is_null()
            || GetWindowLongPtrW(preceding, GWL_EXSTYLE) & WS_EX_TOPMOST as isize != 0
        {
            preceding = HWND_TOP;
        }
        if moving || sizing || showing || reorder {
            SetWindowPos(
                terminal,
                preceding,
                x,
                y,
                width,
                height,
                SWP_NOACTIVATE
                    | SWP_ASYNCWINDOWPOS
                    | if showing { SWP_SHOWWINDOW } else { 0 }
                    | if moving { 0 } else { SWP_NOMOVE }
                    | if sizing { 0 } else { SWP_NOSIZE }
                    | if reorder { 0 } else { SWP_NOZORDER },
            );
        }
    }
}
