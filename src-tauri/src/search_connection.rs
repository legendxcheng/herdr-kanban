//! One SSH child, multiple discovery requests. No terminal pane or prompt dispatch.
use crate::{machines::Machine, search_wire};
use serde_json::{Value, json};
use std::{
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

type Exchange = (Value, mpsc::Sender<Result<Value, String>>);
pub struct Connection {
    child: Child,
    requests: Option<mpsc::SyncSender<Exchange>>,
    worker: Option<thread::JoinHandle<()>>,
    sequence: u64,
    pub touched: Instant,
}

impl Connection {
    pub fn connect(machine: &Machine) -> Result<Self, String> {
        crate::remote_transport::verify_profile(machine)?;
        let helper = machine
            .helper_name
            .as_deref()
            .ok_or("machine.setup_required")?;
        if helper.len() != 32 || !helper.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("machine.invalid_response".into());
        }
        let mut command = crate::remote_transport::ssh(
            machine
                .ssh_target
                .as_deref()
                .ok_or("machine.invalid_target")?,
        )?;
        // Binary handles pass directly through cmd, avoiding PowerShell transcoding.
        command.args(["cmd", "/d", "/s", "/c", &format!(
            "\"\"%USERPROFILE%\\.herdr-kanban\\helpers\\{helper}.exe\" --remote-helper --search-stream-v1\""
        )]);
        Self::spawn(command)
    }

    pub(crate) fn spawn(mut command: Command) -> Result<Self, String> {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let mut input = child.stdin.take().ok_or("machine.io")?;
        let mut output = child.stdout.take().ok_or("machine.io")?;
        let (requests, receive) = mpsc::sync_channel::<Exchange>(1);
        let worker = thread::spawn(move || {
            while let Ok((value, reply)) = receive.recv() {
                let result = search_wire::write(&mut input, &value)
                    .and_then(|_| search_wire::read(&mut output))
                    .and_then(|value| value.ok_or_else(|| "machine.connection_failed".to_string()));
                let failed = result.is_err();
                let _ = reply.send(result);
                if failed {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            requests: Some(requests),
            worker: Some(worker),
            sequence: 0,
            touched: Instant::now(),
        })
    }

    pub fn request(
        &mut self,
        machine: &Machine,
        request: Value,
        stop: &AtomicBool,
    ) -> Result<Value, String> {
        self.exchange(machine, request, stop, Duration::from_secs(30))
    }

    pub(crate) fn exchange(
        &mut self,
        machine: &Machine,
        request: Value,
        stop: &AtomicBool,
        timeout: Duration,
    ) -> Result<Value, String> {
        self.sequence += 1;
        let id = self.sequence;
        let (reply, receive) = mpsc::channel();
        self.requests.as_ref().ok_or("machine.connection_failed")?.try_send((
            json!({"version":2,"id":id,"host_identity":machine.host_identity,"session":machine.session,"request":request}), reply
        )).map_err(|_| "machine.connection_failed")?;
        let deadline = Instant::now() + timeout;
        let mut response = loop {
            if stop.load(Ordering::Acquire) {
                return Err("machine.cancelled".into());
            }
            if Instant::now() >= deadline {
                return Err("machine.timeout".into());
            }
            match receive.recv_timeout(Duration::from_millis(25)) {
                Ok(result) => break result?,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err("machine.connection_failed".into()),
            }
        };
        self.touched = Instant::now();
        if response["version"] != 2
            || response["id"] != id
            || response["host_identity"].as_str() != machine.host_identity.as_deref()
        {
            return Err("machine.identity_changed".into());
        }
        if let Some(error) = response["error"].as_str() {
            return Err(error.into());
        }
        Ok(response["result"].take())
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.requests.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    fn peer(mode: &str) -> Connection {
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", r#"
            if ($env:KANBAN_SEARCH_TEST_MODE -eq 'eof') { exit }
            if ($env:KANBAN_SEARCH_TEST_MODE -eq 'wait') { Start-Sleep 60; exit }
            $reader = [IO.BinaryReader]::new([Console]::OpenStandardInput())
            $writer = [IO.BinaryWriter]::new([Console]::OpenStandardOutput())
            while ($true) {
                $size = $reader.ReadUInt32()
                $request = [Text.Encoding]::UTF8.GetString($reader.ReadBytes($size)) | ConvertFrom-Json
                $hostId = $request.host_identity
                if ($env:KANBAN_SEARCH_TEST_MODE -eq 'wrong') { $hostId = 'other-host' }
                $reply = @{version=2;id=$request.id;host_identity=$hostId;result=$request.request} | ConvertTo-Json -Compress
                $bytes = [Text.Encoding]::UTF8.GetBytes($reply)
                $writer.Write([uint32]$bytes.Length); $writer.Write($bytes); $writer.Flush()
            }
        "#]).env("KANBAN_SEARCH_TEST_MODE", mode);
        Connection::spawn(command).unwrap()
    }
    fn machine() -> Machine {
        Machine {
            id: "test".into(),
            name: "test".into(),
            kind: "ssh".into(),
            profile_id: None,
            ssh_target: None,
            session: "test".into(),
            platform: None,
            disabled: false,
            helper_name: None,
            host_identity: Some("expected-host".into()),
        }
    }
    #[test]
    fn persistent_child_preserves_request_identity_and_rejects_wrong_host() {
        let mut connection = peer("echo");
        let pid = connection.child.id();
        let stop = AtomicBool::new(false);
        for query in ["first", "中文"] {
            let reply = connection
                .request(&machine(), json!({"query":query}), &stop)
                .unwrap();
            assert_eq!(reply["query"], query);
            assert_eq!(connection.child.id(), pid);
        }
        assert_eq!(connection.sequence, 2);
        assert_eq!(
            peer("wrong")
                .request(&machine(), json!({}), &stop)
                .unwrap_err(),
            "machine.identity_changed"
        );
    }
    #[test]
    fn eof_timeout_and_shutdown_release_blocked_child_and_worker() {
        let stop = AtomicBool::new(false);
        assert!(peer("eof").request(&machine(), json!({}), &stop).is_err());
        let start = Instant::now();
        let mut waiting = peer("wait");
        assert_eq!(
            waiting
                .exchange(&machine(), json!({}), &stop, Duration::from_millis(100))
                .unwrap_err(),
            "machine.timeout"
        );
        drop(waiting);
        let mut cancelled = peer("wait");
        stop.store(true, Ordering::Release);
        assert_eq!(
            cancelled.request(&machine(), json!({}), &stop).unwrap_err(),
            "machine.cancelled"
        );
        drop(cancelled);
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
