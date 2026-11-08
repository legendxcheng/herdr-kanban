//! Real CLI transport boundary: a session switch while waiting must write nothing.
use super::*;

#[test]
fn first_prompt_pins_known_session_through_busy_observations() {
    let dir = std::env::temp_dir().join(format!("kanban-first-cli-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let executable = dir.join("herdr.cmd");
    std::fs::write(
        &executable,
        "@echo off\r\npowershell.exe -NoProfile -File \"%~dp0probe.ps1\" %*\r\n",
    )
    .unwrap();
    std::fs::write(dir.join("probe.ps1"), r#"
$command="$($args[2]) $($args[3])"
if ($command -eq 'agent get') {
    $n=0; if (Test-Path "$PSScriptRoot/count") { $n=[int](Get-Content "$PSScriptRoot/count") }
    $n++; Set-Content "$PSScriptRoot/count" $n
    $id=if($n -eq 1){'11111111-1111-4111-8111-111111111111'}else{'22222222-2222-4222-8222-222222222222'}
    @{result=@{agent=@{workspace_id='w1';pane_id='w1:p1';terminal_id='term';agent='codex';agent_status='working';interactive_ready=$true;state_change_seq=$n;agent_session=@{agent='codex';kind='id';value=$id}}}} | ConvertTo-Json -Depth 8 -Compress
    exit 0
}
if ($command -eq 'pane process-info') { '{"result":{"process_info":{"shell_pid":1,"foreground_process_group_id":2}}}'; exit 0 }
Add-Content "$PSScriptRoot/writes" $command
'{}'
"#).unwrap();
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
    let result = cli.send_first_prompt(
        &target,
        "once",
        Some("11111111-1111-4111-8111-111111111111"),
        &|| Ok(()),
        &|| Ok(false),
    );
    let failure = result.unwrap_err();
    assert_eq!(failure.message, "flow.session_mismatch");
    assert!(!failure.uncertain);
    assert!(!dir.join("writes").exists());
    std::fs::remove_dir_all(dir).unwrap();
}
