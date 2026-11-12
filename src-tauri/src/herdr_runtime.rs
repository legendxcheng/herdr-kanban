use super::*;
use crate::runtime_processes::Runtime;
impl Cli {
    pub(super) fn runtime_capture(&self, target: &Target) -> Result<Runtime, String> {
        if let Some(machine) = &self.remote {
            let mut runtime: Runtime = crate::remote_transport::request(
                machine,
                serde_json::json!({"method":"runtime_capture","target":target}),
            )?;
            runtime.target.machine_id = machine.id.clone();
            return Ok(runtime);
        }
        self.validate(&target.workspace_id, target)?;
        let runtime = crate::runtime_processes::capture(target)?;
        self.validate(&target.workspace_id, target)?;
        Ok(runtime)
    }
    pub(super) fn runtime_gone(&self, runtime: &Runtime) -> Result<bool, String> {
        if let Some(machine) = &self.remote {
            return crate::remote_transport::request(
                machine,
                serde_json::json!({"method":"runtime_absent","runtime":runtime}),
            );
        }
        Ok(self.cleanup_target_absent(&runtime.target)?
            && crate::runtime_processes::absent(runtime)?)
    }
    pub(super) fn runtime_close(
        &self,
        runtime: &Runtime,
        check: &dyn Fn() -> Result<(), String>,
    ) -> Result<(), SendError> {
        let certain = |message| SendError {
            message,
            uncertain: false,
        };
        check().map_err(certain)?;
        if let Some(machine) = &self.remote {
            let reply: super::idle_cleanup::CloseReply = crate::remote_transport::request(
                machine,
                serde_json::json!({"method":"runtime_close","runtime":runtime}),
            )
            .map_err(|message| SendError {
                message,
                uncertain: true,
            })?;
            return match reply.error {
                None => Ok(()),
                Some(message) => Err(SendError {
                    message,
                    uncertain: reply.uncertain,
                }),
            };
        }
        self.validate(&runtime.target.workspace_id, &runtime.target)
            .map_err(certain)?;
        let current = crate::runtime_processes::capture(&runtime.target).map_err(certain)?;
        let root = runtime
            .target
            .process_id
            .split(':')
            .next()
            .unwrap_or("")
            .parse::<u32>()
            .map_err(|_| certain("binding.identity_unknown".into()))?;
        if !runtime.processes.iter().any(|old| {
            old.pid == root
                && current
                    .processes
                    .iter()
                    .any(|new| new.pid == root && new.birth == old.birth)
        }) {
            return Err(certain("binding.identity_changed".into()));
        }
        check().map_err(certain)?;
        self.request(&["pane", "close", &runtime.target.pane_id])
            .map(|_| ())
    }
}
