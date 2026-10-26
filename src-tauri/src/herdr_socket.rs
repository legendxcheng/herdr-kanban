//! Small bounded JSONL client for methods not exposed by the installed CLI.
//! Resolve the endpoint through that CLI, preserving its session/config rules.
use super::{Cli, field, workspace_directories};
use crate::projects::Workspace;
use serde_json::Value;
use std::{
    io::{Read, Write},
    thread,
    time::{Duration, Instant},
};

impl Cli {
    pub(super) fn parse_workspace_move(
        &self,
        doc: &Value,
        id: &str,
        before: Option<&str>,
    ) -> Result<Vec<Workspace>, String> {
        // The RPC returns workspace_list. workspace.reordered is the separate event.
        if doc.pointer("/result/type").and_then(Value::as_str) != Some("workspace_list") {
            return Err("herdr.invalid_response".into());
        }
        let workspaces = self.parse_workspaces(doc)?;
        let position = workspaces
            .iter()
            .position(|w| w.id == id)
            .ok_or("herdr.invalid_response")?;
        if workspaces.get(position + 1).map(|w| w.id.as_str()) != before {
            return Err("herdr.invalid_response".into());
        }
        Ok(workspaces)
    }
    pub(super) fn parse_workspaces(&self, doc: &Value) -> Result<Vec<Workspace>, String> {
        let spaces = doc
            .pointer("/result/workspaces")
            .and_then(Value::as_array)
            .ok_or("herdr.invalid_response")?;
        let directories = workspace_directories(self.session_file().as_deref()).unwrap_or_default();
        let mut ids = std::collections::HashSet::new();
        spaces
            .iter()
            .map(|w| {
                let id = field(w, "workspace_id")?;
                if !ids.insert(id) {
                    return Err("herdr.invalid_response".into());
                }
                Ok(Workspace {
                    id: id.into(),
                    name: field(w, "label")?.into(),
                    directory: directories.get(id).cloned(),
                })
            })
            .collect()
    }

    pub(super) fn socket_request(&self, method: &str, params: Value) -> Result<Value, String> {
        let status = self.output(&["status", "server"]).map_err(|e| e.message)?;
        let status = std::str::from_utf8(&status).map_err(|_| "herdr.invalid_response")?;
        let endpoint = status
            .lines()
            .find_map(|line| line.strip_prefix("socket: "))
            .filter(|s| !s.trim().is_empty())
            .ok_or("herdr.invalid_response")?
            .trim();
        let deadline = Instant::now() + Duration::from_secs(12);
        let mut stream = connect(endpoint, deadline)?;
        let id = uuid::Uuid::new_v4().to_string();
        let mut request = serde_json::to_vec(
            &serde_json::json!({ "id": id, "method": method, "params": params }),
        )
        .map_err(|_| "herdr.invalid_response")?;
        request.push(b'\n');
        exchange(&mut stream, &request, &id, deadline)
    }
}

fn retry(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::WouldBlock || error.raw_os_error() == Some(232)
}
fn pause(deadline: Instant) -> Result<(), String> {
    if Instant::now() >= deadline {
        return Err("herdr.timeout".into());
    }
    thread::sleep(Duration::from_millis(10));
    Ok(())
}
fn exchange(
    stream: &mut (impl Read + Write),
    request: &[u8],
    id: &str,
    deadline: Instant,
) -> Result<Value, String> {
    let mut written = 0;
    while written < request.len() {
        if Instant::now() >= deadline {
            return Err("herdr.timeout".into());
        }
        match stream.write(&request[written..]) {
            Ok(0) => pause(deadline)?,
            Ok(n) => written += n,
            Err(e) if retry(&e) => pause(deadline)?,
            Err(e) => return Err(format!("herdr.failed: {e}")),
        }
    }
    let mut response = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        if Instant::now() >= deadline {
            return Err("herdr.timeout".into());
        }
        match stream.read(&mut buffer) {
            Ok(0) => return Err("herdr.invalid_response".into()),
            Ok(n) => response.extend_from_slice(&buffer[..n]),
            Err(e) if retry(&e) => {
                pause(deadline)?;
                continue;
            }
            Err(e) => return Err(format!("herdr.failed: {e}")),
        }
        if response.len() > 4 * 1024 * 1024 {
            return Err("herdr.invalid_response".into());
        }
        if let Some(end) = response.iter().position(|b| *b == b'\n') {
            let doc: Value =
                serde_json::from_slice(&response[..end]).map_err(|_| "herdr.invalid_response")?;
            if doc.get("id").and_then(Value::as_str) != Some(id) {
                return Err("herdr.invalid_response".into());
            }
            if let Some(error) = doc.get("error").filter(|e| !e.is_null()) {
                let detail: String = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Request rejected")
                    .chars()
                    .take(300)
                    .collect();
                return Err(format!("herdr.failed: {detail}"));
            }
            if !doc.get("result").is_some_and(Value::is_object) {
                return Err("herdr.invalid_response".into());
            }
            return Ok(doc);
        }
    }
}

#[cfg(windows)]
struct Pipe(std::fs::File);

#[cfg(windows)]
impl Read for Pipe {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        use std::{ffi::c_void, os::windows::io::AsRawHandle};
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn PeekNamedPipe(
                handle: *mut c_void,
                buffer: *mut c_void,
                size: u32,
                read: *mut u32,
                available: *mut u32,
                left: *mut u32,
            ) -> i32;
        }
        let mut available = 0;
        if unsafe {
            PeekNamedPipe(
                self.0.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        // std::fs::File maps Windows ERROR_NO_DATA to Ok(0). On a NOWAIT pipe,
        // that means "not yet", not EOF. Peek also distinguishes a broken peer.
        if available == 0 {
            return Err(std::io::ErrorKind::WouldBlock.into());
        }
        self.0.read(buffer)
    }
}
#[cfg(windows)]
impl Write for Pipe {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.write(data)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

#[cfg(windows)]
fn connect(endpoint: &str, deadline: Instant) -> Result<Pipe, String> {
    use std::{ffi::c_void, os::windows::io::AsRawHandle};
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetNamedPipeHandleState(
            handle: *mut c_void,
            mode: *const u32,
            max_count: *const u32,
            timeout: *const u32,
        ) -> i32;
    }
    let path = if endpoint.starts_with(r"\\.\pipe\") {
        endpoint.to_owned()
    } else {
        format!(r"\\.\pipe\{endpoint}")
    };
    let pipe = loop {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
        {
            Ok(pipe) => break pipe,
            Err(e) if e.raw_os_error() == Some(231) => pause(deadline)?,
            Err(e) => return Err(format!("herdr.failed: {e}")),
        }
    };
    // PIPE_NOWAIT makes both reads and writes bounded by our deadline. File owns
    // the handle; no detached I/O thread or pending operation survives a timeout.
    let mode = 1u32;
    if unsafe {
        SetNamedPipeHandleState(
            pipe.as_raw_handle(),
            &mode,
            std::ptr::null(),
            std::ptr::null(),
        )
    } == 0
    {
        return Err(format!("herdr.failed: {}", std::io::Error::last_os_error()));
    }
    Ok(Pipe(pipe))
}

#[cfg(unix)]
fn connect(endpoint: &str, _deadline: Instant) -> Result<std::os::unix::net::UnixStream, String> {
    let stream = std::os::unix::net::UnixStream::connect(endpoint)
        .map_err(|e| format!("herdr.failed: {e}"))?;
    stream
        .set_nonblocking(true)
        .map_err(|e| format!("herdr.failed: {e}"))?;
    Ok(stream)
}

#[cfg(not(any(windows, unix)))]
fn connect(_: &str, _: Instant) -> Result<std::io::Cursor<Vec<u8>>, String> {
    Err("workspace.unsupported".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cli() -> Cli {
        Cli {
            remote: None,
            executable: "herdr".into(),
            session: "test-reorder".into(),
        }
    }
    fn reply() -> Value {
        // Native protocol 19 reply: move_block returns workspace_list, not an event.
        serde_json::json!({"id":"probe","result":{"type":"workspace_list","workspaces":[
            {"workspace_id":"wB","label":"B"},{"workspace_id":"wA","label":"A"}
        ]}})
    }
    #[test]
    fn accepts_real_move_block_reply_and_validates_the_requested_placement() {
        let moved = cli()
            .parse_workspace_move(&reply(), "wB", Some("wA"))
            .unwrap();
        assert_eq!(
            moved.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            ["wB", "wA"]
        );
        assert!(cli().parse_workspace_move(&reply(), "wA", None).is_ok());
        assert!(
            cli()
                .parse_workspace_move(&reply(), "wA", Some("wB"))
                .is_err()
        );
        assert!(
            cli()
                .parse_workspace_move(&reply(), "missing", None)
                .is_err()
        );
    }
    #[test]
    fn malformed_or_duplicate_workspace_reply_cannot_replace_durable_navigation() {
        let mut doc = reply();
        doc["result"]["workspaces"][1]["workspace_id"] = "wB".into();
        assert!(cli().parse_workspace_move(&doc, "wB", None).is_err());
        assert!(
            cli()
                .parse_workspace_move(&serde_json::json!({"result":{}}), "wB", None)
                .is_err()
        );
    }

    #[cfg(windows)]
    #[test]
    fn named_pipe_waits_for_delayed_fragmented_reply_instead_of_treating_no_data_as_eof() {
        use std::{
            ffi::c_void,
            os::windows::io::{AsRawHandle, FromRawHandle},
        };
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn CreateNamedPipeW(
                name: *const u16,
                access: u32,
                mode: u32,
                instances: u32,
                out_size: u32,
                in_size: u32,
                timeout: u32,
                security: *const c_void,
            ) -> *mut c_void;
            fn ConnectNamedPipe(handle: *mut c_void, overlapped: *mut c_void) -> i32;
        }
        let path = format!(r"\\.\pipe\kanban-reorder-test-{}", uuid::Uuid::new_v4());
        let name: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
        let handle =
            unsafe { CreateNamedPipeW(name.as_ptr(), 3, 0, 1, 4096, 4096, 0, std::ptr::null()) };
        assert_ne!(handle as isize, -1);
        let mut pipe = unsafe { std::fs::File::from_raw_handle(handle) };
        let server = thread::spawn(move || {
            if unsafe { ConnectNamedPipe(pipe.as_raw_handle(), std::ptr::null_mut()) } == 0 {
                assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(535));
            }
            let mut request = Vec::new();
            let mut byte = [0];
            loop {
                pipe.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
                if byte[0] == b'\n' {
                    break;
                }
            }
            thread::sleep(Duration::from_millis(70));
            pipe.write_all(b"{\"id\":\"fixture\",").unwrap();
            thread::sleep(Duration::from_millis(20));
            pipe.write_all(b"\"result\":{\"type\":\"pong\"}}\n")
                .unwrap();
            thread::sleep(Duration::from_millis(30));
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut client = connect(&path, deadline).unwrap();
        let response = exchange(
            &mut client,
            b"{\"id\":\"fixture\",\"method\":\"ping\",\"params\":{}}\n",
            "fixture",
            deadline,
        );
        server.join().unwrap();
        assert_eq!(response.unwrap()["result"]["type"], "pong");
    }
}
