use super::*;
use std::time::Duration;
use serde_json::json;
use std::{process::Command, sync::atomic::{AtomicUsize, AtomicBool, Ordering}};

fn machine() -> Machine {
    Machine {
        id: "test".into(),
        name: "test".into(),
        kind: "ssh".into(),
        profile_id: Some("p".into()),
        ssh_target: Some("host".into()),
        session: "default".into(),
        platform: Some("windows".into()),
        disabled: false,
        helper_name: Some("v1".into()),
        host_identity: Some("identity".into()),
    }
}
fn peer() -> Connection {
    let mut command = Command::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-Command", r#"
$reader=[IO.BinaryReader]::new([Console]::OpenStandardInput())
$writer=[IO.BinaryWriter]::new([Console]::OpenStandardOutput())
while ($true) {
 try { $size=$reader.ReadInt32() } catch { break }
 $r=[Text.Encoding]::UTF8.GetString($reader.ReadBytes($size)) | ConvertFrom-Json
 if ($r.request.method -eq 'disconnect') { exit }
 $errorCode=if ($r.request.method -eq 'domain_error') { 'workspace.missing' } else { $null }
 $reply=@{version=2;id=$r.id;host_identity=$r.host_identity;session=$r.session;result=@{value=$r.request;error=$errorCode}} | ConvertTo-Json -Compress -Depth 8
 if ($r.request.method -eq 'malformed') { $reply=@{version=2;id=$r.id;host_identity=$r.host_identity;session=$r.session;result=@{}} | ConvertTo-Json -Compress -Depth 8 }
 $bytes=[Text.Encoding]::UTF8.GetBytes($reply);$writer.Write([int]$bytes.Length);$writer.Write($bytes);$writer.Flush()
}
"#]);
    Connection::spawn(command).unwrap()
}

#[test]
fn malformed_result_retires_the_connection_before_the_next_query() {
    let pool = Pool::default();
    let connections = AtomicUsize::new(0);
    let connect = || { connections.fetch_add(1, Ordering::SeqCst); Ok(peer()) };
    assert_eq!(pool.request(&machine(), "view", json!({"method":"malformed"}), connect).unwrap_err(), "machine.invalid_response");
    assert!(pool.request(&machine(), "view", json!({"method":"hello"}), connect).is_ok());
    assert_eq!(connections.load(Ordering::SeqCst), 2);
}
#[test]
fn warm_reads_reuse_connections_and_identity_changes_reconnect() {
    let pool = Pool::default();
    let mut machine = machine();
    let connections = AtomicUsize::new(0);
    let connect = || {
        connections.fetch_add(1, Ordering::SeqCst);
        Ok(peer())
    };
    for _ in 0..3 {
        assert_eq!(
            pool.request(
                &machine,
                "inventory",
                json!({"method":"workspaces"}),
                connect
            )
            .unwrap()["method"],
            "workspaces"
        );
    }
    assert_eq!(connections.load(Ordering::SeqCst), 1);
    pool.request(
        &machine,
        "status",
        json!({"method":"agent_status_batch"}),
        connect,
    )
    .unwrap();
    assert_eq!(connections.load(Ordering::SeqCst), 2);
    machine.helper_name = Some("v2".into());
    pool.request(
        &machine,
        "inventory",
        json!({"method":"workspaces"}),
        connect,
    )
    .unwrap();
    assert_eq!(connections.load(Ordering::SeqCst), 3);
    machine.disabled = true;
    assert_eq!(
        pool.request(&machine, "inventory", json!({}), connect)
            .unwrap_err(),
        "machine.disabled"
    );
    assert_eq!(connections.load(Ordering::SeqCst), 3);
}

#[test]
fn lost_read_is_not_replayed_and_next_poll_can_reconnect() {
    let pool = Pool::default();
    let machine = machine();
    let connections = AtomicUsize::new(0);
    let connect = || {
        connections.fetch_add(1, Ordering::SeqCst);
        Ok(peer())
    };
    assert!(
        pool.request(
            &machine,
            "inventory",
            json!({"method":"disconnect"}),
            connect
        )
        .is_err()
    );
    assert_eq!(connections.load(Ordering::SeqCst), 1);
    assert_eq!(
        pool.request(&machine, "inventory", json!({"method":"hello"}), connect)
            .unwrap()["method"],
        "hello"
    );
    assert_eq!(connections.load(Ordering::SeqCst), 2);
    assert_eq!(
        pool.request(
            &machine,
            "inventory",
            json!({"method":"domain_error"}),
            connect
        )
        .unwrap_err(),
        "workspace.missing"
    );
    pool.request(&machine, "inventory", json!({"method":"hello"}), connect)
        .unwrap();
    assert_eq!(connections.load(Ordering::SeqCst), 2);
    pool.0.shutdown();
    assert_eq!(
        pool.request(&machine, "inventory", json!({}), connect)
            .unwrap_err(),
        "machine.cancelled"
    );
}

#[test]
fn writes_and_task_observations_never_enter_shared_read_lanes() {
    for method in [
        "cli_output",
        "image",
        "stage_startup_prompt",
        "observe_startup_prompt",
        "move_workspace",
        "idle_cleanup_close",
        "prompt_receipt",
    ] {
        assert!(lane(&json!({"method":method})).is_none(), "{method}");
    }
    assert_ne!(
        lane(&json!({"method":"workspaces"})),
        lane(&json!({"method":"agent_status_batch"}))
    );
    assert_ne!(
        lane(&json!({"method":"workspaces"})),
        lane(&json!({"method":"git_statuses"}))
    );
}

#[test]
#[ignore = "Uploads a content-addressed helper to the selected SSH machine and benchmarks read-only queries"]
fn live_ssh_read_performance() {
    use std::time::Instant;
    let target = std::env::var("KANBAN_PERF_SSH_TARGET").expect("explicit SSH target");
    let helper = std::env::var("KANBAN_PERF_HELPER").expect("built helper");
    let output = std::env::var("KANBAN_PERF_OUTPUT").expect("evidence output");
    let db = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    let profile = crate::remote_transport::saved_profiles()
        .unwrap()
        .into_iter()
        .find(|p| p.target == target)
        .unwrap();
    let machine = crate::machines::import_profile(&db, &profile, "windows").unwrap();
    let machine = crate::remote_install::install(&machine, std::path::Path::new(&helper)).unwrap();
    let pool = Pool::default();
    let connections = AtomicUsize::new(0);
    let connect = || {
        connections.fetch_add(1, Ordering::SeqCst);
        crate::remote_transport::verify_profile(&machine)?;
        Connection::spawn(crate::remote_transport::control_command(&machine)?)
    };
    let one_shot = |request: Value| -> Value {
        crate::remote_transport::request_with_budget(
            &machine,
            request,
            &crate::remote_transport::Budget {
                stop: &AtomicBool::new(false),
                deadline: Instant::now() + Duration::from_secs(55),
            },
        )
        .unwrap()
    };
    let mut old = Vec::new();
    for _ in 0..3 {
        let start = Instant::now();
        one_shot(json!({"method":"hello"}));
        assert!(one_shot(json!({"method":"workspaces"})).is_array());
        old.push(start.elapsed().as_millis() as u64);
    }
    let mut new = Vec::new();
    let mut workspaces = Value::Null;
    for _ in 0..3 {
        let start = Instant::now();
        workspaces = pool
            .request(
                &machine,
                "inventory",
                json!({"method":"workspaces"}),
                connect,
            )
            .unwrap();
        assert!(workspaces.is_array());
        new.push(start.elapsed().as_millis() as u64);
    }
    assert_eq!(connections.load(Ordering::SeqCst), 1);
    let ids = workspaces
        .as_array()
        .unwrap()
        .iter()
        .take(3)
        .map(|w| w["id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    let start = Instant::now();
    for id in &ids {
        one_shot(json!({"method":"hello"}));
        one_shot(json!({"method":"agent_status_targets","workspace":id}));
    }
    let old_status = start.elapsed().as_millis() as u64;
    let mut status = Vec::new();
    for _ in 0..3 {
        let start = Instant::now();
        let response = pool
            .request(
                &machine,
                "status",
                json!({"method":"agent_status_batch","workspaces":ids}),
                connect,
            )
            .unwrap();
        for id in &ids {
            assert!(response[id].get("Ok").is_some(), "missing workspace result");
        }
        status.push(start.elapsed().as_millis() as u64);
    }
    assert_eq!(connections.load(Ordering::SeqCst), 2);
    let result = json!({"old_sync_ms":old,"new_sync_ms":new,"workspace_count":ids.len(),"old_status_round_ms":old_status,"new_status_round_ms":status,"connections_for_six_new_requests":connections.load(Ordering::SeqCst)});
    std::fs::write(output, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    println!("{result}");
}
