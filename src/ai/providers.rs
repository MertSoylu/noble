//! Provider adapters. Each one: detection (sync, file/PATH) and
//! usage fetching (network/CLI). Supports Claude Code, Codex, Antigravity, OpenCode Go, Kilo Code and Command Code.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};

use super::json::{clamp_pct, extract_windows, normalize_label, num, reset_at, string, window_from};
use super::{Env, Method, Presence, Usage, Window, http_json, read_json, run_capture, stdio_rpc};
use crate::util;

pub struct ProviderDef {
    pub id: &'static str,
    pub name: &'static str,
    pub login_hint: &'static str,
    pub detect: fn(&Env) -> Presence,
    pub fetch: fn(&Env) -> Result<Usage, String>,
}

pub fn registry() -> Vec<ProviderDef> {
    vec![
        ProviderDef {
            id: "claude",
            name: "Claude Code",
            login_hint: "claude",
            detect: claude_detect,
            fetch: claude_fetch,
        },
        ProviderDef { id: "codex", name: "Codex", login_hint: "codex login", detect: codex_detect, fetch: codex_fetch },
        ProviderDef { id: "antigravity", name: "Antigravity", login_hint: "agy", detect: agy_detect, fetch: agy_fetch },
        ProviderDef {
            id: "opencode-go",
            name: "OpenCode Go",
            login_hint: "opencode auth login",
            detect: ocgo_detect,
            fetch: ocgo_fetch,
        },
        ProviderDef {
            id: "kilo",
            name: "Kilo Code",
            login_hint: "kilo auth login",
            detect: kilo_detect,
            fetch: kilo_fetch,
        },
        ProviderDef {
            id: "command-code",
            name: "Command Code",
            login_hint: "command-code login",
            detect: cmdc_detect,
            fetch: cmdc_fetch,
        },
    ]
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

// ─── Claude Code ─────────────────────────────────────────────────────────────
// ~/.claude/.credentials.json → claudeAiOauth.accessToken
// GET https://api.anthropic.com/api/oauth/usage  (anthropic-beta: oauth-2025-04-20)

fn claude_dir(env: &Env) -> PathBuf {
    env.var("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| env.at(&[".claude"]))
}

fn claude_creds(env: &Env) -> Option<Value> {
    read_json(&claude_dir(env).join(".credentials.json"))?.get("claudeAiOauth").cloned()
}

fn claude_detect(env: &Env) -> Presence {
    let token = claude_creds(env).and_then(|c| string(&c, &["accessToken"]));
    if token.is_some() {
        Presence::Ready
    } else if claude_dir(env).is_dir() {
        Presence::NoLogin
    } else {
        Presence::NotInstalled
    }
}

pub fn parse_claude(data: &Value) -> Vec<Window> {
    let mut out = Vec::new();
    for (key, label) in [("five_hour", "5H"), ("seven_day", "WEEK"), ("seven_day_opus", "OPUS")] {
        if let Some(w) = data.get(key).filter(|v| !v.is_null()).and_then(|v| window_from(v, label)) {
            out.push(Window { label: label.to_string(), ..w });
        }
    }
    out
}

fn claude_fetch(env: &Env) -> Result<Usage, String> {
    let creds = claude_creds(env).ok_or("not signed in")?;
    let token = string(&creds, &["accessToken"]).ok_or("not signed in")?;
    if let Some(exp) = num(&creds, &["expiresAt"])
        && (exp / 1000.0) as i64 <= now()
    {
        return Err("session expired".into());
    }
    let auth = format!("Bearer {token}");
    let data = http_json(
        env,
        Method::Get,
        "https://api.anthropic.com/api/oauth/usage",
        &[("Authorization", &auth), ("anthropic-beta", "oauth-2025-04-20"), ("User-Agent", "claude-cli/2.0.0")],
    )?;
    let windows = parse_claude(&data);
    let mut note = None;
    if let Some(extra) = data.get("extra_usage")
        && extra.get("is_enabled").and_then(Value::as_bool) == Some(true)
        && let (Some(used), Some(limit)) = (num(extra, &["used_credits"]), num(extra, &["monthly_limit"]))
        && limit > 0.0
    {
        note = Some(format!("extra ${:.2} / ${:.2}", used / 100.0, limit / 100.0));
    }
    let plan = string(&creds, &["subscriptionType"]).map(|s| s.to_uppercase());
    if windows.is_empty() {
        return Err("no usage data".into());
    }
    Ok(Usage { windows, plan, note })
}

// ─── Codex ───────────────────────────────────────────────────────────────────
// ~/.codex/auth.json + `codex app-server` → account/rateLimits/read

fn codex_dir(env: &Env) -> PathBuf {
    env.var("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| env.at(&[".codex"]))
}

fn codex_logged_in(env: &Env) -> bool {
    let Some(auth) = read_json(&codex_dir(env).join("auth.json")) else { return false };
    if string(&auth, &["OPENAI_API_KEY"]).is_some() {
        return true;
    }
    auth.get("tokens").is_some_and(|t| string(t, &["access_token", "id_token", "refresh_token"]).is_some())
}

fn codex_detect(env: &Env) -> Presence {
    if codex_logged_in(env) {
        Presence::Ready
    } else if codex_dir(env).is_dir() || util::which("codex").is_some() {
        Presence::NoLogin
    } else {
        Presence::NotInstalled
    }
}

pub fn parse_codex(result: &Value) -> Usage {
    let windows = extract_windows(result, &["fiveHour", "five_hour", "primary", "weekly", "secondary"], 2);
    let plan = result
        .get("rateLimits")
        .and_then(|r| string(r, &["planType", "plan_type"]))
        .or_else(|| string(result, &["planType", "plan_type"]))
        .map(|s| s.to_uppercase());
    Usage { windows, plan, note: None }
}

fn codex_fetch(env: &Env) -> Result<Usage, String> {
    let bin = util::which("codex").ok_or("codex cli not on PATH")?;
    let _ = env;
    let result = stdio_rpc(&bin, &["app-server"], "account/rateLimits/read", json!({}), Duration::from_secs(12))?;
    let usage = parse_codex(&result);
    if usage.windows.is_empty() { Err("no usage data".into()) } else { Ok(usage) }
}

// ─── Antigravity ─────────────────────────────────────────────────────────────
// `agy --print /usage --output-format json` → command.data.groups[].buckets[]
// (remaining_fraction, reset_time, window). Credentials are never touched: the
// CLI uses its own session. Since agy could open a browser login when signed out,
// it only runs when a session already exists (existence check, no content read).

fn agy_bin(env: &Env) -> Option<PathBuf> {
    util::which("agy").or_else(|| {
        let p = PathBuf::from(env.var("LOCALAPPDATA")?).join("agy").join("bin").join("agy.exe");
        p.is_file().then_some(p)
    })
}

fn agy_signed_in(env: &Env) -> bool {
    let files = [
        env.at(&[".gemini", "jetski-standalone-oauth-token"]),
        env.at(&[".gemini", "antigravity-cli", "antigravity-oauth-token"]),
    ];
    files.iter().any(|f| f.is_file()) || agy_credential_exists()
}

/// On Windows the agy session lives in the Credential Manager; `cmdkey /list` only
/// lists target names and never returns the secret.
#[cfg(windows)]
fn agy_credential_exists() -> bool {
    util::command_for(std::path::Path::new("cmdkey"))
        .arg("/list:gemini:antigravity")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).to_lowercase().contains("gemini:antigravity"))
}

/// On Linux the same keyring entry (service `gemini`, user `antigravity`) lives in
/// the Secret Service. `SearchItems` only returns item paths, never a secret.
#[cfg(not(windows))]
fn agy_credential_exists() -> bool {
    let Some(gdbus) = util::which("gdbus") else { return false };
    util::command_for(&gdbus)
        .args(["call", "--session", "--timeout", "3", "--dest", "org.freedesktop.secrets"])
        .args(["--object-path", "/org/freedesktop/secrets", "--method"])
        .args(["org.freedesktop.Secret.Service.SearchItems", "{'service': 'gemini', 'username': 'antigravity'}"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .is_ok_and(|o| secret_search_found(&String::from_utf8_lossy(&o.stdout)))
}

/// `SearchItems` reply: `([objectpath '/org/…/1'], @ao [])` — unlocked and locked
/// item paths. Any path (even a locked one) means a session exists.
#[cfg_attr(windows, allow(dead_code))]
fn secret_search_found(reply: &str) -> bool {
    reply.contains("/org/freedesktop/secrets/")
}

fn agy_detect(env: &Env) -> Presence {
    let installed = agy_bin(env).is_some();
    if installed && agy_signed_in(env) {
        Presence::Ready
    } else if installed || env.at(&[".gemini", "antigravity-cli"]).is_dir() {
        Presence::NoLogin
    } else {
        Presence::NotInstalled
    }
}

/// For each window kind (5 hours, weekly) the fullest of the model groups
/// (Gemini, Claude/GPT…) is shown: whichever pool runs dry first is what matters.
pub fn parse_agy(v: &Value) -> Result<Usage, String> {
    if v.get("status").and_then(Value::as_str) != Some("SUCCESS") {
        return Err("usage command failed".into());
    }
    let groups = v.pointer("/command/data/groups").and_then(Value::as_array).ok_or("no usage data")?;
    let mut windows: Vec<Window> = Vec::new();
    for bucket in groups.iter().filter_map(|g| g.get("buckets")?.as_array()).flatten() {
        let Some(left) = num(bucket, &["remaining_fraction"]).filter(|f| (0.0..=1.0).contains(f)) else { continue };
        let label = normalize_label(&string(bucket, &["window", "id"]).unwrap_or_default());
        if label != "5H" && label != "WEEK" {
            continue;
        }
        let w = Window { label, used: clamp_pct((1.0 - left) * 100.0), resets_at: reset_at(bucket, &["reset_time"]) };
        match windows.iter_mut().find(|x| x.label == w.label) {
            Some(x) if w.used > x.used => *x = w,
            Some(_) => {}
            None => windows.push(w),
        }
    }
    windows.sort_by_key(|w| w.label != "5H");
    if windows.is_empty() { Err("no usage data".into()) } else { Ok(Usage { windows, plan: None, note: None }) }
}

fn agy_fetch(env: &Env) -> Result<Usage, String> {
    let bin = agy_bin(env).ok_or("agy not on PATH")?;
    let args = ["--print", "/usage", "--output-format", "json", "--print-timeout", "20s"];
    let out = run_capture(&bin, &args, bin.parent(), Duration::from_secs(25))?;
    // Skip possible warning lines: the JSON starts at the first '{'.
    let json = out.find('{').map(|i| &out[i..]).ok_or("unexpected response")?;
    parse_agy(&serde_json::from_str(json.trim()).map_err(|_| "unexpected response".to_string())?)
}

// ─── OpenCode Go ─────────────────────────────────────────────────────────────
// ~/.local/share/opencode/auth.json → "opencode-go" (else Zen: "opencode") API key
// GET https://opencode.ai/zen/go/v1/usage → usage.{rolling,weekly,monthly}
// A key without a Go subscription gets 403 → counted as "not signed in", hidden from the panel.

fn opencode_auth(env: &Env) -> PathBuf {
    let data = env.var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| env.at(&[".local", "share"]));
    data.join("opencode").join("auth.json")
}

fn ocgo_key(env: &Env) -> Option<String> {
    let auth = read_json(&opencode_auth(env))?;
    ["opencode-go", "opencode"].iter().find_map(|id| {
        let entry = auth.get(*id)?;
        if entry.get("type").and_then(Value::as_str) != Some("api") {
            return None;
        }
        string(entry, &["key"])
    })
}

fn ocgo_detect(env: &Env) -> Presence {
    if ocgo_key(env).is_some() {
        Presence::Ready
    } else if opencode_auth(env).is_file() || util::which("opencode").is_some() {
        Presence::NoLogin
    } else {
        Presence::NotInstalled
    }
}

pub fn parse_ocgo(v: &Value) -> Vec<Window> {
    let Some(usage) = v.get("usage") else { return Vec::new() };
    [("rolling", "5H"), ("weekly", "WEEK"), ("monthly", "MONTH")]
        .iter()
        .filter_map(|(key, label)| {
            let w = usage.get(*key)?;
            let used = match w.get("status").and_then(Value::as_str) {
                Some("rate-limited") => 100,
                Some("ok") => clamp_pct(num(w, &["percent"])?),
                _ => return None,
            };
            Some(Window { label: label.to_string(), used, resets_at: reset_at(w, &["resetsAt"]) })
        })
        .collect()
}

fn ocgo_fetch(env: &Env) -> Result<Usage, String> {
    let key = ocgo_key(env).ok_or("not signed in")?;
    let auth = format!("Bearer {key}");
    let data = http_json(env, Method::Get, "https://opencode.ai/zen/go/v1/usage", &[("Authorization", &auth)])?;
    let windows = parse_ocgo(&data);
    if windows.is_empty() { Err("no usage data".into()) } else { Ok(Usage { windows, plan: None, note: None }) }
}

// ─── Kilo Code ───────────────────────────────────────────────────────────────
// ~/.local/share/kilo/auth.json → "kilo" (api: key, oauth: access + accountId=organization)
// GET https://api.kilo.ai/api/trpc/kiloPass.getState → subscription.{currentPeriod*Usd, nextBillingAt}
// (The endpoint the Kilo CLI itself uses). Without Kilo Pass the provider is hidden.

fn kilo_auth(env: &Env) -> PathBuf {
    let data = env.var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| env.at(&[".local", "share"]));
    data.join("kilo").join("auth.json")
}

/// The token and (optionally) the organization id.
fn kilo_token(env: &Env) -> Option<(String, Option<String>)> {
    let entry = read_json(&kilo_auth(env))?.get("kilo")?.clone();
    match entry.get("type").and_then(Value::as_str)? {
        "api" => Some((string(&entry, &["key"])?, None)),
        "oauth" => Some((string(&entry, &["access"])?, string(&entry, &["accountId"]))),
        _ => None,
    }
}

fn kilo_detect(env: &Env) -> Presence {
    if kilo_token(env).is_some() {
        Presence::Ready
    } else if kilo_auth(env).is_file() || util::which("kilo").is_some() {
        Presence::NoLogin
    } else {
        Presence::NotInstalled
    }
}

/// The Kilo Pass period from a tRPC batch response: used / (base + bonus) credits.
/// `None` when there is no active subscription.
pub fn parse_kilo_pass(v: &Value) -> Option<Window> {
    let item = v.as_array().and_then(|a| a.first()).unwrap_or(v);
    let data = item.pointer("/result/data")?;
    let sub = data.get("json").unwrap_or(data).get("subscription")?;
    if let Some(status) = sub.get("status").and_then(Value::as_str)
        && !matches!(status, "active" | "past_due" | "trialing")
    {
        return None;
    }
    let usage = num(sub, &["currentPeriodUsageUsd"])?;
    let total = num(sub, &["currentPeriodBaseCreditsUsd"]).unwrap_or(0.0)
        + num(sub, &["currentPeriodBonusCreditsUsd"]).unwrap_or(0.0);
    if total <= 0.0 {
        return None;
    }
    Some(Window {
        label: "MONTH".into(),
        used: clamp_pct(usage / total * 100.0),
        resets_at: reset_at(sub, &["nextBillingAt", "nextRenewalAt"]),
    })
}

fn kilo_fetch(env: &Env) -> Result<Usage, String> {
    let (token, org) = kilo_token(env).ok_or("not signed in")?;
    let auth = format!("Bearer {token}");
    let mut headers = vec![("Authorization", auth.as_str())];
    if let Some(org) = org.as_deref() {
        headers.push(("x-kilocode-organizationid", org));
    }
    // input = {"0":null}
    let url = "https://api.kilo.ai/api/trpc/kiloPass.getState?batch=1&input=%7B%220%22%3Anull%7D";
    let data = http_json(env, Method::Get, url, &headers)?;
    let window = parse_kilo_pass(&data).ok_or("no plan")?;
    Ok(Usage { windows: vec![window], plan: Some("PASS".into()), note: None })
}

// ─── Command Code ────────────────────────────────────────────────────────────
// ~/.commandcode/auth.json → apiKey
// GET https://api.commandcode.ai/alpha/billing/credits → credits.windowLimits.{fiveHour,weekly}
// (The endpoint the CLI's own /usage screen uses; limits come from the server).
// Accounts with no window limit (credits only) are hidden.

fn cmdc_key(env: &Env) -> Option<String> {
    string(&read_json(&env.at(&[".commandcode", "auth.json"]))?, &["apiKey"])
}

fn cmdc_detect(env: &Env) -> Presence {
    if cmdc_key(env).is_some() {
        Presence::Ready
    } else if env.at(&[".commandcode"]).is_dir() || util::which("command-code").is_some() {
        Presence::NoLogin
    } else {
        Presence::NotInstalled
    }
}

pub fn parse_cmdc(v: &Value) -> Usage {
    let credits = v.get("credits").unwrap_or(v);
    let limits = credits.get("windowLimits");
    let windows = [("fiveHour", "5H"), ("weekly", "WEEK")]
        .iter()
        .filter_map(|(key, label)| {
            let w = limits?.get(*key)?;
            let cap = num(w, &["cap"]).filter(|c| *c > 0.0)?;
            let used = num(w, &["used"]).unwrap_or(0.0);
            Some(Window {
                label: label.to_string(),
                used: clamp_pct(used / cap * 100.0),
                resets_at: reset_at(w, &["resetAt", "resetsAt"]),
            })
        })
        .collect();
    // "individual-pro" → "PRO", "teams-pro" → "TEAMS PRO".
    let plan =
        string(credits, &["planId"]).map(|p| p.trim_start_matches("individual-").replace('-', " ").to_uppercase());
    Usage { windows, plan, note: None }
}

fn cmdc_fetch(env: &Env) -> Result<Usage, String> {
    let key = cmdc_key(env).ok_or("not signed in")?;
    let auth = format!("Bearer {key}");
    let data =
        http_json(env, Method::Get, "https://api.commandcode.ai/alpha/billing/credits", &[("Authorization", &auth)])?;
    let usage = parse_cmdc(&data);
    if usage.windows.is_empty() { Err("no plan".into()) } else { Ok(usage) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_payload() {
        let v = json!({
            "five_hour": {"utilization": 71.0, "resets_at": "2026-09-22T18:00:00+00:00"},
            "seven_day": {"utilization": 28.2, "resets_at": "2026-09-26T09:00:00+00:00"},
            "seven_day_opus": null
        });
        let ws = parse_claude(&v);
        assert_eq!(ws.len(), 2);
        assert_eq!((ws[0].label.as_str(), ws[0].used), ("5H", 71));
        assert_eq!((ws[1].label.as_str(), ws[1].used), ("WEEK", 28));
    }

    #[test]
    fn codex_payload() {
        let v = json!({"rateLimits": {"planType": "plus",
            "primary": {"usedPercent": 18, "windowDurationMins": 300, "resetsAt": 1_779_459_394},
            "secondary": {"usedPercent": 44, "windowDurationMins": 10080, "resetsAt": 1_779_900_000}}});
        let u = parse_codex(&v);
        assert_eq!(u.windows.len(), 2);
        assert_eq!(u.windows[0].label, "5H");
        assert_eq!(u.windows[1].used, 44);
        assert_eq!(u.plan.as_deref(), Some("PLUS"));
    }

    #[test]
    fn agy_secret_service_reply() {
        assert!(secret_search_found("([objectpath '/org/freedesktop/secrets/collection/login/7'], @ao [])"));
        assert!(secret_search_found("(@ao [], [objectpath '/org/freedesktop/secrets/collection/login/7'])"));
        assert!(!secret_search_found("(@ao [], @ao [])"));
        assert!(!secret_search_found(""));
    }

    #[test]
    fn agy_payload() {
        let v = json!({"status": "SUCCESS", "response": "usage", "command": {"name": "usage", "data": {"groups": [
            {"name": "Gemini Models", "buckets": [
                {"id": "gemini-weekly", "window": "weekly", "remaining_fraction": 0.85, "reset_time": "2026-09-23T14:09:40Z"},
                {"id": "gemini-5h", "window": "5h", "remaining_fraction": 1, "reset_time": "2026-09-23T18:30:15Z"}]},
            {"name": "Claude and GPT models", "buckets": [
                {"id": "3p-weekly", "window": "weekly", "remaining_fraction": 1},
                {"id": "3p-5h", "window": "5h", "remaining_fraction": 0.4, "reset_time": "2026-09-23T19:00:00Z"},
                {"id": "bad", "window": "5h", "remaining_fraction": 2.0}]}]}}});
        let u = parse_agy(&v).unwrap();
        assert_eq!(u.windows.len(), 2);
        // At 5h the Claude/GPT pool is fuller (60%), weekly Gemini (15%).
        assert_eq!((u.windows[0].label.as_str(), u.windows[0].used), ("5H", 60));
        assert_eq!(u.windows[0].resets_at, Some(1_790_190_000));
        assert_eq!((u.windows[1].label.as_str(), u.windows[1].used), ("WEEK", 15));
        assert!(parse_agy(&json!({"status": "ERROR"})).is_err());
        assert!(parse_agy(&json!({"status": "SUCCESS", "command": {"data": {"groups": []}}})).is_err());
    }

    #[test]
    fn ocgo_payload() {
        let v = json!({"usage": {
            "rolling": {"status": "ok", "percent": 42.4, "resetsAt": "2026-09-24T18:00:00+00:00"},
            "weekly": {"status": "rate-limited", "percent": 97, "resetsAt": "2026-09-28T00:00:00Z"},
            "monthly": {"status": "ok", "percent": 12, "resetsAt": "2026-10-01T00:00:00Z"}}});
        let ws = parse_ocgo(&v);
        let got: Vec<_> = ws.iter().map(|w| (w.label.as_str(), w.used)).collect();
        assert_eq!(got, [("5H", 42), ("WEEK", 100), ("MONTH", 12)]);
        assert_eq!(ws[0].resets_at, Some(1_790_272_800));
        assert!(parse_ocgo(&json!({"error": "x"})).is_empty());
    }

    #[test]
    fn ocgo_key_prefers_go_entry() {
        let home = std::env::temp_dir().join(format!("noble-ocgo-home-{}", std::process::id()));
        let dir = home.join(".local/share/opencode");
        std::fs::create_dir_all(&dir).unwrap();
        let env = Env::new(home.clone());
        std::fs::write(dir.join("auth.json"), r#"{"opencode":{"type":"api","key":"zen"}}"#).unwrap();
        assert_eq!(ocgo_key(&env).as_deref(), Some("zen"));
        let both = r#"{"opencode":{"type":"api","key":"zen"},"opencode-go":{"type":"api","key":"go"}}"#;
        std::fs::write(dir.join("auth.json"), both).unwrap();
        assert_eq!(ocgo_key(&env).as_deref(), Some("go"));
        std::fs::write(dir.join("auth.json"), r#"{"opencode":{"type":"oauth","access":"x"}}"#).unwrap();
        assert_eq!(ocgo_detect(&env), Presence::NoLogin);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn kilo_pass_payload() {
        let v = json!([{"result": {"data": {"json": {"subscription": {
            "status": "active", "currentPeriodBaseCreditsUsd": 19, "currentPeriodBonusCreditsUsd": 1,
            "currentPeriodUsageUsd": 5, "nextBillingAt": "2026-10-01T00:00:00Z"}}}}}]);
        let w = parse_kilo_pass(&v).unwrap();
        assert_eq!((w.label.as_str(), w.used), ("MONTH", 25));
        assert_eq!(w.resets_at, Some(1_790_812_800));
        // Canceled or missing subscription.
        let canceled = json!([{"result": {"data": {"json": {"subscription": {
            "status": "canceled", "currentPeriodBaseCreditsUsd": 19, "currentPeriodUsageUsd": 5}}}}}]);
        assert!(parse_kilo_pass(&canceled).is_none());
        assert!(parse_kilo_pass(&json!([{"result": {"data": {"json": {"subscription": null}}}}])).is_none());
    }

    #[test]
    fn kilo_token_kinds() {
        let home = std::env::temp_dir().join(format!("noble-kilo-home-{}", std::process::id()));
        let dir = home.join(".local/share/kilo");
        std::fs::create_dir_all(&dir).unwrap();
        let env = Env::new(home.clone());
        std::fs::write(dir.join("auth.json"), r#"{"kilo":{"type":"api","key":"k"}}"#).unwrap();
        assert_eq!(kilo_token(&env), Some(("k".into(), None)));
        let oauth = r#"{"kilo":{"type":"oauth","access":"a","refresh":"","expires":0,"accountId":"org"}}"#;
        std::fs::write(dir.join("auth.json"), oauth).unwrap();
        assert_eq!(kilo_token(&env), Some(("a".into(), Some("org".into()))));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn cmdc_payload() {
        let v = json!({"credits": {"planId": "individual-pro", "monthlyCredits": 60,
            "windowLimits": {"limited": true,
                "fiveHour": {"used": 4, "cap": 16, "resetAt": "2026-09-24T18:00:00Z"},
                "weekly": {"used": 30, "cap": 40, "resetAt": 1_790_812_800_000_i64}}}});
        let u = parse_cmdc(&v);
        let got: Vec<_> = u.windows.iter().map(|w| (w.label.as_str(), w.used)).collect();
        assert_eq!(got, [("5H", 25), ("WEEK", 75)]);
        assert_eq!(u.windows[0].resets_at, Some(1_790_272_800));
        assert_eq!(u.windows[1].resets_at, Some(1_790_812_800));
        assert_eq!(u.plan.as_deref(), Some("PRO"));
        // Unlimited (credits only) account.
        assert!(parse_cmdc(&json!({"credits": {"windowLimits": null}})).windows.is_empty());
    }

    #[test]
    fn detection_on_empty_home() {
        let home = std::env::temp_dir().join(format!("noble-ai-home-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&home);
        let env = Env::new(home.clone());
        assert_eq!(claude_detect(&env), Presence::NotInstalled);
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        assert_eq!(claude_detect(&env), Presence::NoLogin);
        std::fs::write(home.join(".claude/.credentials.json"), r#"{"claudeAiOauth":{"accessToken":"x"}}"#).unwrap();
        assert_eq!(claude_detect(&env), Presence::Ready);
        let _ = std::fs::remove_dir_all(&home);
    }
}
