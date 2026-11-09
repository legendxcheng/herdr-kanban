//! Reusable read-only SSH lanes. Writes remain on their operation-owned transport.
use crate::{machines::Machine, search_connection::Connection};
use serde_json::Value;
use std::sync::OnceLock;

fn lane(request: &Value) -> Option<&'static str> {
    match request["method"].as_str()? {
        "workspaces" | "hello" | "directory_exists" | "same_directory" => Some("inventory"),
        "agent_status_targets" | "agent_status_batch" => Some("status"),
        "git_statuses" => Some("git"),
        "view_identity" => Some("view"),
        _ => None,
    }
}

#[derive(Default)]
struct Pool(crate::search_connection::pool::Pool);
fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(Pool::default)
}
pub(crate) fn request(machine: &Machine, value: Value) -> Option<Result<Value, String>> {
    lane(&value).map(|name| {
        pool().request(machine, name, value, || {
            crate::remote_transport::verify_profile(machine)?;
            Connection::spawn(crate::remote_transport::control_command(machine)?)
        })
    })
}

impl Pool {
    fn request(
        &self,
        machine: &Machine,
        lane: &str,
        value: Value,
        connect: impl FnOnce() -> Result<Connection, String>,
    ) -> Result<Value, String> {
        let mut reply = self
            .0
            .request_checked(machine, lane, value, connect, |reply| {
                if !reply.is_object() || reply.get("value").is_none() {
                    return Err("machine.invalid_response".into());
                }
                Ok(())
            })?;
        if let Some(error) = reply["error"].as_str() {
            return Err(error.into());
        }
        Ok(reply["value"].take())
    }
}
pub(crate) fn disable_machine(machine: &str) {
    pool().0.disable_machine(machine);
}
pub(crate) fn enable_machine(machine: &str) {
    pool().0.enable_machine(machine);
}
pub(crate) fn shutdown() {
    pool().0.shutdown();
}

#[cfg(all(test, windows))]
#[path = "remote_reads_tests.rs"]
mod tests;
