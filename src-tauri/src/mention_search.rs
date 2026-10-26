//! Complete remote snapshots, single-flight loading, and local query matching.
use crate::{
    file_search::{self, FileEntry},
    machines::Machine,
    search_connection::Connection,
    skill_discovery::{self, SkillEntry},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const TTL: Duration = Duration::from_secs(30);
const CAPACITY: usize = 8;
type Db = Arc<Mutex<rusqlite::Connection>>;
type Cached<T> = Arc<Mutex<Option<(Instant, Result<Arc<T>, String>)>>>;

struct Cache<T> {
    entries: Mutex<HashMap<String, Cached<T>>>,
}
impl<T> Default for Cache<T> {
    fn default() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }
}
impl<T> Cache<T> {
    fn get(
        &self,
        key: String,
        refresh: bool,
        load: impl FnOnce() -> Result<T, String>,
    ) -> Result<Arc<T>, String> {
        let entry = {
            let mut entries = self.entries.lock().map_err(|e| e.to_string())?;
            if refresh {
                entries.remove(&key);
            }
            if !entries.contains_key(&key) && entries.len() >= CAPACITY {
                // Do not evict an in-flight snapshot: concurrent callers must share it.
                let victim = entries
                    .iter()
                    .filter(|(_, entry)| Arc::strong_count(entry) == 1)
                    .filter_map(|(key, entry)| {
                        entry
                            .try_lock()
                            .ok()
                            .map(|value| (key.clone(), value.as_ref().map(|v| v.0)))
                    })
                    .min_by_key(|(_, time)| *time)
                    .map(|(key, _)| key);
                if let Some(victim) = victim {
                    entries.remove(&victim);
                } else {
                    return Err("mention.busy".into());
                }
            }
            entries
                .entry(key)
                .or_insert_with(|| Arc::new(Mutex::new(None)))
                .clone()
        };
        // Only this scope waits for its scan. No global cache or database lock is held.
        let mut value = entry.lock().map_err(|e| e.to_string())?;
        if let Some((time, result)) = &*value {
            let ttl = if result.is_ok() {
                TTL
            } else {
                Duration::from_secs(1)
            };
            if time.elapsed() < ttl {
                return result.clone();
            }
        }
        let result = load().map(Arc::new);
        *value = Some((Instant::now(), result.clone()));
        result
    }
}

#[derive(Default)]
struct Remote {
    connection: Option<Connection>,
    helper: Option<String>,
}

#[derive(Default)]
pub struct Search {
    files: Cache<Vec<FileEntry>>,
    skills: Cache<Vec<SkillEntry>>,
    remotes: Mutex<HashMap<String, Arc<Mutex<Remote>>>>,
    stop: AtomicBool,
}

fn identity(machine: &Machine) -> String {
    json!([
        machine.id,
        machine.profile_id,
        machine.ssh_target,
        machine.session,
        machine.host_identity
    ])
    .to_string()
}

/// Read current ownership for every IPC, even when the result will come from cache.
pub fn target(db: &Db, project: Option<&str>, root: &str) -> Result<Option<Machine>, String> {
    let Some(project) = project else {
        return Ok(None);
    };
    let db = db.lock().map_err(|e| e.to_string())?;
    let (id, current_root): (String, String) = db
        .query_row(
            "SELECT machine_id,root_path FROM projects WHERE id=?1",
            [project],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| "project.missing")?;
    let machine = crate::machines::get(&db, &id)?;
    if machine.disabled {
        return Err("machine.disabled".into());
    }
    if current_root != root {
        return Err("mention.project_changed".into());
    }
    Ok((machine.kind == "ssh").then_some(machine))
}

impl Search {
    pub fn new() -> Arc<Self> {
        let search = Arc::new(Self::default());
        let weak = Arc::downgrade(&search);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(5));
                let Some(search) = weak.upgrade() else {
                    break;
                };
                if search.stop.load(Ordering::Acquire) {
                    break;
                }
                if let Ok(mut remotes) = search.remotes.lock() {
                    remotes.retain(|_, remote| {
                        if Arc::strong_count(remote) != 1 {
                            return true;
                        }
                        let Ok(mut remote) = remote.try_lock() else {
                            return true;
                        };
                        if remote
                            .connection
                            .as_ref()
                            .is_some_and(|c| c.touched.elapsed() < Duration::from_secs(60))
                        {
                            return true;
                        }
                        remote.connection.take();
                        false
                    });
                }
            }
        });
        search
    }

    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(mut remotes) = self.remotes.lock() {
            remotes.clear();
        }
    }

    pub fn validate_target(
        &self,
        db: &Db,
        machine: &Machine,
        target: &crate::workflow::Target,
    ) -> Result<crate::workflow::Target, String> {
        let live: crate::workflow::Target = serde_json::from_value(self.request(
            db,
            machine,
            json!({"method":"validate_target","target":target}),
        )?)
        .map_err(|_| "machine.invalid_response")?;
        if !live.same_identity(target) {
            return Err("binding.identity_changed".into());
        }
        Ok(live)
    }

    fn request(&self, db: &Db, machine: &Machine, request: Value) -> Result<Value, String> {
        if self.stop.load(Ordering::Acquire) {
            return Err("machine.cancelled".into());
        }
        // A large directory scan must not hold up task switching.
        let key = json!([identity(machine), request["method"] == "validate_target"]).to_string();
        let remote = {
            let mut remotes = self.remotes.lock().map_err(|e| e.to_string())?;
            if !remotes.contains_key(&key) && remotes.len() >= CAPACITY {
                let victim = remotes
                    .iter()
                    .find(|(_, r)| Arc::strong_count(r) == 1)
                    .map(|(key, _)| key.clone());
                if let Some(victim) = victim {
                    remotes.remove(&victim);
                } else {
                    return Err("mention.busy".into());
                }
            }
            remotes.entry(key).or_default().clone()
        };
        let mut remote = remote.lock().map_err(|e| e.to_string())?;
        let current = crate::remote_install::ensure_current(db, machine)?;
        if self.stop.load(Ordering::Acquire) {
            return Err("machine.cancelled".into());
        }
        if remote.helper != current.helper_name {
            remote.connection.take();
        }
        if remote.connection.is_none() {
            remote.connection = Some(Connection::connect(&current)?);
            remote.helper = current.helper_name.clone();
        }
        let result = remote
            .connection
            .as_mut()
            .unwrap()
            .request(&current, request, &self.stop);
        // Never replay a failed request silently. A retry establishes a fresh,
        // identity-checked connection; terminal and dispatch connections are unrelated.
        if result.is_err() {
            remote.connection.take();
        }
        result
    }

    pub fn files(
        &self,
        db: &Db,
        project: Option<&str>,
        root: &str,
        query: &str,
        refresh: bool,
    ) -> Result<Vec<FileEntry>, String> {
        let Some(machine) = target(db, project, root)? else {
            return file_search::search(root.into(), query.into());
        };
        let normalized = query.replace('\\', "/");
        let scope = normalized
            .rsplit_once('/')
            .map_or("", |(prefix, _)| &normalized[..prefix.len() + 1]);
        let key = json!([identity(&machine), project, root, scope]).to_string();
        let files = self.files.get(key, refresh, || {
            serde_json::from_value(self.request(
                db,
                &machine,
                json!({"method":"file_index","root":root,"scope":scope}),
            )?)
            .map_err(|_| "machine.invalid_response".to_string())
        })?;
        Ok(file_search::rank(&files, query))
    }

    pub fn skills(
        &self,
        db: &Db,
        project: Option<&str>,
        root: &str,
        query: &str,
        refresh: bool,
    ) -> Result<Vec<SkillEntry>, String> {
        let Some(machine) = target(db, project, root)? else {
            return skill_discovery::search_skills(query.into(), Some(root.into()));
        };
        let key = json!([identity(&machine), project, root]).to_string();
        let skills = self.skills.get(key, refresh, || {
            serde_json::from_value(self.request(
                db,
                &machine,
                json!({"method":"skill_index","root":root}),
            )?)
            .map_err(|_| "machine.invalid_response".to_string())
        })?;
        Ok(skill_discovery::filter(&skills, query))
    }
}

#[cfg(test)]
#[path = "mention_search_tests.rs"]
mod tests;
