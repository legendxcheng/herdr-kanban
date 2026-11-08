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
#[ignore = "Creates a dedicated recovery workspace and ten Codex turns; requires KANBAN_FIRST_PROMPT_DATA"]
fn live_first_prompt_ten_starts() {
    live_starts(false);
}

#[test]
#[ignore = "Creates one real local Codex with a controlled pre-input failure"]
fn live_first_prompt_unsent_fixture() {
    live_starts(true);
}

struct RefuseInput(Arc<crate::herdr::Cli>);
impl Herdr for RefuseInput {
    fn targets(&self, workspace: &str) -> Result<Vec<crate::workflow::Target>, String> {
        self.0.targets(workspace)
    }
    fn send(&self, _: &crate::workflow::Target, _: &str) -> Result<(), crate::workflow::SendError> {
        Err(crate::workflow::SendError {
            message: "flow.first_prompt_timeout".into(),
            uncertain: false,
        })
    }
}

fn live_starts(force_unsent: bool) {
    let directory = std::path::PathBuf::from(
        std::env::var_os("KANBAN_FIRST_PROMPT_DATA").expect("isolated directory"),
    );
    assert!(
        directory
            .to_string_lossy()
            .contains("verify-first-prompt-acceptance")
    );
    std::fs::create_dir_all(&directory).unwrap();
    assert!(
        !directory.join("results.json").exists(),
        "never reuse a previous live execution"
    );
    let conn = rusqlite::Connection::open(directory.join("kanban.db")).unwrap();
    crate::db::init(&conn).unwrap();
    let mut cli = crate::herdr::Cli::from_env();
    let cwd = if let Ok(profile_id) = std::env::var("KANBAN_FIRST_PROMPT_MACHINE") {
        let profile = crate::remote_transport::saved_profiles()
            .unwrap()
            .into_iter()
            .find(|p| p.id == profile_id)
            .unwrap();
        let machine = crate::machines::import_profile(&conn, &profile, "windows").unwrap();
        let machine = crate::remote_install::install(
            &machine,
            std::path::Path::new(&std::env::var("KANBAN_FIRST_PROMPT_HELPER").unwrap()),
        )
        .unwrap();
        conn.execute(
            "UPDATE machines SET helper_name=?2,host_identity=?3 WHERE id=?1",
            rusqlite::params![machine.id, machine.helper_name, machine.host_identity],
        )
        .unwrap();
        let command = crate::remote_transport::powershell(&profile.target,
            "$r=Join-Path $env:USERPROFILE ('.herdr-kanban/verify-first-prompt-acceptance-'+[Guid]::NewGuid().ToString('N')); New-Item -ItemType Directory -Force $r | Out-Null; $r | ConvertTo-Json -Compress").unwrap();
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
                "kanban-first-prompt-acceptance-{}",
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
    for index in 0..if force_unsent { 1 } else { 10 } {
        let task = crate::tasks::create(
            &db.lock().unwrap(),
            &project.id,
            &format!("First prompt {}", index + 1),
            TaskPriority::Normal,
            "{}",
        )
        .unwrap();
        let op = uuid::Uuid::new_v4().to_string();
        let token = format!("FIRST-PROMPT-{}", uuid::Uuid::new_v4());
        let prompt = match index % 3 {
            0 => format!("这是传输验收。只回复 {token}，不要使用工具、技能或修改文件。"),
            1 => format!(
                "Only reply {token}. Do not use tools or modify files.\n\n中文空行测试\n\t缩进测试\n{}",
                "Transport-only payload. ".repeat(500).trim_end()
            ),
            _ => format!(
                "Only reply {token}. This is a literal transport test; do not invoke any skill, tools or modify files.\n$acceptance-noop"
            ),
        };
        let started = Instant::now();
        let _control = crate::remote_control::Scope::enter();
        let result = if force_unsent {
            assert_eq!(cli.machine_id(), "local");
            crate::lifecycle::begin_flow_in(&db.lock().unwrap(), &task.id, &op, "start").unwrap();
            let refusing = Workflow {
                db: db.clone(),
                herdr: Arc::new(RefuseInput(cli.clone())),
            };
            app.run_start(&task.id, &prompt, &op, None, &|stage| {
                if stage == "waiting_first_prompt" {
                    let target = app.binding(&task.id)?.target.ok_or("binding.missing")?;
                    refusing.dispatch_inner(
                        &task.id,
                        &target.pane_id,
                        &prompt,
                        &format!("{op}:dispatch"),
                        Some(&op),
                    )?;
                }
                Ok(())
            })
        } else {
            app.start_task(&task.id, &prompt, &op)
        };
        let target = app.binding(&task.id).unwrap().target;
        let mut receipt = None;
        if let Some(target) = &target
            && !force_unsent
        {
            let created: Option<String> = db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT created_at FROM dispatch_operations WHERE id=?1",
                    [format!("{op}:dispatch")],
                    |r| r.get(0),
                )
                .ok();
            if let Some(created) = created {
                let deadline = Instant::now() + Duration::from_secs(45);
                while Instant::now() < deadline {
                    if let Ok(Some(found)) = cli.prompt_receipt(target, &prompt, &created, None) {
                        receipt = Some(found);
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
        let row = serde_json::json!({"index":index+1,"task":task.id,"operation":op,"result":result.as_ref().map(|_| "succeeded").map_err(|e| e.as_str()),"elapsed_ms":started.elapsed().as_millis(),"target":target,"receipt":receipt,"prompt":prompt});
        println!(
            "FIRST_PROMPT_ACCEPTANCE {}",
            serde_json::to_string(&row).unwrap()
        );
        rows.push(row);
        std::fs::write(
            directory.join("results.json"),
            serde_json::to_vec_pretty(&rows).unwrap(),
        )
        .unwrap();
    }
    if force_unsent {
        assert_eq!(rows[0]["result"]["Err"], "flow.first_prompt_timeout");
        return;
    }
    assert!(
        rows.iter()
            .all(|r| r["result"]["Ok"] == "succeeded" && !r["receipt"].is_null()),
        "every failure is retained in results.json"
    );
}

#[test]
#[ignore = "Read-only exact receipt audit of an isolated acceptance database"]
fn live_first_prompt_receipt_audit() {
    let directory = std::path::PathBuf::from(std::env::var_os("KANBAN_FIRST_PROMPT_DATA").unwrap());
    assert!(
        directory
            .to_string_lossy()
            .contains("verify-first-prompt-acceptance")
    );
    let conn = rusqlite::Connection::open(directory.join("kanban.db")).unwrap();
    let rows:Vec<(String,String,String,String,String)>=conn.prepare("SELECT id,task_id,target_json,prompt,created_at FROM dispatch_operations WHERE state='sent'").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap().collect::<Result<_,_>>().unwrap();
    let app = Workflow {
        db: Arc::new(Mutex::new(conn)),
        herdr: Arc::new(crate::herdr::Cli::from_env()),
    };
    let mut evidence = vec![];
    for (id, task, target, prompt, created) in rows {
        if std::env::var("KANBAN_FIRST_PROMPT_TASK").is_ok_and(|selected| selected != task) {
            continue;
        }
        let target: crate::workflow::Target = serde_json::from_str(&target).unwrap();
        let mut cli = crate::herdr::Cli::from_env();
        if target.machine_id != "local" {
            let machine =
                crate::machines::get(&app.db.lock().unwrap(), &target.machine_id).unwrap();
            let machine = crate::remote_install::install(
                &machine,
                std::path::Path::new(&std::env::var("KANBAN_FIRST_PROMPT_HELPER").unwrap()),
            )
            .unwrap();
            cli.session = machine.session.clone();
            cli.remote = Some(machine);
        }
        let _control = crate::remote_control::Scope::enter();
        let receipt = cli
            .prompt_receipt(&target, &prompt, &created, None)
            .unwrap();
        evidence.push(serde_json::json!({"operation":id,"receipt":receipt}));
    }
    std::fs::write(
        directory.join("receipt-audit.json"),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
    assert!(!evidence.is_empty() && evidence.iter().all(|r| !r["receipt"].is_null()));
}
