//! Explicitly enabled community replay lookup. Credentials never leave its fixed API origin.
use crate::{
    runtime::Runtime,
    storage::{self, Result},
};
use serde_json::{json, Value};
use std::time::Duration;
use url::Url;

const REPLAY_API: &str = "https://autoxingtu.api.ali.trih.top/api/v1/admin/live-sessions/";
const VIDEO_API: &str = "https://www.xingtu.cn/gw/api/ggeneric/get_video_info";
const JSON_LIMIT: usize = 1024 * 1024;
const PLAYLIST_LIMIT: usize = 4 * 1024 * 1024;
const MEDIA_LIMIT: usize = 16 * 1024 * 1024;

fn key_valid(s: &str) -> bool {
    !s.is_empty() && s.len() <= 8192 && !s.chars().any(char::is_control)
}

// Only developer builds consult local development credentials. The file itself
// and its contents are neither compiled into the program nor sent to the UI.
#[cfg(debug_assertions)]
fn development_key() -> Option<String> {
    if let Some(key) = std::env::var("INTERNAL_API_TOKEN")
        .ok()
        .filter(|s| key_valid(s.trim()))
    {
        return Some(key.trim().to_owned());
    }
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for directory in [
        manifest.parent()?.parent()?,
        manifest.parent()?.parent()?.parent()?,
    ] {
        let file = directory.join(".env");
        if std::fs::metadata(&file)
            .ok()
            .is_none_or(|m| m.len() > 64 * 1024)
        {
            continue;
        }
        let Some(text) = std::fs::read_to_string(file).ok() else {
            continue;
        };
        for line in text.trim_start_matches('\u{feff}').lines() {
            let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            if name.trim() != "INTERNAL_API_TOKEN" {
                continue;
            }
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .or_else(|| value.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
                .unwrap_or(value);
            if key_valid(value) {
                return Some(value.to_owned());
            }
        }
    }
    None
}
#[cfg(not(debug_assertions))]
fn development_key() -> Option<String> {
    None
}

fn public_settings(config: &Value) -> Value {
    json!({"enabled":config["community_enabled"] == true,
        "key_configured":config["community_api_key"].as_str().is_some_and(key_valid) || development_key().is_some()})
}

/// Backend-only request snapshot. Deliberately no Debug/Serialize implementation.
pub struct CommunityReplayLookup {
    enabled: bool,
    key: String,
    session_id: String,
    cancel: tokio_util::sync::CancellationToken,
}
impl CommunityReplayLookup {
    pub async fn fetch(self) -> Result<Value> {
        self.fetch_mode(false).await
    }

    /// Playback starts from the validated source address. The complete HLS
    /// wall-clock map is fetched separately after the player's first frame.
    pub async fn fetch_playback(self) -> Result<Value> {
        self.fetch_mode(true).await
    }

    async fn fetch_mode(self, defer_timing: bool) -> Result<Value> {
        if !self.enabled {
            return Ok(json!({"enabled":false,"available":false,"replay":null}));
        }
        let work = async {
            let _ = rustls::crypto::ring::default_provider().install_default();
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(20))
                .build()
                .map_err(|_| "无法创建回放连接")?;
            let response = client
                .get(format!("{REPLAY_API}{}/replay-url", self.session_id))
                .bearer_auth(&self.key)
                .header(reqwest::header::ACCEPT, "application/json")
                .send()
                .await
                .map_err(|_| "社区回放请求失败，请检查网络连接")?;
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                return Ok(json!({"enabled":true,"available":false,"replay":null}));
            }
            let body = read_success(response, JSON_LIMIT, "社区回放").await?;
            let payload: Value =
                serde_json::from_slice(&body).map_err(|_| "社区回放接口未返回有效 JSON")?;
            let replay = match replay_reference(&payload)? {
                None => None,
                Some(reference) => resolve(&media_client()?, reference, defer_timing).await?,
            };
            // A malformed upstream must never smuggle our API credential back
            // through a media URL or metadata field exposed to the WebView.
            if let Some(replay) = &replay {
                let public = serde_json::to_string(replay).map_err(|_| "回放信息格式无效")?;
                if public.contains(&self.key) {
                    return Err("回放信息包含不允许的凭据内容".into());
                }
            }
            Ok(json!({"enabled":true,"available":replay.is_some(),"replay":replay}))
        };
        tokio::select! { biased;
            _ = self.cancel.cancelled() => Err("回放请求已取消".into()),
            result = tokio::time::timeout(Duration::from_secs(45), work) =>
                result.map_err(|_| "获取回放超时，请稍后重试".to_owned())?,
        }
    }

    /// Reads only anonymous HLS timing metadata, never resolving the community
    /// API a second time and never forwarding its credential to a media host.
    pub async fn fetch_timing(self, raw_url: String) -> Result<Value> {
        if !self.enabled {
            return Err("社区生态未启用".into());
        }
        let url = validated_media_url(&raw_url)?;
        if !url.path().to_ascii_lowercase().ends_with(".m3u8") {
            return Err("此回放无需读取 HLS 时间映射".into());
        }
        if raw_url.contains(&self.key) {
            return Err("回放信息包含不允许的凭据内容".into());
        }
        let work = async {
            let response = timing_response(&media_client()?, url).await?;
            if serde_json::to_string(&response).map_err(|_| "回放信息格式无效")?.contains(&self.key) {
                return Err("回放信息包含不允许的凭据内容".into());
            }
            Ok(response)
        };
        tokio::select! { biased;
            _ = self.cancel.cancelled() => Err("回放请求已取消".into()),
            result = tokio::time::timeout(Duration::from_secs(45), work) =>
                result.map_err(|_| "读取回放时间映射超时".to_owned())?,
        }
    }
}

impl Runtime {
    /// Narrow desktop media transport. It never inherits proxy settings or
    /// provider credentials, and only retrieves allowlisted HTTPS CDN objects.
    pub async fn replay_media_fetch(
        &self,
        raw_url: &str,
        range_start: Option<u64>,
        range_end: Option<u64>,
    ) -> Result<Vec<u8>> {
        self.ensure_open()?;
        fetch_media(&self.shared, raw_url, range_start, range_end).await
    }

    pub fn community_replay_snapshot(&self, session_id: &str) -> Result<CommunityReplayLookup> {
        self.ensure_open()?;
        replay_snapshot(&self.shared, session_id)
    }

    pub(crate) fn community_request(&self, method: &str, params: Value) -> Result<Value> {
        match method {
            "community_settings" => Ok(public_settings(
                &self.shared.state.lock().unwrap().config.value,
            )),
            "community_configure" => {
                let enabled = params["enabled"].as_bool().ok_or("社区生态开关无效")?;
                let mut state = self.shared.state.lock().unwrap();
                let mut candidate = state.config.value.clone();
                if params["clear_key"] == true {
                    candidate["community_api_key"] = json!("");
                } else if let Some(key) = params.get("api_key") {
                    let key = key.as_str().ok_or("社区生态 KEY 格式无效")?.trim();
                    if !key.is_empty() {
                        if !key_valid(key) {
                            return Err("社区生态 KEY 格式无效".into());
                        }
                        candidate["community_api_key"] = json!(key);
                    }
                }
                if enabled
                    && !candidate["community_api_key"]
                        .as_str()
                        .is_some_and(key_valid)
                    && params["clear_key"] != true
                {
                    if let Some(key) = development_key() {
                        candidate["community_api_key"] = json!(key);
                    }
                }
                if enabled
                    && !candidate["community_api_key"]
                        .as_str()
                        .is_some_and(key_valid)
                {
                    return Err("启用社区生态前请输入 KEY".into());
                }
                candidate["community_enabled"] = json!(enabled);
                state.config.replace(candidate)?;
                Ok(public_settings(&state.config.value))
            }
            "session_replay" => self.rt.block_on(
                self.community_replay_snapshot(
                    params["session_id"].as_str().ok_or("缺少直播场次 ID")?,
                )?
                .fetch_playback(),
            ),
            "session_replay_timing" => self.rt.block_on(
                self.community_replay_snapshot(
                    params["session_id"].as_str().ok_or("缺少直播场次 ID")?,
                )?
                .fetch_timing(params["play_url"].as_str().ok_or("缺少回放地址")?.to_owned()),
            ),
            _ => Err("未知社区生态操作".into()),
        }
    }
}

fn media_client() -> Result<reqwest::Client> {
    static CLIENT: std::sync::OnceLock<Result<reqwest::Client>> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
            reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .connect_timeout(Duration::from_secs(8))
                .timeout(Duration::from_secs(20))
                .build()
                .map_err(|_| "无法创建录播媒体连接".into())
        })
        .clone()
}

fn media_range(start: Option<u64>, end: Option<u64>) -> Result<Option<String>> {
    if start.is_none() && end.is_none() {
        return Ok(None);
    }
    let start = start.unwrap_or(0);
    match end {
        Some(end) if end <= start => Err("录播媒体范围无效".into()),
        Some(end) if end - start > MEDIA_LIMIT as u64 => Err("录播媒体范围超过 16 MiB 限制".into()),
        Some(end) => Ok(Some(format!("bytes={start}-{}", end - 1))),
        None => Ok(Some(format!("bytes={start}-"))),
    }
}

fn append_media_chunk(bytes: &mut Vec<u8>, chunk: &[u8]) -> Result<()> {
    if bytes.len().saturating_add(chunk.len()) > MEDIA_LIMIT {
        return Err("录播媒体响应超过 16 MiB 限制".into());
    }
    bytes.extend_from_slice(chunk);
    Ok(())
}

async fn read_success(
    mut response: reqwest::Response,
    limit: usize,
    source: &str,
) -> Result<Vec<u8>> {
    let status = response.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            401 | 403 => format!(
                "{source}无访问权限（HTTP {}），请检查 KEY 或视频授权",
                status.as_u16()
            ),
            429 => format!("{source}请求过于频繁，请稍后重试"),
            300..=399 => format!("{source}返回重定向，已停止请求"),
            _ => format!("{source}请求失败（HTTP {}）", status.as_u16()),
        });
    }
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err(format!("{source}响应超过大小限制"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| format!("读取{source}响应失败"))?
    {
        if bytes.len() + chunk.len() > limit {
            return Err(format!("{source}响应超过大小限制"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

enum ReplayReference {
    Xingtu(String),
    Direct(Url),
}
fn replay_reference(value: &Value) -> Result<Option<ReplayReference>> {
    fn extract(value: &Value, depth: usize) -> Result<Option<&str>> {
        if depth > 3 {
            return Err("社区回放响应层级无效".into());
        }
        match value {
            Value::Null => Ok(None),
            Value::String(s) => Ok(Some(s)),
            Value::Object(map) => {
                for key in ["replay_url", "url", "data"] {
                    if let Some(value) = map.get(key) {
                        return extract(value, depth + 1);
                    }
                }
                Err("社区回放响应缺少链接".into())
            }
            _ => Err("社区回放响应格式无效".into()),
        }
    }
    let Some(raw) = extract(value, 0)?.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if raw.len() > 16384 {
        return Err("回放地址超过大小限制".into());
    }
    let url = Url::parse(raw).map_err(|_| "社区回放地址无效")?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err("社区回放仅支持授权 HTTPS 地址".into());
    }
    if matches!(url.host_str(), Some("www.xingtu.cn" | "xingtu.cn"))
        && url.path().trim_end_matches('/') == "/player"
    {
        let codes = url
            .query_pairs()
            .filter(|(key, _)| key == "code")
            .map(|(_, value)| value.into_owned())
            .collect::<Vec<_>>();
        if codes.is_empty() || codes.iter().all(|s| s.is_empty()) {
            return Ok(None);
        }
        if codes.len() != 1 || !video_id_valid(&codes[0]) {
            return Err("星图视频 code 无效".into());
        }
        return Ok(Some(ReplayReference::Xingtu(codes[0].clone())));
    }
    let url = validated_media_url(raw)?;
    let path = url.path().to_ascii_lowercase();
    if !path.ends_with(".m3u8") && !path.ends_with(".mp4") {
        return Err("回放地址不是支持的视频文件".into());
    }
    Ok(Some(ReplayReference::Direct(url)))
}
fn video_id_valid(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Shared allowlist for the resolver and any desktop media bridge.
pub fn validated_media_url(raw: &str) -> Result<Url> {
    let url = Url::parse(raw).map_err(|_| "媒体地址无效")?;
    let allowed = url.host_str().is_some_and(|host| {
        [
            "xingtu.cn",
            "bytefcdn.com",
            "douyinvod.com",
            "douyincdn.com",
            "bytevcloud.com",
            "bytetos.com",
        ]
        .iter()
        .any(|suffix| {
            host == *suffix || host.strip_suffix(suffix).is_some_and(|p| p.ends_with('.'))
        })
    });
    if raw.len() > 16384
        || url.scheme() != "https"
        || !allowed
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("媒体地址不在授权 HTTPS 来源范围内".into());
    }
    Ok(url)
}

async fn resolve(client: &reqwest::Client, reference: ReplayReference, defer_timing: bool) -> Result<Option<Value>> {
    let (replay, url) = match reference {
        ReplayReference::Xingtu(video_id) => {
            let page_url = format!("https://www.xingtu.cn/player?code={video_id}");
            let mut api_url = Url::parse(VIDEO_API).map_err(|_| "星图接口地址无效")?;
            api_url.query_pairs_mut().append_pair("video_id", &video_id);
            let response = client
                .get(api_url)
                .header(reqwest::header::REFERER, &page_url)
                .header("agw-js-conv", "str")
                .header(reqwest::header::ACCEPT, "application/json")
                .send()
                .await
                .map_err(|_| "获取星图回放失败，请检查网络连接")?;
            let body = read_success(response, JSON_LIMIT, "星图视频").await?;
            let info: Value = serde_json::from_slice(&body).map_err(|_| "星图视频信息格式无效")?;
            let Some((url, duration_ms, width, height)) = video_info(&info)? else {
                return Ok(None);
            };
            (
                json!({"source_kind":"xingtu","video_id":video_id,"page_url":page_url,"duration_ms":duration_ms,"width":width,"height":height}),
                url,
            )
        }
        ReplayReference::Direct(url) => (
            json!({"source_kind":"direct","video_id":null,"page_url":null,"duration_ms":null,"width":null,"height":null}),
            url,
        ),
    };
    Ok(Some(finish_resolution(client, replay, url, defer_timing).await?))
}

async fn finish_resolution(client: &reqwest::Client, mut replay: Value, url: Url, defer_timing: bool) -> Result<Value> {
    replay["resolved_at"] = json!(storage::now());
    replay["time_mapping"] = json!([]);
    replay["time_mapping_pending"] = json!(false);
    if url.path().to_ascii_lowercase().ends_with(".m3u8") {
        // Keep a master playlist as the playback source, preserving external
        // audio renditions. Its first variant is inspected only for timing.
        replay["play_url"] = json!(url.as_str());
        replay["media_kind"] = json!("hls");
        if defer_timing {
            replay["time_mapping_pending"] = json!(true);
            return Ok(replay);
        }
        let timing = timing_response(client, url).await?;
        replay["playlist_duration_ms"] = timing["playlist_duration_ms"].clone();
        replay["time_mapping"] = timing["time_mapping"].clone();
        if replay["duration_ms"].is_null() {
            replay["duration_ms"] = timing["playlist_duration_ms"].clone();
        }
    } else {
        replay["play_url"] = json!(url.as_str());
        replay["media_kind"] = json!("mp4");
    }
    Ok(replay)
}

async fn timing_response(client: &reqwest::Client, url: Url) -> Result<Value> {
    let playback_url = url.clone();
    let (_, playlist) = load_playlist(client, url).await?;
    let (duration_ms, mapping) = playlist_timing(&playlist)?;
    Ok(json!({"play_url":playback_url.as_str(),"playlist_duration_ms":duration_ms,
        "time_mapping":mapping,"time_mapping_pending":false}))
}

fn video_info(info: &Value) -> Result<Option<(Url, u64, Option<u32>, Option<u32>)>> {
    let status = info["base_resp"]["status_code"]
        .as_i64()
        .ok_or("星图视频响应缺少状态")?;
    if status != 0 {
        return Err(format!("星图视频暂不可播放（状态 {status}）"));
    }
    let Some(raw) = info["play_url"].as_str().filter(|s| !s.trim().is_empty()) else {
        return Ok(None);
    };
    let duration = info["duration"]
        .as_f64()
        .filter(|n| n.is_finite() && *n > 0.0 && *n <= 31.0 * 86400.0)
        .ok_or("星图视频时长无效")?;
    let dimension = |field: &str| {
        info[field]
            .as_u64()
            .filter(|n| *n > 0 && *n <= 32768)
            .map(|n| n as u32)
    };
    Ok(Some((
        validated_media_url(raw)?,
        (duration * 1000.0).round() as u64,
        dimension("width"),
        dimension("height"),
    )))
}

async fn load_playlist(client: &reqwest::Client, mut url: Url) -> Result<(Url, String)> {
    for _ in 0..3 {
        let response = client
            .get(url.clone())
            .header(reqwest::header::REFERER, "https://www.xingtu.cn/")
            .send()
            .await
            .map_err(|_| "读取回放清单失败，请重新打开场次重试")?;
        let bytes = read_success(response, PLAYLIST_LIMIT, "回放清单").await?;
        let playlist = String::from_utf8(bytes).map_err(|_| "回放清单编码无效")?;
        if !playlist
            .trim_start_matches('\u{feff}')
            .starts_with("#EXTM3U")
        {
            return Err("回放地址未返回 HLS 清单".into());
        }
        if !playlist
            .lines()
            .any(|line| line.starts_with("#EXT-X-STREAM-INF:"))
        {
            return Ok((url, playlist));
        }
        let mut variant = false;
        let target = playlist
            .lines()
            .find_map(|line| {
                if line.starts_with("#EXT-X-STREAM-INF:") {
                    variant = true;
                    return None;
                }
                let line = line.trim();
                (variant && !line.is_empty() && !line.starts_with('#')).then_some(line)
            })
            .ok_or("HLS 主清单缺少播放流")?;
        url = validated_media_url(url.join(target).map_err(|_| "HLS 子清单地址无效")?.as_str())?;
    }
    Err("HLS 清单层级超过限制".into())
}

fn playlist_timing(playlist: &str) -> Result<(u64, Value)> {
    let mut elapsed: f64 = 0.0;
    let mut duration = None;
    let mut wall_clock: Option<f64> = None;
    let mut direct_pdt = false;
    let mut discontinuity = 0_u32;
    let mut mapping = Vec::new();
    let mut count = 0;
    for line in playlist.lines().map(str::trim) {
        if let Some(value) = line.strip_prefix("#EXTINF:") {
            duration = Some(
                value
                    .split(',')
                    .next()
                    .unwrap_or("")
                    .parse::<f64>()
                    .ok()
                    .filter(|n| n.is_finite() && *n > 0.0 && *n < 86400.0)
                    .ok_or("HLS 分片时长无效")?,
            );
        } else if let Some(value) = line.strip_prefix("#EXT-X-PROGRAM-DATE-TIME:") {
            wall_clock = Some(
                chrono::DateTime::parse_from_rfc3339(value)
                    .map_err(|_| "HLS 时间标记无效")?
                    .timestamp_millis() as f64,
            );
            direct_pdt = true;
        } else if line == "#EXT-X-DISCONTINUITY" {
            discontinuity += 1;
            // RFC 8216 6.3.3: infer undated segments from the preceding PDT.
            // A codec/PTS boundary alone does not establish a wall-clock gap;
            // a later explicit PDT still replaces the inferred clock below.
        } else if !line.is_empty() && !line.starts_with('#') {
            let seconds = duration.take().ok_or("HLS 分片缺少时长")?;
            count += 1;
            if count > 100_000 || elapsed + seconds > 31.0 * 86400.0 {
                return Err("HLS 清单超出回放范围".into());
            }
            if let Some(wall) = wall_clock {
                mapping.push(json!({"offset_seconds":elapsed,"duration":seconds,
                    "program_date_time":chrono::DateTime::from_timestamp_millis(wall.round() as i64).ok_or("HLS 时间标记超出范围")?.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    "discontinuity":discontinuity,
                    "time_source":if direct_pdt {"pdt"} else {"extrapolated"}}));
                wall_clock = Some(wall + seconds * 1000.0);
            }
            direct_pdt = false;
            elapsed += seconds;
        }
    }
    if count == 0 {
        return Err("HLS 清单没有可播放分片".into());
    }
    Ok(((elapsed * 1000.0).round() as u64, json!(mapping)))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn delayed_playlist() -> (reqwest::Client, Url, std::sync::Arc<std::sync::atomic::AtomicUsize>, tokio::task::JoinHandle<()>) {
        use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let _ = rustls::crypto::ring::default_provider().install_default();
        let certificate = rcgen::generate_simple_self_signed(vec!["v26-star.xingtu.cn".into()]).unwrap();
        let trust = reqwest::Certificate::from_der(certificate.cert.der()).unwrap();
        let key = rustls::pki_types::PrivatePkcs8KeyDer::from(certificate.signing_key.serialize_der());
        let config = rustls::ServerConfig::builder().with_no_client_auth()
            .with_single_cert(vec![certificate.cert.der().clone()], key.into()).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = reqwest::Client::builder().no_proxy().add_root_certificate(trust)
            .resolve("v26-star.xingtu.cn", listener.local_addr().unwrap())
            .redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(3)).build().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let observed = requests.clone();
        let task = tokio::spawn(async move {
            let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                let mut socket = acceptor.accept(socket).await.unwrap();
                let mut bytes = Vec::new();
                while !bytes.ends_with(b"\r\n\r\n") {
                    let mut next = [0];socket.read_exact(&mut next).await.unwrap();bytes.push(next[0]);
                    assert!(bytes.len() <= 8192);
                }
                let header = String::from_utf8(bytes).unwrap();
                assert!(!header.to_ascii_lowercase().contains("authorization:"), "media timing must be anonymous");
                observed.fetch_add(1, Ordering::SeqCst);
                let body = if header.starts_with("GET /master.m3u8?") {
                    "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"sound\",URI=\"audio.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=1000000,AUDIO=\"sound\"\nmedia.m3u8\n"
                } else {
                    tokio::time::sleep(Duration::from_millis(600)).await;
                    "#EXTM3U\n#EXT-X-PROGRAM-DATE-TIME:2026-11-02T08:00:00Z\n#EXTINF:2,\na.ts\n#EXT-X-DISCONTINUITY\n#EXT-X-PROGRAM-DATE-TIME:2026-11-02T08:01:00Z\n#EXTINF:2,\nb.ts\n#EXT-X-ENDLIST\n"
                };
                let response=format!("HTTP/1.1 200 OK\r\nContent-Type: application/vnd.apple.mpegurl\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                // A timed-out baseline caller may already have disconnected.
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
        (client, validated_media_url("https://v26-star.xingtu.cn/master.m3u8?signature=synthetic%2Fvalue").unwrap(), requests, task)
    }

    #[tokio::test]
    async fn playback_address_does_not_wait_for_a_slow_hls_timing_playlist() {
        use std::sync::atomic::Ordering;
        let (client,url,requests,server) = delayed_playlist().await;
        let expected = url.to_string();
        let result = tokio::time::timeout(Duration::from_millis(100),
            finish_resolution(&client,json!({"duration_ms":4000,"width":1280,"height":720}),url,true)).await;
        server.abort();
        let replay = result.expect("可播放地址不应等待慢速整场 HLS 时间映射").unwrap();
        assert_eq!(requests.load(Ordering::SeqCst),0,"playback startup must not request the timing playlist");
        assert_eq!(replay["play_url"],expected);
        assert_eq!(replay["media_kind"],"hls");
        assert_eq!(replay["time_mapping_pending"],true);
        assert_eq!(replay["time_mapping"],json!([]));
        assert_eq!(replay["duration_ms"],4000);
    }

    #[tokio::test]
    async fn full_download_resolution_and_deferred_timing_preserve_master_audio_and_pdt_gaps() {
        use std::sync::atomic::Ordering;
        let (client,url,requests,server) = delayed_playlist().await;
        let expected = url.to_string();
        let before = std::time::Instant::now();
        let full = finish_resolution(&client,json!({"duration_ms":null}),url.clone(),false).await.unwrap();
        assert!(before.elapsed() >= Duration::from_millis(550),"the full download path must await complete timing");
        assert_eq!(full["play_url"],expected,"retain master source with its external audio rendition");
        assert_eq!(full["time_mapping_pending"],false);
        assert_eq!(full["duration_ms"],4000);
        assert_eq!(full["time_mapping"].as_array().unwrap().len(),2);
        assert_eq!(full["time_mapping"][1]["program_date_time"],"2026-11-02T08:01:00.000Z");
        let timing = timing_response(&client,url).await.unwrap();
        assert_eq!(timing["play_url"],expected);
        assert_eq!(timing["time_mapping"],full["time_mapping"]);
        assert_eq!(timing["playlist_duration_ms"],4000);
        assert_eq!(requests.load(Ordering::SeqCst),4);
        server.abort();
    }

    #[tokio::test]
    async fn timing_lookup_checks_feature_origin_cancellation_and_key_echo_before_network() {
        let lookup = |enabled,cancel| CommunityReplayLookup {enabled,key:"synthetic-private-key".into(),session_id:"101".into(),cancel};
        let token = tokio_util::sync::CancellationToken::new();
        assert!(lookup(false,token.clone()).fetch_timing("https://v26-star.xingtu.cn/a.m3u8".into()).await.unwrap_err().contains("未启用"));
        for url in ["http://v26-star.xingtu.cn/a.m3u8","https://127.0.0.1/a.m3u8","https://v26-star.xingtu.cn.evil.test/a.m3u8","https://v26-star.xingtu.cn/a.mp4","https://v26-star.xingtu.cn/a.m3u8?token=synthetic-private-key"] {
            assert!(lookup(true,token.clone()).fetch_timing(url.into()).await.is_err());
        }
        token.cancel();
        assert!(lookup(true,token).fetch_timing("https://v26-star.xingtu.cn/a.m3u8".into()).await.unwrap_err().contains("已取消"));
    }

    #[test]
    fn media_ranges_convert_exclusive_end_and_reject_empty_or_oversized_ranges() {
        assert_eq!(media_range(None, None).unwrap(), None);
        assert_eq!(
            media_range(Some(5), Some(10)).unwrap().as_deref(),
            Some("bytes=5-9")
        );
        assert_eq!(
            media_range(None, Some(1024)).unwrap().as_deref(),
            Some("bytes=0-1023")
        );
        assert_eq!(
            media_range(Some(10), None).unwrap().as_deref(),
            Some("bytes=10-")
        );
        for (start, end) in [
            (Some(1), Some(1)),
            (None, Some(0)),
            (Some(5), Some(4)),
            (None, Some(MEDIA_LIMIT as u64 + 1)),
        ] {
            assert!(media_range(start, end).is_err());
        }
    }

    #[test]
    fn streamed_media_cap_rejects_overflow_before_copying_the_chunk() {
        let mut body = vec![0; MEDIA_LIMIT - 1];
        append_media_chunk(&mut body, &[1]).unwrap();
        assert_eq!(body.len(), MEDIA_LIMIT);
        assert!(append_media_chunk(&mut body, &[2]).is_err());
        assert_eq!(body.len(), MEDIA_LIMIT);
        assert_eq!(body[MEDIA_LIMIT - 1], 1);
    }

    #[test]
    fn native_media_transport_checks_feature_and_origin_before_network() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = Runtime::open_with_transport_and_secrets(
            directory.path().into(),
            std::sync::Arc::new(crate::transport::LiveTransport),
            std::sync::Arc::new(FixtureProtector),
        )
        .unwrap();
        assert!(runtime
            .rt
            .block_on(runtime.replay_media_fetch("https://v26-star.xingtu.cn/test.ts", None, None))
            .unwrap_err()
            .contains("未启用"));
        runtime
            .request(
                "community_configure",
                json!({"enabled":true,"api_key":"synthetic-community-key"}),
            )
            .unwrap();
        for url in [
            "https://evil.example/test.ts",
            "http://v26-star.xingtu.cn/test.ts",
            "https://v26-star.xingtu.cn.evil.example/test.ts",
            "https://127.0.0.1/test.ts",
        ] {
            assert!(runtime
                .rt
                .block_on(runtime.replay_media_fetch(url, None, None))
                .unwrap_err()
                .contains("地址"));
        }
        runtime.shutdown().unwrap();
    }

    #[test]
    fn only_replay_fields_are_interpreted_and_empty_pages_are_unavailable() {
        for value in [
            Value::Null,
            json!(""),
            json!({"replay_url":null}),
            json!({"data":{"url":"https://www.xingtu.cn/player?code="}}),
            json!("https://www.xingtu.cn/player"),
        ] {
            assert!(replay_reference(&value).unwrap().is_none());
        }
        assert!(matches!(
            replay_reference(&json!({"data":"https://www.xingtu.cn/player?code=v123"})).unwrap(),
            Some(ReplayReference::Xingtu(_))
        ));
        assert!(
            replay_reference(&json!({"message":"https://www.xingtu.cn/player?code=v123"})).is_err()
        );
        for raw in [
            "http://www.xingtu.cn/player?code=v1",
            "https://www.xingtu.cn/player?code=v1&code=v2",
            "https://www.xingtu.cn/player?code=../etc",
            "https://evil.test/video.m3u8",
            "https://xingtu.cn.evil.test/video.mp4",
            "https://key@v26-star.xingtu.cn/test.m3u8",
        ] {
            assert!(replay_reference(&json!(raw)).is_err());
        }
    }

    #[test]
    fn media_urls_keep_signatures_and_reject_unapproved_origins() {
        let url = "https://v26-star.xingtu.cn/fixture.m3u8?signature=synthetic%2Fvalue&expires=123";
        assert_eq!(validated_media_url(url).unwrap().as_str(), url);
        assert!(validated_media_url("https://lf26-record-tos.bytefcdn.com/segment.ts").is_ok());
        assert!(validated_media_url("https://lr11.douyincdn.com/segment.ts").is_ok());
        for url in [
            "http://v26-star.xingtu.cn/a",
            "https://127.0.0.1/a",
            "https://v26-star.xingtu.cn:8443/a",
            "https://evilxingtu.cn/a",
            "https://v26-star.xingtu.cn/a#fragment",
            "http://lr11.douyincdn.com/a.ts",
            "https://douyincdn.com.evil.test/a.ts",
            "https://evildouyincdn.com/a.ts",
            "https://lr11.douyincdn.com:8443/a.ts",
            "https://user@lr11.douyincdn.com/a.ts",
            "https://lr11.douyincdn.com@evil.test/a.ts",
            "https://lr11.douyincdn.com/a.ts#fragment",
        ] {
            assert!(validated_media_url(url).is_err());
        }
    }

    #[test]
    fn video_fixture_uses_seconds_and_validates_business_status() {
        let mut fixture = json!({"base_resp":{"status_code":0},"play_url":"https://v26-star.xingtu.cn/fixture.m3u8?synthetic=1","duration":5420.203,"width":1920,"height":1080});
        let (_, duration, width, height) = video_info(&fixture).unwrap().unwrap();
        assert_eq!(
            (duration, width, height),
            (5_420_203, Some(1920), Some(1080))
        );
        fixture["duration"] = json!(-1);
        assert!(video_info(&fixture).is_err());
        fixture["play_url"] = json!("");
        assert!(video_info(&fixture).unwrap().is_none());
        fixture["base_resp"]["status_code"] = json!(7);
        assert!(video_info(&fixture).is_err());
    }

    #[test]
    fn playlist_time_mapping_extrapolates_after_discontinuity_without_a_new_pdt() {
        // The real six-hour recording has only its first PDT. A codec/PTS
        // discontinuity must not discard the remaining wall-clock mapping.
        let playlist = "#EXTM3U\n#EXT-X-PROGRAM-DATE-TIME:2026-10-21T13:40:39+08:00\n#EXTINF:23.299,\na.ts\n#EXT-X-DISCONTINUITY\n#EXTINF:21600,\nb.ts\n#EXT-X-ENDLIST\n";
        let (duration, mapping) = playlist_timing(playlist).unwrap();
        assert_eq!(duration, 21_623_299);
        assert_eq!(mapping.as_array().unwrap().len(), 2);
        assert_eq!(mapping[0]["time_source"], "pdt");
        assert_eq!(mapping[1]["offset_seconds"], 23.299);
        assert_eq!(mapping[1]["discontinuity"], 1);
        assert_eq!(mapping[1]["time_source"], "extrapolated");
        assert_eq!(mapping[1]["program_date_time"], "2026-10-21T05:41:02.299Z");
    }

    #[test]
    fn playlist_time_mapping_preserves_a_fresh_pdt_wall_clock_gap() {
        let playlist = "#EXTM3U\n#EXT-X-PROGRAM-DATE-TIME:2026-10-02T20:45:06+08:00\n#EXTINF:2.2,\na.ts\n#EXTINF:3.3,\nb.ts\n#EXT-X-DISCONTINUITY\n#EXTINF:4,\nc.ts\n#EXT-X-PROGRAM-DATE-TIME:2026-10-02T20:50:00+08:00\n#EXTINF:5,\nd.ts\n#EXT-X-ENDLIST\n";
        let (duration, mapping) = playlist_timing(playlist).unwrap();
        assert_eq!(duration, 14_500);
        assert_eq!(mapping.as_array().unwrap().len(), 4);
        assert_eq!(mapping[0]["time_source"], "pdt");
        assert_eq!(mapping[1]["offset_seconds"], 2.2);
        assert_eq!(mapping[1]["program_date_time"], "2026-10-02T12:45:08.200Z");
        assert_eq!(mapping[2]["offset_seconds"], 5.5);
        assert_eq!(mapping[2]["discontinuity"], 1);
        assert_eq!(mapping[2]["program_date_time"], "2026-10-02T12:45:11.500Z");
        assert_eq!(mapping[2]["time_source"], "extrapolated");
        assert_eq!(mapping[3]["offset_seconds"], 9.5);
        assert_eq!(mapping[3]["program_date_time"], "2026-10-02T12:50:00.000Z");
        assert_eq!(mapping[3]["time_source"], "pdt");
        assert!(playlist_timing("#EXTM3U\n#EXTINF:NaN,\na.ts").is_err());
    }

    #[test]
    fn playlist_time_mapping_does_not_invent_epochs_before_the_first_pdt() {
        let (duration, mapping) = playlist_timing(
            "#EXTM3U\n#EXTINF:2,\na.ts\n#EXT-X-DISCONTINUITY\n#EXTINF:3,\nb.ts\n",
        ).unwrap();
        assert_eq!(duration, 5_000);
        assert_eq!(mapping, json!([]));
        let (_, mapping) = playlist_timing(
            "#EXTM3U\n#EXTINF:2,\na.ts\n#EXT-X-DISCONTINUITY\n#EXT-X-PROGRAM-DATE-TIME:2026-10-21T13:40:39+08:00\n#EXTINF:3,\nb.ts\n",
        ).unwrap();
        assert_eq!(mapping.as_array().unwrap().len(), 1);
        assert_eq!(mapping[0]["offset_seconds"], 2.0);
        assert_eq!(mapping[0]["time_source"], "pdt");
    }

    struct FixtureProtector;
    impl storage::SecretProtector for FixtureProtector {
        fn protect(&self, bytes: &[u8]) -> Result<Vec<u8>> {
            Ok(bytes.iter().map(|b| b ^ 0xa5).collect())
        }
        fn unprotect(&self, bytes: &[u8]) -> Result<Vec<u8>> {
            self.protect(bytes)
        }
    }

    #[test]
    fn encrypted_community_settings_never_return_key_and_off_never_connects() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = Runtime::open_with_transport_and_secrets(
            directory.path().into(),
            std::sync::Arc::new(crate::transport::LiveTransport),
            std::sync::Arc::new(FixtureProtector),
        )
        .unwrap();
        assert_eq!(
            runtime.request("community_settings", json!({})).unwrap()["enabled"],
            false
        );
        assert_eq!(
            runtime
                .request("session_replay", json!({"session_id":"123"}))
                .unwrap(),
            json!({"enabled":false,"available":false,"replay":null})
        );
        let public = runtime
            .request(
                "community_configure",
                json!({"enabled":true,"api_key":"synthetic-community-key"}),
            )
            .unwrap();
        assert_eq!(public, json!({"enabled":true,"key_configured":true}));
        let disk = std::fs::read_to_string(directory.path().join("config.json")).unwrap();
        assert!(!disk.contains("synthetic-community-key"));
        assert!(disk.contains("community_api_key_encrypted"));
        let loaded = storage::Settings::load_with_protector(
            directory.path().into(),
            std::sync::Arc::new(FixtureProtector),
        )
        .unwrap();
        assert_eq!(loaded.value["community_api_key"], "synthetic-community-key");
        runtime
            .request("community_configure", json!({"enabled":false}))
            .unwrap();
        assert_eq!(
            runtime
                .request("session_replay", json!({"session_id":"123"}))
                .unwrap()["available"],
            false
        );
        assert!(runtime
            .request(
                "community_configure",
                json!({"enabled":true,"clear_key":true})
            )
            .is_err());
        runtime.shutdown().unwrap();
    }
}

pub(crate) async fn fetch_media(
    shared: &std::sync::Arc<crate::runtime::Shared>,
    raw_url: &str,
    range_start: Option<u64>,
    range_end: Option<u64>,
) -> Result<Vec<u8>> {
    let enabled = || {
        shared
            .state
            .lock()
            .map(|state| state.config.value["community_enabled"] == true)
            .map_err(|_| "社区生态状态不可用".to_owned())
    };
    if !enabled()? {
        return Err("社区生态未启用".into());
    }
    let url = validated_media_url(raw_url)?;
    let range = media_range(range_start, range_end)?;
    let work = async {
        let mut request = media_client()?.get(url);
        if let Some(range) = &range {
            request = request.header(reqwest::header::RANGE, range);
        }
        let mut response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                "录播媒体请求超时"
            } else {
                "录播媒体连接失败"
            }
        })?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("录播媒体请求失败（HTTP {}）", status.as_u16()));
        }
        if range.is_some() && status != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(format!(
                "录播媒体未返回范围响应（HTTP {}）",
                status.as_u16()
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MEDIA_LIMIT as u64)
        {
            return Err("录播媒体响应超过 16 MiB 限制".into());
        }
        let mut bytes = Vec::new();
        loop {
            if !enabled()? {
                return Err("社区生态已关闭".into());
            }
            let Some(chunk) = response.chunk().await.map_err(|error| {
                if error.is_timeout() {
                    "录播媒体读取超时"
                } else {
                    "录播媒体读取失败"
                }
            })?
            else {
                break;
            };
            append_media_chunk(&mut bytes, &chunk)?;
        }
        if !enabled()? {
            return Err("社区生态已关闭".into());
        }
        Ok(bytes)
    };
    tokio::select! { biased;
        _ = shared.cancel.cancelled() => Err("录播媒体请求已取消".into()),
        result = work => result,
    }
}

pub(crate) fn replay_snapshot(
    shared: &std::sync::Arc<crate::runtime::Shared>,
    session_id: &str,
) -> Result<CommunityReplayLookup> {
    if !storage::id(session_id) {
        return Err("直播场次 ID 无效".into());
    }
    let state = shared.state.lock().unwrap();
    let enabled = state.config.value["community_enabled"] == true;
    let key = state.config.value["community_api_key"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    if enabled && !key_valid(&key) {
        return Err("请先在设置中保存社区生态 KEY".into());
    }
    Ok(CommunityReplayLookup {
        enabled,
        key,
        session_id: session_id.into(),
        cancel: shared.cancel.clone(),
    })
}
