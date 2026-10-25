use super::*;
use crate::{db, tasks};
use std::sync::mpsc;
use std::time::Instant;

struct Gate {
    started: mpsc::Sender<String>,
    answer: Mutex<mpsc::Receiver<Result<String, String>>>,
}
impl TitleGenerator for Gate {
    fn generate(&self, input: &str, stop: &AtomicBool) -> Result<String, String> {
        self.started.send(input.into()).unwrap();
        loop {
            if stop.load(Ordering::Acquire) {
                return Err("stopped".into());
            }
            match self
                .answer
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_millis(20))
            {
                Ok(answer) => return answer,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err("disconnected".into()),
            }
        }
    }
}

#[test]
fn manual_name_and_prompt_edits_win_over_late_results_and_priority_keeps_new_name() {
    for edit in ["name", "prompt", "priority"] {
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
        let task = service.create(new_task("原正文")).unwrap();
        input.recv_timeout(Duration::from_secs(2)).unwrap();
        let json = if edit == "prompt" {
            prompt("新正文")
        } else {
            task.prompt_json.clone()
        };
        service
            .update(
                &task.id,
                if edit == "name" {
                    Some("手动名称")
                } else {
                    None
                },
                &json,
                TaskPriority::Urgent,
                None,
            )
            .unwrap();
        answer.send(Ok("模型名称".into())).unwrap();
        changes.recv_timeout(Duration::from_secs(2)).unwrap();
        let expected = match edit {
            "name" => "手动名称",
            "prompt" => "原正文",
            _ => "模型名称",
        };
        wait_name(&db, &task.id, expected);
        // A previously opened editor saving only priority must preserve the generated name.
        service
            .update(&task.id, None, &json, TaskPriority::Normal, None)
            .unwrap();
        wait_name(&db, &task.id, expected);
        service.shutdown();
    }
}
fn database() -> Arc<Mutex<Connection>> {
    let conn = Connection::open_in_memory().unwrap();
    db::init(&conn).unwrap();
    conn.execute("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at) VALUES ('p','Project','.','fixture','now')", []).unwrap();
    Arc::new(Mutex::new(conn))
}
fn prompt(text: &str) -> String {
    serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":text}]}]}).to_string()
}
fn new_task(text: &str) -> NewTask {
    NewTask {
        project_id: "p".into(),
        name: None,
        prompt_json: prompt(text),
        priority: TaskPriority::High,
        draft_id: None,
        locale: "zh-CN".into(),
    }
}
fn wait_name(db: &Arc<Mutex<Connection>>, id: &str, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let tasks = tasks::list(&db.lock().unwrap(), "p").unwrap();
        if tasks.iter().any(|t| t.id == id && t.name == expected) {
            return;
        }
        assert!(Instant::now() < deadline, "expected {expected}");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn create_returns_saved_task_before_generation_and_updates_only_its_name() {
    let db = database();
    let (started, input) = mpsc::channel();
    let (answer, response) = mpsc::channel();
    let service = TaskTitles::new(
        db.clone(),
        Arc::new(Gate {
            started,
            answer: Mutex::new(response),
        }),
        Arc::new(|_| {}),
    );
    let task = service
        .create(new_task("请修复登录页面的失败提示"))
        .unwrap();
    assert_eq!(task.name, "请修复登录页面的失败提示");
    assert_eq!(
        input.recv_timeout(Duration::from_secs(2)).unwrap(),
        "请修复登录页面的失败提示"
    );
    assert_eq!(
        tasks::list(&db.lock().unwrap(), "p").unwrap()[0].priority,
        TaskPriority::High
    );
    answer.send(Ok("修复登录失败提示".into())).unwrap();
    wait_name(&db, &task.id, "修复登录失败提示");
    assert_eq!(
        tasks::list(&db.lock().unwrap(), "p").unwrap()[0].prompt_json,
        task.prompt_json
    );
    service.shutdown();
}

#[test]
fn invalid_model_output_keeps_the_saved_summary() {
    for invalid in ["", "   ", "标题\n解释", "\u{0}oops"] {
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
        let task = service.create(new_task("可用的摘要")).unwrap();
        input.recv_timeout(Duration::from_secs(2)).unwrap();
        answer.send(Ok(invalid.into())).unwrap();
        changes.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            tasks::list(&db.lock().unwrap(), "p").unwrap()[0].name,
            task.name
        );
        service.shutdown();
    }
}

#[test]
fn formatted_first_paragraph_is_one_summary_and_references_are_labels_only() {
    let db = database();
    let (started, input) = mpsc::channel();
    let (answer, response) = mpsc::channel();
    let service = TaskTitles::new(
        db,
        Arc::new(Gate {
            started,
            answer: Mutex::new(response),
        }),
        Arc::new(|_| {}),
    );
    let mut request = new_task("");
    request.prompt_json = serde_json::json!({"type":"doc","content":[
        {"type":"paragraph","content":[{"type":"text","text":"修复"},{"type":"text","text":"登录问题","marks":[{"type":"bold"}]}]},
        {"type":"paragraph","content":[{"type":"mention","attrs":{"kind":"file","ref":"C:/must-not-read.txt","label":"登录页面"}}]}
    ]}).to_string();
    let task = service.create(request).unwrap();
    assert_eq!(task.name, "修复登录问题");
    assert_eq!(
        input.recv_timeout(Duration::from_secs(2)).unwrap(),
        "修复登录问题\n登录页面"
    );
    answer.send(Err("offline".into())).unwrap();
    service.shutdown();
}

#[test]
fn deleted_and_archived_tasks_never_return_from_a_late_result() {
    for archive in [false, true] {
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
        let task = service.create(new_task("待删除任务")).unwrap();
        input.recv_timeout(Duration::from_secs(2)).unwrap();
        {
            let mut conn = db.lock().unwrap();
            if archive {
                conn.execute("UPDATE tasks SET status='done' WHERE id=?1", [&task.id])
                    .unwrap();
                tasks::archive(&mut conn, &task.id).unwrap();
            } else {
                tasks::delete(&mut conn, &task.id).unwrap();
            }
        }
        answer.send(Ok("不能复活".into())).unwrap();
        changes.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(tasks::list(&db.lock().unwrap(), "p").unwrap().is_empty());
        service.shutdown();
    }
}

#[test]
fn shutdown_and_recovery_keep_tasks_without_retrying_generation() {
    let db = database();
    let (started, input) = mpsc::channel();
    let (_answer, response) = mpsc::channel();
    let generator = Arc::new(Gate {
        started,
        answer: Mutex::new(response),
    });
    let service = TaskTitles::new(db.clone(), generator.clone(), Arc::new(|_| {}));
    let first = service.create(new_task("运行中")).unwrap();
    input.recv_timeout(Duration::from_secs(2)).unwrap();
    service.create(new_task("同时可保存另一任务")).unwrap();
    input.recv_timeout(Duration::from_secs(2)).unwrap();
    service.create(new_task("排队中")).unwrap();
    service.shutdown();
    db::recover_interrupted(&db.lock().unwrap()).unwrap();
    let restarted = TaskTitles::new(db.clone(), generator, Arc::new(|_| {}));
    assert!(input.recv_timeout(Duration::from_millis(150)).is_err());
    let saved = tasks::list(&db.lock().unwrap(), "p").unwrap();
    assert_eq!(saved.len(), 3);
    assert!(saved.iter().any(|t| t.id == first.id && t.name == "运行中"));
    restarted.shutdown();
}

#[test]
fn validates_content_and_limits_input_without_generating_for_legacy_or_failed_saves() {
    let db = database();
    let (started, input) = mpsc::channel();
    let (answer, response) = mpsc::channel();
    let service = TaskTitles::new(
        db.clone(),
        Arc::new(Gate {
            started,
            answer: Mutex::new(response),
        }),
        Arc::new(|_| {}),
    );
    assert!(service.create(new_task(" \n\t ")).is_err());
    let mut bad = new_task("失败的事务");
    bad.draft_id = Some("missing-draft".into());
    assert!(service.create(bad).is_err());
    assert!(tasks::list(&db.lock().unwrap(), "p").unwrap().is_empty());
    let mut legacy = new_task("");
    legacy.name = Some("旧调用标题".into());
    legacy.prompt_json = "{}".into();
    assert_eq!(service.create(legacy).unwrap().name, "旧调用标题");
    assert!(input.recv_timeout(Duration::from_millis(100)).is_err());
    let task = service.create(new_task(&"鸭".repeat(2100))).unwrap();
    assert_eq!(task.name.chars().count(), 40);
    assert_eq!(
        input
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .chars()
            .count(),
        2000
    );
    answer.send(Err("offline".into())).unwrap();
    service.shutdown();
}

#[test]
fn image_only_task_is_durable_and_localized_without_a_model_call() {
    let directory =
        std::env::temp_dir().join(format!("kanban-title-image-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    let conn = Connection::open(directory.join("kanban.db")).unwrap();
    db::init(&conn).unwrap();
    conn.execute("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at) VALUES('p','Images','.','fixture','now')", []).unwrap();
    let draft = images::open_draft(&conn, "p", None).unwrap();
    let image = images::import_image(&conn, &draft, "image.png", "image/png", "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAFElEQVR4nGP4z8DwHwyBNBAw/AcAR8oI+ItOQ4UAAAAASUVORK5CYII=").unwrap();
    let db = Arc::new(Mutex::new(conn));
    let (started, input) = mpsc::channel();
    let (_answer, response) = mpsc::channel();
    let service = TaskTitles::new(
        db.clone(),
        Arc::new(Gate {
            started,
            answer: Mutex::new(response),
        }),
        Arc::new(|_| {}),
    );
    let mut request = new_task("");
    request.locale = "en-US".into();
    request.draft_id = Some(draft.clone());
    request.prompt_json = serde_json::json!({"type":"doc","content":[{"type":"promptImage","attrs":{"imageId":image.id,"name":"image.png"}}]}).to_string();
    let task = service.create(request).unwrap();
    assert_eq!(task.name, "Image task");
    images::close_draft(&db.lock().unwrap(), &draft).unwrap();
    assert!(images::resolve_prompt(&db.lock().unwrap(), &task.prompt_json).is_ok());
    assert!(input.recv_timeout(Duration::from_millis(100)).is_err());
    service.shutdown();
    drop(service);
    drop(db);
    assert!(directory.starts_with(std::env::temp_dir()));
    std::fs::remove_dir_all(directory).unwrap();
}
