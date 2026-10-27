//! Opt-in real Workflow/SQLite/Herdr fixture; all resources belong to this test.
use crate::{
    models::TaskPriority,
    workflow::{Herdr, Workflow},
};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[test]
#[ignore = "creates dedicated local/SSH Workspace; requires explicit isolated data directory"]
fn live_idle_cleanup_preserves_inprogress_and_paused_shells() {
    let data = std::env::var("KANBAN_DATA_DIR").unwrap();
    assert!(data.contains("issue41-acceptance"));
    let path = std::path::Path::new(&data);
    std::fs::create_dir_all(path).unwrap();
    let cwd = std::env::var("KANBAN_ACCEPTANCE_CWD").unwrap();
    let conn = rusqlite::Connection::open(path.join("kanban.db")).unwrap();
    crate::db::init(&conn).unwrap();
    let mut cli = crate::herdr::Cli::from_env();
    if let Ok(profile_id) = std::env::var("KANBAN_ACCEPTANCE_MACHINE") {
        let profile = crate::remote_transport::saved_profiles()
            .unwrap()
            .into_iter()
            .find(|p| p.id == profile_id)
            .unwrap();
        let machine = crate::machines::import_profile(&conn, &profile, "windows").unwrap();
        let helper = std::env::var("KANBAN_ACCEPTANCE_HELPER").unwrap();
        let machine =
            crate::remote_install::install(&machine, std::path::Path::new(&helper)).unwrap();
        conn.execute(
            "UPDATE machines SET helper_name=?2,host_identity=?3 WHERE id=?1",
            rusqlite::params![machine.id, machine.helper_name, machine.host_identity],
        )
        .unwrap();
        cli.session = machine.session.clone();
        cli.remote = Some(machine);
    }
    let cli = Arc::new(cli);
    let db = Arc::new(Mutex::new(conn));
    let app = Workflow {
        db: db.clone(),
        herdr: cli.clone(),
    };
    let service = crate::projects::ProjectService::new(db.clone(), cli.clone());
    let project = service
        .create(
            &uuid::Uuid::new_v4().to_string(),
            &format!(
                "kanban-acceptance-issue41-{}",
                &uuid::Uuid::new_v4().to_string()[..8]
            ),
            &cwd,
        )
        .unwrap();
    let workspace = project.workspace_id.as_deref().unwrap();
    std::fs::write(
        path.join("created-workspace.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"project":project.id,"workspace":workspace,"name":project.name}),
        )
        .unwrap(),
    )
    .unwrap();
    let _scope = crate::remote_control::Scope::enter();
    let original = cli.pane_inventory(workspace).unwrap().unwrap();
    let shell = cli
        .create_pane(workspace, &cwd, "shell", &|| Ok(()))
        .unwrap();
    let paused = cli
        .create_pane(workspace, &cwd, "pause", &|| Ok(()))
        .unwrap();
    cli.request(&["pane", "send-text", &paused.pane_id, "cmd.exe /c pause"])
        .map_err(|e| e.message)
        .unwrap();
    cli.output(&["pane", "send-keys", &paused.pane_id, "enter"])
        .map_err(|e| e.message)
        .unwrap();
    let in_process = cli
        .create_pane(workspace, &cwd, "in-process-command", &|| Ok(()))
        .unwrap();
    cli.request(&[
        "pane",
        "send-text",
        &in_process.pane_id,
        "Start-Sleep -Seconds 180",
    ])
    .map_err(|e| e.message)
    .unwrap();
    cli.output(&["pane", "send-keys", &in_process.pane_id, "enter"])
        .map_err(|e| e.message)
        .unwrap();
    let orphan = cli
        .create_pane(workspace, &cwd, "orphan", &|| Ok(()))
        .unwrap();
    let orphan = cli
        .prepare_codex(
            &orphan,
            &cwd,
            None,
            "kb-idle-cleanup-acceptance",
            &|| Ok(()),
        )
        .unwrap();
    let mut task_ids = vec![];
    let mut targets = vec![];
    for name in ["Protected idle A", "Protected idle B"] {
        let task = {
            let conn = db.lock().unwrap();
            let task =
                crate::tasks::create(&conn, &project.id, name, TaskPriority::Normal, "{}").unwrap();
            crate::codex_settings::save(
                &conn,
                &task.id,
                &crate::codex_settings::Settings {
                    model: Some("gpt-6-luna".into()),
                    effort: Some("low".into()),
                },
                false,
            )
            .unwrap();
            task
        };
        app.start_task(
            &task.id,
            "Reply exactly CLEANUP-ACCEPTANCE. Do not use tools or change files.",
            &uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
        let target = app.target(&task.id, true).unwrap();
        assert_ne!(target.terminal_id, shell.terminal_id);
        assert_ne!(target.terminal_id, orphan.terminal_id);
        task_ids.push(task.id);
        targets.push(target);
    }
    assert_ne!(targets[0].terminal_id, targets[1].terminal_id);
    let deadline = Instant::now() + Duration::from_secs(150);
    loop {
        let idle = cli.idle_panes(workspace).unwrap();
        if targets
            .iter()
            .all(|t| idle.iter().any(|p| p.target.same_identity(t)))
        {
            break;
        }
        assert!(Instant::now() < deadline, "test tasks did not become idle");
        std::thread::sleep(Duration::from_millis(500));
    }
    let preview = app.preview_idle_cleanup(&project.id).unwrap();
    assert_eq!(
        preview.count, 1,
        "only unbound idle Codex is eligible; shells lack command-completion evidence"
    );
    let report = app.execute_idle_cleanup(&project.id, &preview.id).unwrap();
    assert_eq!((report.closed, report.skipped, report.failed), (1, 0, 0));
    let remaining = cli.pane_inventory(workspace).unwrap().unwrap();
    for t in &targets {
        assert!(remaining.iter().any(|p| p.terminal_id == t.terminal_id));
    }
    assert!(
        remaining
            .iter()
            .any(|p| p.terminal_id == paused.terminal_id)
    );
    assert!(original.iter().all(|old| remaining.contains(old)));
    for preserved in [&shell, &in_process] {
        assert!(
            remaining
                .iter()
                .any(|p| p.terminal_id == preserved.terminal_id)
        );
    }
    // Leave one eligible pane for the real desktop confirmation/cancel test.
    let desktop_shell = cli
        .create_pane(workspace, &cwd, "desktop", &|| Ok(()))
        .unwrap();
    let desktop_shell = cli
        .prepare_codex(&desktop_shell, &cwd, None, "kb-cleanup-desktop", &|| Ok(()))
        .unwrap();
    db.lock()
        .unwrap()
        .execute(
            "INSERT OR REPLACE INTO project_visibility(project_id,visible) VALUES(?1,1)",
            [&project.id],
        )
        .unwrap();
    db.lock()
        .unwrap()
        .execute("UPDATE navigation_preferences SET configured=1", [])
        .unwrap();
    let fixture = serde_json::json!({"project":project.id,"name":project.name,"workspace":workspace,"machine":cli.machine_id(),"tasks":task_ids,"targets":targets,"paused":paused,"in_process":in_process,"shell":shell,"desktop_shell":desktop_shell,"report":report});
    std::fs::write(
        path.join("fixture.json"),
        serde_json::to_vec_pretty(&fixture).unwrap(),
    )
    .unwrap();
    println!(
        "ISSUE41 {}: 2 fresh task panes, 1 idle closure, protected tasks/shell/pause/in-process-command/other Tab survive; fixture {}",
        cli.machine_id(),
        data
    );
}
