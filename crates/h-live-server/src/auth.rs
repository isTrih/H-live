//! The browser session belongs to H-live, never to the monitored platform.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;

struct Session {
    digest: [u8; 32],
    csrf: String,
    created: Instant,
}
pub struct Auth {
    admin: [u8; 32],
    pub origin: String,
    pub secure: bool,
    sessions: Mutex<VecDeque<Session>>,
    attempts: Mutex<VecDeque<Instant>>,
}
fn digest(v: &[u8]) -> [u8; 32] {
    Sha256::digest(v).into()
}
fn opaque() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}
impl Auth {
    pub fn new(key: [u8; 32], origin: &str) -> Result<Self, String> {
        let url = url::Url::parse(origin).map_err(|_| "公开访问地址无效")?;
        let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if !(url.scheme() == "https" || (url.scheme() == "http" && local))
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.host().is_none()
        {
            return Err("公开访问地址必须是 HTTPS Origin；仅本机可使用 HTTP".into());
        }
        Ok(Self {
            admin: digest(&key),
            origin: url.origin().ascii_serialization(),
            secure: url.scheme() == "https",
            sessions: Mutex::new(VecDeque::new()),
            attempts: Mutex::new(VecDeque::new()),
        })
    }
    pub fn bearer(&self, value: &str) -> bool {
        let Ok(key) = crate::secrets::decode_key(value) else {
            return false;
        };
        bool::from(self.admin.ct_eq(&digest(&key)))
    }
    pub fn login(&self, value: &str) -> Result<(String, String), u16> {
        let mut attempts = self.attempts.lock().unwrap();
        attempts.retain(|t| t.elapsed() < Duration::from_secs(60));
        if attempts.len() >= 10 {
            return Err(429);
        }
        attempts.push_back(Instant::now());
        if !self.bearer(value) {
            return Err(401);
        }
        drop(attempts);
        let token = opaque();
        let csrf = opaque();
        let mut sessions = self.sessions.lock().unwrap();
        sessions.retain(|s| s.created.elapsed() < Duration::from_secs(12 * 3600));
        if sessions.len() >= 16 {
            sessions.pop_front();
        }
        sessions.push_back(Session {
            digest: digest(token.as_bytes()),
            csrf: csrf.clone(),
            created: Instant::now(),
        });
        Ok((token, csrf))
    }
    pub fn session(&self, token: &str) -> Option<String> {
        if token.len() > 100 {
            return None;
        }
        let mut sessions = self.sessions.lock().unwrap();
        sessions.retain(|s| s.created.elapsed() < Duration::from_secs(12 * 3600));
        sessions
            .iter()
            .find(|s| bool::from(s.digest.ct_eq(&digest(token.as_bytes()))))
            .map(|s| s.csrf.clone())
    }
    pub fn logout(&self, token: &str) {
        self.sessions
            .lock()
            .unwrap()
            .retain(|s| !bool::from(s.digest.ct_eq(&digest(token.as_bytes()))));
    }
    pub fn cookie_name(&self) -> &str {
        if self.secure {
            "__Host-hlive-session"
        } else {
            "hlive-session"
        }
    }
    pub fn cookie(&self, token: &str, max_age: u64) -> String {
        format!(
            "{}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}{}",
            self.cookie_name(),
            if self.secure { "; Secure" } else { "" }
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn origin_and_session_boundaries() {
        assert!(Auth::new([1; 32], "http://example.com").is_err());
        assert!(Auth::new([1; 32], "https://example.com/path").is_err());
        let a = Auth::new([1; 32], "https://example.com").unwrap();
        assert!(a.login("bad").is_err());
        let (token, csrf) = a.login(&URL_SAFE_NO_PAD.encode([1; 32])).unwrap();
        assert_eq!(a.session(&token), Some(csrf));
        assert!(a.session("forged").is_none());
        a.logout(&token);
        assert!(a.session(&token).is_none());
        assert!(a.cookie("x", 1).contains("Secure"));
    }
    #[test]
    fn attempts_are_bounded() {
        let a = Auth::new([1; 32], "http://localhost:8080").unwrap();
        for _ in 0..10 {
            assert_eq!(a.login("bad"), Err(401));
        }
        assert_eq!(a.login("bad"), Err(429));
    }
}
