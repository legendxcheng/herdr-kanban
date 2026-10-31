use super::*;
use crate::models::TaskPriority;
use crate::task_titles::{NewTask, TaskTitles};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Instant,
};

struct Server {
    endpoint: String,
    requests: mpsc::Receiver<(String, Value)>,
    release: mpsc::Sender<()>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Server {
    // pause: 0=immediate, 1=before headers, 2=after headers.
    fn new(response: String, pause: u8) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/chat/completions", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let (sent, requests) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let worker = thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                let header_end = loop {
                    let Ok(n) = stream.read(&mut buffer) else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    request.extend_from_slice(&buffer[..n]);
                    if let Some(n) = request.windows(4).position(|b| b == b"\r\n\r\n") {
                        break n + 4;
                    }
                    assert!(request.len() < 65536);
                };
                let headers = String::from_utf8(request[..header_end].to_vec()).unwrap();
                let length: usize = headers
                    .lines()
                    .find_map(|s| {
                        s.to_lowercase()
                            .strip_prefix("content-length:")
                            .map(|s| s.trim().parse().unwrap())
                    })
                    .unwrap();
                while request.len() < header_end + length {
                    let n = stream.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buffer[..n]);
                }
                let body =
                    serde_json::from_slice(&request[header_end..header_end + length]).unwrap();
                let header_len = response.find("\r\n\r\n").map(|n| n + 4).unwrap_or(0);
                if pause == 2 {
                    let _ = stream.write_all(&response.as_bytes()[..header_len]);
                }
                if sent.send((headers, body)).is_err() {
                    return;
                }
                if pause != 0 {
                    while !done.load(Ordering::Acquire) {
                        if wait.recv_timeout(Duration::from_millis(10)).is_ok() {
                            break;
                        }
                    }
                }
                if !done.load(Ordering::Acquire) {
                    let start = if pause == 2 { header_len } else { 0 };
                    let _ = stream.write_all(&response.as_bytes()[start..]);
                }
            }
        });
        Self {
            endpoint,
            requests,
            release,
            stop,
            worker: Some(worker),
        }
    }
    fn generator(&self) -> DeepSeekTitles {
        DeepSeekTitles {
            key: "test-key".into(),
            endpoint: self.endpoint.clone(),
            timeout: Duration::from_secs(3),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.release.send(());
        self.worker.take().unwrap().join().unwrap();
    }
}
fn response(status: u16, body: &str) -> String {
    format!(
        "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn completion(title: &str) -> String {
    json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":json!({"title":title}).to_string()}}]}).to_string()
}
fn database() -> Arc<Mutex<Connection>> {
    let conn = Connection::open_in_memory().unwrap();
    crate::db::init(&conn).unwrap();
    conn.execute_batch(
        "INSERT INTO machines(id,name,kind,session) VALUES('offline','Offline','ssh','default');
        INSERT INTO projects(id,name,root_path,herdr_workspace,created_at,machine_id) VALUES
        ('local-project','Local','.','local-space','now','local'),
        ('ssh-project','Remote','Z:/unavailable','remote-space','now','offline');",
    )
    .unwrap();
    Arc::new(Mutex::new(conn))
}
fn task(project: &str) -> NewTask {
    NewTask { project_id: project.into(), name: None, prompt_json: json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"修复登录失败"},{"type":"mention","attrs":{"kind":"file","ref":"secret.txt","label":"引用名称"}}]}]}).to_string(), priority: TaskPriority::Normal, draft_id: None, locale: "zh-CN".into() }
}
fn service(
    db: &Arc<Mutex<Connection>>,
    generator: DeepSeekTitles,
) -> (TaskTitles, mpsc::Receiver<String>) {
    let (send, received) = mpsc::channel();
    (
        TaskTitles::new(
            db.clone(),
            Arc::new(generator),
            Arc::new(move |project| {
                let _ = send.send(project);
            }),
        ),
        received,
    )
}

#[test]
fn local_and_offline_ssh_tasks_use_same_http_contract_and_commit_titles() {
    let server = Server::new(response(200, &completion("“ 修复登录反馈 ”")), 1);
    let db = database();
    let (service, changed) = service(&db, server.generator());
    for project in ["local-project", "ssh-project"] {
        let task = service.create(task(project)).unwrap();
        assert_eq!(task.name, "修复登录失败");
        let (headers, body) = server
            .requests
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        assert!(headers.starts_with("POST /chat/completions HTTP/1.1"));
        assert!(
            headers
                .to_lowercase()
                .contains("authorization: bearer test-key\r\n")
        );
        assert_eq!(body["model"], "deepseek-flash");
        assert_eq!(body["thinking"]["type"], "disabled");
        assert_eq!(body["max_tokens"], 128);
        assert_eq!(body["stream"], false);
        assert_eq!(body["response_format"]["type"], "json_object");
        assert!(body["tools"].is_null());
        assert_eq!(body["messages"].as_array().unwrap().len(), 2);
        assert!(
            body["messages"][1]["content"]
                .as_str()
                .unwrap()
                .contains("引用名称")
        );
        assert!(!body.to_string().contains("secret.txt"));
        assert!(
            db.try_lock().is_ok(),
            "network request must not hold DB lock"
        );
        server.release.send(()).unwrap();
        assert_eq!(
            changed.recv_timeout(Duration::from_secs(3)).unwrap(),
            project
        );
        let db = db.lock().unwrap();
        assert_eq!(
            crate::tasks::list(&db, project).unwrap()[0].name,
            "修复登录反馈"
        );
        assert_eq!(
            crate::task_titles::view(&db, &task.id)
                .unwrap()
                .unwrap()
                .state,
            "succeeded"
        );
    }
    service.shutdown();
}

#[test]
fn http_failures_keep_summary_and_only_transient_errors_schedule_retry() {
    for (status, code, retry) in [
        (400, "title.bad_request", false),
        (401, "title.authentication", false),
        (402, "title.insufficient_balance", false),
        (403, "title.authentication", false),
        (422, "title.bad_request", false),
        (404, "title.request_rejected", false),
        (408, "title.timeout", true),
        (429, "title.rate_limited", true),
        (500, "title.service_unavailable", true),
        (503, "title.service_unavailable", true),
    ] {
        let server = Server::new(response(status, "sensitive echoed key and task"), 0);
        let db = database();
        let (service, changed) = service(&db, server.generator());
        let task = service.create(task("local-project")).unwrap();
        changed.recv_timeout(Duration::from_secs(3)).unwrap();
        let conn = db.lock().unwrap();
        let state = crate::task_titles::view(&conn, &task.id).unwrap().unwrap();
        assert_eq!(state.error, code);
        assert_eq!(state.state, if retry { "retry_wait" } else { "failed" });
        assert_eq!(state.attempts, 1);
        assert_eq!(
            crate::tasks::list(&conn, "local-project").unwrap()[0].name,
            task.name
        );
        drop(conn);
        service.shutdown();
        assert_eq!(
            server.requests.try_iter().count(),
            1,
            "no HTTP-level retries"
        );
    }
}

#[test]
fn invalid_completions_and_multiline_titles_never_replace_summary() {
    let valid: Value = serde_json::from_str(&completion("Title")).unwrap();
    let mut truncated = valid.clone();
    truncated["choices"][0]["finish_reason"] = json!("length");
    let mut tool = valid.clone();
    tool["choices"][0]["message"]["tool_calls"] = json!([{"id":"tool"}]);
    let mut extra = valid.clone();
    extra["choices"][0]["message"]["content"] = json!("{\"title\":\"Title\",\"extra\":true}");
    for body in [
        "not json".into(),
        "{\"choices\":[]}".into(),
        truncated.to_string(),
        tool.to_string(),
        extra.to_string(),
        completion("  "),
        completion("first\nsecond"),
        completion("first\u{2028}second"),
    ] {
        let server = Server::new(response(200, &body), 0);
        let db = database();
        let (service, changed) = service(&db, server.generator());
        let task = service.create(task("local-project")).unwrap();
        changed.recv_timeout(Duration::from_secs(3)).unwrap();
        let conn = db.lock().unwrap();
        let state = crate::task_titles::view(&conn, &task.id).unwrap().unwrap();
        assert_eq!(state.error, "title.invalid_output");
        assert_eq!(state.state, "retry_wait");
        assert_eq!(
            crate::tasks::list(&conn, "local-project").unwrap()[0].name,
            task.name
        );
        drop(conn);
        service.shutdown();
    }
}

#[test]
fn oversized_bodies_with_or_without_content_length_and_redirects_are_rejected() {
    let large = "x".repeat(MAX_RESPONSE + 1);
    let chunked = format!(
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:X}\r\n{large}\r\n0\r\n\r\n",
        large.len()
    );
    let destination = Server::new(response(200, &completion("Should not be used")), 0);
    for (reply, error) in [
        (response(200, &large), "title.output_limit"),
        (chunked, "title.output_limit"),
        (
            format!(
                "HTTP/1.1 307 Redirect\r\nLocation: {}\r\nContent-Length: 0\r\n\r\n",
                destination.endpoint
            ),
            "title.request_rejected",
        ),
    ] {
        let server = Server::new(reply, 0);
        assert_eq!(
            server
                .generator()
                .generate("p", "input", &AtomicBool::new(false))
                .unwrap_err(),
            error
        );
        assert_eq!(server.requests.try_iter().count(), 1);
    }
    assert!(destination.requests.try_recv().is_err());
}

#[test]
fn missing_or_invalid_key_and_invalid_input_never_send_requests() {
    let server = Server::new(response(200, &completion("Unused")), 0);
    for key in ["", "Bearer test-key", "test\nkey"] {
        let mut generator = server.generator();
        generator.key = key.into();
        assert_eq!(
            generator
                .generate("p", "input", &AtomicBool::new(false))
                .unwrap_err(),
            if key.is_empty() {
                "title.key_missing"
            } else {
                "title.key_invalid"
            }
        );
    }
    for input in [String::new(), "字".repeat(2001)] {
        assert_eq!(
            server
                .generator()
                .generate("p", &input, &AtomicBool::new(false))
                .unwrap_err(),
            "title.input"
        );
    }
    assert!(server.requests.try_recv().is_err());
}

#[test]
fn shutdown_cancels_stalled_headers_or_body_and_preserves_retry_budget() {
    for pause in [1, 2] {
        let server = Server::new(response(200, &completion("Late title")), pause);
        let db = database();
        let (service, _) = service(&db, server.generator());
        let task = service.create(task("local-project")).unwrap();
        server
            .requests
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        let start = Instant::now();
        service.shutdown();
        assert!(start.elapsed() < Duration::from_secs(1));
        let conn = db.lock().unwrap();
        let state = crate::task_titles::view(&conn, &task.id).unwrap().unwrap();
        assert_eq!(state.state, "retry_wait");
        assert_eq!(state.attempts, 1);
        assert_eq!(state.error, "title.cancelled");
    }
}

#[test]
fn total_timeout_covers_stalled_headers_and_body() {
    for pause in [1, 2] {
        let server = Server::new(response(200, &completion("Late title")), pause);
        let mut generator = server.generator();
        generator.timeout = Duration::from_millis(250);
        let start = Instant::now();
        assert_eq!(
            generator
                .generate("p", "input", &AtomicBool::new(false))
                .unwrap_err(),
            "title.timeout"
        );
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}

#[test]
fn connection_failure_is_transient_without_raw_diagnostics() {
    // Close an accepted request without HTTP headers: deterministic on Windows,
    // where connecting to an unused port may time out rather than refuse early.
    let server = Server::new(String::new(), 0);
    assert_eq!(
        server
            .generator()
            .generate("p", "input", &AtomicBool::new(false))
            .unwrap_err(),
        "title.network"
    );
    assert_eq!(server.requests.try_iter().count(), 1);
}

#[test]
#[ignore = "requires the private build-time DeepSeek key; makes one paid naming request"]
fn live_deepseek_title_updates_real_sqlite() {
    assert!(
        !API_KEY.is_empty(),
        "configure private key and rebuild first"
    );
    let db = database();
    let (service, changed) = service(&db, DeepSeekTitles::embedded());
    let task = service.create(task("local-project")).unwrap();
    changed.recv_timeout(Duration::from_secs(35)).unwrap();
    service.shutdown();
    let conn = db.lock().unwrap();
    let state = crate::task_titles::view(&conn, &task.id).unwrap().unwrap();
    assert_eq!(state.state, "succeeded", "{}", state.error);
    let name = &crate::tasks::list(&conn, "local-project").unwrap()[0].name;
    assert!(!name.is_empty() && name.chars().count() <= 40);
}
