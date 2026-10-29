//! Opt-in live acceptance. Only dedicated test workspaces and isolated databases.
use crate::{
    models::TaskPriority,
    workflow::{Herdr, Workflow},
};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[test]
#[ignore = "requires isolated reset-acceptance data and a trusted directory; sends two minimal real Codex prompts"]
fn live_reset_keeps_old_pane_and_starts_a_new_conversation() {
    let data = std::env::var("KANBAN_DATA_DIR").expect("isolated database");
    assert!(data.contains("reset-acceptance"));
    let path = std::path::Path::new(&data);
    std::fs::create_dir_all(path).unwrap();
    assert!(
        !path.join("fixture.json").exists(),
        "use a fresh acceptance directory; do not repeat unknown work"
    );
    let cwd = std::env::var("KANBAN_RESET_CWD").expect("trusted test directory");
    let conn = rusqlite::Connection::open(path.join("kanban.db")).unwrap();
    crate::db::init(&conn).unwrap();
    let mut cli = crate::herdr::Cli::from_env();
    if let Ok(profile_id) = std::env::var("KANBAN_RESET_MACHINE") {
        let profile = crate::remote_transport::saved_profiles()
            .unwrap()
            .into_iter()
            .find(|p| p.id == profile_id)
            .unwrap();
        let machine = crate::machines::import_profile(&conn, &profile, "windows").unwrap();
        let helper = std::env::var("KANBAN_RESET_HELPER").unwrap();
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
    let projects = crate::projects::ProjectService::new(db.clone(), cli.clone());
    let project = projects
        .create(
            &uuid::Uuid::new_v4().to_string(),
            &format!(
                "kanban-acceptance-reset-{}",
                &uuid::Uuid::new_v4().to_string()[..8]
            ),
            &cwd,
        )
        .unwrap();
    let prompt = "Reply exactly RESET-ACCEPTANCE-ONE. Do not use tools or modify files.";
    let document=serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":prompt}]}]}).to_string();
    let task = crate::tasks::create(
        &db.lock().unwrap(),
        &project.id,
        "Reset acceptance",
        TaskPriority::High,
        &document,
    )
    .unwrap();
    let settings = crate::codex_settings::Settings {
        model: Some("gpt-6-luna".into()),
        effort: Some("low".into()),
    };
    crate::codex_settings::save(&db.lock().unwrap(), &task.id, &settings, false).unwrap();
    std::fs::write(path.join("fixture.json"),serde_json::to_vec_pretty(&serde_json::json!({"task":task.id,"project":project.id,"workspace":project.workspace_id,"cwd":cwd,"machine":cli.machine_id()})).unwrap()).unwrap();
    let app = Workflow {
        db: db.clone(),
        herdr: cli.clone(),
    };
    let first = uuid::Uuid::new_v4().to_string();
    app.start_task(&task.id, prompt, &first).unwrap();
    let old = app.target(&task.id, true).unwrap();
    let wait_idle = |target: &crate::workflow::Target| {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let live = cli
                .validate(project.workspace_id.as_deref().unwrap(), target)
                .unwrap();
            if matches!(live.status.as_str(), "idle" | "done") {
                if let Some(session) = cli.session_id(&live).unwrap() {
                    break session;
                }
            }
            assert!(Instant::now() < deadline, "preserve fixture after timeout");
            std::thread::sleep(Duration::from_millis(400));
        }
    };
    let old_session = wait_idle(&old);
    {
        let mut db = db.lock().unwrap();
        // Deliberately inject a lost confirmation while retaining a real live pane.
        // This tests reset behavior, not reproduction of a transport failure.
        db.execute("UPDATE lifecycle_operations SET state='unknown',error='acceptance.injected_confirmation_loss' WHERE id=?1",[&first]).unwrap();
        super::reset(
            &mut db,
            &task.id,
            0,
            Some(&first),
            None,
            &uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
        assert_eq!(
            crate::tasks::list(&db, &project.id).unwrap()[0].prompt_json,
            document
        );
    }
    cli.validate(project.workspace_id.as_deref().unwrap(), &old)
        .unwrap();
    assert!(app.repair(&task.id, &old.pane_id).is_err());
    let revised = "Reply exactly RESET-ACCEPTANCE-TWO. Do not use tools or modify files.";
    let document=serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":revised}]}]}).to_string();
    crate::tasks::update(
        &db.lock().unwrap(),
        &task.id,
        "Reset acceptance",
        &document,
        TaskPriority::High,
    )
    .unwrap();
    app.start_task(&task.id, revised, &uuid::Uuid::new_v4().to_string())
        .unwrap();
    let new = app.target(&task.id, true).unwrap();
    let new_session = wait_idle(&new);
    assert_ne!(old_session, new_session);
    assert_ne!(old.terminal_id, new.terminal_id);
    assert_eq!(
        cli.session_id(&old).unwrap().as_deref(),
        Some(old_session.as_str())
    );
    let count: i64 = db
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM dispatch_operations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2);
    assert!(
        app.complete_task(&task.id, &uuid::Uuid::new_v4().to_string())
            .unwrap()
            .warning
            .is_none()
    );
    // Test cleanup is explicit and limited to the exact old test target.
    cli.validate(project.workspace_id.as_deref().unwrap(), &old)
        .unwrap();
    cli.close_pane(&old, &|| Ok(())).unwrap();
    assert!(!cli.pane_exists(&old).unwrap());
    std::fs::write(path.join("result.json"),serde_json::to_vec_pretty(&serde_json::json!({"passed":true,"machine":cli.machine_id(),"old_session":old_session,"new_session":new_session,"dispatches":count,"old_pane_preserved_until_cleanup":true,"test_panes_closed":true})).unwrap()).unwrap();
}
