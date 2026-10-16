//! Anonymous HTTPS/WSS transport. No login cookie input or credential store.
use crate::{proxy::Route, signature, storage};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_tungstenite::{
    tungstenite::{
        client::IntoClientRequest,
        protocol::{Message, WebSocketConfig},
    },
    MaybeTlsStream, WebSocketStream,
};
use tokio_util::sync::CancellationToken;

pub const USER_AGENT:&str="Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("任务已停止")]
    Cancelled,
    #[error("抖音请求失败：HTTP {0}")]
    Status(u16),
    #[error("直播间信息暂不可用，正在确认是否下播")]
    Unavailable,
    #[error("{0}")]
    Invalid(&'static str),
    #[error("网络或 TLS 连接失败")]
    Network,
    #[error("消息连接已关闭")]
    Closed,
}
pub async fn body(
    mut response: reqwest::Response,
    max: usize,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, Error> {
    if response.status() != 200 {
        return Err(Error::Status(response.status().as_u16()));
    }
    if response.content_length().is_some_and(|n| n > max as u64) {
        return Err(Error::Invalid("响应超过大小限制"));
    }
    let mut out = Vec::new();
    loop {
        let next = tokio::select! {biased;_=cancel.cancelled()=>return Err(Error::Cancelled),v=response.chunk()=>v.map_err(|_|Error::Network)?};
        match next {
            None => return Ok(out),
            Some(chunk) => {
                if out.len() + chunk.len() > max {
                    return Err(Error::Invalid("响应超过大小限制"));
                }
                out.extend_from_slice(&chunk)
            }
        }
    }
}
#[derive(Clone)]
pub struct Visitor {
    pub cookie: String,
    pub did: String,
}
pub async fn visitor(route: &Route, cancel: &CancellationToken) -> Result<Visitor, Error> {
    let _slot = route
        .admit(false, cancel)
        .await
        .map_err(|_| Error::Cancelled)?;
    let response = tokio::select! {biased;_=cancel.cancelled()=>return Err(Error::Cancelled),r=route.client.get("https://live.douyin.com/").send()=>r.map_err(|_|Error::Network)?};
    if response.status() != 200 {
        return Err(Error::Status(response.status().as_u16()));
    }
    let ttwid = response
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|s| s.strip_prefix("ttwid="))
        .filter_map(|s| s.split(';').next())
        .find(|s| !s.is_empty() && s.len() < 4096 && !s.chars().any(char::is_control))
        .ok_or(Error::Invalid("抖音未返回游客凭据，请稍后重试"))?
        .to_owned();
    // Only response headers are needed. Do not download a complete homepage
    // for every room or retain a response body that the collector never uses.
    drop(response);
    use rand::Rng;
    let did = rand::rng()
        .random_range(7_000_000_000_000_000_000u64..7_999_999_999_999_999_999u64)
        .to_string();
    Ok(Visitor {
        cookie: format!("ttwid={ttwid}"),
        did,
    })
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct RoomInfo {
    pub nickname: String,
    pub is_live: bool,
    pub title: String,
    pub anchor_id: String,
    pub room_id: String,
    pub viewer_count: Option<u64>,
    pub started_at: Option<u64>,
}
fn numeric(v: &Value) -> String {
    let s = v
        .as_str()
        .map(str::to_owned)
        .or_else(|| v.as_u64().map(|n| n.to_string()))
        .unwrap_or_default();
    if storage::id(&s) {
        s
    } else {
        String::new()
    }
}
fn epoch(v: &Value) -> Option<u64> {
    let n = v
        .as_u64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))?;
    let n = if n > 10_000_000_000 { n / 1000 } else { n };
    (n > 0 && n < 253402300800).then_some(n)
}
pub fn parse_room(v: &Value) -> Result<RoomInfo, Error> {
    if v["status_code"].as_i64().unwrap_or(0) != 0 {
        return Err(Error::Unavailable);
    }
    let data = &v["data"];
    let room = &data["data"][0];
    let state = room["status"]
        .as_u64()
        .or_else(|| room["status"].as_str().and_then(|s| s.parse().ok()))
        .filter(|s| (1..=4).contains(s))
        .ok_or(Error::Invalid("未取得明确直播状态，请稍后重试"))?;
    let user = &data["user"];
    let owner = &room["owner"];
    let anchor_id = [
        &owner["id_str"],
        &owner["id"],
        &user["id_str"],
        &user["id"],
        &room["owner_user_id_str"],
        &room["owner_user_id"],
    ]
    .iter()
    .map(|v| numeric(v))
    .find(|s| !s.is_empty())
    .unwrap_or_default();
    let viewer_count = ["user_count", "online_count", "user_count_str"]
        .iter()
        .find_map(|k| {
            room[*k]
                .as_u64()
                .or_else(|| room[*k].as_str().and_then(|s| s.parse().ok()))
        })
        .filter(|n| *n <= 9_007_199_254_740_991);
    Ok(RoomInfo {
        nickname: user["nickname"]
            .as_str()
            .or_else(|| owner["nickname"].as_str())
            .unwrap_or("")
            .chars()
            .take(200)
            .collect(),
        is_live: state == 2,
        title: room["title"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(500)
            .collect(),
        anchor_id,
        room_id: [&room["id_str"], &room["id"]]
            .iter()
            .map(|v| numeric(v))
            .find(|s| !s.is_empty())
            .unwrap_or_default(),
        viewer_count: if state == 2 { viewer_count } else { None },
        started_at: epoch(&room["start_time"]),
    })
}
pub async fn room(
    route: &Route,
    visitor: &Visitor,
    rid: &str,
    cancel: &CancellationToken,
) -> Result<RoomInfo, Error> {
    if !storage::id(rid) {
        return Err(Error::Invalid("直播间 ID 无效"));
    }
    let _slot = route
        .admit(false, cancel)
        .await
        .map_err(|_| Error::Cancelled)?;
    let token = {
        use rand::distr::SampleString;
        rand::distr::Alphanumeric.sample_string(&mut rand::rng(), 107)
    };
    let params = [
        ("aid", "6383"),
        ("app_name", "douyin_web"),
        ("live_id", "1"),
        ("device_platform", "web"),
        ("language", "zh-CN"),
        ("enter_from", "page_refresh"),
        ("cookie_enabled", "true"),
        ("screen_width", "1920"),
        ("screen_height", "1080"),
        ("browser_language", "zh-CN"),
        ("browser_platform", "Win32"),
        ("browser_name", "Chrome"),
        ("browser_version", "140.0.0.0"),
        ("web_rid", rid),
        ("room_id_str", rid),
        ("is_need_double_stream", "false"),
        ("msToken", &token),
    ];
    let mut target = url::Url::parse("https://live.douyin.com/webcast/room/web/enter/").unwrap();
    target.query_pairs_mut().extend_pairs(params);
    let request = route
        .client
        .get(target)
        .header("Cookie", &visitor.cookie)
        .header("Referer", format!("https://live.douyin.com/{rid}"));
    let response = tokio::select! {biased;_=cancel.cancelled()=>return Err(Error::Cancelled),v=request.send()=>v.map_err(|_|Error::Network)?};
    let raw = body(response, 2 * 1024 * 1024, cancel).await?;
    let value: Value = serde_json::from_slice(&raw)
        .map_err(|_| Error::Invalid("直播间信息不是有效 JSON，可能遇到验证页面"))?;
    parse_room(&value)
}
pub fn websocket_url(room: &str, did: &str, millis: u64) -> Result<url::Url, Error> {
    if !storage::id(room) || !storage::id(did) {
        return Err(Error::Invalid("真实场次或游客标识无效"));
    }
    let mut u =
        url::Url::parse("wss://webcast100-ws-web-lq.douyin.com/webcast/im/push/v2/").unwrap();
    let cursor = format!("d-1_u-1_fh-0_t-{millis}_r-1");
    let ext=format!("internal_src:dim|wss_push_room_id:{room}|wss_push_did:{did}|first_req_ms:{millis}|fetch_time:{millis}|seq:1|wss_info:0-{millis}-0-0|wrds_v:0");
    u.query_pairs_mut().extend_pairs([
        ("app_name", "douyin_web"),
        ("version_code", "180800"),
        ("webcast_sdk_version", "1.0.14-beta.0"),
        ("update_version_code", "1.0.14-beta.0"),
        ("compress", "gzip"),
        ("device_platform", "web"),
        ("cookie_enabled", "true"),
        ("screen_width", "1536"),
        ("screen_height", "864"),
        ("browser_language", "zh-CN"),
        ("browser_platform", "Win32"),
        ("browser_name", "Mozilla"),
        (
            "browser_version",
            USER_AGENT.strip_prefix("Mozilla/").unwrap_or(USER_AGENT),
        ),
        ("browser_online", "true"),
        ("tz_name", "Asia/Shanghai"),
        ("cursor", &cursor),
        ("internal_ext", &ext),
        ("host", "https://live.douyin.com"),
        ("aid", "6383"),
        ("live_id", "1"),
        ("did_rule", "3"),
        ("endpoint", "live_pc"),
        ("support_wrds", "1"),
        ("user_unique_id", did),
        ("im_path", "/webcast/im/fetch/"),
        ("identity", "audience"),
        ("need_persist_msg_count", "15"),
        ("insert_task_id", ""),
        ("live_reason", ""),
        ("room_id", room),
        ("heartbeatDuration", "0"),
    ]);
    let sign = signature::sign(&u);
    u.query_pairs_mut().append_pair("signature", &sign);
    Ok(u)
}
pub trait Duplex: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Duplex for T {}
pub type Socket = WebSocketStream<MaybeTlsStream<Box<dyn Duplex>>>;
pub async fn connect(
    route: Arc<Route>,
    visitor: &Visitor,
    room: &str,
    cancel: &CancellationToken,
) -> Result<Socket, Error> {
    let slot = route
        .admit(true, cancel)
        .await
        .map_err(|_| Error::Cancelled)?;
    let url = websocket_url(room, &visitor.did, crate::proxy::epoch_ms())?;
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|_| Error::Invalid("消息连接参数无效"))?;
    request
        .headers_mut()
        .insert("User-Agent", USER_AGENT.parse().unwrap());
    request.headers_mut().insert(
        "Cookie",
        visitor
            .cookie
            .parse()
            .map_err(|_| Error::Invalid("游客凭据格式无效"))?,
    );
    let host = url.host_str().unwrap().to_owned();
    let operation = async {
        let stream: Box<dyn Duplex> = if let Some(proxy) = &route.proxy {
            if proxy.protocol == "socks5h" {
                let stream = if proxy.username.is_empty() {
                    tokio_socks::tcp::Socks5Stream::connect(
                        proxy.endpoint.as_str(),
                        (host.as_str(), 443),
                    )
                    .await
                } else {
                    tokio_socks::tcp::Socks5Stream::connect_with_password(
                        proxy.endpoint.as_str(),
                        (host.as_str(), 443),
                        &proxy.username,
                        &proxy.password,
                    )
                    .await
                }
                .map_err(|_| Error::Network)?;
                Box::new(stream)
            } else {
                let mut stream = tokio::net::TcpStream::connect(&proxy.endpoint)
                    .await
                    .map_err(|_| Error::Network)?;
                let auth = proxy
                    .basic_auth()
                    .map(|s| format!("Proxy-Authorization: {s}\r\n"))
                    .unwrap_or_default();
                stream
                    .write_all(
                        format!("CONNECT {host}:443 HTTP/1.1\r\nHost: {host}:443\r\n{auth}\r\n")
                            .as_bytes(),
                    )
                    .await
                    .map_err(|_| Error::Network)?;
                let mut header = Vec::new();
                let mut byte = [0];
                while !header.ends_with(b"\r\n\r\n") {
                    if header.len() >= 16 * 1024 {
                        return Err(Error::Invalid("代理响应头超过限制"));
                    }
                    stream
                        .read_exact(&mut byte)
                        .await
                        .map_err(|_| Error::Network)?;
                    header.push(byte[0]);
                }
                let status = std::str::from_utf8(&header)
                    .ok()
                    .and_then(|s| s.lines().next())
                    .and_then(|s| s.split_whitespace().nth(1))
                    .and_then(|s| s.parse::<u16>().ok())
                    .ok_or(Error::Invalid("代理 CONNECT 响应无效"))?;
                if status != 200 {
                    return Err(Error::Status(status));
                }
                Box::new(stream)
            }
        } else {
            Box::new(
                tokio::net::TcpStream::connect((host.as_str(), 443))
                    .await
                    .map_err(|_| Error::Network)?,
            )
        };
        let config = WebSocketConfig::default()
            .max_message_size(Some(4 * 1024 * 1024))
            .max_frame_size(Some(4 * 1024 * 1024))
            .max_write_buffer_size(1024 * 1024);
        let (socket, _) =
            tokio_tungstenite::client_async_tls_with_config(request, stream, Some(config), None)
                .await
                .map_err(|e| match e {
                    tokio_tungstenite::tungstenite::Error::Http(r) => {
                        Error::Status(r.status().as_u16())
                    }
                    _ => Error::Network,
                })?;
        Ok(socket)
    };
    let result = tokio::select! {biased;_=cancel.cancelled()=>Err(Error::Cancelled),v=tokio::time::timeout(Duration::from_secs(20),operation)=>v.unwrap_or(Err(Error::Network))};
    drop(slot);
    result
}
pub enum Received {
    Frame(Vec<u8>),
    Heartbeat,
}
pub async fn receive(
    socket: &mut Socket,
    cancel: &CancellationToken,
) -> Result<Option<Vec<u8>>, Error> {
    tokio::select! {biased;_=cancel.cancelled()=>Err(Error::Cancelled),v=tokio::time::timeout(Duration::from_secs(60),socket.next())=>match v{Ok(Some(Ok(Message::Binary(bytes))))=>Ok(Some(bytes.to_vec())),Ok(Some(Ok(Message::Ping(p))))=>{socket.send(Message::Pong(p)).await.map_err(|_|Error::Closed)?;Ok(None)},Ok(Some(Ok(Message::Pong(_)|Message::Text(_))))=>Ok(None),Ok(Some(Ok(Message::Close(_))))|Ok(None)=>Err(Error::Closed),Ok(Some(Err(_)))=>Err(Error::Network),Err(_)=>Err(Error::Invalid("消息连接读取超时")),_=>Ok(None)}}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_never_invents_live_state() {
        assert!(parse_room(&json!({"data":{"data":[]}})).is_err());
        assert!(matches!(
            parse_room(&json!({"status_code":1})),
            Err(Error::Unavailable)
        ));
        let info=parse_room(&json!({"data":{"data":[{"status":2,"id_str":"222","owner":{"id_str":"111"},"user_count":42}]}})).unwrap();
        assert_eq!(info.room_id, "222");
        assert_eq!(info.anchor_id, "111");
        assert_eq!(info.viewer_count, Some(42));
        assert_eq!(info.started_at, None);
    }
    #[test]
    fn url_did_and_signature_canonical_share_actual_values() {
        let u = websocket_url("222", "333", 1800000000000).unwrap();
        let c = signature::canonical(&u);
        assert!(c.contains("room_id=222"));
        assert!(c.contains("user_unique_id=333"));
        assert!(!c.contains("7319483754668557238"));
        assert!(u
            .query_pairs()
            .any(|(k, v)| k == "signature" && v.len() == 16));
    }
}
