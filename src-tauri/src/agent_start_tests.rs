//! Real CLI adapter with a deterministic process fixture for the exit/start race.
use super::*;

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new(mode: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("kanban-start-race-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("mode"), mode).unwrap();
        std::fs::write(dir.join("herdr.cmd"), "@echo off\r\npowershell.exe -NoProfile -ExecutionPolicy Bypass -File \"%~dp0probe.ps1\" %*\r\n").unwrap();
        std::fs::write(dir.join("probe.ps1"), r#"
$command = "$($args[2]) $($args[3])"
if ($command -eq 'agent start') { ConvertTo-Json -InputObject @($args) -Compress | Set-Content -LiteralPath "$PSScriptRoot/argv.json" }
$mode = Get-Content -LiteralPath "$PSScriptRoot/mode"
$n = 0
if (Test-Path "$PSScriptRoot/starts") { $n = [int](Get-Content "$PSScriptRoot/starts") }
$started = Test-Path "$PSScriptRoot/started"
$term = if ($mode -eq 'replaced' -and $n -gt 0) { 'replacement' } else { 'term' }
$shell = if ($mode -eq 'shell_replaced' -and $n -gt 0) { 3 } else { 1 }
$pane = @{workspace_id='w1';pane_id='w1:p1';terminal_id=$term;agent_status='unknown'}
if (-not $started -and $mode -in @('registry_cleared','registry_replaced')) {
    if ($command -eq 'pane get') { $pane.agent='codex' }
    if ($command -eq 'agent get') {
        $pane.agent_status='idle'; $pane.interactive_ready=$true
        if ($mode -eq 'registry_replaced') { $pane.terminal_id='replacement' }
    }
}
if ($started) {
    $pane.agent='codex'; $pane.agent_status='idle'; $pane.interactive_ready=$true
    if ($mode -in @('delayed_ready','ready_replaced')) {
        $reads=0
        if (Test-Path "$PSScriptRoot/ready_reads") { $reads=[int](Get-Content "$PSScriptRoot/ready_reads") }
        if ($command -eq 'agent get') { $reads++; Set-Content "$PSScriptRoot/ready_reads" $reads }
        $pane.interactive_ready=($reads -ge 3)
        $pane.launch_pending=($reads -lt 3)
        if ($mode -eq 'ready_replaced' -and $reads -ge 2) { $pane.terminal_id='replacement' }
    }
}
$result = switch ($command) {
    'pane get' { @{pane=$pane} }
    'agent get' { @{agent=$pane} }
    'pane process-info' { @{process_info=@{shell_pid=$shell;foreground_process_group_id=$(if ($started) { 2 } else { $shell })}} }
    'agent start' {
        $n++; Set-Content "$PSScriptRoot/starts" $n
        if ($mode -eq 'unknown' -or ($mode -eq 'unknown_after_busy' -and $n -gt 1)) { [Console]::Error.WriteLine('connection lost'); exit 1 }
        if ($n -eq 1 -or $mode -eq 'always_busy') {
            $errorJson = '{"error":{"code":"agent_pane_busy","message":"not an available shell"},"id":"cli:agent:start"}'
            if ($mode -eq 'stdout') { Write-Output $errorJson; exit 0 }
            [Console]::Error.WriteLine($errorJson); exit 1
        }
        Set-Content "$PSScriptRoot/started" 'yes'; @{}
    }
    default { throw "unexpected command: $command" }
}
@{result=$result} | ConvertTo-Json -Depth 8 -Compress
"#).unwrap();
        Self(dir)
    }
    fn run(&self) -> Result<Target, String> {
        self.configured(&Default::default())
    }
    fn configured(&self, settings: &crate::codex_settings::Settings) -> Result<Target, String> {
        let cli = Cli {
            executable: self.0.join("herdr.cmd"),
            session: "default".into(),
            remote: None,
        };
        let target = Target {
            machine_id: "local".into(),
            session: "default".into(),
            workspace_id: "w1".into(),
            pane_id: "w1:p1".into(),
            terminal_id: "term".into(),
            agent: String::new(),
            process_id: "1:1".into(),
            codex_resume_locks: vec![],
            title: String::new(),
            status: "idle".into(),
        };
        cli.prepare_codex_configured(&target, ".", None, "test", settings, &|| Ok(()))
    }
    fn starts(&self) -> usize {
        std::fs::read_to_string(self.0.join("starts"))
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }
}

#[test]
fn selected_model_and_effort_reach_the_real_cli_adapter_as_arguments() {
    let fixture = Fixture::new("stderr");
    let settings = crate::codex_settings::Settings {
        model: Some("test-model".into()),
        effort: Some("high".into()),
    };
    fixture.configured(&settings).unwrap();
    let bytes = std::fs::read_to_string(fixture.0.join("argv.json")).unwrap();
    let argv: Vec<String> = serde_json::from_str(bytes.trim_start_matches('\u{feff}')).unwrap();
    assert!(argv.windows(3).any(|w| {
        w == [
            "--",
            "--no-daemon",
            "--dangerously-bypass-approvals-and-sandbox",
        ]
    }));
    assert!(!argv.iter().any(|arg| arg == "--approve-for-me"));
    assert!(argv.windows(2).any(|w| w == ["--model", "test-model"]));
    // PowerShell's Windows native argv marshalling may remove TOML quotes;
    // Codex accepts the unquoted string fallback as well.
    assert!(argv.windows(2).any(|w| {
        w[0] == "-c"
            && [
                "model_reasoning_effort=\"high\"",
                "model_reasoning_effort=high",
            ]
            .contains(&w[1].as_str())
    }));
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn pane_busy_retries_only_explicit_rejection_through_cli() {
    for mode in ["stderr", "stdout"] {
        let f = Fixture::new(mode);
        let result = f.run();
        assert!(result.is_ok(), "{mode}: {result:?}");
        assert_eq!(f.starts(), 2);
    }
}

#[test]
fn pane_busy_never_retries_unknown_or_replaced_target() {
    for mode in ["unknown", "replaced", "shell_replaced"] {
        let f = Fixture::new(mode);
        assert!(f.run().is_err());
        assert_eq!(f.starts(), 1);
    }
}

#[test]
fn pane_busy_stops_after_uncertain_retry_and_bounds_persistent_rejection() {
    let f = Fixture::new("unknown_after_busy");
    let error = f.run().unwrap_err();
    assert!(!agent_start_rejected_busy(&error));
    assert_eq!(f.starts(), 2);
    let f = Fixture::new("always_busy");
    let started = std::time::Instant::now();
    let error = f.run().unwrap_err();
    assert!(agent_start_rejected_busy(&error), "{error}");
    assert!(started.elapsed() < std::time::Duration::from_secs(15));
    assert!(f.starts() > 1);
    assert!(!f.0.join("started").exists());
}

#[test]
fn pane_busy_requires_exact_complete_rejection_envelope() {
    for error in [
        "agent_pane_busy",
        r#"herdr.failed: {"id":"cli:agent:prompt","error":{"code":"agent_pane_busy"}}"#,
        r#"herdr.failed: {"id":"cli:agent:start","error":{"code":"timeout"}}"#,
        r#"herdr.failed: {"id":"cli:agent:start","error":{"code":"agent_pane_busy"},"result":{}}"#,
        r#"herdr.failed: {"id":"cli:agent:start","error":{"code":"agent_pane_busy"}"#,
    ] {
        assert!(!agent_start_rejected_busy(error), "{error}");
    }
}

#[test]
fn successful_launch_waits_for_the_same_codex_to_be_interactive_without_relaunching() {
    let f = Fixture::new("delayed_ready");
    let result = f.run();
    assert!(
        result.is_ok(),
        "launch acknowledged before readiness: {result:?}"
    );
    assert_eq!(f.starts(), 2); // one explicit busy rejection, then one successful launch
    assert_eq!(
        std::fs::read_to_string(f.0.join("ready_reads"))
            .unwrap()
            .trim(),
        "3"
    );
}

#[test]
fn waiting_for_launch_readiness_does_not_adopt_a_replaced_terminal() {
    let f = Fixture::new("ready_replaced");
    assert_eq!(f.run().unwrap_err(), "binding.identity_changed");
    assert_eq!(f.starts(), 2);
}

#[test]
fn pane_agent_cleared_between_reads_is_a_shell_only_after_process_confirmation() {
    let f = Fixture::new("registry_cleared");
    let result = f.run();
    assert!(result.is_ok(), "pane/agent registry transition: {result:?}");
    assert_eq!(f.starts(), 2);
}

#[test]
fn pane_agent_transition_does_not_adopt_a_different_terminal() {
    let f = Fixture::new("registry_replaced");
    assert_eq!(f.run().unwrap_err(), "binding.identity_changed");
    assert!(!f.0.join("starts").exists());
}

#[test]
#[ignore = "requires an explicitly selected empty kanban-acceptance workspace and real Herdr/Codex"]
fn live_pane_busy_reuses_idle_codex_without_sending_prompt() {
    let name = std::env::var("KANBAN_LIVE_BUSY_WORKSPACE").unwrap();
    assert!(name.starts_with("kanban-acceptance-"));
    let cwd = std::env::var("KANBAN_LIVE_CWD").unwrap();
    let cli = Cli::from_env();
    let workspaces = cli.workspaces().unwrap();
    let matching: Vec<_> = workspaces.iter().filter(|w| w.name == name).collect();
    assert_eq!(matching.len(), 1);
    let workspace = &matching[0].id;
    assert!(cli.targets(workspace).unwrap().is_empty());
    let mut target = cli
        .create_pane(workspace, &cwd, "start-race", &|| Ok(()))
        .unwrap();
    let pane = target.pane_id.clone();
    let terminal = target.terminal_id.clone();
    for index in 0..4 {
        let next = cli
            .prepare_codex(&target, &cwd, None, &format!("busy-check-{index}"), &|| {
                Ok(())
            })
            .unwrap();
        assert_eq!(next.pane_id, pane);
        assert_eq!(next.terminal_id, terminal);
        assert_ne!(next.process_id, target.process_id);
        eprintln!("launch {index}: {} {}", next.pane_id, next.process_id);
        target = next;
    }
    cli.close_pane(&target, &|| Ok(())).unwrap();
    assert!(!cli.pane_exists(&target).unwrap());
}
