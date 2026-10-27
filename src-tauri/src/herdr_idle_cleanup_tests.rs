//! Actual CLI adapter contract: one unreadable pane must not hide other candidates.
use super::*;

#[test]
fn cleanup_scan_skips_unreadable_panes_and_keeps_shells_and_nonidle_agents() {
    let dir = std::env::temp_dir().join(format!("kanban-cleanup-cli-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("herdr.cmd"), "@echo off\r\npowershell.exe -NoProfile -ExecutionPolicy Bypass -File \"%~dp0probe.ps1\" %*\r\n").unwrap();
    std::fs::write(
        dir.join("probe.ps1"),
        r#"
$command = "$($args[2]) $($args[3])"
$names = @('unreadable','shell','working','blocked','unknown','launching','idle')
$name = if ($command -eq 'pane process-info') { $args[5] } else { $args[4] }
function Pane($id) {
    $p = @{workspace_id='w1';tab_id='w1:t1';pane_id=$id;terminal_id="term-$id";cwd='C:/fixture'}
    if ($id -ne 'shell') { $p.agent='codex'; $p.agent_status=$id; $p.interactive_ready=$true }
    if ($id -eq 'launching') { $p.agent_status='idle'; $p.launch_pending=$true }
    return $p
}
if ($command -eq 'pane get' -and $name -eq 'unreadable') {
    [Console]::Error.WriteLine('{"error":{"code":"pane_not_found","message":"gone"}}')
    exit 1
}
$result = switch ($command) {
    'tab list' { @{tabs=@(@{workspace_id='w1';tab_id='w1:t1';label='kanban'})} }
    'pane list' { @{panes=@($names | ForEach-Object { Pane $_ })} }
    'pane get' { @{pane=(Pane $name)} }
    'agent get' { @{agent=(Pane $name)} }
    'pane process-info' { @{process_info=@{shell_pid=1;foreground_process_group_id=1}} }
    default { throw "Unexpected command: $command" }
}
@{result=$result} | ConvertTo-Json -Depth 8 -Compress
"#,
    )
    .unwrap();
    let cli = Cli {
        remote: None,
        executable: dir.join("herdr.cmd"),
        session: "default".into(),
    };
    let panes = cli.cleanup_panes("w1").unwrap();
    assert_eq!(
        panes
            .iter()
            .map(|p| p.target.pane_id.as_str())
            .collect::<Vec<_>>(),
        ["idle"]
    );
    std::fs::remove_dir_all(dir).unwrap();
}
