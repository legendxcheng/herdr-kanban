//! One private SSH stream for a synchronous start worker. Never replay a request.
//! A scope covers resources, CLI reads and mutations without sharing a queue with UI polling.
use crate::{machines::Machine, search_connection::Connection};
use serde_json::Value;
use std::{cell::RefCell, marker::PhantomData, rc::Rc, sync::atomic::AtomicBool, time::Duration};

thread_local! {
    static ACTIVE: RefCell<Option<Session>> = const { RefCell::new(None) };
}

#[derive(Default)]
struct Session {
    key: Option<String>,
    connection: Option<Connection>,
    failed: bool,
    recoverable_read: bool,
}

/// Only the submit observer may recover a failed read. A lost write reply remains
/// latched, even if a caller subsequently asks to retry an observation.
pub(crate) fn recover_observation() -> bool {
    ACTIVE.with(|slot| {
        slot.borrow_mut()
            .as_mut()
            .is_none_or(Session::recover_observation)
    })
}

pub(crate) fn transient_observation_error(message: &str) -> bool {
    matches!(
        message,
        "machine.timeout" | "machine.connection_failed" | "herdr.timeout" | "herdr.unavailable"
    )
}

fn is_observation(value: &Value) -> bool {
    if value["method"] != "cli_output" {
        return false;
    }
    let Some(args) = value["args"].as_array() else {
        return false;
    };
    match args.as_slice() {
        [agent, get, pane] => {
            agent == "agent"
                && get == "get"
                && pane
                    .as_str()
                    .is_some_and(|p| !p.is_empty() && !p.starts_with('-'))
        }
        [pane, info, flag, target] => {
            pane == "pane"
                && info == "process-info"
                && flag == "--pane"
                && target
                    .as_str()
                    .is_some_and(|p| !p.is_empty() && !p.starts_with('-'))
        }
        _ => false,
    }
}

/// Kept on the worker thread and dropped on success, error or unwind.
pub(crate) struct Scope(Option<Session>, PhantomData<Rc<()>>);

impl Scope {
    /// Manual dispatch shares the same recovery contract as a start worker,
    /// without replacing an already active operation's connection/failure latch.
    pub fn enter_if_absent() -> Option<Self> {
        ACTIVE
            .with(|slot| slot.borrow().is_none())
            .then(Self::enter)
    }

    pub fn enter() -> Self {
        Self(
            ACTIVE.with(|slot| slot.replace(Some(Session::default()))),
            PhantomData,
        )
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        ACTIVE.with(|slot| {
            slot.replace(self.0.take());
        });
    }
}

pub(crate) fn request(machine: &Machine, value: Value) -> Option<Result<Value, String>> {
    ACTIVE.with(|slot| {
        slot.borrow_mut()
            .as_mut()
            .map(|session| session.request(machine, value))
    })
}

impl Session {
    fn recover_observation(&mut self) -> bool {
        if !self.failed {
            return true;
        }
        if !self.recoverable_read {
            return false;
        }
        self.failed = false;
        self.recoverable_read = false;
        true
    }

    fn request(&mut self, machine: &Machine, value: Value) -> Result<Value, String> {
        self.request_connecting(machine, value, Duration::from_secs(55), |machine| {
            crate::remote_transport::verify_profile(machine)?;
            Connection::spawn(crate::remote_transport::control_command(machine)?)
        })
    }

    fn request_connecting(
        &mut self,
        machine: &Machine,
        value: Value,
        timeout: Duration,
        connect: impl FnOnce(&Machine) -> Result<Connection, String>,
    ) -> Result<Value, String> {
        if machine.disabled {
            return Err("machine.disabled".into());
        }
        let key = serde_json::to_string(machine).map_err(|e| e.to_string())?;
        if self.key.as_ref().is_some_and(|old| old != &key) {
            return Err("machine.identity_changed".into());
        }
        if self.failed {
            return Err("machine.connection_failed".into());
        }
        self.key = Some(key);
        let observation = is_observation(&value);
        #[cfg(test)]
        let test_request = value.clone();
        let response = (|| {
            if self.connection.is_none() {
                self.connection = Some(connect(machine)?);
            }
            self.connection.as_mut().unwrap().exchange(
                machine,
                value,
                &AtomicBool::new(false),
                timeout,
            )
        })();
        #[cfg(test)]
        let response = test_fault::exchange(&test_request, response);
        match response {
            Ok(mut outcome) => {
                // Domain/CLI errors are complete replies. Explicit busy rejection may
                // still be handled by the existing bounded launch loop.
                if let Some(error) = outcome["error"].as_str() {
                    return Err(error.into());
                }
                if !outcome.is_object() || outcome.get("value").is_none() {
                    self.failed = true;
                    self.connection.take();
                    return Err("machine.invalid_response".into());
                }
                Ok(outcome["value"].take())
            }
            Err(error) => {
                // No replay here. Only a specifically failed read can later be
                // unlocked by the bounded observer; writes/protocol errors stay latched.
                self.failed = true;
                self.recoverable_read = observation && transient_observation_error(&error);
                self.connection.take();
                Err(error)
            }
        }
    }
}

/// Opt-in live test fault: discard one real read reply after an acknowledged key.
/// The normal error path then drops that SSH child. This code is absent in app builds.
#[cfg(test)]
pub(crate) mod test_fault {
    use super::*;
    struct Fault {
        key: &'static str,
        armed: bool,
        fired: bool,
    }
    thread_local! { static FAULT: RefCell<Option<Fault>> = const { RefCell::new(None) }; }
    pub struct Guard;
    impl Guard {
        pub fn arm(key: &'static str) -> Self {
            FAULT.with(|f| {
                *f.borrow_mut() = Some(Fault {
                    key,
                    armed: false,
                    fired: false,
                })
            });
            Self
        }
        pub fn fired(&self) -> bool {
            FAULT.with(|f| f.borrow().as_ref().is_some_and(|f| f.fired))
        }
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            FAULT.with(|f| f.borrow_mut().take());
        }
    }
    pub(super) fn exchange(
        request: &Value,
        result: Result<Value, String>,
    ) -> Result<Value, String> {
        FAULT.with(|f| {
            let mut f = f.borrow_mut();
            let Some(f) = f.as_mut() else { return result };
            if result.is_ok() && f.armed && !f.fired && is_observation(request) {
                f.fired = true;
                return Err("machine.connection_failed".into());
            }
            if request["method"] == "cli_output"
                && request["args"][0] == "agent"
                && request["args"][1] == "send-keys"
                && request["args"][3] == f.key
                && result.is_ok()
            {
                f.armed = true;
            }
            result
        })
    }
}

#[cfg(all(test, windows))]
#[path = "remote_control_tests.rs"]
mod tests;
