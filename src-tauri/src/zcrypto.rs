
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

pub const PREFIX: &str = "enc:v1:";

pub fn node_platform_for<'a>(os: &'a str) -> &'a str {
    match os {
        "windows" => "win32",
        "macos" => "darwin",
        other => other,
    }
}

fn node_os() -> &'static str {
    node_platform_for(std::env::consts::OS)
}

fn pick_username(username: Option<&str>, user: Option<&str>, logname: Option<&str>) -> String {
    username
        .or(user)
        .or(logname)
        .unwrap_or("unknown")
        .to_string()
}

#[cfg(not(windows))]
fn passwd_username() -> Option<String> {
    let out = std::process::Command::new("id")
        .arg("-un")
        .output()
        .ok()?;
    let n = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!n.is_empty()).then_some(n)
}

fn compose_fallback_secret(platform: &str, home: &str, username: &str) -> String {
    format!("zcode-credential-fallback:{}:{}:{}", platform, home, username)
}

pub fn default_secret(home: &Path) -> String {
    if let Ok(s) = std::env::var("ZCODE_CREDENTIAL_SECRET") {
        return s;
    }
    #[cfg(windows)]
    let primary = std::env::var("USERNAME").ok();
    #[cfg(not(windows))]
    let primary = passwd_username().or_else(|| std::env::var("USER").ok());
    let username = pick_username(
        primary.as_deref(),
        std::env::var("USER").ok().as_deref(),
        std::env::var("LOGNAME").ok().as_deref(),
    );
    compose_fallback_secret(node_os(), &home.display().to_string(), &username)
}

fn derive_key(secret: &str) -> [u8; 32] {
    let d = Sha256::digest(secret.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

pub fn is_encrypted(v: &str) -> bool {
    v.starts_with(PREFIX)
}

pub fn decrypt_with_secret(value: &str, secret: &str) -> Result<String, String> {
    let body = value.strip_prefix(PREFIX).ok_or("不是 enc:v1 格式")?;
    let parts: Vec<&str> = body.split('.').collect();
    if parts.len() != 3 {
        return Err("enc:v1 格式不正确".into());
    }
    let nonce_b = URL_SAFE_NO_PAD.decode(parts[0]).map_err(|e| format!("nonce 解码失败：{e}"))?;
    let tag_b = URL_SAFE_NO_PAD.decode(parts[1]).map_err(|e| format!("tag 解码失败：{e}"))?;
    let ct_b = URL_SAFE_NO_PAD.decode(parts[2]).map_err(|e| format!("密文解码失败：{e}"))?;
    if nonce_b.len() != 12 {
        return Err("nonce 长度异常".into());
    }
    let key = derive_key(secret);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| format!("密钥初始化失败：{e}"))?;
    let mut buf = ct_b.clone();
    buf.extend_from_slice(&tag_b);
    let pt = cipher
        .decrypt(Nonce::from_slice(&nonce_b), buf.as_slice())
        .map_err(|_| "解密失败（密钥不匹配或数据损坏）".to_string())?;
    Ok(String::from_utf8_lossy(&pt).to_string())
}

#[cfg(test)]
pub fn encrypt_with_secret(plain: &str, secret: &str) -> Result<String, String> {
    use aes_gcm::aead::rand_core::RngCore;
    let mut nonce_b = [0u8; 12];
    aes_gcm::aead::OsRng.fill_bytes(&mut nonce_b);
    let key = derive_key(secret);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| format!("密钥初始化失败：{e}"))?;
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce_b), plain.as_bytes())
        .map_err(|e| format!("加密失败：{e}"))?;
    let (ct_b, tag_b) = ct.split_at(ct.len() - 16);
    Ok(format!(
        "{}{}.{}.{}",
        PREFIX,
        URL_SAFE_NO_PAD.encode(nonce_b),
        URL_SAFE_NO_PAD.encode(tag_b),
        URL_SAFE_NO_PAD.encode(ct_b)
    ))
}

pub fn decrypt_json_opt(value: Option<&str>, secret: &str) -> Option<Value> {
    let v = value?;
    let plain = if is_encrypted(v) { decrypt_with_secret(v, secret).ok()? } else { v.to_string() };
    serde_json::from_str(&plain).ok()
}

pub fn decode_jwt(jwt: &str) -> Option<Value> {
    let mut parts = jwt.split('.');
    let _header = parts.next()?;
    let payload = parts.next()?;
    if payload.is_empty() {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Identity {
    pub provider: String,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub user_id: Option<String>,
}

impl Identity {
    pub fn label(&self) -> Option<String> {
        self.display_name
            .clone()
            .or_else(|| self.username.clone())
            .or_else(|| self.email.clone())
    }
}

pub fn identity_with_secret(creds: &Value, secret: &str) -> Identity {
    let mut id = Identity { provider: "bigmodel".into(), ..Default::default() };
    let map = match creds.as_object() {
        Some(m) => m,
        None => return id,
    };
    if let Some(ap) = map.get("oauth:active_provider").and_then(|v| v.as_str()) {
        let plain = if is_encrypted(ap) { decrypt_with_secret(ap, secret).ok() } else { Some(ap.to_string()) };
        if let Some(p) = plain.filter(|p| !p.is_empty()) {
            id.provider = p;
        }
    }
    let ui_key = format!("oauth:{}:user_info", id.provider);
    let ui = map
        .get(ui_key.as_str())
        .and_then(|v| v.as_str())
        .and_then(|v| decrypt_json_opt(Some(v), secret));
    if let Some(ui) = ui {
        id.username = ui.get("username").and_then(|v| v.as_str()).map(String::from);
        id.display_name = ui.get("displayName").and_then(|v| v.as_str()).map(String::from);
        id.email = ui
            .get("email")
            .and_then(|v| v.as_str())
            .or_else(|| ui.get("rawProfile").and_then(|r| r.get("email")).and_then(|v| v.as_str()))
            .map(String::from);
        if let Some(uid) = ui.get("id") {
            id.user_id = uid.as_str().map(String::from).or_else(|| serde_json::to_string(uid).ok());
        }
    }
    if id.user_id.is_none() {
        let at_key = format!("oauth:{}:access_token", id.provider);
        if let Some(at) = map.get(at_key.as_str()).and_then(|v| v.as_str()) {
            let plain = if is_encrypted(at) { decrypt_with_secret(at, secret).ok() } else { Some(at.to_string()) };
            if let Some(jwt) = plain.as_deref().and_then(decode_jwt) {
                id.user_id = jwt
                    .get("user_id")
                    .or_else(|| jwt.get("sub"))
                    .and_then(|v| v.as_str())
                    .map(String::from);
            }
        }
    }
    id
}

pub fn account_identity(creds: &Value, home: &Path) -> Identity {
    identity_with_secret(creds, &default_secret(home))
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: &str = "test-secret-zswitch";

    #[test]
    fn roundtrip() {
        for plain in ["hello", "中文内容测试", "{\"a\":1}", &"x".repeat(500)] {
            let enc = encrypt_with_secret(plain, S).unwrap();
            assert!(enc.starts_with("enc:v1:"));
            assert_eq!(enc.matches('.').count(), 2);
            assert_eq!(decrypt_with_secret(&enc, S).unwrap(), plain);
        }
    }

    #[test]
    fn node_platform_mapping_follows_node_semantics() {
        assert_eq!(node_platform_for("windows"), "win32");
        assert_eq!(node_platform_for("macos"), "darwin");
        assert_eq!(node_platform_for("linux"), "linux");
        assert_eq!(node_os(), node_platform_for(std::env::consts::OS));
    }

    #[test]
    fn fallback_secret_format_is_platform_scoped() {
        assert_eq!(
            compose_fallback_secret("win32", "C:\\Users\\john", "john"),
            "zcode-credential-fallback:win32:C:\\Users\\john:john"
        );
        assert_eq!(
            compose_fallback_secret("darwin", "/Users/john", "john"),
            "zcode-credential-fallback:darwin:/Users/john:john"
        );
        assert_eq!(
            compose_fallback_secret("linux", "/home/john", "john"),
            "zcode-credential-fallback:linux:/home/john:john"
        );
    }

    #[test]
    fn username_pick_prefers_username_then_user_then_logname() {
        assert_eq!(pick_username(Some("a"), Some("b"), Some("c")), "a");
        assert_eq!(pick_username(None, Some("b"), Some("c")), "b");
        assert_eq!(pick_username(None, None, Some("c")), "c");
        assert_eq!(pick_username(None, None, None), "unknown");
    }

    #[test]
    fn node_cross_language_vector() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("test-vectors/node-enc-v1.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("跨语言测试向量缺失 {e}：文件必须随仓库提交"));
        let v = serde_json::from_str::<serde_json::Value>(&raw)
            .expect("跨语言测试向量文件损坏");
        let secret = v["secret"].as_str().expect("向量缺 secret");
        let enc = v["enc"].as_str().expect("向量缺 enc");
        let plain = v["plain"].as_str().expect("向量缺 plain");
        let nonce_part = enc.trim_start_matches("enc:v1:").split('.').next().unwrap();
        let decoded = URL_SAFE_NO_PAD.decode(nonce_part).unwrap();
        assert_eq!(decoded.len(), 12, "向量 nonce 必须是 12 字节");
        assert_eq!(decrypt_with_secret(enc, secret).unwrap(), plain);

        let enc2 = encrypt_with_secret("cross-check", S).unwrap();
        assert_eq!(decrypt_with_secret(&enc2, S).unwrap(), "cross-check");
    }

    #[test]
    fn jwt_decode() {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"user_id":"123456","sub":"s"}"#);
        let jwt = format!("eyJhbGciOiJIUzI1NiJ9.{payload}.sig");
        let v = decode_jwt(&jwt).unwrap();
        assert_eq!(v["user_id"], "123456");
        assert!(decode_jwt("not-a-jwt").is_none());
    }

    #[test]
    fn identity_extract() {
        let enc_ui = encrypt_with_secret(
            r#"{"id":"u1","username":"vcjzxsv6","displayName":"测试号","rawProfile":{"email":"a@b.c"}}"#,
            S,
        )
        .unwrap();
        let creds = serde_json::json!({
            "oauth:active_provider": encrypt_with_secret("bigmodel", S).unwrap(),
            "oauth:bigmodel:user_info": enc_ui,
        });
        let id = identity_with_secret(&creds, S);
        assert_eq!(id.provider, "bigmodel");
        assert_eq!(id.display_name.as_deref(), Some("测试号"));
        assert_eq!(id.username.as_deref(), Some("vcjzxsv6"));
        assert_eq!(id.email.as_deref(), Some("a@b.c"));
        assert_eq!(id.label().as_deref(), Some("测试号"));
    }
}
