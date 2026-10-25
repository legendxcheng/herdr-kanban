use super::*;

#[cfg(windows)]
fn shell(script: &str) -> Command {
    let mut command = Command::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    command
}

#[test]
#[cfg(windows)]
fn process_transfers_stdin_without_shell_interpolation_and_bounds_failures() {
    let stopped = AtomicBool::new(false);
    let value = b"literal $(Write-Output danger); & untouched".to_vec();
    let out = run(
        shell("[Console]::Out.Write([Console]::In.ReadToEnd())"),
        value.clone(),
        &stopped,
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(out, value);
    assert_eq!(
        run(shell("exit 3"), vec![], &stopped, Duration::from_secs(5)).unwrap_err(),
        "title.cli_failed"
    );
    let start = Instant::now();
    assert_eq!(
        run(
            shell("Start-Sleep -Seconds 30"),
            vec![],
            &stopped,
            Duration::from_millis(150)
        )
        .unwrap_err(),
        "title.timeout"
    );
    assert!(start.elapsed() < Duration::from_secs(3));
    assert_eq!(
        run(
            shell("[Console]::Out.Write('x' * 100000)"),
            vec![],
            &stopped,
            Duration::from_secs(5)
        )
        .unwrap_err(),
        "title.output_limit"
    );
    assert_eq!(
        run(
            Command::new("missing-kanban-codex-executable"),
            vec![],
            &stopped,
            Duration::from_secs(1)
        )
        .unwrap_err(),
        "title.unavailable"
    );
}

#[test]
#[cfg(windows)]
fn shutdown_cancels_a_running_process_and_releases_pipe_owners() {
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let worker = thread::spawn(move || {
        run(
            shell("Start-Sleep -Seconds 30"),
            vec![],
            &worker_stop,
            Duration::from_secs(30),
        )
    });
    thread::sleep(Duration::from_millis(200));
    stop.store(true, Ordering::Release);
    let started = Instant::now();
    assert_eq!(worker.join().unwrap().unwrap_err(), "title.cancelled");
    assert!(started.elapsed() < Duration::from_secs(3));
}
