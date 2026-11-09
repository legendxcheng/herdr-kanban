//! Exercise the local CLI adapter, including its receipt callback wiring.
use super::*;
use std::cell::Cell;

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn local_submission_checks_receipt_before_stale_status_expires_without_replaying() {
    let dir = std::env::temp_dir().join(format!("kanban-submit-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let fixture = Fixture(dir);
    let executable = fixture.0.join("herdr.cmd");
    std::fs::write(&executable, concat!(
        "@echo off\r\n",
        "if \"%~3 %~4\"==\"agent get\" (\r\n",
        "echo {\"result\":{\"agent\":{\"workspace_id\":\"w1\",\"pane_id\":\"w1:p1\",\"terminal_id\":\"term\",\"agent\":\"codex\",\"agent_status\":\"idle\",\"interactive_ready\":true,\"state_change_seq\":7}}}\r\nexit /b 0\r\n)\r\n",
        "if \"%~3 %~4\"==\"pane process-info\" (\r\n",
        "echo {\"result\":{\"process_info\":{\"shell_pid\":1,\"foreground_process_group_id\":2}}}\r\nexit /b 0\r\n)\r\n",
        "if \"%~3 %~4\"==\"pane send-text\" (\r\n",
        "echo text>>\"%~dp0writes\"\r\nexit /b 0\r\n)\r\n",
        "if \"%~3 %~4\"==\"agent send-keys\" (\r\n",
        "echo %~6>>\"%~dp0writes\"\r\necho {}\r\nexit /b 0\r\n)\r\n",
        "exit /b 3\r\n",
    )).unwrap();
    let cli = Cli {
        executable,
        session: "default".into(),
        remote: None,
    };
    let target = Target {
        machine_id: "local".into(),
        session: "default".into(),
        workspace_id: "w1".into(),
        pane_id: "w1:p1".into(),
        terminal_id: "term".into(),
        agent: "codex".into(),
        process_id: "1:2".into(),
        codex_resume_locks: vec![],
        title: String::new(),
        status: "idle".into(),
    };
    let checks = Cell::new(0);
    let receipts = Cell::new(0);
    let result = cli.send_checking_receipt(
        &target,
        "once",
        &|| {
            checks.set(checks.get() + 1);
            // Bound a broken callback route by query count instead of waiting 40 s.
            if checks.get() > 32 {
                Err("fixture.cancelled".into())
            } else {
                Ok(())
            }
        },
        &|| {
            receipts.set(receipts.get() + 1);
            Ok(true)
        },
    );
    assert_eq!(
        receipts.get(),
        1,
        "local adapter discarded the exact receipt callback"
    );
    let error = result.unwrap_err();
    assert_eq!(error.message, "dispatch.submit_verify_required");
    assert!(
        error.uncertain,
        "workflow must recheck and commit the receipt"
    );
    let writes = std::fs::read_to_string(fixture.0.join("writes")).unwrap();
    assert_eq!(
        writes.lines().collect::<Vec<_>>(),
        ["text", "right", "esc", "enter"]
    );
}
