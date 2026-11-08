//! Opt-in startup-input acceptance with real local/SSH Herdr and isolated SQLite.
use crate::{
    models::TaskPriority,
    workflow::{Herdr, Workflow},
};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

// Drop only the response after the real local/SSH launch has been issued. Recovery
// uses a fresh Workflow and real CLI, so this cannot manufacture a receipt.
struct LostReply {
    cli: Arc<crate::herdr::Cli>,
    launches: std::sync::atomic::AtomicUsize,
}
impl Herdr for LostReply {
    fn machine_id(&self) -> &str {
        self.cli.machine_id()
    }
    fn session(&self) -> &str {
        self.cli.session()
    }
    fn workspaces(&self) -> Result<Vec<crate::projects::Workspace>, String> {
        self.cli.workspaces()
    }
    fn targets(&self, w: &str) -> Result<Vec<crate::workflow::Target>, String> {
        self.cli.targets(w)
    }
    fn pane_inventory(
        &self,
        w: &str,
    ) -> Result<Option<Vec<crate::workflow::PaneIdentity>>, String> {
        self.cli.pane_inventory(w)
    }
    fn create_pane(
        &self,
        w: &str,
        c: &str,
        n: &str,
        check: &dyn Fn() -> Result<(), String>,
    ) -> Result<crate::workflow::Target, String> {
        self.cli.create_pane(w, c, n, check)
    }
    fn startup_shell(
        &self,
        t: &crate::workflow::Target,
        check: &dyn Fn() -> Result<(), String>,
    ) -> Result<crate::workflow::Target, String> {
        self.cli.startup_shell(t, check)
    }
    fn launch_prompt(
        &self,
        r: &crate::startup_prompt::Request,
        check: &dyn Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        self.launches
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.cli.launch_prompt(r, check)?;
        Err("machine.response_lost".into())
    }
    fn find_prompt_launch(
        &self,
        _: &crate::startup_prompt::Request,
    ) -> Result<Option<crate::workflow::Target>, String> {
        Err("machine.offline".into())
    }
    fn send(&self, _: &crate::workflow::Target, _: &str) -> Result<(), crate::workflow::SendError> {
        panic!("startup must not append terminal input")
    }
}

#[test]
#[ignore = "Uses a completed isolated matrix workspace for one real lost-response/reopen drill"]
fn live_startup_lost_response_reopen() {
    let root = std::path::PathBuf::from(std::env::var_os("KANBAN_STARTUP_DATA").unwrap());
    assert!(root.to_string_lossy().contains("verify-startup-input"));
    assert!(!root.join("lost-response.json").exists());
    let workspace: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("workspace.json")).unwrap()).unwrap();
    let conn = rusqlite::Connection::open(root.join("kanban.db")).unwrap();
    let mut cli = crate::herdr::Cli::from_env();
    if workspace["machine"] != "local" {
        let machine = crate::machines::get(&conn, workspace["machine"].as_str().unwrap()).unwrap();
        cli.session = machine.session.clone();
        cli.remote = Some(machine);
    }
    let cli = Arc::new(cli);
    let lost = Arc::new(LostReply {
        cli: cli.clone(),
        launches: std::sync::atomic::AtomicUsize::new(0),
    });
    let app = Workflow {
        db: Arc::new(Mutex::new(conn)),
        herdr: lost.clone(),
    };
    let task = crate::tasks::create(
        &app.db.lock().unwrap(),
        workspace["project"].as_str().unwrap(),
        "Lost response",
        TaskPriority::Normal,
        "{}",
    )
    .unwrap();
    let op = uuid::Uuid::new_v4().to_string();
    let prompt = format!(
        "Only reply LOST-REPLY-{}. Do not use tools or modify any files.",
        uuid::Uuid::new_v4()
    );
    let _control = crate::remote_control::Scope::enter();
    let error = app.start_task(&task.id, &prompt, &op).unwrap_err();
    assert_eq!(error, "machine.response_lost");
    drop(app);
    let conn = rusqlite::Connection::open(root.join("kanban.db")).unwrap();
    crate::db::recover_interrupted(&conn).unwrap();
    let app = Workflow {
        db: Arc::new(Mutex::new(conn)),
        herdr: cli,
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    let result = loop {
        let result = app.recover_lifecycle(&task.id, &op);
        if result.is_ok() || Instant::now() >= deadline {
            break result;
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    let flow = crate::lifecycle::view(&app.db.lock().unwrap(), &task.id).unwrap();
    let launches = lost.launches.load(std::sync::atomic::Ordering::SeqCst);
    let evidence = serde_json::json!({"task":task.id,"operation":op,"initial_error":error,"result":result,"launches":launches,"flow":flow});
    std::fs::write(
        root.join("lost-response.json"),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
    assert!(result.is_ok());
    assert_eq!(launches, 1);
    assert_eq!(flow.state.as_deref(), Some("succeeded"));
}
#[test]
#[ignore = "Creates a dedicated local or SSH workspace and ten real Codex turns"]
fn live_startup_input_matrix() {
    let directory = std::path::PathBuf::from(
        std::env::var_os("KANBAN_STARTUP_DATA").expect("isolated directory"),
    );
    assert!(directory.to_string_lossy().contains("verify-startup-input"));
    std::fs::create_dir_all(&directory).unwrap();
    assert!(
        !directory.join("results.json").exists(),
        "never reuse a previous live execution"
    );
    let conn = rusqlite::Connection::open(directory.join("kanban.db")).unwrap();
    crate::db::init(&conn).unwrap();
    let mut cli = crate::herdr::Cli::from_env();
    let cwd = if let Ok(profile_id) = std::env::var("KANBAN_STARTUP_MACHINE") {
        let profile = crate::remote_transport::saved_profiles()
            .unwrap()
            .into_iter()
            .find(|p| p.id == profile_id)
            .unwrap();
        let machine = crate::machines::import_profile(&conn, &profile, "windows").unwrap();
        let machine = crate::remote_install::install(
            &machine,
            std::path::Path::new(&std::env::var("KANBAN_STARTUP_HELPER").unwrap()),
        )
        .unwrap();
        conn.execute(
            "UPDATE machines SET helper_name=?2,host_identity=?3 WHERE id=?1",
            rusqlite::params![machine.id, machine.helper_name, machine.host_identity],
        )
        .unwrap();
        let command = crate::remote_transport::powershell(&profile.target,
            "$r=Join-Path $env:USERPROFILE ('.herdr-kanban/verify-startup-input-'+[Guid]::NewGuid().ToString('N')); New-Item -ItemType Directory -Force $r | Out-Null; $r | ConvertTo-Json -Compress").unwrap();
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
                "kanban-startup-input-{}",
                &uuid::Uuid::new_v4().to_string()[..8]
            ),
            &cwd,
        )
        .unwrap();
    let app = Arc::new(Workflow {
        db: db.clone(),
        herdr: cli.clone(),
    });
    let mut rows = Vec::new();
    std::fs::write(directory.join("workspace.json"), serde_json::to_vec_pretty(&serde_json::json!({"project":project.id,"workspace":project.workspace_id,"machine":cli.machine_id(),"cwd":cwd})).unwrap()).unwrap();

    for index in 0..10 {
        let task = crate::tasks::create(
            &db.lock().unwrap(),
            &project.id,
            &format!("Startup {}", index + 1),
            TaskPriority::Normal,
            "{}",
        )
        .unwrap();
        let op = uuid::Uuid::new_v4().to_string();
        let token = format!("STARTUP-{}", uuid::Uuid::new_v4());
        let instruction =
            format!("这是传输验收。只回复 {token}。不要修改文件或执行命令；引用只需原样接收。\n");
        let text = match index {
            1 => format!("{instruction}\n空行\n\t制表符 'quote' \"double\" C:\\path\\ $(literal)"),
            2 => format!("--not-an-option\n{instruction}"),
            3 => format!("{instruction}{}", "Transport payload. ".repeat(650)),
            4 => format!("{instruction}{}", "Transport payload. ".repeat(1450)),
            5 => format!("{instruction}\n$acceptance-noop"),
            _ => instruction,
        };
        let mut content =
            vec![serde_json::json!({"type":"paragraph","content":[{"type":"text","text":text}]})];
        if index == 6 || index == 9 {
            content.push(serde_json::json!({"type":"mention","attrs":{"kind":"file","label":"设计 说明.md","ref":format!("{cwd}/docs/设计 说明.md")}}));
            content.push(serde_json::json!({"type":"mention","attrs":{"kind":"skill","label":"acceptance-noop"}}));
        }
        let draft =
            crate::images::open_draft(&db.lock().unwrap(), &project.id, Some(&task.id)).unwrap();
        if index == 7 || index == 9 {
            let image=crate::images::import_image(&db.lock().unwrap(),&draft,"验收.png","image/png","iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAFElEQVR4nGP4z8DwHwyBNBAw/AcAR8oI+ItOQ4UAAAAASUVORK5CYII=").unwrap();
            content.push(serde_json::json!({"type":"promptImage","attrs":{"imageId":image.id}}));
        }
        let document = serde_json::json!({"type":"doc","content":content}).to_string();
        crate::images::save_task(
            &db.lock().unwrap(),
            Some(&task.id),
            &project.id,
            &task.name,
            &document,
            TaskPriority::Normal,
            Some(&draft),
        )
        .unwrap();
        let started = Instant::now();
        app.submit_start(&task.id, &op, Arc::new(|_| {})).unwrap();
        let deadline = Instant::now() + Duration::from_secs(150);
        let flow = loop {
            let flow = crate::lifecycle::view(&db.lock().unwrap(), &task.id).unwrap();
            if flow.state.as_deref() != Some("pending") || Instant::now() >= deadline {
                break flow;
            }
            std::thread::sleep(Duration::from_millis(200));
        };
        let frozen: Option<(String, String, String)> = db
            .lock()
            .unwrap()
            .query_row(
                "SELECT prompt,created_at,state FROM dispatch_operations WHERE id=?1",
                [format!("{op}:dispatch")],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok();
        let target = app.binding(&task.id).unwrap().target;
        let receipt = if let (Some(target), Some((prompt, created, _))) = (&target, &frozen) {
            cli.prompt_receipt(target, prompt, created, flow.session_id.as_deref())
                .ok()
                .flatten()
        } else {
            None
        };
        use sha2::Digest;
        let row = serde_json::json!({"index":index+1,"task":task.id,"operation":op,"state":flow.state,"error":flow.error,"elapsed_ms":started.elapsed().as_millis(),"target":target,"receipt":receipt,"prompt_bytes":frozen.as_ref().map(|r|r.0.len()),"prompt_sha256":frozen.as_ref().map(|r|format!("{:x}",sha2::Sha256::digest(r.0.as_bytes())))});
        println!("STARTUP_ACCEPTANCE {row}");
        rows.push(row);
        std::fs::write(
            directory.join("results.json"),
            serde_json::to_vec_pretty(&rows).unwrap(),
        )
        .unwrap();
    }
    assert!(
        rows.iter()
            .all(|r| r["state"] == "succeeded" && !r["receipt"].is_null()),
        "all attempts retained"
    );
}
