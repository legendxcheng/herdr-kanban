//! Opt-in real Herdr/Codex acceptance on a newly created, isolated Workspace.
use crate::{
    models::TaskPriority,
    workflow::{Herdr, Workflow},
};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[test]
#[ignore = "Creates a dedicated recovery workspace and one Codex turn; requires KANBAN_RECOVERY_DATA"]
fn live_original_terminal_recovery_local_or_ssh() {
    let directory = std::path::PathBuf::from(
        std::env::var_os("KANBAN_RECOVERY_DATA").expect("isolated directory"),
    );
    assert!(
        directory
            .to_string_lossy()
            .contains("verify-recovery-acceptance")
    );
    std::fs::create_dir_all(&directory).unwrap();
    assert!(
        !directory.join("fixture.json").exists(),
        "never reuse a previous live execution"
    );
    let conn = rusqlite::Connection::open(directory.join("kanban.db")).unwrap();
    crate::db::init(&conn).unwrap();
    let mut cli = crate::herdr::Cli::from_env();
    let cwd = if let Ok(profile_id) = std::env::var("KANBAN_RECOVERY_MACHINE") {
        let profile = crate::remote_transport::saved_profiles()
            .unwrap()
            .into_iter()
            .find(|p| p.id == profile_id)
            .unwrap();
        let machine = crate::machines::import_profile(&conn, &profile, "windows").unwrap();
        let machine = crate::remote_install::install(
            &machine,
            std::path::Path::new(&std::env::var("KANBAN_RECOVERY_HELPER").unwrap()),
        )
        .unwrap();
        conn.execute(
            "UPDATE machines SET helper_name=?2,host_identity=?3 WHERE id=?1",
            rusqlite::params![machine.id, machine.helper_name, machine.host_identity],
        )
        .unwrap();
        let command = crate::remote_transport::powershell(&profile.target,
            "$r=Join-Path $env:USERPROFILE ('.herdr-kanban/verify-recovery-acceptance-'+[Guid]::NewGuid().ToString('N')); New-Item -ItemType Directory -Force $r | Out-Null; $r | ConvertTo-Json -Compress").unwrap();
        let root: String =
            serde_json::from_slice(&crate::remote_transport::run(command, vec![], 45).unwrap())
                .unwrap();
        cli.session = machine.session.clone();
        cli.remote = Some(machine);
        root
    } else {
        let cwd = directory.join("work");
        std::fs::create_dir_all(&cwd).unwrap();
        cwd.to_string_lossy().into_owned()
    };
    let cli = Arc::new(cli);
    let db = Arc::new(Mutex::new(conn));
    let projects = crate::projects::ProjectService::new(db.clone(), cli.clone());
    let project = projects
        .create(
            &uuid::Uuid::new_v4().to_string(),
            &format!(
                "kanban-recovery-acceptance-{}",
                &uuid::Uuid::new_v4().to_string()[..8]
            ),
            &cwd,
        )
        .unwrap();
    let task = crate::tasks::create(
        &db.lock().unwrap(),
        &project.id,
        "Recovery acceptance",
        TaskPriority::Normal,
        "{}",
    )
    .unwrap();
    let app = Arc::new(Workflow {
        db: db.clone(),
        herdr: cli.clone(),
    });
    let operation = uuid::Uuid::new_v4().to_string();
    let prompt = format!(
        "Reply exactly RECOVERY-ACCEPTANCE-{}. Do not use tools or change any files.",
        uuid::Uuid::new_v4()
    );
    let start = app.start_task(&task.id, &prompt, &operation);
    let mut startup_readiness_fallback = false;
    if start
        .as_ref()
        .err()
        .is_some_and(|error| error == "flow.not_idle")
    {
        // Some real environments outlive the existing startup readiness window.
        // Exercise recovery via a separate, explicit test-owned manual dispatch;
        // never retry a prompt, an Enter or agent.start after an uncertain result.
        let count: usize = db
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM dispatch_operations WHERE task_id=?1",
                [&task.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "a dispatch must never be replayed");
        let original = app.binding(&task.id).unwrap().target.unwrap();
        let deadline = Instant::now() + Duration::from_secs(90);
        let live = loop {
            if let Some(live) = cli
                .targets(original.workspace_id.as_str())
                .unwrap()
                .into_iter()
                .find(|t| {
                    t.pane_id == original.pane_id
                        && t.terminal_id == original.terminal_id
                        && t.agent == "codex"
                        && matches!(t.status.as_str(), "idle" | "done")
                })
            {
                break live;
            }
            assert!(Instant::now() < deadline, "test Codex never became ready");
            std::thread::sleep(Duration::from_millis(500));
        };
        app.end_flow(&operation, "failed", "flow.not_idle").unwrap();
        app.repair(&task.id, &live.pane_id).unwrap();
        app.dispatch(
            &task.id,
            &live.pane_id,
            &prompt,
            &format!("{operation}:dispatch"),
        )
        .unwrap();
        db.lock()
            .unwrap()
            .execute(
                "UPDATE lifecycle_operations SET target_json=?2 WHERE id=?1",
                rusqlite::params![operation, serde_json::to_string(&live).unwrap()],
            )
            .unwrap();
        startup_readiness_fallback = true;
    } else {
        start.unwrap();
    }
    let target = app.target(&task.id, true).unwrap();
    let created: String = db
        .lock()
        .unwrap()
        .query_row(
            "SELECT created_at FROM dispatch_operations WHERE id=?1",
            [format!("{operation}:dispatch")],
            |r| r.get(0),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    let receipt = loop {
        if let Some(receipt) = cli
            .prompt_receipt(&target, &prompt, &created, None)
            .unwrap()
        {
            break receipt;
        }
        assert!(Instant::now() < deadline, "no exact real receipt");
        std::thread::sleep(Duration::from_millis(500));
    };
    {
        let db = db.lock().unwrap();
        let tx = db.unchecked_transaction().unwrap();
        tx.execute(
            "UPDATE dispatch_operations SET state='unknown' WHERE id=?1",
            [format!("{operation}:dispatch")],
        )
        .unwrap();
        tx.execute("UPDATE lifecycle_operations SET state='unknown',error='dispatch.submit_verify_required' WHERE id=?1", [&operation]).unwrap();
        tx.execute("UPDATE tasks SET status='todo' WHERE id=?1", [&task.id])
            .unwrap();
        tx.execute(
            "UPDATE task_bindings SET state='reserved' WHERE task_id=?1",
            [&task.id],
        )
        .unwrap();
        tx.commit().unwrap();
    }
    assert!(app.view_task(&task.id).unwrap().read_only);
    let recovery = crate::task_recovery::Recovery::new(app.clone(), Arc::new(|_| {}));
    for worker in recovery.poll(0).unwrap() {
        worker.join().unwrap();
    }
    assert_eq!(app.binding(&task.id).unwrap().state, "valid");
    assert_eq!(
        crate::lifecycle::view(&db.lock().unwrap(), &task.id)
            .unwrap()
            .state
            .as_deref(),
        Some("succeeded")
    );
    assert!(!app.view_task(&task.id).unwrap().read_only);
    let saved: String = db
        .lock()
        .unwrap()
        .query_row(
            "SELECT codex_session_id FROM task_sessions WHERE task_id=?1",
            [&task.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(saved, receipt.session_id);
    let count: usize = db
        .lock()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM dispatch_operations WHERE task_id=?1",
            [&task.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    let desktop = directory.join("desktop");
    std::fs::create_dir_all(&desktop).unwrap();
    {
        let db = db.lock().unwrap();
        db.execute(
            "VACUUM INTO ?1",
            [desktop.join("kanban.db").to_str().unwrap()],
        )
        .unwrap();
    }
    let copy = rusqlite::Connection::open(desktop.join("kanban.db")).unwrap();
    copy.execute(
        "UPDATE dispatch_operations SET state='unknown',created_at=?2 WHERE id=?1",
        rusqlite::params![
            format!("{operation}:dispatch"),
            (chrono::Utc::now() + chrono::Duration::minutes(30)).to_rfc3339()
        ],
    )
    .unwrap();
    copy.execute("UPDATE lifecycle_operations SET state='unknown',error='dispatch.submit_verify_required' WHERE id=?1", [&operation]).unwrap();
    copy.execute("UPDATE tasks SET status='todo' WHERE id=?1", [&task.id])
        .unwrap();
    copy.execute(
        "UPDATE task_bindings SET state='reserved' WHERE task_id=?1",
        [&task.id],
    )
    .unwrap();
    copy.execute("INSERT INTO start_requests(operation_id,project_id,workspace,cwd,prompt_json,needs_prompt,original_status,stage)
        VALUES(?1,?2,?3,?4,'{}',1,'todo','dispatching')", rusqlite::params![operation,project.id,project.workspace_id,cwd]).unwrap();
    let fixture = serde_json::json!({"task":task.id,"project":project.id,"workspace":project.workspace_id,"machine":cli.machine_id(),
        "operation":operation,"target":target,"created_at":created,"session_id":saved,"dispatch_count":count,
        "real_herdr_and_codex":true,"startup_readiness_fallback":startup_readiness_fallback,"reclassified_unknown_in_isolated_database":true,"automatic_recovery":true,
        "read_only_authorization":true,"desktop_receipt_delayed_by_future_boundary":true});
    std::fs::write(
        directory.join("fixture.json"),
        serde_json::to_vec_pretty(&fixture).unwrap(),
    )
    .unwrap();
    println!("{}", serde_json::to_string(&fixture).unwrap());
}
