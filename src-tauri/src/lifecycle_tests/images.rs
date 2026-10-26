use super::*;

struct Resources(Arc<Mutex<rusqlite::Connection>>);
impl crate::execution::Resources for Resources {
    fn read_text(&self, _: &str) -> Result<String, String> {
        assert!(
            self.0.try_lock().is_ok(),
            "network preparation must not hold database mutex"
        );
        Ok("REMOTE CONTENT".into())
    }
    fn directory_exists(&self, _: &str) -> Result<bool, String> {
        Ok(true)
    }
    fn search_files(&self, _: &str, _: &str) -> Result<Vec<crate::file_search::FileEntry>, String> {
        Ok(vec![])
    }
    fn search_skills(
        &self,
        _: Option<&str>,
        _: &str,
    ) -> Result<Vec<crate::skill_discovery::SkillEntry>, String> {
        Ok(vec![])
    }
    fn git_statuses(
        &self,
        _: Vec<(String, String)>,
    ) -> Result<Vec<crate::project_git::ProjectGitStatus>, String> {
        Ok(vec![])
    }
    fn prepare_image(&self, _: &std::path::Path, _: &[u8]) -> Result<std::path::PathBuf, String> {
        Err("unused".into())
    }
}

#[test]
fn remote_reference_resolution_releases_database_and_never_reads_same_local_path() {
    let (mut app, _) = fixture();
    let fake = Arc::new(Fake(
        Mutex::new(State::default()),
        Some(Arc::new(Resources(app.db.clone()))),
    ));
    app.herdr = fake.clone();
    let source = std::env::temp_dir().join(format!(
        "kanban-local-reference-{}.txt",
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&source, "LOCAL CONTENT").unwrap();
    let doc=serde_json::json!({"type":"doc","content":[{"type":"mention","attrs":{"kind":"file","label":"reference","ref":source}}]}).to_string();
    crate::tasks::update(
        &app.db.lock().unwrap(),
        "a",
        "References",
        &doc,
        crate::models::TaskPriority::Normal,
    )
    .unwrap();
    let prompt = app.resolve_document(&doc).unwrap();
    assert!(prompt.contains("REMOTE CONTENT"));
    assert!(!prompt.contains("LOCAL CONTENT"));
    let app = Arc::new(app);
    app.submit_start("a", &op(), Arc::new(|_| {})).unwrap();
    assert_eq!(await_start(&app, "a").state.as_deref(), Some("succeeded"));
    assert_eq!(fake.0.lock().unwrap().sent, vec![prompt]);
    std::fs::remove_file(source).unwrap();
}

#[test]
fn prompt_preview_and_background_start_use_the_same_execution_environment() {
    let (app, fake) = fixture();
    let source =
        std::env::temp_dir().join(format!("kanban-reference-{}.txt", uuid::Uuid::new_v4()));
    std::fs::write(&source, "执行环境中的原文").unwrap();
    let document = serde_json::json!({"type":"doc","content":[
        {"type":"mention","attrs":{"kind":"file","label":"参考","ref":source.to_str().unwrap()}},
        {"type":"mention","attrs":{"kind":"skill","label":"review"}}
    ]})
    .to_string();
    crate::tasks::update(
        &app.db.lock().unwrap(),
        "a",
        "References",
        &document,
        crate::models::TaskPriority::Normal,
    )
    .unwrap();
    let preview = app.resolve_document(&document).unwrap();
    assert!(preview.contains("执行环境中的原文"));
    assert!(preview.contains("$review"));
    let app = Arc::new(app);
    app.submit_start("a", &op(), Arc::new(|_| {})).unwrap();
    assert_eq!(await_start(&app, "a").state.as_deref(), Some("succeeded"));
    assert_eq!(fake.0.lock().unwrap().sent.as_slice(), &[preview]);
    std::fs::remove_file(source).unwrap();
}

#[test]
fn manual_dispatch_reads_images_once_and_rejects_a_stale_document() {
    let (mut app, fake) = fixture();
    let directory =
        std::env::temp_dir().join(format!("kanban-manual-images-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let file = directory.join("kanban.db");
    app.db
        .lock()
        .unwrap()
        .execute("VACUUM INTO ?1", [file.to_str().unwrap()])
        .unwrap();
    app.db = Arc::new(Mutex::new(rusqlite::Connection::open(file).unwrap()));
    crate::db::init(&app.db.lock().unwrap()).unwrap();
    let doc;
    {
        let db = app.db.lock().unwrap();
        let draft = crate::images::open_draft(&db, "p1", Some("a")).unwrap();
        let image=crate::images::import_image(&db,&draft,"手动派发.png","image/png","iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAFElEQVR4nGP4z8DwHwyBNBAw/AcAR8oI+ItOQ4UAAAAASUVORK5CYII=").unwrap();
        doc=serde_json::json!({"type":"doc","content":[{"type":"promptImage","attrs":{"imageId":image.id}}]}).to_string();
        crate::images::save_task(
            &db,
            Some("a"),
            "p1",
            "Manual",
            &doc,
            crate::models::TaskPriority::Normal,
            Some(&draft),
        )
        .unwrap();
        crate::images::close_draft(&db, &draft).unwrap();
    }
    fake.0
        .lock()
        .unwrap()
        .panes
        .insert("manual-pane".into(), (target("manual-pane"), None));
    assert!(
        app.dispatch_document("a", "manual-pane", "{}", "stale-image-op")
            .unwrap_err()
            .contains("image.task_changed")
    );
    let original = app.herdr.clone();
    app.herdr = Arc::new(Fake(
        Mutex::new(State::default()),
        Some(Arc::new(Resources(app.db.clone()))),
    ));
    assert_eq!(
        app.dispatch_document("a", "manual-pane", &doc, "failed-upload")
            .unwrap_err(),
        "unused"
    );
    let pins: i64 = app
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM operation_image_refs WHERE operation_id='failed-upload'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        pins, 0,
        "failed remote upload must not retain an orphan image reference"
    );
    app.herdr = original;
    app.dispatch_document("a", "manual-pane", &doc, "image-manual-op")
        .unwrap();
    app.dispatch_document("a", "manual-pane", &doc, "image-manual-op")
        .unwrap();
    let sent = fake.0.lock().unwrap().sent.clone();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("手动派发.png"));
    assert!(sent[0].contains("view_image"));
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn automatic_start_delivers_the_saved_screenshot_and_resume_does_not_resend() {
    let (mut app, fake) = fixture();
    let directory =
        std::env::temp_dir().join(format!("kanban-start-images-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let file = directory.join("kanban.db");
    app.db
        .lock()
        .unwrap()
        .execute("VACUUM INTO ?1", [file.to_str().unwrap()])
        .unwrap();
    app.db = Arc::new(Mutex::new(rusqlite::Connection::open(file).unwrap()));
    crate::db::init(&app.db.lock().unwrap()).unwrap();
    let image_id;
    {
        let db = app.db.lock().unwrap();
        let draft = crate::images::open_draft(&db, "p1", Some("a")).unwrap();
        let image=crate::images::import_image(&db,&draft,"自动启动截图.png","image/png","iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAFElEQVR4nGP4z8DwHwyBNBAw/AcAR8oI+ItOQ4UAAAAASUVORK5CYII=").unwrap();
        image_id = image.id;
        let document=serde_json::json!({"type":"doc","content":[{"type":"promptImage","attrs":{"imageId":image_id}}]}).to_string();
        crate::images::save_task(
            &db,
            Some("a"),
            "p1",
            "Screenshot",
            &document,
            crate::models::TaskPriority::Normal,
            Some(&draft),
        )
        .unwrap();
        crate::images::close_draft(&db, &draft).unwrap();
    }
    let app = Arc::new(app);
    app.submit_start("a", &op(), Arc::new(|_| {})).unwrap();
    assert_eq!(await_start(&app, "a").state.as_deref(), Some("succeeded"));
    assert_eq!(fake.0.lock().unwrap().sent.len(), 1);
    assert!(fake.0.lock().unwrap().sent[0].contains(&image_id));
    assert!(fake.0.lock().unwrap().sent[0].contains("view_image"));
    app.complete_task("a", &op()).unwrap();
    app.submit_start("a", &op(), Arc::new(|_| {})).unwrap();
    assert_eq!(await_start(&app, "a").state.as_deref(), Some("succeeded"));
    assert_eq!(fake.0.lock().unwrap().sent.len(), 1);
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}
