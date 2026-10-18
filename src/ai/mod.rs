//! AI subscription quota tracking (Claude Code, Codex, Antigravity, OpenCode Go, Kilo Code, Command Code).
//!
//! Credentials are read only from CLI sessions already on this machine and are
//! sent only to the matching provider in the request header; they are never
//! printed or logged. Tokens are never refreshed (so we never race the CLIs' own
//! refresh flow); an expired session falls back to a "sign in" hint.

pub mod json;
pub mod providers;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::AiCfg;
use crate::event::{AppEvent, Tx};
use crate::util;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Window {
    pub label: String,
    /// Percentage used (0..100).
    pub used: u8,
    /// Reset time (unix seconds).
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
    /// Not checked yet (shown from the cache at startup).
    Pending,
    Loading,
    Ok,
    SignIn,
    /// Signed in, but the token no longer works: the cached numbers stay on Home, dimmed, with a hint.
    Expired,
    Error(String),
}

/// The error text providers return for an expired or rejected login.
pub const SESSION_EXPIRED: &str = "session expired";

/// Maps a provider's fetch error to the status shown: an expired login stays on Home (cached bars and a
/// hint), "no plan" (signed in, nothing metered) is hidden like a signed-out provider.
pub fn failure_status(e: String) -> Status {
    match e.as_str() {
        SESSION_EXPIRED => Status::Expired,
        "no plan" => Status::SignIn,
        _ => Status::Error(e),
    }
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
    /// Is the usage data from the last successful fetch (cached/stale)?
    pub fn is_stale(&self) -> bool {
        !matches!(self.status, Status::Ok)
    }

    /// Percent of the fullest window (for the status bar).
    pub fn peak(&self) -> Option<u8> {
        self.usage.as_ref()?.windows.iter().map(|w| w.used).max()
    }
}

/// Readable name of a quota window ("5H" → "5h", "WEEK" → "week").
pub fn window_name(label: &str) -> String {
    match label {
        "5H" => "5h".into(),
        "WEEK" => "week".into(),
        "OPUS" => "opus".into(),
        other => other.to_lowercase(),
    }
}

/// AI agent running in a pane: from the launcher command or the window title.
pub fn agent_kind(command: Option<&str>, label: &str) -> Option<&'static str> {
    // When opened from a launcher the command's file name is authoritative.
    // The path is split on both separators: on Linux `Path` does not treat `\` as a separator.
    let stem = command
        .and_then(|c| c.trim().trim_matches(['&', ' ', '\'', '"']).rsplit(['/', '\\']).next())
        .and_then(|name| std::path::Path::new(name).file_stem())
        .map(|s| s.to_string_lossy().to_lowercase());
    const BY_COMMAND: [(&str, &str); 13] = [
        ("claude", "claude"),
        ("codex", "codex"),
        ("opencode", "opencode"),
        ("copilot", "copilot"),
        ("agy", "antigravity"),
        ("pi", "pi"),
        ("omp", "oh-my-pi"),
        ("freebuff", "freebuff"),
        ("grok", "grok"),
        ("cursor-agent", "cursor"),
        ("command-code", "command code"),
        ("cline", "cline"),
        ("kilo", "kilo"),
    ];
    if let Some(stem) = stem.as_deref()
        && let Some((_, kind)) = BY_COMMAND.iter().find(|(c, _)| *c == stem)
    {
        return Some(kind);
    }
    // Window title for hand-typed commands: only distinctive names.
    let hay = format!("{} {}", command.unwrap_or(""), label).to_lowercase();
    ["claude", "codex", "opencode", "copilot", "antigravity", "freebuff"].into_iter().find(|k| hay.contains(k))
}

/// Working environment of the providers (tests can pass a fake home directory).
pub struct Env {
    pub home: PathBuf,
    pub agent: ureq::Agent,
    /// `home` is the user's real home: the system credential stores (Keychain, Credential
    /// Manager, Secret Service) belong to it. A test's fake home never reaches them.
    pub system_store: bool,
}

impl Env {
    pub fn new(home: PathBuf) -> Env {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .http_status_as_error(false)
            .user_agent("noble/1.0")
            .build();
        let system_store = dirs::home_dir().as_deref() == Some(home.as_path());
        Env { home, agent: ureq::Agent::new_with_config(config), system_store }
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

/// JSON request; turns network/status/parse failures into a short message.
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
        401 | 403 => Err(SESSION_EXPIRED.into()),
        429 => Err("rate limited".into()),
        s => Err(format!("HTTP {s}")),
    }
}

/// Line-based JSON-RPC (codex app-server): initialize handshake, single call.
pub fn stdio_rpc(
    program: &Path,
    args: &[&str],
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<Value, String> {
    let mut cmd = util::command_for(program);
    cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    // Started like `run_command`'s commands: the guard ends the whole tree on every exit below (a failed write
    // and a timeout included), and shutdown reaches it. Before, an early `?` left the child running and unreaped.
    let mut tracked = spawn_tracked(cmd)?;
    let mut stdin = tracked.0.stdin.take().ok_or("no stdin")?;
    let stdout = tracked.0.stdout.take().ok_or("no stdout")?;
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
    result
}

/// Runs a program and returns its stdout; the process is killed on timeout.
/// Output is capped at 1 MiB, stderr is not read (error text is never logged).
pub fn run_capture(program: &Path, args: &[&str], cwd: Option<&Path>, timeout: Duration) -> Result<String, String> {
    let mut cmd = util::command_for(program);
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    run_command(cmd, timeout)
}

/// Runs a prepared command and returns its stdout. Whatever the outcome, the command's whole process tree is
/// killed before the call returns (see [`kill_tree`]), so no grandchild keeps the stdout pipe open.
/// Stdout is piped and capped at 1 MiB, stderr is discarded. These are background calls only: the child
/// gets a process group of its own (Unix), so it is never the terminal's foreground group.
pub fn run_command(mut cmd: std::process::Command, timeout: Duration) -> Result<String, String> {
    use std::io::Read;
    const CAP: u64 = 1 << 20;
    cmd.stdout(Stdio::piped()).stderr(Stdio::null());
    let mut tracked = spawn_tracked(cmd)?;
    let stdout = tracked.0.stdout.take().ok_or("no stdout")?;
    let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>();
    // The reader is never joined: after a timeout it may stay blocked on a pipe that a surviving process
    // holds, and the caller must not wait for it. `stop` closes that pipe's last holder.
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.take(CAP + 1).read_to_end(&mut buf);
        let _ = out_tx.send(buf);
    });
    let result = match out_rx.recv_timeout(timeout) {
        Ok(buf) if buf.len() as u64 > CAP => Err("output too large".to_string()),
        Ok(buf) => Ok(String::from_utf8_lossy(&buf).into_owned()),
        Err(_) => Err("timeout".to_string()),
    };
    // Every outcome ends here (the guard drops): a helper that the command started may still run after the command
    // itself finished, so its whole tree is killed and the child is reaped.
    result
}

/// Background commands that are running now, by pid (on Unix the pid is also the process group id). A command is
/// added when it starts and removed when it is stopped. `closed` is set on shutdown (`kill_background_children`):
/// from then on no command starts, so none can outlive NOBLE. Both live under the one lock, so a command either
/// lands in `pids` before a shutdown or sees `closed`.
struct Registry {
    closed: bool,
    pids: Vec<u32>,
}

static RUNNING: Mutex<Registry> = Mutex::new(Registry { closed: false, pids: Vec::new() });

fn running() -> MutexGuard<'static, Registry> {
    RUNNING.lock().unwrap_or_else(|e| e.into_inner())
}

/// Held by every test that starts a real background command. The shutdown tests end every command in the registry
/// (and close it until they reopen it), so a command of another test must not run meanwhile: each such test takes
/// this lock first, and the registry is reopened before it is released.
#[cfg(test)]
pub(crate) static SPAWN_TESTS: Mutex<()> = Mutex::new(());

/// A started background command. Dropping it ends the whole process tree and reaps the child, so every exit path
/// of the caller (an early return, a timeout, a failed write) cleans up.
pub(crate) struct Tracked(std::process::Child);

impl Drop for Tracked {
    fn drop(&mut self) {
        stop(&mut self.0);
        let _ = self.0.wait();
    }
}

/// Starts a background command and registers it. Unix: the child leads a process group of its own, so the tree is
/// killed as one. Windows: there are no process groups; `kill_tree` walks the tree with `taskkill /T`. Fails once
/// shutdown has begun; the child is then killed again by its guard.
fn spawn_tracked(mut cmd: std::process::Command) -> Result<Tracked, String> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let child = cmd.spawn().map_err(|_| "cli not runnable".to_string())?;
    let pid = child.id();
    let tracked = Tracked(child);
    // The lock is taken after the spawn (a spawn can take a while on Windows), not across it.
    let closed = {
        let mut reg = running();
        if !reg.closed {
            reg.pids.push(pid);
        }
        reg.closed
    };
    if closed {
        // `tracked` drops here, after the lock is released: its `stop` takes the lock itself.
        return Err("shutting down".into());
    }
    Ok(tracked)
}

/// Ends the process tree of `child` and forgets it. Unix: the group is killed under the registry lock, which keeps
/// the group id from being reused meanwhile (the leader is not reaped yet, so it still holds the id). Windows: the
/// tree is killed outside the lock, which a slow `taskkill` must not hold up for every other command. The pid
/// cannot be reused there while `child` still holds its handle.
fn stop(child: &mut std::process::Child) {
    let pid = child.id();
    {
        let mut reg = running();
        reg.pids.retain(|p| *p != pid);
        // Unix: always. The group can outlive its leader, and the leader (not reaped yet) still holds the group id.
        #[cfg(unix)]
        kill_tree(pid);
    }
    // Windows: `taskkill` finds the tree from its root, so once the root has exited there is nothing to find and
    // the process is not started. `try_wait` reaps only on exit, and Windows keeps the pid until the handle closes.
    #[cfg(windows)]
    if matches!(child.try_wait(), Ok(None)) {
        kill_tree(pid);
    }
    let _ = child.kill();
}

/// Ends every background command that is still running and closes the registry, so none starts again. Called on
/// NOBLE's shutdown path (`App::shutdown`), so a command that hangs cannot outlive NOBLE. Each one is killed as its
/// whole tree, as on a timeout; the `run_command` calls then return as usual.
///
/// The lock stays held while the trees are killed (Windows: `taskkill` runs here, so this is shutdown-only). A pid
/// listed here has not been reaped yet; a command that ends meanwhile waits in `stop`, so its pid cannot be reused
/// under a `taskkill` aimed at it.
///
/// Tests: this kills every command in the registry, not only the caller's. The shutdown tests therefore hold
/// `SPAWN_TESTS`, which every test that starts a real command also takes.
pub fn kill_background_children() {
    close_and_kill(&mut running());
}

/// The same from a panic hook on the main thread, where the panicking code may still hold the registry lock. It
/// does not wait for the lock: when the lock is busy it does nothing, and the unwinding (`main.rs`) then reaches
/// `kill_background_children`.
pub fn kill_background_children_if_free() {
    let mut reg = match RUNNING.try_lock() {
        Ok(reg) => reg,
        Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return,
    };
    close_and_kill(&mut reg);
}

fn close_and_kill(reg: &mut Registry) {
    reg.closed = true;
    for pid in reg.pids.drain(..) {
        kill_tree(pid);
    }
}

/// Tests only: reopens the registry after a shutdown test has closed it.
#[cfg(test)]
fn reopen_registry() {
    running().closed = false;
}

/// Kills the process tree led by `pid`. Unix: SIGKILL to the process group (the group id is the pid, set by
/// `spawn_tracked`). Windows: `taskkill /T /F` ends the tree (Windows has no process groups; Unix does not run
/// taskkill). `taskkill` finds the tree by parent links, so a grandchild whose parent has already exited is
/// missed: the caller never waits for a pipe that such a process keeps open.
fn kill_tree(pid: u32) {
    #[cfg(unix)]
    {
        // A negative pid signals the whole group. 0 would be our own group and 1 is init: neither is ours.
        if let Ok(pgid) = libc::pid_t::try_from(pid)
            && pgid > 1
        {
            // SAFETY: a plain syscall with a valid signal; a group that is already gone yields ESRCH, ignored.
            unsafe {
                libc::kill(-pgid, libc::SIGKILL);
            }
        }
    }
    #[cfg(windows)]
    {
        // Windows: taskkill ends the tree. Unix (above) signals the process group instead.
        // `command_for` hides the console window; the System32 copy is preferred over a PATH entry of that name.
        let _ = util::command_for(&taskkill_program())
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Windows: the `taskkill.exe` that ships with the OS, else the one on PATH, else the bare name.
#[cfg(windows)]
fn taskkill_program() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("System32").join("taskkill.exe"))
        .filter(|p| p.is_file())
        .or_else(|| util::which("taskkill"))
        .unwrap_or_else(|| PathBuf::from("taskkill"))
}

// ─── Cache ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheEntry {
    pub usage: Usage,
    pub fetched_at: i64,
}

pub fn load_cache(file: &Path) -> HashMap<String, CacheEntry> {
    std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Several windows run a collector and share this file: the write is atomic (temp file + rename),
/// so a reader never sees a half-written cache.
fn save_cache(file: &Path, cache: &HashMap<String, CacheEntry>) {
    crate::store::write_json(file, cache);
}

/// Environment variable that keeps NOBLE away from the user's real machine-wide integrations: the AI
/// collector never reads credentials or calls the network, and the Claude Code hooks never touch
/// `~/.claude/settings.json`. `NOBLE_HOME` only moves NOBLE's own config and data, so the end-to-end
/// tests (which run the real binary) set this as well. Unset in production; any value turns it on.
pub const ISOLATED_ENV: &str = "NOBLE_NO_SYSTEM_INTEGRATIONS";

/// Is [`ISOLATED_ENV`] set?
pub fn isolated() -> bool {
    std::env::var_os(ISOLATED_ENV).is_some()
}

/// Ids of the providers installed on this machine (even if not signed in).
pub fn installed_providers() -> Vec<&'static str> {
    if isolated() {
        return Vec::new();
    }
    let Some(home) = dirs::home_dir() else { return Vec::new() };
    let env = Env::new(home);
    providers::registry().into_iter().filter(|d| (d.detect)(&env) != Presence::NotInstalled).map(|d| d.id).collect()
}

/// First states to show at startup: those that have cached data.
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

/// Requests to the collector.
pub enum AiReq {
    /// Manual refresh (R, settings change): fetch right away.
    Refresh,
    /// Quota panel visibility. While hidden nothing is requested (battery friendly);
    /// when it becomes visible and the data is stale it is fetched immediately.
    Visible(bool),
}

/// Data younger than this is not re-fetched when returning to Home (fast tab switches).
const FRESH_ENOUGH: Duration = Duration::from_secs(30);

/// Is it time for the next fetch?
pub fn fetch_due(visible: bool, last: Option<Instant>, every: Duration, now: Instant) -> bool {
    visible && last.is_none_or(|t| now.duration_since(t) >= every)
}

/// How long the collector waits for a request before it looks at the clock again.
/// `None` = until a request arrives: a hidden panel or a disabled collector has nothing to do on its
/// own (waking up anyway would spin, as `last` never advances while nothing is fetched).
fn wait_time(enabled: bool, visible: bool, last: Option<Instant>, every: Duration, now: Instant) -> Option<Duration> {
    if !enabled || !visible {
        return None;
    }
    Some(last.map_or(Duration::ZERO, |t| every.saturating_sub(now.duration_since(t))))
}

/// Collector thread: detect → fetch → send → write cache → wait.
/// Runs only while the quota panel is visible.
pub fn spawn(
    shared_cfg: std::sync::Arc<std::sync::Mutex<AiCfg>>,
    cache_file: PathBuf,
    tx: Tx,
    requests: Receiver<AiReq>,
) {
    let _ = std::thread::Builder::new().name("ai".into()).spawn(move || {
        let Some(home) = dirs::home_dir() else { return };
        let mut cache = load_cache(&cache_file);
        let mut visible = false;
        let mut last_fetch: Option<Instant> = None;
        // The settings as they are right now (the app updates them before it sends `Refresh`).
        let load = || {
            let c = shared_cfg.lock().map(|c| c.clone()).unwrap_or_default();
            let every = Duration::from_secs(c.refresh_minutes.clamp(1, 240) * 60);
            let enabled = c.enabled && !isolated();
            (c, every, enabled)
        };
        loop {
            let (_, every, enabled) = load();
            // Wait: indefinitely while hidden or disabled, until the next refresh while visible.
            let msg = match wait_time(enabled, visible, last_fetch, every, Instant::now()) {
                Some(left) => requests.recv_timeout(left),
                None => requests.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            let mut manual = false;
            // Panel just appeared: fetch immediately if the data is older than 30 s.
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
            // Re-read: the config may have changed while waiting (e.g. AI just turned off).
            let (cfg, every, enabled) = load();
            let now = Instant::now();
            let stale = last_fetch.is_none_or(|t| now.duration_since(t) >= FRESH_ENOUGH);
            let due = manual || (entered && visible && stale) || fetch_due(visible, last_fetch, every, now);
            if !enabled || !due {
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
                        // "no plan": signed in but no metered plan → hidden from the panel;
                        // an expired login stays visible with its cached numbers.
                        status: failure_status(e),
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
        assert_eq!(agent_kind(Some(r"& 'C:\Users\a\AppData\Local\agy\bin\agy.exe'"), "pwsh"), Some("antigravity"));
        assert_eq!(agent_kind(Some("opencode"), "pwsh"), Some("opencode"));
        assert_eq!(agent_kind(None, "GitHub Copilot"), Some("copilot"));
        assert_eq!(agent_kind(None, "pwsh"), None);
        assert_eq!(agent_kind(None, "legacy agy notes"), None);
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
    fn disabled_or_hidden_collector_sleeps_until_asked() {
        let now = Instant::now();
        let every = Duration::from_secs(300);
        // Regression: disabled + visible + never fetched used to return a zero wait forever (100% CPU).
        assert_eq!(wait_time(false, true, None, every, now), None);
        assert_eq!(wait_time(false, true, Some(now - Duration::from_secs(900)), every, now), None);
        assert_eq!(wait_time(true, false, None, every, now), None);
        assert_eq!(wait_time(true, true, None, every, now), Some(Duration::ZERO));
        let left = wait_time(true, true, Some(now - Duration::from_secs(100)), every, now);
        assert_eq!(left, Some(Duration::from_secs(200)));
        // Overdue: fetch right away, never a negative wait.
        assert_eq!(wait_time(true, true, Some(now - Duration::from_secs(900)), every, now), Some(Duration::ZERO));
    }

    #[test]
    fn cache_is_written_atomically() {
        let dir = std::env::temp_dir().join(format!("noble-ai-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("ai-cache.json");
        let mut cache = HashMap::new();
        cache.insert(
            "claude".to_string(),
            CacheEntry { usage: Usage { windows: vec![], plan: Some("max".into()), note: None }, fetched_at: 7 },
        );
        save_cache(&file, &cache);
        save_cache(&file, &cache);
        assert_eq!(load_cache(&file).get("claude").map(|e| e.fetched_at), Some(7));
        // No temp files are left behind after the rename.
        let names: Vec<String> =
            std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["ai-cache.json".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn expired_login_is_told_apart_from_no_plan() {
        assert_eq!(failure_status(SESSION_EXPIRED.into()), Status::Expired);
        assert_eq!(failure_status("no plan".into()), Status::SignIn);
        assert_eq!(failure_status("offline".into()), Status::Error("offline".into()));
        let expired = ProviderState {
            id: "claude",
            name: "Claude Code",
            login_hint: "claude",
            presence: Presence::Ready,
            status: Status::Expired,
            usage: None,
            fetched_at: Some(1),
        };
        assert!(expired.is_stale(), "cached numbers of an expired login are stale");
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

    /// A directory under the system temp dir, removed when dropped (also when the test fails).
    struct ScratchDir(PathBuf);

    impl ScratchDir {
        fn new(name: &str) -> ScratchDir {
            let dir = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            ScratchDir(dir)
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Unix: `ps` reports a zombie as `Z`; a zombie has been killed but not reaped yet, so it counts as gone.
    /// `None` when `ps` cannot run: the caller then skips the check.
    #[cfg(unix)]
    fn process_alive(pid: u32) -> Option<bool> {
        let out = std::process::Command::new("ps").args(["-o", "stat=", "-p", &pid.to_string()]).output().ok()?;
        let stat = String::from_utf8_lossy(&out.stdout);
        let stat = stat.trim();
        Some(!stat.is_empty() && !stat.starts_with('Z'))
    }

    /// Unix: waits up to 5 s for `pid` to be gone (SIGKILL is asynchronous). `None` when `ps` cannot run.
    #[cfg(unix)]
    fn wait_until_gone(pid: u32) -> Option<bool> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if !process_alive(pid)? {
                return Some(true);
            }
            if Instant::now() >= deadline {
                return Some(false);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// A background `sleep` started by the shell inherits the stdout pipe, so the reader can only finish
    /// when it dies. The timeout must kill it too, and the caller must not wait for the pipe.
    #[cfg(unix)]
    #[test]
    fn timeout_kills_grandchild_holding_the_pipe() {
        let _serial = SPAWN_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = ScratchDir::new("noble-run-command");
        let pid_file = dir.0.join("grandchild.pid");
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c").arg(r#"sleep 30 & echo $! > "$1"; wait"#).arg("sh").arg(&pid_file);
        let started = Instant::now();
        let result = run_command(cmd, Duration::from_secs(2));
        assert_eq!(result, Err("timeout".to_string()));
        assert!(started.elapsed() < Duration::from_secs(10), "the caller waited for the pipe");
        let pid: u32 = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
        match wait_until_gone(pid) {
            Some(gone) => assert!(gone, "the grandchild survived the timeout"),
            // `ps` is missing on this system: nothing to check the grandchild with (the pipe check above ran).
            None => eprintln!("ps is not installed: the grandchild check is skipped"),
        }
    }

    /// Reopens the registry when a test ends, also when it fails: the shutdown tests close it for the whole process.
    struct Reopen;

    impl Drop for Reopen {
        fn drop(&mut self) {
            reopen_registry();
        }
    }

    /// NOBLE's shutdown ends a command that is still running: the call returns, and on Unix its grandchild is gone
    /// too. Windows checks the return only (the grandchild is not observable there without extra tooling).
    #[test]
    fn shutdown_ends_a_running_command() {
        let _serial = SPAWN_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let _reopen = Reopen;
        let dir = ScratchDir::new("noble-shutdown-command");
        let pid_file = dir.0.join("grandchild.pid");
        #[cfg(unix)]
        let cmd = {
            let mut cmd = std::process::Command::new("sh");
            cmd.arg("-c").arg(r#"sleep 30 & echo $! > "$1"; wait"#).arg("sh").arg(&pid_file);
            cmd
        };
        // Windows: `ping` runs for about 30 s; the command itself is what must end.
        #[cfg(windows)]
        let cmd = {
            let mut cmd = std::process::Command::new("cmd");
            cmd.args(["/C", "ping", "-n", "30", "127.0.0.1"]);
            cmd
        };
        let (done_tx, done_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = done_tx.send(run_command(cmd, Duration::from_secs(60)));
        });
        // Running once registered; on Unix also once the grandchild has written its pid.
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && (running().pids.is_empty() || !pid_file_ready(&pid_file)) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!running().pids.is_empty(), "the command never registered");
        kill_background_children();
        let _ = done_rx.recv_timeout(Duration::from_secs(20)).expect("the shutdown did not end the command");
        #[cfg(unix)]
        {
            let pid: u32 = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
            match wait_until_gone(pid) {
                Some(gone) => assert!(gone, "the grandchild survived the shutdown"),
                None => eprintln!("ps is not installed: the grandchild check is skipped"),
            }
        }
    }

    /// After shutdown no command starts: the call fails at once, and nothing is left running.
    #[test]
    fn commands_do_not_start_after_shutdown() {
        let _serial = SPAWN_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let _reopen = Reopen;
        kill_background_children();
        let (program, args): (&str, &[&str]) = if cfg!(windows) { ("cmd", &["/C", "exit"]) } else { ("true", &[]) };
        let mut cmd = std::process::Command::new(program);
        cmd.args(args);
        assert_eq!(run_command(cmd, Duration::from_secs(10)), Err("shutting down".to_string()));
        assert!(running().pids.is_empty(), "a command was registered after shutdown");
    }

    /// The codex-style RPC child never answers: the timeout ends its whole tree (Unix: the grandchild too), and the
    /// caller does not wait for it. Windows: `cmd` starts `ping`, which `taskkill /T` ends with it.
    #[test]
    fn rpc_timeout_ends_the_child_tree() {
        let _serial = SPAWN_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = ScratchDir::new("noble-stdio-rpc");
        let pid_file = dir.0.join("grandchild.pid");
        let started = Instant::now();
        #[cfg(unix)]
        let (program, args) = (
            Path::new("sh"),
            vec![
                "-c".to_string(),
                r#"sleep 30 & echo $! > "$1"; wait"#.to_string(),
                "sh".to_string(),
                pid_file.display().to_string(),
            ],
        );
        #[cfg(windows)]
        let (program, args) =
            (Path::new("cmd"), vec!["/C".to_string(), "ping".into(), "-n".into(), "30".into(), "127.0.0.1".into()]);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let result = stdio_rpc(program, &args, "account/rateLimits/read", Value::Null, Duration::from_secs(2));
        assert_eq!(result.err().as_deref(), Some("timeout"));
        assert!(started.elapsed() < Duration::from_secs(10), "the caller waited for the child");
        #[cfg(unix)]
        {
            let pid: u32 = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
            match wait_until_gone(pid) {
                Some(gone) => assert!(gone, "the grandchild survived the timeout"),
                None => eprintln!("ps is not installed: the grandchild check is skipped"),
            }
        }
    }

    /// Whether the grandchild has written its pid. Unix only: Windows has no such file, so it is ready. The file
    /// exists before its content is written, so the pid itself must parse.
    fn pid_file_ready(pid_file: &Path) -> bool {
        !cfg!(unix) || std::fs::read_to_string(pid_file).is_ok_and(|t| t.trim().parse::<u32>().is_ok())
    }

    /// Windows: `cmd` starts `ping`, which inherits the stdout pipe; `taskkill /T` ends both on timeout.
    /// Only the caller's prompt return is asserted: the grandchild is not observable here without extra
    /// tooling, so this test cannot prove the kill itself.
    #[cfg(windows)]
    #[test]
    fn timeout_returns_promptly_with_grandchild_holding_the_pipe() {
        let _serial = SPAWN_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/C", "ping", "-n", "30", "127.0.0.1"]);
        let started = Instant::now();
        let result = run_command(cmd, Duration::from_secs(2));
        assert_eq!(result, Err("timeout".to_string()));
        assert!(started.elapsed() < Duration::from_secs(10), "the caller waited for the pipe");
    }
}
