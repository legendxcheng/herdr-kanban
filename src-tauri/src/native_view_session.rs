//! Retained native viewer; selections are short-lived leases on one machine/session.
use super::*;
use std::sync::atomic::AtomicIsize;

pub(super) struct Session {
    pub id: String,
    pub machine: Option<String>,
    selection: Mutex<(Arc<View>, String)>,
    pub stopped: AtomicBool,
    pub window_found: AtomicBool,
    pub hwnd: AtomicIsize,
    pub touched: Mutex<Instant>,
    pub profile: Mutex<Option<PathBuf>>,
}
impl Session {
    pub fn new(view: Arc<View>, pane: String) -> Arc<Self> {
        Self::create(view, pane, None)
    }
    pub fn for_client(view: Arc<View>, pane: String, cli: &Cli) -> Arc<Self> {
        Self::create(view, pane, cli.remote.as_ref().map(|m| m.id.clone()))
    }
    fn create(view: Arc<View>, pane: String, machine: Option<String>) -> Arc<Self> {
        let session = Arc::new(Self {
            id: uuid::Uuid::new_v4().to_string(),
            machine,
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
    pub fn expire_if_idle(&self) {
        if self.selected().0.check().is_ok() {
            *self.touched.lock().unwrap() = Instant::now();
        } else if self.machine.is_none()
            && self.touched.lock().unwrap().elapsed() > Duration::from_secs(60)
        {
            self.stop("terminal.closed");
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
            record(
                message.split(':').next().unwrap_or("closed"),
                &self.selected().0.id,
                self.machine.as_deref(),
                Some(&self.id),
                Duration::ZERO,
            );
            self.hide();
            self.selected().0.stop(message);
        }
    }
}

/// Connection phases only: never persist terminal input, targets, or credentials.
pub(super) fn record(
    event: &str,
    selection: &str,
    machine: Option<&str>,
    connection: Option<&str>,
    elapsed: Duration,
) {
    use std::io::Write;
    static WRITE: Mutex<()> = Mutex::new(());
    let Ok(_lock) = WRITE.lock() else {
        return;
    };
    let Some(root) = crate::db::db_path().parent().map(Path::to_path_buf) else {
        return;
    };
    let root = root.join("terminal");
    if std::fs::create_dir_all(&root).is_err() {
        return;
    }
    let path = root.join("connection-events.jsonl");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 2 * 1024 * 1024) {
        let _ = std::fs::remove_file(root.join("connection-events.previous.jsonl"));
        let _ = std::fs::rename(&path, root.join("connection-events.previous.jsonl"));
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let value = serde_json::json!({"at":chrono::Utc::now().to_rfc3339(),"event":event,"selection":selection,"machine_id":machine,"connection_id":connection,"elapsed_ms":elapsed.as_millis()});
        let _ = writeln!(file, "{value}");
    }
}

pub(super) fn key(cli: &Cli) -> String {
    let remote = cli.remote.as_ref().map(|m| {
        serde_json::json!([
            m.id,
            m.profile_id,
            m.ssh_target,
            m.host_identity,
            m.helper_name
        ])
    });
    serde_json::json!([cli.executable, cli.session, remote]).to_string()
}
