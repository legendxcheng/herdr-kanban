use super::*;
use crate::herdr::Cli;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

#[test]
#[ignore = "requires explicit SSH machine, isolated kanban-acceptance workspace and fixed-reply skill fixture"]
fn live_ssh_submission_recovers_read_disconnect_without_replaying_input() {
    let machine: crate::machines::Machine = serde_json::from_slice(
        &std::fs::read(std::env::var("KANBAN_ACCEPTANCE_MACHINE_FILE").unwrap()).unwrap(),
    )
    .unwrap();
    let workspace = std::env::var("KANBAN_LIVE_WORKSPACE").unwrap();
    assert!(workspace.starts_with("kanban-acceptance-"));
    let cwd = std::env::var("KANBAN_LIVE_CWD").unwrap();
    assert!(cwd.contains("kanban-acceptance-"));
    let artifacts = std::path::PathBuf::from(std::env::var("KANBAN_LIVE_ARTIFACT_DIR").unwrap());
    crate::remote_transport::verify_profile(&machine).unwrap();
    let mut cli = Cli::from_env();
    cli.session = machine.session.clone();
    cli.remote = Some(machine.clone());
    let spaces = cli.workspaces().unwrap();
    let spaces: Vec<_> = spaces.iter().filter(|w| w.name == workspace).collect();
    assert_eq!(spaces.len(), 1);
    let selected_case = std::env::var("KANBAN_LIVE_CASE")
        .ok()
        .map(|s| s.parse::<usize>().unwrap());
    assert!(selected_case.is_none_or(|index| index < 3));

    for (index, fault) in [None, Some("right"), Some("esc")].into_iter().enumerate() {
        if selected_case.is_some_and(|selected| selected != index) {
            continue;
        }
        let operation = uuid::Uuid::new_v4().to_string();
        let directory = artifacts.join(&operation);
        std::fs::create_dir_all(&directory).unwrap();
        let (mut app, _) = fixture();
        let database = directory.join("kanban.db");
        app.db
            .lock()
            .unwrap()
            .execute("VACUUM INTO ?1", [database.to_str().unwrap()])
            .unwrap();
        app.db = Arc::new(Mutex::new(Connection::open(&database).unwrap()));
        crate::db::init(&app.db.lock().unwrap()).unwrap();
        app.herdr = Arc::new(cli.clone());
        {
            let db = app.db.lock().unwrap();
            db.execute("DELETE FROM tasks", []).unwrap();
            db.execute("INSERT INTO machines(id,name,kind,profile_id,ssh_target,session,platform,disabled,helper_name,host_identity) VALUES (?1,?2,'ssh',?3,?4,?5,'windows',0,?6,?7)", params![machine.id,machine.name,machine.profile_id,machine.ssh_target,machine.session,machine.helper_name,machine.host_identity]).unwrap();
            db.execute("UPDATE projects SET workspace_id=?1,herdr_session=?2,root_path=?3,machine_id=?4 WHERE id='p1'", params![spaces[0].id,cli.session(),cwd,machine.id]).unwrap();
            db.execute("INSERT INTO tasks(id,project_id,name,status,created_at,updated_at) VALUES ('a','p1','SSH input acceptance','todo','now','now')", []).unwrap();
        }
        let prompt = if index == 0 {
            format!(
                "/C:/Temp/kanban-{operation}.md\n\n这是输入测试，请保留换行。\n只回复 KANBAN_RECEIPT_OK，不读文件、不调用工具。"
            )
        } else {
            format!(
                "Input transport test {operation}. Reply exactly KANBAN_RECEIPT_OK. Do not use tools or change files.\n第一行\n\n第二行\t保留中文和制表符\n{}\n$kanban-submit-fixture",
                if index == 2 {
                    "padding ".repeat(1450)
                } else {
                    "短文本".into()
                }
            )
        };
        let _scope = crate::remote_control::Scope::enter();
        let injection = fault.map(crate::remote_control::test_fault::Guard::arm);
        let result = app.start_task("a", &prompt, &operation);
        let target = app
            .binding("a")
            .unwrap()
            .target
            .expect("dedicated pane created");
        eprintln!(
            "SSH case {index} pane={} operation={operation} result={result:?}",
            target.pane_id
        );
        result.unwrap();
        if let Some(injection) = &injection {
            assert!(injection.fired(), "read disconnect was not exercised");
        }
        let after: String = app
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT created_at FROM dispatch_operations WHERE id=?1",
                [format!("{operation}:dispatch")],
                |r| r.get(0),
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(90);
        let receipt = loop {
            if let Some(receipt) = cli.prompt_receipt(&target, &prompt, &after, None).unwrap() {
                break receipt;
            }
            assert!(
                Instant::now() < deadline,
                "exact receipt missing; pane retained"
            );
            std::thread::sleep(Duration::from_millis(500));
        };
        // Completion is checked in the actual session log, not inferred from idle.
        loop {
            let summary = remote_summary(&machine, &receipt.session_id, &prompt);
            if summary["completed"].as_u64().unwrap() > 0 {
                assert_eq!(summary["completed"], 1);
                assert_eq!(summary["user_messages"], 1);
                assert_eq!(summary["reply"], "KANBAN_RECEIPT_OK");
                std::fs::write(
                    directory.join("session-summary.json"),
                    serde_json::to_vec_pretty(&summary).unwrap(),
                )
                .unwrap();
                break;
            }
            assert!(
                Instant::now() < deadline,
                "fixed reply missing; pane retained"
            );
            std::thread::sleep(Duration::from_millis(500));
        }
        assert!(
            cli.prompt_receipt(&target, &prompt, &after, Some(&receipt.session_id))
                .unwrap()
                .is_some()
        );
        let states: (String,String,String,String,i64) = app.db.lock().unwrap().query_row("SELECT t.status,b.state,o.state,l.state,(SELECT COUNT(*) FROM dispatch_operations WHERE task_id=t.id) FROM tasks t JOIN task_bindings b ON b.task_id=t.id JOIN dispatch_operations o ON o.task_id=t.id JOIN lifecycle_operations l ON l.task_id=t.id WHERE t.id='a'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
        assert_eq!(
            states,
            (
                "in_progress".into(),
                "valid".into(),
                "sent".into(),
                "succeeded".into(),
                1
            )
        );
        let trace = std::fs::read_to_string(directory.join("dispatch-input.jsonl")).unwrap();
        let rows: Vec<Value> = trace
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        for stage in ["text", "right", "escape", "enter"] {
            assert_eq!(
                rows.iter()
                    .filter(|r| r["stage"] == stage && r["outcome"] == "requested")
                    .count(),
                1
            );
            assert_eq!(
                rows.iter()
                    .filter(|r| r["stage"] == stage && r["outcome"] == "acknowledged")
                    .count(),
                1
            );
        }
        assert_eq!(
            rows.iter()
                .filter(|r| r["outcome"] == "observation_retry")
                .count(),
            usize::from(fault.is_some())
        );
        std::fs::write(directory.join("result.json"), serde_json::to_vec_pretty(&json!({"target":target,"operation":operation,"case":index,"fault_after_key":fault,"receipt":receipt})).unwrap()).unwrap();
        eprintln!(
            "SSH case {index}: exact receipt, one completion, one text and Enter; session={}",
            receipt.session_id
        );
        cli.close_pane(&target, &|| Ok(())).unwrap();
    }
}

fn remote_summary(machine: &crate::machines::Machine, id: &str, prompt: &str) -> Value {
    use sha2::{Digest, Sha256};
    uuid::Uuid::parse_str(id).unwrap();
    let body_hash = format!("{:x}", Sha256::digest(prompt.as_bytes()));
    let script = format!(
        r#"
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$root=if ($env:CODEX_HOME) {{$env:CODEX_HOME}} else {{Join-Path $env:USERPROFILE '.codex'}}
$files=@(Get-ChildItem (Join-Path $root 'sessions') -Recurse -File -Filter '*-{id}.jsonl')
if ($files.Count -ne 1) {{throw 'ambiguous session'}}
$records=@(Get-Content -LiteralPath $files[0].FullName -Encoding UTF8 | ForEach-Object {{$_ | ConvertFrom-Json}})
$events=@($records | Where-Object {{$_.type -eq 'event_msg'}})
$completed=@($events | Where-Object {{$_.payload.type -eq 'task_complete'}})
$hasher=[Security.Cryptography.SHA256]::Create()
$users=@($records | Where-Object {{
    if ($_.type -eq 'response_item' -and $_.payload.type -eq 'message' -and $_.payload.role -eq 'user') {{
        $text=@($_.payload.content | ForEach-Object {{$_.text}}) -join "`n"
        $hash=[BitConverter]::ToString($hasher.ComputeHash([Text.Encoding]::UTF8.GetBytes($text))).Replace('-','').ToLowerInvariant()
        $hash -ceq '{body_hash}'
    }} else {{$false}}
}})
$hasher.Dispose()
$reply=if ($completed.Count) {{$completed[-1].payload.last_agent_message}} else {{$null}}
@{{session_id='{id}';completed=$completed.Count;user_messages=$users.Count;reply=$reply}} | ConvertTo-Json -Compress
"#
    );
    let command =
        crate::remote_transport::powershell(machine.ssh_target.as_deref().unwrap(), &script)
            .unwrap();
    serde_json::from_slice(&crate::remote_transport::run(command, vec![], 30).unwrap()).unwrap()
}
