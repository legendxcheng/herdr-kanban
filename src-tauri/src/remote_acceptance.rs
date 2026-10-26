//! Opt-in integration acceptance against a dedicated directory/workspace on Windows SSH.
use crate::{
    models::TaskPriority,
    workflow::{Herdr, Workflow},
};
use std::sync::{Arc, Mutex};

#[test]
#[ignore = "creates an isolated devpc workspace and runs Codex; requires explicit acceptance environment"]
fn live_remote_task_images_references_complete_and_resume() {
    let profile_id = std::env::var("KANBAN_ACCEPTANCE_MACHINE").expect("explicit profile");
    let executable = std::env::var("KANBAN_ACCEPTANCE_HELPER").expect("built helper");
    let data = std::env::var("KANBAN_DATA_DIR").expect("isolated database");
    assert!(data.contains("ssh-multi-machine") && data.contains("acceptance"));
    std::fs::create_dir_all(&data).unwrap();
    let conn = rusqlite::Connection::open(crate::db::db_path()).unwrap();
    crate::db::init(&conn).unwrap();
    let profile = crate::remote_transport::saved_profiles()
        .unwrap()
        .into_iter()
        .find(|p| p.id == profile_id)
        .unwrap();
    let machine = crate::machines::import_profile(&conn, &profile, "windows").unwrap();
    let machine =
        crate::remote_install::install(&machine, std::path::Path::new(&executable)).unwrap();
    conn.execute(
        "UPDATE machines SET helper_name=?2,host_identity=?3 WHERE id=?1",
        rusqlite::params![machine.id, machine.helper_name, machine.host_identity],
    )
    .unwrap();
    std::fs::write(
        std::path::Path::new(&data).join("machine.json"),
        serde_json::to_vec_pretty(&machine).unwrap(),
    )
    .unwrap();
    let mut cli = crate::herdr::Cli::from_env();
    cli.session = machine.session.clone();
    cli.remote = Some(machine.clone());
    let cli = Arc::new(cli);
    let script = r#"$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$root=Join-Path $env:USERPROFILE ('.herdr-kanban/acceptance-'+[Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force (Join-Path $root '.agents/skills/kanban-acceptance') | Out-Null
[IO.File]::WriteAllText((Join-Path $root 'reference.txt'),'REMOTE-REFERENCE-7c92',[Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText((Join-Path $root '.agents/skills/kanban-acceptance/SKILL.md'),"---`nname: kanban-acceptance`ndescription: Isolated Kanban acceptance`n---`nWrite exactly SKILL-READ in the requested output file along with the reference marker and image color.",[Text.UTF8Encoding]::new($false))
& git -C $root init --quiet
if ($LASTEXITCODE -ne 0) { throw 'git init failed' }
@{root=$root} | ConvertTo-Json -Compress"#;
    let bytes = crate::remote_transport::run(
        crate::remote_transport::powershell(&profile.target, script).unwrap(),
        Vec::new(),
        30,
    )
    .unwrap();
    let setup: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let root = setup["root"].as_str().unwrap();
    let db = Arc::new(Mutex::new(conn));
    let projects = crate::projects::ProjectService::new(db.clone(), cli.clone());
    let project = projects
        .create(
            &uuid::Uuid::new_v4().to_string(),
            "kanban-acceptance-ssh",
            root,
        )
        .unwrap();
    assert_eq!(project.machine_id, machine.id);
    projects
        .update(&project.id, "kanban-acceptance-ssh-renamed", None)
        .unwrap();
    let files = cli.resources().search_files(root, "reference").unwrap();
    assert_eq!(files.len(), 1);
    let skills = cli
        .resources()
        .search_skills(Some(root), "kanban-acceptance")
        .unwrap();
    assert!(skills.iter().any(|s| s.name == "kanban-acceptance"));
    let statuses = cli
        .resources()
        .git_statuses(vec![(project.id.clone(), root.into())])
        .unwrap();
    assert_eq!(
        statuses[0].status.state,
        crate::project_git::GitState::Branch
    );
    let task = {
        use base64::Engine;
        let db = db.lock().unwrap();
        let draft = crate::images::open_draft(&db, &project.id, None).unwrap();
        let mut image = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            100,
            100,
            image::Rgb([255, 0, 0]),
        ))
        .write_to(&mut image, image::ImageFormat::Png)
        .unwrap();
        let image = crate::images::import_image(
            &db,
            &draft,
            "red.png",
            "image/png",
            &base64::engine::general_purpose::STANDARD.encode(image.into_inner()),
        )
        .unwrap();
        let doc=serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Acceptance only: read the attached image using view_image, follow the named skill, and write acceptance-result.txt in the current directory containing the reference marker, SKILL-READ, and the image color. Do not edit any other files. "},{"type":"mention","attrs":{"kind":"file","label":"reference.txt","ref":files[0].abs_path}},{"type":"mention","attrs":{"kind":"skill","label":"kanban-acceptance","ref":skills.iter().find(|s|s.name=="kanban-acceptance").unwrap().skill_md_path}}]},{"type":"promptImage","attrs":{"imageId":image.id}}]}).to_string();
        let task = crate::images::save_task(
            &db,
            None,
            &project.id,
            "SSH acceptance",
            &doc,
            TaskPriority::Normal,
            Some(&draft),
        )
        .unwrap();
        crate::images::close_draft(&db, &draft).unwrap();
        task
    };
    let app = Arc::new(Workflow {
        db: db.clone(),
        herdr: cli.clone(),
    });
    std::fs::write(std::path::Path::new(&data).join("fixture.json"),serde_json::to_vec_pretty(&serde_json::json!({"task":task.id,"project":project.id,"workspace":project.workspace_id,"root":root})).unwrap()).unwrap();
    exercise(app, cli, &task.id, root);
}

fn exercise(app: Arc<Workflow>, cli: Arc<crate::herdr::Cli>, task: &str, root: &str) {
    let db = app.db.clone();
    app.submit_start(
        task,
        &uuid::Uuid::new_v4().to_string(),
        Arc::new(|event| println!("start stage changed {}", event.operation_id)),
    )
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    loop {
        let flow = crate::lifecycle::view(&db.lock().unwrap(), task).unwrap();
        if flow.state.as_deref() == Some("succeeded") {
            break;
        }
        assert!(
            !matches!(flow.state.as_deref(), Some("failed" | "unknown")),
            "start failed: {} {:?}",
            flow.error,
            flow.stage
        );
        assert!(std::time::Instant::now() < deadline, "start timed out");
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    let result_path = format!("{root}/acceptance-result.txt");
    loop {
        if let Ok(text) = cli.resources().read_text(&result_path) {
            if text.contains("REMOTE-REFERENCE-7c92")
                && text.contains("SKILL-READ")
                && (text.to_lowercase().contains("red") || text.contains("红"))
            {
                println!("remote file+skill+image result: {text}");
                break;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "remote agent result timed out"
        );
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    let target = app.target(task, true).unwrap();
    let session = cli.session_id(&target).unwrap().unwrap();
    app.complete_task(task, &uuid::Uuid::new_v4().to_string())
        .unwrap();
    let count: i64 = db
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM dispatch_operations WHERE task_id=?1",
            [task],
            |r| r.get(0),
        )
        .unwrap();
    app.start_task(task, "MUST NOT RESEND", &uuid::Uuid::new_v4().to_string())
        .unwrap();
    let resumed = app.target(task, true).unwrap();
    assert_eq!(
        cli.session_id(&resumed).unwrap().as_deref(),
        Some(session.as_str())
    );
    let after: i64 = db
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM dispatch_operations WHERE task_id=?1",
            [task],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(after, count);
    println!(
        "complete/exact resume passed; task {} pane {} session {}",
        task, resumed.pane_id, session
    );
}

#[test]
#[ignore = "continues the explicitly selected isolated fixture after resolving first-run UI"]
fn live_continue_prepared_remote_acceptance() {
    let data = std::env::var("KANBAN_DATA_DIR").unwrap();
    assert!(data.contains("ssh-multi-machine") && data.contains("acceptance"));
    let fixture: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::path::Path::new(&data).join("fixture.json")).unwrap(),
    )
    .unwrap();
    let machine: crate::machines::Machine = serde_json::from_slice(
        &std::fs::read(std::path::Path::new(&data).join("machine.json")).unwrap(),
    )
    .unwrap();
    let db = Arc::new(Mutex::new(
        rusqlite::Connection::open(crate::db::db_path()).unwrap(),
    ));
    crate::db::init(&db.lock().unwrap()).unwrap();
    let mut cli = crate::herdr::Cli::from_env();
    cli.session = machine.session.clone();
    cli.remote = Some(machine);
    let cli = Arc::new(cli);
    let app = Arc::new(Workflow {
        db: db.clone(),
        herdr: cli.clone(),
    });
    let task = fixture["task"].as_str().unwrap();
    let root = fixture["root"].as_str().unwrap();
    assert!(root.contains(".herdr-kanban") && root.contains("acceptance-"));
    let flow = crate::lifecycle::view(&db.lock().unwrap(), task).unwrap();
    if flow.state.as_deref() == Some("unknown") {
        let count: i64 = db
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM dispatch_operations WHERE task_id=?1",
                [task],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "never close or retry an ambiguous dispatch");
        let target = app.binding(task).unwrap().target.unwrap();
        assert_eq!(target.workspace_id, fixture["workspace"].as_str().unwrap());
        if cli.pane_exists(&target).unwrap() {
            cli.request(&["pane", "close", &target.pane_id])
                .map_err(|e| e.message)
                .unwrap();
        }
        app.recover_lifecycle(task, flow.operation_id.as_deref().unwrap())
            .unwrap();
    }
    exercise(app, cli, task, root);
}

#[test]
#[ignore = "repairs only an isolated fixture resume after a verified environment fix; never resends"]
fn live_remote_exact_resume_after_environment_repair() {
    let data = std::env::var("KANBAN_DATA_DIR").unwrap();
    assert!(data.contains("ssh-multi-machine") && data.contains("acceptance"));
    let fixture: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::path::Path::new(&data).join("fixture.json")).unwrap(),
    )
    .unwrap();
    let conn = rusqlite::Connection::open(crate::db::db_path()).unwrap();
    crate::db::init(&conn).unwrap();
    let task = fixture["task"].as_str().unwrap();
    let id = crate::machines::project_machine(&conn, fixture["project"].as_str().unwrap()).unwrap();
    let machine = crate::machines::get(&conn, &id).unwrap();
    let machine = crate::remote_install::install(
        &machine,
        std::path::Path::new(&std::env::var("KANBAN_ACCEPTANCE_HELPER").unwrap()),
    )
    .unwrap();
    conn.execute(
        "UPDATE machines SET helper_name=?2,host_identity=?3 WHERE id=?1",
        rusqlite::params![id, machine.helper_name, machine.host_identity],
    )
    .unwrap();
    std::fs::write(
        std::path::Path::new(&data).join("machine.json"),
        serde_json::to_vec_pretty(&machine).unwrap(),
    )
    .unwrap();
    let expected: String = conn
        .query_row(
            "SELECT codex_session_id FROM task_sessions WHERE task_id=?1",
            [task],
            |r| r.get(0),
        )
        .unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM dispatch_operations WHERE task_id=?1",
            [task],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    let db = Arc::new(Mutex::new(conn));
    let mut cli = crate::herdr::Cli::from_env();
    cli.session = machine.session.clone();
    cli.remote = Some(machine);
    let cli = Arc::new(cli);
    let app = Workflow {
        db: db.clone(),
        herdr: cli.clone(),
    };
    let flow = crate::lifecycle::view(&db.lock().unwrap(), task).unwrap();
    assert_eq!(flow.state.as_deref(), Some("unknown"));
    assert_eq!(flow.kind.as_deref(), Some("start"));
    let target = app.binding(task).unwrap().target.unwrap();
    assert_eq!(target.workspace_id, fixture["workspace"].as_str().unwrap());
    assert!(fixture["root"].as_str().unwrap().contains(".herdr-kanban"));
    if cli.pane_exists(&target).unwrap() {
        cli.request(&["pane", "close", &target.pane_id])
            .map_err(|e| e.message)
            .unwrap();
    }
    app.recover_lifecycle(task, flow.operation_id.as_deref().unwrap())
        .unwrap();
    app.start_task(task, "MUST NOT RESEND", &uuid::Uuid::new_v4().to_string())
        .unwrap();
    let target = app.target(task, true).unwrap();
    assert_eq!(
        cli.session_id(&target).unwrap().as_deref(),
        Some(expected.as_str())
    );
    let after: i64 = db
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM dispatch_operations WHERE task_id=?1",
            [task],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(after, count);
    println!(
        "verified exact remote resume: {} pane {}, dispatch count remains {}",
        expected, target.pane_id, after
    );
}
