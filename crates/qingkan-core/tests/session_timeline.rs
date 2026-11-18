use qingkan_core::{archive::*, export::CancelToken};
use serde_json::json;

#[test]
fn whole_session_preserves_old_long_and_untimed_records_and_events() {
    let d = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(d.path(), ArchiveOptions::default()).unwrap();
    let start = "2026-10-23T14:00:00+08:00";
    store
        .quality_request(
            "register",
            json!({"anchor_id":"11","session_id":"101","started_at":start}),
            CancelToken::default(),
        )
        .unwrap();
    for i in 0..202 {
        let kind = if i == 201 { "activity" } else { "chat" };
        let missing = i == 200;
        let content = if i == 0 {
            "完整长文本😀".repeat(1000)
        } else {
            format!("record-{i}")
        };
        let m:MessageRecord=serde_json::from_value(json!({"anchor_id":"11","anchor_name":"主播","web_rid":"111","session_id":"101","sender_id":(i+1).to_string(),"sender_name":"观众","message_type":kind,"content":content,"message_id":i.to_string(),"message_time":if missing {""} else {start},"received_at":"2026-10-23T14:00:01+08:00","time_source":if missing {"missing"} else {"server"},"payload":{"source_record":i}})).unwrap();
        store
            .record_message(m, String::new(), Some(start.to_owned()))
            .unwrap();
    }
    store.quality_request("audience_record",json!({"anchor_id":"11","session_id":"101","viewer_count":159,"observed_at":"2026-10-23T14:00:02+08:00"}),CancelToken::default()).unwrap();
    let result = store.session_timeline("11", "101").unwrap();
    assert_eq!(result["total_messages"], 202);
    assert_eq!(result["chat_count"], 201);
    assert_eq!(result["timed_chat_count"], 200);
    assert_eq!(result["untimed_chat_count"], 1);
    assert_eq!(result["complete"], true);
    let rows = result["messages"].as_array().unwrap();
    assert_eq!(rows.len(), 202);
    assert_eq!(
        rows[0]["content"].as_str().unwrap(),
        "完整长文本😀".repeat(1000)
    );
    assert!(rows
        .iter()
        .any(|m| m["message_type"] == "activity" && m["payload"]["source_record"] == 201));
    let untimed = rows.iter().find(|m| m["time_source"] == "missing").unwrap();
    assert!(untimed["offset_seconds"].is_null());
    assert!(untimed["observed_offset_seconds"].is_number());
    assert_eq!(
        result["buckets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["count"].as_u64().unwrap())
            .sum::<u64>(),
        200
    );
    assert_eq!(result["online"][0]["peak"], 159);
    assert!(store.session_timeline("11", "999").is_err());
    assert!(store.session_timeline("../11", "101").is_err());
}

#[test]
fn zero_chat_registered_session_has_a_valid_empty_timeline() {
    let d = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(d.path(), ArchiveOptions::default()).unwrap();
    store
        .quality_request(
            "register",
            json!({"anchor_id":"11","session_id":"101"}),
            CancelToken::default(),
        )
        .unwrap();
    let result = store.session_timeline("11", "101").unwrap();
    assert_eq!(result["total_messages"], 0);
    assert_eq!(result["duration_seconds"], 0.);
    assert_eq!(result["buckets"], json!([{"offset_seconds":0,"count":0}]));
}

#[test]
fn rounded_audience_minute_does_not_move_a_known_session_start() {
    let d = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(d.path(), ArchiveOptions::default()).unwrap();
    let started = "2026-10-23T14:00:30+08:00";
    store
        .quality_request(
            "register",
            json!({"anchor_id":"11","session_id":"101","started_at":started}),
            CancelToken::default(),
        )
        .unwrap();
    store.quality_request("audience_record",json!({"anchor_id":"11","session_id":"101","viewer_count":159,"observed_at":"2026-10-23T14:00:32+08:00"}),CancelToken::default()).unwrap();
    let result = store.session_timeline("11", "101").unwrap();
    assert_eq!(
        result["start_epoch"].as_f64().unwrap(),
        parse_time(started).unwrap().timestamp() as f64
    );
    assert_eq!(result["online"][0]["offset_seconds"], 0.);
}

#[test]
fn activity_timeline_uses_canonical_rounds_keeps_observation_times_explicit_and_density_chat_only()
{
    let d = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(d.path(), ArchiveOptions::default()).unwrap();
    let start = "2026-10-23T14:00:00+08:00";
    let start_epoch = parse_time(start).unwrap().timestamp();
    let record = |id: &str, kind: &str, sent: &str, received: &str, payload| -> MessageRecord {
        serde_json::from_value(json!({"anchor_id":"11","anchor_name":"主播","web_rid":"111","session_id":"101","sender_id":"9001","sender_name":"观众","message_type":kind,"content":format!("完整记录 {id}"),"message_id":id,"message_time":sent,"received_at":received,"time_source":if sent.is_empty(){"missing"}else{"server"},"payload":payload})).unwrap()
    };
    for (id, at) in [
        ("chat-start", start),
        ("chat-end", "2026-10-23T14:02:00+08:00"),
    ] {
        store
            .record_message(
                record(id, "chat", at, at, json!({})),
                String::new(),
                Some(start.into()),
            )
            .unwrap();
    }
    let first = record(
        "lottery-notice",
        "activity",
        "2026-10-23T14:00:30+08:00",
        "2026-10-23T14:00:31+08:00",
        json!({"activity_kind":"lottery","activity_id":"0001234567890123456789","activity_stage":"notice","activity_details":{"draw_at":start_epoch+300,"winner_count":3}}),
    );
    store
        .record_message(first.clone(), String::new(), Some(start.into()))
        .unwrap();
    let mut enriched = first.clone();
    enriched.message_id = "lottery-details".into();
    enriched.payload["activity_id"] = json!("same-draw-other-notification-id");
    enriched.payload["activity_details"] = json!({"draw_at":(start_epoch+300)*1000,"prize_name":"限定玩偶","conditions":[{"content":"发送评论「开心冒险」"}]});
    assert!(!store
        .record_message(enriched, String::new(), Some(start.into()))
        .unwrap());
    for (id, sent, received) in [
        ("packet-1", "", "2026-10-23T14:01:00+08:00"),
        (
            "packet-2",
            "2026-10-23T14:01:10+08:00",
            "2026-10-23T14:01:11+08:00",
        ),
    ] {
        store.record_message(record(id, "activity", sent, received, json!({"activity_kind":"red_packet","activity_id":id,"activity_details":{"title":"同名红包","expires_at":start_epoch+600}})), String::new(), Some(start.into())).unwrap();
    }
    store.record_message(record("unconfirmed", "activity", "2026-10-23T14:01:30+08:00", "2026-10-23T14:01:31+08:00", json!({"activity_kind":"lottery","activity_stage":"notice","activity_details":{"title":"缺少真实身份"}})), String::new(), Some(start.into())).unwrap();

    store
        .quality_request("step", json!({"limit":128}), CancelToken::default())
        .unwrap();
    let result = store.session_timeline("11", "101").unwrap();
    assert_eq!(result["duration_seconds"], 120.);
    assert_eq!(result["total_messages"], 6);
    assert_eq!(result["activity_count"], 3);
    assert_eq!(result["unconfirmed_activity_count"], 1);
    assert_eq!(result["chat_count"], 2);
    assert_eq!(
        result["buckets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["count"].as_u64().unwrap())
            .sum::<u64>(),
        2
    );
    let activities = result["activities"].as_array().unwrap();
    let lottery = activities.iter().find(|a| a["kind"] == "lottery").unwrap();
    assert_eq!(lottery["offset_seconds"], 30.);
    assert_eq!(lottery["time_source"], "server");
    assert_eq!(lottery["draw_at"], start_epoch + 300);
    assert_eq!(lottery["details"]["prize_name"], "限定玩偶");
    assert_eq!(
        lottery["details"]["conditions"][0]["content"],
        "发送评论「开心冒险」"
    );
    let packets: Vec<_> = activities
        .iter()
        .filter(|a| a["kind"] == "red_packet")
        .collect();
    assert_eq!(
        packets.len(),
        2,
        "same expiry or title must not merge distinct red packets"
    );
    let observed = packets
        .iter()
        .find(|a| a["activity_id"] == "packet-1")
        .unwrap();
    assert_eq!(observed["time_source"], "observed");
    assert_eq!(observed["offset_seconds"], 60.);
    assert_eq!(observed["observed_epoch"], start_epoch as f64 + 60.);
    assert!(observed["draw_at"].is_null());
    assert_eq!(observed["expires_at"], start_epoch + 600);
    let raw = result["messages"].as_array().unwrap();
    assert!(raw
        .iter()
        .any(|m| m["payload"]["activity_id"] == "packet-1" && m["offset_seconds"].is_null()));
    assert!(raw
        .iter()
        .any(|m| m["payload"]["activity_details"]["title"] == "缺少真实身份"));
    assert_eq!(
        raw.len(),
        6,
        "activity projection must preserve all raw prompt records"
    );
    assert!(result["summary"].is_object());
}
