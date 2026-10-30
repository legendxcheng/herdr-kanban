use super::*;

fn queued(db: &Arc<Mutex<Connection>>) -> Task {
    let service = TaskTitles::new(db.clone(), Arc::new(NoTitle), Arc::new(|_| {}));
    service.shutdown();
    let task = service.create(new_task("原始内容")).unwrap();
    db.lock()
        .unwrap()
        .execute(
            "UPDATE task_title_requests SET state='pending',created_ms=0,next_attempt_ms=0 WHERE task_id=?1",
            [&task.id],
        )
        .unwrap();
    task
}

#[test]
fn timeout_retries_after_backoff_and_succeeds_without_expiring_in_queue() {
    let db = database();
    let task = queued(&db);
    let conn = db.lock().unwrap();
    let job = take_next_at(&conn, 100_000).unwrap().unwrap();
    finish_at(&conn, &job, Err("machine.timeout".into()), 100_000).unwrap();
    assert!(take_next_at(&conn, 109_999).unwrap().is_none());
    let retry = take_next_at(&conn, 110_000).unwrap().unwrap();
    assert_eq!(retry.attempt, 2);
    finish_at(&conn, &retry, Ok("正式标题".into()), 110_001).unwrap();
    assert_eq!(tasks::list(&conn, "p").unwrap()[0].name, "正式标题");
    assert_eq!(view(&conn, &task.id).unwrap().unwrap().state, "succeeded");
}

#[test]
fn retry_budget_is_bounded_and_permanent_errors_stop_immediately() {
    for error in [
        "title.timeout",
        "machine.identity_changed",
        "machine.disabled",
        "title.unavailable",
    ] {
        let db = database();
        let task = queued(&db);
        let conn = db.lock().unwrap();
        let mut now = 100_000;
        let expected_attempts = if error == "title.timeout" { 4 } else { 1 };
        for attempt in 1..=expected_attempts {
            let job = take_next_at(&conn, now).unwrap().unwrap();
            assert_eq!(job.attempt, attempt);
            finish_at(&conn, &job, Err(error.into()), now).unwrap();
            let state = view(&conn, &task.id).unwrap().unwrap();
            if attempt < expected_attempts {
                assert_eq!(
                    state.next_attempt_ms,
                    now + [10_000, 30_000, 120_000][attempt as usize - 1]
                );
                now = state.next_attempt_ms;
            }
        }
        assert!(take_next_at(&conn, now + 999_999).unwrap().is_none());
        assert_eq!(view(&conn, &task.id).unwrap().unwrap().state, "failed");
        assert_eq!(tasks::list(&conn, "p").unwrap()[0].name, task.name);
    }
}

#[test]
fn restart_preserves_queued_work_and_attempt_budget() {
    let db = database();
    let task = queued(&db);
    let other = queued(&db);
    let conn = db.lock().unwrap();
    let job = take_next_at(&conn, 100).unwrap().unwrap();
    recover_at(&conn, 200).unwrap();
    let recovered = view(&conn, &job.task).unwrap().unwrap();
    assert_eq!(recovered.state, "retry_wait");
    assert_eq!(recovered.attempts, 1);
    assert_eq!(recovered.next_attempt_ms, 10_200);
    let waiting = take_next_at(&conn, 201).unwrap().unwrap();
    assert_ne!(waiting.task, job.task);
    finish_at(&conn, &waiting, Ok("第二个".into()), 202).unwrap();
    let retry = take_next_at(&conn, 10_200).unwrap().unwrap();
    assert_eq!(retry.attempt, 2);
    finish_at(&conn, &job, Ok("迟到的旧结果".into()), 10_201).unwrap();
    assert_eq!(view(&conn, &retry.task).unwrap().unwrap().state, "running");
    finish_at(&conn, &retry, Ok("恢复成功".into()), 10_202).unwrap();
    assert_ne!(task.id, other.id);
}

#[test]
fn manual_retry_uses_current_content_and_rejects_old_results_and_duplicate_clicks() {
    let db = database();
    let task = queued(&db);
    let conn = db.lock().unwrap();
    let old = take_next_at(&conn, 1).unwrap().unwrap();
    finish_at(&conn, &old, Err("title.unavailable".into()), 2).unwrap();
    retry_request(&conn, &task.id, &old.id, 3).unwrap();
    let fresh = view(&conn, &task.id).unwrap().unwrap();
    retry_request(&conn, &task.id, &old.id, 4).unwrap();
    assert_eq!(view(&conn, &task.id).unwrap().unwrap().id, fresh.id);
    let new = take_next_at(&conn, 5).unwrap().unwrap();
    finish_at(&conn, &old, Ok("旧结果".into()), 6).unwrap();
    assert_eq!(tasks::list(&conn, "p").unwrap()[0].name, task.name);
    finish_at(&conn, &new, Ok("新结果".into()), 7).unwrap();
    assert_eq!(tasks::list(&conn, "p").unwrap()[0].name, "新结果");
    assert!(retry_request(&conn, &task.id, &old.id, 8).is_err());
}

#[test]
fn edits_during_backoff_or_after_failure_revoke_retry_permission() {
    for state in ["retry_wait", "failed", "interrupted"] {
        for edit in ["name", "prompt", "archive", "delete"] {
            let db = database();
            let task = queued(&db);
            let conn = db.lock().unwrap();
            let old = take_next_at(&conn, 1).unwrap().unwrap();
            conn.execute(
                "UPDATE task_title_requests SET state=?1 WHERE task_id=?2",
                params![state, task.id],
            )
            .unwrap();
            match edit {
                "name" => conn.execute("UPDATE tasks SET name='人工标题' WHERE id=?1", [&task.id]),
                "prompt" => conn.execute(
                    "UPDATE tasks SET prompt_json=?2 WHERE id=?1",
                    params![task.id, prompt("新内容")],
                ),
                "archive" => {
                    conn.execute("UPDATE tasks SET archived_at='now' WHERE id=?1", [&task.id])
                }
                _ => conn.execute("DELETE FROM tasks WHERE id=?1", [&task.id]),
            }
            .unwrap();
            assert!(take_next_at(&conn, 999_999).unwrap().is_none());
            assert!(retry_request(&conn, &task.id, &old.id, 1_000_000).is_err());
            finish_at(&conn, &old, Ok("旧结果".into()), 1_000_001).unwrap();
            assert!(
                !tasks::list(&conn, "p")
                    .unwrap()
                    .iter()
                    .any(|t| t.name == "旧结果")
            );
        }
    }
}

#[test]
fn legacy_failures_are_manual_only_and_retry_reads_current_content() {
    let db = database();
    let task = queued(&db);
    let conn = db.lock().unwrap();
    // Recreate the deployed v11 table to test the real upgrade rather than a fresh schema.
    conn.execute_batch("DROP TRIGGER invalidate_task_title; DROP TABLE task_title_requests;
        DELETE FROM schema_migrations WHERE version=19;
        CREATE TABLE task_title_requests(task_id TEXT PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
            id TEXT NOT NULL UNIQUE,input TEXT NOT NULL,state TEXT NOT NULL,created_ms INTEGER NOT NULL,
            edit_version INTEGER NOT NULL DEFAULT 0,error TEXT NOT NULL DEFAULT '');").unwrap();
    conn.execute("INSERT INTO task_title_requests(task_id,id,input,state,created_ms,error) VALUES(?1,'legacy','旧内容','failed',0,'machine.timeout')", [&task.id]).unwrap();
    conn.execute(
        "UPDATE tasks SET prompt_json=?2 WHERE id=?1",
        params![task.id, prompt("当前内容")],
    )
    .unwrap();
    init(&conn).unwrap();
    init(&conn).unwrap();
    recover_at(&conn, 100).unwrap();
    assert!(take_next_at(&conn, 100).unwrap().is_none());
    retry_request(&conn, &task.id, "legacy", 100).unwrap();
    let job = take_next_at(&conn, 100).unwrap().unwrap();
    assert_eq!(job.input, "当前内容");
    assert_ne!(job.id, "legacy");
}

#[test]
fn retry_schedule_survives_database_reopen_and_final_interruption_is_bounded() {
    let directory =
        std::env::temp_dir().join(format!("kanban-title-retry-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("kanban.db");
    let task_id;
    {
        let conn = Connection::open(&path).unwrap();
        db::init(&conn).unwrap();
        conn.execute("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at) VALUES('p','P','.','fixture','now')", []).unwrap();
        let db = Arc::new(Mutex::new(conn));
        let task = queued(&db);
        task_id = task.id;
        let conn = db.lock().unwrap();
        let job = take_next_at(&conn, 100).unwrap().unwrap();
        finish_at(&conn, &job, Err("title.timeout".into()), 100).unwrap();
    }
    {
        let conn = Connection::open(&path).unwrap();
        db::init(&conn).unwrap();
        recover_at(&conn, 500).unwrap();
        let state = view(&conn, &task_id).unwrap().unwrap();
        assert_eq!((state.attempts, state.next_attempt_ms), (1, 10_100));
        assert!(take_next_at(&conn, 10_099).unwrap().is_none());
        let retry = take_next_at(&conn, 10_100).unwrap().unwrap();
        assert_eq!(retry.attempt, 2);
        conn.execute(
            "UPDATE task_title_requests SET attempts=4 WHERE task_id=?1",
            [&task_id],
        )
        .unwrap();
        recover_at(&conn, 10_200).unwrap();
        assert_eq!(view(&conn, &task_id).unwrap().unwrap().state, "failed");
        assert!(take_next_at(&conn, 999_999).unwrap().is_none());
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn real_workers_retry_a_failed_request_and_do_not_block_other_tasks_during_backoff() {
    let db = database();
    let (started, input) = mpsc::channel();
    let (answer, response) = mpsc::channel();
    let (changed, changes) = mpsc::channel();
    let service = TaskTitles::new(
        db.clone(),
        Arc::new(Gate {
            started,
            answer: Mutex::new(response),
        }),
        Arc::new(move |_| {
            let _ = changed.send(());
        }),
    );
    let first = service.create(new_task("第一个")).unwrap();
    assert_eq!(
        input.recv_timeout(Duration::from_secs(2)).unwrap(),
        "第一个"
    );
    answer.send(Err("machine.timeout".into())).unwrap();
    changes.recv_timeout(Duration::from_secs(2)).unwrap();
    service.create(new_task("第二个")).unwrap();
    assert_eq!(
        input.recv_timeout(Duration::from_secs(2)).unwrap(),
        "第二个"
    );
    answer.send(Ok("第二个标题".into())).unwrap();
    changes.recv_timeout(Duration::from_secs(2)).unwrap();
    // Make the stored retry due without sleeping through the production backoff.
    db.lock()
        .unwrap()
        .execute(
            "UPDATE task_title_requests SET next_attempt_ms=0 WHERE task_id=?1",
            [&first.id],
        )
        .unwrap();
    assert_eq!(
        input.recv_timeout(Duration::from_secs(2)).unwrap(),
        "第一个"
    );
    answer.send(Ok("重试成功".into())).unwrap();
    changes.recv_timeout(Duration::from_secs(2)).unwrap();
    wait_name(&db, &first.id, "重试成功");
    assert_eq!(
        view(&db.lock().unwrap(), &first.id)
            .unwrap()
            .unwrap()
            .attempts,
        2
    );
    service.shutdown();
}

#[test]
#[ignore = "Uses one real local Codex request and waits for the production retry backoff"]
fn live_codex_title_succeeds_after_automatic_retry() {
    struct FailOnce(std::sync::atomic::AtomicUsize);
    impl TitleGenerator for FailOnce {
        fn generate(&self, _: &str, input: &str, stop: &AtomicBool) -> Result<String, String> {
            if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err("title.timeout".into());
            }
            crate::title_cli::CodexTitleGenerator::from_env().generate(input, stop)
        }
    }
    let db = database();
    let generator = Arc::new(FailOnce(std::sync::atomic::AtomicUsize::new(0)));
    let (changed, changes) = mpsc::channel();
    let service = TaskTitles::new(
        db.clone(),
        generator.clone(),
        Arc::new(move |_| {
            let _ = changed.send(());
        }),
    );
    let task = service
        .create(new_task("用户登录失败时应显示错误原因并保留已经输入的账号"))
        .unwrap();
    changes.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(
        view(&db.lock().unwrap(), &task.id).unwrap().unwrap().state,
        "retry_wait"
    );
    changes.recv_timeout(Duration::from_secs(45)).unwrap();
    let conn = db.lock().unwrap();
    let state = view(&conn, &task.id).unwrap().unwrap();
    assert_eq!(state.state, "succeeded", "{}", state.error);
    assert_eq!(state.attempts, 2);
    assert_eq!(generator.0.load(Ordering::SeqCst), 2);
    println!(
        "automatic_retry_attempts={} title={}",
        state.attempts,
        tasks::list(&conn, "p").unwrap()[0].name
    );
    drop(conn);
    service.shutdown();
}
