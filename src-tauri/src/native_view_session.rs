//! Retained native viewer; selections are short-lived leases on one machine/session.
use super::*;
use std::sync::atomic::AtomicIsize;

pub(super) struct Session {
    pub id: String,
    selection: Mutex<(Arc<View>, String)>,
    pub stopped: AtomicBool,
    pub window_found: AtomicBool,
    pub hwnd: AtomicIsize,
    pub touched: Mutex<Instant>,
    pub profile: Mutex<Option<PathBuf>>,
}
impl Session {
    pub fn new(view: Arc<View>, pane: String) -> Arc<Self> {
        let session = Arc::new(Self {
            id: uuid::Uuid::new_v4().to_string(),
            selection: Mutex::new((view.clone(), pane)),
            stopped: AtomicBool::new(false),
            window_found: AtomicBool::new(false),
            hwnd: AtomicIsize::new(0),
            touched: Mutex::new(Instant::now()),
            profile: Mutex::new(None),
        });
        *view.session.lock().unwrap() = Arc::downgrade(&session);
        session
    }
    pub fn selected(&self) -> (Arc<View>, String) {
        self.selection.lock().unwrap().clone()
    }
    pub fn select(self: &Arc<Self>, view: Arc<View>, pane: String) -> Result<(), String> {
        self.check()?;
        view.check()?;
        self.hide();
        view.focused.store(false, Ordering::Release);
        *self.selection.lock().map_err(err)? = (view.clone(), pane);
        *view.session.lock().map_err(err)? = Arc::downgrade(self);
        *self.touched.lock().map_err(err)? = Instant::now();
        if let Some(dir) = &*self.profile.lock().map_err(err)? {
            view.font.lock().map_err(err)?.resume(&self.id, dir)?;
        }
        Ok(())
    }
    pub fn check(&self) -> Result<(), String> {
        if self.stopped.load(Ordering::Acquire) {
            Err("terminal.disconnected".into())
        } else {
            Ok(())
        }
    }
    pub fn hide(&self) {
        let hwnd = self.hwnd.load(Ordering::Acquire);
        if hwnd != 0 {
            windows::hide(hwnd);
        }
    }
    pub fn pause(&self, id: &str) {
        let (view, _) = self.selected();
        if view.id != id {
            return;
        }
        self.hide();
        if let Ok(mut font) = view.font.lock() {
            font.finish(&self.id);
        }
        if let Ok(mut touched) = self.touched.lock() {
            *touched = Instant::now();
        }
    }
    pub fn stop(&self, message: &str) {
        if !self.stopped.swap(true, Ordering::AcqRel) {
            self.hide();
            self.selected().0.stop(message);
        }
    }
}

pub(super) fn key(cli: &Cli) -> String {
    let remote = cli
        .remote
        .as_ref()
        .map(|m| serde_json::json!([m.id, m.profile_id, m.ssh_target, m.host_identity]));
    serde_json::json!([cli.executable, cli.session, remote]).to_string()
}
