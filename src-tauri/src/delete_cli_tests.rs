//! Exercise deletion through the workflow with the real CLI adapter and a local process fixture.
use crate::{
    db,
    herdr::Cli,
    tasks,
    workflow::{Target, Workflow},
};
use rusqlite::{Connection, params};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[test]
fn deletion_during_cli_reads_prevents_subsequent_create_and_close() {
    for phase in ["create", "close"] {
        let dir = std::env::temp_dir().join(format!("kanban-delete-cli-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("phase"), phase).unwrap();
        std::fs::write(dir.join("herdr.cmd"), "@echo off\r\npowershell.exe -NoProfile -ExecutionPolicy Bypass -File \"%~dp0probe.ps1\" %*\r\n").unwrap();
        std::fs::write(dir.join("probe.ps1"), r#"
$command = "$($args[2]) $($args[3])"
$phase = Get-Content -LiteralPath "$PSScriptRoot/phase"
$counter = Join-Path $PSScriptRoot ($command.Replace(' ','-') + '.count')
$n = 1
if (Test-Path -LiteralPath $counter) { $n = [int](Get-Content -LiteralPath $counter) + 1 }
Set-Content -LiteralPath $counter -Value $n
if (($phase -eq 'create' -and $command -eq 'workspace list' -and $n -eq 1) -or ($phase -eq 'close' -and $command -eq 'pane process-info' -and $n -eq 4)) {
    Set-Content -LiteralPath "$PSScriptRoot/waiting" -Value 'ready'
    $deadline = [DateTime]::UtcNow.AddSeconds(8)
    while (!(Test-Path -LiteralPath "$PSScriptRoot/release")) {
        if ([DateTime]::UtcNow -gt $deadline) { exit 3 }
        Start-Sleep -Milliseconds 15
    }
}
$agent = @{workspace_id='w1';pane_id='w1:p1';terminal_id='term';agent='codex';agent_status='idle';interactive_ready=$true;agent_session=@{agent='codex';kind='id';value='12345678-1234-4234-8234-123456789abc'}}
$result = switch ($command) {
    'workspace list' { @{workspaces=@(@{workspace_id='w1';label='Test'})} }
    'tab list' { @{tabs=@()} }
    'pane list' { if ($phase -eq 'create') { @{panes=@()} } else { @{panes=@($agent)} } }
    'agent get' { @{agent=$agent} }
    'pane process-info' { @{process_info=@{shell_pid=1;foreground_process_group_id=2}} }
    'tab create' { Add-Content -LiteralPath "$PSScriptRoot/writes" -Value 'create'; @{root_pane=$agent} }
    'pane close' { Add-Content -LiteralPath "$PSScriptRoot/writes" -Value 'close'; @{} }
    default { exit 4 }
}
@{result=$result} | ConvertTo-Json -Depth 8 -Compress
"#).unwrap();
        let conn = Connection::open_in_memory().unwrap();
        db::init(&conn).unwrap();
        conn.execute("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at,herdr_session,workspace_id,sync_status) VALUES ('p','Test',?1,'Test','now','default','w1','ready')", [dir.to_string_lossy().as_ref()]).unwrap();
        let task = tasks::create(
            &conn,
            "p",
            "Delete me",
            crate::models::TaskPriority::Normal,
            "{}",
        )
        .unwrap();
        if phase == "close" {
            let target = Target {
                machine_id: crate::machines::local_id(),
                session: "default".into(),
                workspace_id: "w1".into(),
                pane_id: "w1:p1".into(),
                terminal_id: "term".into(),
                agent: "codex".into(),
                process_id: "1:2".into(),
                codex_resume_locks: Vec::new(),
                title: "".into(),
                status: "idle".into(),
            };
            conn.execute(
                "INSERT INTO task_bindings(task_id,session,pane_id,target_json,state) VALUES (?1,'default','w1:p1',?2,'valid')",
                params![task.id, serde_json::to_string(&target).unwrap()],
            )
            .unwrap();
        }
        let app = Arc::new(Workflow {
            db: Arc::new(Mutex::new(conn)),
            herdr: Arc::new(Cli {
                remote: None,
                executable: dir.join("herdr.cmd"),
                session: "default".into(),
            }),
        });
        let worker = app.clone();
        let id = task.id.clone();
        let thread = std::thread::spawn(move || {
            let op = uuid::Uuid::new_v4().to_string();
            if phase == "create" {
                worker.start_task(&id, "once", &op)
            } else {
                worker.complete_task(&id, &op)
            }
        });
        let deadline = Instant::now() + Duration::from_secs(15);
        while !dir.join("waiting").exists() && !thread.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let reached = dir.join("waiting").exists();
        tasks::delete(&mut app.db.lock().unwrap(), &task.id).unwrap();
        std::fs::write(dir.join("release"), "continue").unwrap();
        let result = thread.join().unwrap();
        assert!(
            reached,
            "{phase}: fixture never reached read barrier: {result:?}"
        );
        assert!(
            !dir.join("writes").exists(),
            "{phase}: external write after deletion"
        );
        assert!(
            tasks::list(&app.db.lock().unwrap(), "p")
                .unwrap()
                .is_empty()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
