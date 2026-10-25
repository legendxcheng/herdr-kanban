use super::*;

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
