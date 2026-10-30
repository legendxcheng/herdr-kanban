use super::*;
use serde_json::json;

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
        host_identity: Some("expected".into()),
    }
}

fn peer(mode: &str) -> Connection {
    let mut command = std::process::Command::new("powershell.exe");
    command.args(["-NoProfile","-NonInteractive","-Command",r#"
        $reader=[IO.BinaryReader]::new([Console]::OpenStandardInput())
        $writer=[IO.BinaryWriter]::new([Console]::OpenStandardOutput())
        $sequence=0
        while ($true) {
            $size=$reader.ReadUInt32()
            $request=[Text.Encoding]::UTF8.GetString($reader.ReadBytes($size)) | ConvertFrom-Json
            $sequence++
            if ($env:KANBAN_CONTROL_TEST_MODE -eq 'lost') { exit }
            if ($env:KANBAN_CONTROL_TEST_MODE -eq 'slow') { Start-Sleep 60; exit }
            $session=$request.session
            $id=$request.id
            if ($env:KANBAN_CONTROL_TEST_MODE -eq 'session') { $session='other-session' }
            if ($env:KANBAN_CONTROL_TEST_MODE -eq 'id') { $id=0 }
            $replyError=$null
            if ($request.request.method -eq 'reject') { $replyError='flow.not_idle' }
            $reply=@{version=2;id=$id;host_identity=$request.host_identity;session=$session;
                result=@{value=@{sequence=$sequence;pid=$PID;text=$request.request.text};error=$replyError}} | ConvertTo-Json -Depth 8 -Compress
            $bytes=[Text.Encoding]::UTF8.GetBytes($reply)
            $writer.Write([uint32]$bytes.Length); $writer.Write($bytes); $writer.Flush()
        }
    "#]).env("KANBAN_CONTROL_TEST_MODE",mode);
    Connection::spawn(command).unwrap()
}

fn session(mode: &str) -> Session {
    Session {
        connection: Some(peer(mode)),
        ..Session::default()
    }
}

#[test]
fn one_child_preserves_frames_and_acknowledged_errors_do_not_poison_it() {
    let mut session = session("echo");
    let first = session
        .request(&machine(), json!({"text":"测试\n$ @"}))
        .unwrap();
    assert_eq!(first["sequence"], 1);
    assert_eq!(first["text"], "测试\n$ @");
    assert_eq!(
        session
            .request(&machine(), json!({"method":"reject"}))
            .unwrap_err(),
        "flow.not_idle"
    );
    let third = session.request(&machine(), json!({})).unwrap();
    assert_eq!(third["sequence"], 3);
    assert_eq!(first["pid"], third["pid"]);
}

#[test]
fn lost_reply_or_wrong_identity_never_reconnects_or_replays() {
    for mode in ["lost", "session", "id"] {
        let mut session = session(mode);
        assert!(
            session
                .request(&machine(), json!({"method":"mutation"}))
                .is_err()
        );
        assert!(session.connection.is_none());
        assert!(!session.recover_observation());
        assert_eq!(
            session
                .request(&machine(), json!({"method":"mutation"}))
                .unwrap_err(),
            "machine.connection_failed"
        );
    }
}

#[test]
fn only_failed_observation_can_reconnect_after_eof_or_timeout() {
    for mode in ["lost", "slow"] {
        let mut session = session(mode);
        let read = json!({"method":"cli_output","args":["agent","get","w1:p1"]});
        let timeout = if mode == "lost" {
            Duration::from_secs(5)
        } else {
            Duration::from_millis(800)
        };
        let result = session.request_connecting(&machine(), read.clone(), timeout, |_| {
            panic!("already connected")
        });
        assert_eq!(
            result.unwrap_err(),
            if mode == "lost" {
                "machine.connection_failed"
            } else {
                "machine.timeout"
            }
        );
        assert!(session.connection.is_none());
        assert!(session.failed);
        assert!(session.recover_observation());
        // Recovery preserves the frozen machine metadata before constructing any child.
        let mut changed = machine();
        changed.host_identity = Some("other-host".into());
        assert_eq!(
            session
                .request_connecting(&changed, read.clone(), Duration::from_secs(3), |_| panic!(
                    "wrong host must not connect"
                ))
                .unwrap_err(),
            "machine.identity_changed"
        );
        let reply = session
            .request_connecting(&machine(), read, Duration::from_secs(3), |_| {
                Ok(peer("echo"))
            })
            .unwrap();
        assert_eq!(
            reply["sequence"], 1,
            "new child only receives the new observation"
        );
        assert_eq!(
            session
                .request(&machine(), json!({"method":"mutation"}))
                .unwrap()["sequence"],
            2
        );
    }
}

#[test]
fn wrong_identity_or_protocol_during_a_read_cannot_be_recovered() {
    for mode in ["session", "id"] {
        let mut session = session(mode);
        assert!(
            session
                .request(
                    &machine(),
                    json!({"method":"cli_output","args":["agent","get","pane"]})
                )
                .is_err()
        );
        assert!(!session.recover_observation());
    }
    for args in [
        json!(["agent", "send-keys", "pane", "enter"]),
        json!(["pane", "send-text", "pane", "body"]),
        json!(["agent", "get", "pane", "extra"]),
    ] {
        let mut session = session("lost");
        assert!(
            session
                .request(&machine(), json!({"method":"cli_output","args":args}))
                .is_err()
        );
        assert!(!session.recover_observation());
    }
}

#[test]
fn scope_is_thread_local_pins_all_machine_fields_and_drops_on_unwind() {
    assert!(request(&machine(), json!({})).is_none());
    let result = std::panic::catch_unwind(|| {
        let _scope = Scope::enter();
        ACTIVE.with(|slot| *slot.borrow_mut() = Some(session("echo")));
        assert!(Scope::enter_if_absent().is_none());
        request(&machine(), json!({})).unwrap().unwrap();
        let mut changed = machine();
        changed.helper_name = Some("another-version".into());
        assert_eq!(
            request(&changed, json!({})).unwrap().unwrap_err(),
            "machine.identity_changed"
        );
        std::thread::spawn(|| assert!(request(&machine(), json!({})).is_none()))
            .join()
            .unwrap();
        panic!("simulated worker unwind");
    });
    assert!(result.is_err());
    assert!(request(&machine(), json!({})).is_none());
}
