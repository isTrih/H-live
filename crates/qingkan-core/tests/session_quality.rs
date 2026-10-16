use qingkan_core::{
    archive::*,
    export::{export_quality, CancelToken},
};
use serde_json::{json, Value};
use std::{io::Read, time::Duration};

fn rpc(s: &ArchiveStore, action: &str, p: Value) -> Value {
    s.quality_request(action, p, CancelToken::default())
        .unwrap()
}
fn drain(s: &ArchiveStore) {
    for _ in 0..2000 {
        if rpc(s, "step", json!({"limit":128}))["processed"] == 0 {
            return;
        }
    }
    panic!("analysis did not drain");
}
fn message(id: &str, user: &str, text: &str, seconds: i64) -> MessageRecord {
    let t = chrono::DateTime::parse_from_rfc3339("2026-10-01T12:00:00+08:00").unwrap()
        + chrono::Duration::seconds(seconds);
    serde_json::from_value(json!({"anchor_id":"11","anchor_name":"主播甲","web_rid":"111","session_id":"101","sender_id":user,"sender_name":"相同昵称","message_type":"chat","content":text,"message_time":t.to_rfc3339(),"received_at":t.to_rfc3339(),"time_source":"server","message_id":id,"payload":{}})).unwrap()
}
fn put(s: &ArchiveStore, id: &str, user: &str, text: &str, seconds: i64) {
    assert!(s
        .record_message(message(id, user, text, seconds), String::new(), None)
        .unwrap());
}
fn overview(s: &ArchiveStore) -> Value {
    rpc(s, "detail", json!({"anchor_id":"11","session_id":"101"}))["overview"].clone()
}
#[test]
fn conservative_normalization_preserves_numbers_negation_and_short_replies() {
    use qingkan_core::archive::quality::normalize;
    assert_eq!(normalize(" ＡＢＣ　你好  "), "abc 你好");
    assert_eq!(normalize("e\u{301}"), normalize("é"));
    assert_ne!(normalize("能退"), normalize("不能退"));
    assert_ne!(normalize("100"), normalize("1000"));
    assert_ne!(normalize("好"), normalize("好好"));
    assert_eq!(normalize("谢谢！"), "谢谢!");
}
#[test]
fn self_spam_retracts_earlier_messages_but_keeps_users_other_valid_speech() {
    let d = tempfile::tempdir().unwrap();
    let s = ArchiveStore::open(d.path(), ArchiveOptions::default()).unwrap();
    put(&s, "1", "1", "领福利", 0);
    put(&s, "2", "1", "领福利", 1);
    drain(&s);
    assert_eq!(overview(&s)["valid_messages"], 2);
    put(&s, "3", "1", "领福利", 2);
    drain(&s);
    assert_eq!(overview(&s)["valid_speakers"], 0);
    assert_eq!(overview(&s)["valid_messages"], 0);
    put(&s, "4", "1", "可以详细解释一下吗", 3);
    put(&s, "5", "2", "谢谢", 4);
    put(&s, "6", "", "谢谢！", 5);
    drain(&s);
    let v = overview(&s);
    assert_eq!(v["valid_speakers"], 2);
    assert_eq!(v["speakers"], 2);
    assert_eq!(v["unknown_sender_messages"], 1);
    assert_eq!(v["valid_messages"], 3);
    let detail = rpc(
        &s,
        "detail",
        json!({"anchor_id":"11","session_id":"101","reason":"self_repeat"}),
    );
    assert_eq!(detail["total"], 3);
}
#[test]
fn crowd_spam_and_session_spam_retract_all_matching_messages() {
    let d = tempfile::tempdir().unwrap();
    let s = ArchiveStore::open(d.path(), ArchiveOptions::default()).unwrap();
    for i in 0..20 {
        put(&s, &i.to_string(), &(i % 10 + 1).to_string(), "主播加油", i);
    }
    drain(&s);
    assert_eq!(overview(&s)["valid_speakers"], 0);
    for i in 0..50 {
        put(
            &s,
            &format!("b{i}"),
            &(i % 20 + 1).to_string(),
            "分散口令",
            1000 + i * 100,
        );
    }
    drain(&s);
    let v = overview(&s);
    assert_eq!(v["chat_messages"], 70);
    assert_eq!(v["valid_messages"], 0);
    assert_eq!(
        rpc(
            &s,
            "detail",
            json!({"anchor_id":"11","session_id":"101","reason":"session_repeat"})
        )["total"],
        50
    );
}
#[test]
fn time_window_boundary_and_protocol_retransmission_are_distinct() {
    let d = tempfile::tempdir().unwrap();
    let s = ArchiveStore::open(d.path(), ArchiveOptions::default()).unwrap();
    put(&s, "1", "1", "具体问题", 0);
    put(&s, "2", "1", "具体问题", 1);
    put(&s, "3", "1", "具体问题", 61);
    assert!(!s
        .record_message(message("3", "1", "具体问题", 61), String::new(), None)
        .unwrap());
    let mut gift = message("gift", "1", "礼物", 62);
    gift.message_type = "gift".into();
    s.record_message(gift, String::new(), None).unwrap();
    drain(&s);
    assert_eq!(overview(&s)["chat_messages"], 3);
    assert_eq!(overview(&s)["valid_messages"], 3);
    put(&s, "4", "2", "边界", 100);
    put(&s, "5", "2", "边界", 101);
    put(&s, "6", "2", "边界", 160);
    drain(&s);
    assert_eq!(overview(&s)["valid_messages"], 3);
}
#[test]
fn zero_message_sessions_survive_restart_and_chat_deletion() {
    let d = tempfile::tempdir().unwrap();
    let s = ArchiveStore::open(d.path(), ArchiveOptions::default()).unwrap();
    rpc(
        &s,
        "register",
        json!({"anchor_id":"11","session_id":"101","anchor_name":"主播甲","web_rid":"111"}),
    );
    rpc(
        &s,
        "register",
        json!({"anchor_id":"11","session_id":"101","anchor_name":"主播甲","web_rid":"222"}),
    );
    assert_eq!(rpc(&s, "sessions", json!({"anchor_id":"11"}))["total"], 1);
    s.save_monitor_duration("11", "101", 100.5).unwrap();
    put(&s, "1", "1", "谢谢", 0);
    drain(&s);
    s.execute_delete(&ArchiveDeleteFilter {
        anchor_ids: vec!["11".into()],
        month: None,
        session_ids: None,
    })
    .unwrap();
    drain(&s);
    let v = overview(&s);
    assert_eq!(v["chat_messages"], 0);
    assert_eq!(v["valid_speakers"], 0);
    assert_eq!(v["records_cleared"], true);
    assert_eq!(v["monitor_seconds"], 100.5);
    assert!(s.close(Duration::from_secs(1)));
    let s = ArchiveStore::open(d.path(), ArchiveOptions::default()).unwrap();
    assert_eq!(rpc(&s, "sessions", json!({"anchor_id":"11"}))["total"], 1);
}
#[test]
fn new_sessions_months_and_equal_nicknames_do_not_merge() {
    let d = tempfile::tempdir().unwrap();
    let s = ArchiveStore::open(d.path(), ArchiveOptions::default()).unwrap();
    put(&s, "1", "1", "谢谢", 0);
    put(&s, "2", "2", "谢谢", 1);
    let mut row = message("3", "1", "谢谢", 2);
    row.session_id = "102".into();
    s.record_message(row, String::new(), None).unwrap();
    drain(&s);
    assert_eq!(overview(&s)["speakers"], 2);
    assert_eq!(
        rpc(&s, "sessions", json!({"anchor_id":"11","month":"2026-10"}))["total"],
        2
    );
    assert_eq!(
        rpc(&s, "sessions", json!({"anchor_id":"11","month":"2026-09"}))["total"],
        0
    );
}
#[test]
fn manual_commands_rebuild_pause_and_new_messages_never_resurrect_old_results() {
    let d = tempfile::tempdir().unwrap();
    let s = ArchiveStore::open(d.path(), ArchiveOptions::default()).unwrap();
    put(&s, "1", "1", "口令", 0);
    put(&s, "2", "2", "具体反馈", 1);
    drain(&s);
    rpc(
        &s,
        "commands",
        json!({"anchor_id":"11","session_id":"101","commands":["口令"]}),
    );
    assert_eq!(overview(&s)["valid_speakers"], 0);
    assert_eq!(overview(&s)["analysis_state"], "analyzing");
    rpc(&s, "pause", json!({"anchor_id":"11","session_id":"101"}));
    put(&s, "3", "3", "口令", 2);
    drain(&s);
    assert_eq!(overview(&s)["analysis_state"], "paused");
    rpc(&s, "rebuild", json!({"anchor_id":"11","session_id":"101"}));
    drain(&s);
    assert_eq!(overview(&s)["valid_speakers"], 1);
    assert_eq!(
        rpc(
            &s,
            "detail",
            json!({"anchor_id":"11","session_id":"101","reason":"activity_command"})
        )["total"],
        2
    );
    let mut cfg = serde_json::to_value(quality::Rules::default()).unwrap();
    cfg["self_count"] = json!(2);
    rpc(&s, "configure", json!({"anchor_id":"11","rules":cfg}));
    drain(&s);
    assert_eq!(overview(&s)["valid_speakers"], 1);
}
#[test]
fn late_activity_uses_explicit_comment_and_server_time_only() {
    let d = tempfile::tempdir().unwrap();
    let s = ArchiveStore::open(d.path(), ArchiveOptions::default()).unwrap();
    put(&s, "1", "1", "周末快乐", 0);
    let mut missing = message("2", "2", "周末快乐", 1);
    missing.message_time = "".into();
    missing.time_source = "missing".into();
    s.record_message(missing, String::new(), None).unwrap();
    drain(&s);
    let mut event = message("a", "", "福袋活动通知", 3);
    event.message_type = "activity".into();
    let t = chrono::DateTime::parse_from_rfc3339("2026-10-01T12:00:00+08:00")
        .unwrap()
        .timestamp();
    event.payload = json!({"activity_details":{"start_at":t,"draw_at":t+30,"conditions":[{"content":"发送评论「周末快乐」"},{"content":"关注主播"}]}});
    s.record_message(event, String::new(), None).unwrap();
    drain(&s);
    let v = overview(&s);
    assert_eq!(v["valid_messages"], 0);
    assert_eq!(v["uncertain"], 1);
    assert_eq!(v["analysis_state"], "needs_review");
    assert_eq!(
        rpc(
            &s,
            "detail",
            json!({"anchor_id":"11","session_id":"101","reason":"activity_command"})
        )["total"],
        1
    );
    rpc(
        &s,
        "commands",
        json!({"anchor_id":"11","session_id":"101","commands":["周末快乐"]}),
    );
    drain(&s);
    assert_eq!(overview(&s)["pending"], 0);
}
#[test]
fn unknown_activity_time_stays_pending_but_hard_spam_still_wins() {
    let d = tempfile::tempdir().unwrap();
    let s = ArchiveStore::open(d.path(), ArchiveOptions::default()).unwrap();
    let mut event = message("a", "", "活动", 0);
    event.message_type = "activity".into();
    event.payload = json!({"activity_details":{"conditions":[{"content":"发送评论“快乐”"}]}});
    s.record_message(event, String::new(), None).unwrap();
    put(&s, "1", "1", "快乐", 1);
    drain(&s);
    assert_eq!(overview(&s)["uncertain"], 1);
    put(&s, "2", "1", "快乐", 2);
    put(&s, "3", "1", "快乐", 3);
    drain(&s);
    assert_eq!(overview(&s)["uncertain"], 0);
    assert_eq!(overview(&s)["valid_messages"], 0);
}
#[test]
fn cancellation_and_invalid_controls_preserve_committed_state() {
    let d = tempfile::tempdir().unwrap();
    let s = ArchiveStore::open(d.path(), ArchiveOptions::default()).unwrap();
    put(&s, "1", "1", "正常评论", 0);
    let token = CancelToken::default();
    token.cancel();
    assert!(s.quality_request("step", json!({}), token).is_err());
    drain(&s);
    let before = overview(&s);
    assert!(s
        .quality_request(
            "commands",
            json!({"anchor_id":"11","session_id":"101","commands":["ok",""]}),
            CancelToken::default()
        )
        .is_err());
    assert_eq!(overview(&s), before);
    assert!(s
        .quality_request(
            "sessions",
            json!({"anchor_id":"11","limit":true}),
            CancelToken::default()
        )
        .is_err());
}
#[test]
fn summary_export_is_separate_atomic_and_preserves_long_identifiers() {
    let d = tempfile::tempdir().unwrap();
    let s = ArchiveStore::open(d.path(), ArchiveOptions::default()).unwrap();
    put(&s, "1", "99999999999999999999", "谢谢", 0);
    drain(&s);
    for format in ["csv", "xlsx"] {
        let path = d.path().join(format!("summary.{format}"));
        let r = export_quality(
            d.path(),
            "11",
            "2026-10",
            &path,
            format,
            &CancelToken::default(),
        )
        .unwrap();
        assert_eq!(r["row_count"], 1);
        let text = if format == "csv" {
            std::fs::read_to_string(&path).unwrap()
        } else {
            let mut z = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
            let mut text = String::new();
            z.by_name("xl/worksheets/sheet1.xml")
                .unwrap()
                .read_to_string(&mut text)
                .unwrap();
            text
        };
        assert!(text.contains("有效发言人数"));
        assert!(text.contains("interaction-v1"));
        let old = std::fs::read(&path).unwrap();
        let token = CancelToken::default();
        token.cancel();
        assert!(export_quality(d.path(), "11", "", &path, format, &token).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), old);
    }
}

#[test]
fn late_activity_reclassification_is_batched_and_never_shows_a_final_rate_early() {
    let d = tempfile::tempdir().unwrap();
    let s = ArchiveStore::open(d.path(), ArchiveOptions::default()).unwrap();
    rpc(
        &s,
        "configure",
        json!({"anchor_id":"11","rules":{"window_seconds":5,"session_count":1000,"session_users":500}}),
    );
    for i in 0..300 {
        put(
            &s,
            &format!("chat-{i}"),
            &(i + 1).to_string(),
            "活动口令",
            i,
        );
    }
    drain(&s);
    assert_eq!(overview(&s)["valid_messages"], 300);
    let mut event = message("late", "", "活动", 301);
    event.message_type = "activity".into();
    let begin = chrono::DateTime::parse_from_rfc3339("2026-10-01T12:00:00+08:00")
        .unwrap()
        .timestamp();
    event.payload = json!({"activity_details":{"start_at":begin,"draw_at":begin+300,"conditions":[{"content":"发送评论“活动口令”"}]}});
    s.record_message(event, String::new(), None).unwrap();
    rpc(&s, "step", json!({"limit":1}));
    let partial = overview(&s);
    assert_eq!(partial["valid_messages"], 172);
    assert_eq!(partial["analysis_state"], "analyzing");
    assert!(partial["valid_speaker_rate"].is_null());
    drain(&s);
    assert_eq!(overview(&s)["valid_messages"], 0);
    assert_eq!(overview(&s)["valid_speaker_rate"], 0.0);
    rpc(&s, "rebuild", json!({"anchor_id":"11","session_id":"101"}));
    for _ in 0..1000 {
        rpc(&s, "step", json!({"limit":128}));
        if overview(&s)["valid_messages"] == 172 {
            break;
        }
    }
    assert_eq!(overview(&s)["valid_messages"], 172);
    rpc(&s, "pause", json!({"anchor_id":"11","session_id":"101"}));
    for _ in 0..3 {
        rpc(&s, "step", json!({"limit":128}));
    }
    assert_eq!(overview(&s)["valid_messages"], 172);
    assert_eq!(overview(&s)["analysis_state"], "paused");
    rpc(&s, "rebuild", json!({"anchor_id":"11","session_id":"101"}));
    drain(&s);
    assert_eq!(overview(&s)["valid_messages"], 0);
}
