//! Synthetic wire fixtures based on the pinned source schema; no captured users.
use flate2::{write::GzEncoder, Compression};
use qingkan_core::protocol::{decode_frame, decode_message, ProtocolError};
use std::io::Write;

fn var(mut n: u64) -> Vec<u8> {
    let mut out = vec![];
    while n > 127 {
        out.push((n as u8 & 127) | 128);
        n >>= 7;
    }
    out.push(n as u8);
    out
}
fn int(tag: u64, n: u64) -> Vec<u8> {
    [var(tag << 3), var(n)].concat()
}
fn bytes(tag: u64, v: &[u8]) -> Vec<u8> {
    [var(tag << 3 | 2), var(v.len() as u64), v.to_vec()].concat()
}
fn text(tag: u64, s: &str) -> Vec<u8> {
    bytes(tag, s.as_bytes())
}
fn common() -> Vec<u8> {
    bytes(
        1,
        &[
            int(2, u64::MAX),
            int(3, u64::MAX - 1),
            int(4, 1800000000123),
        ]
        .concat(),
    )
}
fn payload(method: &str, raw: &[u8]) -> serde_json::Value {
    let event = decode_message(method, raw, "88").unwrap().unwrap();
    assert_eq!(event.kind, "activity");
    event.payload
}

#[test]
fn lottery_details_keep_ids_unknown_status_and_missing_sender_exact() {
    let condition = [
        int(1, 99),
        text(2, "输入公开口令"),
        int(3, 1),
        int(4, 5),
        text(5, "仅限本场"),
    ]
    .concat();
    let raw = [
        common(),
        int(2, u64::MAX),
        int(3, 99),
        int(4, 1800000000),
        int(5, 1800000060123),
        int(6, 1800000001),
        int(8, 101),
        int(13, 0),
        int(14, 3),
        text(18, "测试奖品"),
        text(19, "合成定义样本"),
        bytes(21, &condition),
    ]
    .concat();
    let p = payload("WebcastLotteryEventNewMessage", &raw);
    assert_eq!(p["activity_id"], u64::MAX.to_string());
    assert_eq!(p["room_id"], (u64::MAX - 1).to_string());
    assert_eq!(p["message_id"], u64::MAX.to_string());
    assert_eq!(p["event_time"], 1800000000u64);
    assert_eq!(p["sender_id"], "");
    assert_eq!(p["sender_name"], "");
    let d = &p["activity_details"];
    assert_eq!(d["draw_at"], 1800000060u64);
    assert_eq!(d["status_code"], 99);
    assert_eq!(d["prize_count"], "0");
    assert_eq!(d["winner_count"], "3");
    assert_eq!(d["conditions"][0]["min_fans_level"], "5");
    let content = p["content"].as_str().unwrap();
    assert!(
        content.contains("测试奖品")
            && content.contains("输入公开口令")
            && content.contains("开奖时间：")
    );
}

#[test]
fn missing_timestamps_and_counts_are_not_invented() {
    let p = payload("WebcastLotteryEventMessage", &[]);
    assert_eq!(p["event_time"], 0);
    assert_eq!(p["message_id"], "");
    assert_eq!(p["room_id"], "88");
    assert_eq!(p["activity_id"], "");
    assert!(p["activity_details"]["draw_at"].is_null());
    assert!(p["activity_details"]["status_code"].is_null());
    assert!(p["activity_details"]["prize_count"].is_null());
}

#[test]
fn red_packet_keeps_explicit_sender_and_does_not_guess_delay_unit() {
    let raw = [
        common(),
        int(2, 10),
        int(3, 12),
        int(4, 1800000005),
        int(5, 60),
        text(7, "测试红包"),
        bytes(12, &[int(1, u64::MAX - 2), text(3, "合成发送者")].concat()),
        bytes(13, &text(2, "公开活动说明")),
        bytes(19, &[text(2, "5份"), text(3, "10钻石")].concat()),
        text(24, "92233720368547758079"),
    ]
    .concat();
    let p = payload("WebcastLuckyBoxMessage", &raw);
    assert_eq!(p["sender_id"], (u64::MAX - 2).to_string());
    assert_eq!(p["activity_id"], "92233720368547758079");
    assert_eq!(p["event_time"], 1800000005u64);
    assert_eq!(p["activity_details"]["delay_time_raw"], "60");
    assert!(p["activity_details"].get("draw_at").is_none());
    assert!(p["content"].as_str().unwrap().contains("5份"));
}

#[test]
fn outcome_methods_ignore_recipient_lists_arbitrary_extra_and_urls() {
    for (method, stage) in [
        ("WebcastLuckyBoxEndMessage", "ended"),
        ("WebcastLuckyBoxRewardMessage", "reward"),
        ("WebcastLuckyBoxTempStatusMessage", "status"),
    ] {
        let raw = [
            common(),
            int(2, 123),
            text(3, "long-box-id"),
            text(4, "SECRET_WINNER"),
            text(5, "SECRET_RESULT"),
            text(5000, "SECRET_OPENID"),
        ]
        .concat();
        let p = payload(method, &raw);
        assert_eq!(p["activity_stage"], stage);
        assert!(!p.to_string().contains("SECRET"));
        if stage == "status" {
            assert_eq!(p["activity_id"], "");
        }
    }
    let p = payload(
        "WebcastLotteryDrawResultEventMessage",
        &[
            common(),
            int(2, 777),
            int(3, 123),
            text(4, "SECRET_JSON"),
            text(5000, "SECRET_OPENID"),
        ]
        .concat(),
    );
    assert_eq!(p["activity_stage"], "draw_result");
    assert!(!p.to_string().contains("SECRET"));
    assert!(
        decode_message("WebcastLotteryCandidateEventMessage", &[], "")
            .unwrap()
            .is_none()
    );
}

#[test]
fn undefined_fields_for_old_or_common_only_methods_stay_unknown() {
    let p = payload(
        "WebcastLotteryEventMessage",
        &[text(18, "not-defined-here"), text(8, "unknown-wire")].concat(),
    );
    assert_eq!(p["activity_details"]["prize_name"], "");
    let p = payload(
        "WebcastLuckyBoxTempStatusMessage",
        &[text(2, "unknown-wire"), int(3, 99)].concat(),
    );
    assert_eq!(p["activity_id"], "");
    let p = payload("WebcastLuckyBoxEndMessage", &text(6, "unknown-wire"));
    assert!(p["activity_details"]["reward_message_type"].is_null());
}

#[test]
fn large_ignored_recipient_lists_do_not_discard_activity_notifications() {
    let reward = [common(), int(2, 777), text(4, "ignored-user").repeat(257)].concat();
    let p = payload("WebcastLuckyBoxRewardMessage", &reward);
    assert_eq!(p["activity_id"], "777");
    assert_eq!(p["activity_stage"], "reward");
    assert!(!p.to_string().contains("ignored-user"));
    let result = [common(), int(2, 888), int(3, 99).repeat(257)].concat();
    let p = payload("WebcastLotteryDrawResultEventMessage", &result);
    assert_eq!(p["activity_id"], "888");
    assert_eq!(p["activity_stage"], "draw_result");
}

#[test]
fn old_lucky_money_and_all_eight_methods_are_supported() {
    let p = payload(
        "WebcastLuckyMoneyMessage",
        &[common(), int(3, 777), int(4, 99)].concat(),
    );
    assert_eq!(p["activity_id"], "777");
    assert_eq!(p["activity_kind"], "red_packet");
    assert_eq!(p["activity_details"]["diamond_count"], "99");
}

#[test]
fn malformed_activity_is_isolated_ack_and_control_barrier_preserved() {
    let envelope = |method: &str, raw: &[u8]| bytes(1, &[text(1, method), bytes(2, raw)].concat());
    let response = [
        envelope("WebcastLotteryEventNewMessage", &[0xff]),
        envelope("WebcastLuckyBoxEndMessage", &common()),
        envelope("WebcastControlMessage", &int(2, 3)),
        envelope("WebcastLuckyMoneyMessage", &common()),
        text(5, "ack"),
        int(9, 1),
    ]
    .concat();
    let mut gzip = GzEncoder::new(Vec::new(), Compression::fast());
    gzip.write_all(&response).unwrap();
    let result = decode_frame(
        &[int(2, 7), bytes(8, &gzip.finish().unwrap())].concat(),
        "88",
    )
    .unwrap();
    assert!(result.ack.is_some());
    assert_eq!(result.skipped_messages, 1);
    assert_eq!(result.events.len(), 2);
    assert_eq!(result.events[0].kind, "activity");
    assert_eq!(result.events[1].kind, "control");
}

#[test]
fn repeated_details_and_export_summary_are_bounded() {
    let oversized = bytes(21, &[]).repeat(257);
    assert_eq!(
        decode_message("WebcastLotteryEventNewMessage", &oversized, ""),
        Err(ProtocolError::InvalidMessage)
    );
    let raw = bytes(21, &text(2, &"长".repeat(3000))).repeat(100);
    let p = payload("WebcastLotteryEventNewMessage", &raw);
    assert_eq!(p["activity_summary_truncated"], true);
    assert_eq!(p["content"].as_str().unwrap().chars().count(), 16000);
    assert_eq!(
        p["activity_details"]["conditions"][0]["content"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        1024
    );
}
