use qingkan_core::archive::*;
use serde_json::json;

fn message(id: usize, kind: &str) -> MessageRecord {
    serde_json::from_value(json!({"anchor_id":"11","anchor_name":"主播甲","web_rid":"111","session_id":"101","sender_id":"9007199254740993123","sender_name":"观众","message_type":kind,"content":format!("{kind}-{id}"),"message_id":id.to_string(),"message_time":"2026-10-16T12:00:00+08:00","received_at":"2026-10-16T12:00:01+08:00","time_source":"server","payload":{"activity_kind":"lottery","unrelated":"not exposed"}})).unwrap()
}
#[test]
fn rare_types_remain_available_after_entry_flood_and_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    for (i, kind) in ["gift", "activity", "emoji", "fansclub", "social"]
        .iter()
        .enumerate()
    {
        store
            .record_message(message(i, kind), String::new(), None)
            .unwrap();
    }
    for i in 5..1210 {
        store
            .record_message(message(i, "member"), String::new(), None)
            .unwrap();
    }
    for kind in ["gift", "activity", "emoji", "fansclub", "social"] {
        let result = store
            .recent_messages(json!({"type":kind,"web_rid":"111","limit":150}))
            .unwrap();
        assert_eq!(result["messages"].as_array().unwrap().len(), 1);
        let row = &result["messages"][0];
        assert_eq!(row["message_type"], kind);
        assert_eq!(row["rid"], "111");
        assert_eq!(row["sender_id"], "9007199254740993123");
        assert!(row["archive_id"].is_string());
        assert!(row["payload"]["unrelated"].is_null());
    }
    let all = store.recent_messages(json!({})).unwrap();
    assert_eq!(all["messages"].as_array().unwrap().len(), 4);
    assert!(all["messages"]
        .as_array()
        .unwrap()
        .iter()
        .all(|m| m["message_type"] != "member" && m["message_type"] != "gift"));
    let entries = store.recent_messages(json!({"type":"member"})).unwrap();
    assert_eq!(entries["messages"].as_array().unwrap().len(), 150);
    assert_eq!(entries["messages"][0]["content"], "member-1209");
    assert_eq!(all["high_watermark"], "1210");
    let paused = store.recent_messages(json!({"before_id":"5"})).unwrap();
    assert_eq!(paused["messages"].as_array().unwrap().len(), 4);
    assert_eq!(paused["messages"][0]["message_type"], "social");
    assert!(
        store.recent_messages(json!({"before_id":"0"})).unwrap()["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(store
        .recent_messages(json!({"type":"gift","web_rid":"222"}))
        .unwrap()["messages"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(store.close(std::time::Duration::from_secs(2)));
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    assert_eq!(
        store.recent_messages(json!({"type":"activity"})).unwrap()["messages"][0]["payload"]
            ["activity_kind"],
        "lottery"
    );
}
#[test]
fn bounded_preview_preserves_archive_and_rejects_invalid_filters() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    let mut row = message(1, "gift");
    row.content = "长文本😀".repeat(10000);
    store.record_message(row, String::new(), None).unwrap();
    let result = store.recent_messages(json!({"type":"gift"})).unwrap();
    assert_eq!(result["messages"][0]["content_truncated"], true);
    assert!(result.to_string().len() < 16384);
    for p in [
        json!({"limit":151}),
        json!({"limit":true}),
        json!({"type":"unknown"}),
        json!({"web_rid":"1' OR 1=1"}),
        json!({"before_id":true}),
        json!({"before_id":"9223372036854775808"}),
        json!({"before_id":"-1"}),
    ] {
        assert!(store.recent_messages(p).is_err());
    }
    let mut length = 0;
    store
        .snapshot()
        .unwrap()
        .visit_messages(None, "2026-10", None, |row| {
            length = row.record.content.chars().count();
            Ok(())
        })
        .unwrap();
    assert_eq!(length, 40000);
    let before = store.recent_messages(json!({"type":"gift"})).unwrap();
    assert_eq!(before["messages"].as_array().unwrap().len(), 1);
    store
        .execute_delete(&ArchiveDeleteFilter {
            anchor_ids: vec!["11".into()],
            month: None,
            session_ids: None,
        })
        .unwrap();
    assert!(
        store.recent_messages(json!({"type":"gift"})).unwrap()["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
