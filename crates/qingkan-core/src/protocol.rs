//! Bounded Douyin wire decoding. Tags come from the checked-in douyin.proto.
//!
//! Only fields used by the collector are materialized. Unknown message methods
//! and unused fields are ignored; IDs are strings before crossing the FFI/JSON
//! boundary. This module never performs network I/O or logs message contents.

use flate2::bufread::GzDecoder;
use prost::Message as ProstMessage;
use serde::Serialize;
use serde_json::{json, Value};
use std::io::Read;

#[path = "protocol_activities.rs"]
mod activities;

pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
pub const MAX_MESSAGES_PER_FRAME: usize = 16_384;
const MAX_VIEWERS: i64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProtocolError {
    #[error("直播消息包超过大小限制")]
    FrameTooLarge,
    #[error("直播消息解压后超过大小限制")]
    ResponseTooLarge,
    #[error("直播消息包内消息数量超过限制")]
    TooManyMessages,
    #[error("单条直播消息超过大小限制")]
    MessageTooLarge,
    #[error("直播消息包格式无效")]
    InvalidFrame,
    #[error("直播消息压缩数据无效")]
    InvalidCompression,
    #[error("直播消息响应格式无效")]
    InvalidResponse,
    #[error("直播消息格式无效")]
    InvalidMessage,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Event {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub payload: Value,
}

#[derive(Debug, PartialEq)]
pub struct DecodedFrame {
    pub ack: Option<Vec<u8>>,
    pub events: Vec<Event>,
    /// Malformed/oversized supported payloads; never contains payload text.
    pub skipped_messages: usize,
}

// Minimal, private prost projections of the local schema. In particular,
// User.id is uint64 (not signed/double), Fansclub uses commonInfo at tag 1,
// Gift.user is tag 7, Like.user is tag 5, and viewers.total is signed tag 3.
// Nested content messages use opaque bytes before projection so repeated
// occurrences keep the legacy betterproto last-occurrence-wins behavior.
#[derive(Clone, PartialEq, ProstMessage)]
struct PushFrame {
    #[prost(uint64, tag = "2")]
    log_id: u64,
    #[prost(string, tag = "7")]
    payload_type: String,
    #[prost(bytes = "vec", tag = "8")]
    payload: Vec<u8>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct Response {
    #[prost(message, repeated, tag = "1")]
    messages: Vec<Envelope>,
    #[prost(string, tag = "5")]
    internal_ext: String,
    #[prost(bool, tag = "9")]
    need_ack: bool,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct Envelope {
    #[prost(string, tag = "1")]
    method: String,
    #[prost(bytes = "vec", tag = "2")]
    payload: Vec<u8>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct Common {
    #[prost(uint64, tag = "2")]
    msg_id: u64,
    #[prost(uint64, tag = "3")]
    room_id: u64,
    #[prost(uint64, tag = "4")]
    create_time: u64,
    #[prost(string, tag = "7")]
    describe: String,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct User {
    #[prost(uint64, tag = "1")]
    id: u64,
    #[prost(string, tag = "3")]
    nick_name: String,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct ChatMessage {
    #[prost(bytes = "vec", optional, tag = "1")]
    common: Option<Vec<u8>>,
    #[prost(bytes = "vec", optional, tag = "2")]
    user: Option<Vec<u8>>,
    #[prost(string, tag = "3")]
    content: String,
    #[prost(uint64, tag = "15")]
    event_time: u64,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct GiftMessage {
    #[prost(bytes = "vec", optional, tag = "1")]
    common: Option<Vec<u8>>,
    #[prost(uint64, tag = "4")]
    group_count: u64,
    #[prost(uint64, tag = "5")]
    repeat_count: u64,
    #[prost(uint64, tag = "6")]
    combo_count: u64,
    #[prost(bytes = "vec", optional, tag = "7")]
    user: Option<Vec<u8>>,
    #[prost(bytes = "vec", optional, tag = "15")]
    gift: Option<Vec<u8>>,
    #[prost(uint64, tag = "33")]
    send_time: u64,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct Gift {
    #[prost(string, tag = "16")]
    name: String,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct LikeMessage {
    #[prost(bytes = "vec", optional, tag = "1")]
    common: Option<Vec<u8>>,
    #[prost(uint64, tag = "2")]
    count: u64,
    #[prost(bytes = "vec", optional, tag = "5")]
    user: Option<Vec<u8>>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct MemberMessage {
    #[prost(bytes = "vec", optional, tag = "1")]
    common: Option<Vec<u8>>,
    #[prost(bytes = "vec", optional, tag = "2")]
    user: Option<Vec<u8>>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct SocialMessage {
    #[prost(bytes = "vec", optional, tag = "1")]
    common: Option<Vec<u8>>,
    #[prost(bytes = "vec", optional, tag = "2")]
    user: Option<Vec<u8>>,
    #[prost(uint64, tag = "4")]
    action: u64,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct FansclubMessage {
    #[prost(bytes = "vec", optional, tag = "1")]
    common: Option<Vec<u8>>,
    #[prost(string, tag = "3")]
    content: String,
    #[prost(bytes = "vec", optional, tag = "4")]
    user: Option<Vec<u8>>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct EmojiChatMessage {
    #[prost(bytes = "vec", optional, tag = "1")]
    common: Option<Vec<u8>>,
    #[prost(bytes = "vec", optional, tag = "2")]
    user: Option<Vec<u8>>,
    #[prost(string, tag = "5")]
    content: String,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct ControlMessage {
    #[prost(bytes = "vec", optional, tag = "1")]
    common: Option<Vec<u8>>,
    #[prost(int32, tag = "2")]
    status: i32,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct RoomUserSeqMessage {
    #[prost(bytes = "vec", optional, tag = "1")]
    common: Option<Vec<u8>>,
    // Optional is deliberate: an absent scalar is unknown, explicit 0 is zero.
    #[prost(int64, optional, tag = "3")]
    total: Option<i64>,
    #[prost(string, tag = "10")]
    online_user_for_anchor: String,
}

fn numeric_id(value: u64) -> String {
    if value == 0 {
        String::new()
    } else {
        value.to_string()
    }
}

fn fallback_id(value: &str) -> &str {
    let value = value.trim();
    if !value.is_empty()
        && value.bytes().all(|v| v.is_ascii_digit())
        && value.bytes().any(|v| v != b'0')
    {
        value
    } else {
        ""
    }
}

fn room_id(common: &Common, fallback: &str) -> String {
    if common.room_id > 0 {
        common.room_id.to_string()
    } else {
        fallback_id(fallback).to_owned()
    }
}

fn epoch_seconds(value: u64) -> u64 {
    if value >= 100_000_000_000_000 {
        return 0;
    }
    let seconds = if value >= 100_000_000_000 {
        value / 1000
    } else {
        value
    };
    // Match the original Windows datetime.fromtimestamp / CRT gmtime64 bound.
    // It is deliberately stable on other native build hosts as well.
    if seconds > 32_536_850_399 {
        0
    } else {
        seconds
    }
}

fn time_string(seconds: u64) -> String {
    if seconds == 0 {
        return String::new();
    }
    let time = (seconds + 8 * 3600) % 86400;
    format!(
        "{:02}:{:02}:{:02}",
        time / 3600,
        time % 3600 / 60,
        time % 60
    )
}

fn content_event(
    kind: &'static str,
    label: &str,
    common: &Common,
    user: &User,
    content: String,
    source_time: u64,
    fallback: &str,
) -> Event {
    let mut seconds = epoch_seconds(source_time);
    if seconds == 0 {
        seconds = epoch_seconds(common.create_time);
    }
    let time = time_string(seconds);
    let display_time = if time.is_empty() {
        "时间未知"
    } else {
        &time
    };
    let text = format!("[{display_time}] 【{label}】 {}: {content}", user.nick_name);
    Event {
        kind,
        payload: json!({
            "sender_id": numeric_id(user.id), "sender_name": user.nick_name,
            "user_name": user.nick_name, "content": content,
            "message_id": numeric_id(common.msg_id), "room_id": room_id(common, fallback),
            "event_time": seconds, "time_str": time, "text": text,
        }),
    }
}

fn parse<M: ProstMessage + Default>(payload: &[u8]) -> Result<M, ProtocolError> {
    M::decode(payload).map_err(|_| ProtocolError::InvalidMessage)
}

/// Decode one supported message. Unknown types have no effect, including all
/// cumulative/ranking/room statistics. Current viewers are UI-only `stats`.
pub fn decode_message(
    method: &str,
    payload: &[u8],
    fallback: &str,
) -> Result<Option<Event>, ProtocolError> {
    let supported = activities::supports(method)
        || matches!(
            method,
            "WebcastChatMessage"
                | "WebcastGiftMessage"
                | "WebcastLikeMessage"
                | "WebcastMemberMessage"
                | "WebcastSocialMessage"
                | "WebcastFansclubMessage"
                | "WebcastEmojiChatMessage"
                | "WebcastControlMessage"
                | "WebcastRoomUserSeqMessage"
                | "WebcastRoomDataSyncMessage"
        );
    if !supported {
        return Ok(None);
    }
    if payload.len() > MAX_MESSAGE_BYTES {
        return Err(ProtocolError::MessageTooLarge);
    }
    if method == "WebcastRoomDataSyncMessage" {
        return activities::decode_sync(payload, fallback);
    }
    if activities::supports(method) {
        return activities::decode(method, payload, fallback).map(Some);
    }
    let event = match method {
        "WebcastChatMessage" => {
            let m: ChatMessage = parse(payload)?;
            let common: Common = parse(m.common.as_deref().unwrap_or_default())?;
            let user: User = parse(m.user.as_deref().unwrap_or_default())?;
            content_event(
                "chat",
                "弹幕",
                &common,
                &user,
                m.content,
                m.event_time,
                fallback,
            )
        }
        "WebcastGiftMessage" => {
            let m: GiftMessage = parse(payload)?;
            let common: Common = parse(m.common.as_deref().unwrap_or_default())?;
            let user: User = parse(m.user.as_deref().unwrap_or_default())?;
            let name = parse::<Gift>(m.gift.as_deref().unwrap_or_default())?.name;
            let count = [m.combo_count, m.repeat_count, m.group_count]
                .into_iter()
                .find(|n| *n != 0)
                .unwrap_or(0);
            let mut e = content_event(
                "gift",
                "礼物",
                &common,
                &user,
                format!("送出 {name} x{count}"),
                m.send_time,
                fallback,
            );
            e.payload["gift_name"] = json!(name);
            e.payload["gift_cnt"] = json!(count);
            e
        }
        "WebcastLikeMessage" => {
            let m: LikeMessage = parse(payload)?;
            let common: Common = parse(m.common.as_deref().unwrap_or_default())?;
            let user: User = parse(m.user.as_deref().unwrap_or_default())?;
            let mut e = content_event(
                "like",
                "点赞",
                &common,
                &user,
                format!("点了{}个赞", m.count),
                0,
                fallback,
            );
            e.payload["count"] = json!(m.count);
            e
        }
        "WebcastMemberMessage" => {
            let m: MemberMessage = parse(payload)?;
            let common: Common = parse(m.common.as_deref().unwrap_or_default())?;
            let user: User = parse(m.user.as_deref().unwrap_or_default())?;
            let mut e = content_event(
                "member",
                "进场",
                &common,
                &user,
                "进入了直播间".into(),
                0,
                fallback,
            );
            e.payload["user_id"] = json!(numeric_id(user.id));
            e
        }
        "WebcastSocialMessage" => {
            let m: SocialMessage = parse(payload)?;
            let common: Common = parse(m.common.as_deref().unwrap_or_default())?;
            let user: User = parse(m.user.as_deref().unwrap_or_default())?;
            let content = if common.describe.is_empty() {
                format!("直播间互动（动作 {}）", m.action)
            } else {
                common.describe.clone()
            };
            let mut e = content_event("social", "互动", &common, &user, content, 0, fallback);
            e.payload["user_id"] = json!(numeric_id(user.id));
            e.payload["action"] = json!(m.action);
            e
        }
        "WebcastFansclubMessage" => {
            let m: FansclubMessage = parse(payload)?;
            let common: Common = parse(m.common.as_deref().unwrap_or_default())?;
            let user: User = parse(m.user.as_deref().unwrap_or_default())?;
            content_event("fansclub", "粉丝团", &common, &user, m.content, 0, fallback)
        }
        "WebcastEmojiChatMessage" => {
            let m: EmojiChatMessage = parse(payload)?;
            let common: Common = parse(m.common.as_deref().unwrap_or_default())?;
            let user: User = parse(m.user.as_deref().unwrap_or_default())?;
            content_event("emoji", "表情", &common, &user, m.content, 0, fallback)
        }
        "WebcastControlMessage" => {
            let m: ControlMessage = parse(payload)?;
            let common: Common = parse(m.common.as_deref().unwrap_or_default())?;
            if m.status != 3 {
                return Ok(None);
            }
            let mut e = content_event(
                "control",
                "",
                &common,
                &User::default(),
                "直播间已结束".into(),
                0,
                fallback,
            );
            e.payload["text"] = json!("直播间已结束");
            e
        }
        "WebcastRoomUserSeqMessage" => {
            let m: RoomUserSeqMessage = parse(payload)?;
            let common: Common = parse(m.common.as_deref().unwrap_or_default())?;
            let current = match m.total {
                Some(n) => Some(n),
                None => {
                    let text = m.online_user_for_anchor.trim();
                    if !text.is_empty() && text.bytes().all(|v| v.is_ascii_digit()) {
                        text.parse::<i64>().ok()
                    } else {
                        None
                    }
                }
            };
            let Some(current) = current.filter(|v| (0..=MAX_VIEWERS).contains(v)) else {
                return Ok(None);
            };
            Event {
                kind: "stats",
                payload: json!({"viewer_count": current,
                "room_id": room_id(&common, fallback)}),
            }
        }
        _ => unreachable!(),
    };
    Ok(Some(event))
}

/// Check repetition count before prost allocates a Vec of envelope structs.
/// The encoding helpers are from the workspace's pinned prost version and keep
/// its recursion/unknown-group checks, rather than a second protobuf parser.
fn check_response_count(mut payload: &[u8]) -> Result<(), ProtocolError> {
    use prost::encoding::{decode_key, skip_field, DecodeContext};
    let mut count = 0;
    while !payload.is_empty() {
        let (tag, wire) = decode_key(&mut payload).map_err(|_| ProtocolError::InvalidResponse)?;
        if tag == 1 {
            count += 1;
            if count > MAX_MESSAGES_PER_FRAME {
                return Err(ProtocolError::TooManyMessages);
            }
        }
        skip_field(wire, tag, &mut payload, DecodeContext::default())
            .map_err(|_| ProtocolError::InvalidResponse)?;
    }
    Ok(())
}

/// Decode a whole WebSocket push in one native call. A malformed outer frame,
/// gzip stream or response rejects the frame. Individual bad message payloads
/// are isolated. End-of-broadcast is a barrier, matching the collector's stop.
pub fn decode_frame(frame: &[u8], fallback: &str) -> Result<DecodedFrame, ProtocolError> {
    if frame.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge);
    }
    let package = PushFrame::decode(frame).map_err(|_| ProtocolError::InvalidFrame)?;
    let mut inflated = Vec::new();
    // Match gzip.decompress: empty input and zero padding after a completed
    // member are accepted; arbitrary nonzero trailing data is still rejected.
    let mut compressed = package.payload.as_slice();
    while !compressed.is_empty() {
        let mut decoder = GzDecoder::new(compressed);
        decoder
            .by_ref()
            .take((MAX_RESPONSE_BYTES + 1 - inflated.len()) as u64)
            .read_to_end(&mut inflated)
            .map_err(|_| ProtocolError::InvalidCompression)?;
        if inflated.len() > MAX_RESPONSE_BYTES {
            return Err(ProtocolError::ResponseTooLarge);
        }
        compressed = decoder.into_inner();
        while compressed.first() == Some(&0) {
            compressed = &compressed[1..];
        }
    }
    check_response_count(&inflated)?;
    let response =
        Response::decode(inflated.as_slice()).map_err(|_| ProtocolError::InvalidResponse)?;
    let ack = response.need_ack.then(|| {
        PushFrame {
            log_id: package.log_id,
            payload_type: "ack".into(),
            payload: response.internal_ext.into_bytes(),
        }
        .encode_to_vec()
    });
    let mut events = Vec::new();
    let mut skipped_messages = 0;
    for message in response.messages {
        match decode_message(&message.method, &message.payload, fallback) {
            Ok(Some(event)) => {
                let ended = event.kind == "control";
                events.push(event);
                if ended {
                    break;
                }
            }
            Ok(None) => (),
            Err(_) => skipped_messages += 1,
        }
    }
    Ok(DecodedFrame {
        ack,
        events,
        skipped_messages,
    })
}

/// Opcode selection remains with the socket owner (PING for hb, BINARY for ack).
pub fn hb_frame() -> Vec<u8> {
    PushFrame {
        payload_type: "hb".into(),
        ..Default::default()
    }
    .encode_to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }
    fn frame(response: Response) -> Vec<u8> {
        PushFrame {
            log_id: u64::MAX,
            payload: gzip(&response.encode_to_vec()),
            ..Default::default()
        }
        .encode_to_vec()
    }
    fn envelope(method: &str, payload: Vec<u8>) -> Envelope {
        Envelope {
            method: method.into(),
            payload,
        }
    }
    fn chat() -> ChatMessage {
        ChatMessage {
            common: Some(
                Common {
                    msg_id: u64::MAX,
                    room_id: u64::MAX - 1,
                    create_time: 1_800_000_001,
                    ..Default::default()
                }
                .encode_to_vec(),
            ),
            user: Some(
                User {
                    id: u64::MAX - 2,
                    nick_name: "测试用户".into(),
                }
                .encode_to_vec(),
            ),
            content: "消息😀\n第二行".into(),
            event_time: 1_800_000_000_123,
        }
    }

    #[test]
    fn ack_heartbeat_and_maximum_ids_are_exact() {
        let data = frame(Response {
            need_ack: true,
            internal_ext: "确认😀".into(),
            messages: vec![envelope("WebcastChatMessage", chat().encode_to_vec())],
        });
        let result = decode_frame(&data, "11").unwrap();
        let ack = PushFrame::decode(result.ack.unwrap().as_slice()).unwrap();
        assert_eq!(ack.log_id, u64::MAX);
        assert_eq!(ack.payload_type, "ack");
        assert_eq!(ack.payload, "确认😀".as_bytes());
        assert_eq!(hb_frame(), b"\x3a\x02hb");
        let p = &result.events[0].payload;
        assert_eq!(p["sender_id"], (u64::MAX - 2).to_string());
        assert_eq!(p["message_id"], u64::MAX.to_string());
        assert_eq!(p["room_id"], (u64::MAX - 1).to_string());
        assert_eq!(p["event_time"], 1_800_000_000u64);
        assert_eq!(result.skipped_messages, 0);
    }

    #[test]
    fn absent_time_identity_and_zero_are_not_fabricated() {
        let result = decode_message("WebcastChatMessage", &[], " 000123 ")
            .unwrap()
            .unwrap();
        assert_eq!(result.payload["room_id"], "000123");
        for key in ["sender_id", "message_id", "time_str", "sender_name"] {
            assert_eq!(result.payload[key], "");
        }
        assert_eq!(result.payload["event_time"], 0);
        assert_eq!(
            decode_message("WebcastChatMessage", &[], "0")
                .unwrap()
                .unwrap()
                .payload["room_id"],
            ""
        );
        assert_eq!(
            decode_message("WebcastChatMessage", &[], "12e3")
                .unwrap()
                .unwrap()
                .payload["room_id"],
            ""
        );
    }

    #[test]
    fn source_time_priority_and_rejected_microseconds() {
        let mut m = chat();
        m.event_time = 1_800_000_000_000_000;
        assert_eq!(
            decode_message("WebcastChatMessage", &m.encode_to_vec(), "")
                .unwrap()
                .unwrap()
                .payload["event_time"],
            1_800_000_001u64
        );
        let gift = GiftMessage {
            common: m.common,
            send_time: 1_800_000_002_999,
            ..Default::default()
        };
        assert_eq!(
            decode_message("WebcastGiftMessage", &gift.encode_to_vec(), "")
                .unwrap()
                .unwrap()
                .payload["event_time"],
            1_800_000_002u64
        );
        assert_eq!(epoch_seconds(99_999_999_999_999), 0);
        assert_eq!(epoch_seconds(32_536_850_399), 32_536_850_399);
        assert_eq!(epoch_seconds(32_536_850_400), 0);
        assert_eq!(epoch_seconds(100_000_000_000_000), 0);
        assert_eq!(time_string(1), "08:00:01");
    }

    #[test]
    fn gift_count_and_social_description_do_not_change_meaning() {
        for (combo, repeat, group, expected) in
            [(3, 2, 1, 3), (0, 2, 1, 2), (0, 0, 1, 1), (0, 0, 0, 0)]
        {
            let m = GiftMessage {
                combo_count: combo,
                repeat_count: repeat,
                group_count: group,
                gift: Some(Gift { name: "花".into() }.encode_to_vec()),
                ..Default::default()
            };
            let e = decode_message("WebcastGiftMessage", &m.encode_to_vec(), "")
                .unwrap()
                .unwrap();
            assert_eq!(e.payload["gift_cnt"], expected);
            assert_eq!(e.payload["content"], format!("送出 花 x{expected}"));
        }
        let m = SocialMessage {
            action: 999,
            ..Default::default()
        };
        assert_eq!(
            decode_message("WebcastSocialMessage", &m.encode_to_vec(), "")
                .unwrap()
                .unwrap()
                .payload["content"],
            "直播间互动（动作 999）"
        );
        let m = SocialMessage {
            common: Some(
                Common {
                    describe: "分享直播".into(),
                    ..Default::default()
                }
                .encode_to_vec(),
            ),
            ..m
        };
        assert_eq!(
            decode_message("WebcastSocialMessage", &m.encode_to_vec(), "")
                .unwrap()
                .unwrap()
                .payload["content"],
            "分享直播"
        );
    }

    #[test]
    fn viewers_require_exact_current_count_and_presence() {
        assert_eq!(
            decode_message("WebcastRoomUserSeqMessage", &[], "1").unwrap(),
            None
        );
        let zero = RoomUserSeqMessage {
            total: Some(0),
            online_user_for_anchor: "999".into(),
            ..Default::default()
        };
        assert_eq!(
            decode_message("WebcastRoomUserSeqMessage", &zero.encode_to_vec(), "1")
                .unwrap()
                .unwrap()
                .payload,
            json!({"viewer_count":0,"room_id":"1"})
        );
        for n in [-1, MAX_VIEWERS + 1] {
            let invalid = RoomUserSeqMessage {
                total: Some(n),
                online_user_for_anchor: "10".into(),
                ..Default::default()
            };
            assert_eq!(
                decode_message("WebcastRoomUserSeqMessage", &invalid.encode_to_vec(), "").unwrap(),
                None
            );
        }
        for text in ["1.2万", "1.2", "-1", "", "9007199254740992"] {
            let m = RoomUserSeqMessage {
                online_user_for_anchor: text.into(),
                ..Default::default()
            };
            assert_eq!(
                decode_message("WebcastRoomUserSeqMessage", &m.encode_to_vec(), "").unwrap(),
                None
            );
        }
        let m = RoomUserSeqMessage {
            online_user_for_anchor: " 00037 ".into(),
            ..Default::default()
        };
        assert_eq!(
            decode_message("WebcastRoomUserSeqMessage", &m.encode_to_vec(), "")
                .unwrap()
                .unwrap()
                .payload["viewer_count"],
            37
        );
        // Tags 7/8 are cumulative viewers and must never become current viewers.
        assert_eq!(
            decode_message("WebcastRoomUserSeqMessage", &[0x38, 100, 0x42, 1, b'9'], "").unwrap(),
            None
        );
    }

    #[test]
    fn unknown_types_and_fields_are_ignored() {
        for method in [
            "WebcastRoomStatsMessage",
            "WebcastRoomRankMessage",
            "WebcastRoomMessage",
            "WebcastRoomStreamAdaptationMessage",
            "future",
        ] {
            assert_eq!(decode_message(method, &[0xff], "").unwrap(), None);
        }
        let mut data = chat().encode_to_vec();
        data.extend_from_slice(&[0xa2, 0x06, 3, 1, 2, 3]); // unknown length field 100
        assert_eq!(
            decode_message("WebcastChatMessage", &data, "").unwrap(),
            decode_message("WebcastChatMessage", &chat().encode_to_vec(), "").unwrap()
        );
    }

    #[test]
    fn one_bad_message_does_not_drop_ack_or_next_message() {
        let data = frame(Response {
            need_ack: true,
            messages: vec![
                envelope("WebcastChatMessage", vec![0x1a, 0x02, 0xff]),
                envelope("WebcastChatMessage", chat().encode_to_vec()),
            ],
            ..Default::default()
        });
        let result = decode_frame(&data, "").unwrap();
        assert!(result.ack.is_some());
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.skipped_messages, 1);
    }

    #[test]
    fn control_stops_subsequent_frame_events_but_other_codes_do_not() {
        let ended = ControlMessage {
            status: 3,
            ..Default::default()
        }
        .encode_to_vec();
        let data = frame(Response {
            messages: vec![
                envelope("WebcastControlMessage", ended),
                envelope("WebcastChatMessage", chat().encode_to_vec()),
            ],
            ..Default::default()
        });
        let result = decode_frame(&data, "99").unwrap();
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].kind, "control");
        assert_eq!(result.events[0].payload["text"], "直播间已结束");
        for status in [0, 1, 2, 4, -1] {
            assert_eq!(
                decode_message(
                    "WebcastControlMessage",
                    &ControlMessage {
                        status,
                        ..Default::default()
                    }
                    .encode_to_vec(),
                    ""
                )
                .unwrap(),
                None
            );
        }
    }

    #[test]
    fn invalid_inputs_are_bounded_and_errors_do_not_echo_data() {
        assert_eq!(
            decode_frame(&vec![0; MAX_FRAME_BYTES + 1], ""),
            Err(ProtocolError::FrameTooLarge)
        );
        assert_eq!(
            decode_message("WebcastChatMessage", &vec![0; MAX_MESSAGE_BYTES + 1], ""),
            Err(ProtocolError::MessageTooLarge)
        );
        assert_eq!(
            decode_frame(b"private-invalid-frame", ""),
            Err(ProtocolError::InvalidFrame)
        );
        let data = PushFrame {
            payload: b"private-not-gzip".to_vec(),
            ..Default::default()
        }
        .encode_to_vec();
        assert_eq!(
            decode_frame(&data, ""),
            Err(ProtocolError::InvalidCompression)
        );
        assert!(!ProtocolError::InvalidCompression
            .to_string()
            .contains("private"));
        let data = PushFrame {
            payload: gzip(&vec![0; MAX_RESPONSE_BYTES + 1]),
            ..Default::default()
        }
        .encode_to_vec();
        assert_eq!(
            decode_frame(&data, ""),
            Err(ProtocolError::ResponseTooLarge)
        );
    }

    #[test]
    fn repetition_limit_precedes_envelope_allocation() {
        let response = [0x0a, 0x00].repeat(MAX_MESSAGES_PER_FRAME + 1);
        let data = PushFrame {
            payload: gzip(&response),
            ..Default::default()
        }
        .encode_to_vec();
        assert_eq!(decode_frame(&data, ""), Err(ProtocolError::TooManyMessages));
    }

    #[test]
    fn gzip_crc_and_truncation_are_verified() {
        let compressed = gzip(&Response::default().encode_to_vec());
        let mut corrupt = compressed.clone();
        let last = corrupt.len() - 5;
        corrupt[last] ^= 0x01;
        for payload in [corrupt, compressed[..compressed.len() - 3].to_vec()] {
            let data = PushFrame {
                payload,
                ..Default::default()
            }
            .encode_to_vec();
            assert_eq!(
                decode_frame(&data, ""),
                Err(ProtocolError::InvalidCompression)
            );
        }
    }

    #[test]
    fn concatenated_gzip_members_follow_python_behavior() {
        let body = Response {
            need_ack: true,
            ..Default::default()
        }
        .encode_to_vec();
        let mut compressed = gzip(&body[..1]);
        compressed.extend(gzip(&body[1..]));
        let data = PushFrame {
            payload: compressed,
            ..Default::default()
        }
        .encode_to_vec();
        assert!(decode_frame(&data, "").unwrap().ack.is_some());
    }

    #[test]
    fn empty_gzip_and_zero_padding_follow_python_behavior() {
        assert_eq!(decode_frame(&[], "").unwrap().events, Vec::new());
        let body = Response {
            need_ack: true,
            ..Default::default()
        }
        .encode_to_vec();
        let mut compressed = gzip(&body[..1]);
        compressed.extend([0, 0, 0]);
        compressed.extend(gzip(&body[1..]));
        compressed.extend([0, 0]);
        let data = PushFrame {
            payload: compressed,
            ..Default::default()
        }
        .encode_to_vec();
        assert!(decode_frame(&data, "").unwrap().ack.is_some());
    }

    #[test]
    fn duplicate_nested_fields_use_legacy_last_occurrence() {
        let mut bytes = ChatMessage {
            common: Some(
                Common {
                    msg_id: 123,
                    ..Default::default()
                }
                .encode_to_vec(),
            ),
            user: Some(
                User {
                    id: 456,
                    ..Default::default()
                }
                .encode_to_vec(),
            ),
            ..Default::default()
        }
        .encode_to_vec();
        bytes.extend(
            ChatMessage {
                common: Some(
                    Common {
                        room_id: 789,
                        ..Default::default()
                    }
                    .encode_to_vec(),
                ),
                user: Some(
                    User {
                        nick_name: "名称".into(),
                        ..Default::default()
                    }
                    .encode_to_vec(),
                ),
                ..Default::default()
            }
            .encode_to_vec(),
        );
        let e = decode_message("WebcastChatMessage", &bytes, "")
            .unwrap()
            .unwrap();
        assert_eq!(e.payload["message_id"], "");
        assert_eq!(e.payload["sender_id"], "");
        assert_eq!(e.payload["room_id"], "789");
        assert_eq!(e.payload["sender_name"], "名称");
    }
}
