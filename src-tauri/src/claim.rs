
use crate::quota;
use crate::zcrypto;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

pub const BILLING_PREVIEW_URL: &str = "https://zcode.z.ai/api/v1/zcode-plan/billing/preview";
pub const BILLING_CLAIM_URL: &str = "https://zcode.z.ai/api/v1/zcode-plan/billing/claim";
pub const CLIENT_CONFIGS_URL: &str = "https://zcode.z.ai/api/v1/client/configs";

const CLAIM_TIMEOUT_SECS: u64 = 25;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ClaimGrant {
    pub name: String,
    pub units: f64,
    pub period: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ClaimPlan {
    pub plan_id: String,
    pub name: String,
    pub description: String,
    pub priority: i64,
    pub grants: Vec<String>,
    #[serde(default)]
    pub grant_items: Vec<ClaimGrant>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CaptchaConfig {
    pub enabled: bool,
    pub region: String,
    pub prefix: String,
    pub scene_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimOutcome {
    pub account_id: String,
    pub account_name: String,
    pub plan_name: String,
    pub starts_at: Option<i64>,
    pub ends_at: Option<i64>,
    pub server_time: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ClaimError {
    pub code: i64,
    pub message: String,
    pub next_at: Option<i64>,
}

impl From<String> for ClaimError {
    fn from(message: String) -> Self {
        ClaimError {
            code: -1,
            message,
            next_at: None,
        }
    }
}

fn claim_error(code: i64, body: &Value) -> ClaimError {
    let next_at = if code == 1005 {
        body.pointer("/data/plan/ends_at")
            .and_then(|x| x.as_i64())
            .map(|s| s * 1000)
    } else {
        None
    };
    ClaimError {
        code,
        message: failure_message(code, body),
        next_at,
    }
}

pub fn failure_payload(
    account_id: &str,
    account_name: &str,
    plan_name: &str,
    err: &ClaimError,
) -> Value {
    serde_json::json!({
        "ok": false,
        "accountId": account_id,
        "accountName": account_name,
        "planName": plan_name,
        "code": err.code,
        "nextAt": err.next_at,
        "message": err.message,
    })
}

fn claim_token(creds: &Value, config: Option<&Value>, secret: &str) -> Result<String, String> {
    let jwt = creds
        .get("zcodejwttoken")
        .and_then(|v| v.as_str())
        .and_then(|v| decrypt_credential(v, secret));
    if let Some(t) = jwt.filter(|t| t.trim().len() > 20) {
        return Ok(t);
    }
    if let Some(t) = quota::zai_billing_token(creds, config, secret) {
        return Ok(t);
    }
    Err(crate::i18n::tr("err.claim.no_jwt"))
}

fn decrypt_credential(v: &str, secret: &str) -> Option<String> {
    if zcrypto::is_encrypted(v) {
        zcrypto::decrypt_with_secret(v, secret).ok()
    } else {
        Some(v.to_string())
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(CLAIM_TIMEOUT_SECS))
        .build()
}

pub fn preview_plans(
    home: &Path,
    creds: &Value,
    config: Option<&Value>,
    device_mid: Option<String>,
) -> Result<Vec<ClaimPlan>, String> {
    let secret = zcrypto::default_secret(home);
    let token = claim_token(creds, config, &secret)?;
    let url = format!(
        "{BILLING_PREVIEW_URL}?app_version={}&platform={}",
        quota::zcode_app_version(),
        quota::client_platform()
    );
    preview_once(&url, &token, device_mid)
}

fn http_err(prefix: &str, e: ureq::Error) -> String {
    if let ureq::Error::Status(code, resp) = e {
        let body = resp.into_string().unwrap_or_default();
        let msg = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| ["msg", "message", "error"]
                .iter()
                .find_map(|k| v.get(k).and_then(|x| x.as_str()).map(String::from)))
            .unwrap_or_default();
        return format!("{prefix} HTTP {code}: {msg}");
    }
    format!("{prefix} {e}")
}

fn preview_once(url: &str, token: &str, mid: Option<String>) -> Result<Vec<ClaimPlan>, String> {
    let mut req = agent().get(url);
    for (k, v) in quota::zai_billing_headers_with_mid(token, mid) {
        req = req.set(&k, &v);
    }
    let resp = req
        .call()
        .map_err(|e| http_err(&crate::i18n::tr("err.claim.preview_req"), e))?
        .into_string()
        .map_err(|e| crate::i18n::trf("err.http.read", &[("e", &e.to_string())]))?;
    let v: Value = serde_json::from_str(&resp).unwrap_or(Value::String(resp));
    let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
    if code != 0 {
        return Err(failure_message(code, &v));
    }
    let plans = v
        .pointer("/data/plans")
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out: Vec<ClaimPlan> = plans.iter().filter_map(parse_plan).collect();
    out.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.plan_id.cmp(&b.plan_id)));
    Ok(out)
}

pub fn submit_claim(
    home: &Path,
    creds: &Value,
    config: Option<&Value>,
    plan_id: &str,
    captcha_param: &str,
    captcha_region: Option<&str>,
    device_mid: Option<String>,
) -> Result<Value, ClaimError> {
    if captcha_param.trim().is_empty() {
        return Err(crate::i18n::tr("err.claim.no_captcha").into());
    }
    let secret = zcrypto::default_secret(home);
    let token = claim_token(creds, config, &secret)?;
    let mut req = agent().post(BILLING_CLAIM_URL);
    for (k, v) in quota::zai_billing_headers_with_mid(&token, device_mid) {
        req = req.set(&k, &v);
    }
    req = req.set("X-Aliyun-Captcha-Verify-Param", captcha_param.trim());
    if let Some(r) = captcha_region.filter(|r| !r.trim().is_empty()) {
        req = req.set("X-Aliyun-Captcha-Verify-Region", r.trim());
    }
    let resp = req
        .send_json(serde_json::json!({ "plan_id": plan_id }))
        .map_err(|e| ClaimError {
            code: -1,
            message: http_err(&crate::i18n::tr("err.claim.claim_req"), e),
            next_at: None,
        })?
        .into_string()
        .map_err(|e| ClaimError {
            code: -1,
            message: crate::i18n::trf("err.http.read", &[("e", &e.to_string())]),
            next_at: None,
        })?;
    let v: Value = serde_json::from_str(&resp).unwrap_or(Value::String(resp));
    let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
    if code != 0 {
        return Err(claim_error(code, &v));
    }
    Ok(v)
}

pub fn fetch_captcha_config() -> Result<CaptchaConfig, String> {
    let mut req = agent().get(CLIENT_CONFIGS_URL);
    for (k, v) in quota::zai_billing_headers("") {
        req = req.set(&k, &v);
    }
    let resp = req
        .call()
        .map_err(|e| crate::i18n::trf("err.claim.config_req", &[("e", &e.to_string())]))?
        .into_string()
        .map_err(|e| crate::i18n::trf("err.http.read", &[("e", &e.to_string())]))?;
    let v: Value = serde_json::from_str(&resp).unwrap_or(Value::String(resp));
    if v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1) != 0 {
        return Err(crate::i18n::tr("err.claim.config_unavailable"));
    }
    let c = v.pointer("/data/configs/captcha").cloned().unwrap_or(Value::Null);
    let s = |k: &str| c.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
    Ok(CaptchaConfig {
        enabled: c.get("enabled").and_then(|x| x.as_bool()).unwrap_or(false),
        region: s("region"),
        prefix: s("prefix"),
        scene_id: s("sceneId"),
    })
}

fn str_field<'a>(e: &'a Value, snake: &str, camel: &str) -> Option<&'a str> {
    e.get(snake)
        .and_then(|v| v.as_str())
        .or_else(|| e.get(camel).and_then(|v| v.as_str()))
}

fn num_field(e: &Value, snake: &str, camel: &str) -> Option<f64> {
    e.get(snake)
        .and_then(|v| v.as_f64())
        .or_else(|| e.get(camel).and_then(|v| v.as_f64()))
}

fn parse_plan(p: &Value) -> Option<ClaimPlan> {
    let plan_id = str_field(p, "plan_id", "planId")?.trim().to_string();
    if plan_id.is_empty() {
        return None;
    }
    let grant_items: Vec<ClaimGrant> = p
        .get("entitlements")
        .and_then(|e| e.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|e| {
                    str_field(e, "meter", "meter") == Some("model_usage")
                        && str_field(e, "unit_type", "unitType") == Some("token")
                        && str_field(e, "show_name", "showName")
                            .map(|s| !s.trim().is_empty())
                            .unwrap_or(false)
                })
                .map(|e| ClaimGrant {
                    name: str_field(e, "show_name", "showName").unwrap_or("").to_string(),
                    units: num_field(e, "grant_units", "grantUnits").unwrap_or(0.0),
                    period: str_field(e, "period", "period").unwrap_or("one_time").to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    let grants = grant_items
        .iter()
        .map(|g| {
            let period_cn = match g.period.as_str() {
                "daily" => "每日",
                "weekly" => "每周",
                "monthly" => "每月",
                _ => "一次性",
            };
            format!("{} · {} Token（{period_cn}）", g.name, fmt_units(g.units))
        })
        .collect();
    Some(ClaimPlan {
        name: p.get("name").and_then(|s| s.as_str()).unwrap_or("").trim().to_string(),
        description: p
            .get("description")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .trim()
            .to_string(),
        priority: p.get("priority").and_then(|x| x.as_i64()).unwrap_or(0),
        plan_id,
        grants,
        grant_items,
    })
}

fn fmt_units(n: f64) -> String {
    let trim = |x: f64| {
        let r = (x * 10.0).round() / 10.0;
        if (r - r.trunc()).abs() < f64::EPSILON {
            format!("{}", r.trunc() as i64)
        } else {
            format!("{r:.1}")
        }
    };
    if n >= 1e8 {
        format!("{}亿", trim(n / 1e8))
    } else if n >= 1e4 {
        format!("{}万", trim(n / 1e4))
    } else {
        format!("{}", n.round() as i64)
    }
}

pub fn failure_message(code: i64, body: &Value) -> String {
    let server_msg = ["msg", "message"]
        .iter()
        .find_map(|k| body.get(k).and_then(|x| x.as_str()).map(String::from))
        .unwrap_or_default();
    let base = crate::i18n::tr(match code {
        1001 => "claim.fail.1001",
        1002 => "claim.fail.1002",
        1003 => "claim.fail.1003",
        1004 => "claim.fail.1004",
        1005 => "claim.fail.1005",
        3001 => "claim.fail.3001",
        3007 => "claim.fail.3007",
        401 => "claim.fail.401",
        _ => "claim.fail.generic",
    });
    if server_msg.is_empty() {
        base
    } else {
        crate::i18n::trf("claim.fail.with_server", &[("base", &base), ("server_msg", &server_msg)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_plan_extracts_grants() {
        let v = json!({
            "plan_id": "zcode-v3-start-plan-0828",
            "name": "ZCode Weekend Build",
            "description": "ZCode 周末活动",
            "priority": 100,
            "entitlements": [
                { "entitlement_id": "e1", "show_name": "GLM-5.3-Flash", "meter": "model_usage",
                  "unit_type": "token", "grant_units": 300000000, "period": "one_time" },
                { "entitlement_id": "e2", "show_name": "噪音", "meter": "other",
                  "unit_type": "token", "grant_units": 1, "period": "one_time" }
            ]
        });
        let p = parse_plan(&v).unwrap();
        assert_eq!(p.plan_id, "zcode-v3-start-plan-0828");
        assert_eq!(p.priority, 100);
        assert_eq!(p.grants, vec!["GLM-5.3-Flash · 3亿 Token（一次性）".to_string()]);
        assert_eq!(p.grant_items.len(), 1);
        assert_eq!(p.grant_items[0].name, "GLM-5.3-Flash");
        assert_eq!(p.grant_items[0].units, 300000000.0);
        assert_eq!(p.grant_items[0].period, "one_time");
    }

    #[test]
    fn parse_plan_accepts_camel_case() {
        let v = json!({
            "planId": "p1",
            "name": "N",
            "entitlements": [
                { "showName": "GLM-5", "meter": "model_usage",
                  "unitType": "token", "grantUnits": 5000000, "period": "daily" }
            ]
        });
        let p = parse_plan(&v).unwrap();
        assert_eq!(p.plan_id, "p1");
        assert_eq!(p.grants, vec!["GLM-5 · 500万 Token（每日）".to_string()]);
    }

    #[test]
    fn parse_plan_skips_empty_plan_id() {
        let v = json!({ "plan_id": "  ", "name": "x" });
        assert!(parse_plan(&v).is_none());
    }

    #[test]
    fn preview_sorts_by_priority_desc() {
        let raw = json!({
            "code": 0,
            "data": { "plans": [
                { "plan_id": "b", "name": "B", "priority": 1 },
                { "plan_id": "a", "name": "A", "priority": 100 }
            ] }
        });
        let plans = raw.pointer("/data/plans").unwrap().as_array().unwrap();
        let mut out: Vec<ClaimPlan> = plans.iter().filter_map(parse_plan).collect();
        out.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.plan_id.cmp(&b.plan_id)));
        assert_eq!(out[0].plan_id, "a");
    }

    #[test]
    fn failure_messages_match_client_i18n() {
        assert_eq!(failure_message(1003, &json!({})), "该套餐已经领取过");
        assert_eq!(failure_message(1005, &json!({"msg": "quota"})), "今日领取名额已用完（quota）");
        assert_eq!(failure_message(3007, &json!({})), "验证码校验失败，请重试");
        assert_eq!(failure_message(401, &json!({})), "请先登录后再领取");
    }

    #[test]
    fn fmt_units_round_numbers() {
        assert_eq!(fmt_units(300000000.0), "3亿");
        assert_eq!(fmt_units(5000000.0), "500万");
        assert_eq!(fmt_units(1234.0), "1234");
        assert_eq!(fmt_units(150000000.0), "1.5亿");
        assert_eq!(fmt_units(15000.0), "1.5万");
    }

    #[test]
    fn captcha_config_parses() {
        let v = json!({ "data": { "configs": { "captcha": {
            "enabled": true, "prefix": "no8xfe", "region": "cn", "sceneId": "11xygtvd"
        } } } });
        let c = v.pointer("/data/configs/captcha").unwrap();
        assert_eq!(c.get("sceneId").and_then(|x| x.as_str()), Some("11xygtvd"));
    }

    #[test]
    fn claim_outcome_serializes_camel_case() {
        let o = ClaimOutcome {
            account_id: "a1".into(),
            account_name: "n".into(),
            plan_name: "p".into(),
            starts_at: Some(1787918400_000),
            ends_at: Some(1788138000_000),
            server_time: Some(1787800000_000),
        };
        let v = serde_json::to_value(&o).unwrap();
        assert!(v.get("accountId").is_some(), "必须输出 camelCase accountId");
        assert!(v.get("accountName").is_some());
        assert!(v.get("planName").is_some());
        assert!(v.get("startsAt").is_some());
        assert!(v.get("endsAt").is_some());
        assert!(v.get("serverTime").is_some(), "3.11.2：server_time 必须随成功载荷下发");
        assert!(v.get("account_id").is_none(), "不得残留 snake_case 键");
        assert!(v.get("server_time").is_none(), "server_time 同样只出 camelCase");
    }

    #[test]
    fn claim_error_extracts_next_at_only_for_1005() {
        let body = json!({ "code": 1005, "msg": "quota", "data": { "plan": { "ends_at": 1787900000 } } });
        let e = claim_error(1005, &body);
        assert_eq!(e.code, 1005);
        assert_eq!(e.next_at, Some(1787900000_000));
        assert!(e.message.contains("名额已用完"));

        let e2 = claim_error(1003, &body);
        assert_eq!(e2.code, 1003);
        assert_eq!(e2.next_at, None);

        let e3 = claim_error(1005, &json!({ "code": 1005, "msg": "x" }));
        assert_eq!(e3.next_at, None);
    }

    #[test]
    fn from_string_maps_to_code_minus_one() {
        let e = ClaimError::from("网络炸了".to_string());
        assert_eq!(e.code, -1);
        assert_eq!(e.next_at, None);
        assert_eq!(e.message, "网络炸了");
        let p = failure_payload("a1", "n", "p", &e);
        assert_eq!(p["ok"], serde_json::json!(false));
        assert_eq!(p["code"], serde_json::json!(-1));
        assert_eq!(p["nextAt"], serde_json::Value::Null);
        assert_eq!(p["message"], serde_json::json!("网络炸了"));
    }

    #[test]
    fn failure_payload_serializes_camel_case() {
        let e = ClaimError {
            code: 1005,
            message: "今日领取名额已用完".into(),
            next_at: Some(1787900000_000),
        };
        let p = failure_payload("a1", "n", "p", &e);
        for k in ["ok", "accountId", "accountName", "planName", "code", "nextAt", "message"] {
            assert!(p.get(k).is_some(), "失败载荷缺 {k}");
        }
        assert!(p.get("account_id").is_none(), "不得残留 snake_case 键");
        assert!(p.get("next_at").is_none());
        assert_eq!(p["nextAt"], serde_json::json!(1787900000_000i64));
    }
}
