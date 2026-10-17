//! Server-only authenticated encryption. Keys are separate from the data volume.
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
};
use h_live_runtime::storage::{Result, SecretProtector};
use std::{fs, io::Write, path::Path};
use zeroize::Zeroizing;

const MAGIC: &[u8] = b"HLIVESEC1";
const AAD: &[u8] = b"h-live-server/proxy-configuration/v1";

pub struct ServerProtector(Zeroizing<[u8; 32]>);
impl ServerProtector {
    pub fn from_key(key: [u8; 32]) -> Self {
        Self(Zeroizing::new(key))
    }
    pub fn load(path: &Path) -> Result<Self> {
        Ok(Self::from_key(read_key(path)?))
    }
}
impl SecretProtector for ServerProtector {
    fn protect(&self, input: &[u8]) -> Result<Vec<u8>> {
        if input.len() > 8 * 1024 * 1024 {
            return Err("代理配置超过加密大小限制".into());
        }
        let cipher = XChaCha20Poly1305::new_from_slice(self.0.as_ref()).map_err(|_| "密钥无效")?;
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let encrypted = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: input,
                    aad: AAD,
                },
            )
            .map_err(|_| "代理凭据保护失败")?;
        Ok([MAGIC, nonce.as_slice(), &encrypted].concat())
    }
    fn unprotect(&self, input: &[u8]) -> Result<Vec<u8>> {
        if !input.starts_with(MAGIC)
            || input.len() < MAGIC.len() + 24 + 16
            || input.len() > 8 * 1024 * 1024 + 128
        {
            return Err("不是有效的服务端加密配置；Windows DPAPI 配置须重新导入".into());
        }
        let cipher = XChaCha20Poly1305::new_from_slice(self.0.as_ref()).map_err(|_| "密钥无效")?;
        let nonce = XNonce::from_slice(&input[MAGIC.len()..MAGIC.len() + 24]);
        cipher
            .decrypt(
                nonce,
                Payload {
                    msg: &input[MAGIC.len() + 24..],
                    aad: AAD,
                },
            )
            .map_err(|_| "代理配置密钥不匹配或内容已损坏；原文件未修改".into())
    }
}
pub fn decode_key(raw: &str) -> Result<[u8; 32]> {
    let decoded = Zeroizing::new(
        STANDARD
            .decode(raw.trim())
            .or_else(|_| URL_SAFE_NO_PAD.decode(raw.trim()))
            .map_err(|_| "密钥文件格式无效")?,
    );
    decoded
        .as_slice()
        .try_into()
        .map_err(|_| "密钥必须为随机 32 字节的 Base64 编码".into())
}
pub fn read_key(path: &Path) -> Result<[u8; 32]> {
    let info = fs::symlink_metadata(path).map_err(|_| "无法读取密钥文件，请先初始化密钥")?;
    if !info.is_file() || info.len() > 4096 {
        return Err("密钥必须为普通文件".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if info.permissions().mode() & 0o077 != 0 {
            return Err("密钥文件权限必须为 0600 或 0400".into());
        }
    }
    let raw = Zeroizing::new(fs::read_to_string(path).map_err(|_| "密钥文件不可读")?);
    decode_key(&raw)
}
pub fn init(directory: &Path) -> Result<usize> {
    if directory.exists()
        && !fs::symlink_metadata(directory)
            .map_err(|_| "密钥目录不可读")?
            .is_dir()
    {
        return Err("密钥目录不能为链接或普通文件".into());
    }
    fs::create_dir_all(directory).map_err(|_| "无法创建密钥目录")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .map_err(|_| "无法限制密钥目录权限")?;
    }
    let mut created = 0;
    for name in ["master.key", "admin-token"] {
        let path = directory.join(name);
        if path.exists() {
            read_key(&path)?;
            continue;
        }
        let key = Zeroizing::new(rand::random::<[u8; 32]>());
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&path)
            .map_err(|_| "无法创建密钥文件；已有文件不会覆盖")?;
        let encoded = Zeroizing::new(URL_SAFE_NO_PAD.encode(key.as_ref()));
        writeln!(file, "{}", encoded.as_str())
            .and_then(|_| file.sync_all())
            .map_err(|_| "密钥写入失败")?;
        created += 1;
    }
    #[cfg(unix)]
    fs::File::open(directory)
        .and_then(|f| f.sync_all())
        .map_err(|_| "密钥目录同步失败")?;
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ciphertext_is_randomized_authenticated_and_bound_to_key() {
        let p = ServerProtector::from_key([7; 32]);
        let a = p.protect(b"synthetic-proxy-credential").unwrap();
        let b = p.protect(b"synthetic-proxy-credential").unwrap();
        assert_ne!(a, b);
        assert!(!a.windows(26).any(|w| w == b"synthetic-proxy-credential"));
        assert_eq!(p.unprotect(&a).unwrap(), b"synthetic-proxy-credential");
        assert!(ServerProtector::from_key([8; 32]).unprotect(&a).is_err());
        let mut damaged = a;
        *damaged.last_mut().unwrap() ^= 1;
        assert!(p.unprotect(&damaged).is_err());
        assert!(p.unprotect(b"legacy-dpapi-or-plaintext").is_err());
    }
    #[test]
    fn initialization_does_not_replace_existing_secrets() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(init(d.path()).unwrap(), 2);
        let before = fs::read(d.path().join("master.key")).unwrap();
        assert_eq!(init(d.path()).unwrap(), 0);
        assert_eq!(fs::read(d.path().join("master.key")).unwrap(), before);
        assert_ne!(
            read_key(&d.path().join("master.key")).unwrap(),
            read_key(&d.path().join("admin-token")).unwrap()
        );
    }
}
