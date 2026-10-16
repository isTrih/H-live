use qingkan_core::archive::*;
use rusqlite::Connection;
use serde_json::json;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

#[test]
fn cancellation_interrupts_sql_and_preserves_the_entire_archive() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    seed(&store);
    let c = Connection::open(store.path()).unwrap();
    c.execute_batch("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<20000) INSERT INTO messages(anchor_id,anchor_name,web_rid,session_id,sender_id,sender_name,message_type,content,message_time,received_at,time_source,message_id,message_time_epoch,received_at_epoch,payload_json) SELECT anchor_id,anchor_name,web_rid,session_id,sender_id,sender_name,message_type,content,message_time,received_at,time_source,'copy-'||x,message_time_epoch,received_at_epoch,payload_json FROM messages,n WHERE id=1;").unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let copy = calls.clone();
    assert!(matches!(
        store.execute_delete_cancellable(&selection(&["11"], None, None), move || copy
            .fetch_add(1, Ordering::SeqCst)
            > 5),
        Err(ArchiveError::DeleteCancelled)
    ));
    assert!(calls.load(Ordering::SeqCst) > 5);
    assert_eq!(
        store
            .preview_delete(&selection(&["11"], None, None))
            .unwrap()["records_count"],
        20003
    );
    assert!(!c
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .exists([])
        .unwrap());
    assert!(store.close(Duration::from_secs(2)));
}

#[test]
fn cancellation_while_waiting_for_write_barrier_does_not_delete_accepted_data() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    let c = Connection::open(store.path()).unwrap();
    c.execute_batch("BEGIN IMMEDIATE;").unwrap();
    let copy = store.clone();
    let writing = thread::spawn(move || {
        copy.record_message(message("11", "101", "late", ""), String::new(), None)
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while store.metrics()["pending_requests"] == 0 {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let copy = store.clone();
    let deleting = thread::spawn(move || {
        copy.execute_delete_cancellable(&selection(&["11"], None, None), move || {
            flag.load(Ordering::Acquire)
        })
    });
    cancelled.store(true, Ordering::Release);
    assert!(matches!(
        deleting.join().unwrap(),
        Err(ArchiveError::DeleteCancelled)
    ));
    c.execute_batch("COMMIT;").unwrap();
    assert!(writing.join().unwrap().unwrap());
    assert_eq!(
        store
            .preview_delete(&selection(&["11"], None, None))
            .unwrap()["records_count"],
        1
    );
    assert!(store.close(Duration::from_secs(2)));
}

fn message(anchor: &str, session: &str, id: &str, time: &str) -> MessageRecord {
    serde_json::from_value(json!({"anchor_id":anchor,"anchor_name":"主播","web_rid":"111","session_id":session,"message_type":"chat","content":"synthetic","message_time":time,"received_at":"2026-10-31T23:59:59.999+08:00","time_source":if time.is_empty(){"missing"}else{"server"},"message_id":id})).unwrap()
}
fn selection(ids: &[&str], month: Option<&str>, sessions: Option<&[&str]>) -> ArchiveDeleteFilter {
    ArchiveDeleteFilter {
        anchor_ids: ids.iter().map(|v| v.to_string()).collect(),
        month: month.map(str::to_owned),
        session_ids: sessions.map(|v| v.iter().map(|v| v.to_string()).collect()),
    }
}
fn seed(store: &ArchiveStore) {
    for row in [
        message("11", "101", "1", "2026-10-01T00:00:00+08:00"),
        message("11", "101", "2", "2026-11-01T00:00:00+08:00"),
        message("11", "102", "3", ""),
        message("22", "101", "4", "2026-10-02T00:00:00+08:00"),
    ] {
        assert!(store.record_message(row, "title".into(), None).unwrap());
    }
}

#[test]
fn month_and_sessions_intersect_and_preserve_other_anchors_and_months() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    seed(&store);
    let filter = selection(&["11"], Some("2026-10"), Some(&["101"]));
    let preview = store.preview_delete(&filter).unwrap();
    assert_eq!(preview["records_count"], 1);
    assert_eq!(preview["sessions_count"], 1);
    let result = store.execute_delete(&filter).unwrap();
    assert_eq!(result["deleted_records"], 1);
    assert_eq!(result["deleted_sessions"], 0);
    assert_eq!(result["remaining_records"], 3);
    assert_eq!(
        store
            .preview_delete(&selection(&["11"], Some("2026-11"), None))
            .unwrap()["records_count"],
        1
    );
    assert_eq!(
        store
            .preview_delete(&selection(&["22"], None, None))
            .unwrap()["records_count"],
        1
    );
    assert_eq!(
        store
            .preview_delete(&selection(&["11"], Some("2026-10"), None))
            .unwrap()["records_count"],
        1
    );
    assert!(store.close(Duration::from_secs(2)));
}

#[test]
fn whole_anchor_cleanup_preserves_foreign_keys_and_releases_dedup_keys() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    seed(&store);
    let result = store
        .execute_delete(&selection(&["11"], None, None))
        .unwrap();
    assert_eq!(result["deleted_records"], 3);
    assert_eq!(result["deleted_sessions"], 2);
    assert_eq!(store.list_anchors().unwrap().len(), 1);
    let c = Connection::open(store.path()).unwrap();
    assert!(!c
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .exists([])
        .unwrap());
    assert!(store
        .record_message(
            message("11", "101", "1", "2026-10-01T00:00:00+08:00"),
            String::new(),
            None
        )
        .unwrap());
    assert!(!store
        .record_message(
            message("11", "101", "1", "2026-10-01T00:00:00+08:00"),
            String::new(),
            None
        )
        .unwrap());
    assert!(store.close(Duration::from_secs(2)));
}

#[test]
fn destructive_filters_reject_missing_empty_invalid_or_oversized_selections() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    seed(&store);
    for filter in [
        selection(&[], None, None),
        selection(&["0"], None, None),
        selection(&["11"], Some("2026-13"), None),
        selection(&["11"], None, Some(&[])),
        selection(&["11"], None, Some(&["1 OR 1=1"])),
        selection(&vec!["11"; 501], None, None),
    ] {
        assert!(store.preview_delete(&filter).is_err());
        assert!(store.execute_delete(&filter).is_err());
    }
    assert!(serde_json::from_str::<ArchiveDeleteFilter>("{}").is_err());
    assert!(serde_json::from_str::<ArchiveDeleteFilter>(
        r#"{"anchor_ids":["11"],"session_id":"101"}"#
    )
    .is_err());
    assert_eq!(
        store
            .preview_delete(&selection(&["11", "22"], None, None))
            .unwrap()["records_count"],
        4
    );
    assert!(store.close(Duration::from_secs(2)));
}

#[test]
fn session_listing_is_filtered_counted_and_paginated_without_inventing_start_time() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    seed(&store);
    let filter = selection(&["11"], Some("2026-10"), None);
    let first = store.list_sessions(&filter, 1, 0).unwrap();
    let second = store.list_sessions(&filter, 1, 1).unwrap();
    assert_eq!(first["total"], 2);
    assert_eq!(first["next_offset"], 1);
    assert_eq!(second["next_offset"], json!(null));
    assert_ne!(
        first["sessions"][0]["session_id"],
        second["sessions"][0]["session_id"]
    );
    assert_eq!(first["sessions"][0]["records_count"], 1);
    assert_eq!(first["sessions"][0]["started_at"], json!(null));
    assert!(store.list_sessions(&filter, 0, 0).is_err());
    assert!(store.close(Duration::from_secs(2)));
}

#[test]
fn a_failed_delete_rolls_back_messages_and_metadata_together() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    seed(&store);
    let c = Connection::open(store.path()).unwrap();
    c.execute_batch("CREATE TRIGGER reject_session_delete BEFORE DELETE ON sessions BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(store
        .execute_delete(&selection(&["11"], None, None))
        .is_err());
    assert_eq!(
        store
            .preview_delete(&selection(&["11"], None, None))
            .unwrap()["records_count"],
        3
    );
    assert_eq!(store.list_anchors().unwrap().len(), 2);
    assert!(store.close(Duration::from_secs(2)));
}

#[test]
fn delete_waits_for_accepted_writes_then_removes_them_before_returning() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    let c = Connection::open(store.path()).unwrap();
    c.execute_batch("BEGIN IMMEDIATE;").unwrap();
    let writer = store.clone();
    let writing = thread::spawn(move || {
        writer.record_message(message("11", "101", "late", ""), String::new(), None)
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    while store.metrics()["pending_requests"] == 0 {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    let deleting = store.clone();
    let deleting = thread::spawn(move || deleting.execute_delete(&selection(&["11"], None, None)));
    thread::sleep(Duration::from_millis(40));
    assert!(!deleting.is_finished());
    c.execute_batch("COMMIT;").unwrap();
    assert!(writing.join().unwrap().unwrap());
    assert_eq!(deleting.join().unwrap().unwrap()["deleted_records"], 1);
    assert_eq!(
        store
            .preview_delete(&selection(&["11"], None, None))
            .unwrap()["records_count"],
        0
    );
    assert!(store.close(Duration::from_secs(2)));
    assert!(matches!(
        store.execute_delete(&selection(&["11"], None, None)),
        Err(ArchiveError::Closed)
    ));
}
