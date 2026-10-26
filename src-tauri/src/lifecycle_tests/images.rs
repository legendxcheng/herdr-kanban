use super::*;

struct Resources;
impl crate::execution::Resources for Resources {
    fn read_text(&self, _: &str) -> Result<String, String> {
        panic!("file and skill references must never read bodies")
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
fn remote_references_send_relative_paths_without_reading_bodies() {
    let (mut app, _) = fixture();
    let fake = Arc::new(Fake(
        Mutex::new(State::default()),
        Some(Arc::new(Resources)),
    ));
    app.herdr = fake.clone();
    app.db
        .lock()
        .unwrap()
        .execute(
            "UPDATE projects SET root_path='D:/remote/work' WHERE id='p1'",
            [],
        )
        .unwrap();
    let doc = serde_json::json!({"type":"doc","content":[
        {"type":"mention","attrs":{"kind":"file","label":"README.md","ref":"D:/remote/work/docs/设计 说明.md"}},
        {"type":"text","text":" "},
        {"type":"mention","attrs":{"kind":"skill","label":"review","ref":"D:/skills/review/SKILL.md"}}
    ]}).to_string();
    crate::tasks::update(
        &app.db.lock().unwrap(),
        "a",
        "References",
        &doc,
        crate::models::TaskPriority::Normal,
    )
    .unwrap();
    let prompt = app.resolve_document(&doc, "D:/remote/work").unwrap();
    assert_eq!(prompt, "\"docs/设计 说明.md\" $review");
    let app = Arc::new(app);
    app.submit_start("a", &op(), Arc::new(|_| {})).unwrap();
    assert_eq!(await_start(&app, "a").state.as_deref(), Some("succeeded"));
    assert_eq!(fake.0.lock().unwrap().sent, vec![prompt]);
}

#[test]
fn local_reference_content_is_not_sent_by_background_start() {
    let (app, fake) = fixture();
    let directory =
        std::env::temp_dir().join(format!("kanban-references-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("README.md");
    std::fs::write(&source, "FILE BODY MUST NOT BE SENT").unwrap();
    let root = directory.to_str().unwrap();
    app.db
        .lock()
        .unwrap()
        .execute("UPDATE projects SET root_path=?1 WHERE id='p1'", [root])
        .unwrap();
    let doc = serde_json::json!({"type":"doc","content":[
        {"type":"mention","attrs":{"kind":"file","label":"README.md","ref":source}},
        {"type":"text","text":" "},
        {"type":"mention","attrs":{"kind":"skill","label":"review"}}
    ]})
    .to_string();
    crate::tasks::update(
        &app.db.lock().unwrap(),
        "a",
        "References",
        &doc,
        crate::models::TaskPriority::Normal,
    )
    .unwrap();
    let prompt = app.resolve_document(&doc, root).unwrap();
    assert_eq!(prompt, "README.md $review");
    let app = Arc::new(app);
    app.submit_start("a", &op(), Arc::new(|_| {})).unwrap();
    assert_eq!(await_start(&app, "a").state.as_deref(), Some("succeeded"));
    assert_eq!(fake.0.lock().unwrap().sent, vec![prompt]);
    std::fs::remove_file(source).unwrap();
    std::fs::remove_dir(directory).unwrap();
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
        db.execute(
            "UPDATE projects SET root_path='D:/remote/work' WHERE id='p1'",
            [],
        )
        .unwrap();
        doc=serde_json::json!({"type":"doc","content":[
            {"type":"promptImage","attrs":{"imageId":image.id}},
            {"type":"mention","attrs":{"kind":"file","label":"README.md","ref":"D:/remote/work/docs/设计 说明.md"}},
            {"type":"text","text":" "},
        {"type":"mention","attrs":{"kind":"skill","label":"review"}}
        ]}).to_string();
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
        Some(Arc::new(Resources)),
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
    assert!(sent[0].contains("\"docs/设计 说明.md\" $review"));
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
