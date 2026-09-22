//! AI abonelik kotası takibi (Claude Code, Codex).
//!
//! Kimlik bilgileri yalnızca makinedeki mevcut CLI oturumlarından okunur, sadece
//! ilgili sağlayıcıya istek başlığında gönderilir; asla ekrana basılmaz ya da
//! loglanmaz. Tokenlar yenilenmez (CLI'ların kendi yenileme akışıyla yarışmamak
//! için); süresi dolan oturum "oturum aç" ipucuna düşer.

pub mod json;
pub mod providers;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::AiCfg;
use crate::event::{AppEvent, Tx};
use crate::util;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Window {
    pub label: String,
    /// Kullanılan yüzde (0..100).
    pub used: u8,
    /// Sıfırlanma zamanı (unix saniye).
    pub resets_at: Option<i64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Usage {
    pub windows: Vec<Window>,
    pub plan: Option<String>,
    pub note: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    Ready,
    NoLogin,
    NotInstalled,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Henüz kontrol edilmedi (açılışta önbellekten gösterilir).
    Pending,
    Loading,
    Ok,
    SignIn,
    Error(String),
}

#[derive(Clone, Debug)]
pub struct ProviderState {
    pub id: &'static str,
    pub name: &'static str,
    pub login_hint: &'static str,
    pub presence: Presence,
    pub status: Status,
    pub usage: Option<Usage>,
    pub fetched_at: Option<i64>,
}

impl ProviderState {
    /// Kullanım verisi son başarılı çekimden mi (önbellek/eskimiş)?
    pub fn is_stale(&self) -> bool {
        !matches!(self.status, Status::Ok)
    }

    /// En dolu pencerenin yüzdesi (durum çubuğu için).
    pub fn peak(&self) -> Option<u8> {
        self.usage.as_ref()?.windows.iter().map(|w| w.used).max()
    }
}

/// Kota penceresinin okunur adı ("5H" → "5h", "WEEK" → "week").
pub fn window_name(label: &str) -> String {
    match label {
        "5H" => "5h".into(),
        "WEEK" => "week".into(),
        "OPUS" => "opus".into(),
        other => other.to_lowercase(),
    }
}

/// Pane'de çalışan AI aracı: başlatıcı komutundan ya da pencere başlığından.
pub fn agent_kind(command: Option<&str>, label: &str) -> Option<&'static str> {
    let hay = format!("{} {}", command.unwrap_or(""), label).to_lowercase();
    if hay.contains("claude") {
        Some("claude")
    } else if hay.contains("codex") {
        Some("codex")
    } else {
        None
    }
}

/// Sağlayıcıların çalışma ortamı (testte sahte ev dizini verilebilir).
pub struct Env {
    pub home: PathBuf,
    pub agent: ureq::Agent,
}

impl Env {
    pub fn new(home: PathBuf) -> Env {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .http_status_as_error(false)
            .user_agent("noble/1.0")
            .build();
        Env { home, agent: ureq::Agent::new_with_config(config) }
    }

    pub fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
    }

    pub fn at(&self, parts: &[&str]) -> PathBuf {
        parts.iter().fold(self.home.clone(), |p, s| p.join(s))
    }
}

pub fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

pub enum Method<'a> {
    Get,
    Post(&'a str),
}

/// JSON isteği; ağ/durum/ayrıştırma hatalarını kısa bir mesaja çevirir.
pub fn http_json(env: &Env, method: Method<'_>, url: &str, headers: &[(&str, &str)]) -> Result<Value, String> {
    let result = match method {
        Method::Get => {
            let mut req = env.agent.get(url).header("Accept", "application/json");
            for (k, v) in headers {
                req = req.header(*k, *v);
            }
            req.call()
        }
        Method::Post(body) => {
            let mut req =
                env.agent.post(url).header("Accept", "application/json").header("Content-Type", "application/json");
            for (k, v) in headers {
                req = req.header(*k, *v);
            }
            req.send(body)
        }
    };
    let mut resp = result.map_err(|e| match e {
        ureq::Error::Timeout(_) => "timeout".to_string(),
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => "offline".to_string(),
        _ => "network error".to_string(),
    })?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    match status {
        200..=299 => serde_json::from_str(&text).map_err(|_| "unexpected response".to_string()),
        401 | 403 => Err("session expired".into()),
        429 => Err("rate limited".into()),
        s => Err(format!("HTTP {s}")),
    }
}

/// Satır tabanlı JSON-RPC (codex app-server): initialize el sıkışması, tek çağrı.
pub fn stdio_rpc(
    program: &Path,
    args: &[&str],
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<Value, String> {
    let mut child = util::command_for(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "cli not runnable".to_string())?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let (line_tx, line_rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line_tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut send = |v: Value| -> Result<(), String> {
        writeln!(stdin, "{v}").and_then(|_| stdin.flush()).map_err(|_| "rpc write failed".to_string())
    };
    send(serde_json::json!({"jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {"clientInfo": {"name": "noble", "title": "NOBLE", "version": env!("CARGO_PKG_VERSION")}}}))?;
    send(serde_json::json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}))?;
    let deadline = Instant::now() + timeout;
    let result = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break Err("timeout".to_string());
        }
        match line_rx.recv_timeout(left) {
            Ok(line) => {
                let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else { continue };
                match msg.get("id").and_then(Value::as_i64) {
                    Some(0) => {
                        if msg.get("error").is_some() {
                            break Err("rpc initialize failed".into());
                        }
                        send(serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))?;
                    }
                    Some(1) => {
                        break match msg.get("result") {
                            Some(r) => Ok(r.clone()),
                            None => Err("rpc error".into()),
                        };
                    }
                    _ => {}
                }
            }
            Err(RecvTimeoutError::Timeout) => break Err("timeout".into()),
            Err(RecvTimeoutError::Disconnected) => break Err("cli exited".into()),
        }
    };
    let _ = child.kill();
    let _ = child.wait();
    result
}

// ─── Önbellek ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheEntry {
    pub usage: Usage,
    pub fetched_at: i64,
}

pub fn load_cache(file: &Path) -> HashMap<String, CacheEntry> {
    std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save_cache(file: &Path, cache: &HashMap<String, CacheEntry>) {
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(cache) {
        let _ = std::fs::write(file, text);
    }
}

/// Açılışta gösterilecek ilk durumlar: önbellekte verisi olanlar.
pub fn initial_states(cfg: &AiCfg, cache: &HashMap<String, CacheEntry>) -> Vec<ProviderState> {
    providers::registry()
        .into_iter()
        .filter(|d| cfg.providers.iter().any(|p| p.eq_ignore_ascii_case(d.id)))
        .filter_map(|d| {
            let entry = cache.get(d.id)?;
            Some(ProviderState {
                id: d.id,
                name: d.name,
                login_hint: d.login_hint,
                presence: Presence::Ready,
                status: Status::Pending,
                usage: Some(entry.usage.clone()),
                fetched_at: Some(entry.fetched_at),
            })
        })
        .collect()
}

/// Toplayıcıya istekler.
pub enum AiReq {
    /// Elle yenile (R, ayar değişikliği): hemen çek.
    Refresh,
    /// Kota panelinin görünürlüğü. Görünmüyorken hiç istek atılmaz (pil dostu);
    /// görünür olunca veri eskiyse hemen çekilir.
    Visible(bool),
}

/// Home'a dönüşte bundan yeni veri tekrar çekilmez (hızlı sekme geçişleri).
const FRESH_ENOUGH: Duration = Duration::from_secs(30);

/// Bir sonraki çekimin zamanı geldi mi?
pub fn fetch_due(visible: bool, last: Option<Instant>, every: Duration, now: Instant) -> bool {
    visible && last.is_none_or(|t| now.duration_since(t) >= every)
}

/// Toplayıcı iş parçacığı: algıla → çek → gönder → önbelleğe yaz → bekle.
/// Yalnızca kota paneli görünürken çalışır.
pub fn spawn(cfg: std::sync::Arc<std::sync::Mutex<AiCfg>>, cache_file: PathBuf, tx: Tx, requests: Receiver<AiReq>) {
    let _ = std::thread::Builder::new().name("ai".into()).spawn(move || {
        let Some(home) = dirs::home_dir() else { return };
        let mut cache = load_cache(&cache_file);
        let mut visible = false;
        let mut last_fetch: Option<Instant> = None;
        loop {
            let cfg = cfg.lock().map(|c| c.clone()).unwrap_or_default();
            let every = Duration::from_secs(cfg.refresh_minutes.clamp(1, 240) * 60);
            // Bekle: görünmüyorken süresiz, görünürken bir sonraki yenilemeye kadar.
            let msg = if visible {
                let left = last_fetch.map_or(Duration::ZERO, |t| every.saturating_sub(t.elapsed()));
                requests.recv_timeout(left)
            } else {
                requests.recv().map_err(|_| RecvTimeoutError::Disconnected)
            };
            let mut manual = false;
            // Panel yeni göründü: veri 30 sn'den eskiyse hemen çekilir.
            let mut entered = false;
            let mut handle = |m: AiReq, visible: &mut bool| match m {
                AiReq::Refresh => manual = true,
                AiReq::Visible(v) => {
                    entered |= v && !*visible;
                    *visible = v;
                }
            };
            match msg {
                Ok(m) => handle(m, &mut visible),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            while let Ok(m) = requests.try_recv() {
                handle(m, &mut visible);
            }
            let now = Instant::now();
            let stale = last_fetch.is_none_or(|t| now.duration_since(t) >= FRESH_ENOUGH);
            let due = manual || (entered && visible && stale) || fetch_due(visible, last_fetch, every, now);
            if !cfg.enabled || !due {
                continue;
            }
            last_fetch = Some(now);
            let defs: Vec<providers::ProviderDef> = providers::registry()
                .into_iter()
                .filter(|d| cfg.providers.iter().any(|p| p.eq_ignore_ascii_case(d.id)))
                .collect();
            let (res_tx, res_rx) = mpsc::channel::<(usize, Result<Usage, String>)>();
            let mut pending = 0;
            for (i, def) in defs.iter().enumerate() {
                let env = Env::new(home.clone());
                let presence = (def.detect)(&env);
                let cached = cache.get(def.id);
                let mut state = ProviderState {
                    id: def.id,
                    name: def.name,
                    login_hint: def.login_hint,
                    presence,
                    status: match presence {
                        Presence::Ready => Status::Loading,
                        Presence::NoLogin => Status::SignIn,
                        Presence::NotInstalled => Status::Pending,
                    },
                    usage: cached.map(|c| c.usage.clone()),
                    fetched_at: cached.map(|c| c.fetched_at),
                };
                if presence != Presence::Ready {
                    state.usage = None;
                }
                if tx.send(AppEvent::Ai(Box::new(state))).is_err() {
                    return;
                }
                if presence == Presence::Ready {
                    pending += 1;
                    let fetch = def.fetch;
                    let res_tx = res_tx.clone();
                    std::thread::spawn(move || {
                        let _ = res_tx.send((i, fetch(&env)));
                    });
                }
            }
            drop(res_tx);
            let deadline = Instant::now() + Duration::from_secs(30);
            while pending > 0 {
                let left = deadline.saturating_duration_since(Instant::now());
                let Ok((i, result)) = res_rx.recv_timeout(left) else { break };
                pending -= 1;
                let def = &defs[i];
                let now = chrono::Utc::now().timestamp();
                let cached = cache.get(def.id).cloned();
                let state = match result {
                    Ok(usage) => {
                        cache.insert(def.id.to_string(), CacheEntry { usage: usage.clone(), fetched_at: now });
                        ProviderState {
                            id: def.id,
                            name: def.name,
                            login_hint: def.login_hint,
                            presence: Presence::Ready,
                            status: Status::Ok,
                            usage: Some(usage),
                            fetched_at: Some(now),
                        }
                    }
                    Err(e) => ProviderState {
                        id: def.id,
                        name: def.name,
                        login_hint: def.login_hint,
                        presence: Presence::Ready,
                        status: if e == "session expired" { Status::SignIn } else { Status::Error(e) },
                        usage: cached.as_ref().map(|c| c.usage.clone()),
                        fetched_at: cached.map(|c| c.fetched_at),
                    },
                };
                if tx.send(AppEvent::Ai(Box::new(state))).is_err() {
                    return;
                }
            }
            save_cache(&cache_file, &cache);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_detection() {
        assert_eq!(agent_kind(Some(r"& 'C:\bin\claude.exe'"), "pwsh"), Some("claude"));
        assert_eq!(agent_kind(None, "✳ Claude Code"), Some("claude"));
        assert_eq!(agent_kind(Some("codex"), "node"), Some("codex"));
        assert_eq!(agent_kind(None, "pwsh"), None);
    }

    #[test]
    fn fetches_only_while_visible() {
        let now = Instant::now();
        let every = Duration::from_secs(300);
        assert!(!fetch_due(false, None, every, now), "hidden panel never fetches");
        assert!(fetch_due(true, None, every, now), "first view fetches immediately");
        assert!(!fetch_due(true, Some(now - Duration::from_secs(10)), every, now));
        assert!(fetch_due(true, Some(now - Duration::from_secs(301)), every, now));
    }

    #[test]
    fn initial_states_come_from_cache() {
        let mut cache = HashMap::new();
        cache.insert(
            "claude".to_string(),
            CacheEntry { usage: Usage { windows: vec![], plan: Some("max".into()), note: None }, fetched_at: 5 },
        );
        let states = initial_states(&AiCfg::default(), &cache);
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].id, "claude");
        assert!(states[0].is_stale());
        let none = initial_states(&AiCfg { providers: vec!["codex".into()], ..AiCfg::default() }, &cache);
        assert!(none.is_empty());
    }
}
