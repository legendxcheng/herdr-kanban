use super::*;
use crate::{db, models::TaskPriority, tasks};
use rusqlite::Connection;

const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAFElEQVR4nGP4z8DwHwyBNBAw/AcAR8oI+ItOQ4UAAAAASUVORK5CYII=";

fn await_reclaimed(db: &Connection, document: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while resolve_prompt(db, document).is_ok() {
        assert!(
            std::time::Instant::now() < deadline,
            "unreferenced image was not eventually reclaimed"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
fn remove_fixture(directory: std::path::PathBuf) {
    assert!(directory.starts_with(std::env::temp_dir()));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while let Err(error) = std::fs::remove_dir_all(&directory) {
        assert!(
            std::time::Instant::now() < deadline,
            "fixture cleanup failed: {error}"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn restart_reclaims_unsaved_drafts_and_allows_a_fresh_screenshot() {
    let directory =
        std::env::temp_dir().join(format!("kanban-image-draft-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let conn = Connection::open(directory.join("kanban.db")).unwrap();
    db::init(&conn).unwrap();
    conn.execute("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at) VALUES ('p','Images','.','fixture','now')",[]).unwrap();
    let draft = open_draft(&conn, "p", None).unwrap();
    let image = import_image(&conn, &draft, "unsaved.png", "image/png", PNG).unwrap();
    let document=serde_json::json!({"type":"doc","content":[{"type":"promptImage","attrs":{"imageId":image.id}}]}).to_string();
    db::recover_interrupted(&conn).unwrap();
    assert!(preview(&conn, &draft, &image.id).is_err());
    await_reclaimed(&conn, &document);
    let fresh = open_draft(&conn, "p", None).unwrap();
    assert!(import_image(&conn, &fresh, "fresh.png", "image/png", PNG).is_ok());
    close_draft(&conn, &fresh).unwrap();
    drop(conn);
    remove_fixture(directory);
}

#[test]
fn task_deletion_reclaims_unused_images_but_keeps_operation_evidence() {
    let directory =
        std::env::temp_dir().join(format!("kanban-image-delete-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let mut conn = Connection::open(directory.join("kanban.db")).unwrap();
    db::init(&conn).unwrap();
    conn.execute("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at) VALUES ('p','Images','.','fixture','now')",[]).unwrap();
    for pinned in [false, true] {
        let draft = open_draft(&conn, "p", None).unwrap();
        let image = import_image(&conn, &draft, "delete.png", "image/png", PNG).unwrap();
        let document=serde_json::json!({"type":"doc","content":[{"type":"promptImage","attrs":{"imageId":image.id}}]}).to_string();
        let task = save_task(
            &conn,
            None,
            "p",
            "Delete",
            &document,
            TaskPriority::Normal,
            Some(&draft),
        )
        .unwrap();
        close_draft(&conn, &draft).unwrap();
        if pinned {
            prepare_prompt(&conn, &task.id, "retained-operation", &document).unwrap();
            conn.execute("INSERT INTO dispatch_operations(id,task_id,workspace,target_json,prompt,created_at,state,had_binding) VALUES('retained-operation',?1,'fixture','{}','image prompt','now','sent',0)",[&task.id]).unwrap();
        }
        tasks::delete(&mut conn, &task.id).unwrap();
        if !pinned {
            await_reclaimed(&conn, &document);
        }
        assert_eq!(resolve_prompt(&conn, &document).is_ok(), pinned);
    }
    drop(conn);
    remove_fixture(directory);
}

#[test]
fn abandoned_prompt_preparation_does_not_keep_a_deleted_tasks_image_forever() {
    let directory =
        std::env::temp_dir().join(format!("kanban-image-abandoned-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let mut conn = Connection::open(directory.join("kanban.db")).unwrap();
    db::init(&conn).unwrap();
    conn.execute("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at) VALUES('p','P','.','fixture','now')",[]).unwrap();
    let draft = open_draft(&conn, "p", None).unwrap();
    let image = import_image(&conn, &draft, "unsent.png", "image/png", PNG).unwrap();
    let document=serde_json::json!({"type":"doc","content":[{"type":"promptImage","attrs":{"imageId":image.id}}]}).to_string();
    let task = save_task(
        &conn,
        None,
        "p",
        "Unsent",
        &document,
        TaskPriority::Normal,
        Some(&draft),
    )
    .unwrap();
    close_draft(&conn, &draft).unwrap();
    prepare_prompt(&conn, &task.id, "target-discovery-never-sent", &document).unwrap();
    tasks::delete(&mut conn, &task.id).unwrap();
    await_reclaimed(&conn, &document);
    drop(conn);
    remove_fixture(directory);
}

#[test]
fn saved_screenshot_survives_cancelled_draft_and_database_reopen() {
    let directory = std::env::temp_dir().join(format!("kanban-images-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("kanban.db");
    let conn = Connection::open(&path).unwrap();
    db::init(&conn).unwrap();
    conn.execute("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at) VALUES ('p','Images','.','fixture','now')", []).unwrap();
    let draft = open_draft(&conn, "p", None).unwrap();
    let image = import_image(&conn, &draft, "截图.png", "image/png", PNG).unwrap();
    let document = serde_json::json!({"type":"doc","content":[{"type":"promptImage","attrs":{"imageId":image.id,"name":"截图.png"}}]}).to_string();
    let task = save_task(
        &conn,
        None,
        "p",
        "Image task",
        &document,
        TaskPriority::Normal,
        Some(&draft),
    )
    .unwrap();
    close_draft(&conn, &draft).unwrap();
    drop(conn);
    let conn = Connection::open(&path).unwrap();
    db::init(&conn).unwrap();
    let task = tasks::list(&conn, "p")
        .unwrap()
        .into_iter()
        .find(|entry| entry.id == task.id)
        .unwrap();
    assert_eq!(task.prompt_json, document);
    let edit = open_draft(&conn, "p", Some(&task.id)).unwrap();
    assert_eq!(
        preview(&conn, &edit, &image.id).unwrap(),
        format!("data:image/png;base64,{PNG}")
    );
    let prompt = prepare_prompt(&conn, &task.id, "manual-test", &task.prompt_json).unwrap();
    assert!(prompt.contains("view_image"));
    assert!(prompt.contains("截图.png"));
    assert!(prompt.contains(&image.id));
    close_draft(&conn, &edit).unwrap();
    drop(conn);
    remove_fixture(directory);
}
