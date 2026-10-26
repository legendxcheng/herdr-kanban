use super::*;
use rusqlite::params;

fn fixture() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    db.execute_batch("INSERT INTO machines(id,name,kind,session,disabled) VALUES ('remote','Devpc','ssh','default',1);
        INSERT INTO projects(id,name,root_path,herdr_workspace,created_at,machine_id,workspace_id,sync_status) VALUES
        ('p','Same name','.','w','now','local','w1','ready'),
        ('remote','Same name','.','w','now','remote','w1','ready'),
        ('hidden','Hidden','.','w','now','local','w2','ready'),
        ('missing','Missing','.','w','now','local','w3','missing'),
        ('unselected','Unselected','.','w','now','local','w4','ready');
        INSERT INTO project_visibility(project_id,visible) VALUES ('p',1),('remote',1),('hidden',0),('missing',1);").unwrap();
    db
}

#[test]
fn interactive_board_reads_full_tasks_only_from_visible_scope() {
    let db = fixture();
    task(&db, "local-normal", "p", TaskPriority::Normal);
    task(&db, "remote-urgent", "remote", TaskPriority::Urgent);
    task(&db, "hidden", "hidden", TaskPriority::Urgent);
    task(&db, "missing", "missing", TaskPriority::Urgent);
    task(&db, "unselected", "unselected", TaskPriority::Urgent);
    let scope = ["p", "remote", "hidden", "missing", "unselected", "p"].map(String::from);
    let rows = crate::tasks::list_visible(&db, &scope).unwrap();
    assert_eq!(
        rows.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        ["remote-urgent", "local-normal"]
    );
    let regular = crate::tasks::list(&db, "remote").unwrap();
    assert_eq!(
        serde_json::to_value(&rows[0]).unwrap(),
        serde_json::to_value(&regular[0]).unwrap()
    );
    assert_eq!(rows[0].prompt_json, "large prompt excluded from summary");
    assert!(crate::tasks::list_visible(&db, &[]).unwrap().is_empty());
}

fn task(db: &Connection, id: &str, project: &str, priority: TaskPriority) {
    crate::tasks::create(
        db,
        project,
        id,
        priority,
        "large prompt excluded from summary",
    )
    .unwrap();
    db.execute(
        "UPDATE tasks SET id=?1,created_at='2026-10-26T00:00:00Z' WHERE name=?1",
        [id],
    )
    .unwrap();
}

#[test]
fn summary_respects_visible_scope_and_orders_across_machines_without_prompts() {
    let mut db = fixture();
    task(&db, "normal", "p", TaskPriority::Normal);
    task(&db, "high", "remote", TaskPriority::High);
    task(&db, "urgent-b", "p", TaskPriority::Urgent);
    task(&db, "urgent-a", "remote", TaskPriority::Urgent);
    task(&db, "urgent-older", "remote", TaskPriority::Urgent);
    db.execute(
        "UPDATE tasks SET created_at='2026-10-25T00:00:00Z' WHERE id='urgent-older'",
        [],
    )
    .unwrap();
    for project in ["hidden", "missing", "unselected"] {
        task(&db, project, project, TaskPriority::Urgent);
    }
    task(&db, "archived", "p", TaskPriority::Urgent);
    db.execute("UPDATE tasks SET status='done' WHERE id='archived'", [])
        .unwrap();
    crate::tasks::archive(&mut db, "archived").unwrap();
    let scope = [
        "p",
        "remote",
        "hidden",
        "missing",
        "unselected",
        "p",
        "nonexistent",
    ]
    .map(String::from);
    let rows = read(&db, &scope).unwrap();
    assert_eq!(
        rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["urgent-older", "urgent-a", "urgent-b", "high", "normal"]
    );
    assert_eq!(rows[1].project_id, "remote");
    assert_eq!(rows[2].project_id, "p");
    let json = serde_json::to_value(&rows).unwrap();
    assert!(json[0].get("prompt_json").is_none());
    assert!(json[0].get("binding").is_none());
    assert!(read(&db, &[]).unwrap().is_empty());
    assert_eq!(read(&db, &["p".into()]).unwrap().len(), 2);
    db.execute(
        "UPDATE project_visibility SET visible=0 WHERE project_id='p'",
        [],
    )
    .unwrap();
    assert!(read(&db, &["p".into()]).unwrap().is_empty());
    crate::project_visibility::show(&db, "p", true).unwrap();
    assert_eq!(read(&db, &["p".into()]).unwrap().len(), 2);
    crate::tasks::delete(&mut db, "normal").unwrap();
    assert_eq!(read(&db, &["p".into()]).unwrap().len(), 1);
}

#[test]
fn summary_projection_matches_regular_board_and_overview_through_start_and_recovery() {
    let db = fixture();
    task(&db, "starting", "p", TaskPriority::Urgent);
    for original in ["todo", "done"] {
        db.execute("UPDATE tasks SET status=?1 WHERE id='starting'", [original])
            .unwrap();
        db.execute_batch("DELETE FROM start_requests; DELETE FROM lifecycle_operations;
            INSERT INTO lifecycle_operations(id,task_id,kind,state,created_at) VALUES ('start','starting','start','pending','now');
            INSERT INTO start_requests(operation_id,project_id,workspace,cwd,prompt_json,needs_prompt,original_status)
            VALUES ('start','p','w','.','{}',1,'todo');").unwrap();
        for state in ["pending", "unknown", "failed", "succeeded"] {
            db.execute(
                "UPDATE lifecycle_operations SET state=?1 WHERE id='start'",
                [state],
            )
            .unwrap();
            let full = crate::tasks::list(&db, "p").unwrap().remove(0);
            let interactive = crate::tasks::list_visible(&db, &["p".into()])
                .unwrap()
                .remove(0);
            assert_eq!(
                serde_json::to_value(&interactive).unwrap(),
                serde_json::to_value(&full).unwrap()
            );
            let status = full
                .flow
                .unwrap()
                .requested_status
                .unwrap_or(full.status.as_str().into());
            let summary = read(&db, &["p".into()]).unwrap().remove(0);
            assert_eq!(summary.status.as_str(), status);
            assert_eq!(
                status,
                if ["pending", "unknown"].contains(&state) {
                    "in_progress"
                } else {
                    original
                }
            );
            let counts = crate::task_overview::read(
                &db,
                &["p".into()],
                "2026-10-26T00:00:00Z",
                "2026-10-27T00:00:00Z",
            )
            .unwrap()
            .remove(0);
            assert_eq!(counts.in_progress, u64::from(status == "in_progress"));
        }
        db.execute("INSERT INTO lifecycle_operations(id,task_id,kind,state,created_at) VALUES ('later','starting','complete','failed','now')", []).unwrap();
        assert_eq!(
            read(&db, &["p".into()]).unwrap()[0].status.as_str(),
            original
        );
    }
    // A legacy pending start without a durable start request does not move columns.
    db.execute_batch("DELETE FROM start_requests; DELETE FROM lifecycle_operations;
        INSERT INTO lifecycle_operations(id,task_id,kind,state,created_at) VALUES ('legacy','starting','start','unknown','now');").unwrap();
    assert_eq!(
        read(&db, &["p".into()]).unwrap()[0].status,
        TaskStatus::Done
    );
    db.execute(
        "UPDATE tasks SET priority=?1,name=?2 WHERE id='starting'",
        params!["normal", "New generated title"],
    )
    .unwrap();
    let updated = read(&db, &["p".into()]).unwrap().remove(0);
    assert_eq!(updated.name, "New generated title");
    assert_eq!(updated.priority, TaskPriority::Normal);
}
