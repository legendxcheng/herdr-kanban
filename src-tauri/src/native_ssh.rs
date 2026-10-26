//! Raw official Herdr remote-client-bridge transport. Never decode terminal bytes.
use std::{
    io::{Read, Write},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub struct Stream {
    child: Child,
    input: Option<mpsc::SyncSender<(Vec<u8>, mpsc::Sender<Result<(), String>>)>>,
    output: Option<mpsc::Receiver<Result<Vec<u8>, String>>>,
    workers: Vec<thread::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}
impl Stream {
    pub fn connect(machine: &crate::machines::Machine) -> Result<Self, String> {
        // The operation-scoped adapter already verified the saved profile and host.
        // Do not add blocking probes to the cancellable native relay thread.
        if machine.session.is_empty()
            || !machine
                .session
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
        {
            return Err("machine.invalid_input".into());
        }
        let mut command = crate::remote_transport::ssh(
            machine
                .ssh_target
                .as_deref()
                .ok_or("machine.invalid_target")?,
        )?;
        let helper = machine
            .helper_name
            .as_deref()
            .ok_or("machine.setup_required")?;
        let identity = machine
            .host_identity
            .as_deref()
            .ok_or("machine.setup_required")?;
        if helper.len() != 32
            || !helper.bytes().all(|c| c.is_ascii_hexdigit())
            || uuid::Uuid::parse_str(identity).is_err()
        {
            return Err("machine.identity_changed".into());
        }
        // cmd inherits binary handles; PowerShell's native output pipeline would
        // decode/re-encode the official client stream.
        command.args(["cmd", "/d", "/s", "/c", &format!(
            "\"\"%USERPROFILE%\\.herdr-kanban\\helpers\\{helper}.exe\" --remote-client-bridge {identity} {}\"", machine.session
        )]);
        Self::spawn(command)
    }
    fn spawn(mut command: Command) -> Result<Self, String> {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("terminal.client_connect: {e}"))?;
        let mut stdin = child.stdin.take().ok_or("terminal.client_connect")?;
        let mut stdout = child.stdout.take().ok_or("terminal.client_connect")?;
        let (input, receive) = mpsc::sync_channel::<(Vec<u8>, mpsc::Sender<Result<(), String>>)>(1);
        let writer = thread::spawn(move || {
            while let Ok((bytes, reply)) = receive.recv() {
                let result = stdin
                    .write_all(&bytes)
                    .and_then(|_| stdin.flush())
                    .map_err(|e| format!("terminal.disconnected: {e}"));
                let failed = result.is_err();
                let _ = reply.send(result);
                if failed {
                    break;
                }
            }
        });
        let (output_send, output) = mpsc::sync_channel(8);
        let stop = Arc::new(AtomicBool::new(false));
        let cancelled = stop.clone();
        let reader = thread::spawn(move || {
            let mut buffer = [0u8; 65536];
            while !cancelled.load(Ordering::Acquire) {
                let result = match stdout.read(&mut buffer) {
                    Ok(0) => Err("terminal.disconnected".into()),
                    Ok(n) => Ok(buffer[..n].to_vec()),
                    Err(e) => Err(format!("terminal.disconnected: {e}")),
                };
                let failed = result.is_err();
                if output_send.send(result).is_err() || failed {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input: Some(input),
            output: Some(output),
            workers: vec![reader, writer],
            stop,
        })
    }
    pub fn read_available(&mut self, buffer: &mut Vec<u8>) -> Result<bool, String> {
        match self
            .output
            .as_ref()
            .ok_or("terminal.disconnected")?
            .try_recv()
        {
            Ok(result) => {
                buffer.extend(result?);
                if buffer.len() > 64 * 1024 * 1024 + 65540 {
                    return Err("terminal.client_protocol".into());
                }
                Ok(true)
            }
            Err(mpsc::TryRecvError::Empty) => Ok(false),
            Err(_) => Err("terminal.disconnected".into()),
        }
    }
    pub fn send(
        &mut self,
        bytes: &[u8],
        check: impl Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        check()?;
        let (send, reply) = mpsc::channel();
        self.input
            .as_ref()
            .ok_or("terminal.disconnected")?
            .try_send((bytes.to_vec(), send))
            .map_err(|_| "terminal.disconnected")?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            check()?;
            match reply.recv_timeout(Duration::from_millis(10)) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("terminal.disconnected".into());
                }
                Err(_) => {}
            }
            if Instant::now() >= deadline {
                return Err("terminal.client_focus_timeout".into());
            }
        }
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.output.take();
        self.input.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}
