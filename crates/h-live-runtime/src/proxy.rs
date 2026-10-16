//! Validated imports and stable per-exit admission/lifetime state.
use crate::storage::Result;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    net::{IpAddr, SocketAddr},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

pub fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let [a, b, c, _] = v.octets();
            !v.is_private()
                && !v.is_loopback()
                && !v.is_link_local()
                && !v.is_multicast()
                && !v.is_unspecified()
                && a != 0
                && a < 224
                && a != 127
                && !(a == 100 && (64..128).contains(&b))
                && !(a == 192 && b == 0 && (c == 0 || c == 2))
                && !(a == 198 && (b == 18 || b == 19 || b == 51 && c == 100))
                && !(a == 203 && b == 0 && c == 113)
                && !(a == 192 && b == 88 && c == 99)
        }
        IpAddr::V6(v) => {
            let s = v.segments();
            s[0] & 0xe000 == 0x2000
                && !(s[0] == 0x2001 && (s[1] == 0xdb8 || s[1] <= 0x01ff))
                && !(s[0] == 0x2002)
                && !(s[0] == 0x3fff)
        }
    }
}
pub fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))[..24].into()
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct ProxyRecord {
    pub id: String,
    pub endpoint: String,
    pub protocol: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub city: String,
    #[serde(default)]
    pub country_code: String,
}
impl ProxyRecord {
    pub fn from_value(v: &Value, protocol: &str) -> Result<Self> {
        if let Some(s) = v.as_str() {
            return Self::from_line(s, protocol);
        }
        let endpoint = if let Some(s) = v["endpoint"].as_str() {
            s.to_owned()
        } else {
            let ip = v["ip"].as_str().ok_or("代理缺少 IP")?;
            let port = v["port"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| v["port"].to_string());
            format!(
                "{}:{port}",
                if ip.contains(':') {
                    format!("[{ip}]")
                } else {
                    ip.into()
                }
            )
        };
        let address: SocketAddr = endpoint.parse().map_err(|_| "代理地址或端口无效")?;
        if address.port() == 0 || !public_ip(address.ip()) {
            return Err("仅接受有效公网 IP 与端口".into());
        }
        let protocol = v["protocol"]
            .as_str()
            .or_else(|| {
                v["protocols"].as_array().and_then(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .find(|s| matches!(*s, "http" | "https" | "socks5" | "socks5h"))
                })
            })
            .unwrap_or(protocol)
            .to_lowercase();
        let protocol = match protocol.as_str() {
            "http" | "https" => "http",
            "socks5" | "socks5h" => "socks5h",
            _ => return Err("仅支持 HTTP CONNECT 或 SOCKS5 代理".into()),
        }
        .to_owned();
        let user = v["username"].as_str().unwrap_or("").to_owned();
        let pass = v["password"].as_str().unwrap_or("").to_owned();
        if user.is_empty() != pass.is_empty()
            || [&user, &pass]
                .iter()
                .any(|s| s.len() > 255 || s.chars().any(char::is_control))
            || protocol == "http" && (!user.is_ascii() || !pass.is_ascii() || user.contains(':'))
        {
            return Err("代理认证信息格式无效".into());
        }
        let expires_at = v["expires_at"]
            .as_str()
            .filter(|s| !s.is_empty() && *s != "-" && *s != "永久")
            .map(|s| {
                if chrono::DateTime::parse_from_rfc3339(s).is_ok() {
                    Ok(s.to_owned())
                } else {
                    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                        .map(|n| {
                            n.and_local_timezone(qingkan_core::archive::shanghai())
                                .single()
                                .unwrap()
                                .to_rfc3339()
                        })
                        .map_err(|_| "代理到期时间无效".to_string())
                }
            })
            .transpose()?;
        let short = |key: &str| {
            v[key]
                .as_str()
                .unwrap_or("")
                .chars()
                .filter(|c| !c.is_control())
                .take(100)
                .collect::<String>()
        };
        Ok(Self {
            id: digest(&address.to_string()),
            endpoint: address.to_string(),
            protocol,
            username: user,
            password: pass,
            expires_at,
            city: short("city"),
            country_code: if v["country_code"].is_string() {
                short("country_code")
            } else {
                short("country")
            },
        })
    }
    pub fn from_line(text: &str, protocol: &str) -> Result<Self> {
        let text = text.trim().trim_matches('|').trim();
        if text.contains("://") {
            let u = url::Url::parse(text).map_err(|_| "代理链接无效")?;
            if u.path() != "" && u.path() != "/" || u.query().is_some() || u.fragment().is_some() {
                return Err("代理链接不可包含路径参数".into());
            }
            let host = u.host_str().ok_or("代理缺少 IP")?.trim_matches(['[', ']']);
            let port = u.port().ok_or("代理缺少端口")?;
            let decode = |s: &str| {
                percent_encoding::percent_decode_str(s)
                    .decode_utf8()
                    .map(|s| s.into_owned())
                    .map_err(|_| "代理认证编码无效")
            };
            return Self::from_value(
                &json!({"ip":host,"port":port,"protocol":u.scheme(),"username":decode(u.username())?,"password":decode(u.password().unwrap_or(""))?}),
                protocol,
            );
        }
        if let Ok(a) = text.parse::<SocketAddr>() {
            return Self::from_value(&json!({"endpoint":a.to_string()}), protocol);
        }
        let parts: Vec<_> = text
            .split(|c: char| c == '\t' || c == '|' || c == ',' || c.is_whitespace())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if parts.len() == 1 {
            let p: Vec<_> = text.splitn(4, ':').collect();
            if p.len() == 4 {
                return Self::from_value(
                    &json!({"ip":p[0],"port":p[1],"username":p[2],"password":p[3]}),
                    protocol,
                );
            }
        }
        let ip_index = parts
            .iter()
            .position(|s| s.parse::<IpAddr>().is_ok())
            .ok_or("代理行缺少 IP")?;
        let tail = &parts[ip_index + 1..];
        let pi = tail
            .iter()
            .position(|s| s.parse::<u16>().is_ok_and(|p| p > 0))
            .ok_or("代理行缺少端口")?;
        let credentials: Vec<_> = tail[pi + 1..]
            .iter()
            .copied()
            .filter(|s| !s.is_empty())
            .collect();
        let clean = |s: &str| {
            s.trim_start_matches("账号：")
                .trim_start_matches("帐号：")
                .trim_start_matches("密码：")
                .to_owned()
        };
        let (user, pass) = if credentials.len() >= 2
            && !credentials[0].starts_with("20")
            && credentials[0] != "正常"
        {
            (clean(credentials[0]), clean(credentials[1]))
        } else {
            (String::new(), String::new())
        };
        Self::from_value(
            &json!({"ip":parts[ip_index],"port":tail[pi],"username":user,"password":pass}),
            protocol,
        )
    }
    pub fn expired(&self) -> bool {
        self.expires_at
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .is_some_and(|t| t <= chrono::Utc::now())
    }
    pub fn public(&self, status: &str) -> Value {
        json!({"id":self.id,"endpoint":self.endpoint,"protocol":self.protocol,"expires_at":self.expires_at,"city":self.city,"country_code":self.country_code,"auth_present":!self.username.is_empty(),"status":status})
    }
    pub fn url(&self) -> Result<url::Url> {
        let mut u = url::Url::parse(&format!("{}://{}", self.protocol, self.endpoint))
            .map_err(|_| "代理地址无效")?;
        if !self.username.is_empty() {
            u.set_username(&self.username).map_err(|_| "代理账号无效")?;
            u.set_password(Some(&self.password))
                .map_err(|_| "代理密码无效")?;
        }
        Ok(u)
    }
    pub fn basic_auth(&self) -> Option<String> {
        (!self.username.is_empty()).then(|| {
            format!(
                "Basic {}",
                STANDARD.encode(format!("{}:{}", self.username, self.password))
            )
        })
    }
}
fn header_key(s: &str) -> Option<&'static str> {
    match s.trim().to_lowercase().as_str() {
        "ip" | "ip地址" | "ip信息" | "地址" => Some("ip"),
        "port" | "端口" => Some("port"),
        "username" | "user" | "账号" | "帐号" | "用户名" => Some("username"),
        "password" | "pass" | "密码" => Some("password"),
        "protocol" | "协议" => Some("protocol"),
        "city" | "城市" | "所属国家/地区" => Some("city"),
        "country_code" | "国家" => Some("country_code"),
        "expires_at" | "到期时间" | "过期时间" => Some("expires_at"),
        _ => None,
    }
}
fn text_rows(text: &str) -> Result<Vec<Value>> {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .collect::<Vec<_>>();
    if lines.iter().any(|s| s.parse::<IpAddr>().is_ok())
        && lines
            .iter()
            .any(|s| s.starts_with("账号：") || s.starts_with("帐号："))
    {
        let mut out = vec![];
        let mut row = json!({});
        for line in lines {
            if line.parse::<IpAddr>().is_ok() {
                if row.get("ip").is_some() {
                    out.push(row)
                }
                row = json!({"ip":line});
            } else if row.get("ip").is_some() {
                if line.parse::<u16>().is_ok_and(|p| p > 0) && row.get("port").is_none() {
                    row["port"] = json!(line)
                } else if let Some(value) = line
                    .strip_prefix("账号：")
                    .or_else(|| line.strip_prefix("帐号："))
                {
                    row["username"] = json!(value.trim())
                } else if let Some(value) = line.strip_prefix("密码：") {
                    row["password"] = json!(value.trim())
                } else if chrono::NaiveDateTime::parse_from_str(line, "%Y-%m-%d %H:%M:%S").is_ok() {
                    row["expires_at"] = json!(line)
                } else if row.get("port").is_none() && row.get("city").is_none() {
                    row["city"] = json!(line)
                }
            }
        }
        if row.get("ip").is_some() {
            out.push(row)
        }
        return Ok(out);
    }
    let first = lines.first().copied().unwrap_or("");
    let delimiter = if first.contains('\t') {
        Some(b'\t')
    } else if first.contains(',') {
        Some(b',')
    } else if first.contains('|') {
        Some(b'|')
    } else {
        None
    };
    if let Some(delimiter) = delimiter {
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .flexible(true)
            .trim(csv::Trim::All)
            .delimiter(delimiter)
            .from_reader(text.as_bytes());
        let mut headers: Option<Vec<Option<&str>>> = None;
        let mut out = vec![];
        for record in reader.records() {
            let record = record.map_err(|_| "代理表格格式无效")?;
            let keys = record.iter().map(header_key).collect::<Vec<_>>();
            if headers.is_none() && keys.contains(&Some("ip")) && keys.contains(&Some("port")) {
                headers = Some(keys);
                continue;
            }
            if record.iter().all(|s| {
                s.is_empty() || s.chars().all(|c| c == '-' || c == ':' || c.is_whitespace())
            }) {
                continue;
            }
            if let Some(headers) = &headers {
                let mut v = json!({});
                for (key, value) in headers.iter().zip(record.iter()) {
                    if let Some(key) = key {
                        v[*key] = json!(value)
                    }
                }
                if !record
                    .iter()
                    .any(|v| v.contains("账号：") || v.contains("密码："))
                {
                    out.push(v);
                    continue;
                }
            }
            out.push(json!(record.iter().collect::<Vec<_>>().join("\t")));
        }
        return Ok(out);
    }
    Ok(lines.into_iter().map(|s| json!(s)).collect())
}
pub fn parse(
    text: &str,
    protocol: &str,
    existing: &[ProxyRecord],
) -> Result<(Vec<ProxyRecord>, Value)> {
    if text.len() > 800_000 {
        return Err("导入内容超过大小限制".into());
    }
    let text = text.trim().trim_start_matches('\u{feff}');
    let rows: Vec<Value> =
        if text.starts_with('{') || text.starts_with('[') && !text.starts_with("[2") {
            let v: Value = serde_json::from_str(text).map_err(|_| "代理 JSON 格式无效")?;
            if let Some(a) = v.as_array() {
                a.clone()
            } else if let Some(a) = v["data"].as_array() {
                a.clone()
            } else if let Some(a) = v["data"]["proxies"].as_array() {
                a.clone()
            } else {
                return Err("代理 JSON 缺少数组".into());
            }
        } else {
            text_rows(text)?
        };
    if rows.len() > 5000 {
        return Err("最多导入 5000 条代理".into());
    }
    let old: HashSet<_> = existing.iter().map(|r| r.id.clone()).collect();
    let mut seen = HashSet::new();
    let mut records = vec![];
    let (mut invalid, mut duplicate, mut added, mut updated) = (0, 0, 0, 0);
    for row in rows {
        match ProxyRecord::from_value(&row, protocol) {
            Ok(r) => {
                if !seen.insert(r.id.clone()) {
                    duplicate += 1;
                    continue;
                }
                if old.contains(&r.id) {
                    updated += 1
                } else {
                    added += 1
                }
                records.push(r)
            }
            Err(_) => invalid += 1,
        }
    }
    let preview = json!({"added":added,"updated":updated,"duplicates":duplicate,"invalid":invalid,"valid":records.len(),"total":records.len(),"items":records.iter().take(100).map(|r|r.public("unchecked")).collect::<Vec<_>>()});
    Ok((records, preview))
}
pub struct Route {
    pub id: String,
    pub proxy: Option<ProxyRecord>,
    pub client: reqwest::Client,
    pub retired: AtomicBool,
    pub load: AtomicUsize,
    pub until: AtomicU64,
    connects: Arc<Semaphore>,
    requests: Arc<Semaphore>,
    next: Mutex<Instant>,
}
pub fn client(record: Option<&ProxyRecord>) -> Result<reqwest::Client> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut b = reqwest::Client::builder()
        .no_proxy()
        .https_only(true)
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(18))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(crate::net::USER_AGENT)
        .pool_max_idle_per_host(4);
    if let Some(r) = record {
        b = b.proxy(reqwest::Proxy::all(r.url()?.as_str()).map_err(|_| "代理配置失败")?)
    }
    b.build().map_err(|_| "网络客户端初始化失败".into())
}
impl Route {
    pub fn new(proxy: Option<ProxyRecord>) -> Result<Arc<Self>> {
        let http = client(proxy.as_ref())?;
        Ok(Arc::new(Self {
            id: proxy
                .as_ref()
                .map(|p| p.id.clone())
                .unwrap_or("direct".into()),
            proxy,
            client: http,
            retired: AtomicBool::new(false),
            load: AtomicUsize::new(0),
            until: AtomicU64::new(0),
            connects: Arc::new(Semaphore::new(2)),
            requests: Arc::new(Semaphore::new(2)),
            next: Mutex::new(Instant::now()),
        }))
    }
    pub fn ready(&self) -> bool {
        !self.retired.load(Ordering::Acquire)
            && self.until.load(Ordering::Acquire) <= epoch_ms()
            && !self.proxy.as_ref().is_some_and(ProxyRecord::expired)
    }
    pub fn cooldown(&self, seconds: u64) {
        self.until
            .fetch_max(epoch_ms() + seconds * 1000, Ordering::AcqRel);
    }
    pub async fn admit(
        &self,
        connect: bool,
        cancel: &CancellationToken,
    ) -> Result<OwnedSemaphorePermit> {
        let sem = if connect {
            &self.connects
        } else {
            &self.requests
        };
        loop {
            if self.retired.load(Ordering::Acquire)
                || self.proxy.as_ref().is_some_and(ProxyRecord::expired)
            {
                return Err("出口已退役，请使用当前出口".into());
            }
            let permit = tokio::select! {biased;_=cancel.cancelled()=>return Err("任务已停止".into()),p=sem.clone().acquire_owned()=>p.map_err(|_|"出口已关闭")?};
            let delay = {
                let mut next = self.next.lock().unwrap();
                let now = Instant::now();
                let cool = Duration::from_millis(
                    self.until
                        .load(Ordering::Acquire)
                        .saturating_sub(epoch_ms()),
                );
                let at = (*next).max(now + cool);
                if at <= now {
                    *next = now + Duration::from_millis(500);
                    None
                } else {
                    Some(at - now)
                }
            };
            if let Some(delay) = delay {
                drop(permit);
                tokio::select! {biased;_=cancel.cancelled()=>return Err("任务已停止".into()),_=tokio::time::sleep(delay)=>{}}
            } else {
                return Ok(permit);
            }
        }
    }
    pub fn snapshot(&self) -> Value {
        let cool = self
            .until
            .load(Ordering::Acquire)
            .saturating_sub(epoch_ms()) as f64
            / 1000.;
        json!({"route_id":self.id,"kind":if self.proxy.is_some(){"proxy"}else{"direct"},"endpoint":self.proxy.as_ref().map(|p|p.endpoint.clone()).unwrap_or_default(),"state":if self.retired.load(Ordering::Acquire){"draining"}else if cool>0.{"cooldown"}else{"ready"},"assigned_tasks":self.load.load(Ordering::Acquire),"connecting":2-self.connects.available_permits(),"request_inflight":2-self.requests.available_permits(),"cooldown_seconds":cool})
    }
}
pub fn epoch_ms() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}
pub struct Lease(pub Arc<Route>);
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.load.fetch_sub(1, Ordering::AcqRel);
    }
}
pub struct Pool {
    pub routes: BTreeMap<String, Arc<Route>>,
    pub failed: HashSet<String>,
    pub checked: HashSet<String>,
    pub checking: usize,
    pub refresh: Value,
    pub last_error: String,
    pub enabled: bool,
    pub include_direct: bool,
    pub source: String,
    pub candidates: BTreeMap<String, ProxyRecord>,
    pub verified: BTreeMap<String, u64>,
    pub retired: Vec<Arc<Route>>,
}
impl Pool {
    pub fn new() -> Result<Self> {
        let direct = Route::new(None)?;
        Ok(Self {
            routes: BTreeMap::from([("direct".into(), direct)]),
            failed: HashSet::new(),
            checked: HashSet::new(),
            checking: 0,
            refresh: json!({"active":false,"state":"idle","target":0}),
            last_error: String::new(),
            enabled: false,
            include_direct: true,
            source: "public".into(),
            candidates: BTreeMap::new(),
            verified: BTreeMap::new(),
            retired: vec![],
        })
    }
    pub fn lease(&self) -> Option<Lease> {
        let r = self
            .routes
            .values()
            .filter(|r| {
                r.ready()
                    && (r.proxy.is_none() && (!self.enabled || self.include_direct)
                        || r.proxy.is_some() && self.enabled)
            })
            .min_by_key(|r| r.load.load(Ordering::Acquire))?
            .clone();
        r.load.fetch_add(1, Ordering::AcqRel);
        Some(Lease(r))
    }
    pub fn snapshot(&self) -> Value {
        json!({"enabled":self.enabled,"source":self.source,"shared":true,"available":self.routes.values().filter(|r|r.proxy.is_some()&&r.ready()).count(),"assigned":self.routes.values().chain(self.retired.iter()).map(|r|r.load.load(Ordering::Acquire)).sum::<usize>(),"failed":self.failed.len(),"checking":self.checking,"total":self.checked.len(),"refresh":self.refresh,"last_error":self.last_error,"routes":self.routes.values().chain(self.retired.iter()).map(|r|r.snapshot()).collect::<Vec<_>>(),"leases":[],"max_checks":50,"ttl_seconds":300})
    }
}
pub async fn fetch_subscription(raw: &str, cancel: &CancellationToken) -> Result<String> {
    let u = url::Url::parse(raw).map_err(|_| "订阅 URL 无效")?;
    if u.scheme() != "https"
        || u.port_or_known_default() != Some(443)
        || !u.username().is_empty()
        || u.password().is_some()
        || u.fragment().is_some()
        || raw.len() > 4096
    {
        return Err("订阅仅支持公共 HTTPS 地址，不接受账户段或非标准端口".into());
    }
    let host = u.host_str().ok_or("订阅主机无效")?;
    let addresses = tokio::select! {biased;_=cancel.cancelled()=>return Err("订阅已取消".into()),v=tokio::time::timeout(Duration::from_secs(5),tokio::net::lookup_host((host,443)))=>v.map_err(|_|"订阅 DNS 超时")?.map_err(|_|"订阅 DNS 失败")?.collect::<Vec<_>>()};
    if addresses.is_empty() || addresses.iter().any(|a| !public_ip(a.ip())) {
        return Err("订阅主机不可解析到内网、回环或保留地址".into());
    }
    let http = reqwest::Client::builder()
        .no_proxy()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &addresses)
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| "订阅网络初始化失败")?;
    let response = tokio::select! {biased;_=cancel.cancelled()=>return Err("订阅已取消".into()),r=http.get(u).send()=>r.map_err(|_|"订阅下载失败")?};
    let bytes = crate::net::body(response, 800_000, cancel)
        .await
        .map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|_| "订阅必须是 UTF-8 文本".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reject_internal_and_redact_credentials() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "100.64.1.2",
            "169.254.169.254",
            "192.0.2.1",
            "::1",
            "::ffff:8.8.8.8",
            "2001:db8::1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        let (r, p) = parse("8.8.8.8:80:fixture:synthetic-secret", "http", &[]).unwrap();
        assert_eq!(r.len(), 1);
        assert!(!p.to_string().contains("synthetic-secret"));
        assert!(r[0].basic_auth().is_some());
    }
    #[test]
    fn json_provider_and_geo_imports() {
        for input in [
            r#"[{"ip":"8.8.8.8","port":80,"username":"fixture","password":"secret"}]"#,
            r#"{"data":[{"ip":"8.8.4.4","port":"1080","protocols":["socks5"],"country":"US"}]}"#,
            r#"{"data":{"proxies":["8.8.8.8:80"]}}"#,
        ] {
            assert_eq!(parse(input, "http", &[]).unwrap().0.len(), 1);
        }
    }
    #[test]
    fn csv_markdown_and_multiline_vendor_imports() {
        for text in ["ip,port,username,password,expires_at\n\"8.8.8.8\",8080,fixture,secret,2099-01-01 00:00:00", "|IP信息|所属国家/地区|端口|账号/密码|二维码|\n|---|---|---|---|---|\n|8.8.8.8|City|8080|账号：fixture|密码：secret|", "8.8.8.8\nCity\n8080\n账号：fixture\n密码：secret\n2026-01-01 00:00:00\n2099-01-01 00:00:00"]{
            let (records,preview)=parse(text,"http",&[]).unwrap();assert_eq!(records.len(),1,"{preview}");assert_eq!(records[0].username,"fixture");assert_eq!(records[0].password,"secret");assert!(!preview.to_string().contains("secret"));
        }
    }
    #[tokio::test]
    async fn cancellation_and_balanced_leases() {
        let mut p = Pool::new().unwrap();
        p.enabled = true;
        let r = Route::new(Some(ProxyRecord::from_line("8.8.8.8:80", "http").unwrap())).unwrap();
        p.routes.insert(r.id.clone(), r);
        let l = (0..50).map(|_| p.lease().unwrap()).collect::<Vec<_>>();
        assert!(p
            .routes
            .values()
            .all(|r| r.load.load(Ordering::Acquire) == 25));
        let token = CancellationToken::new();
        token.cancel();
        assert!(l[0].0.admit(true, &token).await.is_err());
        drop(l);
        assert!(p
            .routes
            .values()
            .all(|r| r.load.load(Ordering::Acquire) == 0));
    }
}
