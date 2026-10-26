//! Best-effort bounded diagnostics: identifiers and elapsed stages, never prompt content.
use serde_json::{Value, json};
use std::{cell::RefCell, io::Write, path::PathBuf, sync::Mutex, time::Instant};

pub(crate) struct Timing {
    start: Instant,
    stages: RefCell<Vec<Value>>,
    operation: String,
    machine: String,
    path: Option<PathBuf>,
}

impl Timing {
    pub fn new(operation: &str, machine: &str, database: Option<PathBuf>) -> Self {
        Self {
            start: Instant::now(),
            stages: RefCell::new(Vec::new()),
            operation: operation.into(),
            machine: machine.into(),
            path: database.map(|p| p.with_file_name("start-timing.jsonl")),
        }
    }
    pub fn stage(&self, stage: &str) {
        self.stages
            .borrow_mut()
            .push(json!({"stage":stage,"elapsed_ms":self.start.elapsed().as_millis()}));
    }
    pub fn finish(&self, succeeded: bool) {
        let row = json!({"at":chrono::Utc::now().to_rfc3339(),"operation_id":self.operation,
            "machine_id":self.machine,"succeeded":succeeded,"elapsed_ms":self.start.elapsed().as_millis(),
            "stages":self.stages.borrow().as_slice()});
        if let Some(path) = &self.path {
            let _ = append(path, &row);
        }
    }
}

fn append(path: &std::path::Path, row: &Value) -> std::io::Result<()> {
    static LOCK: Mutex<()> = Mutex::new(());
    let _lock = LOCK
        .lock()
        .map_err(|_| std::io::Error::other("timing lock"))?;
    let truncate = std::fs::metadata(path).is_ok_and(|m| m.len() > 2 * 1024 * 1024);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(!truncate)
        .truncate(truncate)
        .open(path)?;
    writeln!(file, "{row}")
}
