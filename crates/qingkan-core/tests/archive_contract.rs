use qingkan_core::archive::*;
use rusqlite::Connection;
use serde_json::json;
use std::{
    sync::{Arc, Barrier},
    thread,
    time::{Duration, Instant},
};
fn message(id: &str) -> MessageRecord {
    serde_json::from_value(json!({"anchor_id":"9007199254740993123","anchor_name":"主播","web_rid":"111","session_id":"7696456021958609673","sender_id":"00123456789012345678901234567890","sender_name":"观众😀","message_type":"chat","content":"内容","message_time":"2026-10-01T00:00:00.123+08:00","received_at":"2026-10-01T00:00:01.123+08:00","time_source":"server","message_id":id,"payload":{"content":"内容"}})).unwrap()
}
fn count(store: &ArchiveStore, month: &str) -> usize {
    let mut n = 0;
    store
        .snapshot()
        .unwrap()
        .visit_messages(None, month, None, |_| {
            n += 1;
            Ok(())
        })
        .unwrap();
    n
}
#[test]
fn schema_and_message_identity_are_compatible() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    assert!(store
        .record_message(message("1"), "标题".into(), None)
        .unwrap());
    assert!(!store
        .record_message(message("1"), "标题".into(), None)
        .unwrap());
    assert!(store
        .record_message(message(""), String::new(), None)
        .unwrap());
    assert!(store
        .record_message(message(""), String::new(), None)
        .unwrap());
    let conn = Connection::open(store.path()).unwrap();
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        conn.query_row("SELECT sender_id FROM messages LIMIT 1", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        message("1").sender_id
    );
    assert_eq!(count(&store, "2026-10"), 3);
    assert!(store.close(Duration::from_secs(2)));
}
#[test]
fn unknown_future_schema_is_rejected_without_rewriting() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Connection::open(dir.path().join("messages.sqlite3")).unwrap();
    conn.execute_batch("PRAGMA user_version=99").unwrap();
    drop(conn);
    assert!(ArchiveStore::open(dir.path(), ArchiveOptions::default()).is_err());
}
#[test]
fn snapshot_is_fixed_and_missing_server_time_uses_receipt_month() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    store
        .record_message(message("1"), String::new(), None)
        .unwrap();
    let snapshot = store.snapshot().unwrap();
    let mut missing = message("2");
    missing.message_time = String::new();
    missing.time_source = "missing".into();
    store.record_message(missing, String::new(), None).unwrap();
    let mut n = 0;
    snapshot
        .visit_messages(None, "2026-10", None, |_| {
            n += 1;
            Ok(())
        })
        .unwrap();
    assert_eq!(n, 1);
    assert_eq!(count(&store, "2026-10"), 2);
    assert_eq!(
        store
            .snapshot()
            .unwrap()
            .count_missing_time(None, Some("2026-10"), None)
            .unwrap(),
        1
    );
    assert!(store.close(Duration::from_secs(2)));
}
#[test]
fn invalid_identity_stats_and_timezone_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    for field in ["identity", "kind", "time"] {
        let mut row = message("1");
        match field {
            "identity" => row.anchor_id = "nickname".into(),
            "kind" => row.message_type = "stats".into(),
            _ => row.received_at = "2026-10-01T00:00:00".into(),
        }
        assert!(store.record_message(row, String::new(), None).is_err());
    }
    assert_eq!(count(&store, "2026-10"), 0);
}
#[test]
fn concurrent_producers_commit_before_return_and_close_drains() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    let gate = Arc::new(Barrier::new(21));
    let mut handles = vec![];
    for index in 0..20 {
        let s = store.clone();
        let g = gate.clone();
        handles.push(thread::spawn(move || {
            g.wait();
            for n in 0..20 {
                assert!(s
                    .record_message(message(&format!("{index}-{n}")), String::new(), None)
                    .unwrap());
            }
        }));
    }
    gate.wait();
    for h in handles {
        h.join().unwrap();
    }
    assert!(store.close(Duration::from_secs(3)));
    assert_eq!(count(&store, "2026-10"), 400);
    let metrics = store.metrics();
    assert_eq!(metrics["pending_requests"], 0);
    assert_eq!(metrics["committed_messages"], 400);
    assert!(metrics["largest_batch"].as_u64().unwrap() > 1);
    assert!(matches!(
        store.record_message(message("later"), String::new(), None),
        Err(ArchiveError::Closed)
    ));
}
#[test]
fn failed_batch_rolls_back_metadata_and_wakes_all_then_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(
        dir.path(),
        ArchiveOptions {
            batch_size: 2,
            batch_wait: Duration::from_secs(1),
            ..Default::default()
        },
    )
    .unwrap();
    let conn = Connection::open(store.path()).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_test BEFORE INSERT ON messages WHEN NEW.content='reject' BEGIN SELECT RAISE(ABORT,'test failure'); END").unwrap();
    let gate = Arc::new(Barrier::new(3));
    let mut handles = vec![];
    for content in ["valid", "reject"] {
        let s = store.clone();
        let g = gate.clone();
        let mut row = message(content);
        row.content = content.into();
        handles.push(thread::spawn(move || {
            g.wait();
            s.record_message(row, String::new(), None)
        }));
    }
    gate.wait();
    for handle in handles {
        assert!(handle.join().unwrap().is_err());
    }
    for table in ["anchors", "sessions", "messages"] {
        assert_eq!(
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(store.metrics()["pending_requests"], 0);
    assert!(store
        .record_message(message("after"), String::new(), None)
        .unwrap());
    assert!(store.close(Duration::from_secs(2)));
}
#[test]
fn close_timeout_retains_accepted_transaction_and_rejects_new_work() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    let conn = Connection::open(store.path()).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    let s = store.clone();
    let handle = thread::spawn(move || s.record_message(message("accepted"), String::new(), None));
    let deadline = Instant::now() + Duration::from_secs(2);
    while store.metrics()["pending_requests"] == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    assert!(!store.close(Duration::from_millis(10)));
    assert!(matches!(
        store.record_message(message("rejected"), String::new(), None),
        Err(ArchiveError::Closed)
    ));
    conn.execute_batch("ROLLBACK").unwrap();
    assert!(handle.join().unwrap().unwrap());
    assert!(store.close(Duration::from_secs(2)));
    assert_eq!(count(&store, "2026-10"), 1);
}
#[test]
fn existing_schema_reopens_and_session_start_is_not_replaced() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
        store
            .record_message(
                message("1"),
                "one".into(),
                Some("2026-09-30T23:00:00+08:00".into()),
            )
            .unwrap();
        assert!(store.close(Duration::from_secs(2)));
    }
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    let mut row = message("2");
    row.anchor_name = "新名字".into();
    row.web_rid = "222".into();
    store.record_message(row, "two".into(), None).unwrap();
    assert_eq!(store.list_anchors().unwrap()[0].anchor_name, "新名字");
    let conn = Connection::open(store.path()).unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row("SELECT started_at FROM sessions", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "2026-09-30T23:00:00+08:00"
    );
    assert!(store.close(Duration::from_secs(2)));
}
#[test]
fn month_boundaries_include_shanghai_midnight_and_reject_injection() {
    let (a, b) = month_bounds("2027-01").unwrap();
    assert_eq!(a, 1798732800.0);
    assert_eq!(b - a, 31.0 * 86400.0);
    for value in [
        "0000-01",
        "2027-1",
        "2027-00",
        "2027-13",
        "2027-01 OR 1=1",
        "9999-12",
    ] {
        assert!(month_bounds(value).is_err());
    }
}
