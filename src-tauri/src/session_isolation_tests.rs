//! Opt-in regression at the task lifecycle boundary, using the real CLI adapter.
//! Prepare an empty, isolated workspace with the Codex integration installed.
use crate::{
    herdr::Cli,
    models::TaskPriority,
    tasks,
    workflow::{Herdr, Target, Workflow},
};
use rusqlite::{Connection, params};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn operation() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn assert_automatic_review_launch(cli: &Cli, target: &Target, cwd: &str, resume: Option<&str>) {
    let doc = cli
        .json(&["pane", "process-info", "--pane", &target.pane_id])
        .unwrap();
    let info = &doc["result"]["process_info"];
    let group = info["foreground_process_group_id"].as_u64().unwrap();
    let processes = info["foreground_processes"].as_array().unwrap();
    let process = processes
        .iter()
        .find(|p| p["pid"].as_u64() == Some(group))
        .unwrap();
    let argv: Vec<_> = process["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.as_str().unwrap())
        .collect();
    let mut expected = vec!["--no-daemon", "--approve-for-me"];
    if let Some(id) = resume {
        expected.extend(["resume", id]);
    }
    expected.extend(["-C", cwd]);
    assert_eq!(&argv[1..], expected);
}

#[test]
#[ignore = "requires one isolated Codex pane switched from resume A to B without a new message"]
fn live_completion_preserves_a_pane_with_ambiguous_resume_identity() {
    let name = std::env::var("KANBAN_LIVE_SWITCHED_WORKSPACE")
        .expect("explicit switched workspace required");
    assert!(name.starts_with("kanban-acceptance-"));
    let cwd = std::env::var("KANBAN_LIVE_CWD").unwrap();
    let original = std::env::var("KANBAN_LIVE_ORIGINAL_SESSION").unwrap();
    uuid::Uuid::parse_str(&original).unwrap();
    let app = Workflow {
        db: Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        herdr: Arc::new(Cli::from_env()),
    };
    let spaces = app.herdr.workspaces().unwrap();
    let matches: Vec<_> = spaces.iter().filter(|w| w.name == name).collect();
    assert_eq!(matches.len(), 1);
    let workspace = &matches[0].id;
    let targets = app.herdr.targets(workspace).unwrap();
    assert_eq!(targets.len(), 1);
    let task = {
        let db = app.db.lock().unwrap();
        crate::db::init(&db).unwrap();
        db.execute("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at,herdr_session,workspace_id,sync_status) VALUES ('test','test',?1,?2,'now',?3,?4,'ready')",params![cwd,name,app.herdr.session(),workspace]).unwrap();
        let task = tasks::create(&db, "test", "switched", TaskPriority::Normal, "{}").unwrap();
        db.execute("INSERT INTO task_sessions(task_id,codex_session_id,cwd,updated_at) VALUES (?1,?2,?3,'now')",params![task.id,original,cwd]).unwrap();
        task
    };
    app.repair(&task.id, &targets[0].pane_id).unwrap();
    let outcome = app.complete_task(&task.id, &operation()).unwrap();
    assert_eq!(outcome.warning.as_deref(), Some("flow.session_mismatch"));
    assert!(app.herdr.pane_exists(&targets[0]).unwrap());
    let tasks = tasks::list(&app.db.lock().unwrap(), "test").unwrap();
    assert!(tasks[0].cleanup_required);
    assert_eq!(
        tasks[0].flow.as_ref().unwrap().session_id.as_deref(),
        Some(original.as_str())
    );
    assert!(tasks[0].binding.as_ref().unwrap().target.is_some());
}

#[test]
#[ignore = "requires a dedicated Herdr workspace with trusted Codex integration; starts and closes test panes"]
fn live_tasks_keep_distinct_sessions_through_completion_and_resume() {
    let name = std::env::var("KANBAN_LIVE_WORKSPACE").expect("explicit workspace required");
    assert!(name.starts_with("kanban-acceptance-"));
    let cwd = std::env::var("KANBAN_LIVE_CWD").expect("explicit test directory required");
    let path = std::env::temp_dir().join(format!("kanban-session-{}.db", operation()));
    let app = Workflow {
        db: Arc::new(Mutex::new(Connection::open(&path).unwrap())),
        herdr: Arc::new(Cli::from_env()),
    };
    let spaces = app.herdr.workspaces().unwrap();
    let matches: Vec<_> = spaces.iter().filter(|w| w.name == name).collect();
    assert_eq!(matches.len(), 1, "acceptance workspace must be unambiguous");
    let workspace = &matches[0].id;
    assert!(
        app.herdr.targets(workspace).unwrap().is_empty(),
        "use an empty workspace"
    );
    // The original shell tab is a keepalive, even with the same cwd. Automatic allocation
    // must ignore it and create just one shared kanban tab for concurrent starts.
    let cli = Cli::from_env();
    assert!(cli.kanban_tab(workspace).unwrap().is_none());
    let ids = {
        let db = app.db.lock().unwrap();
        crate::db::init(&db).unwrap();
        db.execute("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at,herdr_session,workspace_id,sync_status) VALUES ('test','test',?1,?2,'now',?3,?4,'ready')",
            params![cwd, name, app.herdr.session(), workspace]).unwrap();
        ["A", "B"].map(|name| {
            tasks::create(&db, "test", name, TaskPriority::Normal, "{}")
                .unwrap()
                .id
        })
    };
    let mut targets = Vec::new();
    let mut sessions = Vec::new();
    std::thread::scope(|scope| {
        let starts: Vec<_> = ids
            .iter()
            .map(|id| {
                scope.spawn(|| {
                    app.start_task(
                        id,
                        "Reply only READY. Do not run commands or edit files.",
                        &operation(),
                    )
                    .unwrap();
                })
            })
            .collect();
        for start in starts {
            start.join().unwrap();
        }
    });
    let shared_tab = cli.kanban_tab(workspace).unwrap().unwrap();
    for id in &ids {
        let target = app.binding(id).unwrap().target.unwrap();
        assert_automatic_review_launch(&cli, &target, &cwd, None);
        let pane = cli.json(&["pane", "get", &target.pane_id]).unwrap();
        assert_eq!(
            pane.pointer("/result/pane/tab_id")
                .and_then(serde_json::Value::as_str),
            Some(shared_tab.as_str())
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        let session = loop {
            if let Some(session) = app.herdr.session_id(&target).unwrap() {
                break session;
            }
            assert!(
                Instant::now() < deadline,
                "task pane {} did not receive its session ID",
                target.pane_id
            );
            std::thread::sleep(Duration::from_millis(200));
        };
        targets.push(target);
        sessions.push(session);
    }
    assert_ne!(targets[0].pane_id, targets[1].pane_id);
    assert_ne!(sessions[0], sessions[1]);
    // Starting B must not replace A's report with B's session.
    assert_eq!(
        app.herdr.session_id(&targets[0]).unwrap(),
        Some(sessions[0].clone())
    );
    for (index, id) in ids.iter().enumerate() {
        assert!(
            app.complete_task(id, &operation())
                .unwrap()
                .warning
                .is_none()
        );
        assert!(!app.herdr.pane_exists(&targets[index]).unwrap());
        if index == 0 {
            assert!(app.herdr.pane_exists(&targets[1]).unwrap());
            assert_eq!(cli.kanban_tab(workspace).unwrap(), Some(shared_tab.clone()));
        }
    }
    assert!(cli.kanban_tab(workspace).unwrap().is_none());
    drop(app);
    let conn = Connection::open(&path).unwrap();
    crate::db::init(&conn).unwrap();
    crate::db::recover_interrupted(&conn).unwrap();
    let mut app = Workflow {
        db: Arc::new(Mutex::new(conn)),
        herdr: Arc::new(Cli::from_env()),
    };
    for (index, id) in ids.iter().enumerate() {
        app.start_task(id, "MUST_NOT_BE_SENT_ON_RESUME", &operation())
            .unwrap();
        let target = app.binding(id).unwrap().target.unwrap();
        assert_automatic_review_launch(&cli, &target, &cwd, Some(&sessions[index]));
        let tab = cli.kanban_tab(workspace).unwrap().unwrap();
        let pane = cli.json(&["pane", "get", &target.pane_id]).unwrap();
        assert_eq!(
            pane.pointer("/result/pane/tab_id")
                .and_then(serde_json::Value::as_str),
            Some(tab.as_str())
        );
        assert_eq!(
            app.herdr.session_id(&target).unwrap(),
            Some(sessions[index].clone())
        );
        assert_eq!(target.codex_resume_locks.len(), 2);
        // The new process evidence must survive a Kanban restart before input.
        drop(app);
        let conn = Connection::open(&path).unwrap();
        crate::db::init(&conn).unwrap();
        crate::db::recover_interrupted(&conn).unwrap();
        app = Workflow {
            db: Arc::new(Mutex::new(conn)),
            herdr: Arc::new(Cli::from_env()),
        };
        assert_eq!(
            app.binding(id).unwrap().target.unwrap().codex_resume_locks,
            target.codex_resume_locks
        );
        assert!(
            app.complete_task(id, &operation())
                .unwrap()
                .warning
                .is_none()
        );
        assert!(!app.herdr.pane_exists(&target).unwrap());
    }
    let listed = tasks::list(&app.db.lock().unwrap(), "test").unwrap();
    for task in listed {
        let index = ids.iter().position(|id| id == &task.id).unwrap();
        assert_eq!(task.status, crate::models::TaskStatus::Done);
        assert!(!task.cleanup_required);
        assert_eq!(task.flow.unwrap().session_id, Some(sessions[index].clone()));
        assert_eq!(
            task.prompt_snapshot.as_deref(),
            Some("Reply only READY. Do not run commands or edit files.")
        );
    }
    // A later in-process switch keeps the original argv. Its changed lock set
    // must not be accepted as the startup helper, even after automatic review.
    app.start_task(&ids[0], "MUST_NOT_BE_SENT_ON_RESUME", &operation())
        .unwrap();
    let target = app.binding(&ids[0]).unwrap().target.unwrap();
    cli.send_checked(&target, &format!("/resume {}", sessions[1]), &|| Ok(()))
        .unwrap_or_else(|e| panic!("{}", e.message));
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.herdr.session_id(&target).is_ok() {
        assert!(
            Instant::now() < deadline,
            "switched session was not detected"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let outcome = app.complete_task(&ids[0], &operation()).unwrap();
    assert_eq!(outcome.warning.as_deref(), Some("flow.session_mismatch"));
    assert!(app.herdr.pane_exists(&target).unwrap());
    let task = tasks::list(&app.db.lock().unwrap(), "test")
        .unwrap()
        .into_iter()
        .find(|task| task.id == ids[0])
        .unwrap();
    assert!(task.cleanup_required);
    assert_eq!(
        task.flow.unwrap().session_id.as_deref(),
        Some(sessions[0].as_str())
    );
    // Explicit cleanup belongs to this isolated regression, never the task flow.
    cli.json(&["pane", "close", &target.pane_id]).unwrap();
    drop(app);
    std::fs::remove_file(path).unwrap();
}
