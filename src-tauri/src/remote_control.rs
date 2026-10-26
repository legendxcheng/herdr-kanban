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
}

/// Kept on the worker thread and dropped on success, error or unwind.
pub(crate) struct Scope(Option<Session>, PhantomData<Rc<()>>);

impl Scope {
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
    fn request(&mut self, machine: &Machine, value: Value) -> Result<Value, String> {
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
        let response = (|| {
            if self.connection.is_none() {
                crate::remote_transport::verify_profile(machine)?;
                self.connection = Some(Connection::spawn(
                    crate::remote_transport::control_command(machine)?,
                )?);
            }
            self.connection.as_mut().unwrap().exchange(
                machine,
                value,
                &AtomicBool::new(false),
                Duration::from_secs(55),
            )
        })();
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
                // It may already have executed. Discard the stream and latch failure
                // until this operation ends; no reconnect, fallback or resend.
                self.failed = true;
                self.connection.take();
                Err(error)
            }
        }
    }
}

#[cfg(all(test, windows))]
#[path = "remote_control_tests.rs"]
mod tests;
