//! Sağlayıcı bağdaştırıcıları. Her biri: algılama (senkron, dosya/PATH) ve
//! kullanım çekme (ağ/CLI). Şimdilik Claude Code ve Codex desteklenir.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};

use super::json::{extract_windows, num, string, window_from};
use super::{Env, Method, Presence, Usage, Window, http_json, read_json, stdio_rpc};
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
