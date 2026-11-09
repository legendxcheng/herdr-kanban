use super::*;

fn legacy_database() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    crate::db::init(&db).unwrap();
    db.execute_batch(
        "DROP TABLE workspace_codex_settings;
        DELETE FROM schema_migrations WHERE version=20;
        INSERT INTO last_codex_settings VALUES(1,'unrelated-global-model','ultra');
        INSERT INTO projects(id,name,root_path,herdr_workspace,created_at) VALUES
            ('a','same name','.','same workspace','now'),
            ('b','same name','.','same workspace','now'),
            ('empty','empty','.','empty','now'),
            ('defaults','defaults','.','defaults','now'),
            ('tie','tie','.','tie','now');
        INSERT INTO tasks(id,project_id,name,created_at,updated_at,archived_at) VALUES
            ('a-old','a','old','2026-10-29','2026-11-01',NULL),
            ('a-new','a','archived newest','2026-10-31','2026-10-31','2026-11-01'),
            ('b-new','b','newest','2026-11-01','2026-11-01',NULL),
            ('defaults-old','defaults','explicit','2026-10-29','2026-10-29',NULL),
            ('defaults-new','defaults','legacy default','2026-11-01','2026-11-01',NULL),
            ('tie-a','tie','tie a','2026-10-31','2026-10-31',NULL),
            ('tie-z','tie','tie z','2026-10-31','2026-10-31',NULL);
        INSERT INTO task_codex_settings VALUES
            ('a-old','old-model','low'),
            ('a-new','first-model','high'),
            ('b-new','second-model','low'),
            ('defaults-old','old-model','high'),
            ('tie-a','old-model','high'),
            ('tie-z','tie-model',NULL);",
    )
    .unwrap();
    db
}

#[test]
fn workspace_migration_uses_each_newest_retained_task_and_preserves_defaults() {
    let db = legacy_database();
    crate::db::init(&db).unwrap();
    assert_eq!(
        last(&db, "a").unwrap(),
        Settings {
            model: Some("first-model".into()),
            effort: Some("high".into()),
        }
    );
    assert_eq!(
        last(&db, "b").unwrap(),
        Settings {
            model: Some("second-model".into()),
            effort: Some("low".into()),
        }
    );
    assert_eq!(
        last(&db, "tie").unwrap(),
        Settings {
            model: Some("tie-model".into()),
            effort: None,
        }
    );
    assert_eq!(last(&db, "empty").unwrap(), Settings::default());
    assert_eq!(last(&db, "defaults").unwrap(), Settings::default());

    // A later edit, task deletion or restart must not re-run the backfill.
    let saved = last(&db, "a").unwrap();
    save(&db, "b-new", &Settings::default(), false).unwrap();
    db.execute("DELETE FROM tasks WHERE project_id='a'", [])
        .unwrap();
    crate::db::init(&db).unwrap();
    assert_eq!(last(&db, "a").unwrap(), saved);
    assert_eq!(
        last(&db, "b").unwrap().model.as_deref(),
        Some("second-model")
    );
    db.execute("DELETE FROM projects WHERE id='a'", []).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM workspace_codex_settings WHERE project_id='a'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn workspace_migration_is_atomic_and_can_retry_after_failure() {
    let db = legacy_database();
    db.execute_batch("CREATE TRIGGER reject_migration BEFORE INSERT ON schema_migrations WHEN NEW.version=20 BEGIN SELECT RAISE(ABORT,'test rollback'); END;").unwrap();
    assert!(init(&db).is_err());
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='workspace_codex_settings'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE version=20",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        read(&db, "a-new").unwrap().model.as_deref(),
        Some("first-model")
    );
    db.execute_batch("DROP TRIGGER reject_migration;").unwrap();
    init(&db).unwrap();
    assert_eq!(
        last(&db, "a").unwrap().model.as_deref(),
        Some("first-model")
    );
}
