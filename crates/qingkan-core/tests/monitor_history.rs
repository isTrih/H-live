use qingkan_core::{archive::*, export::CancelToken};
use rusqlite::{Connection, params};
use serde_json::json;
use std::time::Duration;

fn register(store: &ArchiveStore, rid: &str, anchor: &str, session: &str, name: &str) {
    store
        .quality_request(
            "register",
            json!({"web_rid":rid,"anchor_id":anchor,"session_id":session,"anchor_name":name}),
            CancelToken::default(),
        )
        .unwrap();
}

fn put(store: &ArchiveStore, rid: &str, anchor: &str, session: &str, message: &str) {
    let record = serde_json::from_value(json!({
        "web_rid":rid,"anchor_id":anchor,"anchor_name":"主播旧名","session_id":session,
        "sender_id":"9","sender_name":"观众","message_type":"chat","content":"谢谢",
        "message_time":"2026-10-23T14:00:00+08:00","received_at":"2026-10-23T14:00:01+08:00",
        "time_source":"server","message_id":message,"payload":{}
    }))
    .unwrap();
    store.record_message(record, String::new(), None).unwrap();
}

fn history(store: &ArchiveStore) -> Vec<(String, String, String, String)> {
    store
        .monitor_history()
        .unwrap()
        .into_iter()
        .map(|row| (row.web_rid, row.anchor_id, row.session_id, row.anchor_name))
        .collect()
}

#[test]
fn zero_message_and_cleared_sessions_remain_recoverable_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(directory.path(), ArchiveOptions::default()).unwrap();
    register(&store, "111", "11", "101", "零消息主播");
    register(&store, "222", "22", "202", "已清理主播");
    store.save_monitor_duration("11", "101", 35.5).unwrap();
    store.save_monitor_duration("22", "202", 60.0).unwrap();
    put(&store, "222", "22", "202", "1");
    store
        .execute_delete(&ArchiveDeleteFilter {
            anchor_ids: vec!["22".into()],
            month: None,
            session_ids: None,
        })
        .unwrap();
    assert!(store.list_anchors().unwrap().is_empty());
    assert!(store.close(Duration::from_secs(2)));
    let store = ArchiveStore::open(directory.path(), ArchiveOptions::default()).unwrap();
    assert_eq!(
        history(&store),
        vec![
            ("111".into(), "11".into(), "101".into(), "零消息主播".into()),
            ("222".into(), "22".into(), "202".into(), "已清理主播".into()),
        ]
    );
    assert_eq!(store.load_monitor_duration("11", "101").unwrap(), 35.5);
    assert_eq!(store.load_monitor_duration("22", "202").unwrap(), 60.0);
}

#[test]
fn registered_exact_aliases_follow_latest_real_anchor_session_without_scanning_messages() {
    let directory = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(directory.path(), ArchiveOptions::default()).unwrap();
    put(&store, "9999", "11", "999", "old-message-only-alias");
    put(&store, "222", "11", "999", "alias-changed");
    register(&store, "111", "11", "998", "主播旧名");
    register(&store, "222", "11", "999", "主播旧名");
    register(&store, "333", "11", "101", "主播新名");
    let c = Connection::open(store.path()).unwrap();
    c.execute("UPDATE quality_registry SET first_seen='2026-10-22T00:00:00+08:00',last_seen='2026-10-22T01:00:00+08:00' WHERE session_id!='101'", []).unwrap();
    c.execute("UPDATE sessions SET first_seen='2026-10-22T00:00:00+08:00',last_seen='2026-10-22T01:00:00+08:00'", []).unwrap();
    c.execute("UPDATE quality_registry SET first_seen='2026-10-23T00:00:00+08:00',last_seen='2026-10-23T01:00:00+08:00' WHERE session_id='101'", []).unwrap();
    assert_eq!(
        history(&store),
        vec![
            ("111".into(), "11".into(), "101".into(), "主播新名".into()),
            ("222".into(), "11".into(), "101".into(), "主播新名".into()),
            ("333".into(), "11".into(), "101".into(), "主播新名".into()),
        ]
    );
}

#[test]
fn shared_room_ids_keep_ambiguous_real_anchors_and_ignore_equal_nicknames() {
    let directory = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(directory.path(), ArchiveOptions::default()).unwrap();
    register(&store, "111", "11", "101", "相同名字");
    register(&store, "111", "22", "202", "相同名字");
    register(&store, "333", "33", "303", "相同名字");
    assert_eq!(
        history(&store),
        vec![
            ("111".into(), "11".into(), "101".into(), "相同名字".into()),
            ("111".into(), "22".into(), "202".into(), "相同名字".into()),
            ("333".into(), "33".into(), "303".into(), "相同名字".into()),
        ]
    );
}

#[test]
fn legacy_session_metadata_is_recovered_without_registering_or_mutating_it() {
    let directory = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(directory.path(), ArchiveOptions::default()).unwrap();
    put(&store, "111", "11", "101", "1");
    let c = Connection::open(store.path()).unwrap();
    c.execute("DELETE FROM quality_registry", []).unwrap();
    assert_eq!(
        history(&store),
        vec![("111".into(), "11".into(), "101".into(), "主播旧名".into())]
    );
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM quality_registry", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn timestamps_determine_latest_session_and_invalid_ids_cannot_supply_aliases() {
    let directory = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(directory.path(), ArchiveOptions::default()).unwrap();
    let c = Connection::open(store.path()).unwrap();
    for (rid, anchor, session, first, last) in [
        (
            "111",
            "11",
            "999",
            "2026-10-23T00:00:00Z",
            "2026-10-23T08:30:00+08:00",
        ),
        (
            "111",
            "11",
            "101",
            "2026-10-23T00:00:00Z",
            "2026-10-23T01:00:00Z",
        ),
        (
            "111",
            "11",
            "102",
            "2026-10-23T00:30:00Z",
            "2026-10-23T01:00:00Z",
        ),
        (
            "111",
            "11",
            "103",
            "2026-10-23T00:30:00Z",
            "2026-10-23T01:00:00Z",
        ),
        (
            "not-a-rid",
            "11",
            "104",
            "2026-10-23T02:00:00Z",
            "2026-10-23T03:00:00Z",
        ),
        (
            "111",
            "invalid",
            "105",
            "2026-10-23T02:00:00Z",
            "2026-10-23T03:00:00Z",
        ),
        (
            "111",
            "11",
            "0",
            "2026-10-23T02:00:00Z",
            "2026-10-23T03:00:00Z",
        ),
        (
            "000",
            "44",
            "404",
            "2026-10-23T02:00:00Z",
            "2026-10-23T03:00:00Z",
        ),
    ] {
        c.execute("INSERT INTO quality_registry(anchor_id,session_id,web_rid,anchor_name,title,first_seen,last_seen) VALUES(?,?,?,'主播','','' || ?,?)", params![anchor,session,rid,first,last]).unwrap();
    }
    assert_eq!(
        history(&store),
        vec![("111".into(), "11".into(), "103".into(), "主播".into())]
    );
}
