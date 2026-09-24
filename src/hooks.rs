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
pub const EVENTS: [(&str, &str); 5] = [
    ("UserPromptSubmit", "prompt"),
    ("Stop", "stop"),
    ("Notification", "notification"),
    ("SessionStart", "session-start"),
    ("SessionEnd", "session-end"),
];

/// The shared marker used to recognize our hook commands.
const MARKER: &str = " hook ";

/// The last event for a pane.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HookRecord {
    pub event: String,
    #[serde(default)]
    pub message: Option<String>,
    pub ts: i64,
}

pub fn settings_path() -> Option<PathBuf> {
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

fn is_noble_hook(entry: &Value) -> bool {
    entry.get("hooks").and_then(Value::as_array).is_some_and(|hooks| {
        hooks.iter().any(|h| {
            h.get("command")
                .and_then(Value::as_str)
                .is_some_and(|c| c.contains(MARKER) && c.to_lowercase().contains("noble"))
        })
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
    if path.exists() {
        let backup = path.with_extension("json.noble-bak");
        std::fs::copy(path, &backup).map_err(|e| format!("backup failed: {e}"))?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.noble-tmp");
    std::fs::write(&tmp, text + "\n").map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
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
    let message = serde_json::from_str::<Value>(stdin)
        .ok()
        .and_then(|v| v.get("message").and_then(Value::as_str).map(|m| crate::util::truncate(m.trim(), 120)));
    let rec = HookRecord { event: event.to_string(), message, ts: chrono::Utc::now().timestamp() };
    let dir = agents_dir(data);
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(text) = serde_json::to_string(&rec) {
        let file = dir.join(format!("{instance}-{pane}.json"));
        let tmp = dir.join(format!("{instance}-{pane}.tmp"));
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, file);
        }
    }
}

/// Records belonging to this NOBLE instance (pane → last event).
pub fn read_records(data: &Path, instance: u32) -> HashMap<PaneId, HookRecord> {
    let mut out = HashMap::new();
    let Ok(entries) = std::fs::read_dir(agents_dir(data)) else { return out };
    let prefix = format!("{instance}-");
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(pane) = name.strip_prefix(&prefix).and_then(|r| r.strip_suffix(".json")) else { continue };
        let Ok(pane) = pane.parse::<PaneId>() else { continue };
        if let Ok(text) = std::fs::read_to_string(e.path())
            && let Ok(rec) = serde_json::from_str::<HookRecord>(&text)
        {
            out.insert(pane, rec);
        }
    }
    out
}

/// Deletes records older than a day (left behind by closed NOBLE instances).
pub fn prune(data: &Path) {
    let Ok(entries) = std::fs::read_dir(agents_dir(data)) else { return };
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age.as_secs() > 86_400);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

pub fn remove_record(data: &Path, instance: u32, pane: PaneId) {
    let _ = std::fs::remove_file(agents_dir(data).join(format!("{instance}-{pane}.json")));
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
}
