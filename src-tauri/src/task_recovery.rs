//! Application-owned, bounded reconciliation. Never replays an external write.
use crate::workflow::Workflow;
use rusqlite::Connection;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const DELAYS: [u64; 5] = [2_000, 5_000, 10_000, 30_000, 60_000];
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Manual decisions and automatic probes serialize per database/task, never under SQLite.
pub(crate) fn gate(app: &Workflow, task: &str) -> Result<Arc<Mutex<()>>, String> {
    type Gates = HashMap<(usize, String), Weak<Mutex<()>>>;
    static GATES: OnceLock<Mutex<Gates>> = OnceLock::new();
    let mut gates = GATES.get_or_init(Default::default).lock().map_err(err)?;
    gates.retain(|_, value| value.strong_count() > 0);
    let key = (Arc::as_ptr(&app.db) as usize, task.to_owned());
    if let Some(gate) = gates.get(&key).and_then(Weak::upgrade) {
        return Ok(gate);
    }
    let gate = Arc::new(Mutex::new(()));
    gates.insert(key, Arc::downgrade(&gate));
    Ok(gate)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Candidate {
    task: String,
    project: String,
    machine: String,
    epoch: i64,
    operation: String,
    lifecycle: bool,
    target: Option<String>,
}
impl Candidate {
    fn read(db: &Connection, task: &str) -> Result<Option<Self>, String> {
        use rusqlite::OptionalExtension;
        let owner: Option<(String, String, i64)> = db.query_row(
            "SELECT t.project_id,p.machine_id,t.execution_epoch FROM tasks t
             JOIN projects p ON p.id=t.project_id JOIN machines m ON m.id=p.machine_id
             WHERE t.id=?1 AND t.archived_at IS NULL AND m.disabled=0
             AND p.workspace_id IS NOT NULL AND p.sync_status!='missing'
             AND NOT EXISTS(SELECT 1 FROM lifecycle_operations o WHERE o.task_id=t.id
                AND (o.state='pending' OR (o.kind='complete' AND o.state='unknown')))
             AND NOT EXISTS(SELECT 1 FROM dispatch_operations d WHERE d.task_id=t.id AND d.state='pending')",
            [task], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).optional().map_err(err)?;
        let Some((project, machine, epoch)) = owner else {
            return Ok(None);
        };
        let flow = crate::lifecycle::view(db, task)?;
        let binding = crate::workflow::binding(db, task)?;
        let (operation, lifecycle) =
            if flow.state.as_deref() == Some("unknown") && flow.kind.as_deref() == Some("start") {
                (
                    flow.operation_id.ok_or("dispatch.operation_conflict")?,
                    true,
                )
            } else if let Some(op) = binding.operation.filter(|op| op.state == "unknown") {
                (op.id, false)
            } else {
                return Ok(None);
            };
        let target = binding
            .target
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(err)?;
        Ok(Some(Self {
            task: task.into(),
            project,
            machine,
            epoch,
            operation,
            lifecycle,
            target,
        }))
    }
}

impl Workflow {
    #[cfg(test)]
    pub(crate) fn recover_pending(&self, task: &str, operation: &str) -> Result<(), String> {
        self.recover_pending_checked(task, operation, &|| Ok(()))
    }
    pub(crate) fn recover_pending_checked(
        &self,
        task: &str,
        operation: &str,
        check_cancelled: &dyn Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        let gate = gate(self, task)?;
        let _guard = gate.lock().map_err(err)?;
        check_cancelled()?;
        let candidate = Candidate::read(&*self.db.lock().map_err(err)?, task)?
            .ok_or("dispatch.operation_conflict")?;
        if candidate.operation != operation || candidate.machine != self.herdr.machine_id() {
            return Err("dispatch.operation_conflict".into());
        }
        if candidate.lifecycle {
            self.recover_lifecycle_locked(task, operation, check_cancelled)
        } else {
            let op = self
                .binding(task)?
                .operation
                .ok_or("dispatch.operation_conflict")?;
            self.confirm_received_dispatch(&op, check_cancelled)
        }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Status {
    pub task_id: String,
    pub operation_id: String,
    pub state: &'static str,
    pub attempt: usize,
    pub error: String,
}
struct Batch {
    candidate: Candidate,
    attempt: usize,
    due: u64,
    running: bool,
    error: String,
    generation: u64,
}
#[derive(Default)]
struct Queue {
    batches: HashMap<String, Batch>,
    offline: HashSet<String>,
    generation: u64,
    running_local: usize,
    running_remote: usize,
    health_due: HashMap<String, u64>,
    probing: HashSet<String>,
}
#[derive(Clone)]
pub struct Changed {
    pub project: String,
    pub task: String,
    pub operation: String,
}

pub struct Recovery {
    app: Arc<Workflow>,
    queue: Mutex<Queue>,
    stopped: AtomicBool,
    clock: AtomicU64,
    changed: Arc<dyn Fn(Changed) + Send + Sync>,
}
impl Recovery {
    pub fn new(app: Arc<Workflow>, changed: Arc<dyn Fn(Changed) + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            app,
            queue: Mutex::new(Queue::default()),
            stopped: AtomicBool::new(false),
            clock: AtomicU64::new(0),
            changed,
        })
    }
    pub fn start(app: Arc<Workflow>, changed: Arc<dyn Fn(Changed) + Send + Sync>) -> Arc<Self> {
        let recovery = Self::new(app, changed);
        let weak = Arc::downgrade(&recovery);
        thread::spawn(move || {
            let start = Instant::now();
            while let Some(recovery) = weak.upgrade() {
                if recovery.stopped.load(Ordering::Acquire) {
                    break;
                }
                let _ = recovery.poll(start.elapsed().as_millis() as u64);
                drop(recovery);
                thread::sleep(Duration::from_millis(500));
            }
        });
        recovery
    }
    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
    }

    /// Production and deterministic tests enter through the same complete workflow.
    pub fn poll(self: &Arc<Self>, now: u64) -> Result<Vec<JoinHandle<()>>, String> {
        self.clock.store(now, Ordering::Release);
        if self.stopped.load(Ordering::Acquire) {
            return Ok(vec![]);
        }
        let candidates = {
            let db = self.app.db.lock().map_err(err)?;
            let mut statement = db.prepare("SELECT t.id FROM tasks t WHERE t.archived_at IS NULL
                AND (EXISTS(SELECT 1 FROM lifecycle_operations o WHERE o.task_id=t.id AND o.state='unknown' AND o.kind='start')
                  OR EXISTS(SELECT 1 FROM dispatch_operations d WHERE d.task_id=t.id AND d.state='unknown'))
                ORDER BY t.created_at,t.id").map_err(err)?;
            let ids = statement
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(err)?;
            ids.iter()
                .map(|id| Candidate::read(&db, id))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
        };
        let mut queue = self.queue.lock().map_err(err)?;
        queue.batches.retain(|task, batch| {
            candidates
                .iter()
                .any(|c| &c.task == task && c == &batch.candidate)
        });
        for candidate in candidates {
            if !queue.batches.contains_key(&candidate.task) {
                queue.generation += 1;
                let generation = queue.generation;
                queue.batches.insert(
                    candidate.task.clone(),
                    Batch {
                        candidate,
                        attempt: 0,
                        due: now,
                        running: false,
                        error: String::new(),
                        generation,
                    },
                );
            }
        }
        let machines: HashSet<_> = queue
            .batches
            .values()
            .map(|b| b.candidate.machine.clone())
            .collect();
        queue.offline.retain(|machine| machines.contains(machine));
        queue
            .health_due
            .retain(|machine, _| machines.contains(machine));
        let mut local = queue.running_local;
        let mut remote = queue.running_remote;
        let mut ready: Vec<_> = queue
            .batches
            .values_mut()
            .filter(|b| !b.running && b.attempt < 6 && b.due <= now)
            .collect();
        ready.sort_by_key(|b| (b.due, b.candidate.task.clone()));
        let mut jobs = vec![];
        for batch in ready {
            let (used, limit) = if batch.candidate.machine == "local" {
                (&mut local, 2)
            } else {
                (&mut remote, 3)
            };
            if *used >= limit {
                continue;
            }
            *used += 1;
            batch.running = true;
            batch.attempt += 1;
            jobs.push((batch.candidate.clone(), batch.generation));
        }
        // Independent machine health is intentionally separate from receipt attempts.
        // An off-page machine can become available after its last receipt attempt.
        let mut health_jobs = vec![];
        let offline: Vec<_> = queue.offline.iter().cloned().collect();
        for machine in offline {
            if queue.probing.contains(&machine)
                || queue.health_due.get(&machine).is_none_or(|due| *due > now)
            {
                continue;
            }
            let (used, limit) = if machine == "local" {
                (&mut local, 2)
            } else {
                (&mut remote, 3)
            };
            if *used >= limit {
                continue;
            }
            if queue
                .batches
                .values()
                .any(|batch| batch.candidate.machine == machine)
            {
                *used += 1;
                queue.probing.insert(machine.clone());
                health_jobs.push(machine);
            }
        }
        queue.running_local = local;
        queue.running_remote = remote;
        drop(queue);
        let mut workers: Vec<_> = jobs
            .into_iter()
            .map(|(candidate, generation)| {
                let recovery = self.clone();
                thread::spawn(move || recovery.run(candidate, generation))
            })
            .collect();
        workers.extend(health_jobs.into_iter().map(|machine| {
            let recovery = self.clone();
            thread::spawn(move || recovery.probe_machine(machine))
        }));
        Ok(workers)
    }
    fn run(&self, candidate: Candidate, generation: u64) {
        let result = (|| {
            if self.stopped.load(Ordering::Acquire) {
                return Err("recovery.stopped".into());
            }
            let routed = self.app.for_task(&candidate.task)?;
            if Candidate::read(&*routed.db.lock().map_err(err)?, &candidate.task)?.as_ref()
                != Some(&candidate)
            {
                return Err("dispatch.operation_conflict".into());
            }
            if self.stopped.load(Ordering::Acquire) {
                return Err("recovery.stopped".into());
            }
            routed.recover_pending_checked(&candidate.task, &candidate.operation, &|| {
                if self.stopped.load(Ordering::Acquire) {
                    Err("recovery.stopped".into())
                } else {
                    Ok(())
                }
            })
        })();
        let code = result
            .as_ref()
            .err()
            .map(|e: &String| stable_code(e))
            .unwrap_or_default();
        if let Ok(mut queue) = self.queue.lock() {
            if candidate.machine == "local" {
                queue.running_local -= 1;
            } else {
                queue.running_remote -= 1;
            }
            if crate::remote_control::transient_observation_error(&code) {
                queue.offline.insert(candidate.machine.clone());
                queue
                    .health_due
                    .entry(candidate.machine.clone())
                    .or_insert(self.clock.load(Ordering::Acquire) + 60_000);
            }
            if let Some(batch) = queue
                .batches
                .get_mut(&candidate.task)
                .filter(|b| b.generation == generation && b.candidate == candidate)
            {
                batch.running = false;
                batch.error = code.clone();
                batch.due = self.clock.load(Ordering::Acquire)
                    + DELAYS.get(batch.attempt - 1).copied().unwrap_or(0);
                if result.is_ok() || permanent(&code) {
                    batch.attempt = 6;
                }
            }
        }
        if let Some(path) = self.app.db.lock().ok().and_then(|db| {
            db.path()
                .filter(|p| !p.is_empty())
                .map(std::path::PathBuf::from)
        }) {
            let _ = crate::start_timing::append(
                &path.with_file_name("task-recovery.jsonl"),
                &serde_json::json!({
                    "at": chrono::Utc::now().to_rfc3339(), "task_id": candidate.task,
                    "operation_id": candidate.operation, "machine_id": candidate.machine,
                    "outcome": if result.is_ok() { "reconciled" } else { "unconfirmed" }, "code": code,
                }),
            );
        }
        if !self.stopped.load(Ordering::Acquire) {
            (self.changed)(Changed {
                project: candidate.project,
                task: candidate.task,
                operation: candidate.operation,
            });
        }
    }
    pub fn snapshot(&self) -> Vec<Status> {
        self.queue
            .lock()
            .map(|queue| {
                queue
                    .batches
                    .values()
                    .map(|batch| Status {
                        task_id: batch.candidate.task.clone(),
                        operation_id: batch.candidate.operation.clone(),
                        state: if batch.attempt < 6 || batch.running {
                            "recovering"
                        } else {
                            "needs_attention"
                        },
                        attempt: batch.attempt,
                        error: batch.error.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    fn probe_machine(&self, machine: String) {
        let result = (|| {
            if self.stopped.load(Ordering::Acquire) {
                return Err("recovery.stopped".into());
            }
            {
                let db = self.app.db.lock().map_err(err)?;
                crate::machines::ensure_enabled(&db, &machine)?;
                let mut query = db.prepare("SELECT t.id FROM tasks t JOIN projects p ON p.id=t.project_id WHERE p.machine_id=?1 AND t.archived_at IS NULL").map_err(err)?;
                let ids = query
                    .query_map([&machine], |r| r.get::<_, String>(0))
                    .map_err(err)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(err)?;
                if !ids
                    .iter()
                    .any(|id| Candidate::read(&db, id).ok().flatten().is_some())
                {
                    return Err("recovery.no_pending".into());
                }
            }
            let adapter = crate::machine_routing::adapter(&self.app.db, &self.app.herdr, &machine)?;
            if self.stopped.load(Ordering::Acquire) {
                return Err("recovery.stopped".into());
            }
            adapter.workspaces().map(|_| ())
        })();
        if result.is_ok() && !self.stopped.load(Ordering::Acquire) {
            self.machine_available(&machine);
        }
        if let Ok(mut queue) = self.queue.lock() {
            if machine == "local" {
                queue.running_local -= 1;
            } else {
                queue.running_remote -= 1;
            }
            queue.probing.remove(&machine);
            if result.as_ref().err().is_some_and(|error| {
                matches!(
                    stable_code(error).as_str(),
                    "machine.disabled" | "machine.identity_changed" | "machine.profile_missing"
                )
            }) {
                queue.offline.remove(&machine);
            }
            if queue.offline.contains(&machine) {
                queue
                    .health_due
                    .insert(machine, self.clock.load(Ordering::Acquire) + 60_000);
            } else {
                queue.health_due.remove(&machine);
            }
        }
    }
    pub fn retry(&self, task: &str, operation: &str) -> Result<(), String> {
        let candidate = Candidate::read(&*self.app.db.lock().map_err(err)?, task)?
            .ok_or("dispatch.operation_conflict")?;
        if candidate.operation != operation {
            return Err("dispatch.operation_conflict".into());
        }
        let mut queue = self.queue.lock().map_err(err)?;
        if queue.batches.get(task).is_some_and(|b| b.running) {
            return Ok(());
        }
        queue.generation += 1;
        let generation = queue.generation;
        queue.batches.insert(
            task.into(),
            Batch {
                candidate,
                attempt: 0,
                due: self.clock.load(Ordering::Acquire),
                running: false,
                error: String::new(),
                generation,
            },
        );
        Ok(())
    }
    /// A successful independent service observation after a transport failure is a new trigger.
    pub fn machine_available(&self, machine: &str) {
        if let Ok(mut queue) = self.queue.lock() {
            if !queue.offline.remove(machine) {
                return;
            }
            queue.health_due.remove(machine);
            for batch in queue
                .batches
                .values_mut()
                .filter(|b| b.candidate.machine == machine && !b.running)
            {
                batch.attempt = 0;
                batch.due = self.clock.load(Ordering::Acquire);
                batch.error.clear();
            }
        }
    }
}
fn permanent(code: &str) -> bool {
    matches!(
        code,
        "binding.identity_changed"
            | "binding.occupied"
            | "flow.session_mismatch"
            | "machine.identity_changed"
            | "machine.disabled"
            | "reset.stale"
            | "task.missing"
            | "task.archived"
            | "dispatch.operation_conflict"
    )
}
fn stable_code(error: &str) -> String {
    let code = error.split(':').next().unwrap_or("");
    if code.len() <= 80
        && code.contains('.')
        && code
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b"._".contains(&b))
    {
        code.into()
    } else {
        "recovery.failed".into()
    }
}
