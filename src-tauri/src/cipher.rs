
use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use pbkdf2::pbkdf2_hmac;
use serde_json::{json, Value};
use sha2::Sha256;

pub const FORMAT_BUNDLE: &str = "zsw-accounts-bundle";
pub const KDF_ITERS: u32 = 100_000;

fn derive_key(password: &str, salt: &[u8]) -> [u8; 32] {
    let mut key = [0u8; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, KDF_ITERS, &mut key);
    key
}

pub fn seal(payload: &Value, password: &str, format: &str) -> Result<Value, String> {
    if password.trim().is_empty() {
        return Err("密码不能为空（导出文件包含登录凭据，必须加密）".into());
    }
    let mut salt = [0u8; 16];
    let mut nonce_b = [0u8; 12];
    aes_gcm::aead::OsRng.fill_bytes(&mut salt);
    aes_gcm::aead::OsRng.fill_bytes(&mut nonce_b);

    let plain = serde_json::to_vec(payload).map_err(|e| format!("序列化失败：{e}"))?;
    let key = derive_key(password, &salt);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| format!("密钥错误：{e}"))?;
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce_b), plain.as_slice())
        .map_err(|e| format!("加密失败：{e}"))?;
    let (data, tag) = ct.split_at(ct.len() - 16);

    Ok(json!({
        "format": format,
        "version": 1,
        "kdf": { "algo": "pbkdf2-hmac-sha256", "iters": KDF_ITERS, "salt": B64.encode(salt) },
        "cipher": { "algo": "aes-256-gcm", "nonce": B64.encode(nonce_b), "tag": B64.encode(tag), "data": B64.encode(data) },
    }))
}

pub fn open(envelope: &Value, password: &str) -> Result<Value, String> {
    let kdf = envelope.get("kdf").ok_or("文件缺少 kdf 段（不是有效的加密导出）")?;
    let c = envelope.get("cipher").ok_or("文件缺少 cipher 段")?;
    let salt = B64.decode(kdf.get("salt").and_then(|v| v.as_str()).unwrap_or_default())
        .map_err(|_| "salt 解码失败")?;
    let nonce_b = B64.decode(c.get("nonce").and_then(|v| v.as_str()).unwrap_or_default())
        .map_err(|_| "nonce 解码失败")?;
    let tag = B64.decode(c.get("tag").and_then(|v| v.as_str()).unwrap_or_default())
        .map_err(|_| "tag 解码失败")?;
    let data = B64.decode(c.get("data").and_then(|v| v.as_str()).unwrap_or_default())
        .map_err(|_| "数据解码失败")?;
    if nonce_b.len() != 12 {
        return Err("nonce 长度异常".into());
    }

    let key = derive_key(password, &salt);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| format!("密钥错误：{e}"))?;
    let mut buf = data.clone();
    buf.extend_from_slice(&tag);
    let plain = cipher
        .decrypt(Nonce::from_slice(&nonce_b), buf.as_slice())
        .map_err(|_| "密码错误或文件已损坏".to_string())?;
    serde_json::from_slice(&plain).map_err(|e| format!("解密后内容异常：{e}"))
}

pub fn is_sealed(v: &Value) -> bool {
    v.get("kdf").is_some() && v.get("cipher").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn seal_open_roundtrip() {
        let payload = json!({
            "format": "zcode-account", "version": 2, "name": "测试号",
            "credentials": { "oauth:bigmodel:access_token": "enc:v1:xxx" },
            "config": { "provider": {} },
        });
        let sealed = seal(&payload, "我的密码123", FORMAT_BUNDLE).unwrap();
        assert_eq!(sealed["format"], FORMAT_BUNDLE);
        assert_eq!(sealed["kdf"]["iters"], KDF_ITERS);
        let raw = serde_json::to_string(&sealed).unwrap();
        assert!(!raw.contains("测试号"));
        assert!(!raw.contains("oauth:bigmodel"));

        let opened = open(&sealed, "我的密码123").unwrap();
        assert_eq!(opened["name"], "测试号");
        assert_eq!(opened["config"]["provider"], json!({}));
    }

    #[test]
    fn wrong_password_rejected() {
        let sealed = seal(&json!({"a": 1}), "correct", FORMAT_BUNDLE).unwrap();
        assert!(open(&sealed, "wrong").unwrap_err().contains("密码错误"));
    }

    #[test]
    fn empty_password_rejected_and_detect() {
        assert!(seal(&json!({}), "  ", FORMAT_BUNDLE).is_err());
        assert!(is_sealed(&seal(&json!({}), "x", FORMAT_BUNDLE).unwrap()));
        assert!(!is_sealed(&json!({"format": "zcode-account"})));
    }
}
