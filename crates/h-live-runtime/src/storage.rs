//! Atomic legacy-compatible settings. Only proxy secrets are decrypted.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub type Result<T> = std::result::Result<T, String>;
pub fn now() -> String {
    chrono::Utc::now()
        .with_timezone(&qingkan_core::archive::shanghai())
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, false)
}
pub fn id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes().all(|b| b.is_ascii_digit())
        && s.bytes().any(|b| b != b'0')
}
pub fn room_id(input: &str) -> Result<String> {
    let s = input.trim();
    if id(s) {
        return Ok(s.into());
    }
    let u = url::Url::parse(s).map_err(|_| "直播间链接或 ID 无效")?;
    if u.scheme() != "https"
        || u.port().is_some()
        || !u.username().is_empty()
        || u.password().is_some()
    {
        return Err("直播间链接无效".into());
    }
    let path = u.path();
    let rid = match u.host_str() {
        Some("live.douyin.com") => path.trim_matches('/'),
        Some("www.douyin.com" | "douyin.com") => path
            .strip_prefix("/follow/live/")
            .unwrap_or("")
            .trim_end_matches('/'),
        _ => "",
    };
    if !id(rid) {
        return Err("直播间链接或 ID 无效".into());
    }
    Ok(rid.into())
}
pub fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or("保存路径无效")?;
    fs::create_dir_all(parent).map_err(|_| "无法创建数据目录")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|_| "无法创建临时文件")?;
    temp.write_all(bytes)
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|_| "写入或同步数据失败")?;
    temp.persist(path).map_err(|_| "保存失败；原文件已保留")?;
    Ok(())
}
#[cfg(windows)]
fn crypt(input: &[u8], decrypt: bool) -> Result<Vec<u8>> {
    use windows_sys::Win32::{Foundation::LocalFree, Security::Cryptography::*};
    if input.len() > 8 * 1024 * 1024 {
        return Err("凭据超出大小限制".into());
    }
    let src = CRYPT_INTEGER_BLOB {
        cbData: input.len() as u32,
        pbData: input.as_ptr() as *mut u8,
    };
    let mut out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let ok = unsafe {
        if decrypt {
            CryptUnprotectData(
                &src,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        } else {
            CryptProtectData(
                &src,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        }
    };
    if ok == 0 {
        return Err("代理凭据保护失败；请使用保存配置的 Windows 用户，未写入明文".into());
    }
    let result = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec() };
    unsafe {
        LocalFree(out.pbData as _);
    };
    Ok(result)
}
#[cfg(not(windows))]
fn crypt(_: &[u8], _: bool) -> Result<Vec<u8>> {
    Err("认证代理配置需要 Windows 用户凭据保护".into())
}
pub struct Settings {
    pub value: Value,
    pub directory: PathBuf,
}
impl Settings {
    pub fn load(directory: PathBuf) -> Result<Self> {
        fs::create_dir_all(&directory).map_err(|_| "无法创建数据目录")?;
        let path = directory.join("config.json");
        let mut value = if path.exists() {
            if fs::metadata(&path).map_err(|_| "无法读取配置")?.len() > 16 * 1024 * 1024 {
                return Err("配置超过大小限制".into());
            }
            let bytes = fs::read(&path).map_err(|_| "无法读取配置")?;
            serde_json::from_slice::<Value>(bytes.strip_prefix(&[239, 187, 191]).unwrap_or(&bytes))
                .map_err(|_| "配置无效；原文件已保留")?
        } else {
            json!({"anchors":{},"anchor_metadata":{},"proxy_pool":{"enabled":false,"source":"public","include_direct":true},"imported_proxies":[],"proxy_subscriptions":[]})
        };
        if !value.is_object()
            || !value["anchors"].is_object()
            || value["anchors"].as_object().unwrap().len() > 500
        {
            return Err("主播配置无效；原文件已保留".into());
        }
        for (rid, name) in value["anchors"].as_object().unwrap() {
            if !id(rid) || !name.is_string() || name.as_str().unwrap().chars().count() > 200 {
                return Err("主播配置无效".into());
            }
        }
        for key in ["imported_proxies", "proxy_subscriptions"] {
            let encrypted = format!("{key}_encrypted");
            if let Some(s) = value[&encrypted].as_str() {
                let raw = STANDARD.decode(s).map_err(|_| "代理配置编码无效")?;
                let clear = crypt(&raw, true)?;
                value[key] = serde_json::from_slice(&clear).map_err(|_| "代理配置格式无效")?;
            } else if !value[key].is_array() {
                value[key] = json!([])
            }
        }
        // Legacy login fields are opaque: never decrypt, expose or send them.
        Ok(Self { value, directory })
    }
    pub fn save(&self) -> Result<()> {
        self.persist(&self.value)
    }
    pub fn replace(&mut self, candidate: Value) -> Result<()> {
        self.persist(&candidate)?;
        self.value = candidate;
        Ok(())
    }
    fn persist(&self, value: &Value) -> Result<()> {
        let mut disk = value.clone();
        disk["schema_version"] = json!(2);
        for key in ["imported_proxies", "proxy_subscriptions"] {
            let list = disk
                .as_object_mut()
                .unwrap()
                .remove(key)
                .unwrap_or(json!([]));
            let encrypted = format!("{key}_encrypted");
            disk.as_object_mut().unwrap().remove(&encrypted);
            if list.as_array().is_some_and(|v| !v.is_empty()) {
                let encoded = serde_json::to_vec(&list).map_err(|_| "代理配置无效")?;
                disk[encrypted] = json!(STANDARD.encode(crypt(&encoded, false)?));
            }
        }
        atomic(
            &self.directory.join("config.json"),
            &serde_json::to_vec_pretty(&disk).map_err(|_| "配置无效")?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids_and_links_are_strict() {
        for s in [
            "123",
            "https://live.douyin.com/123",
            "https://www.douyin.com/follow/live/123?anchor_id=4",
        ] {
            assert_eq!(room_id(s).unwrap(), "123");
        }
        for s in [
            "0",
            "١٢٣",
            "https://evil.example/123",
            "https://live.douyin.com:8443/123",
            "../123",
        ] {
            assert!(room_id(s).is_err(), "{s}");
        }
    }
    #[test]
    fn first_install_empty_and_unknown_fields_preserved() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Settings::load(d.path().into()).unwrap();
        assert_eq!(s.value["anchors"], json!({}));
        s.value["opaque_old_field"] = json!("retain");
        s.save().unwrap();
        assert_eq!(
            Settings::load(d.path().into()).unwrap().value["opaque_old_field"],
            "retain"
        );
    }
    #[cfg(windows)]
    #[test]
    fn legacy_dpapi_roundtrip_and_no_plain_proxy_secrets() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Settings::load(d.path().into()).unwrap();
        s.value["imported_proxies"] = json!([{"password":"synthetic-secret","username":"fixture"}]);
        s.save().unwrap();
        let raw = fs::read_to_string(d.path().join("config.json")).unwrap();
        assert!(!raw.contains("synthetic-secret"));
        assert_eq!(
            Settings::load(d.path().into()).unwrap().value["imported_proxies"][0]["password"],
            "synthetic-secret"
        );
    }
}
