//! Minimal activity projections, not a general activity/commerce parser.
//! Field tags: jwwsjlm/douyinlive-proto, MIT, commit
//! 0d661be57c8f80947054b2ef0cd1ac2fdce3cc4b, protobuf/new_douyin.proto.
//! See docs/research/live-activities-protocol.md and the accompanying license.
//! Unknown enums stay numeric. Recipient lists, arbitrary extra JSON and URLs
//! are intentionally not materialized. Fixtures are synthetic, not live proof.

use super::{content_event, epoch_seconds, numeric_id, parse, Common, Event, ProtocolError, User};
use prost::Message;
use serde_json::{json, Value};

#[derive(Clone, PartialEq, Message)]
struct RoomSync {
    #[prost(bytes = "vec", optional, tag = "1")]
    common: Option<Vec<u8>>,
    #[prost(uint64, tag = "2")]
    room: u64,
    #[prost(string, tag = "3")]
    key: String,
    #[prost(uint64, optional, tag = "4")]
    version: Option<u64>,
    #[prost(bytes = "vec", tag = "5")]
    payload: Vec<u8>,
}
#[derive(Clone, PartialEq, Message)]
struct LotterySync {
    #[prost(uint64, tag = "1")]
    id: u64,
    #[prost(uint64, optional, tag = "2")]
    winners: Option<u64>,
    #[prost(uint64, optional, tag = "5")]
    kind: Option<u64>,
    #[prost(uint64, optional, tag = "6")]
    prizes: Option<u64>,
    #[prost(uint64, tag = "7")]
    start: u64,
    #[prost(uint64, tag = "8")]
    draw: u64,
}
#[derive(Clone, PartialEq, Message)]
struct Preview {
    #[prost(message, optional, tag = "1")]
    packet: Option<PreviewPacket>,
    #[prost(bytes = "vec", optional, tag = "2")]
    common: Option<Vec<u8>>,
}
#[derive(Clone, PartialEq, Message)]
struct PreviewPacket {
    #[prost(string, tag = "1")]
    order: String,
    #[prost(uint64, tag = "2")]
    sent: u64,
    #[prost(uint64, tag = "3")]
    expires: u64,
    #[prost(uint64, optional, tag = "4")]
    delay: Option<u64>,
    #[prost(uint64, optional, tag = "5")]
    status: Option<u64>,
    #[prost(string, tag = "6")]
    currency: String,
    #[prost(string, tag = "8")]
    text: String,
    #[prost(uint64, optional, tag = "9")]
    total_amount: Option<u64>,
    #[prost(uint64, optional, tag = "10")]
    left_amount: Option<u64>,
    #[prost(uint64, optional, tag = "11")]
    total: Option<u64>,
    #[prost(uint64, optional, tag = "12")]
    left: Option<u64>,
}
/// Room sync is a generic envelope. Only this independently documented key is
/// an activity; ranking, participant totals and arbitrary sync payloads stay ignored.
pub(super) fn decode_sync(payload: &[u8], fallback: &str) -> Result<Option<Event>, ProtocolError> {
    let m: RoomSync = parse(payload)?;
    if m.key != "LotteryInfoSyncData" {
        return Ok(None);
    }
    let lottery: LotterySync = parse(&m.payload)?;
    if lottery.id == 0 {
        return Ok(None);
    }
    let mut common: Common = parse(m.common.as_deref().unwrap_or_default())?;
    if common.room_id == 0 {
        common.room_id = m.room;
    }
    let mut parts = vec![
        "福袋同步通知".to_owned(),
        format!("活动 ID：{}", lottery.id),
    ];
    if let Some(n) = lottery.prizes {
        parts.push(format!("奖品数量：{n}"));
    }
    if let Some(n) = lottery.winners {
        parts.push(format!("中奖名额：{n}"));
    }
    if let Some(date) = chrono::DateTime::from_timestamp(epoch_seconds(lottery.draw) as i64, 0)
        .filter(|_| epoch_seconds(lottery.draw) > 0)
    {
        parts.push(format!(
            "开奖时间：{}",
            date.with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
                .format("%Y-%m-%d %H:%M:%S%:z")
        ));
    }
    Ok(Some(event(
        "WebcastRoomDataSyncMessage",
        "lottery",
        "sync",
        numeric_id(lottery.id),
        &common,
        &User::default(),
        parts,
        0,
        fallback,
        json!({"sync_key":m.key,"sync_version":number(m.version),"prize_type":number(lottery.kind),"prize_count":number(lottery.prizes),"winner_count":number(lottery.winners),"start_at":timestamp(lottery.start),"draw_at":timestamp(lottery.draw)}),
    )))
}

#[derive(Clone, PartialEq, Message)]
struct Lottery {
    #[prost(bytes = "vec", optional, tag = "1")]
    common: Option<Vec<u8>>,
    #[prost(uint64, tag = "2")]
    id: u64,
    #[prost(int32, optional, tag = "3")]
    status: Option<i32>,
    #[prost(uint64, tag = "4")]
    start_time: u64,
    #[prost(uint64, tag = "5")]
    draw_time: u64,
    #[prost(uint64, tag = "6")]
    current_time: u64,
    #[prost(uint64, optional, tag = "8")]
    prize_type: Option<u64>,
    #[prost(int32, repeated, tag = "10")]
    conditions: Vec<i32>,
    #[prost(uint64, optional, tag = "13")]
    prize_count: Option<u64>,
    #[prost(uint64, optional, tag = "14")]
    lucky_count: Option<u64>,
    #[prost(string, tag = "18")]
    award_name: String,
    #[prost(string, tag = "19")]
    award_description: String,
    #[prost(message, repeated, tag = "21")]
    conditions_detail: Vec<Condition>,
}

#[derive(Clone, PartialEq, Message)]
struct OldLottery {
    #[prost(bytes = "vec", optional, tag = "1")]
    common: Option<Vec<u8>>,
    #[prost(uint64, tag = "2")]
    id: u64,
    #[prost(int32, optional, tag = "3")]
    status: Option<i32>,
    #[prost(uint64, tag = "4")]
    start_time: u64,
    #[prost(uint64, tag = "5")]
    draw_time: u64,
    #[prost(uint64, tag = "6")]
    current_time: u64,
}

#[derive(Clone, PartialEq, Message)]
struct Condition {
    #[prost(int32, optional, tag = "1")]
    kind: Option<i32>,
    #[prost(string, tag = "2")]
    content: String,
    #[prost(uint64, optional, tag = "3")]
    need_diamond_count: Option<u64>,
    #[prost(uint64, optional, tag = "4")]
    min_fans_level: Option<u64>,
    #[prost(string, tag = "5")]
    remarks: String,
}

#[derive(Clone, PartialEq, Message)]
struct LuckyBox {
    #[prost(bytes = "vec", optional, tag = "1")]
    common: Option<Vec<u8>>,
    #[prost(uint64, optional, tag = "2")]
    diamond_count: Option<u64>,
    #[prost(uint64, tag = "3")]
    box_id: u64,
    #[prost(uint64, tag = "4")]
    send_time: u64,
    #[prost(uint64, optional, tag = "5")]
    delay_time: Option<u64>,
    #[prost(uint64, optional, tag = "6")]
    box_type: Option<u64>,
    #[prost(string, tag = "7")]
    title: String,
    #[prost(bytes = "vec", optional, tag = "12")]
    user: Option<Vec<u8>>,
    #[prost(message, repeated, tag = "13")]
    descriptions: Vec<ImgText>,
    #[prost(int32, optional, tag = "16")]
    status: Option<i32>,
    #[prost(bytes = "vec", optional, tag = "19")]
    meta: Option<Vec<u8>>,
    #[prost(string, tag = "24")]
    box_id_str: String,
    #[prost(string, tag = "27")]
    activity_id: String,
}

#[derive(Clone, PartialEq, Message)]
struct ImgText {
    #[prost(string, tag = "2")]
    text: String,
}

#[derive(Clone, PartialEq, Message)]
struct BoxMeta {
    #[prost(string, tag = "1")]
    title_desc: String,
    #[prost(string, tag = "2")]
    content_count_desc: String,
    #[prost(string, tag = "3")]
    content_amount_desc: String,
    #[prost(string, tag = "5")]
    before_unpack_desc: String,
    #[prost(string, tag = "9")]
    im_desc: String,
}

#[derive(Clone, PartialEq, Message)]
struct BoxNotice {
    #[prost(bytes = "vec", optional, tag = "1")]
    common: Option<Vec<u8>>,
    #[prost(uint64, tag = "2")]
    box_id: u64,
    #[prost(string, tag = "3")]
    box_id_str: String,
}

#[derive(Clone, PartialEq, Message)]
struct RewardType {
    #[prost(uint64, optional, tag = "6")]
    value: Option<u64>,
}

#[derive(Clone, PartialEq, Message)]
struct CommonOnly {
    #[prost(bytes = "vec", optional, tag = "1")]
    common: Option<Vec<u8>>,
}

#[derive(Clone, PartialEq, Message)]
struct LotteryResult {
    #[prost(bytes = "vec", optional, tag = "1")]
    common: Option<Vec<u8>>,
    #[prost(uint64, tag = "2")]
    lottery_id: u64,
}

#[derive(Clone, PartialEq, Message)]
struct LuckyMoney {
    #[prost(bytes = "vec", optional, tag = "1")]
    common: Option<Vec<u8>>,
    #[prost(bytes = "vec", optional, tag = "2")]
    user: Option<Vec<u8>>,
    #[prost(uint64, tag = "3")]
    id: u64,
    #[prost(uint64, optional, tag = "4")]
    diamond_count: Option<u64>,
}

pub(super) fn supports(method: &str) -> bool {
    matches!(
        method,
        "WebcastLotteryEventMessage"
            | "WebcastLotteryEventNewMessage"
            | "WebcastLotteryDrawResultEventMessage"
            | "WebcastLuckyBoxMessage"
            | "WebcastLuckyBoxEndMessage"
            | "WebcastLuckyBoxRewardMessage"
            | "WebcastLuckyBoxTempStatusMessage"
            | "WebcastLuckyMoneyMessage"
            | "WebcastPreviewCjRpMessage"
    )
}

fn short(value: &str) -> String {
    value.chars().take(1024).collect()
}
fn id(string: &str, integer: u64) -> String {
    if !string.is_empty() && string.len() <= 128 {
        string.to_owned()
    } else {
        numeric_id(integer)
    }
}
fn number(value: Option<u64>) -> Value {
    value.map(|v| json!(v.to_string())).unwrap_or(Value::Null)
}
fn timestamp(value: u64) -> Value {
    let seconds = epoch_seconds(value);
    if seconds == 0 {
        Value::Null
    } else {
        json!(seconds)
    }
}
fn add(parts: &mut Vec<String>, label: &str, value: &str) {
    if !value.is_empty() {
        parts.push(format!("{label}{}", short(value)));
    }
}

fn event(
    method: &str,
    kind: &str,
    stage: &str,
    activity_id: String,
    common: &Common,
    user: &User,
    parts: Vec<String>,
    source_time: u64,
    fallback: &str,
    details: Value,
) -> Event {
    let content = parts.join("；");
    let truncated = content.chars().count() > 16000;
    let mut event = content_event(
        "activity",
        "红包/福袋",
        common,
        user,
        content.chars().take(16000).collect(),
        source_time,
        fallback,
    );
    let payload = event
        .payload
        .as_object_mut()
        .expect("content payload is an object");
    payload.insert("activity_kind".into(), json!(kind));
    payload.insert("activity_stage".into(), json!(stage));
    payload.insert("activity_method".into(), json!(method));
    payload.insert("activity_id".into(), json!(activity_id));
    payload.insert("activity_details".into(), details);
    payload.insert("activity_summary_truncated".into(), json!(truncated));
    event
}

pub(super) fn decode(method: &str, payload: &[u8], fallback: &str) -> Result<Event, ProtocolError> {
    if method == "WebcastPreviewCjRpMessage" {
        let m: Preview = parse(payload)?;
        let packet = m.packet.unwrap_or_default();
        let common: Common = parse(m.common.as_deref().unwrap_or_default())?;
        let mut parts = vec!["红包预告".to_owned()];
        add(&mut parts, "内容：", &packet.text);
        if let Some(total) = packet.total {
            parts.push(format!("红包份数：{total}"));
        }
        if let Some(left) = packet.left {
            parts.push(format!("剩余份数：{left}"));
        }
        return Ok(event(
            method,
            "red_packet",
            "preview",
            id(&packet.order, 0),
            &common,
            &User::default(),
            parts,
            packet.sent,
            fallback,
            json!({"sent_at":timestamp(packet.sent),"expires_at":timestamp(packet.expires),"display_delay_seconds":number(packet.delay),"status_code":number(packet.status),"currency":short(&packet.currency),"description":short(&packet.text),"total_amount_raw":number(packet.total_amount),"remaining_amount_raw":number(packet.left_amount),"total_count":number(packet.total),"remaining_count":number(packet.left)}),
        ));
    }
    // Bound allocations before prost materializes repeated nested messages.
    // Even a 1 MiB payload can otherwise contain half a million empty entries.
    let mut rest = payload;
    let mut counts = [0usize; 2];
    while !rest.is_empty() {
        let (tag, wire) =
            prost::encoding::decode_key(&mut rest).map_err(|_| ProtocolError::InvalidMessage)?;
        let slot = match (method, tag) {
            ("WebcastLotteryEventNewMessage", 10) | ("WebcastLuckyBoxMessage", 13) => Some(0),
            ("WebcastLotteryEventNewMessage", 21) => Some(1),
            _ => None,
        };
        // Recipient lists and unknown fields are skipped without allocation.
        // Their multiplicity must not suppress a legitimate result notice.
        if let Some(slot) = slot {
            counts[slot] += 1;
            if counts[slot] > 256 {
                return Err(ProtocolError::InvalidMessage);
            }
        }
        prost::encoding::skip_field(
            wire,
            tag,
            &mut rest,
            prost::encoding::DecodeContext::default(),
        )
        .map_err(|_| ProtocolError::InvalidMessage)?;
    }
    match method {
        "WebcastLotteryEventMessage" | "WebcastLotteryEventNewMessage" => {
            let m: Lottery = if method == "WebcastLotteryEventNewMessage" {
                parse(payload)?
            } else {
                let old: OldLottery = parse(payload)?;
                Lottery {
                    common: old.common,
                    id: old.id,
                    status: old.status,
                    start_time: old.start_time,
                    draw_time: old.draw_time,
                    current_time: old.current_time,
                    ..Default::default()
                }
            };
            let common: Common = parse(m.common.as_deref().unwrap_or_default())?;
            let mut parts = vec!["福袋活动通知".to_owned()];
            add(&mut parts, "奖品：", &m.award_name);
            add(&mut parts, "说明：", &m.award_description);
            if let Some(n) = m.prize_count {
                parts.push(format!("奖品数量：{n}"));
            }
            if let Some(n) = m.lucky_count {
                parts.push(format!("中奖名额：{n}"));
            }
            if let Some(status) = m.status {
                parts.push(format!("状态码：{status}"));
            }
            let conditions: Vec<_> = m.conditions_detail.iter().take(100).map(|c| {
                add(&mut parts, "参与条件：", &c.content);
                add(&mut parts, "条件备注：", &c.remarks);
                if let Some(n) = c.need_diamond_count { parts.push(format!("条件所需钻石：{n}")); }
                if let Some(n) = c.min_fans_level { parts.push(format!("最低粉丝等级：{n}")); }
                json!({"type": c.kind, "content": short(&c.content), "remarks": short(&c.remarks),
                    "need_diamond_count": number(c.need_diamond_count), "min_fans_level": number(c.min_fans_level)})
            }).collect();
            if epoch_seconds(m.draw_time) > 0 {
                // The complete epoch is kept below; rendering uses Shanghai in
                // the UI/export layer, not a guessed countdown or local now.
                if let Some(date) =
                    chrono::DateTime::from_timestamp(epoch_seconds(m.draw_time) as i64, 0)
                {
                    let zone = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
                    parts.push(format!(
                        "开奖时间：{}",
                        date.with_timezone(&zone).format("%Y-%m-%d %H:%M:%S%:z")
                    ));
                }
            }
            Ok(event(
                method,
                "lottery",
                "notice",
                numeric_id(m.id),
                &common,
                &User::default(),
                parts,
                0,
                fallback,
                json!({"status_code": m.status, "start_at": timestamp(m.start_time),
                    "draw_at": timestamp(m.draw_time), "server_time": timestamp(m.current_time),
                    "prize_type": number(m.prize_type), "prize_name": short(&m.award_name),
                    "prize_description": short(&m.award_description), "prize_count": number(m.prize_count),
                    "winner_count": number(m.lucky_count), "condition_codes": m.conditions.into_iter().take(100).collect::<Vec<_>>(),
                    "conditions": conditions}),
            ))
        }
        "WebcastLuckyBoxMessage" => {
            let m: LuckyBox = parse(payload)?;
            let common: Common = parse(m.common.as_deref().unwrap_or_default())?;
            let user: User = parse(m.user.as_deref().unwrap_or_default())?;
            let meta: BoxMeta = parse(m.meta.as_deref().unwrap_or_default())?;
            let mut parts = vec!["红包通知".to_owned()];
            add(&mut parts, "标题：", &m.title);
            if let Some(n) = m.diamond_count {
                parts.push(format!("钻石数量：{n}"));
            }
            for text in [
                &meta.title_desc,
                &meta.content_count_desc,
                &meta.content_amount_desc,
                &meta.before_unpack_desc,
                &meta.im_desc,
            ] {
                add(&mut parts, "", text);
            }
            let descriptions: Vec<_> = m
                .descriptions
                .iter()
                .take(100)
                .map(|d| {
                    add(&mut parts, "", &d.text);
                    short(&d.text)
                })
                .collect();
            if let Some(status) = m.status {
                parts.push(format!("状态码：{status}"));
            }
            Ok(event(
                method,
                "red_packet",
                "notice",
                id(&m.box_id_str, m.box_id),
                &common,
                &user,
                parts,
                m.send_time,
                fallback,
                json!({"title": short(&m.title), "diamond_count": number(m.diamond_count),
                    "status_code": m.status, "box_type": number(m.box_type), "sent_at": timestamp(m.send_time),
                    // Source declares uint64 but no unit. Preserve raw value,
                    // never manufacture draw_at = send_time + delay_time.
                    "delay_time_raw": number(m.delay_time), "campaign_id": short(&m.activity_id),
                    "descriptions": descriptions, "title_description": short(&meta.title_desc),
                    "count_description": short(&meta.content_count_desc), "amount_description": short(&meta.content_amount_desc),
                    "before_unpack_description": short(&meta.before_unpack_desc), "notice_description": short(&meta.im_desc)}),
            ))
        }
        "WebcastLuckyMoneyMessage" => {
            let m: LuckyMoney = parse(payload)?;
            let common: Common = parse(m.common.as_deref().unwrap_or_default())?;
            let user: User = parse(m.user.as_deref().unwrap_or_default())?;
            let mut parts = vec!["红包通知".to_owned()];
            if let Some(n) = m.diamond_count {
                parts.push(format!("钻石数量：{n}"));
            }
            Ok(event(
                method,
                "red_packet",
                "notice",
                numeric_id(m.id),
                &common,
                &user,
                parts,
                0,
                fallback,
                json!({"diamond_count": number(m.diamond_count)}),
            ))
        }
        "WebcastLotteryDrawResultEventMessage" => {
            let m: LotteryResult = parse(payload)?;
            let common: Common = parse(m.common.as_deref().unwrap_or_default())?;
            Ok(event(
                method,
                "lottery",
                "draw_result",
                numeric_id(m.lottery_id),
                &common,
                &User::default(),
                vec!["福袋开奖结果通知".into()],
                0,
                fallback,
                json!({}),
            ))
        }
        _ => {
            let m: BoxNotice = if method == "WebcastLuckyBoxTempStatusMessage" {
                BoxNotice {
                    common: parse::<CommonOnly>(payload)?.common,
                    ..Default::default()
                }
            } else {
                parse(payload)?
            };
            let common: Common = parse(m.common.as_deref().unwrap_or_default())?;
            let (stage, text) = match method {
                "WebcastLuckyBoxEndMessage" => ("ended", "红包结束通知"),
                "WebcastLuckyBoxRewardMessage" => ("reward", "红包奖励通知"),
                _ => ("status", "红包状态通知（未提供详情）"),
            };
            let reward_type = if method == "WebcastLuckyBoxRewardMessage" {
                parse::<RewardType>(payload)?.value
            } else {
                None
            };
            Ok(event(
                method,
                "red_packet",
                stage,
                id(&m.box_id_str, m.box_id),
                &common,
                &User::default(),
                vec![text.into()],
                0,
                fallback,
                json!({"reward_message_type": number(reward_type)}),
            ))
        }
    }
}
