
use crate::zcrypto;
use serde_json::Value;
use std::path::Path;
use std::thread::sleep;
use std::time::Duration;

#[cfg(windows)]
fn no_window(prog: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut c = std::process::Command::new(prog);
    c.creation_flags(0x0800_0000);
    c
}
#[cfg(not(windows))]
fn no_window(prog: &str) -> std::process::Command {
    std::process::Command::new(prog)
}

pub const QUOTA_LIMIT_URL: &str = "https://open.bigmodel.cn/api/monitor/usage/quota/limit";
pub const SUBSCRIPTION_URL: &str = "https://open.bigmodel.cn/api/biz/subscription/list";
pub const BILLING_BALANCE_URL: &str = "https://zcode.z.ai/api/v1/zcode-plan/billing/balance";
pub const CLIENT_APP_VERSION: &str = "3.10.1";

pub(crate) fn client_platform() -> String {
    let os = crate::zcrypto::node_platform_for(std::env::consts::OS);
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    format!("{os}-{arch}")
}

const ZCODE_ORIGIN: &str = "https://zcode.z.ai";
const ZCODE_LANG: &str = "zh-CN";
const ZCODE_CHANNEL: &str = "stable";

pub(crate) fn device_mid() -> Option<String> {
    static CACHE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            let home = std::env::var("ZCODE_SWITCH_HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_default();
            let p = std::path::Path::new(&home).join(".zcode").join("v2").join("telemetry-state.json");
            std::fs::read_to_string(p)
                .ok()
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .and_then(|v| v.get("deviceMid").and_then(|m| m.as_str()).map(String::from))
        })
        .clone()
}

fn os_version() -> Option<String> {
    static CACHE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            let out = no_window("reg")
                .args(["query", r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion", "/v", "CurrentBuildNumber"])
                .output()
                .ok()?;
            let txt = String::from_utf8_lossy(&out.stdout);
            let build = txt.lines().find(|l| l.contains("CurrentBuildNumber"))?
                .rsplit(' ').find(|t| !t.is_empty())?.to_string();
            Some(format!("10.0.{build}"))
        })
        .clone()
}

fn client_timezone() -> String {
    static CACHE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            let out = no_window("tzutil").arg("/g").output().ok();
            let name = out
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            match name.as_str() {
                "China Standard Time" | "China Daylight Time" => "Asia/Shanghai",
                "Singapore Standard Time" => "Asia/Singapore",
                "Tokyo Standard Time" => "Asia/Tokyo",
                "UTC" => "UTC",
                _ => "unknown",
            }
            .to_string()
        })
        .clone()
}

pub(crate) fn zcode_app_version() -> String {
    static CACHE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            for hive in [
                r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
                r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
                r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
            ] {
                let Ok(out) = no_window("reg").args(["query", hive, "/s"]).output() else {
                    continue;
                };
                let txt = String::from_utf8_lossy(&out.stdout);
                let (mut name, mut ver) = (String::new(), String::new());
                for line in txt.lines() {
                    let l = line.trim();
                    if l.starts_with("HKEY_") {
                        if is_zcode_display_name(&name) && !ver.is_empty() {
                            return normalize_version(&ver);
                        }
                        name.clear();
                        ver.clear();
                        continue;
                    }
                    if let Some(rest) = l.strip_prefix("DisplayName") {
                        name = rest.trim_start().trim_start_matches("REG_SZ").trim().to_string();
                    } else if let Some(rest) = l.strip_prefix("DisplayVersion") {
                        ver = rest.trim_start().trim_start_matches("REG_SZ").trim().to_string();
                    }
                }
                if is_zcode_display_name(&name) && !ver.is_empty() {
                    return normalize_version(&ver);
                }
            }
            CLIENT_APP_VERSION.to_string()
        })
        .clone()
}

fn is_zcode_display_name(name: &str) -> bool {
    let l = name.to_lowercase();
    l.contains("zcode") && !l.contains("switch")
}

fn normalize_version(v: &str) -> String {
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() >= 3 {
        format!("{}.{}.{}", parts[0], parts[1], parts[2])
    } else {
        v.to_string()
    }
}

pub(crate) fn zai_billing_headers(token: &str) -> Vec<(String, String)> {
    zai_billing_headers_with_mid(token, device_mid())
}

pub(crate) fn zai_billing_headers_with_mid(token: &str, mid: Option<String>) -> Vec<(String, String)> {
    let ver = zcode_app_version();
    let mut h: Vec<(String, String)> = vec![
        ("User-Agent".into(), format!("ZCode/{ver}")),
        ("HTTP-Referer".into(), ZCODE_ORIGIN.into()),
        ("X-Title".into(), "Z Code@electron".into()),
        ("X-ZCode-App-Version".into(), ver.clone()),
        ("X-Platform".into(), client_platform()),
        ("X-Release-Channel".into(), ZCODE_CHANNEL.into()),
        ("X-Client-Language".into(), ZCODE_LANG.into()),
        ("X-Client-Timezone".into(), client_timezone()),
        ("X-Os-Category".into(), std::env::consts::OS.into()),
    ];
    if let Some(v) = os_version() {
        h.push(("X-Os-Version".into(), v));
    }
    if let Some(mid) = mid {
        h.push(("X-Device-Mid".into(), mid));
    }
    h.push(("Authorization".into(), format!("Bearer {token}")));
    h.push(("x-request-id".into(), uuid::Uuid::new_v4().to_string()));
    h
}

fn bigmodel_headers(token: &str) -> Vec<(String, String)> {
    vec![
        ("Authorization".into(), format!("Bearer {token}")),
        ("User-Agent".into(), format!("ZCode/{}", zcode_app_version())),
        ("x-request-id".into(), uuid::Uuid::new_v4().to_string()),
    ]
}

#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct QuotaItem {
    pub name: String,
    pub total: Option<f64>,
    pub used: Option<f64>,
    pub remaining: Option<f64>,
    pub percent_used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_percentage: Option<f64>,
    pub unit: String,
    pub period_end: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct PlanSlot {
    #[serde(skip_serializing)]
    pub pid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expire: Option<String>,
    pub total: Option<f64>,
    pub used: Option<f64>,
    pub remaining: Option<f64>,
    pub percent_used: Option<f64>,
    pub items: Vec<QuotaItem>,
}

#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct QuotaOverview {
    pub total: Option<f64>,
    pub used: Option<f64>,
    pub remaining: Option<f64>,
    pub percent_used: Option<f64>,
    pub plan_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_expire: Option<String>,
    pub is_empty: bool,
    pub items: Vec<QuotaItem>,
    pub refreshed_at: i64,
    pub source: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub plans: Vec<PlanSlot>,
}

fn safe_decrypt(value: Option<&str>, secret: &str) -> Option<String> {
    let v = value?;
    if zcrypto::is_encrypted(v) {
        zcrypto::decrypt_with_secret(v, secret).ok()
    } else {
        Some(v.to_string())
    }
}

fn looks_like_token(v: &str) -> bool {
    v.trim().len() > 20
}

fn coding_plan_api_keys(config: Option<&Value>) -> Vec<String> {
    let mut keys = vec![];
    let providers = match config.and_then(|c| c.get("provider")).and_then(|p| p.as_object()) {
        Some(p) => p,
        None => return keys,
    };
    let mut ordered: Vec<(&String, &Value)> = providers.iter().collect();
    ordered.sort_by_key(|(_, p)| if p.get("enabled").and_then(|e| e.as_bool()).unwrap_or(false) { 0 } else { 1 });
    for (id, p) in ordered {
        if !id.contains("coding-plan") {
            continue;
        }
        if let Some(k) = p.get("options").and_then(|o| o.get("apiKey")).and_then(|k| k.as_str()) {
            if !k.starts_with("enc:") && looks_like_token(k) && !keys.contains(&k.to_string()) {
                keys.push(k.to_string());
            }
        }
    }
    keys
}

pub fn candidate_tokens(creds: &Value, config: Option<&Value>, secret: &str) -> Vec<String> {
    let mut tokens: Vec<String> = vec![];
    let add = |plain: Option<String>, tokens: &mut Vec<String>| {
        if let Some(p) = plain {
            if looks_like_token(&p) && !tokens.contains(&p) {
                tokens.push(p);
            }
        }
    };
    for k in coding_plan_api_keys(config) {
        tokens.push(k);
    }
    let active = safe_decrypt(
        creds.get("oauth:active_provider").and_then(|v| v.as_str()),
        secret,
    )
    .unwrap_or_else(|| "zai".into());
    let map = creds.as_object();
    add(
        map.and_then(|m| m.get("zcodejwttoken")).and_then(|v| v.as_str()).and_then(|v| safe_decrypt(Some(v), secret)),
        &mut tokens,
    );
    for key in [
        format!("oauth:{active}:access_token"),
        "oauth:bigmodel:access_token".to_string(),
        "oauth:zai:access_token".to_string(),
    ] {
        add(
            map.and_then(|m| m.get(&key)).and_then(|v| v.as_str()).and_then(|v| safe_decrypt(Some(v), secret)),
            &mut tokens,
        );
    }
    tokens
}

fn http_get_json(url: &str, token: &str, retry_429: bool) -> Result<Value, String> {
    let retry_delays = [500u64, 1500, 4000];
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build();
    let headers = if url.contains("zcode.z.ai") {
        zai_billing_headers(token)
    } else {
        bigmodel_headers(token)
    };
    let mut last_err: Option<String> = None;
    let mut backoff: std::slice::Iter<'_, u64> = if retry_429 { retry_delays.iter() } else { [].iter() };
    loop {
        let mut req = agent.get(url);
        for (k, v) in &headers {
            req = req.set(k, v);
        }
        let resp = req.call();
        match resp {
            Ok(r) => {
                let text = r.into_string().map_err(|e| format!("读取响应失败：{e}"))?;
                return Ok(if text.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_str(&text).unwrap_or(Value::String(text.clone()))
                });
            }
            Err(ureq::Error::Status(code, r)) => {
                let body = r.into_string().unwrap_or_default();
                if let Ok(v) = serde_json::from_str::<Value>(&body) {
                    if v.get("code").and_then(|c| c.as_i64()) == Some(401) {
                        return Err("Token 已过期或无效（业务码 401）".into());
                    }
                }
                if code == 429 {
                    match backoff.next() {
                        Some(d) => {
                            last_err = Some("服务端限流，正在重试...".into());
                            sleep(Duration::from_millis(*d));
                            continue;
                        }
                        None => return Err(last_err.unwrap_or_else(|| "额度接口 HTTP 429".into())),
                    }
                }
                if code == 401 || code == 403 {
                    return Err(format!("Token 已过期或无效（HTTP {code}）"));
                }
                let msg = serde_json::from_str::<Value>(&body)
                    .ok()
                    .and_then(|v| ["message", "msg", "error"].iter().find_map(|k| v.get(k).and_then(|x| x.as_str()).map(String::from)));
                return Err(format!("额度接口 HTTP {code}: {}", msg.unwrap_or_default()));
            }
            Err(e) => return Err(format!("网络请求失败：{e}")),
        }
    }
}

type FetchFn<'a> = dyn Fn(&str, &str) -> Result<Value, String> + 'a;

fn query_with_token_via(token: &str, fetch: &FetchFn) -> Result<QuotaOverview, String> {
    let mut best_err: Option<String>;

    match fetch(QUOTA_LIMIT_URL, token) {
        Ok(limit_resp) => {
            if business_ok(&limit_resp) {
                let sub = fetch(SUBSCRIPTION_URL, token).ok();
                let mut ov = normalize_quota_limit(&limit_resp, sub.as_ref());
                ov.refreshed_at = chrono::Local::now().timestamp_millis();
                ov.source = "bigmodel.cn/api/monitor".into();
                return Ok(ov);
            }
            let code = limit_resp.get("code").and_then(|c| c.as_i64());
            best_err = Some(match code {
                Some(401) => "Token 已过期或无效（业务码 401）".into(),
                Some(c) => {
                    let msg = ["msg", "message", "error"]
                        .iter()
                        .find_map(|k| limit_resp.get(k).and_then(|x| x.as_str()).map(String::from))
                        .unwrap_or_default();
                    format!("业务码 {c}: {msg}")
                }
                None => "额度接口返回异常".into(),
            });
        }
        Err(e) => best_err = Some(e),
    }

    let url = format!("{BILLING_BALANCE_URL}?app_version={}", zcode_app_version());
    match fetch(&url, token) {
        Ok(balance) if business_ok(&balance) => {
            let mut ov = normalize_balance(&balance);
            ov.refreshed_at = chrono::Local::now().timestamp_millis();
            ov.source = "zcode.z.ai/billing".into();
            return Ok(ov);
        }
        Ok(_) => {}
        Err(e) => {
            if best_err.is_none() {
                best_err = Some(e);
            }
        }
    }

    Err(best_err.unwrap_or_else(|| "额度查询失败".into()))
}

fn query_with_token(token: &str) -> Result<QuotaOverview, String> {
    query_with_token_via(token, &|url, tok| http_get_json(url, tok, true))
}

fn business_ok(v: &Value) -> bool {
    let code = v.get("code").and_then(|c| c.as_i64());
    let success = v.get("success").and_then(|s| s.as_bool());
    (code.is_none() || code == Some(200) || code == Some(0)) && success != Some(false)
}

pub fn query_quota(tokens: &[String]) -> Result<QuotaOverview, String> {
    if tokens.is_empty() {
        return Err("未找到可用于查询额度的 ZCode token，请先登录或切换账号".into());
    }
    let mut last_err: Option<String> = None;
    let mut first_business: Option<String> = None;
    let mut auth_fail = 0usize;
    for t in tokens {
        match query_with_token(t) {
            Ok(ov) => return Ok(ov),
            Err(e) => {
                if e.contains("401") {
                    auth_fail += 1;
                } else if first_business.is_none() {
                    first_business = Some(e.clone());
                }
                last_err = Some(e);
            }
        }
    }
    if auth_fail > 0 && auth_fail == tokens.len() {
        sleep(Duration::from_millis(1500));
        if let Ok(ov) = query_with_token(&tokens[0]) {
            return Ok(ov);
        }
        return Err("该账号 Token 已过期，请删除后重新登录".into());
    }
    Err(first_business.or(last_err).unwrap_or_else(|| "额度查询失败".into()))
}

#[derive(Debug, Clone, PartialEq)]
enum Channel {
    Monitor(String),
    ZaiBilling(String),
}

fn is_no_plan_message(msg: &str) -> bool {
    msg.contains("不存在coding plan") || msg.contains("没有资格")
}

pub(crate) fn zai_billing_token(creds: &Value, config: Option<&Value>, secret: &str) -> Option<String> {
    let jwt = safe_decrypt(creds.get("zcodejwttoken").and_then(|v| v.as_str()), secret);
    let active = safe_decrypt(
        creds.get("oauth:active_provider").and_then(|v| v.as_str()),
        secret,
    );
    let use_jwt = if active.as_deref() != Some("bigmodel") {
        jwt.is_some()
    } else {
        false
    };
    if use_jwt {
        return jwt;
    }
    let providers = config?.get("provider")?.as_object()?;
    let mut ordered: Vec<(&String, &Value)> = providers.iter().collect();
    ordered.sort_by_key(|(_, p)| if p.get("enabled").and_then(|e| e.as_bool()).unwrap_or(false) { 0 } else { 1 });
    for (id, p) in ordered {
        if id.contains("start-plan") {
            if let Some(k) = p.get("options").and_then(|o| o.get("apiKey")).and_then(|k| k.as_str()) {
                if !k.starts_with("enc:") && looks_like_token(k) {
                    return Some(k.to_string());
                }
            }
        }
    }
    None
}

fn pick_channels(creds: &Value, config: Option<&Value>, secret: &str) -> Vec<Channel> {
    let mut chans: Vec<Channel> = vec![];
    let providers = match config.and_then(|c| c.get("provider")).and_then(|p| p.as_object()) {
        Some(p) => p,
        None => return chans,
    };
    let mut ordered: Vec<(&String, &Value)> = providers.iter().collect();
    ordered.sort_by_key(|(_, p)| if p.get("enabled").and_then(|e| e.as_bool()).unwrap_or(false) { 0 } else { 1 });
    let provider_key = |p: &Value| -> Option<String> {
        let k = p.get("options").and_then(|o| o.get("apiKey")).and_then(|k| k.as_str())?;
        (!k.starts_with("enc:") && looks_like_token(k)).then(|| k.to_string())
    };
    for (id, p) in ordered {
        let key = provider_key(p);
        if id.contains("start-plan") {
            let jwt = safe_decrypt(
                creds.get("zcodejwttoken").and_then(|v| v.as_str()),
                secret,
            );
            let active = safe_decrypt(
                creds.get("oauth:active_provider").and_then(|v| v.as_str()),
                secret,
            );
            let use_jwt = if id.starts_with("builtin:zai") {
                jwt.is_some()
            } else {
                jwt.is_some() && active.as_deref() == Some("bigmodel")
            };
            let tok = if use_jwt { jwt } else { None }.or(key);
            if let Some(t) = tok {
                if !chans.contains(&Channel::ZaiBilling(t.clone())) {
                    chans.push(Channel::ZaiBilling(t));
                }
            }
        } else if id.contains("coding-plan") {
            if let Some(k) = key {
                if !chans.contains(&Channel::Monitor(k.clone())) {
                    chans.push(Channel::Monitor(k));
                }
            }
        }
    }
    chans
}

fn no_plan_overview() -> QuotaOverview {
    QuotaOverview {
        plan_tier: None,
        is_empty: true,
        source: "no_plan".into(),
        ..Default::default()
    }
}

fn query_channels_via(channels: &[Channel], fetch: &FetchFn) -> Result<QuotaOverview, String> {
    let mut best_err: Option<String> = None;
    let mut saw_no_plan = false;
    let mut parts: Vec<QuotaOverview> = vec![];
    for ch in channels {
        match ch {
            Channel::Monitor(key) => match fetch(QUOTA_LIMIT_URL, key) {
                Ok(resp) => {
                    if business_ok(&resp) {
                        let sub = fetch(SUBSCRIPTION_URL, key).ok();
                        let mut ov = normalize_quota_limit(&resp, sub.as_ref());
                        ov.refreshed_at = chrono::Local::now().timestamp_millis();
                        ov.source = "bigmodel.cn/api/monitor".into();
                        parts.push(ov);
                    } else {
                        let msg = ["msg", "message", "error"]
                            .iter()
                            .find_map(|k| resp.get(k).and_then(|x| x.as_str()).map(String::from))
                            .unwrap_or_default();
                        if is_no_plan_message(&msg) {
                            saw_no_plan = true;
                        } else if best_err.is_none() {
                            let code = resp.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
                            best_err = Some(format!("业务码 {code}: {msg}"));
                        }
                    }
                }
                Err(e) => {
                    if best_err.is_none() {
                        best_err = Some(e);
                    }
                }
            },
            Channel::ZaiBilling(tok) => {
                let url = format!("{BILLING_BALANCE_URL}?app_version={}", zcode_app_version());
                match fetch(&url, tok) {
                    Ok(balance) if business_ok(&balance) => {
                        let mut ov = normalize_balance(&balance);
                        ov.refreshed_at = chrono::Local::now().timestamp_millis();
                        ov.source = "zcode.z.ai/billing".into();
                        let has_plan = ov.plan_tier.is_some() || !ov.plans.is_empty();
                        if has_plan {
                            parts.push(ov);
                        } else {
                            saw_no_plan = true;
                        }
                    }
                    Ok(resp) => {
                        if best_err.is_none() {
                            let msg = ["msg", "message", "error"]
                                .iter()
                                .find_map(|k| resp.get(k).and_then(|x| x.as_str()).map(String::from))
                                .unwrap_or_default();
                            let code = resp.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
                            best_err = Some(format!("业务码 {code}: {msg}"));
                        }
                    }
                    Err(e) => {
                        if best_err.is_none() {
                            best_err = Some(e);
                        }
                    }
                }
            }
        }
    }
    if parts.is_empty() {
        if saw_no_plan {
            return Ok(no_plan_overview());
        }
        return Err(best_err.unwrap_or_else(|| "额度查询失败".into()));
    }
    Ok(merge_parts(parts))
}

fn merge_parts(parts: Vec<QuotaOverview>) -> QuotaOverview {
    let mut slots: Vec<PlanSlot> = vec![];
    let mut slot_src: Vec<String> = vec![];
    let mut sources: Vec<&str> = vec![];
    let mut refreshed = 0i64;
    for p in &parts {
        if !sources.contains(&p.source.as_str()) {
            sources.push(&p.source);
        }
        refreshed = refreshed.max(p.refreshed_at);
        for s in &p.plans {
            let dup = slot_src.iter().enumerate().any(|(i, src)| {
                src != &p.source
                    && slots[i].tier == s.tier
                    && slots[i].name == s.name
                    && !slots[i].items.is_empty()
                    && !s.items.is_empty()
            });
            if !dup {
                slots.push(s.clone());
                slot_src.push(p.source.clone());
            }
        }
    }
    let items: Vec<QuotaItem> = slots.iter().flat_map(|s| s.items.iter().cloned()).collect();
    let content = |s: &PlanSlot| s.tier.is_some() || !s.items.is_empty() || s.total.is_some();
    let mut pri_idx: Option<usize> = None;
    for (i, s) in slots.iter().enumerate() {
        if !content(s) {
            continue;
        }
        pri_idx = match pri_idx {
            None => Some(i),
            Some(p) if tier_rank(s.tier.as_deref()) > tier_rank(slots[p].tier.as_deref()) => Some(i),
            _ => pri_idx,
        };
    }
    let (total, used, remaining, percent_used) = match pri_idx.map(|i| &slots[i]) {
        Some(p) => (p.total, p.used, p.remaining, p.percent_used),
        None => (None, None, None, None),
    };
    QuotaOverview {
        total,
        used,
        remaining,
        percent_used,
        plan_tier: pri_idx.map(|i| slots[i].tier.clone()).flatten(),
        plan_expire: pri_idx.map(|i| slots[i].expire.clone()).flatten(),
        is_empty: slots.is_empty(),
        items,
        refreshed_at: refreshed,
        source: sources.join(" + "),
        plans: slots,
    }
}

fn query_channels(channels: &[Channel]) -> Result<QuotaOverview, String> {
    query_channels_via(channels, &|url, tok| http_get_json(url, tok, true))
}

pub fn quota_for_live(home: &Path, creds: &Value, config: Option<&Value>) -> Result<QuotaOverview, String> {
    let secret = zcrypto::default_secret(home);
    let channels = pick_channels(creds, config, &secret);
    if !channels.is_empty() {
        if let Ok(ov) = query_channels(&channels) {
            return Ok(ov);
        }
    }
    let tokens = candidate_tokens(creds, config, &secret);
    query_quota(&tokens)
}

pub fn quota_for_snapshot(home: &Path, creds: &Value, config: Option<&Value>) -> Result<QuotaOverview, String> {
    quota_for_live(home, creds, config)
}

fn unit_label(unit: Option<i64>, number: Option<i64>) -> String {
    match unit {
        Some(3) => format!("每 {} 小时", number.unwrap_or(5)),
        Some(4) => "每天".into(),
        Some(5) => "每月".into(),
        Some(6) => "每周".into(),
        _ => "每周期".into(),
    }
}

fn fmt_reset_time(ms: Option<i64>) -> Option<String> {
    let ms = ms?;
    if ms <= 0 {
        return None;
    }
    use chrono::TimeZone;
    Some(chrono::Local.timestamp_millis_opt(ms).single()?.format("%m-%d %H:%M 重置").to_string())
}

fn safe_prefix(s: &str, n: usize) -> &str {
    if s.len() <= n {
        return s;
    }
    let mut end = n;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn safe_suffix(s: &str, n: usize) -> &str {
    if s.len() <= n {
        return s;
    }
    let mut start = s.len() - n;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

fn looks_like_date(d: &str) -> bool {
    d.len() == 10 && d.as_bytes().get(4) == Some(&b'-') && d.as_bytes().get(7) == Some(&b'-')
}

fn extract_expire(obj: &Value) -> Option<String> {
    const KEYS: [&str; 8] = ["nextRenewTime", "expireTime", "expire_time", "endTime", "end_time", "expireAt", "expiredTime", "validEndTime"];
    for k in KEYS {
        let Some(v) = obj.get(k) else { continue };
        if let Some(n) = v.as_i64() {
            if n > 1_000_000_000_000 {
                use chrono::TimeZone;
                return chrono::Local.timestamp_millis_opt(n).single().map(|t| t.format("%Y-%m-%d").to_string());
            }
            if n > 1_000_000_000 {
                use chrono::TimeZone;
                return chrono::Local.timestamp_opt(n, 0).single().map(|t| t.format("%Y-%m-%d").to_string());
            }
        }
        if let Some(s) = v.as_str() {
            let t = s.trim();
            if t.is_empty() {
                continue;
            }
            if let Ok(n) = t.parse::<i64>() {
                if n > 1_000_000_000_000 {
                    use chrono::TimeZone;
                    if let Some(dt) = chrono::Local.timestamp_millis_opt(n).single() {
                        return Some(dt.format("%Y-%m-%d").to_string());
                    }
                }
            }
            let d = safe_prefix(t, 10);
            if looks_like_date(d) {
                return Some(d.to_string());
            }
            return Some(t.to_string());
        }
    }
    if let Some(s) = obj.get("valid").and_then(|v| v.as_str()) {
        let tail = safe_suffix(s, 19);
        if looks_like_date(safe_prefix(tail, 10)) {
            return Some(safe_prefix(tail, 10).to_string());
        }
        let d = safe_prefix(s, 10);
        if looks_like_date(d) {
            return Some(d.to_string());
        }
    }
    None
}

fn tier_from_level(level: &str) -> String {
    let l = level.to_lowercase();
    if l.contains("max") {
        "Max".into()
    } else if l.contains("pro") {
        "Pro".into()
    } else if l.contains("lite") {
        "Lite".into()
    } else {
        level.to_string()
    }
}

fn normalize_quota_limit(limit_resp: &Value, sub_resp: Option<&Value>) -> QuotaOverview {
    let data = limit_resp.get("data").cloned().unwrap_or(Value::Null);
    let limits = data.get("limits").and_then(|l| l.as_array()).cloned().unwrap_or_default();

    let mut items = vec![];
    let mut main: Option<QuotaItem> = None;
    for l in &limits {
        let typ = l.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let unit = l.get("unit").and_then(|u| u.as_i64());
        let number = l.get("number").and_then(|n| n.as_i64());
        let total = l.get("usage").and_then(|v| v.as_f64());
        let used = l.get("currentValue").and_then(|v| v.as_f64());
        let remaining = l.get("remaining").and_then(|v| v.as_f64());
        let percentage = l.get("percentage").and_then(|v| v.as_f64());
        let reset_ms = l.get("nextResetTime").and_then(|v| v.as_i64());
        let period = unit_label(unit, number);
        let (name, unit_str) = match typ {
            "TOKENS_LIMIT" => (format!("提示次数（{period}）"), "次".into()),
            "TIME_LIMIT" => (format!("使用时长（{period}）"), "分钟".into()),
            _ => (format!("{typ}（{period}）"), "".into()),
        };
        let percent_used = if let (Some(t), Some(u)) = (total, used) {
            if t > 0.0 { Some((u / t * 100.0).clamp(0.0, 100.0)) } else { None }
        } else {
            percentage.map(|p| p.clamp(0.0, 100.0))
        };
        let item = QuotaItem {
            name,
            total,
            used,
            remaining,
            percent_used,
            server_percentage: percentage,
            unit: unit_str,
            period_end: fmt_reset_time(reset_ms),
        };
        if typ == "TIME_LIMIT" && total.is_some() {
            main = main.or(Some(item.clone()));
        }
        items.push(item);
    }

    let level = data.get("level").and_then(|l| l.as_str()).map(String::from);
    let mut plan_tier = level.as_deref().map(tier_from_level);
    let mut plan_expire: Option<String> = None;
    let mut product_name: Option<String> = None;
    if let Some(sub) = sub_resp {
        if business_ok(sub) {
            if std::env::var("ZSW_DUMP_SUB").is_ok() {
                eprintln!("[zsw] subscription/list raw: {sub}");
            }
            if let Some(arr) = sub.get("data").and_then(|d| d.as_array()) {
                let current = arr
                    .iter()
                    .find(|s| {
                        let valid = s.get("status").and_then(|x| x.as_str()) == Some("VALID");
                        let in_period = s.get("inCurrentPeriod").and_then(|x| x.as_bool()).unwrap_or(true);
                        valid && in_period
                    })
                    .or_else(|| arr.first());
                if let Some(s) = current {
                    if let Some(pn) = s.get("productName").and_then(|x| x.as_str()) {
                        if !pn.trim().is_empty() {
                            product_name = Some(pn.to_string());
                            plan_tier = Some(tier_from_level(pn));
                        }
                    }
                    plan_expire = extract_expire(s);
                }
            }
        }
    }

    let main = main.or_else(|| items.iter().find(|i| i.total.is_some()).cloned());
    let (total, used, remaining, percent_used) = match &main {
        Some(m) => (m.total, m.used, m.remaining, m.percent_used),
        None => {
            let p = items.iter().find_map(|i| i.percent_used);
            (None, None, None, p)
        }
    };

    let plans = if plan_tier.is_some() || !items.is_empty() {
        vec![PlanSlot {
            pid: String::new(),
            tier: plan_tier.clone(),
            name: product_name,
            expire: plan_expire.clone(),
            total,
            used,
            remaining,
            percent_used,
            items: items.clone(),
        }]
    } else {
        vec![]
    };

    QuotaOverview {
        total,
        used,
        remaining,
        percent_used,
        plan_tier,
        plan_expire,
        is_empty: limits.is_empty(),
        items,
        refreshed_at: 0,
        source: String::new(),
        plans,
    }
}

fn unwrap(data: &Value) -> Value {
    let mut cur = data.clone();
    for _ in 0..4 {
        if !cur.is_object() {
            return cur;
        }
        if let Some(d) = cur.get("data") {
            cur = d.clone();
            continue;
        }
        if let Some(r) = cur.get("result") {
            cur = r.clone();
            continue;
        }
        break;
    }
    cur
}

fn flatten_numbers(obj: &Value, prefix: &str, out: &mut Vec<(String, f64)>) {
    if let Some(m) = obj.as_object() {
        for (k, v) in m {
            let p = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
            if let Some(n) = to_number(v) {
                out.push((p, n));
            } else {
                flatten_numbers(v, &p, out);
            }
        }
    } else if let Some(arr) = obj.as_array() {
        for (i, v) in arr.iter().enumerate() {
            let p = format!("{prefix}.{i}");
            if let Some(n) = to_number(v) {
                out.push((p, n));
            } else {
                flatten_numbers(v, &p, out);
            }
        }
    }
}

fn to_number(v: &Value) -> Option<f64> {
    if let Some(n) = v.as_f64() {
        return if n.is_finite() { Some(n) } else { None };
    }
    if let Some(s) = v.as_str() {
        let t = s.replace(',', "");
        return t.trim().parse::<f64>().ok();
    }
    None
}

fn sum_numbers(pool: &[(String, f64)], keys: &[&str]) -> Option<f64> {
    let mut total = 0.0;
    let mut count = 0;
    for (path, v) in pool {
        let name = path.rsplit('.').next().unwrap_or("");
        if keys.contains(&name) {
            total += v;
            count += 1;
        }
    }
    if count > 0 { Some(total) } else { None }
}

fn first_number(pool: &[(String, f64)], keys: &[&str]) -> Option<f64> {
    for (path, v) in pool {
        let name = path.rsplit('.').next().unwrap_or("");
        if keys.contains(&name) {
            return Some(*v);
        }
    }
    None
}

pub fn extract_plan_tier(current_data: &Value) -> Option<String> {
    let cur = unwrap(current_data);
    let plans = cur.get("plans").and_then(|p| p.as_array())?;
    let active: Vec<&Value> = plans
        .iter()
        .filter(|p| p.get("status").and_then(|s| s.as_str()).unwrap_or("").to_lowercase() == "active")
        .collect();
    if active.is_empty() {
        return None;
    }
    let matches = |kw: &[&str]| {
        active.iter().any(|p| {
            let id = p.get("plan_id").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
            kw.iter().any(|k| id.contains(k) || name.contains(k))
        })
    };
    if matches(&["max"]) {
        Some("Max".into())
    } else if matches(&["pro"]) {
        Some("Pro".into())
    } else if matches(&["lite"]) {
        Some("Lite".into())
    } else if matches(&["start-plan", "start plan", "start"]) {
        Some("Start Plan".into())
    } else {
        None
    }
}

fn plan_tier_from_id(plan_id: &str, name: Option<&str>) -> String {
    let mut hay = plan_id.to_lowercase();
    if let Some(n) = name {
        hay.push(' ');
        hay.push_str(&n.to_lowercase());
    }
    if hay.contains("max") {
        "Max".into()
    } else if hay.contains("pro") {
        "Pro".into()
    } else if hay.contains("lite") {
        "Lite".into()
    } else if hay.contains("start") {
        "Start Plan".into()
    } else if ["trial", "taste", "experience", "gift", "weekend", "promo", "activity", "体验"]
        .iter()
        .any(|k| hay.contains(k))
    {
        "体验".into()
    } else {
        plan_id.to_string()
    }
}

fn tier_rank(tier: Option<&str>) -> u8 {
    let t = tier.unwrap_or("");
    if t.eq_ignore_ascii_case("max") {
        5
    } else if t.eq_ignore_ascii_case("pro") {
        4
    } else if t.eq_ignore_ascii_case("lite") {
        3
    } else if t.eq_ignore_ascii_case("Start Plan") {
        2
    } else if t == "体验" {
        1
    } else {
        0
    }
}

fn normalize_balance(balance_data: &Value) -> QuotaOverview {
    let balance = unwrap(balance_data);
    if std::env::var("ZSW_DUMP_SUB").is_ok() {
        eprintln!("[zsw] billing/balance raw: {balance_data}");
    }
    let mut pool = vec![];
    flatten_numbers(&balance, "", &mut pool);

    let mut slots: Vec<PlanSlot> = balance
        .get("plans")
        .and_then(|p| p.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|pl| {
                    pl.get("status")
                        .and_then(|s| s.as_str())
                        .map(|s| s.eq_ignore_ascii_case("active"))
                        .unwrap_or(false)
                })
                .map(|pl| {
                    let pid = pl.get("plan_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let pname = pl.get("name").and_then(|v| v.as_str()).map(str::to_string);
                    PlanSlot {
                        pid: pid.clone(),
                        tier: Some(plan_tier_from_id(&pid, pname.as_deref())),
                        name: Some(pname.filter(|s| !s.trim().is_empty()).unwrap_or(pid)),
                        expire: extract_expire(pl),
                        ..Default::default()
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let mut loose: Vec<QuotaItem> = vec![];
    let any_pid = balance
        .get("balances")
        .and_then(|b| b.as_array())
        .map(|arr| {
            arr.iter().any(|item| {
                ["plan_id", "planId", "entitlement_id"]
                    .iter()
                    .any(|k| item.get(k).and_then(Value::as_str).map(|s| !s.is_empty()).unwrap_or(false))
            })
        })
        .unwrap_or(false);
    if let Some(arr) = balance.get("balances").and_then(|b| b.as_array()) {
        for item in arr {
            let it_total = item.get("total_units").and_then(to_number);
            let it_used = item.get("used_units").and_then(to_number);
            let it_remaining = item
                .get("remaining_units")
                .and_then(to_number)
                .or_else(|| item.get("available_units").and_then(to_number));
            let it = QuotaItem {
                name: ["show_name", "name", "entitlement_id", "plan_id"]
                    .iter()
                    .find_map(|k| item.get(k).and_then(Value::as_str))
                    .unwrap_or("未知模型")
                    .to_string(),
                total: it_total,
                used: it_used,
                remaining: it_remaining,
                percent_used: match (it_total, it_used) {
                    (Some(t), Some(u)) if t > 0.0 => Some((u / t * 100.0).clamp(0.0, 100.0)),
                    _ => None,
                },
                server_percentage: None,
                unit: item.get("unit_type").or_else(|| item.get("meter")).and_then(Value::as_str).unwrap_or("quota").to_string(),
                period_end: ["period_end", "expires_at"].iter().find_map(|k| item.get(k).and_then(Value::as_str)).map(String::from),
            };
            let bpid = ["plan_id", "planId", "entitlement_id"]
                .iter()
                .find_map(|k| item.get(k).and_then(Value::as_str))
                .unwrap_or("");
            let target = if !bpid.is_empty() {
                slots.iter_mut().find(|s| s.pid == bpid)
            } else if slots.len() == 1 && !any_pid {
                slots.first_mut()
            } else {
                None
            };
            match target {
                Some(s) => s.items.push(it),
                None => loose.push(it),
            }
        }
    }

    if slots.is_empty() {
        if !loose.is_empty() {
            slots.push(PlanSlot {
                tier: extract_plan_tier(balance_data),
                items: std::mem::take(&mut loose),
                ..Default::default()
            });
        } else {
            let ptot = sum_numbers(&pool, &["total_units"]).or_else(|| {
                first_number(&pool, &["total", "totalQuota", "totalCredits", "quotaTotal", "amountTotal", "creditTotal"])
            });
            let pused = sum_numbers(&pool, &["used_units"]).or_else(|| {
                first_number(&pool, &["used", "usedQuota", "usedCredits", "quotaUsed", "amountUsed", "consumed", "totalUsed"])
            });
            let prem = sum_numbers(&pool, &["remaining_units"]).or_else(|| {
                first_number(&pool, &["remaining", "remain", "balance", "available", "availableQuota", "left", "quotaRemaining"])
            });
            if ptot.is_some() || pused.is_some() || prem.is_some() {
                slots.push(PlanSlot {
                    tier: extract_plan_tier(balance_data),
                    total: ptot,
                    used: pused,
                    remaining: prem,
                    ..Default::default()
                });
            }
        }
    } else if !loose.is_empty() {
        slots.push(PlanSlot {
            name: Some("其他额度".into()),
            items: loose,
            ..Default::default()
        });
    }

    for s in &mut slots {
        let sum = |f: fn(&QuotaItem) -> Option<f64>| -> Option<f64> {
            let vals: Vec<f64> = s.items.iter().filter_map(f).collect();
            (!vals.is_empty()).then(|| vals.iter().sum())
        };
        if s.items.is_empty() && s.total.is_none() {
            continue;
        }
        s.total = sum(|i| i.total).or(s.total);
        s.used = sum(|i| i.used).or(s.used);
        s.remaining = sum(|i| i.remaining).or(s.remaining);
        if s.total.is_none() {
            if let (Some(u), Some(r)) = (s.used, s.remaining) {
                s.total = Some(u + r);
            }
        }
        if s.used.is_none() {
            if let (Some(t), Some(r)) = (s.total, s.remaining) {
                s.used = Some((t - r).max(0.0));
            }
        }
        if s.remaining.is_none() {
            if let (Some(t), Some(u)) = (s.total, s.used) {
                s.remaining = Some((t - u).max(0.0));
            }
        }
        if s.percent_used.is_none() {
            s.percent_used = match (s.total, s.used) {
                (Some(t), Some(u)) if t > 0.0 => Some((u / t * 100.0).clamp(0.0, 100.0)),
                _ => None,
            };
        }
    }
    let mut pri_idx = 0usize;
    for (i, s) in slots.iter().enumerate() {
        if tier_rank(s.tier.as_deref()) > tier_rank(slots[pri_idx].tier.as_deref()) {
            pri_idx = i;
        }
    }

    let items: Vec<QuotaItem> = slots.iter().flat_map(|s| s.items.iter().cloned()).collect();

    let plan_expire_chain = extract_expire(&balance)
        .or_else(|| {
            balance.get("plans").and_then(|p| p.as_array()).and_then(|arr| {
                arr.iter()
                    .find(|pl| pl.get("status").and_then(|s| s.as_str()).map(|s| s.eq_ignore_ascii_case("active")).unwrap_or(false))
                    .and_then(extract_expire)
                    .or_else(|| arr.first().and_then(extract_expire))
            })
        })
        .or_else(|| {
            balance.get("balances").and_then(|b| b.as_array()).and_then(|arr| {
                arr.iter()
                    .filter_map(|it| it.get("expires_at").and_then(|v| v.as_i64()))
                    .filter(|n| *n > 1_000_000_000)
                    .max()
                    .and_then(|n| {
                        use chrono::TimeZone;
                        chrono::Local.timestamp_opt(n, 0).single().map(|t| t.format("%Y-%m-%d").to_string())
                    })
            })
        })
        .or_else(|| items.iter().find_map(|i| i.period_end.clone().filter(|s| !s.is_empty())));

    if slots.len() == 1 && slots[0].expire.is_none() {
        slots[0].expire = plan_expire_chain.clone();
    }
    let (total, used, remaining, percent_used) = match slots.get(pri_idx) {
        Some(p) => (p.total, p.used, p.remaining, p.percent_used),
        None => {
            let p = items.iter().find_map(|i| i.percent_used);
            (None, None, None, p)
        }
    };

    QuotaOverview {
        total,
        used,
        remaining,
        percent_used,
        plan_tier: slots.get(pri_idx).and_then(|p| p.tier.clone()),
        plan_expire: slots.get(pri_idx).and_then(|p| p.expire.clone()).or(plan_expire_chain),
        is_empty: balance.get("balances").and_then(|b| b.as_array()).map(|a| a.is_empty()).unwrap_or(false),
        items,
        refreshed_at: 0,
        source: String::new(),
        plans: slots,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn candidate_token_priority() {
        let s = "sec";
        let enc = |p: &str| zcrypto::encrypt_with_secret(p, s).unwrap();
        let creds = json!({
            "zcodejwttoken": enc("jwt-token-aaaaaaaaaaaaaaaaaaaaaa"),
            "oauth:active_provider": enc("bigmodel"),
            "oauth:bigmodel:access_token": enc("bigmodel-token-bbbbbbbbbbbbbbb"),
        });
        let config = json!({
            "provider": {
                "builtin:bigmodel-coding-plan": { "enabled": true, "options": { "apiKey": "api-key-derived-ccccccccc" } },
                "builtin:bigmodel": { "enabled": true, "options": { "apiKey": "not-coding-plan-ddddddddd" } },
                "enc-entry": { "options": { "apiKey": "enc:v1:short" } },
            }
        });
        let t = candidate_tokens(&creds, Some(&config), s);
        assert_eq!(t[0], "api-key-derived-ccccccccc", "coding-plan apiKey 必须第一");
        assert_eq!(t[1], "jwt-token-aaaaaaaaaaaaaaaaaaaaaa");
        assert!(t.contains(&"bigmodel-token-bbbbbbbbbbbbbbb".to_string()));
        assert!(!t.iter().any(|x| x.contains("not-coding-plan")), "非 coding-plan apiKey 不参与");
        assert!(!t.iter().any(|x| x.starts_with("enc:v1")));
    }

    #[test]
    fn normalize_quota_limit_real_shape() {
        let limit = json!({
            "code": 200, "msg": "操作成功", "success": true,
            "data": {
                "level": "max",
                "limits": [
                    { "type": "TOKENS_LIMIT", "unit": 3, "number": 5, "percentage": 3, "nextResetTime": 1787389544700i64 },
                    { "type": "TOKENS_LIMIT", "unit": 6, "number": 1, "percentage": 2, "nextResetTime": 1787849868998i64 },
                    { "type": "TIME_LIMIT", "unit": 5, "number": 1, "percentage": 1, "usage": 4000, "currentValue": 71, "remaining": 3929, "nextResetTime": 1788109068998i64 },
                ]
            }
        });
        let sub = json!({ "code": 200, "success": true, "data": [
            { "productId": "x", "productName": "GLM Coding Max", "status": "VALID" }
        ]});
        let ov = normalize_quota_limit(&limit, Some(&sub));
        assert_eq!(ov.plan_tier.as_deref(), Some("Max"));
        assert_eq!(ov.items.len(), 3);
        assert_eq!(ov.total, Some(4000.0));
        assert_eq!(ov.used, Some(71.0));
        assert_eq!(ov.remaining, Some(3929.0));
        assert!((ov.percent_used.unwrap() - 1.775).abs() < 0.01);
        assert_eq!(ov.items[0].percent_used, Some(3.0));
        assert_eq!(ov.items[1].percent_used, Some(2.0));
        assert!(ov.items[0].name.contains("提示次数"));
        assert!(ov.items[1].name.contains("每周"));
        assert!(ov.items[2].period_end.as_deref().unwrap_or("").contains("重置"));
        let precise = ov.items[2].percent_used.unwrap();
        assert_eq!(precise.floor(), ov.items[2].server_percentage.unwrap());
    }

    #[test]
    fn normalize_quota_limit_empty_and_no_sub() {
        let limit = json!({ "code": 200, "success": true, "data": { "level": "pro", "limits": [] } });
        let ov = normalize_quota_limit(&limit, None);
        assert!(ov.is_empty);
        assert_eq!(ov.plan_tier.as_deref(), Some("Pro"));
        assert!(ov.total.is_none());
    }

    #[test]
    fn business_envelope_detect() {
        assert!(business_ok(&json!({"code": 200, "success": true})));
        assert!(business_ok(&json!({})));
        assert!(!business_ok(&json!({"code": 401, "success": false})));
        assert!(!business_ok(&json!({"code": 200, "success": false})));
    }

    #[test]
    fn normalize_balance_fallback() {
        let b = json!({ "data": { "balances": [
            { "show_name": "GLM-5.3", "total_units": 600, "used_units": 150, "remaining_units": 450 },
        ], "plans": [ { "plan_id": "start-plan", "status": "active" } ]}});
        let ov = normalize_balance(&b);
        assert_eq!(ov.total, Some(600.0));
        assert_eq!(ov.remaining, Some(450.0));
        assert_eq!(ov.plan_tier.as_deref(), Some("Start Plan"));
        assert_eq!(ov.items.len(), 1);
    }

    #[test]
    fn balance_multi_plan_groups_slots_by_plan_id() {
        let b = json!({ "data": {
            "balances": [
                { "show_name": "GLM-4.7", "plan_id": "zcode-v3-start-plan-0821", "total_units": 1000, "used_units": 100, "remaining_units": 900 },
                { "show_name": "GLM-5.3-Flash", "plan_id": "zcode-v3-trial-0926", "total_units": 300000000u64, "used_units": 0, "remaining_units": 300000000u64 },
            ],
            "plans": [
                { "plan_id": "zcode-v3-start-plan-0821", "name": "GLM Coding Max", "status": "active", "expireTime": 1810000000000i64 },
                { "plan_id": "zcode-v3-trial-0926", "name": "体验套餐", "status": "active", "expireTime": 1789000000000i64 },
            ]}});
        let ov = normalize_balance(&b);
        assert_eq!(ov.plans.len(), 2, "个人 + 体验 双切片");
        assert_eq!(ov.plan_tier.as_deref(), Some("Max"));
        assert_eq!(ov.total, Some(1000.0));
        let max = ov.plans.iter().find(|s| s.tier.as_deref() == Some("Max")).unwrap();
        assert_eq!(max.name.as_deref(), Some("GLM Coding Max"));
        assert_eq!(max.items.len(), 1);
        assert_eq!(max.items[0].name, "GLM-4.7");
        assert!(max.expire.is_some());
        let trial = ov.plans.iter().find(|s| s.tier.as_deref() == Some("体验")).unwrap();
        assert_eq!(trial.items.len(), 1);
        assert_eq!(trial.items[0].name, "GLM-5.3-Flash");
        assert_eq!(trial.total, Some(300000000.0));
        assert_eq!(trial.percent_used, Some(0.0));
        assert_eq!(ov.items.len(), 2);
    }

    #[test]
    fn balance_plan_skeleton_without_balances_still_listed() {
        let b = json!({ "data": {
            "balances": [],
            "plans": [ { "plan_id": "zcode-v3-trial-0926", "name": "体验套餐", "status": "active" } ]}});
        let ov = normalize_balance(&b);
        assert_eq!(ov.plans.len(), 1);
        assert_eq!(ov.plans[0].tier.as_deref(), Some("体验"));
        assert!(ov.items.is_empty());
    }

    #[test]
    fn same_channel_same_named_plans_not_deduped() {
        let fetch = |url: &str, _t: &str| -> Result<Value, String> {
            assert!(url.contains("zcode.z.ai"));
            Ok(json!({ "code": 0, "data": {
                "balances": [
                    { "show_name": "GLM-5.3-Flash", "plan_id": "trial-a", "total_units": 100, "used_units": 0, "remaining_units": 100 },
                    { "show_name": "GLM-5.3-Flash", "plan_id": "trial-b", "total_units": 200, "used_units": 0, "remaining_units": 200 },
                ],
                "plans": [
                    { "plan_id": "trial-a", "name": "体验套餐", "status": "active" },
                    { "plan_id": "trial-b", "name": "体验套餐", "status": "active" },
                ]
            }}))
        };
        let ov = query_channels_via(&[Channel::ZaiBilling("j".into())], &fetch).unwrap();
        assert_eq!(ov.plans.len(), 2, "同通道同名切片必须都保留");
        assert_eq!(ov.items.len(), 2);
        assert_eq!(ov.items[0].total, Some(100.0));
        assert_eq!(ov.items[1].total, Some(200.0));
    }

    #[test]
    fn unmatched_balances_get_trailing_slot() {
        let b = json!({ "data": {
            "balances": [
                { "show_name": "GLM-4.7", "plan_id": "p-max", "total_units": 1000, "used_units": 100, "remaining_units": 900 },
                { "show_name": "GLM-orphan", "total_units": 50, "used_units": 0, "remaining_units": 50 },
            ],
            "plans": [ { "plan_id": "p-max", "name": "GLM Coding Max", "status": "active" } ]}});
        let ov = normalize_balance(&b);
        assert_eq!(ov.plans.len(), 2);
        let orphan = ov.plans.last().unwrap();
        assert_eq!(orphan.name.as_deref(), Some("其他额度"));
        assert_eq!(orphan.total, Some(50.0));
        assert_eq!(ov.plan_tier.as_deref(), Some("Max"));
        assert_eq!(ov.total, Some(1000.0));
        assert_eq!(ov.items.len(), 2, "顶层 items = 全部切片合计");
    }

    #[test]
    fn normalize_quota_limit_emits_single_slot() {
        let limit = json!({ "code": 200, "data": { "level": "max", "limits": [
            { "type": "TIME_LIMIT", "unit": 5, "percentage": 1, "usage": 4000, "currentValue": 71, "remaining": 3929 },
        ]}});
        let ov = normalize_quota_limit(&limit, None);
        assert_eq!(ov.plans.len(), 1);
        assert_eq!(ov.plans[0].tier.as_deref(), Some("Max"));
        assert_eq!(ov.plans[0].total, Some(4000.0));
        assert_eq!(ov.plans[0].items.len(), 1);
    }

    #[test]
    fn dual_channel_merges_monitor_and_zai_slots() {
        let fetch = |url: &str, _t: &str| -> Result<Value, String> {
            if url.ends_with("/quota/limit") {
                Ok(json!({ "code": 200, "data": { "level": "max", "limits": [
                    { "type": "TIME_LIMIT", "unit": 5, "percentage": 1, "usage": 4000, "currentValue": 71, "remaining": 3929 },
                ]}}))
            } else if url.contains("subscription") {
                Ok(json!({ "code": 200, "data": [ { "productName": "GLM Coding Max", "status": "VALID" } ]}))
            } else {
                Ok(json!({ "code": 0, "data": { "balances": [
                    { "show_name": "GLM-5.3-Flash", "total_units": 300000000u64, "used_units": 0, "remaining_units": 300000000u64 },
                ], "plans": [ { "plan_id": "zcode-v3-trial-0926", "status": "active" } ]}}))
            }
        };
        let ov = query_channels_via(
            &[Channel::Monitor("k".into()), Channel::ZaiBilling("j".into())],
            &fetch,
        )
        .unwrap();
        assert_eq!(ov.plans.len(), 2, "monitor 切片 + z.ai 切片都要在");
        assert_eq!(ov.plan_tier.as_deref(), Some("Max"));
        assert_eq!(ov.used, Some(71.0), "顶层 = 主切片（个人订阅）");
        let trial = ov.plans.iter().find(|s| s.tier.as_deref() == Some("体验")).unwrap();
        assert_eq!(trial.total, Some(300000000.0));
        assert!(ov.source.contains("bigmodel") && ov.source.contains("z.ai"));
    }

    #[test]
    fn zai_no_plan_side_drops_empty_slot() {
        let fetch = |url: &str, _t: &str| -> Result<Value, String> {
            if url.ends_with("/quota/limit") {
                Ok(json!({ "code": 200, "data": { "level": "pro", "limits": [
                    { "type": "TIME_LIMIT", "unit": 5, "percentage": 1, "usage": 100, "currentValue": 1, "remaining": 99 },
                ]}}))
            } else if url.contains("subscription") {
                Ok(json!({ "code": 200, "data": [] }))
            } else {
                Ok(json!({ "code": 0, "data": { "balances": [], "plans": null } }))
            }
        };
        let ov = query_channels_via(
            &[Channel::Monitor("k".into()), Channel::ZaiBilling("j".into())],
            &fetch,
        )
        .unwrap();
        assert_eq!(ov.plans.len(), 1);
        assert_eq!(ov.plan_tier.as_deref(), Some("Pro"));
    }

    #[test]
    fn start_plan_token_falls_through_monitor_401_to_zai() {
        let fetch = |url: &str, _tok: &str| -> Result<Value, String> {
            if url.contains("bigmodel.cn") {
                Ok(json!({ "code": 401, "msg": "令牌已过期或验证不正确" }))
            } else {
                Ok(json!({ "code": 0, "data": { "balances": [
                    { "show_name": "ZCode Weekend Build", "total_units": 100, "used_units": 10, "remaining_units": 90 },
                ], "plans": [ { "plan_id": "zcode-v3-start-plan-0821", "status": "active" } ]}}))
            }
        };
        let ov = query_with_token_via("tok-start-plan", &fetch).unwrap();
        assert_eq!(ov.source, "zcode.z.ai/billing");
        assert_eq!(ov.plan_tier.as_deref(), Some("Start Plan"));
        assert_eq!(ov.total, Some(100.0));
    }

    #[test]
    fn business_error_not_masked_by_zai_401() {
        let fetch = |url: &str, _tok: &str| -> Result<Value, String> {
            if url.contains("bigmodel.cn") {
                Ok(json!({ "code": 500, "msg": "当前用户不存在coding plan" }))
            } else {
                Err("Token 已过期或无效（HTTP 401）".into())
            }
        };
        let err = query_with_token_via("tok-cp-key", &fetch).unwrap_err();
        assert!(err.contains("不存在coding plan"), "got: {err}");
        assert!(!err.contains("HTTP 401"), "端点不匹配的 401 不应掩盖业务错: {err}");
    }

    #[test]
    fn monitor_success_short_circuits() {
        let fetch = |url: &str, _tok: &str| -> Result<Value, String> {
            if url.ends_with("/quota/limit") {
                Ok(json!({ "code": 200, "data": { "level": "max", "limits": [
                    { "type": "TIME_LIMIT", "unit": 5, "percentage": 1, "usage": 4000, "currentValue": 71, "remaining": 3929 },
                ]}}))
            } else if url.contains("subscription") {
                Ok(json!({ "code": 200, "data": [ { "productName": "GLM Coding Max", "status": "VALID" } ]}))
            } else {
                panic!("z.ai 不应被调用");
            }
        };
        let ov = query_with_token_via("tok-cp", &fetch).unwrap();
        assert_eq!(ov.source, "bigmodel.cn/api/monitor");
        assert_eq!(ov.plan_tier.as_deref(), Some("Max"));
        assert_eq!(ov.used, Some(71.0));
    }

    fn enc_creds(s: &str) -> Value {
        let e = |p: &str| zcrypto::encrypt_with_secret(p, s).unwrap();
        json!({
            "zcodejwttoken": e("start-plan-jwt-eeeeeeeeeeeeeeee"),
            "oauth:active_provider": e("bigmodel"),
        })
    }

    #[test]
    fn pick_channels_start_plan_enabled_prefers_zai_billing() {
        let s = "sec";
        let creds = enc_creds(s);
        let config = json!({ "provider": {
            "builtin:bigmodel": { "enabled": true, "options": { "apiKey": "shared-key-not-channel-related-aaaa" } },
            "builtin:bigmodel-start-plan": { "enabled": true, "options": { "apiKey": "start-key-ffffffffffffffff" } },
            "builtin:bigmodel-coding-plan": { "enabled": false, "options": { "apiKey": "cp-key-gggggggggggggggg" } },
        }});
        let ch = pick_channels(&creds, Some(&config), s);
        assert_eq!(ch[0], Channel::ZaiBilling("start-plan-jwt-eeeeeeeeeeeeeeee".into()));
        assert!(!ch.iter().any(|c| matches!(c, Channel::Monitor(k) if k.contains("shared-key"))));
        assert_eq!(ch[1], Channel::Monitor("cp-key-gggggggggggggggg".into()));
    }

    #[test]
    fn pick_channels_coding_plan_enabled_prefers_monitor() {
        let s = "sec";
        let creds = enc_creds(s);
        let config = json!({ "provider": {
            "builtin:bigmodel-coding-plan": { "enabled": true, "options": { "apiKey": "cp-key-hhhhhhhhhhhhhhhh" } },
        }});
        let ch = pick_channels(&creds, Some(&config), s);
        assert_eq!(ch, vec![Channel::Monitor("cp-key-hhhhhhhhhhhhhhhh".into())]);
    }

    #[test]
    fn zai_billing_channel_hits_balance_exactly_once() {
        let calls = std::cell::RefCell::new(vec![]);
        let fetch = |url: &str, tok: &str| -> Result<Value, String> {
            calls.borrow_mut().push((url.to_string(), tok.to_string()));
            Ok(json!({ "code": 0, "data": { "balances": [
                { "show_name": "GLM-5.3", "total_units": 100, "used_units": 10, "remaining_units": 90 },
            ], "plans": [ { "plan_id": "zcode-v3-start-plan-0821", "status": "active" } ]}}))
        };
        let ov = query_channels_via(&[Channel::ZaiBilling("tok".into())], &fetch).unwrap();
        assert_eq!(ov.source, "zcode.z.ai/billing");
        assert_eq!(ov.plan_tier.as_deref(), Some("Start Plan"));
        assert_eq!(calls.borrow().len(), 1, "start-plan 通道必须单次命中");
        assert!(calls.borrow()[0].0.contains("zcode.z.ai"));
    }

    #[test]
    fn monitor_channel_never_touches_zai_on_success() {
        let calls = std::cell::RefCell::new(vec![]);
        let fetch = |url: &str, _tok: &str| -> Result<Value, String> {
            calls.borrow_mut().push(url.to_string());
            if url.ends_with("/quota/limit") {
                Ok(json!({ "code": 200, "data": { "level": "max", "limits": [
                    { "type": "TIME_LIMIT", "unit": 5, "percentage": 1, "usage": 4000, "currentValue": 71, "remaining": 3929 },
                ]}}))
            } else {
                Ok(json!({ "code": 200, "data": [] }))
            }
        };
        let ov = query_channels_via(&[Channel::Monitor("k".into())], &fetch).unwrap();
        assert_eq!(ov.source, "bigmodel.cn/api/monitor");
        let all = calls.borrow();
        assert!(all.iter().all(|u| u.contains("bigmodel.cn")), "不得触碰 z.ai: {all:?}");
        assert_eq!(all.len(), 2, "monitor + subscription 各一次");
    }

    #[test]
    fn no_plan_message_is_state_not_error() {
        let fetch = |_url: &str, _tok: &str| -> Result<Value, String> {
            Ok(json!({ "code": 500, "msg": "当前用户不存在coding plan" }))
        };
        let ov = query_channels_via(&[Channel::Monitor("k".into())], &fetch).unwrap();
        assert_eq!(ov.source, "no_plan");
        assert!(ov.is_empty);
        assert!(ov.plan_tier.is_none());
    }

    #[test]
    fn zai_billing_headers_match_expected_identity_set() {
        let h = zai_billing_headers("tok-start");
        let keys: Vec<&str> = h.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys[..9],
            ["User-Agent", "HTTP-Referer", "X-Title", "X-ZCode-App-Version",
             "X-Platform", "X-Release-Channel", "X-Client-Language",
             "X-Client-Timezone", "X-Os-Category"]
        );
        assert_eq!(h[0].1, format!("ZCode/{}", zcode_app_version()));
        assert_eq!(h[1].1, "https://zcode.z.ai");
        assert_eq!(h[2].1, "Z Code@electron");
        assert_eq!(h[4].1, client_platform());
        assert_eq!(h[5].1, "stable");
        assert_eq!(h[6].1, "zh-CN");
        let os_cat = h.iter().find(|(k, _)| k == "X-Os-Category").unwrap();
        assert_eq!(os_cat.1, std::env::consts::OS);
        let auth = h.iter().find(|(k, _)| k == "Authorization").unwrap();
        assert_eq!(auth.1, "Bearer tok-start");
        let rid = h.iter().find(|(k, _)| k == "x-request-id").unwrap();
        assert_eq!(rid.1.len(), 36, "uuid v4 形");
        assert_eq!(rid.1.as_bytes()[14], b'4');
        if let Some(pos) = keys.iter().position(|k| *k == "X-Device-Mid") {
            assert_eq!(keys[pos + 1], "Authorization");
        }
    }

    #[test]
    fn bigmodel_headers_minimal_with_request_id() {
        let h = bigmodel_headers("tok-cp");
        let keys: Vec<&str> = h.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["Authorization", "User-Agent", "x-request-id"]);
        assert_eq!(h[0].1, "Bearer tok-cp");
        assert_eq!(h[1].1, format!("ZCode/{}", zcode_app_version()));
        assert_eq!(h[2].1.as_bytes()[14], b'4');
    }

    #[test]
    fn extract_expire_prefers_next_renew_and_handles_valid_range() {
        let sub = json!({ "nextRenewTime": "2027-07-28", "valid": "2027-07-28 10:00:00-2028-07-28 10:00:00" });
        assert_eq!(extract_expire(&sub).as_deref(), Some("2027-07-28"));
        let sub2 = json!({ "valid": "2027-07-28 10:00:00-2028-07-28 10:00:00" });
        assert_eq!(extract_expire(&sub2).as_deref(), Some("2028-07-28"));
        let sub3 = json!({ "expireTime": null, "endTime": "2026-12-31 00:00:00" });
        assert_eq!(extract_expire(&sub3).as_deref(), Some("2026-12-31"));
    }

    #[test]
    fn extract_expire_survives_multibyte_garbage() {
        let bad = json!({ "nextRenewTime": "2026-08-中文字符串拼接很长很长很长" });
        assert!(extract_expire(&bad).is_some());
        let bad2 = json!({ "valid": "2027-07-28 10:00:00-2028-07月中尾巴很长很长的字符串" });
        let _ = extract_expire(&bad2);
        let bad3 = json!({ "valid": "中-文-乱码输入测试数据" });
        let _ = extract_expire(&bad3);
    }

    #[test]
    fn balance_plan_expire_takes_max_bucket_expiry() {
        let bal = json!({ "data": { "balances": [
            { "show_name": "GLM-5.3", "total_units": 100, "used_units": 10, "expires_at": 1787414399 },
            { "show_name": "GLM-5.3", "total_units": 50, "used_units": 0, "expires_at": 1787533200 }
        ] } });
        let ov = normalize_balance(&bal);
        assert_eq!(ov.plan_expire.as_deref(), Some("2026-08-24"));
    }
}
