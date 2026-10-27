//! Explicit opt-in acceptance; uses its own database and newly created workspace.
use crate::{
    codex_settings::Settings,
    models::TaskPriority,
    workflow::{Herdr, Workflow},
};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[test]
#[ignore = "creates a dedicated workspace and consumes a small real Codex turn; explicit environment required"]
fn live_model_start_and_exact_resume() {
    let data = std::env::var("KANBAN_DATA_DIR").expect("isolated database");
    assert!(data.contains("model-acceptance"));
    let cwd = std::env::var("KANBAN_MODEL_CWD").expect("explicit trusted test directory");
    let path = std::path::Path::new(&data);
    std::fs::create_dir_all(path).unwrap();
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
    let projects = crate::projects::ProjectService::new(db.clone(), cli.clone());
    let existing: Option<serde_json::Value> = std::fs::read(path.join("fixture.json"))
        .ok()
        .map(|bytes| serde_json::from_slice(&bytes).unwrap());
    let project = match &existing {
        Some(f) => projects.get(f["project"].as_str().unwrap()).unwrap(),
        None => projects
            .create(
                &uuid::Uuid::new_v4().to_string(),
                &format!(
                    "kanban-acceptance-model-{}",
                    &uuid::Uuid::new_v4().to_string()[..8]
                ),
                &cwd,
            )
            .unwrap(),
    };
    let settings = Settings {
        model: Some("gpt-6-luna".into()),
        effort: Some("high".into()),
    };
    let catalog = cli.codex_catalog(&cwd).unwrap();
    catalog.validate(&settings).unwrap();
    let prompt = "Reply exactly MODEL-CONFIG-ACCEPTANCE. Do not use tools or change files.";
    let task = if let Some(f) = &existing {
        crate::tasks::list(&db.lock().unwrap(), &project.id)
            .unwrap()
            .into_iter()
            .find(|t| Some(t.id.as_str()) == f["task"].as_str())
            .unwrap()
    } else {
        let db = db.lock().unwrap();
        let task = crate::tasks::create(
            &db,
            &project.id,
            "Model acceptance",
            TaskPriority::Normal,
            "{}",
        )
        .unwrap();
        crate::codex_settings::save(&db, &task.id, &settings, true).unwrap();
        task
    };
    let app = Workflow {
        db: db.clone(),
        herdr: cli.clone(),
    };
    std::fs::write(path.join("fixture.json"), serde_json::to_vec_pretty(&serde_json::json!({"task":task.id,"project":project.id,"workspace":project.workspace_id,"cwd":cwd,"settings":settings})).unwrap()).unwrap();
    if existing.is_none() {
        app.start_task(&task.id, prompt, &uuid::Uuid::new_v4().to_string())
            .unwrap();
    }
    let target = app.target(&task.id, true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(150);
    let session = loop {
        let targets = cli
            .targets(project.workspace_id.as_deref().unwrap())
            .unwrap();
        if targets
            .iter()
            .any(|t| t.same_identity(&target) && matches!(t.status.as_str(), "idle" | "done"))
        {
            if let Some(session) = cli.session_id(&target).unwrap() {
                break session;
            }
        }
        assert!(
            Instant::now() < deadline,
            "generation timed out; preserve fixture for inspection"
        );
        std::thread::sleep(Duration::from_millis(500));
    };
    let process = cli
        .output(&["pane", "process-info", "--pane", &target.pane_id])
        .map_err(|e| e.message)
        .unwrap();
    std::fs::write(path.join("started-process.json"), process).unwrap();
    std::fs::write(path.join("session.txt"), &session).unwrap();
    app.complete_task(&task.id, &uuid::Uuid::new_v4().to_string())
        .unwrap();
    app.start_task(
        &task.id,
        "MUST NOT RESEND",
        &uuid::Uuid::new_v4().to_string(),
    )
    .unwrap();
    let resumed = app.target(&task.id, true).unwrap();
    assert_eq!(
        cli.session_id(&resumed).unwrap().as_deref(),
        Some(session.as_str())
    );
    let process = cli
        .output(&["pane", "process-info", "--pane", &resumed.pane_id])
        .map_err(|e| e.message)
        .unwrap();
    std::fs::write(path.join("resumed-process.json"), process).unwrap();
    let count: i64 = db
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM dispatch_operations WHERE task_id=?1",
            [&task.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    app.complete_task(&task.id, &uuid::Uuid::new_v4().to_string())
        .unwrap();
    println!(
        "model acceptance passed: machine={} task={} session={} dispatches={count}",
        cli.machine_id(),
        task.id,
        session
    );
    std::fs::write(path.join("result.json"), serde_json::to_vec_pretty(&serde_json::json!({"session":session,"dispatches":count,"settings":settings,"machine":cli.machine_id(),"passed":true})).unwrap()).unwrap();
}
