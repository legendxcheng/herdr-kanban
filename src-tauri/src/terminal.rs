//! A pane-scoped, writable Herdr terminal session (including native Windows).
//! Uses the upstream JSONL terminal session control protocol, not agent attach.
use crate::{
    herdr::Cli,
    workflow::{Target, Workflow},
};
use base64::Engine;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, serde::Serialize)]
pub struct Output {
    pub kind: &'static str,
    pub data: Vec<u8>,
    pub message: String,
    pub cols: u16,
    pub rows: u16,
}
pub type Sink = Arc<dyn Fn(Output) -> bool + Send + Sync>;

#[derive(serde::Deserialize)]
pub struct ScrollEvent {
    pub column: u16,
    pub row: u16,
    pub cols: u16,
    pub rows: u16,
    pub modifiers: u8,
    pub direction: ScrollDirection,
    pub lines: u16,
}
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollDirection {
    Up,
    Down,
}

struct Attachment {
    task: String,
    target: Target,
    writer: Mutex<ChildStdin>,
    size: Mutex<(u16, u16)>,
    child: Arc<Mutex<Child>>,
    stopped: AtomicBool,
    unacked: AtomicUsize,
    sink: Sink,
}
impl Attachment {
    fn stop(&self, message: &str) {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        // Release only our controller. Never close a pane or signal its process.
        if let Ok(mut writer) = self.writer.try_lock() {
            let _ = writeln!(writer, "{}", json!({"type":"terminal.release"}));
            let _ = writer.flush();
        }
        let child = self.child.clone();
        thread::spawn(move || {
            let until = Instant::now() + Duration::from_millis(500);
            loop {
                if let Ok(mut child) = child.lock() {
                    if child.try_wait().ok().flatten().is_some() {
                        break;
                    }
                    if Instant::now() >= until {
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                } else {
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
        });
        (self.sink)(Output {
            kind: "closed",
            data: vec![],
            message: message.into(),
            cols: 0,
            rows: 0,
        });
    }
    fn command(&self, value: Value) -> Result<(), String> {
        let mut writer = self.writer.lock().map_err(error)?;
        if self.stopped.load(Ordering::SeqCst) {
            return Err("terminal.stale".into());
        }
        writeln!(writer, "{value}").map_err(error)?;
        writer.flush().map_err(error)
    }
}
struct Slot {
    id: String,
    task: String,
    attachment: Option<Arc<Attachment>>,
}
#[derive(Default)]
pub struct TerminalManager {
    slot: Mutex<Option<Slot>>,
}
impl TerminalManager {
    pub fn select(&self, id: &str, task: &str) -> Result<(), String> {
        if id.is_empty() {
            return Err("terminal.stale".into());
        }
        let old = self.slot.lock().map_err(error)?.replace(Slot {
            id: id.into(),
            task: task.into(),
            attachment: None,
        });
        if let Some(old) = old.and_then(|s| s.attachment) {
            old.stop("terminal.switched");
        }
        Ok(())
    }
    pub fn close(&self, id: &str) {
        let old = if let Ok(mut slot) = self.slot.lock() {
            if slot.as_ref().is_some_and(|s| s.id == id) {
                slot.take().and_then(|s| s.attachment)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(old) = old {
            old.stop("terminal.closed");
        }
    }
    pub fn close_all(&self) {
        let old = self
            .slot
            .lock()
            .ok()
            .and_then(|mut s| s.take())
            .and_then(|s| s.attachment);
        if let Some(old) = old {
            old.stop("terminal.closed");
        }
    }
    pub fn close_task(&self, task: &str) {
        let id = self
            .slot
            .lock()
            .ok()
            .and_then(|s| s.as_ref().filter(|s| s.task == task).map(|s| s.id.clone()));
        if let Some(id) = id {
            self.close(&id);
        }
    }
    fn current(&self, id: &str) -> Result<Arc<Attachment>, String> {
        self.slot
            .lock()
            .map_err(error)?
            .as_ref()
            .filter(|s| s.id == id)
            .and_then(|s| s.attachment.clone())
            .filter(|a| !a.stopped.load(Ordering::SeqCst))
            .ok_or_else(|| "terminal.stale".into())
    }
    pub fn open(
        &self,
        app: Arc<Workflow>,
        cli: &Cli,
        task: &str,
        id: &str,
        cols: u16,
        rows: u16,
        sink: Sink,
    ) -> Result<(), String> {
        check_size(cols, rows)?;
        let target = app.target(task, true)?;
        if !self
            .slot
            .lock()
            .map_err(error)?
            .as_ref()
            .is_some_and(|s| s.id == id)
        {
            return Err("terminal.stale".into());
        }
        let mut child = cli
            .command()
            .args([
                "terminal",
                "session",
                "control",
                &target.pane_id,
                "--cols",
                &cols.to_string(),
                "--rows",
                &rows.to_string(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(error)?;
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let writer = child.stdin.take().unwrap();
        let attachment = Arc::new(Attachment {
            task: task.into(),
            target,
            writer: Mutex::new(writer),
            size: Mutex::new((cols, rows)),
            child: Arc::new(Mutex::new(child)),
            stopped: AtomicBool::new(false),
            unacked: AtomicUsize::new(0),
            sink,
        });
        {
            let mut slot = self.slot.lock().map_err(error)?;
            match slot.as_mut().filter(|s| s.id == id) {
                Some(slot) => slot.attachment = Some(attachment.clone()),
                None => {
                    attachment.stop("terminal.switched");
                    return Err("terminal.stale".into());
                }
            }
        }
        let stderr_text = Arc::new(Mutex::new(String::new()));
        let errors = stderr_text.clone();
        thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.take(16384).read_to_string(&mut text);
            if let Ok(mut dst) = errors.lock() {
                *dst = text;
            }
        });
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let output = attachment.clone();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut first = true;
            loop {
                if output.stopped.load(Ordering::SeqCst) {
                    break;
                }
                // Limit a single JSON envelope before allocation can grow without bound.
                let mut line = Vec::new();
                let read = Read::by_ref(&mut reader)
                    .take(8 * 1024 * 1024)
                    .read_until(b'\n', &mut line);
                if !matches!(read,Ok(n) if n>0) {
                    break;
                }
                if line.last() != Some(&b'\n') {
                    output.stop("herdr.invalid_response");
                    break;
                }
                let Ok(frame) = serde_json::from_slice::<Value>(&line) else {
                    output.stop("herdr.invalid_response");
                    break;
                };
                if frame["type"] == "terminal.closed" {
                    let reason = frame["reason"].as_str().unwrap_or("terminal.disconnected");
                    let _ = ready_tx.try_send(Err(reason.to_string()));
                    output.stop(reason);
                    break;
                }
                if frame["type"] != "terminal.frame" {
                    continue;
                }
                let bytes = frame["bytes"]
                    .as_str()
                    .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok());
                let Some(bytes) = bytes else {
                    output.stop("herdr.invalid_response");
                    break;
                };
                if first {
                    let _ = ready_tx.try_send(Ok(()));
                    first = false;
                }
                for (index, chunk) in bytes.chunks(8192).enumerate() {
                    let start = Instant::now();
                    while output.unacked.load(Ordering::SeqCst) >= 128 * 1024 {
                        if output.stopped.load(Ordering::SeqCst) {
                            return;
                        }
                        if start.elapsed() > Duration::from_secs(10) {
                            output.stop("terminal.backpressure");
                            return;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    if output.stopped.load(Ordering::SeqCst) {
                        return;
                    }
                    output.unacked.fetch_add(chunk.len(), Ordering::SeqCst);
                    if !(output.sink)(Output {
                        kind: "data",
                        data: chunk.to_vec(),
                        message: String::new(),
                        // Mark geometry only after the entire frame has been rendered.
                        cols: if (index + 1) * 8192 >= bytes.len() {
                            frame["width"].as_u64().unwrap_or(0) as u16
                        } else {
                            0
                        },
                        rows: if (index + 1) * 8192 >= bytes.len() {
                            frame["height"].as_u64().unwrap_or(0) as u16
                        } else {
                            0
                        },
                    }) {
                        output.stop("terminal.closed");
                        return;
                    }
                }
            }
            let error = stderr_text
                .lock()
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            let reason = if error.is_empty() {
                "terminal.disconnected".into()
            } else {
                format!("herdr.failed: {error}")
            };
            let _ = ready_tx.try_send(Err(reason.clone()));
            output.stop(&reason);
        });
        let watched = attachment.clone();
        thread::spawn(move || {
            while !watched.stopped.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_secs(2));
                if watched.stopped.load(Ordering::SeqCst) {
                    break;
                }
                match app.target(&watched.task, true) {
                    Ok(target) if target.same_identity(&watched.target) => {}
                    Ok(_) => watched.stop("binding.identity_changed"),
                    Err(e) => watched.stop(&e),
                }
            }
        });
        match ready_rx.recv_timeout(Duration::from_secs(8)) {
            Ok(Ok(())) if !attachment.stopped.load(Ordering::SeqCst) => Ok(()),
            Ok(Err(e)) => {
                attachment.stop(&e);
                Err(e)
            }
            _ => {
                attachment.stop("herdr.timeout");
                Err("herdr.timeout".into())
            }
        }
    }
    pub fn input(&self, app: &Workflow, id: &str, data: &str) -> Result<(), String> {
        if data.len() > 65536 {
            return Err("terminal.input_limit".into());
        }
        let a = self.current(id)?;
        let target = app.target(&a.task, true).inspect_err(|e| a.stop(e))?;
        if !target.same_identity(&a.target) {
            a.stop("binding.identity_changed");
            return Err("binding.identity_changed".into());
        }
        self.current(id)?
            .command(json!({"type":"terminal.input","text":data}))
    }
    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), String> {
        check_size(cols, rows)?;
        let a = self.current(id)?;
        let mut size = a.size.lock().map_err(error)?;
        a.command(json!({"type":"terminal.resize","cols":cols,"rows":rows}))?;
        *size = (cols, rows);
        Ok(())
    }
    pub fn scroll(&self, app: &Workflow, id: &str, event: ScrollEvent) -> Result<(), String> {
        check_size(event.cols, event.rows)?;
        if event.column >= event.cols
            || event.row >= event.rows
            || event.modifiers > 7
            || !(1..=24).contains(&event.lines)
        {
            return Err("terminal.scroll_invalid".into());
        }
        let a = self.current(id)?;
        let target = app.target(&a.task, true).inspect_err(|e| a.stop(e))?;
        if !target.same_identity(&a.target) {
            a.stop("binding.identity_changed");
            return Err("binding.identity_changed".into());
        }
        let current = self.current(id)?;
        let size = current.size.lock().map_err(error)?;
        if *size != (event.cols, event.rows) {
            return Err("terminal.scroll_stale_size".into());
        }
        let mut writer = current.writer.lock().map_err(error)?;
        // Herdr ignores `lines` when the application owns the wheel. A batch of
        // one-line commands gives both host history and mouse-aware apps the
        // requested amount, without one expensive identity lookup per line.
        let command = json!({"type":"terminal.scroll", "direction":event.direction,"lines":1,
            "column":event.column,"row":event.row,"modifiers":event.modifiers});
        for _ in 0..event.lines {
            if current.stopped.load(Ordering::SeqCst) {
                return Err("terminal.stale".into());
            }
            writeln!(writer, "{command}").map_err(error)?;
        }
        writer.flush().map_err(error)
    }
    pub fn ack(&self, id: &str, bytes: usize) {
        if let Ok(a) = self.current(id) {
            let _ = a
                .unacked
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                    Some(n.saturating_sub(bytes))
                });
        }
    }
}
impl Drop for TerminalManager {
    fn drop(&mut self) {
        self.close_all();
    }
}
fn check_size(cols: u16, rows: u16) -> Result<(), String> {
    if !(2..=500).contains(&cols) || !(2..=300).contains(&rows) {
        Err("terminal.size".into())
    } else {
        Ok(())
    }
}
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
