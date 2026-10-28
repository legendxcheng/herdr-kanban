use super::*;

#[test]
#[ignore = "requires an explicit kanban-acceptance workspace and real Herdr/Codex"]
fn live_multiline_start_has_exact_receipt_and_one_submission() {
    let name = std::env::var("KANBAN_LIVE_WORKSPACE").unwrap();
    assert!(name.starts_with("kanban-acceptance-"));
    let cwd = std::env::var("KANBAN_LIVE_CWD").unwrap();
    let cli = crate::herdr::Cli::from_env();
    let spaces = cli.workspaces().unwrap();
    let matches: Vec<_> = spaces.iter().filter(|w| w.name == name).collect();
    assert_eq!(matches.len(), 1);
    let (mut app, _) = fixture();
    app.herdr = Arc::new(cli.clone());
    app.db
        .lock()
        .unwrap()
        .execute(
            "UPDATE projects SET workspace_id=?1,herdr_session=?2,root_path=?3 WHERE id='p1'",
            params![matches[0].id, cli.session(), cwd],
        )
        .unwrap();
    let prompt = format!(
        "This is an input transport test {}. Reply exactly KANBAN_RECEIPT_OK. Do not use tools or change files.\n第一行：保留换行\n\n第二行：\t保留中文、制表符与空行\n最后一行 $kanban-dev",
        uuid::Uuid::new_v4()
    );
    let operation = uuid::Uuid::new_v4().to_string();
    let result = app.start_task("a", &prompt, &operation);
    let target = app
        .binding("a")
        .unwrap()
        .target
        .expect("test pane was created");
    eprintln!("isolated test pane: {} start: {result:?}", target.pane_id);
    let created: String = app
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT created_at FROM dispatch_operations WHERE id=?1",
            [format!("{operation}:dispatch")],
            |r| r.get(0),
        )
        .unwrap();
    // A reply alone does not prove that the submitted body retained its newlines.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let receipt = loop {
        if let Some(receipt) = cli
            .prompt_receipt(&target, &prompt, &created, None)
            .unwrap()
        {
            break receipt;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "exact multiline receipt missing; pane {} retained; start={result:?}",
            target.pane_id
        );
        std::thread::sleep(std::time::Duration::from_millis(500));
    };
    eprintln!("{}", receipt.evidence);
    result.unwrap();
    let db = app.db.lock().unwrap();
    let states: (String, String, String, String, i64) = db.query_row(
        "SELECT t.status,b.state,o.state,l.state,(SELECT COUNT(*) FROM dispatch_operations WHERE task_id=t.id) FROM tasks t JOIN task_bindings b ON b.task_id=t.id JOIN dispatch_operations o ON o.task_id=t.id JOIN lifecycle_operations l ON l.task_id=t.id WHERE t.id='a'",
        [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
    ).unwrap();
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
    drop(db);
    // Preserve a still-running fixture on failure; only close after it settles.
    while !matches!(
        cli.validate(&target.workspace_id, &target)
            .unwrap()
            .status
            .as_str(),
        "idle" | "done"
    ) {
        assert!(
            std::time::Instant::now() < deadline,
            "fixture did not settle; pane retained"
        );
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    cli.close_pane(&target, &|| Ok(())).unwrap();
}

#[test]
#[ignore = "requires an isolated idle Codex pane; exits only that test agent to its existing shell"]
fn live_codex_quit_preserves_the_test_shell() {
    let workspace = std::env::var("KANBAN_LIVE_WORKSPACE").expect("explicit workspace required");
    assert!(workspace.starts_with("kanban-acceptance-"));
    let pane = std::env::var("KANBAN_LIVE_SUBMIT_PANE").expect("explicit test pane required");
    let cli = crate::herdr::Cli::from_env();
    let matches: Vec<_> = cli
        .workspaces()
        .unwrap()
        .into_iter()
        .filter(|w| w.name == workspace)
        .collect();
    assert_eq!(matches.len(), 1);
    let target = cli
        .targets(&matches[0].id)
        .unwrap()
        .into_iter()
        .find(|t| t.pane_id == pane)
        .unwrap();
    cli.send(&target, "/quit")
        .map_err(|error| error.message)
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while cli
        .targets(&matches[0].id)
        .unwrap()
        .iter()
        .any(|t| t.pane_id == pane)
    {
        assert!(
            std::time::Instant::now() < deadline,
            "test agent did not exit"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let doc = cli
        .request(&["pane", "get", &pane])
        .map_err(|error| error.message)
        .unwrap();
    assert_eq!(
        doc["result"]["pane"]["terminal_id"].as_str(),
        Some(target.terminal_id.as_str())
    );
}

#[test]
#[ignore = "requires an isolated Codex fixture that replies KANBAN_SUBMIT_OK without tools"]
fn live_short_chinese_skill_prompt_is_submitted() {
    let workspace = std::env::var("KANBAN_LIVE_WORKSPACE").expect("explicit workspace required");
    assert!(workspace.starts_with("kanban-acceptance-"));
    let pane = std::env::var("KANBAN_LIVE_SUBMIT_PANE").expect("explicit test pane required");
    let cli = crate::herdr::Cli::from_env();
    let spaces = cli.workspaces().unwrap();
    let matches: Vec<_> = spaces.iter().filter(|w| w.name == workspace).collect();
    assert_eq!(matches.len(), 1);
    let (mut app, _) = fixture();
    app.herdr = Arc::new(crate::herdr::Cli::from_env());
    app.db
        .lock()
        .unwrap()
        .execute(
            "UPDATE projects SET workspace_id=?1,herdr_session=?2",
            params![matches[0].id, cli.session()],
        )
        .unwrap();
    let target = cli
        .targets(&matches[0].id)
        .unwrap()
        .into_iter()
        .find(|target| target.pane_id == pane)
        .expect("pane must belong to the isolated workspace");
    assert!(matches!(target.status.as_str(), "idle" | "done"));
    let transcript =
        std::env::var("KANBAN_LIVE_TRANSCRIPT").expect("explicit fixture transcript required");
    let reply_count = || {
        std::fs::read_to_string(&transcript)
            .unwrap()
            .lines()
            .filter(|line| {
                let record: serde_json::Value = serde_json::from_str(line).unwrap();
                record["type"] == "event_msg"
                    && record["payload"]["type"] == "task_complete"
                    && record["payload"]["last_agent_message"] == "KANBAN_SUBMIT_OK"
            })
            .count()
    };
    // resolve_prompt trims trailing spaces, including the one after the skill.
    let prompt = "右侧的【已完成】区域 需要补上一键归档或一键删除  $kanban-dev ";
    let prompts = [
        prompt.trim().to_string(),
        prompt.trim().to_string(),
        prompt.trim().to_string(),
        "普通中文提示词".into(),
        "第一行\n第二行 $kanban-dev".into(),
        format!("{}\n$kanban-dev", "padding ".repeat(1450)),
    ];
    for (index, prompt) in prompts.iter().enumerate() {
        let before = reply_count();
        let result = app.dispatch("a", &pane, prompt, &format!("submit-{index}"));
        println!(
            "case {index} dispatch result: {}",
            if result.is_ok() {
                "accepted"
            } else {
                "unconfirmed"
            }
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while reply_count() == before {
            assert!(
                std::time::Instant::now() < deadline,
                "case {index} stayed in the composer or no actual reply arrived; dispatch: {result:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        result.unwrap();
        assert_eq!(
            reply_count(),
            before + 1,
            "the prompt must produce exactly one reply"
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !matches!(
            cli.targets(&matches[0].id)
                .unwrap()
                .into_iter()
                .find(|t| t.pane_id == pane)
                .unwrap()
                .status
                .as_str(),
            "idle" | "done"
        ) {
            assert!(std::time::Instant::now() < deadline, "agent did not settle");
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}

#[test]
#[ignore = "requires an explicitly prepared isolated Herdr acceptance workspace"]
fn live_dispatch_and_exclusive_binding() {
    let workspace =
        std::env::var("KANBAN_LIVE_WORKSPACE").expect("explicit acceptance workspace required");
    assert!(workspace.starts_with("kanban-acceptance-"));
    let (mut app, _) = fixture();
    app.herdr = Arc::new(crate::herdr::Cli::from_env());
    let spaces = app.herdr.workspaces().unwrap();
    let selected: Vec<_> = spaces.iter().filter(|w| w.name == workspace).collect();
    assert_eq!(selected.len(), 1);
    let workspace_id = &selected[0].id;
    app.db
        .lock()
        .unwrap()
        .execute(
            "UPDATE projects SET workspace_id=?1,herdr_session=?2",
            params![workspace_id, app.herdr.session()],
        )
        .unwrap();
    let targets = app.herdr.targets(workspace_id).unwrap();
    let pane = &targets.first().expect("isolated agent required").pane_id;
    let reply = format!(
        "KANBAN_ACCEPT_{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    app.dispatch(
        "a",
        pane,
        &format!("Reply only {reply}. Do not run commands or edit files. Ignore the following padding.\n{}", "padding ".repeat(1450)),
        "live-op",
    )
    .unwrap();
    assert_eq!(app.binding("a").unwrap().state, "valid");
    assert_eq!(
        app.dispatch("b", pane, "SHOULD_NOT_SEND", "live-other")
            .unwrap_err(),
        "binding.occupied"
    );
    // A successful CLI response alone does not prove Enter submitted the
    // prompt. Keep a real terminal attached and require the agent's reply.
    let cli = crate::herdr::Cli::from_env();
    let app = Arc::new(app);
    let manager = crate::terminal::TerminalManager::default();
    manager.select("submit-check", "a").unwrap();
    manager
        .open(app, &cli, "a", "submit-check", 100, 30, Arc::new(|_| true))
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
    let mut last_output = String::new();
    let mut replied = false;
    while std::time::Instant::now() < deadline {
        let output = cli
            .command()
            .args([
                "agent", "read", pane, "--source", "visible", "--lines", "12",
            ])
            .output()
            .unwrap();
        last_output = String::from_utf8_lossy(&output.stdout).into_owned();
        if last_output.contains(&format!("• {reply}")) {
            replied = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    manager.close("submit-check");
    assert!(
        replied,
        "prompt was not submitted or no reply arrived: {last_output}"
    );
}

#[test]
#[ignore = "requires an explicitly prepared isolated Herdr acceptance workspace"]
fn live_terminal_attachment_input_resize_and_safe_detach() {
    let workspace =
        std::env::var("KANBAN_LIVE_WORKSPACE").expect("explicit acceptance workspace required");
    assert!(workspace.starts_with("kanban-acceptance-"));
    let (mut app, _) = fixture();
    let cli = crate::herdr::Cli::from_env();
    app.herdr = Arc::new(cli.clone());
    let spaces = app.herdr.workspaces().unwrap();
    let selected: Vec<_> = spaces.iter().filter(|w| w.name == workspace).collect();
    assert_eq!(selected.len(), 1);
    let workspace_id = &selected[0].id;
    app.db
        .lock()
        .unwrap()
        .execute(
            "UPDATE projects SET workspace_id=?1,herdr_session=?2",
            params![workspace_id, app.herdr.session()],
        )
        .unwrap();
    let pane = app.herdr.targets(workspace_id).unwrap()[0].pane_id.clone();
    app.dispatch(
        "a",
        &pane,
        "Reply only KANBAN_ATTACH_READY. Do not run commands or edit files.",
        "live-attach",
    )
    .unwrap();
    let app = Arc::new(app);
    let manager = crate::terminal::TerminalManager::default();
    let (tx, rx) = std::sync::mpsc::channel();
    manager.select("view-a", "a").unwrap();
    manager
        .open(
            app.clone(),
            &cli,
            "a",
            "view-a",
            80,
            24,
            Arc::new(move |output| tx.send(output).is_ok()),
        )
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut bytes = Vec::new();
    while std::time::Instant::now() < deadline && bytes.len() < 100 {
        let Ok(packet) = rx.recv_timeout(std::time::Duration::from_secs(1)) else {
            continue;
        };
        assert_ne!(
            packet.kind,
            "closed",
            "{}; output={}",
            packet.message,
            String::from_utf8_lossy(&bytes)
        );
        manager.ack("view-a", packet.data.len());
        bytes.extend(packet.data);
    }
    assert!(
        bytes.len() > 100,
        "no real terminal content: {}",
        String::from_utf8_lossy(&bytes)
    );
    manager.resize("view-a", 100, 30).unwrap();
    // A harmless unsent draft tests the keyboard path without another model turn.
    manager.input(&app, "view-a", "KANBAN_INPUT_PROBE").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    manager
        .input(&app, "view-a", &"\u{7f}".repeat("KANBAN_INPUT_PROBE".len()))
        .unwrap();
    manager.close("view-a");
    assert!(
        app.target("a", true).is_ok(),
        "detaching must preserve the real agent"
    );
    assert!(manager.input(&app, "view-a", "MUST_NOT_ARRIVE").is_err());
}
