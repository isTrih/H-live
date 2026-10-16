use qingkan_core::archive::*;
use rusqlite::Connection;
use std::{thread, time::Duration};

#[test]
fn checkpoints_are_persistent_monotonic_and_isolated_by_real_session() {
    let directory = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(directory.path(), ArchiveOptions::default()).unwrap();
    assert_eq!(store.load_monitor_duration("11", "101").unwrap(), 0.0);
    store.save_monitor_duration("11", "101", 10.125).unwrap();
    store.save_monitor_duration("11", "101", 2.0).unwrap();
    assert_eq!(store.load_monitor_duration("11", "101").unwrap(), 10.125);
    assert_eq!(store.load_monitor_duration("11", "102").unwrap(), 0.0);
    assert_eq!(store.load_monitor_duration("22", "101").unwrap(), 0.0);
    assert!(store.close(Duration::from_secs(2)));
    assert!(matches!(
        store.save_monitor_duration("11", "101", 20.0),
        Err(ArchiveError::Closed)
    ));
    let reopened = ArchiveStore::open(directory.path(), ArchiveOptions::default()).unwrap();
    assert_eq!(reopened.load_monitor_duration("11", "101").unwrap(), 10.125);
    // No wall-clock delta is added by reopening, even for very old checkpoints.
    let c = Connection::open(reopened.path()).unwrap();
    c.execute(
        "UPDATE session_monitor_durations SET last_checkpoint_at='2000-01-01T00:00:00Z'",
        [],
    )
    .unwrap();
    assert_eq!(reopened.load_monitor_duration("11", "101").unwrap(), 10.125);
    let filter = ArchiveDeleteFilter {
        anchor_ids: vec!["11".into()],
        month: None,
        session_ids: None,
    };
    assert_eq!(
        reopened.execute_delete(&filter).unwrap()["deleted_records"],
        0
    );
    assert_eq!(reopened.load_monitor_duration("11", "101").unwrap(), 10.125);
    assert!(reopened.close(Duration::from_secs(2)));
}

#[test]
fn invalid_checkpoints_do_not_create_rows() {
    let directory = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(directory.path(), ArchiveOptions::default()).unwrap();
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
        assert!(store.save_monitor_duration("11", "101", value).is_err());
    }
    for (anchor, session) in [("", "101"), ("11", "0"), ("name", "101")] {
        assert!(store.save_monitor_duration(anchor, session, 1.0).is_err());
        assert!(store.load_monitor_duration(anchor, session).is_err());
    }
    assert_eq!(store.load_monitor_duration("11", "101").unwrap(), 0.0);
    assert!(store.list_anchors().unwrap().is_empty());
}

#[test]
fn concurrent_out_of_order_checkpoints_cannot_move_time_backwards() {
    let directory = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(directory.path(), ArchiveOptions::default()).unwrap();
    let handles: Vec<_> = (1..=20)
        .map(|seconds| {
            let copy = store.clone();
            thread::spawn(move || {
                copy.save_monitor_duration("11", "101", seconds as f64)
                    .unwrap()
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    assert_eq!(store.load_monitor_duration("11", "101").unwrap(), 20.0);
}

#[test]
fn earlier_workspace_v2_gets_additive_table_without_touching_messages() {
    let directory = tempfile::tempdir().unwrap();
    let c = Connection::open(directory.path().join("messages.sqlite3")).unwrap();
    c.execute_batch(include_str!("fixtures/archive_v1.sql"))
        .unwrap();
    c.execute_batch("PRAGMA user_version=2;").unwrap();
    let table: String = c
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name='messages'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let store = ArchiveStore::open(directory.path(), ArchiveOptions::default()).unwrap();
    store.save_monitor_duration("11", "101", 3.0).unwrap();
    assert_eq!(store.load_monitor_duration("11", "101").unwrap(), 3.0);
    assert_eq!(
        c.query_row(
            "SELECT sql FROM sqlite_schema WHERE name='messages'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        table
    );
    assert_eq!(
        c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
}
