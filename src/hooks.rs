//! Claude Code integration: Claude's hooks run `noble hook <event>`, which
//! writes the pane's state to a small file in the data folder; an open NOBLE
//! reads those files and shows the session's real state (running, waiting for
//! you, done). Hooks are added to `~/.claude/settings.json` only when the user
//! turns them on in Settings; the file is backed up when that happens.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::term::layout::PaneId;

/// Claude Code event → `noble hook` argument.
pub const EVENTS: [(&str, &str); 7] = [
    ("UserPromptSubmit", "prompt"),
    ("Stop", "stop"),
    ("Notification", "notification"),
    ("SessionStart", "session-start"),
    ("SessionEnd", "session-end"),
    ("SubagentStart", SUBAGENT_START),
    ("SubagentStop", SUBAGENT_STOP),
];

/// The `noble hook` argument that ends a session (Claude's `SessionEnd`).
pub const SESSION_END: &str = "session-end";

/// A subagent started / finished. Claude's `Stop` fires when the main answer ends, even
/// while background subagents keep working, so NOBLE counts the running ones: each has a
/// marker file next to the pane's record (`<instance>-<pane>.<agent_id>.sub`).
pub const SUBAGENT_START: &str = "subagent-start";
pub const SUBAGENT_STOP: &str = "subagent-stop";

/// A subagent marker older than this no longer counts, in case Claude never reported the
/// end (a crash, a killed process), so a session cannot stay "working" forever.
const SUBAGENT_MAX_AGE_SECS: u64 = 3 * 3600;

/// Notifications that ask nothing of the user: the idle reminder Claude sends a while after
/// its answer (it is already "your turn", and it also comes while background subagents still
/// run) and a background subagent finishing (the subagent hooks track that).
const QUIET_NOTIFICATIONS: [&str; 2] = ["idle_prompt", "agent_completed"];
/// The idle reminder's text, for Claude versions that do not send `notification_type`.
const IDLE_MESSAGE: &str = "Claude is waiting for your input";

/// The shared marker used to recognize our hook commands.
const MARKER: &str = " hook ";

/// The last event for a pane.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HookRecord {
    pub event: String,
    #[serde(default)]
    pub message: Option<String>,
    pub ts: i64,
    /// Subagents of the session still running (counted from the marker files, not stored).
    #[serde(skip)]
    pub subagents: usize,
}

/// `~/.claude/settings.json`; `None` without a home directory or when `ai::ISOLATED_ENV` is set
/// (tests that run the real binary must never install, upgrade or remove the user's hooks).
pub fn settings_path() -> Option<PathBuf> {
    if crate::ai::isolated() {
        return None;
    }
    Some(dirs::home_dir()?.join(".claude").join("settings.json"))
}

/// Base of the hook command: `noble` from PATH if present, else the running file's full path.
pub fn command_base() -> String {
    if crate::util::which("noble").is_some() {
        return "noble".into();
    }
    match std::env::current_exe() {
        Ok(p) => format!("\"{}\"", p.display()),
        Err(_) => "noble".into(),
    }
}

/// Is `command` NOBLE's own hook command: `<base> hook <arg>` where `<arg>` is one of the known
/// events and `<base>` is `noble`, `noble-dev` or a (possibly quoted) path to a `noble` executable.
fn is_noble_command(command: &str) -> bool {
    // The last " hook " is ours: the executable's path may contain one too (`/opt/my hook tools/noble`).
    let Some((base, arg)) = command.rsplit_once(MARKER) else { return false };
    if !EVENTS.iter().any(|(_, a)| *a == arg.trim()) {
        return false;
    }
    let base = base.trim().trim_matches('"');
    let file = base.rsplit(['/', '\\']).next().unwrap_or(base).to_lowercase();
    let stem = file.strip_suffix(".exe").unwrap_or(&file);
    stem == "noble" || stem == "noble-dev"
}

fn is_noble_hook(entry: &Value) -> bool {
    entry.get("hooks").and_then(Value::as_array).is_some_and(|hooks| {
        hooks.iter().any(|h| h.get("command").and_then(Value::as_str).is_some_and(is_noble_command))
    })
}

fn read_settings(path: &Path) -> Result<Value, String> {
    match std::fs::read_to_string(path) {
        Ok(text) if crate::util::strip_bom(&text).trim().is_empty() => Ok(json!({})),
        Ok(text) => serde_json::from_str(crate::util::strip_bom(&text))
            .map_err(|e| format!("cannot parse {}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

fn write_settings(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // The backup keeps the file as it was before NOBLE first touched it; later writes must not
    // overwrite it with an already edited copy.
    let backup = path.with_extension("json.noble-bak");
    if path.is_file() && !backup.exists() {
        std::fs::copy(path, &backup).map_err(|e| format!("backup failed: {e}"))?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    // Atomic, through a symlinked file, keeping its permissions; no temp file is left on failure.
    crate::store::write_text(path, &(text + "\n")).map_err(|e| e.to_string())
}

/// Are the hooks installed for every event?
pub fn is_installed(path: &Path) -> bool {
    let Ok(v) = read_settings(path) else { return false };
    EVENTS.iter().all(|(event, _)| {
        v.pointer(&format!("/hooks/{event}")).and_then(Value::as_array).is_some_and(|l| l.iter().any(is_noble_hook))
    })
}

/// Adds the missing hooks; touches neither other settings nor other hooks.
pub fn install(path: &Path, base: &str) -> Result<(), String> {
    let mut v = read_settings(path)?;
    let root = v.as_object_mut().ok_or("settings.json is not a JSON object")?;
    let hooks = root.entry("hooks").or_insert_with(|| json!({}));
    let hooks = hooks.as_object_mut().ok_or("\"hooks\" is not an object")?;
    for (event, arg) in EVENTS {
        let list = hooks.entry(event).or_insert_with(|| json!([]));
        let list = list.as_array_mut().ok_or(format!("hooks.{event} is not a list"))?;
        if !list.iter().any(is_noble_hook) {
            list.push(json!({ "hooks": [ { "type": "command", "command": format!("{base} hook {arg}") } ] }));
        }
    }
    write_settings(path, &v)
}

/// Startup: when the hooks of an older NOBLE are installed, adds the events added since
/// (with the same command) so existing users need not turn the setting off and on.
/// Does nothing when no NOBLE hook is installed or all of them already are.
pub fn upgrade(path: &Path) -> Result<(), String> {
    let v = read_settings(path)?;
    let base = EVENTS.iter().find_map(|(event, _)| {
        let list = v.pointer(&format!("/hooks/{event}"))?.as_array()?;
        let entry = list.iter().find(|e| is_noble_hook(e))?;
        entry.pointer("/hooks")?.as_array()?.iter().find_map(|h| {
            let command = h.get("command")?.as_str()?;
            // The last " hook " is ours: the executable's path may contain one too
            // (`C:\my hook tools\noble.exe`), and what follows it must be one of our events.
            let (base, arg) = command.rsplit_once(MARKER)?;
            EVENTS.iter().any(|(_, a)| *a == arg.trim()).then(|| base.to_string())
        })
    });
    match base {
        Some(base) if !is_installed(path) => install(path, &base),
        _ => Ok(()),
    }
}

/// Removes the hooks NOBLE added; also deletes lists left empty.
pub fn uninstall(path: &Path) -> Result<(), String> {
    let mut v = read_settings(path)?;
    let Some(hooks) = v.get_mut("hooks").and_then(Value::as_object_mut) else { return Ok(()) };
    for (event, _) in EVENTS {
        if let Some(list) = hooks.get_mut(event).and_then(Value::as_array_mut) {
            list.retain(|e| !is_noble_hook(e));
            if list.is_empty() {
                hooks.remove(event);
            }
        }
    }
    if hooks.is_empty()
        && let Some(root) = v.as_object_mut()
    {
        root.remove("hooks");
    }
    write_settings(path, &v)
}

pub fn agents_dir(data: &Path) -> PathBuf {
    data.join("agents")
}

/// `noble hook <event>`: takes the message from the JSON Claude passes on stdin
/// and writes the pane's state file. Does nothing when run outside NOBLE.
/// Errors never surface to Claude (always silent).
pub fn run_cli(event: &str, stdin: &str, data: &Path, instance: Option<&str>, pane: Option<&str>) {
    let (Some(instance), Some(pane)) = (instance, pane) else { return };
    if !instance.bytes().all(|b| b.is_ascii_digit()) || !pane.bytes().all(|b| b.is_ascii_digit()) {
        return;
    }
    let dir = agents_dir(data);
    let file = dir.join(format!("{instance}-{pane}.json"));
    // The session is over (Claude exited, or `/clear` right before a new `session-start`):
    // the pane no longer runs Claude, so its record and subagent markers go.
    if event == SESSION_END {
        remove_pane_files(&dir, instance, pane);
        return;
    }
    let input = serde_json::from_str::<Value>(stdin).unwrap_or(Value::Null);
    let field = |key: &str| input.get(key).and_then(Value::as_str).map(str::trim);
    if event == SUBAGENT_START || event == SUBAGENT_STOP {
        // Only file-name-safe characters of the id; without an id the subagent is not tracked.
        let id: String = field("agent_id")
            .unwrap_or("")
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
            .take(64)
            .collect();
        if id.is_empty() {
            return;
        }
        let marker = dir.join(format!("{instance}-{pane}.{id}.sub"));
        if event == SUBAGENT_START {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(marker, "");
        } else {
            let _ = std::fs::remove_file(marker);
        }
        return;
    }
    // Reminders that ask nothing of the user leave the state as it is.
    if event == "notification"
        && (field("notification_type").is_some_and(|t| QUIET_NOTIFICATIONS.contains(&t))
            || field("message") == Some(IDLE_MESSAGE))
    {
        return;
    }
    let message = field("message").map(|m| crate::util::truncate(m, 120));
    let rec = HookRecord { event: event.to_string(), message, ts: chrono::Utc::now().timestamp(), subagents: 0 };
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(text) = serde_json::to_string(&rec) {
        let tmp = dir.join(format!("{instance}-{pane}.tmp"));
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, file);
        }
    }
}

/// Deletes a pane's record and its subagent markers.
fn remove_pane_files(dir: &Path, instance: impl std::fmt::Display, pane: impl std::fmt::Display) {
    let _ = std::fs::remove_file(dir.join(format!("{instance}-{pane}.json")));
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let prefix = format!("{instance}-{pane}.");
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with(&prefix) && name.ends_with(".sub") {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Records belonging to this NOBLE instance (pane → last event and running subagents).
pub fn read_records(data: &Path, instance: u32) -> HashMap<PaneId, HookRecord> {
    let mut out = HashMap::new();
    let Ok(entries) = std::fs::read_dir(agents_dir(data)) else { return out };
    let prefix = format!("{instance}-");
    let mut subagents: HashMap<PaneId, usize> = HashMap::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(rest) = name.strip_prefix(&prefix) else { continue };
        if let Some(marker) = rest.strip_suffix(".sub") {
            let fresh = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_none_or(|age| age.as_secs() < SUBAGENT_MAX_AGE_SECS);
            if let Some(Ok(pane)) = marker.split_once('.').map(|(p, _)| p.parse::<PaneId>())
                && fresh
            {
                *subagents.entry(pane).or_default() += 1;
            }
            continue;
        }
        let Some(pane) = rest.strip_suffix(".json") else { continue };
        let Ok(pane) = pane.parse::<PaneId>() else { continue };
        if let Ok(text) = std::fs::read_to_string(e.path())
            && let Ok(rec) = serde_json::from_str::<HookRecord>(&text)
        {
            out.insert(pane, rec);
        }
    }
    for (pane, rec) in &mut out {
        rec.subagents = subagents.get(pane).copied().unwrap_or(0);
    }
    out
}

/// Startup cleanup of the agents folder: files older than a day (left behind by NOBLE
/// instances that crashed or were killed), and every file named after this `instance`.
/// No pane of this instance exists yet, so such a file can only come from an earlier
/// process that had the same id (process ids are reused) and would otherwise mark a
/// fresh pane as running Claude. Records of other running instances stay.
pub fn prune(data: &Path, instance: u32) {
    let Ok(entries) = std::fs::read_dir(agents_dir(data)) else { return };
    let prefix = format!("{instance}-");
    for e in entries.flatten() {
        let mine = e.file_name().to_string_lossy().starts_with(&prefix);
        let old = mine
            || e.metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age.as_secs() > 86_400);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Deletes a pane's record and its subagent markers.
pub fn remove_record(data: &Path, instance: u32, pane: PaneId) {
    remove_pane_files(&agents_dir(data), instance, pane);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("noble-hooks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A failed write leaves no temporary file next to the settings (a directory in the way
    /// makes the rename fail on every platform).
    /// Only NOBLE's own command shape counts: another tool that merely mentions "noble" and " hook " does not.
    #[test]
    fn noble_command_shape_is_strict() {
        for ok in [
            "noble hook stop",
            "noble-dev hook prompt",
            "\"C:/Users/me/.cargo/bin/noble.exe\" hook session-end",
            "\"/opt/my hook tools/noble\" hook notification",
        ] {
            assert!(is_noble_command(ok), "{ok}");
        }
        for bad in [
            "my-noble-notifier hook stop",
            "noblex hook stop",
            "noble hook unknown-event",
            "echo noble hook stop; rm -rf ~",
            "python noble.py hook stop",
            "noble stop",
        ] {
            assert!(!is_noble_command(bad), "{bad}");
        }
    }

    /// The first backup (the user's original file) survives later writes.
    #[test]
    fn backup_is_written_only_once() {
        let dir = std::env::temp_dir().join(format!("noble-hook-bak-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("settings.json");
        std::fs::write(&file, "{\"theme\": \"dark\"}").unwrap();
        install(&file, "noble").unwrap();
        uninstall(&file).unwrap();
        let bak = std::fs::read_to_string(dir.join("settings.json.noble-bak")).unwrap();
        assert_eq!(bak, "{\"theme\": \"dark\"}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_settings_cleans_up_its_temp_file() {
        let dir = temp("tmp");
        let blocked = dir.join("settings.json");
        std::fs::create_dir_all(blocked.join("inside")).unwrap();
        assert!(write_settings(&blocked, &serde_json::json!({})).is_err());
        assert!(blocked.is_dir(), "the directory in the way is untouched");
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from("settings.json")], "temp file left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_keeps_other_settings_and_is_idempotent() {
        let dir = temp("install");
        let file = dir.join("settings.json");
        std::fs::write(
            &file,
            r#"{ "model": "opus", "hooks": { "Stop": [ { "hooks": [ { "type": "command", "command": "say done" } ] } ] } }"#,
        )
        .unwrap();
        assert!(!is_installed(&file));
        install(&file, "noble").unwrap();
        install(&file, "noble").unwrap();
        assert!(is_installed(&file));
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(v["model"], "opus");
        // The user's key order is kept (`model` stays before `hooks`).
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.find("\"model\"").unwrap() < text.find("\"hooks\"").unwrap(), "{text}");
        let stop = v["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2, "user hook kept, ours added once");
        assert_eq!(stop[1]["hooks"][0]["command"], "noble hook stop");
        assert!(dir.join("settings.json.noble-bak").exists());
        uninstall(&file).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert!(v["hooks"].get("Notification").is_none());
        assert!(!is_installed(&file));
        // A UTF-8 BOM (PowerShell 5, Notepad) does not make the file unreadable.
        std::fs::write(&file, "\u{feff}{ \"model\": \"opus\" }").unwrap();
        install(&file, "noble").unwrap();
        assert!(is_installed(&file));
        // Never writes to a corrupt file.
        std::fs::write(&file, "{ not json").unwrap();
        assert!(install(&file, "noble").is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ not json");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cli_writes_records_only_inside_noble() {
        let data = temp("cli");
        run_cli("stop", "{}", &data, None, Some("3"));
        assert!(read_records(&data, 42).is_empty());
        run_cli(
            "notification",
            r#"{"message":"Claude needs your permission to use Bash"}"#,
            &data,
            Some("42"),
            Some("3"),
        );
        run_cli("prompt", "", &data, Some("7"), Some("1"));
        let recs = read_records(&data, 42);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[&3].event, "notification");
        assert_eq!(recs[&3].message.as_deref(), Some("Claude needs your permission to use Bash"));
        remove_record(&data, 42, 3);
        assert!(read_records(&data, 42).is_empty());
        let _ = std::fs::remove_dir_all(&data);
    }

    /// `session-end` clears the pane's record instead of leaving a state behind;
    /// `/clear` (session-end, then session-start) ends with a fresh record.
    #[test]
    fn session_end_clears_the_record() {
        let data = temp("end");
        run_cli("prompt", "{}", &data, Some("42"), Some("3"));
        run_cli("prompt", "{}", &data, Some("42"), Some("4"));
        run_cli(SESSION_END, "{}", &data, Some("42"), Some("3"));
        let recs = read_records(&data, 42);
        assert!(!recs.contains_key(&3), "{recs:?}");
        assert_eq!(recs[&4].event, "prompt", "other panes keep their record");
        // Ending a session that has no record is harmless.
        run_cli(SESSION_END, "{}", &data, Some("42"), Some("9"));
        run_cli("session-start", "{}", &data, Some("42"), Some("3"));
        assert_eq!(read_records(&data, 42)[&3].event, "session-start");
        let names: Vec<String> = std::fs::read_dir(agents_dir(&data))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|n| n.ends_with(".json")), "no temp files left: {names:?}");
        let _ = std::fs::remove_dir_all(&data);
    }

    /// Running subagents are counted from `subagent-start` until `subagent-stop`, per pane;
    /// markers of subagents that never reported their end stop counting after a while, and
    /// `session-end` removes them with the record.
    #[test]
    fn subagents_are_counted_until_they_stop() {
        let data = temp("subagents");
        let hook = |event: &str, stdin: &str| run_cli(event, stdin, &data, Some("42"), Some("3"));
        let count = || read_records(&data, 42).get(&3).map(|r| (r.event.clone(), r.subagents));
        hook("prompt", "{}");
        hook(SUBAGENT_START, r#"{"agent_id":"agent-001","agent_type":"Explore"}"#);
        hook(SUBAGENT_START, r#"{"agent_id":"agent-002","agent_type":"general-purpose"}"#);
        hook(SUBAGENT_START, r#"{"agent_id":"agent-001"}"#); // reported twice: still one
        hook(SUBAGENT_START, "{}"); // no id: not tracked
        hook(SUBAGENT_START, r#"{"agent_id":"../.."}"#); // nothing file-name-safe left
        run_cli(SUBAGENT_START, r#"{"agent_id":"agent-009"}"#, &data, Some("42"), Some("4")); // another pane
        hook("stop", "{}");
        assert_eq!(count(), Some(("stop".into(), 2)), "the main answer ended, two subagents run");
        hook(SUBAGENT_STOP, r#"{"agent_id":"agent-001","last_assistant_message":"done"}"#);
        assert_eq!(count(), Some(("stop".into(), 1)));
        // A marker left behind hours ago (Claude killed) no longer counts.
        let marker = agents_dir(&data).join("42-3.agent-002.sub");
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(SUBAGENT_MAX_AGE_SECS + 60);
        std::fs::File::options().write(true).open(&marker).unwrap().set_modified(old).unwrap();
        assert_eq!(count(), Some(("stop".into(), 0)));
        hook(SUBAGENT_START, r#"{"agent_id":"agent-003"}"#);
        hook(SESSION_END, "{}");
        let names: Vec<String> = std::fs::read_dir(agents_dir(&data))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["42-4.agent-009.sub".to_string()], "only the other pane's marker is left");
        remove_record(&data, 42, 4);
        assert!(std::fs::read_dir(agents_dir(&data)).unwrap().next().is_none());
        let _ = std::fs::remove_dir_all(&data);
    }

    /// The idle reminder and a background subagent finishing ask nothing of the user, so
    /// they keep the last state; a permission prompt (from Claude or a subagent) is recorded.
    #[test]
    fn only_real_notifications_are_recorded() {
        let data = temp("quiet");
        let hook = |event: &str, stdin: &str| run_cli(event, stdin, &data, Some("42"), Some("3"));
        let event = || read_records(&data, 42)[&3].event.clone();
        hook("stop", "{}");
        hook("notification", r#"{"notification_type":"idle_prompt","message":"Claude is waiting for your input"}"#);
        assert_eq!(event(), "stop");
        hook("notification", r#"{"message":"Claude is waiting for your input"}"#); // older Claude
        assert_eq!(event(), "stop");
        hook("notification", r#"{"notification_type":"agent_completed","message":"Agent finished"}"#);
        assert_eq!(event(), "stop");
        hook(
            "notification",
            r#"{"notification_type":"permission_prompt","message":"Claude needs your permission to use Bash","agent_id":"agent-001"}"#,
        );
        assert_eq!(event(), "notification");
        assert_eq!(read_records(&data, 42)[&3].message.as_deref(), Some("Claude needs your permission to use Bash"));
        let _ = std::fs::remove_dir_all(&data);
    }

    /// Hooks installed by an older NOBLE get the events added since, with the same command;
    /// settings without NOBLE hooks are left alone.
    #[test]
    fn upgrade_adds_new_events_to_old_installs() {
        let dir = temp("upgrade");
        let file = dir.join("settings.json");
        let base = r#""C:\Tools\noble.exe""#;
        let mut hooks = serde_json::Map::new();
        for (event, arg) in &EVENTS[..5] {
            hooks.insert(
                event.to_string(),
                json!([{ "hooks": [ { "type": "command", "command": format!("{base} hook {arg}") } ] }]),
            );
        }
        std::fs::write(&file, serde_json::to_string(&json!({ "hooks": hooks })).unwrap()).unwrap();
        assert!(!is_installed(&file));
        upgrade(&file).unwrap();
        assert!(is_installed(&file));
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(v["hooks"]["SubagentStop"][0]["hooks"][0]["command"], format!("{base} hook subagent-stop"));
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 1, "existing hooks are not duplicated");
        uninstall(&file).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert!(v.get("hooks").is_none(), "uninstall removes the new events too: {v}");
        // Without NOBLE hooks nothing is written.
        let other = dir.join("other.json");
        std::fs::write(&other, r#"{ "model": "opus" }"#).unwrap();
        upgrade(&other).unwrap();
        assert_eq!(std::fs::read_to_string(&other).unwrap(), r#"{ "model": "opus" }"#);
        assert!(!is_installed(&other));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The executable's path may itself contain " hook ": the upgrade keeps the whole path
    /// as the command base instead of cutting it at the first match.
    #[test]
    fn upgrade_keeps_a_base_path_that_contains_hook() {
        let dir = temp("upgrade-path");
        let file = dir.join("settings.json");
        let base = r#""/opt/my hook tools/noble""#;
        let mut hooks = serde_json::Map::new();
        for (event, arg) in &EVENTS[..5] {
            hooks.insert(
                event.to_string(),
                json!([{ "hooks": [ { "type": "command", "command": format!("{base} hook {arg}") } ] }]),
            );
        }
        std::fs::write(&file, serde_json::to_string(&json!({ "hooks": hooks })).unwrap()).unwrap();
        upgrade(&file).unwrap();
        assert!(is_installed(&file));
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(v["hooks"]["SubagentStart"][0]["hooks"][0]["command"], format!("{base} hook subagent-start"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Startup cleanup: records a day old (crashed instances) and leftovers of an earlier
    /// process with this instance's id go; another running instance's records stay.
    #[test]
    fn prune_removes_stale_and_reused_instance_records() {
        let data = temp("prune");
        run_cli("prompt", "{}", &data, Some("42"), Some("1")); // an earlier process with our id
        run_cli("stop", "{}", &data, Some("420"), Some("1")); // another instance (shares the prefix digits)
        run_cli("notification", "{}", &data, Some("7"), Some("2")); // crashed long ago
        let dir = agents_dir(&data);
        std::fs::write(dir.join("42-5.tmp"), "{").unwrap(); // write cut short by a crash
        let day_old = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 86_400);
        std::fs::File::options().write(true).open(dir.join("7-2.json")).unwrap().set_modified(day_old).unwrap();
        prune(&data, 42);
        assert!(read_records(&data, 42).is_empty());
        assert!(!dir.join("42-5.tmp").exists());
        assert!(read_records(&data, 7).is_empty(), "day-old record of a crashed instance");
        assert_eq!(read_records(&data, 420)[&1].event, "stop", "another live instance is untouched");
        // Without an agents folder there is nothing to do.
        prune(&data.join("missing"), 42);
        let _ = std::fs::remove_dir_all(&data);
    }
}
