
use crate::quota;
use crate::zcrypto;
use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use uuid::Uuid;

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

#[cfg(windows)]
fn detached(mut c: std::process::Command) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    c.creation_flags(0x0000_0008 | 0x0000_0200);
    c
}
#[cfg(not(windows))]
fn detached(c: std::process::Command) -> std::process::Command {
    use std::process::Stdio;
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
}

pub struct Paths {
    pub home: PathBuf,
}

impl Paths {
    pub fn detect() -> Paths {
        let home = std::env::var("ZCODE_SWITCH_HOME")
            .map(PathBuf::from)
            .ok()
            .or_else(|| std::env::var("USERPROFILE").ok().map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."));
        Paths { home }
    }

    #[cfg(test)]
    pub fn new(home: impl AsRef<Path>) -> Paths {
        Paths { home: home.as_ref().to_path_buf() }
    }

    pub fn store_dir(&self) -> PathBuf { self.home.join(".zcode-switch") }
    pub fn accounts_dir(&self) -> PathBuf { self.store_dir().join("accounts") }
    pub fn settings_file(&self) -> PathBuf { self.store_dir().join("settings.json") }
    pub fn live_file(&self) -> PathBuf { self.home.join(".zcode").join("v2").join("credentials.json") }
    pub fn live_config(&self) -> PathBuf { self.home.join(".zcode").join("v2").join("config.json") }
    pub fn live_telemetry(&self) -> PathBuf { self.home.join(".zcode").join("v2").join("telemetry-state.json") }

    pub fn ensure_dirs(&self) -> Result<(), String> {
        fs::create_dir_all(self.accounts_dir()).map_err(|e| format!("无法创建账号库目录：{e}"))?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub hash: String,
    pub credentials: Value,
    #[serde(default)]
    pub config: Option<Value>,
    #[serde(default)]
    pub virtual_device_mid: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Settings {
    pub zcode_path: Option<String>,
    pub launch_after_switch: Option<bool>,
    pub close_to_tray: Option<bool>,
    #[serde(default)]
    pub hot_switch: Option<bool>,
    #[serde(default)]
    pub auth_proxy_on: Option<bool>,
    #[serde(default)]
    pub auth_proxy_url: Option<String>,
}

impl Settings {
    pub fn launch_after_switch(&self) -> bool { self.launch_after_switch.unwrap_or(true) }
    pub fn close_to_tray(&self) -> bool { self.close_to_tray.unwrap_or(true) }
    pub fn hot_switch(&self) -> bool { self.hot_switch.unwrap_or(false) }
    pub fn auth_proxy(&self) -> Option<&str> {
        if self.auth_proxy_on.unwrap_or(false) {
            self.auth_proxy_url.as_deref().map(str::trim).filter(|s| !s.is_empty())
        } else {
            None
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct AccountSummary {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub is_active: bool,
    pub has_config: bool,
    pub identity: zcrypto::Identity,
}

#[derive(Serialize, Clone, Debug)]
pub struct AppState {
    pub zcode_running: bool,
    pub live_exists: bool,
    pub live_logged_in: bool,
    pub live_hash: Option<String>,
    pub active_account_id: Option<String>,
    pub live_identity: Option<zcrypto::Identity>,
    pub accounts: Vec<AccountSummary>,
    pub zcode_path: String,
    pub zcode_path_ok: bool,
    pub store_dir: String,
    pub launch_after_switch: bool,
    pub close_to_tray: bool,
    pub hot_switch: bool,
    pub auth_proxy_on: bool,
    pub auth_proxy_url: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct SwitchResult {
    pub switched: bool,
    pub already_active: bool,
    pub name: String,
    pub preserved_as: Option<String>,
    pub killed: bool,
    pub launched: bool,
    pub hot: bool,
    #[serde(default)]
    pub config_stale: bool,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct ImportReport {
    pub picked: bool,
    pub added: Vec<String>,
    pub skipped: Vec<String>,
    pub errors: Vec<String>,
}

pub fn now_ts() -> String {
    Local::now().format("%Y-%m-%d %H:%M").to_string()
}

pub fn canonical_hash(v: &Value) -> String {
    let bytes = serde_json::to_vec(v).unwrap_or_default();
    let d = Sha256::digest(&bytes);
    format!("{d:x}")
}

pub fn is_logged_in(v: &Value) -> bool {
    let Some(map) = v.as_object() else { return false };
    if map.keys().any(|k| k.starts_with("oauth:") && k.ends_with(":access_token")) {
        return true;
    }
    map.get("zcodejwttoken")
        .and_then(|t| t.as_str())
        .map(|t| !t.trim().is_empty())
        .unwrap_or(false)
}

pub fn atomic_write(path: &Path, data: &str) -> Result<(), String> {
    let tmp = path.with_extension(format!("tmp-{}", Uuid::new_v4().simple()));
    fs::write(&tmp, data).map_err(|e| format!("写入失败 {}: {e}", path.display()))?;
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("落盘失败 {}: {e}", path.display()));
    }
    Ok(())
}

pub fn read_live(paths: &Paths) -> Result<Option<Value>, String> {
    if !paths.live_file().exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(paths.live_file())
        .map_err(|e| format!("无法读取 {}: {e}", paths.live_file().display()))?;
    let v: Value = serde_json::from_str(&raw)
        .map_err(|e| format!("{} 不是有效的 JSON：{e}", paths.live_file().display()))?;
    if !v.is_object() {
        return Err("credentials.json 内容不是对象".into());
    }
    Ok(Some(v))
}

pub fn read_live_config(paths: &Paths) -> Option<Value> {
    let raw = fs::read_to_string(paths.live_config()).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn write_live(paths: &Paths, v: &Value) -> Result<(), String> {
    if let Some(parent) = paths.live_file().parent() {
        fs::create_dir_all(parent).map_err(|e| format!("无法创建目录：{e}"))?;
    }
    let body = serde_json::to_string_pretty(v).unwrap_or_default() + "\n";
    atomic_write(&paths.live_file(), &body)
}

pub fn write_live_config(paths: &Paths, v: &Value) -> Result<(), String> {
    let body = serde_json::to_string_pretty(v).unwrap_or_default() + "\n";
    atomic_write(&paths.live_config(), &body)
}

fn in_sandbox() -> bool {
    std::env::var("ZCODE_SWITCH_HOME").is_ok()
}

pub fn zcode_running() -> bool {
    if in_sandbox() {
        return false;
    }
    let out = no_window("tasklist")
        .args(["/FI", "IMAGENAME eq ZCode.exe", "/FO", "CSV", "/NH"])
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .lines()
            .any(|l| l.to_lowercase().starts_with("\"zcode.exe\"")),
        Err(_) => false,
    }
}

pub fn kill_zcode() -> Result<bool, String> {
    if in_sandbox() {
        return Ok(true);
    }
    if !zcode_running() {
        return Ok(true);
    }
    let _ = no_window("taskkill")
        .args(["/F", "/IM", "ZCode.exe"])
        .output();
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        if !zcode_running() {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    Ok(!zcode_running())
}

pub fn launch_zcode(path: &str) -> Result<(), String> {
    if in_sandbox() {
        return Ok(());
    }
    let p = PathBuf::from(path);
    if !p.exists() {
        return Err(format!("ZCode 不存在：{path}（在设置里修改路径）"));
    }
    detached(Command::new(&p))
        .spawn()
        .map_err(|e| format!("启动失败：{e}"))?;
    Ok(())
}

pub fn load_settings(paths: &Paths) -> Settings {
    match fs::read_to_string(paths.settings_file()) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
            eprintln!("settings.json 损坏，已回退默认值：{e}");
            Settings::default()
        }),
        Err(_) => Settings::default(),
    }
}

pub fn save_settings(paths: &Paths, s: &Settings) -> Result<(), String> {
    paths.ensure_dirs()?;
    let body = serde_json::to_string_pretty(s).unwrap_or_default() + "\n";
    atomic_write(&paths.settings_file(), &body)
}

pub fn effective_zcode_path(paths: &Paths) -> (String, bool) {
    let s = load_settings(paths);
    if let Some(p) = s.zcode_path {
        let ok = PathBuf::from(&p).exists();
        return (p, ok);
    }
    let candidates = [
        r"C:\Program Files\ZCode\ZCode.exe".to_string(),
        std::env::var("LOCALAPPDATA")
            .map(|l| format!(r"{}\Programs\ZCode\ZCode.exe", l))
            .unwrap_or_default(),
    ];
    for c in &candidates {
        if !c.is_empty() && PathBuf::from(c).exists() {
            return (c.clone(), true);
        }
    }
    (candidates[0].clone(), false)
}

pub fn list_accounts(paths: &Paths) -> Result<Vec<Account>, String> {
    let dir = paths.accounts_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut out = vec![];
    for entry in fs::read_dir(&dir).map_err(|e| format!("读取账号库失败：{e}"))? {
        let entry = entry.map_err(|e| format!("读取账号库失败：{e}"))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(raw) = fs::read_to_string(&path) {
            if let Ok(a) = serde_json::from_str::<Account>(&raw) {
                out.push(a);
            }
        }
    }
    out.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.name.cmp(&b.name)));
    Ok(out)
}

pub fn save_account(paths: &Paths, acc: &Account) -> Result<(), String> {
    paths.ensure_dirs()?;
    let path = paths.accounts_dir().join(format!("{}.json", acc.id));
    let body = serde_json::to_string_pretty(acc).unwrap_or_default() + "\n";
    atomic_write(&path, &body)
}

pub fn load_account(paths: &Paths, id: &str) -> Result<Account, String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("非法的账号 id".into());
    }
    let path = paths.accounts_dir().join(format!("{id}.json"));
    let raw = fs::read_to_string(&path).map_err(|_| format!("账号不存在：{id}"))?;
    serde_json::from_str(&raw).map_err(|e| format!("账号存档损坏：{e}"))
}

fn name_exists(accounts: &[Account], name: &str) -> bool {
    accounts.iter().any(|a| a.name.eq_ignore_ascii_case(name))
}

pub fn unique_name(accounts: &[Account], base: &str) -> String {
    if !name_exists(accounts, base) {
        return base.to_string();
    }
    for n in 2..1000 {
        let cand = format!("{base} {n}");
        if !name_exists(accounts, &cand) {
            return cand;
        }
    }
    format!("{base} {}", Uuid::new_v4().simple())
}

pub fn capture_current(paths: &Paths, name: Option<String>) -> Result<Account, String> {
    let live = read_live(paths)?.ok_or("当前没有 credentials.json，请先在 ZCode 里登录")?;
    if !is_logged_in(&live) {
        return Err("当前文件里没有登录凭据（未登录）".into());
    }
    let hash = canonical_hash(&live);
    let accounts = list_accounts(paths)?;
    if let Some(dup) = accounts.iter().find(|a| a.hash == hash) {
        return Err(format!("当前登录已保存为「{}」，无需重复保存", dup.name));
    }
    let config = read_live_config(paths);
    let name = match name {
        Some(n) => unique_name(&accounts, &n),
        None => {
            let id = zcrypto::account_identity(&live, &paths.home);
            unique_name(&accounts, &id.label().unwrap_or_else(|| "账号 1".into()))
        }
    };
    let ts = now_ts();
    let mut acc = Account {
        id: Uuid::new_v4().to_string(),
        name,
        created_at: ts.clone(),
        updated_at: ts,
        hash,
        credentials: live,
        config,
        virtual_device_mid: None,
    };
    adopt_virtual_device_mid(paths, &mut acc)?;
    Ok(acc)
}

fn auto_preserve(paths: &Paths, accounts: &[Account], target_hash: &str) -> Result<Option<String>, String> {
    let live = match read_live(paths)? {
        Some(v) if is_logged_in(&v) => v,
        _ => return Ok(None),
    };
    let hash = canonical_hash(&live);
    if hash == target_hash {
        return Ok(None);
    }
    if accounts.iter().any(|a| a.hash == hash) {
        return Ok(None);
    }
    let name = unique_name(accounts, &format!("自动保存 {}", Local::now().format("%m-%d %H%M")));
    let ts = now_ts();
    let mut acc = Account {
        id: Uuid::new_v4().to_string(),
        name: name.clone(),
        created_at: ts.clone(),
        updated_at: ts,
        hash,
        credentials: live,
        config: read_live_config(paths),
        virtual_device_mid: None,
    };
    adopt_virtual_device_mid(paths, &mut acc)?;
    Ok(Some(name))
}

pub fn switch_to(paths: &Paths, id: &str, force: bool, restart: bool, hot: bool) -> Result<SwitchResult, String> {
    let target = load_account(paths, id)?;
    let accounts = list_accounts(paths)?;
    let live = read_live(paths)?;
    let live_hash = live.as_ref().map(canonical_hash);

    if live_hash.as_deref() == Some(target.hash.as_str()) {
        let mid = ensure_virtual_device_mid(paths, &target.id)?;
        write_live_device_mid(paths, &mid)?;
        return Ok(SwitchResult {
            switched: false,
            already_active: true,
            name: target.name,
            preserved_as: None,
            killed: false,
            launched: false,
            hot: false,
            config_stale: false,
        });
    }

    let running = zcode_running();
    if hot && running {
        let preserved_as = auto_preserve(paths, &accounts, &target.hash)?;
        hot_swap_verified(paths, &target)?;
        let mid = ensure_virtual_device_mid(paths, &target.id)?;
        write_live_device_mid(paths, &mid)?;
        return Ok(SwitchResult {
            switched: true,
            already_active: false,
            name: target.name,
            preserved_as,
            killed: false,
            launched: false,
            hot: true,
            config_stale: target.config.is_none() && paths.live_config().exists(),
        });
    }

    let mut killed = false;
    if running {
        if !force {
            return Err("ZCode 正在运行，请先完全退出（含托盘），或使用强制切换（自动关闭并重启）".into());
        }
        if !kill_zcode()? {
            return Err("关闭 ZCode 超时，已取消切换（避免登录态损坏）".into());
        }
        killed = true;
    }

    let preserved_as = auto_preserve(paths, &accounts, &target.hash)?;

    write_live(paths, &target.credentials)?;
    if let Some(cfg) = &target.config {
        write_live_config(paths, cfg)?;
    }
    let mid = ensure_virtual_device_mid(paths, &target.id)?;
    write_live_device_mid(paths, &mid)?;

    let mut launched = false;
    if restart {
        let (p, ok) = effective_zcode_path(paths);
        if ok && launch_zcode(&p).is_ok() {
            launched = true;
        }
    }

    Ok(SwitchResult {
        switched: true,
        already_active: false,
        name: target.name,
        preserved_as,
        killed,
        launched,
        hot: false,
        config_stale: target.config.is_none() && paths.live_config().exists(),
    })
}

fn hot_swap_verified(paths: &Paths, target: &Account) -> Result<(), String> {
    let want = zcrypto::account_identity(&target.credentials, &paths.home);
    let use_hash = !identity_has_signal(&want);
    let verify = |v: &Value| {
        if use_hash {
            canonical_hash(v) == target.hash
        } else {
            identity_matches(&want, &zcrypto::account_identity(v, &paths.home))
        }
    };
    let mut last_err: Option<String> = None;
    let backoff = |attempt: u32| std::thread::sleep(std::time::Duration::from_millis(250 + u64::from(attempt) * 250));
    for attempt in 0..3u32 {
        if let Err(e) = write_live(paths, &target.credentials) {
            last_err = Some(format!("写入失败：{e}"));
            backoff(attempt);
            continue;
        }
        if let Some(cfg) = &target.config {
            if let Err(e) = write_live_config(paths, cfg) {
                last_err = Some(format!("写入 config 失败：{e}"));
                backoff(attempt);
                continue;
            }
        }
        if let Ok(Some(v)) = read_live(paths) {
            if verify(&v) {
                std::thread::sleep(std::time::Duration::from_millis(150));
                if let Ok(Some(v2)) = read_live(paths) {
                    if verify(&v2) {
                        return Ok(());
                    }
                }
            }
        }
        backoff(attempt);
    }
    Err(last_err.unwrap_or_else(|| "热切换校验失败：ZCode 并发回写冲突，已重试 3 次。建议改用重启式切换".into()))
}

fn identity_has_signal(id: &zcrypto::Identity) -> bool {
    id.user_id.as_deref().is_some_and(|s| !s.is_empty())
        || id.email.as_deref().is_some_and(|s| !s.is_empty())
        || id.username.as_deref().is_some_and(|s| !s.is_empty())
}

fn identity_matches(a: &zcrypto::Identity, b: &zcrypto::Identity) -> bool {
    let norm = |s: &str| s.trim().to_lowercase();
    let opt = |s: &Option<String>| norm(s.as_deref().unwrap_or("")).to_string();
    let (au, ae, ap, an) = (opt(&a.user_id), opt(&a.email), norm(&a.provider), opt(&a.username));
    let (bu, be, bp, bn) = (opt(&b.user_id), opt(&b.email), norm(&b.provider), opt(&b.username));
    if !au.is_empty() && !bu.is_empty() {
        return au == bu;
    }
    if !ae.is_empty() && !be.is_empty() {
        return ae == be;
    }
    ap == bp && !an.is_empty() && an == bn
}

pub fn rename_account(paths: &Paths, id: &str, new_name: &str) -> Result<Account, String> {
    let name = new_name.trim();
    if name.is_empty() {
        return Err("名称不能为空".into());
    }
    if name.chars().count() > 40 {
        return Err("名称过长（最多 40 字符）".into());
    }
    let mut acc = load_account(paths, id)?;
    let accounts = list_accounts(paths)?;
    if let Some(other) = accounts
        .iter()
        .find(|a| a.id != id && a.name.eq_ignore_ascii_case(name))
    {
        return Err(format!("名称「{}」已被账号「{}」占用", name, other.name));
    }
    acc.name = name.to_string();
    acc.updated_at = now_ts();
    save_account(paths, &acc)?;
    Ok(acc)
}

pub fn delete_account(paths: &Paths, id: &str) -> Result<(), String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("非法的账号 id".into());
    }
    let path = paths.accounts_dir().join(format!("{id}.json"));
    if !path.exists() {
        return Err("账号不存在".into());
    }
    fs::remove_file(&path).map_err(|e| format!("删除失败：{e}"))
}

pub fn update_account_from_live(paths: &Paths, id: &str) -> Result<Account, String> {
    let live = read_live(paths)?.ok_or("当前没有登录文件")?;
    if !is_logged_in(&live) {
        return Err("当前未登录".into());
    }
    let hash = canonical_hash(&live);
    let accounts = list_accounts(paths)?;
    if let Some(other) = accounts.iter().find(|a| a.id != id && a.hash == hash) {
        return Err(format!("当前登录与「{}」一致，请直接切换", other.name));
    }
    let mut acc = load_account(paths, id)?;
    acc.hash = hash;
    acc.credentials = live;
    acc.config = read_live_config(paths);
    acc.updated_at = now_ts();
    save_account(paths, &acc)?;
    Ok(acc)
}

pub fn live_quota(paths: &Paths) -> Result<quota::QuotaOverview, String> {
    let creds = read_live(paths)?.ok_or("当前没有登录文件")?;
    if !is_logged_in(&creds) {
        return Err("当前未登录，无法查询额度".into());
    }
    quota::quota_for_live(&paths.home, &creds, read_live_config(paths).as_ref())
}

pub fn account_quota(paths: &Paths, id: &str) -> Result<quota::QuotaOverview, String> {
    let acc = load_account(paths, id)?;
    quota::quota_for_snapshot(&paths.home, &acc.credentials, acc.config.as_ref())
}

pub fn ensure_virtual_device_mid(paths: &Paths, id: &str) -> Result<String, String> {
    {
        let acc = load_account(paths, id)?;
        if let Some(m) = acc.virtual_device_mid.clone() {
            if !m.trim().is_empty() {
                return Ok(m);
            }
        }
    }
    let mid = {
        let _guard = crate::store_guard();
        let mut acc = load_account(paths, id)?;
        if let Some(m) = acc.virtual_device_mid.clone() {
            if !m.trim().is_empty() {
                return Ok(m);
            }
        }
        let m = Uuid::new_v4().to_string();
        acc.virtual_device_mid = Some(m.clone());
        acc.updated_at = now_ts();
        save_account(paths, &acc)?;
        m
    };
    Ok(mid)
}

pub fn write_live_device_mid(paths: &Paths, mid: &str) -> Result<(), String> {
    let mut v: Value = fs::read_to_string(paths.live_telemetry())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));
    if !v.is_object() {
        v = json!({});
    }
    if v.get("deviceMid").and_then(|m| m.as_str()) == Some(mid) {
        return Ok(());
    }
    v["deviceMid"] = json!(mid);
    atomic_write(&paths.live_telemetry(), &(serde_json::to_string(&v).unwrap_or_default() + "\n"))
}

fn adopt_virtual_device_mid(paths: &Paths, acc: &mut Account) -> Result<(), String> {
    if acc.virtual_device_mid.as_deref().map_or(false, |m| !m.trim().is_empty()) {
        return Ok(());
    }
    let live_mid: Option<String> = fs::read_to_string(paths.live_telemetry())
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.get("deviceMid").and_then(|m| m.as_str()).map(String::from))
        .filter(|m| !m.trim().is_empty());
    let taken = |m: &str| {
        list_accounts(paths)
            .map(|accs| accs.iter().any(|a| a.virtual_device_mid.as_deref() == Some(m)))
            .unwrap_or(false)
    };
    let mid = match live_mid {
        Some(m) if !taken(&m) => m,
        _ => Uuid::new_v4().to_string(),
    };
    acc.virtual_device_mid = Some(mid);
    acc.updated_at = now_ts();
    save_account(paths, acc)
}

pub fn export_bundle_value(accounts: &[Account]) -> Value {
    json!({
        "format": "zcode-accounts-bundle",
        "version": 2,
        "exportedAt": now_ts(),
        "accounts": accounts.iter().map(|a| json!({
            "name": a.name,
            "createdAt": a.created_at,
            "credentials": a.credentials,
            "config": a.config,
        })).collect::<Vec<_>>(),
    })
}

type ImportCandidate = (Option<String>, Value, Option<Value>);

fn import_candidates(v: &Value) -> Result<Vec<ImportCandidate>, String> {
    if v.get("format").and_then(|f| f.as_str()) == Some("zcode-accounts-bundle") {
        let arr = v
            .get("accounts")
            .and_then(|a| a.as_array())
            .ok_or("捆绑包缺少 accounts 数组")?;
        let mut out = vec![];
        for item in arr {
            let creds = item.get("credentials").cloned().ok_or("捆绑包条目缺少 credentials")?;
            out.push((
                item.get("name").and_then(|n| n.as_str()).map(String::from),
                creds,
                item.get("config").cloned(),
            ));
        }
        return Ok(out);
    }
    Err("无法识别的文件格式（仅支持本工具导出的加密捆绑包 .zsb）".into())
}

pub fn import_values(paths: &Paths, files: &[(String, Value)]) -> Result<ImportReport, String> {
    let mut report = ImportReport { picked: true, ..Default::default() };
    let accounts = list_accounts(paths)?;

    let mut new_accounts: Vec<Account> = vec![];

    for (fname, v) in files {
        let cands = match import_candidates(v) {
            Ok(c) => c,
            Err(e) => {
                report.errors.push(format!("{fname}：{e}"));
                continue;
            }
        };
        for (name_opt, creds, config_opt) in cands {
            if !is_logged_in(&creds) {
                report.skipped.push(format!("{fname}：无登录凭据"));
                continue;
            }
            let hash = canonical_hash(&creds);
            if accounts.iter().any(|a| a.hash == hash) || new_accounts.iter().any(|a| a.hash == hash) {
                report.skipped.push(format!("{fname}：已存在于账号库"));
                continue;
            }
            let base_name = name_opt.unwrap_or_else(|| {
                let id = zcrypto::account_identity(&creds, &paths.home);
                id.label().unwrap_or_else(|| format!("导入 {}", Local::now().format("%m-%d %H%M")))
            });
            let name = unique_name(&accounts, &base_name);
            let ts = now_ts();
            let acc = Account {
                id: Uuid::new_v4().to_string(),
                name: name.clone(),
                created_at: ts.clone(),
                updated_at: ts,
                hash,
                credentials: creds,
                config: config_opt,
                virtual_device_mid: None,
            };
            new_accounts.push(acc);
            report.added.push(name);
        }
    }

    for acc in &new_accounts {
        save_account(paths, acc)?;
    }
    Ok(report)
}

pub fn get_state(paths: &Paths) -> Result<AppState, String> {
    let accounts = list_accounts(paths)?;
    let live = read_live(paths)?;
    let live_hash = live.as_ref().map(canonical_hash);
    let live_logged_in = live.as_ref().map(is_logged_in).unwrap_or(false);
    let active_account_id = live_hash
        .as_ref()
        .and_then(|h| accounts.iter().find(|a| &a.hash == h).map(|a| a.id.clone()));
    let live_identity = live
        .as_ref()
        .filter(|_| live_logged_in)
        .map(|v| zcrypto::account_identity(v, &paths.home));
    let (zcode_path, zcode_path_ok) = effective_zcode_path(paths);
    let settings = load_settings(paths);
    let summaries = accounts
        .iter()
        .map(|a| AccountSummary {
            id: a.id.clone(),
            name: a.name.clone(),
            created_at: a.created_at.clone(),
            updated_at: a.updated_at.clone(),
            is_active: live_hash.as_deref() == Some(a.hash.as_str()),
            has_config: a.config.is_some(),
            identity: zcrypto::account_identity(&a.credentials, &paths.home),
        })
        .collect();
    Ok(AppState {
        zcode_running: zcode_running(),
        live_exists: paths.live_file().exists(),
        live_logged_in,
        live_hash,
        active_account_id,
        live_identity,
        accounts: summaries,
        zcode_path,
        zcode_path_ok,
        store_dir: paths.store_dir().to_string_lossy().to_string(),
        launch_after_switch: settings.launch_after_switch(),
        close_to_tray: settings.close_to_tray(),
        hot_switch: settings.hot_switch(),
        auth_proxy_on: settings.auth_proxy_on.unwrap_or(false),
        auth_proxy_url: settings.auth_proxy_url.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;

    fn fake_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zswitch-test-{}-{}", tag, Uuid::new_v4().simple()));
        fs::create_dir_all(dir.join(".zcode").join("v2")).unwrap();
        std::env::set_var("ZCODE_SWITCH_HOME", &dir);
        dir
    }

    fn write_live_raw(home: &Path, marker: &str) -> Value {
        let v = json!({
            "oauth:bigmodel:access_token": format!("enc:v1:AAA{marker}"),
            "oauth:bigmodel:user_info": "enc:v1:BBBB",
            "oauth:active_provider": "enc:v1:CCCC",
            "zcodejwttoken": "enc:v1:DDDD",
        });
        let raw = if marker.chars().next().map(|c| c as u32).unwrap_or(0).is_multiple_of(2) {
            serde_json::to_string(&v).unwrap()
        } else {
            let m = v.as_object().unwrap();
            let mut keys: Vec<_> = m.keys().cloned().collect();
            keys.reverse();
            let mut o = Map::new();
            for k in keys { let val = m[&k].clone(); o.insert(k, val); }
            serde_json::to_string(&Value::Object(o)).unwrap()
        };
        fs::write(home.join(".zcode/v2/credentials.json"), raw).unwrap();
        v
    }

    fn write_config_raw(home: &Path, marker: &str) -> Value {
        let v = json!({ "provider": { "builtin:bigmodel-coding-plan": {
            "options": { "apiKey": format!("key-{marker}"), "baseURL": "https://open.bigmodel.cn/api/anthropic" }
        }}});
        fs::write(home.join(".zcode/v2/config.json"), serde_json::to_string(&v).unwrap()).unwrap();
        v
    }

    #[test]
    fn capture_list_rename_delete_roundtrip() {
        let home = fake_home("cap");
        let p = Paths::new(&home);
        write_live_raw(&home, "X1");

        let acc = capture_current(&p, Some("主号".into())).unwrap();
        assert_eq!(acc.name, "主号");
        assert!(acc.config.is_none(), "没有 config 文件时应为 None");

        write_config_raw(&home, "C1");
        write_live_raw(&home, "X2");
        let acc2 = capture_current(&p, Some("二号".into())).unwrap();
        assert!(acc2.config.is_some(), "config 文件存在时应一起快照");

        let list = list_accounts(&p).unwrap();
        assert_eq!(list.len(), 2);

        let err = capture_current(&p, Some("again".into())).unwrap_err();
        assert!(err.contains("二号"));

        let r = rename_account(&p, &acc.id, "工作号").unwrap();
        assert_eq!(r.name, "工作号");
        assert!(rename_account(&p, &acc2.id, "工作号").is_err());
        assert!(rename_account(&p, &acc2.id, "  ").is_err());

        delete_account(&p, &acc2.id).unwrap();
        assert_eq!(list_accounts(&p).unwrap().len(), 1);

        let outside = home.join("outside-victim.json");
        fs::write(&outside, "{}").unwrap();
        assert!(delete_account(&p, "../../outside-victim").is_err(), "相对穿越必须被拒");
        let abs_no_ext = outside.with_extension("");
        let abs_id = abs_no_ext.to_string_lossy().to_string();
        assert!(delete_account(&p, &abs_id).is_err(), "绝对路径必须被拒");
        assert!(delete_account(&p, "C:/evil").is_err(), "盘符路径必须被拒");
        assert!(delete_account(&p, "").is_err());
        assert!(outside.exists(), "库外文件不允许被删除");
    }

    #[test]
    fn capture_auto_name_from_identity() {
        let home = fake_home("idn");
        let p = Paths::new(&home);
        let secret = zcrypto::default_secret(&home);
        let ui = zcrypto::encrypt_with_secret(r#"{"username":"vcjzxsv6","displayName":"小明"}"#, &secret).unwrap();
        let ap = zcrypto::encrypt_with_secret("bigmodel", &secret).unwrap();
        let at = zcrypto::encrypt_with_secret("access-token-1234567890abcdef", &secret).unwrap();
        fs::write(
            home.join(".zcode/v2/credentials.json"),
            serde_json::to_string(&json!({
                "oauth:bigmodel:access_token": at,
                "oauth:bigmodel:user_info": ui,
                "oauth:active_provider": ap,
            }))
            .unwrap(),
        )
        .unwrap();
        let acc = capture_current(&p, None).unwrap();
        assert_eq!(acc.name, "小明", "自动命名应取 displayName");

        let st = get_state(&p).unwrap();
        assert_eq!(st.live_identity.as_ref().unwrap().username.as_deref(), Some("vcjzxsv6"));
        assert_eq!(st.accounts[0].identity.display_name.as_deref(), Some("小明"));
    }

    #[test]
    fn switch_with_config_replaces_live() {
        let home = fake_home("sw2");
        let p = Paths::new(&home);
        write_live_raw(&home, "A1");
        write_config_raw(&home, "CA");
        let a = capture_current(&p, Some("A".into())).unwrap();

        let r = switch_to(&p, &a.id, true, false, false).unwrap();
        assert!(r.already_active && !r.switched);

        write_live_raw(&home, "B7");
        write_config_raw(&home, "CB");
        let _b = capture_current(&p, Some("B".into())).unwrap();

        write_live_raw(&home, "B8");
        let r = switch_to(&p, &a.id, true, false, false).unwrap();
        assert!(r.switched);
        assert!(r.preserved_as.is_some());
        assert!(!r.killed, "沙箱模式下视为未运行，无需 kill 也能完成切换");

        let live_cred = fs::read_to_string(p.live_file()).unwrap();
        assert!(live_cred.contains("A1"));
        let live_cfg = fs::read_to_string(p.live_config()).unwrap();
        assert!(live_cfg.contains("key-CA"), "config 应切到 A 的快照");
    }

    #[test]
    fn switch_blocked_without_force_when_running_not_sandbox() {
        let home = fake_home("sw3");
        let p = Paths::new(&home);
        write_live_raw(&home, "A1");
        let a = capture_current(&p, Some("A".into())).unwrap();
        write_live_raw(&home, "B1");
        let b = capture_current(&p, Some("B".into())).unwrap();
        let r = switch_to(&p, &a.id, false, false, false).unwrap();
        assert!(r.switched, "未运行时 force=false 也应可切");
        let _ = b;
    }

    #[test]
    fn export_import_roundtrip_with_config() {
        let home = fake_home("ex2");
        let p = Paths::new(&home);
        write_live_raw(&home, "E1");
        write_config_raw(&home, "CE");
        let a = capture_current(&p, Some("导出源".into())).unwrap();

        let ev = export_bundle_value(std::slice::from_ref(&a));
        assert_eq!(ev["accounts"][0]["config"]["provider"]["builtin:bigmodel-coding-plan"]["options"]["apiKey"], "key-CE");

        let home3 = fake_home("ex2c");
        let p3 = Paths::new(&home3);
        let rep = import_values(&p3, &[("all.zsb".into(), export_bundle_value(std::slice::from_ref(&a)))]).unwrap();
        assert_eq!(rep.added.len(), 1);
        let rep = import_values(&p3, &[("all.zsb".into(), export_bundle_value(&[a]))]).unwrap();
        assert_eq!(rep.added.len(), 0);
        assert_eq!(rep.skipped.len(), 1);
        let accs = list_accounts(&p3).unwrap();
        assert!(accs[0].config.is_some(), "导入应带 config");

        let target = &accs[0];
        write_live_raw(&home3, "Z9");
        switch_to(&p3, &target.id, true, false, false).unwrap();
        let cfg = fs::read_to_string(p3.live_config()).unwrap();
        assert!(cfg.contains("key-CE"));
    }

    #[test]
    fn import_rejects_non_bundle_payloads() {
        let home = fake_home("ex3");
        let p = Paths::new(&home);
        let single = json!({ "format": "zcode-account", "version": 2, "credentials": { "oauth:bigmodel:access_token": "enc:v1:X" } });
        let rep = import_values(&p, &[("old-format.zs".into(), single)]).unwrap();
        assert_eq!(rep.added.len(), 0);
        assert!(rep.errors[0].contains("无法识别"), "err: {:?}", rep.errors);
        let raw = json!({ "oauth:bigmodel:access_token": "enc:v1:X", "zcodejwttoken": "enc:v1:J" });
        let rep = import_values(&p, &[("credentials.json".into(), raw)]).unwrap();
        assert_eq!(rep.added.len(), 0);
        assert!(rep.errors[0].contains("无法识别"), "err: {:?}", rep.errors);
    }

    #[test]
    fn state_reflects_active_and_identity() {
        let home = fake_home("st2");
        let p = Paths::new(&home);
        write_live_raw(&home, "S1");
        let a = capture_current(&p, Some("活跃".into())).unwrap();
        let st = get_state(&p).unwrap();
        assert!(st.live_logged_in);
        assert_eq!(st.active_account_id.as_deref(), Some(a.id.as_str()));
        assert!(st.accounts[0].is_active);
        assert!(st.launch_after_switch && st.close_to_tray, "行为默认开启");

        write_live_raw(&home, "S2");
        let st = get_state(&p).unwrap();
        assert_eq!(st.active_account_id, None);
        assert!(!st.accounts[0].is_active);
    }

    #[test]
    fn update_account_from_live_syncs_config() {
        let home = fake_home("up2");
        let p = Paths::new(&home);
        write_live_raw(&home, "U1");
        let a = capture_current(&p, Some("U".into())).unwrap();
        assert!(a.config.is_none());
        write_live_raw(&home, "U2");
        write_config_raw(&home, "CU2");
        let upd = update_account_from_live(&p, &a.id).unwrap();
        assert!(upd.config.is_some());
        let st = get_state(&p).unwrap();
        assert_eq!(st.active_account_id.as_deref(), Some(upd.id.as_str()));
    }

    #[test]
    fn hot_switch_settings_default_and_roundtrip() {
        let home = fake_home("hot-cfg");
        let paths = Paths::new(&home);
        assert!(!load_settings(&paths).hot_switch(), "默认关：冷切换保证设备码生效");
        let mut s = load_settings(&paths);
        s.hot_switch = Some(false);
        save_settings(&paths, &s).unwrap();
        assert!(!load_settings(&paths).hot_switch());
        let mut s = load_settings(&paths);
        s.hot_switch = Some(true);
        save_settings(&paths, &s).unwrap();
        assert!(load_settings(&paths).hot_switch());
    }

    #[test]
    fn auth_proxy_gate_and_roundtrip() {
        let home = fake_home("auth-proxy");
        let paths = Paths::new(&home);
        assert_eq!(load_settings(&paths).auth_proxy(), None);
        let mut s = Settings {
            auth_proxy_on: Some(false),
            auth_proxy_url: Some("http://127.0.0.1:7890".into()),
            ..Settings::default()
        };
        assert_eq!(s.auth_proxy(), None);
        s.auth_proxy_on = Some(true);
        s.auth_proxy_url = Some("  socks5://127.0.0.1:1080  ".into());
        assert_eq!(s.auth_proxy(), Some("socks5://127.0.0.1:1080"));
        for empty in ["", "   "] {
            s.auth_proxy_url = Some(empty.into());
            assert_eq!(s.auth_proxy(), None);
        }
        s.auth_proxy_url = None;
        assert_eq!(s.auth_proxy(), None);
        save_settings(&paths, &s).unwrap();
        let mut s2 = load_settings(&paths);
        assert_eq!(s2.auth_proxy_on, Some(true));
        assert_eq!(s2.auth_proxy_url, None);
        s2.auth_proxy_on = Some(true);
        s2.auth_proxy_url = Some("http://proxy.lan:8080".into());
        save_settings(&paths, &s2).unwrap();
        let s3 = load_settings(&paths);
        assert_eq!(s3.auth_proxy(), Some("http://proxy.lan:8080"));
    }

    #[test]
    fn identity_matches_by_user_id_then_email_then_fallback() {
        let mk = |uid: Option<&str>, email: Option<&str>, provider: &str, user: Option<&str>| zcrypto::Identity {
            provider: provider.into(), username: user.map(String::from),
            display_name: None, email: email.map(String::from), user_id: uid.map(String::from),
        };
        let a = mk(Some("u-123"), Some("a@x.com"), "bigmodel", Some("aa"));
        assert!(identity_matches(&a, &mk(Some("U-123 "), None, "zai", None)), "user_id 命中即等");
        assert!(!identity_matches(&a, &mk(Some("u-999"), Some("a@x.com"), "bigmodel", Some("aa"))), "user_id 不同时不能算等（防串号）");
        let b = mk(None, Some("B@x.com"), "bigmodel", Some("bb"));
        assert!(identity_matches(&b, &mk(None, Some(" b@X.COM "), "zai", None)), "email 命中即等");
        let c = mk(None, None, "bigmodel", Some("cc"));
        assert!(identity_matches(&c, &mk(None, None, "BigModel", Some("CC"))), "兜底：provider+username");
        assert!(!identity_matches(&c, &mk(None, None, "zai", Some("cc"))));
        let e1 = mk(None, None, "bigmodel", None);
        assert!(!identity_matches(&e1, &mk(None, None, "bigmodel", None)), "空对空不得判等");
        assert!(!identity_matches(&e1, &mk(None, None, "bigmodel", Some("x"))), "一方有 username 时不判等");
        assert!(identity_has_signal(&mk(Some("u"), None, "p", None)));
        assert!(!identity_has_signal(&mk(None, None, "p", None)), "仅 provider 不算可判别信号");
    }

    #[test]
    fn hot_swap_verified_replaces_live_and_passes() {
        let home = fake_home("hot-swap");
        let paths = Paths::new(&home);
        write_live_raw(&home, "before");
        let target = {
            let mut a = capture_current(&paths, None).unwrap();
            a.name = "target".into();
            a
        };
        write_live_raw(&home, "other");
        let before_hash = canonical_hash(&read_live(&paths).unwrap().unwrap());
        assert_ne!(before_hash, target.hash);
        hot_swap_verified(&paths, &target).unwrap();
        let after = read_live(&paths).unwrap().unwrap();
        assert_eq!(canonical_hash(&after), target.hash, "热切换后 live 即目标账号");
    }

    #[test]
    fn settings_behavior_toggle() {
        let home = fake_home("cfg2");
        let p = Paths::new(&home);
        let mut s = load_settings(&p);
        s.launch_after_switch = Some(false);
        s.close_to_tray = Some(false);
        save_settings(&p, &s).unwrap();
        let st = get_state(&p).unwrap();
        assert!(!st.launch_after_switch);
        assert!(!st.close_to_tray);
        let (path, _ok) = effective_zcode_path(&p);
        assert!(!path.is_empty());
    }

    #[test]
    fn sealed_export_import_roundtrip() {
        let home = fake_home("seal");
        let p = Paths::new(&home);
        write_live_raw(&home, "S1");
        write_config_raw(&home, "CS");
        let a = capture_current(&p, Some("加密号".into())).unwrap();

        let payload = export_bundle_value(&[a]);
        let sealed = crate::cipher::seal(&payload, "pass-123456", crate::cipher::FORMAT_BUNDLE).unwrap();
        let raw = serde_json::to_string(&sealed).unwrap();
        assert!(!raw.contains("加密号"));
        assert!(!raw.contains("oauth:bigmodel"));

        let env: Value = serde_json::from_str(&raw).unwrap();
        assert!(crate::cipher::open(&env, "wrong").is_err());

        let home2 = fake_home("seal2");
        let p2 = Paths::new(&home2);
        let opened = crate::cipher::open(&env, "pass-123456").unwrap();
        let rep = import_values(&p2, &[("加密号.zsb".into(), opened)]).unwrap();
        assert_eq!(rep.added.len(), 1, "errs: {:?}", rep.errors);
        let accs = list_accounts(&p2).unwrap();
        assert!(accs[0].config.is_some(), "解密导入应带 config");

        write_live_raw(&home2, "X9");
        switch_to(&p2, &accs[0].id, true, false, false).unwrap();
        let cfg = fs::read_to_string(p2.live_config()).unwrap();
        assert!(cfg.contains("key-CS"), "切换后 config 应来自加密导入的捆包");
    }

    #[test]
    fn quota_commands_read_files() {
        let home = fake_home("qt");
        let p = Paths::new(&home);
        let e = live_quota(&p).unwrap_err();
        assert!(e.contains("未登录") || e.contains("没有"));
        write_live_raw(&home, "Q1");
        let _ = live_quota(&p);
    }

    #[test]
    fn ensure_virtual_device_mid_stable_and_persisted() {
        let home = fake_home("vmid");
        let p = Paths::new(&home);
        write_live_raw(&home, "V1");
        let acc = capture_current(&p, Some("虚拟设备号".into())).unwrap();
        let m0 = acc.virtual_device_mid.expect("capture 应已生成虚拟 mid");
        assert_eq!(m0.len(), 36, "应为标准 UUID v4 长度");

        let m1 = ensure_virtual_device_mid(&p, &acc.id).unwrap();
        assert_eq!(m0, m1, "ensure 必须复用已有 mid");
        let m2 = ensure_virtual_device_mid(&p, &acc.id).unwrap();
        assert_eq!(m1, m2, "重复调用必须恒定");
        let reloaded = load_account(&p, &acc.id).unwrap();
        assert_eq!(reloaded.virtual_device_mid.as_deref(), Some(m1.as_str()));
        let raw = fs::read_to_string(p.accounts_dir().join(format!("{}.json", acc.id))).unwrap();
        let legacy: Account = serde_json::from_str(&raw.replace(
            &format!("\"virtual_device_mid\": \"{m1}\""),
            "\"virtual_device_mid\": null",
        ))
        .unwrap();
        assert!(legacy.virtual_device_mid.is_none());
    }

    #[test]
    fn capture_adopts_live_device_mid_with_collision_fallback() {
        let home = fake_home("adopt");
        let p = Paths::new(&home);
        write_live_raw(&home, "A1");

        let a1 = capture_current(&p, Some("一号".into())).unwrap();
        let m1 = a1.virtual_device_mid.unwrap();
        assert_eq!(m1.len(), 36);

        let real_mid = "11111111-2222-3333-4444-555555555555";
        fs::write(
            p.live_telemetry(),
            json!({ "deviceMid": real_mid, "lastDailyActiveDate": "2026-08-28" }).to_string(),
        )
        .unwrap();
        write_live_raw(&home, "B1");
        let a2 = capture_current(&p, Some("二号".into())).unwrap();
        assert_eq!(a2.virtual_device_mid.as_deref(), Some(real_mid), "无人占用应收养真实 mid");

        write_live_raw(&home, "C1");
        let a3 = capture_current(&p, Some("三号".into())).unwrap();
        let m3 = a3.virtual_device_mid.unwrap();
        assert_ne!(m3, real_mid, "占用冲突必须发新 mid");
        assert_ne!(m3, m1);
    }

    #[test]
    fn switch_rewrites_live_device_mid() {
        let home = fake_home("swmid");
        let p = Paths::new(&home);
        fs::write(
            p.live_telemetry(),
            json!({ "deviceMid": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee", "lastDailyActiveDate": "2026-08-01" }).to_string(),
        )
        .unwrap();
        write_live_raw(&home, "S1");
        let a = capture_current(&p, Some("源号".into())).unwrap();
        write_live_raw(&home, "S2");
        let b = capture_current(&p, Some("目标号".into())).unwrap();
        assert_ne!(
            a.virtual_device_mid, b.virtual_device_mid,
            "前置条件：两号不同设备"
        );

        write_live_raw(&home, "S1");
        switch_to(&p, &b.id, false, false, false).unwrap();
        let t: Value =
            serde_json::from_str(&fs::read_to_string(p.live_telemetry()).unwrap()).unwrap();
        assert_eq!(
            t.get("deviceMid").and_then(|m| m.as_str()),
            b.virtual_device_mid.as_deref(),
            "切换后设备身份必须是目标账号的虚拟 mid"
        );
        assert_eq!(
            t.get("lastDailyActiveDate").and_then(|m| m.as_str()),
            Some("2026-08-01"),
            "其它 telemetry 字段必须保留"
        );
    }
}
