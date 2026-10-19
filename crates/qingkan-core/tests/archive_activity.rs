use qingkan_core::archive::*;
use qingkan_core::export::{export_month, CancelToken, ExportOptions};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::time::Duration;

fn activity(id: &str) -> MessageRecord {
    serde_json::from_value(json!({"anchor_id":"11","anchor_name":"主播","web_rid":"111",
        "session_id":"101","message_type":"activity","content":"福袋：礼物 3 份",
        "message_time":"2026-10-01T00:00:00+08:00","received_at":"2026-10-01T00:00:01+08:00",
        "time_source":"server","message_id":id,
        "payload":{"activity_kind":"lottery","activity_id":"001234567890123456789","prize_count":3}})).unwrap()
}

#[test]
fn same_draw_lottery_with_distinct_notifications_is_stored_once() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    let mut first = activity("notice-1");
    first.payload["activity_details"] = json!({"draw_at":1790784300});
    let mut repeat = first.clone();
    repeat.message_id = "sync-2".into();
    assert!(store.record_message(first, String::new(), None).unwrap());
    assert!(!store.record_message(repeat, String::new(), None).unwrap());
    let c = Connection::open(store.path()).unwrap();
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(store.close(Duration::from_secs(2)));
}

#[test]
fn lottery_draw_dedupe_is_scoped_persistent_and_does_not_guess_unknown_times() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    let mut base = activity("");
    base.payload["activity_details"] = json!({"draw_at":1790784300});
    assert!(store
        .record_message(base.clone(), String::new(), None)
        .unwrap());
    assert!(!store
        .record_message(base.clone(), String::new(), None)
        .unwrap());
    let mut ms = base.clone();
    ms.payload["activity_details"]["draw_at"] = json!(1790784300000_i64);
    assert!(!store.record_message(ms, String::new(), None).unwrap());
    for i in 0..8 {
        let mut separate = base.clone();
        separate.message_id = format!("separate-{i}");
        match i {
            0 => separate.anchor_id = "12".into(),
            1 => separate.session_id = "102".into(),
            2 => {
                separate.payload["activity_details"]["draw_at"] = json!(1790784301);
                separate.payload["activity_id"] = json!("another-round");
            }
            3 | 4 => {
                separate.payload["activity_kind"] = json!("red_packet");
                separate.payload["activity_id"] = json!(format!("packet-{i}"));
            }
            5 => {
                separate.payload["activity_details"]["draw_at"] = Value::Null;
                separate.payload["activity_id"] = json!("");
            }
            6 => {
                separate.payload["activity_details"]["draw_at"] = json!(0);
                separate.payload["activity_id"] = json!("");
            }
            _ => separate.message_type = "chat".into(),
        }
        assert!(store.record_message(separate, String::new(), None).unwrap());
    }
    assert!(store.close(Duration::from_secs(2)));
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    base.message_id = "after-restart".into();
    assert!(!store
        .record_message(base.clone(), String::new(), None)
        .unwrap());
    let c = Connection::open(store.path()).unwrap();
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        9
    );
    drop(c);
    store
        .execute_delete(&ArchiveDeleteFilter {
            anchor_ids: vec!["11".into()],
            month: None,
            session_ids: Some(vec!["101".into()]),
        })
        .unwrap();
    assert!(store.record_message(base, String::new(), None).unwrap());
    assert!(store.close(Duration::from_secs(2)));
}

#[test]
fn lottery_enrichment_keeps_details_and_updates_already_analyzed_participation() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    let draw = chrono::DateTime::parse_from_rfc3339("2026-10-01T00:05:00+08:00")
        .unwrap()
        .timestamp();
    let mut first = activity("sync");
    first.payload["activity_details"] =
        json!({"start_at":draw-300,"draw_at":draw,"winner_count":3});
    store
        .record_message(first.clone(), String::new(), None)
        .unwrap();
    let mut chat = activity("chat");
    chat.message_type = "chat".into();
    chat.sender_id = "123".into();
    chat.content = "领取福利".into();
    store.record_message(chat, String::new(), None).unwrap();
    let drain = || {
        for _ in 0..30 {
            if store
                .quality_request("step", json!({}), CancelToken::default())
                .unwrap()["processed"]
                == 0
            {
                return;
            }
        }
        panic!("analysis did not drain");
    };
    drain();
    let overview = || {
        store
            .quality_request(
                "detail",
                json!({"anchor_id":"11","session_id":"101"}),
                CancelToken::default(),
            )
            .unwrap()["overview"]
            .clone()
    };
    assert_eq!(overview()["valid_speakers"], 1);
    let mut detail = first.clone();
    detail.message_id = "notice".into();
    detail.payload["activity_details"] = json!({"draw_at":draw,"prize_name":"可爱玩偶","conditions":[{"content":"发送评论「领取福利」"}]});
    assert!(!store.record_message(detail, String::new(), None).unwrap());
    // A later sparse sync cannot erase known prize/conditions.
    first.message_id = "sparse".into();
    first.payload["activity_details"]["prize_name"] = json!("");
    assert!(!store.record_message(first, String::new(), None).unwrap());
    drain();
    assert_eq!(overview()["valid_speakers"], 0);
    let c = Connection::open(store.path()).unwrap();
    let (payload, content): (String, String) = c
        .query_row(
            "SELECT payload_json,content FROM messages WHERE message_type='activity'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let p: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(p["activity_details"]["winner_count"], 3);
    assert_eq!(
        p["activity_details"]["conditions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(content.contains("可爱玩偶") && content.contains("领取福利"));
    assert!(store.close(Duration::from_secs(2)));
}

#[test]
fn existing_lottery_duplicates_are_merged_transactionally_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    let mut a = activity("first");
    a.payload["activity_details"] = json!({"draw_at":1790784300,"winner_count":2});
    store.record_message(a, String::new(), None).unwrap();
    let path = store.path().to_owned();
    assert!(store.close(Duration::from_secs(2)));
    let c = Connection::open(&path).unwrap();
    c.execute_batch("DROP INDEX messages_lottery_draw;
        INSERT INTO messages(anchor_id,anchor_name,web_rid,session_id,sender_id,sender_name,message_type,content,message_time,received_at,time_source,message_id,message_time_epoch,received_at_epoch,payload_json)
        SELECT anchor_id,anchor_name,web_rid,session_id,sender_id,sender_name,message_type,content,message_time,received_at,time_source,'duplicate',message_time_epoch,received_at_epoch,json_set(payload_json,'$.activity_details.prize_name','新详情') FROM messages;").unwrap();
    drop(c);
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    let c = Connection::open(store.path()).unwrap();
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        c.query_row("SELECT message_id FROM messages", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "first"
    );
    assert!(c
        .query_row("SELECT content FROM messages", [], |r| r
            .get::<_, String>(0))
        .unwrap()
        .contains("新详情"));
    assert!(!c
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .exists([])
        .unwrap());
    assert!(store.close(Duration::from_secs(2)));
}

#[test]
fn concurrent_lottery_notifications_commit_exactly_one_row() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    let inserted = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..32)
            .map(|i| {
                let store = &store;
                scope.spawn(move || {
                    let mut r = activity(&format!("concurrent-{i}"));
                    r.payload["activity_details"] = json!({"draw_at":1790784300});
                    store.record_message(r, String::new(), None).unwrap()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|v| *v)
            .count()
    });
    assert_eq!(inserted, 1);
    let c = Connection::open(store.path()).unwrap();
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(store.close(Duration::from_secs(2)));
}

#[test]
fn failed_historical_merge_rolls_back_rows_details_and_index() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    let mut a = activity("first");
    a.payload["activity_details"] = json!({"draw_at":1790784300});
    store.record_message(a, String::new(), None).unwrap();
    let path = store.path().to_owned();
    assert!(store.close(Duration::from_secs(2)));
    let c = Connection::open(&path).unwrap();
    c.execute_batch("DROP INDEX messages_lottery_draw;
        INSERT INTO messages(anchor_id,anchor_name,web_rid,session_id,sender_id,sender_name,message_type,content,message_time,received_at,time_source,message_id,message_time_epoch,received_at_epoch,payload_json)
        SELECT anchor_id,anchor_name,web_rid,session_id,sender_id,sender_name,message_type,content,message_time,received_at,time_source,'duplicate',message_time_epoch,received_at_epoch,json_set(payload_json,'$.activity_details.prize_name','应回滚的详情') FROM messages;
        CREATE TRIGGER fail_lottery_merge BEFORE DELETE ON messages BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    let before: Vec<(i64, String)> = c
        .prepare("SELECT id,payload_json FROM messages ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert!(ArchiveStore::open(dir.path(), ArchiveOptions::default()).is_err());
    let after: Vec<(i64, String)> = c
        .prepare("SELECT id,payload_json FROM messages ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(before, after);
    assert!(!c
        .prepare("SELECT 1 FROM sqlite_schema WHERE name='messages_lottery_draw'")
        .unwrap()
        .exists([])
        .unwrap());
    c.execute_batch("DROP TRIGGER fail_lottery_merge").unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(store.close(Duration::from_secs(2)));
}

#[test]
fn merged_lottery_summary_stays_exportable_with_many_long_conditions() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    let mut first = activity("short");
    first.payload["activity_details"] = json!({"draw_at":1790784300});
    store
        .record_message(first.clone(), String::new(), None)
        .unwrap();
    first.message_id = "long".into();
    first.payload["activity_details"]["conditions"] = json!((0..100)
        .map(|i| json!({"content":format!("{i}{}","😀".repeat(1024))}))
        .collect::<Vec<_>>());
    assert!(!store.record_message(first, String::new(), None).unwrap());
    let c = Connection::open(store.path()).unwrap();
    let (content, payload): (String, String) = c
        .query_row("SELECT content,payload_json FROM messages", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert!(content.encode_utf16().count() <= 32000);
    let p: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(p["activity_summary_truncated"], true);
    assert_eq!(
        p["activity_details"]["conditions"]
            .as_array()
            .unwrap()
            .len(),
        100
    );
    export_month(
        dir.path(),
        ExportOptions {
            anchor_ids: vec!["11".into()],
            month: "2026-10".into(),
            output_path: dir.path().join("long.xlsx"),
            format: "xlsx".into(),
            types: vec!["activity".into()],
        },
        &CancelToken::default(),
    )
    .unwrap();
    assert!(store.close(Duration::from_secs(2)));
}

#[test]
fn activity_keeps_evidence_and_unknown_sender_through_export_and_delete() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
    assert!(store
        .record_message(activity("a"), String::new(), None)
        .unwrap());
    assert!(!store
        .record_message(activity("a"), String::new(), None)
        .unwrap());
    let c = Connection::open(store.path()).unwrap();
    let (sender, name, evidence): (String, String, String) = c
        .query_row(
            "SELECT sender_id,sender_name,payload_json FROM messages",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((sender.as_str(), name.as_str()), ("", ""));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&evidence).unwrap(),
        activity("a").payload
    );
    for format in ["csv", "xlsx"] {
        let path = dir.path().join(format!("activities.{format}"));
        let result = export_month(
            dir.path(),
            ExportOptions {
                anchor_ids: vec!["11".into()],
                month: "2026-10".into(),
                output_path: path.clone(),
                format: format.into(),
                types: vec!["activity".into()],
            },
            &CancelToken::default(),
        )
        .unwrap();
        assert_eq!(result.row_count, 1);
        assert_eq!(result.missing_sender_count, 1);
        let text = if format == "csv" {
            std::fs::read_to_string(path).unwrap()
        } else {
            use std::io::Read;
            let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
            let mut s = String::new();
            zip.by_name("xl/worksheets/sheet1.xml")
                .unwrap()
                .read_to_string(&mut s)
                .unwrap();
            s
        };
        assert!(text.contains("红包/福袋"));
        assert!(text.contains("福袋：礼物 3 份"));
    }
    let selection = ArchiveDeleteFilter {
        anchor_ids: vec!["11".into()],
        month: Some("2026-10".into()),
        session_ids: Some(vec!["101".into()]),
    };
    assert_eq!(
        store.preview_delete(&selection).unwrap()["records_count"],
        1
    );
    assert_eq!(
        store.execute_delete(&selection).unwrap()["deleted_records"],
        1
    );
    assert_eq!(
        store.preview_delete(&selection).unwrap()["records_count"],
        0
    );
    assert!(store.close(Duration::from_secs(2)));
}
