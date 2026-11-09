//! Application-owned read channels. Invalidation cancels in-flight and queued readers.
use super::Connection;
use crate::machines::Machine;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

struct Slot {
    identity: String,
    connection: Mutex<Option<Connection>>,
    cancelled: AtomicBool,
}
#[derive(Default)]
struct State {
    slots: HashMap<(String, String), Arc<Slot>>,
    disabled: HashSet<String>,
    stopped: bool,
}
#[derive(Default)]
pub(crate) struct Pool(Mutex<State>);

impl Pool {
    pub fn request(
        &self,
        machine: &Machine,
        lane: &str,
        value: Value,
        connect: impl FnOnce() -> Result<Connection, String>,
    ) -> Result<Value, String> {
        self.request_checked(machine, lane, value, connect, |_| Ok(()))
    }

    pub fn request_checked(
        &self,
        machine: &Machine,
        lane: &str,
        value: Value,
        connect: impl FnOnce() -> Result<Connection, String>,
        validate: impl FnOnce(&Value) -> Result<(), String>,
    ) -> Result<Value, String> {
        let identity = serde_json::to_string(&(
            &machine.profile_id,
            &machine.ssh_target,
            &machine.session,
            &machine.helper_name,
            &machine.host_identity,
        ))
        .map_err(|e| e.to_string())?;
        let (slot, retired) = {
            let mut state = self.0.lock().map_err(|e| e.to_string())?;
            if state.stopped {
                return Err("machine.cancelled".into());
            }
            if machine.disabled || state.disabled.contains(&machine.id) {
                return Err("machine.disabled".into());
            }
            let key = (machine.id.clone(), lane.into());
            let retired = if state
                .slots
                .get(&key)
                .is_some_and(|s| s.identity != identity)
            {
                state
                    .slots
                    .remove(&key)
                    .inspect(|s| s.cancelled.store(true, Ordering::Release))
            } else {
                None
            };
            let slot = state
                .slots
                .entry(key)
                .or_insert_with(|| {
                    Arc::new(Slot {
                        identity,
                        connection: Mutex::new(None),
                        cancelled: AtomicBool::new(false),
                    })
                })
                .clone();
            (slot, retired)
        };
        drop(retired); // Never kill/join a child under the pool lock.
        let mut connection = slot.connection.lock().map_err(|e| e.to_string())?;
        if slot.cancelled.load(Ordering::Acquire) {
            return Err("machine.cancelled".into());
        }
        if connection.is_none() {
            *connection = Some(connect()?);
        }
        if slot.cancelled.load(Ordering::Acquire) {
            connection.take();
            return Err("machine.cancelled".into());
        }
        let result = connection
            .as_mut()
            .unwrap()
            .exchange(machine, value, &slot.cancelled, Duration::from_secs(30))
            .and_then(|reply| {
                validate(&reply)?;
                Ok(reply)
            });
        if slot.cancelled.load(Ordering::Acquire) {
            connection.take();
            return Err("machine.cancelled".into());
        }
        if result.is_err() {
            connection.take();
        }
        result // A lost read is not replayed. The next consumer establishes a new connection.
    }

    pub fn disable_machine(&self, machine: &str) {
        let retired = {
            let mut state = self.0.lock().unwrap();
            state.disabled.insert(machine.into());
            let keys: Vec<_> = state
                .slots
                .keys()
                .filter(|(id, _)| id == machine)
                .cloned()
                .collect();
            keys.into_iter()
                .filter_map(|k| state.slots.remove(&k))
                .inspect(|s| s.cancelled.store(true, Ordering::Release))
                .collect::<Vec<_>>()
        };
        drop(retired);
    }
    pub fn enable_machine(&self, machine: &str) {
        self.0.lock().unwrap().disabled.remove(machine);
    }
    pub fn shutdown(&self) {
        let retired = {
            let mut state = self.0.lock().unwrap();
            state.stopped = true;
            for slot in state.slots.values() {
                slot.cancelled.store(true, Ordering::Release);
            }
            std::mem::take(&mut state.slots)
        };
        drop(retired);
    }
}
