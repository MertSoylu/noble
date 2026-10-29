//! Claude Code's background sessions (`claude --bg`), read with `claude agents --json --all`. Home lists them with
//! the open agent sessions, and a click attaches to one in a new tab (`claude attach <id>`). This is optional context:
//! every failure (no `claude`, an older version without `agents`, a timeout, output that does not parse) gives an
//! empty list without a message.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::Duration;

use serde_json::Value;

use crate::event::{AppEvent, Tx};
use crate::util;

/// A background Claude session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BgSession {
    /// The short id `claude attach` takes (`[A-Za-z0-9_-]`, see `valid_id`).
    pub id: String,
    /// The folder it was started in; `None` when it was missing or names a network path, which is never touched.
    pub cwd: Option<PathBuf>,
    /// Its name, cleaned of control characters (may be empty).
    pub name: String,
    /// The state Claude reports (`running`, `done`, `failed` …), lower case, cleaned like the name.
    pub state: String,
    /// Start time in Unix seconds (0 when unknown).
    pub started: i64,
}

/// How a session's state reads on Home.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BgKind {
    Working,
    NeedsYou,
    Done,
    Failed,
    Other,
}

impl BgSession {
    pub fn kind(&self) -> BgKind {
        match self.state.as_str() {
            "running" | "busy" | "working" | "active" | "starting" => BgKind::Working,
            "waiting" | "needs_input" | "needs-input" | "blocked" | "idle" | "paused" => BgKind::NeedsYou,
            "done" | "completed" | "finished" | "stopped" | "exited" => BgKind::Done,
            "failed" | "error" | "crashed" | "killed" => BgKind::Failed,
            _ => BgKind::Other,
        }
    }

    /// Finished sessions stay listed for a day after they started, so a result that came while you were away is
    /// still there; running ones always are.
    pub fn shown(&self, now: i64) -> bool {
        !matches!(self.kind(), BgKind::Done | BgKind::Failed) || now.saturating_sub(self.started) < SHOW_FINISHED_FOR
    }
}

/// How long a finished session stays on Home after it started (seconds).
const SHOW_FINISHED_FOR: i64 = 24 * 3600;
/// Longest `claude agents` may take.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// Most sessions kept from one answer.
const MAX_SESSIONS: usize = 32;

/// An id `claude attach` can be given on a command line without quoting: `[A-Za-z0-9_-]`, 1 to 64 characters,
/// and not starting with `-` (so it is never read as an option of `claude attach`).
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && !id.starts_with('-')
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The command line that attaches to a session; `None` for an id that is not `valid_id`.
pub fn attach_command(id: &str) -> Option<String> {
    valid_id(id).then(|| format!("claude attach {id}"))
}

/// The background sessions `claude` knows about.
pub fn fetch(claude: &Path) -> Option<Vec<BgSession>> {
    let mut cmd = util::command_for(claude);
    cmd.args(["agents", "--json", "--all"]).env("NO_COLOR", "1");
    Some(parse(&crate::ai::run_command(cmd, FETCH_TIMEOUT).ok()?))
}

/// Background thread: each request is answered with the current list (empty on any failure). `claude` is looked up
/// once; without it, or in an isolated run (`ai::isolated`), no program runs and every answer is empty.
pub fn spawn(tx: Tx, requests: Receiver<()>) {
    let _ = std::thread::Builder::new().name("bg-sessions".into()).spawn(move || {
        let claude = if crate::ai::isolated() { None } else { util::which("claude") };
        for () in requests {
            let list = claude.as_deref().and_then(fetch).unwrap_or_default();
            if tx.send(AppEvent::BgSessions(list)).is_err() {
                break;
            }
        }
    });
}

/// Text from the other program, made safe to draw: control characters become spaces, the length is capped.
fn clean(text: &str, max: usize) -> String {
    let text: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    util::truncate(text.trim(), max)
}

/// Parses the `claude agents --json --all` output: only the `background` entries with a usable id, newest first.
/// Interactive sessions are left out (NOBLE's own panes show those).
pub fn parse(json: &str) -> Vec<BgSession> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else { return Vec::new() };
    let mut out: Vec<BgSession> = items
        .iter()
        .filter(|v| v.get("kind").and_then(Value::as_str) == Some("background"))
        .filter_map(|v| {
            let id = v.get("id")?.as_str()?;
            if !valid_id(id) {
                return None;
            }
            let cwd = v
                .get("cwd")
                .and_then(Value::as_str)
                .filter(|c| !c.is_empty() && !crate::term::pane::is_network_path(c))
                .map(PathBuf::from);
            let state = v.get("state").or_else(|| v.get("status")).and_then(Value::as_str).unwrap_or_default();
            let started_ms = v.get("startedAt").and_then(Value::as_i64).unwrap_or(0);
            Some(BgSession {
                id: id.to_string(),
                cwd,
                name: clean(v.get("name").and_then(Value::as_str).unwrap_or_default(), 80),
                state: clean(state, 16).to_lowercase(),
                started: started_ms / 1000,
            })
        })
        .collect();
    // Running and waiting sessions are kept before finished ones, so the cap never drops an active session for an
    // older finished one; the kept list is then newest first again.
    out.sort_by_key(|s| (matches!(s.kind(), BgKind::Done | BgKind::Failed), std::cmp::Reverse(s.started)));
    out.truncate(MAX_SESSIONS);
    out.sort_by_key(|s| std::cmp::Reverse(s.started));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_background_sessions_only() {
        let json = r#"[
          {"id":"922d0aee","cwd":"C:\\work\\app","kind":"background","startedAt":1790202214094,
           "sessionId":"922d0aee-798f","name":"Fix the build","state":"failed"},
          {"id":"a25dfa10","cwd":"/home/me/app","kind":"background","startedAt":1792869236026,
           "name":"tests\u001b[31m red","state":"Running"},
          {"pid":10452,"cwd":"C:\\work","kind":"interactive","startedAt":1793284226489,"status":"busy"}
        ]"#;
        let list = parse(json);
        assert_eq!(list.len(), 2, "{list:?}");
        // Newest first; names and states are cleaned.
        assert_eq!(list[0].id, "a25dfa10");
        assert_eq!(list[0].name, "tests [31m red");
        assert_eq!((list[0].state.as_str(), list[0].kind()), ("running", BgKind::Working));
        assert_eq!(list[0].started, 1_792_869_236);
        assert_eq!(list[1].cwd, Some(PathBuf::from(r"C:\work\app")));
        assert_eq!(list[1].kind(), BgKind::Failed);
    }

    #[test]
    fn refuses_bad_ids_network_folders_and_garbage() {
        let json = r#"[
          {"id":"x; rm -rf ~","kind":"background"},
          {"id":"","kind":"background"},
          {"id":"ok-1","kind":"background","cwd":"\\\\server\\share\\x"},
          {"id":"ok_2","kind":"background","cwd":"//server/share"}
        ]"#;
        let list = parse(json);
        assert_eq!(list.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["ok-1", "ok_2"]);
        assert!(list.iter().all(|s| s.cwd.is_none()), "a network folder is dropped: {list:?}");
        for bad in ["", "{", "{}", "null", "[1, \"x\", null]", "[{\"kind\":\"background\"}]"] {
            assert!(parse(bad).is_empty(), "{bad}");
        }
        assert_eq!(attach_command("ok-1").as_deref(), Some("claude attach ok-1"));
        assert_eq!(attach_command("a b"), None);
        assert!(!valid_id(&"a".repeat(65)));
        // A flag-shaped id would be read as an option of `claude attach`.
        assert_eq!(attach_command("--help"), None);
        assert!(!valid_id("-x"));
    }

    #[test]
    fn the_cap_keeps_running_sessions_before_finished_ones() {
        let mut items: Vec<String> = (0..40)
            .map(|i| {
                format!(
                    r#"{{"id":"done-{i}","kind":"background","state":"done","startedAt":{}}}"#,
                    2_000_000_000_000i64 + i64::from(i) * 1000
                )
            })
            .collect();
        // Started before every finished session, so it was the oldest entry before the cap.
        items.push(r#"{"id":"run-1","kind":"background","state":"running","startedAt":1000000000000}"#.into());
        let list = parse(&format!("[{}]", items.join(",")));
        assert_eq!(list.len(), MAX_SESSIONS);
        assert!(list.iter().any(|s| s.id == "run-1"), "the running session is kept");
        assert_eq!(list[0].id, "done-39", "newest first");
    }

    #[test]
    fn finished_sessions_stay_listed_for_a_day() {
        let now = 1_800_000_000;
        let s = |state: &str, started: i64| BgSession {
            id: "a".into(),
            cwd: None,
            name: String::new(),
            state: state.into(),
            started,
        };
        assert!(s("running", now - 10 * 86_400).shown(now));
        assert!(s("done", now - 3600).shown(now));
        assert!(!s("done", now - 2 * 86_400).shown(now));
        assert!(!s("failed", now - 2 * 86_400).shown(now));
        assert!(s("something-new", now - 2 * 86_400).shown(now), "an unknown state is not hidden");
    }
}
