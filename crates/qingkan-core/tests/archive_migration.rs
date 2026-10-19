use qingkan_core::archive::*;
use rusqlite::{params, types::Value, Connection};
use serde_json::json;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

fn legacy(dir: &std::path::Path, populated: bool) -> Connection {
    let c = Connection::open(dir.join("messages.sqlite3")).unwrap();
    c.execute_batch(include_str!("fixtures/archive_v1.sql"))
        .unwrap();
    if populated {
        c.execute_batch("INSERT INTO anchors VALUES('11','原主播😀','111','first','last');
            INSERT INTO sessions VALUES('11','101','原主播😀','111','标题','first','last','2026-09-30T23:00:00+08:00');
            INSERT INTO sessions VALUES('11','102','空场次','111','','first','last',NULL);").unwrap();
        for (id, time, sent, received) in [
            (
                41,
                "2026-09-30T23:59:59.999+08:00",
                Some(month_bounds("2026-10").unwrap().0 - 0.001),
                month_bounds("2026-10").unwrap().0,
            ),
            (
                42,
                "2026-10-01T00:00:00+08:00",
                Some(month_bounds("2026-10").unwrap().0),
                month_bounds("2026-10").unwrap().0,
            ),
            (43, "", None, month_bounds("2026-11").unwrap().0),
        ] {
            c.execute("INSERT INTO messages VALUES(?,'11','原主播😀','111','101','001234567890123456789','观众','chat','原文',?,'2026-11-01T00:00:00+08:00',?,?,?,?,'{\"fact\":\"原文\"}')",
                params![id,time,if sent.is_some(){"server"}else{"missing"},format!("old-{id}"),sent,received]).unwrap();
        }
        // A deleted high ID must remain reserved after migration.
        c.execute(
            "UPDATE sqlite_sequence SET seq=999 WHERE name='messages'",
            [],
        )
        .unwrap();
    }
    c
}
fn rows(c: &Connection, sql: &str) -> Vec<Vec<Value>> {
    let mut statement = c.prepare(sql).unwrap();
    let n = statement.column_count();
    statement
        .query_map([], |r| (0..n).map(|i| r.get::<_, Value>(i)).collect())
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap()
}
fn state(c: &Connection) -> Vec<Vec<Vec<Value>>> {
    [
        "SELECT * FROM anchors ORDER BY anchor_id",
        "SELECT * FROM sessions ORDER BY anchor_id,session_id",
        "SELECT * FROM messages ORDER BY id",
        "SELECT * FROM sqlite_sequence ORDER BY name",
    ]
    .map(|q| rows(c, q))
    .to_vec()
}
fn version(c: &Connection) -> i64 {
    c.query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}
fn activity() -> MessageRecord {
    serde_json::from_value(
        json!({"anchor_id":"11","anchor_name":"新主播","session_id":"101",
        "message_type":"activity","content":"红包开始","message_time":"2026-10-31T23:59:59+08:00",
        "received_at":"2026-11-01T00:00:00+08:00","time_source":"server","message_id":"activity-1",
        "payload":{"activity_kind":"red_packet","activity_id":"00123"}}),
    )
    .unwrap()
}
#[test]
fn populated_v1_preserves_values_ids_indexes_fks_and_autoincrement() {
    let dir = tempfile::tempdir().unwrap();
    let c = legacy(dir.path(), true);
    c.execute_batch("CREATE INDEX custom_content ON messages(content);
        CREATE TABLE external_ref(id INTEGER REFERENCES messages(id) ON DELETE CASCADE);
        INSERT INTO external_ref VALUES(42);
        CREATE TABLE audit(value TEXT);
        CREATE TRIGGER custom_audit AFTER INSERT ON messages BEGIN INSERT INTO audit VALUES(NEW.message_id); END;
        CREATE VIEW visible_messages AS SELECT id,content FROM messages;
        CREATE TRIGGER view_delete INSTEAD OF DELETE ON visible_messages BEGIN DELETE FROM messages WHERE id=OLD.id; END;").unwrap();
    let before = state(&c);
    let objects = rows(&c,"SELECT type,name,sql FROM sqlite_schema WHERE type IN ('index','trigger','view') AND tbl_name NOT LIKE 'quality_%' AND tbl_name<>'session_monitor_durations' AND name<>'messages_quality_session' AND name NOT IN ('messages_type_recent','messages_room_recent','messages_room_type_recent','messages_lottery_draw','activity_alias_row','activity_round_scope','messages_activity_scope') ORDER BY type,name");
    let fk = rows(&c, "PRAGMA foreign_key_list(messages)");
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    assert_eq!(version(&c), 3);
    assert_eq!(state(&c), before);
    assert_eq!(rows(&c, "PRAGMA foreign_key_list(messages)"), fk);
    assert_eq!(rows(&c,"SELECT type,name,sql FROM sqlite_schema WHERE type IN ('index','trigger','view') AND tbl_name NOT LIKE 'quality_%' AND tbl_name<>'session_monitor_durations' AND name<>'messages_quality_session' AND name NOT IN ('messages_type_recent','messages_room_recent','messages_room_type_recent','messages_lottery_draw','activity_alias_row','activity_round_scope','messages_activity_scope') ORDER BY type,name"),objects);
    assert_eq!(
        rows(&c, "SELECT * FROM external_ref"),
        vec![vec![Value::Integer(42)]]
    );
    assert!(rows(&c, "SELECT * FROM audit").is_empty());
    for month in ["2026-09", "2026-10", "2026-11"] {
        let mut count = 0;
        store
            .snapshot()
            .unwrap()
            .visit_messages(None, month, None, |_| {
                count += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(count, 1);
    }
    assert!(store
        .record_message(activity(), String::new(), None)
        .unwrap());
    assert!(!store
        .record_message(activity(), String::new(), None)
        .unwrap());
    assert_eq!(
        c.query_row(
            "SELECT id FROM messages WHERE message_id='activity-1'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1000
    );
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM audit", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(rows(&c, "PRAGMA foreign_key_check").is_empty());
    assert!(store.close(Duration::from_secs(2)));
}
#[test]
fn empty_v1_and_new_database_become_v3_and_second_open_does_not_rebuild() {
    for existing in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        if existing {
            drop(legacy(dir.path(), false));
        }
        let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
        let c = Connection::open(store.path()).unwrap();
        assert_eq!(version(&c), 3);
        assert!(rows(&c, "SELECT * FROM sqlite_sequence").is_empty());
        let schema = rows(&c, "SELECT * FROM sqlite_schema ORDER BY name");
        let schema_version = rows(&c, "PRAGMA schema_version");
        assert!(store.close(Duration::from_secs(2)));
        let reopened = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
        assert_eq!(
            rows(&c, "SELECT * FROM sqlite_schema ORDER BY name"),
            schema
        );
        assert_eq!(rows(&c, "PRAGMA schema_version"), schema_version);
        assert!(reopened.close(Duration::from_secs(2)));
    }
}
#[test]
fn constraint_error_rolls_back_all_schema_rows_and_sequence() {
    let dir = tempfile::tempdir().unwrap();
    let c = legacy(dir.path(), true);
    c.execute_batch("PRAGMA ignore_check_constraints=ON; UPDATE messages SET message_type='invalid' WHERE id=42; PRAGMA ignore_check_constraints=OFF;").unwrap();
    let before = state(&c);
    let schema = rows(&c, "SELECT * FROM sqlite_schema ORDER BY name");
    assert!(ArchiveStore::open(dir.path(), ArchiveOptions::default()).is_err());
    assert_eq!(version(&c), 1);
    assert_eq!(state(&c), before);
    assert_eq!(
        rows(&c, "SELECT * FROM sqlite_schema ORDER BY name"),
        schema
    );
}
#[test]
fn interrupted_copy_rolls_back_then_a_normal_open_can_retry() {
    let dir = tempfile::tempdir().unwrap();
    let c = legacy(dir.path(), true);
    c.execute_batch("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<20000)
        INSERT INTO messages(anchor_id,anchor_name,web_rid,session_id,sender_id,sender_name,message_type,content,message_time,received_at,time_source,message_id,message_time_epoch,received_at_epoch,payload_json)
        SELECT anchor_id,anchor_name,web_rid,session_id,sender_id,sender_name,message_type,content,message_time,received_at,time_source,'copy-'||x,message_time_epoch,received_at_epoch,payload_json FROM messages,n WHERE id=41;").unwrap();
    let before = state(&c);
    let schema = rows(&c, "SELECT * FROM sqlite_schema ORDER BY name");
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    assert!(matches!(
        ArchiveStore::open_cancellable(dir.path(), ArchiveOptions::default(), move || observed
            .fetch_add(1, Ordering::SeqCst)
            > 8),
        Err(ArchiveError::MigrationCancelled)
    ));
    assert!(calls.load(Ordering::SeqCst) > 8);
    assert_eq!(version(&c), 1);
    assert_eq!(state(&c), before);
    assert_eq!(
        rows(&c, "SELECT * FROM sqlite_schema ORDER BY name"),
        schema
    );
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    assert_eq!(version(&c), 3);
    assert_eq!(state(&c), before);
    assert!(store.close(Duration::from_secs(2)));
}
#[test]
fn future_database_is_rejected_without_rewriting() {
    let dir = tempfile::tempdir().unwrap();
    let c = legacy(dir.path(), true);
    c.execute_batch("PRAGMA user_version=4;").unwrap();
    let before = state(&c);
    let schema = rows(&c, "SELECT * FROM sqlite_schema ORDER BY name");
    assert!(ArchiveStore::open(dir.path(), ArchiveOptions::default()).is_err());
    assert_eq!(version(&c), 4);
    assert_eq!(state(&c), before);
    assert_eq!(
        rows(&c, "SELECT * FROM sqlite_schema ORDER BY name"),
        schema
    );
}

#[test]
fn foreign_key_validation_failure_preserves_the_original_archive() {
    let dir = tempfile::tempdir().unwrap();
    let c = legacy(dir.path(), true);
    c.execute_batch("PRAGMA foreign_keys=OFF;").unwrap();
    c.execute("DELETE FROM sessions WHERE session_id='101'", [])
        .unwrap();
    let before = state(&c);
    let schema = rows(&c, "SELECT * FROM sqlite_schema ORDER BY name");
    assert!(ArchiveStore::open(dir.path(), ArchiveOptions::default()).is_err());
    assert_eq!(version(&c), 1);
    assert_eq!(state(&c), before);
    assert_eq!(
        rows(&c, "SELECT * FROM sqlite_schema ORDER BY name"),
        schema
    );
}

#[test]
fn unknown_columns_are_not_silently_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let c = legacy(dir.path(), true);
    c.execute_batch("ALTER TABLE messages ADD COLUMN additional_evidence TEXT; UPDATE messages SET additional_evidence='preserve';").unwrap();
    let before = state(&c);
    assert!(ArchiveStore::open(dir.path(), ArchiveOptions::default()).is_err());
    assert_eq!(version(&c), 1);
    assert_eq!(state(&c), before);
}

#[test]
fn pre_cancelled_migration_keeps_v1_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let c = legacy(dir.path(), false);
    let before = state(&c);
    assert!(matches!(
        ArchiveStore::open_cancellable(dir.path(), ArchiveOptions::default(), || true),
        Err(ArchiveError::MigrationCancelled)
    ));
    assert_eq!(version(&c), 1);
    assert_eq!(state(&c), before);
}
