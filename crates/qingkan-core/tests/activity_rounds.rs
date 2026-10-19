use qingkan_core::archive::{ArchiveOptions, ArchiveStore, MessageRecord};
use rusqlite::Connection;
use serde_json::json;
use std::time::Duration;

fn packet(message: &str, stage: &str, remaining: u64) -> MessageRecord {
    serde_json::from_value(json!({
        "anchor_id":"11","anchor_name":"合成主播","web_rid":"111","session_id":"101",
        "message_type":"activity","message_id":message,"content":"红包预告；剩余份数变化",
        "message_time":"2026-10-19T12:00:00+08:00","received_at":"2026-10-19T12:00:01+08:00","time_source":"server",
        "payload":{"activity_kind":"red_packet","activity_id":"9876543210123456789","activity_stage":stage,
            "activity_details":{"remaining_count":remaining,"total_count":10}}
    })).unwrap()
}

#[test]
fn same_red_packet_preview_updates_and_finish_are_one_archive_record() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    assert!(store
        .record_message(packet("preview-1", "preview", 10), String::new(), None)
        .unwrap());
    assert!(
        !store
            .record_message(packet("preview-2", "preview", 9), String::new(), None)
            .unwrap(),
        "one red packet must not be counted again when remaining count changes"
    );
    assert!(!store
        .record_message(packet("finish-3", "ended", 0), String::new(), None)
        .unwrap());
    let c = Connection::open(store.path()).unwrap();
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM messages WHERE message_type='activity'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert!(store.close(Duration::from_secs(2)));
}

#[test]
fn lifecycle_counts_distinct_packets_and_never_reopens_an_ended_packet() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    let now = 1792382400;
    let mut first = packet("p1", "preview", 10);
    first.payload["activity_details"]["expires_at"] = json!((now + 60).to_string());
    assert!(store
        .record_message(first.clone(), String::new(), None)
        .unwrap());
    let s = store.activity_summary("11", "101", now).unwrap();
    assert_eq!(s["red_packet"]["total"], 1);
    assert_eq!(s["red_packet"]["active_until"], now + 60);
    assert_eq!(s["red_packet"]["uncertain"], false);
    let mut reward = first.clone();
    reward.message_id = "reward".into();
    reward.payload["activity_stage"] = json!("reward");
    assert!(!store.record_message(reward, String::new(), None).unwrap());
    assert_eq!(
        store.activity_summary("11", "101", now).unwrap()["red_packet"]["active_until"],
        now + 60
    );
    let mut other = first.clone();
    other.message_id = "different".into();
    other.payload["activity_id"] = json!("another-packet-same-title-amount-and-expiry");
    assert!(store.record_message(other, String::new(), None).unwrap());
    let mut end = first.clone();
    end.message_id = "end".into();
    end.payload["activity_stage"] = json!("ended");
    assert!(!store.record_message(end, String::new(), None).unwrap());
    first.message_id = "late-preview".into();
    assert!(!store.record_message(first, String::new(), None).unwrap());
    let s = store.activity_summary("11", "101", now + 61).unwrap();
    assert_eq!(s["red_packet"]["total"], 2);
    assert!(s["red_packet"]["active_until"].is_null());
    assert_eq!(s["red_packet"]["uncertain"], false);
    assert!(store.close(Duration::from_secs(2)));
    let reopened = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    assert_eq!(
        reopened.activity_summary("11", "101", now).unwrap()["red_packet"]["total"],
        2
    );
    assert!(reopened.close(Duration::from_secs(2)));
}

#[test]
fn identities_are_scoped_unknown_notices_are_not_guessed_and_deletion_cascades() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    for i in 0..4 {
        let mut r = packet(&format!("scope-{i}"), "notice", 10);
        match i {
            1 => r.anchor_id = "12".into(),
            2 => r.session_id = "102".into(),
            3 => r.payload["activity_kind"] = json!("lottery"),
            _ => (),
        }
        assert!(store.record_message(r, String::new(), None).unwrap());
    }
    let mut r = packet("unidentified", "notice", 10);
    r.payload["activity_id"] = json!("");
    assert!(store.record_message(r, String::new(), None).unwrap());
    let s = store.activity_summary("11", "101", 1792382400).unwrap();
    assert_eq!(s["red_packet"]["total"], 1);
    assert_eq!(s["lottery"]["total"], 1);
    assert_eq!(s["red_packet"]["unidentified"], true);
    assert_eq!(s["red_packet"]["uncertain"], true);
    assert!(s["red_packet"]["active_until"].is_null());
    store
        .execute_delete(&qingkan_core::archive::ArchiveDeleteFilter {
            anchor_ids: vec!["11".into()],
            month: None,
            session_ids: Some(vec!["101".into()]),
        })
        .unwrap();
    let c = Connection::open(store.path()).unwrap();
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM activity_aliases WHERE anchor_id='11' AND session_id='101'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        store.activity_summary("11", "101", 1792382400).unwrap()["red_packet"]["total"],
        0
    );
    assert!(store.close(Duration::from_secs(2)));
}

#[test]
fn migration_consolidates_historical_packet_updates_and_keeps_first_timestamp() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    let path = store.path().to_owned();
    store
        .record_message(packet("original", "preview", 10), String::new(), None)
        .unwrap();
    assert!(store.close(Duration::from_secs(2)));
    let c = Connection::open(&path).unwrap();
    c.execute_batch("DROP TABLE activity_aliases; DROP TABLE activity_rounds;")
        .unwrap();
    c.execute("INSERT INTO messages(anchor_id,anchor_name,web_rid,session_id,sender_id,sender_name,message_type,content,message_time,received_at,time_source,message_id,payload_json,message_time_epoch,received_at_epoch) SELECT anchor_id,anchor_name,web_rid,session_id,sender_id,sender_name,message_type,content,'2026-10-19T12:01:00+08:00',received_at,time_source,'later',json_set(payload_json,'$.activity_stage','ended'),message_time_epoch,received_at_epoch FROM messages",[]).unwrap();
    drop(c);
    let reopened = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    let c = Connection::open(&path).unwrap();
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        c.query_row("SELECT message_id FROM messages", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "original"
    );
    let s = reopened.activity_summary("11", "101", 1792382400).unwrap();
    assert_eq!(s["red_packet"]["total"], 1);
    assert_eq!(s["red_packet"]["uncertain"], false);
    assert!(reopened.close(Duration::from_secs(2)));
}

#[test]
fn lottery_draw_can_bridge_id_only_and_time_only_notices_and_remember_both_ids() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    let mut first = packet("id-only", "notice", 10);
    first.payload["activity_kind"] = json!("lottery");
    assert!(store
        .record_message(first.clone(), String::new(), None)
        .unwrap());
    let mut second = first.clone();
    second.message_id = "draw-only".into();
    second.payload["activity_id"] = json!("alias-second");
    second.payload["activity_details"]["draw_at"] = json!(1792382460);
    assert!(store
        .record_message(second.clone(), String::new(), None)
        .unwrap());
    first.message_id = "bridge".into();
    first.payload["activity_details"]["draw_at"] = json!(1792382460000_i64);
    assert!(!store.record_message(first, String::new(), None).unwrap());
    second.message_id = "late-result".into();
    second.payload["activity_stage"] = json!("draw_result");
    second.payload["activity_details"] = json!({});
    assert!(!store.record_message(second, String::new(), None).unwrap());
    let s = store.activity_summary("11", "101", 1792382400).unwrap();
    assert_eq!(s["lottery"]["total"], 1);
    assert!(s["lottery"]["active_until"].is_null());
    assert_eq!(s["lottery"]["uncertain"], false);
    assert!(store.close(Duration::from_secs(2)));
}
