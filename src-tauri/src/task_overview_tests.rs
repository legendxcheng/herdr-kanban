use super::*;

const START: &str = "2026-10-25T00:00:00+08:00";
const END: &str = "2026-10-26T00:00:00+08:00";

fn fixture() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    db.execute_batch("INSERT INTO projects(id,name,root_path,herdr_workspace,created_at) VALUES
        ('p','P','.','workspace','now'), ('empty','Empty','.','empty','now'), ('hidden','Hidden','.','hidden','now');").unwrap();
    db
}

fn task(db: &Connection, id: &str, status: &str, completed: Option<&str>) {
    db.execute("INSERT INTO tasks(id,project_id,name,status,created_at,updated_at,completed_at) VALUES (?1,'p',?1,?2,'now','now',?3)", params![id,status,completed]).unwrap();
}

#[test]
fn overview_matches_columns_and_local_day_including_archive_but_not_deletion() {
    let mut db = fixture();
    task(&db, "todo", "todo", None);
    task(&db, "working", "in_progress", None);
    task(&db, "old", "done", None);
    task(
        &db,
        "yesterday",
        "done",
        Some("2026-10-24T15:59:59.999+00:00"),
    );
    task(&db, "start", "done", Some("2026-10-24T16:00:00+00:00"));
    task(&db, "last", "done", Some("2026-10-25T15:59:59.999+00:00"));
    task(&db, "tomorrow", "done", Some("2026-10-25T16:00:00+00:00"));
    task(&db, "archived", "done", Some(START));
    crate::tasks::archive(&mut db, "archived").unwrap();
    task(&db, "deleted", "done", Some(START));
    crate::tasks::delete(&mut db, "deleted").unwrap();
    task(&db, "hidden_task", "todo", None);
    db.execute(
        "UPDATE tasks SET project_id='hidden' WHERE id='hidden_task'",
        [],
    )
    .unwrap();
    let rows = read(&db, &["p".into(), "empty".into(), "p".into()], START, END).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0],
        ProjectCounts {
            project_id: "empty".into(),
            todo: 0,
            in_progress: 0,
            done: 0,
            today_done: 0
        }
    );
    assert_eq!(
        rows[1],
        ProjectCounts {
            project_id: "p".into(),
            todo: 1,
            in_progress: 1,
            done: 5,
            today_done: 3
        }
    );
    assert!(read(&db, &[], START, END).unwrap().is_empty());
}

#[test]
fn pending_and_unknown_starts_match_task_column_then_failure_restores_completed_count() {
    let db = fixture();
    task(&db, "starting", "done", Some(START));
    db.execute_batch("INSERT INTO lifecycle_operations(id,task_id,kind,state,created_at) VALUES ('op','starting','start','pending','now');
        INSERT INTO start_requests(operation_id,project_id,workspace,cwd,prompt_json,needs_prompt,original_status) VALUES ('op','p','w','.','{}',0,'done');").unwrap();
    for state in ["pending", "unknown", "failed"] {
        db.execute(
            "UPDATE lifecycle_operations SET state=?1 WHERE id='op'",
            [state],
        )
        .unwrap();
        let task = crate::tasks::list(&db, "p").unwrap().remove(0);
        let projected = task
            .flow
            .unwrap()
            .requested_status
            .unwrap_or(task.status.as_str().into());
        let row = read(&db, &["p".into()], START, END).unwrap().remove(0);
        assert_eq!(row.in_progress, u64::from(projected == "in_progress"));
        assert_eq!(row.done, u64::from(projected == "done"));
        assert_eq!(row.today_done, row.done);
    }
    // A historical start must not override the latest completion operation.
    db.execute_batch("INSERT INTO lifecycle_operations(id,task_id,kind,state,created_at) VALUES ('complete','starting','complete','failed','now');").unwrap();
    assert_eq!(
        read(&db, &["p".into()], START, END).unwrap()[0].today_done,
        1
    );
}

#[test]
fn completion_timestamp_migration_preserves_legacy_unknown_dates_and_reopens() {
    let path = std::env::temp_dir().join(format!("kanban-overview-{}.db", uuid::Uuid::new_v4()));
    {
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE projects(id TEXT PRIMARY KEY,name TEXT NOT NULL,root_path TEXT NOT NULL,herdr_workspace TEXT NOT NULL,created_at TEXT NOT NULL);
            CREATE TABLE tasks(id TEXT PRIMARY KEY,project_id TEXT NOT NULL,name TEXT NOT NULL,status TEXT NOT NULL,prompt_json TEXT NOT NULL DEFAULT '{}',prompt_snapshot TEXT,dispatched_at TEXT,dispatched_to TEXT,created_at TEXT NOT NULL,updated_at TEXT NOT NULL);
            INSERT INTO projects VALUES ('p','P','.','w','now');
            INSERT INTO tasks(id,project_id,name,status,created_at,updated_at) VALUES ('old','p','Old','done','now','2026-10-25T01:00:00Z');").unwrap();
        crate::db::init(&db).unwrap();
        crate::db::init(&db).unwrap();
    }
    {
        let db = Connection::open(&path).unwrap();
        crate::db::init(&db).unwrap();
        let row = read(&db, &["p".into()], START, END).unwrap().remove(0);
        assert_eq!((row.done, row.today_done), (1, 0));
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn date_range_accepts_dst_days_and_rejects_invalid_input() {
    let db = fixture();
    for (start, end) in [
        ("2026-03-08T00:00:00-05:00", "2026-03-09T00:00:00-04:00"),
        ("2026-11-01T00:00:00-04:00", "2026-11-02T00:00:00-05:00"),
    ] {
        assert!(read(&db, &["p".into()], start, end).is_ok());
    }
    assert!(read(&db, &[], "invalid", END).is_err());
    assert!(read(&db, &[], END, START).is_err());
}
