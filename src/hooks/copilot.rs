//! GitHub Copilot CLI's hooks: NOBLE owns `noble.json` in the hooks folder (`$COPILOT_HOME/hooks`, else
//! `~/.copilot/hooks`) and that file is the only thing an install writes. Copilot loads every `*.json` file there,
//! and its events are camelCase lists of command entries (`bash` and `powershell` per entry, no matcher groups).
//! Uninstall removes NOBLE's entries; the file goes when nothing else is left in it.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{
    Agent, NOTIFICATION, PERMISSION_REQUEST, PROMPT, SESSION_END, SESSION_START, STOP, STOP_FAILURE, SUBAGENT_START,
    SUBAGENT_STOP, field, noble_base, plain_message, read_settings, write_settings,
};

/// Copilot event → `noble hook` argument (the camelCase names are Copilot's own).
pub const EVENTS: [(&str, &str); 9] = [
    ("sessionStart", SESSION_START),
    ("sessionEnd", SESSION_END),
    ("userPromptSubmitted", PROMPT),
    ("agentStop", STOP),
    ("subagentStart", SUBAGENT_START),
    ("subagentStop", SUBAGENT_STOP),
    ("notification", NOTIFICATION),
    ("permissionRequest", PERMISSION_REQUEST),
    ("errorOccurred", STOP_FAILURE),
];

/// Seconds a hook may run; NOBLE's hook only writes a small file.
const TIMEOUT_SECS: u64 = 10;

/// The notification kinds that ask nothing of the user: a background shell or subagent finished, or the agent is
/// idle. The subagent hooks and the state already show those.
const QUIET_KINDS: [&str; 4] = ["agent_idle", "agent_completed", "shell_completed", "shell_detached_completed"];

/// `COPILOT_HOME` when it is set, else `~/.copilot`; `None` without a home folder.
fn copilot_home(env: Option<&OsStr>, home: Option<PathBuf>) -> Option<PathBuf> {
    match env {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => home.map(|h| h.join(".copilot")),
    }
}

/// `noble.json` in Copilot's hooks folder; `None` when `ai::isolated` is set or there is no home folder.
pub fn settings_path() -> Option<PathBuf> {
    settings_file(crate::ai::isolated(), std::env::var_os("COPILOT_HOME").as_deref(), dirs::home_dir())
}

pub(super) fn settings_file(isolated: bool, env: Option<&OsStr>, home: Option<PathBuf>) -> Option<PathBuf> {
    if isolated {
        return None;
    }
    copilot_home(env, home).map(|dir| dir.join("hooks").join("noble.json"))
}

/// The base of a NOBLE command entry (`bash`, `powershell` or `command`), without PowerShell's call operator.
fn entry_base(entry: &Value) -> Option<String> {
    ["bash", "powershell", "command"].into_iter().find_map(|key| {
        let base = noble_base(entry.get(key)?.as_str()?)?;
        Some(base.strip_prefix("& ").unwrap_or(base.as_str()).to_string())
    })
}

/// Is this entry one of NOBLE's hooks?
fn is_noble_entry(entry: &Value) -> bool {
    entry_base(entry).is_some()
}

/// One command entry of an event. PowerShell runs a quoted path only after the call operator `&`; the bare
/// `noble` from PATH needs none.
fn entry(base: &str, arg: &str) -> Value {
    let command = format!("{base} hook {arg} --agent {}", Agent::Copilot.id());
    let powershell = if base.starts_with(['"', '\'']) { format!("& {command}") } else { command.clone() };
    json!({ "type": "command", "bash": command, "powershell": powershell, "timeoutSec": TIMEOUT_SECS })
}

/// Is every event installed with one of NOBLE's entries? An unreadable file is an error.
pub fn installed_state(path: &Path) -> Result<bool, String> {
    let v = read_settings(path)?;
    Ok(EVENTS.iter().all(|(event, _)| {
        v.pointer(&format!("/hooks/{event}")).and_then(Value::as_array).is_some_and(|l| l.iter().any(is_noble_entry))
    }))
}

/// Adds the missing entries and the `version` Copilot expects; other keys and entries are kept.
pub fn install(path: &Path, base: &str) -> Result<(), String> {
    let mut v = read_settings(path)?;
    let root = v.as_object_mut().ok_or("the hooks file is not a JSON object")?;
    root.entry("version").or_insert(json!(1));
    let hooks = root.entry("hooks").or_insert_with(|| json!({}));
    let hooks = hooks.as_object_mut().ok_or("\"hooks\" is not an object")?;
    for (event, arg) in EVENTS {
        let list = hooks.entry(event).or_insert_with(|| json!([]));
        let list = list.as_array_mut().ok_or(format!("hooks.{event} is not a list"))?;
        if !list.iter().any(is_noble_entry) {
            list.push(entry(base, arg));
        }
    }
    write_settings(path, &v)
}

/// Removes NOBLE's entries. The file is deleted when nothing else is in it; otherwise it is written back
/// without them (the user's own entries and keys stay). A file without NOBLE's entries is left as it is.
pub fn uninstall(path: &Path) -> Result<(), String> {
    let mut v = read_settings(path)?;
    let Some(root) = v.as_object_mut() else { return Ok(()) };
    let mut changed = false;
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        for (event, _) in EVENTS {
            if let Some(list) = hooks.get_mut(event).and_then(Value::as_array_mut) {
                let before = list.len();
                list.retain(|e| !is_noble_entry(e));
                changed |= list.len() != before;
                if list.is_empty() {
                    hooks.remove(event);
                }
            }
        }
    }
    if !changed {
        return Ok(());
    }
    if root.get("hooks").and_then(Value::as_object).is_some_and(|h| h.is_empty()) {
        root.remove("hooks");
    }
    if root.keys().all(|k| k.as_str() == "version") {
        return match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("cannot remove {}: {e}", path.display())),
        };
    }
    write_settings(path, &v)
}

/// Startup: a file with NOBLE's entries from an older build gets the events added since, with the same base.
pub fn upgrade(path: &Path) -> Result<(), String> {
    let v = read_settings(path)?;
    let base = EVENTS
        .iter()
        .find_map(|(event, _)| v.pointer(&format!("/hooks/{event}"))?.as_array()?.iter().find_map(entry_base));
    let Some(base) = base else { return Ok(()) };
    if installed_state(path)? {
        return Ok(());
    }
    install(path, &base)
}

/// The message a Copilot payload leaves on the pane's record (see `claude::record_message`). A notification that
/// asks nothing of the user is not recorded; a permission prompt keeps its text, a question says so; an error the
/// agent recovers from is not recorded, a failed turn names the error.
pub fn record_message(event: &str, input: &Value) -> Option<Option<String>> {
    let text = field(input, "message").filter(|m| !m.is_empty()).map(str::to_string);
    let message = match event {
        NOTIFICATION => match field(input, "notification_type") {
            Some(kind) if QUIET_KINDS.contains(&kind) => return None,
            Some("permission_prompt") => Some(text.unwrap_or_else(|| "Copilot asks for permission".to_string())),
            Some("elicitation_dialog") => Some("Copilot asks a question".to_string()),
            _ => text,
        },
        PERMISSION_REQUEST => Some(match field(input, "toolName").filter(|t| !t.is_empty()) {
            Some(tool) => format!("Copilot asks to run {tool}"),
            None => "Copilot asks for permission".to_string(),
        }),
        STOP_FAILURE => {
            if input.get("recoverable").and_then(Value::as_bool) == Some(true) {
                return None;
            }
            // The error is an object with a `message` (a plain string is taken as it is).
            let reason = match input.get("error") {
                Some(Value::String(s)) => Some(s.as_str()),
                Some(error) => error.get("message").and_then(Value::as_str),
                None => None,
            };
            reason.map(str::trim).filter(|r| !r.is_empty()).map(|r| format!("Copilot stopped with an error: {r}"))
        }
        _ => return plain_message(input),
    };
    Some(message.map(|m| crate::util::truncate(&m, 120)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::{HOOK_ARGS, quote_for_shell, read_records, run_cli};

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("noble-copilot-hooks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn read(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn events_map_to_known_hook_arguments() {
        for (native, arg) in EVENTS {
            assert!(HOOK_ARGS.contains(&arg), "{native}");
        }
        let mut names: Vec<&str> = EVENTS.iter().map(|(e, _)| *e).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), EVENTS.len(), "no native event twice");
    }

    #[test]
    fn copilot_home_is_taken_from_the_environment() {
        let home = Some(PathBuf::from("/home/me"));
        assert_eq!(copilot_home(None, home.clone()), Some(PathBuf::from("/home/me/.copilot")));
        assert_eq!(copilot_home(Some(OsStr::new("")), home.clone()), Some(PathBuf::from("/home/me/.copilot")));
        assert_eq!(copilot_home(Some(OsStr::new("/srv/copilot")), home), Some(PathBuf::from("/srv/copilot")));
        assert_eq!(copilot_home(None, None), None, "no home folder");
    }

    /// Under `ai::isolated` the settings path is off, so no test or isolated run touches `~/.copilot`.
    #[test]
    fn isolation_turns_the_settings_path_off() {
        let home = Some(PathBuf::from("/home/me"));
        assert_eq!(settings_file(true, None, home.clone()), None);
        assert_eq!(settings_file(true, Some(OsStr::new("/srv/copilot")), home.clone()), None);
        assert_eq!(settings_file(false, None, home), Some(PathBuf::from("/home/me/.copilot/hooks/noble.json")));
        assert_eq!(
            settings_file(false, Some(OsStr::new("/srv/copilot")), None),
            Some(PathBuf::from("/srv/copilot/hooks/noble.json")),
            "COPILOT_HOME moves the file even without a home folder"
        );
    }

    #[test]
    fn install_writes_one_file_with_every_event_and_is_idempotent() {
        let dir = temp("install");
        let file = dir.join("hooks").join("noble.json");
        assert!(!installed_state(&file).unwrap());
        install(&file, "noble").unwrap();
        install(&file, "noble").unwrap();
        assert!(installed_state(&file).unwrap());
        let v = read(&file);
        assert_eq!(v["version"], 1);
        let start = v["hooks"]["sessionStart"].as_array().unwrap();
        assert_eq!(start.len(), 1, "installed once");
        assert_eq!(start[0]["bash"], "noble hook session-start --agent copilot");
        assert_eq!(start[0]["powershell"], "noble hook session-start --agent copilot");
        assert_eq!(start[0]["timeoutSec"], 10);
        assert_eq!(v["hooks"]["errorOccurred"][0]["bash"], "noble hook stop-failure --agent copilot");
        let backup = dir.join("hooks").join("noble.json.noble-bak");
        assert_eq!(std::fs::read_to_string(backup).unwrap(), "", "an empty backup: the file did not exist");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// PowerShell needs the call operator before a quoted path; the bash form keeps the quotes as they are.
    #[test]
    fn a_quoted_base_gets_the_powershell_call_operator() {
        let dir = temp("powershell");
        let file = dir.join("noble.json");
        let base = quote_for_shell("C:/my tools/noble.exe");
        install(&file, &base).unwrap();
        let v = read(&file);
        let entry = &v["hooks"]["sessionEnd"][0];
        assert_eq!(entry["bash"], format!("{base} hook session-end --agent copilot"));
        assert_eq!(entry["powershell"], format!("& {base} hook session-end --agent copilot"));
        assert!(installed_state(&file).unwrap(), "the call operator still reads as NOBLE's");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Entries the user wrote in the same file stay, and uninstall keeps them and the file.
    #[test]
    fn install_keeps_the_users_entries_and_uninstall_leaves_them() {
        let dir = temp("user");
        let file = dir.join("noble.json");
        let original = r#"{"disableAllHooks":false,"hooks":{"agentStop":[{"type":"command","bash":"say done"}]}}"#;
        std::fs::write(&file, original).unwrap();
        install(&file, "noble").unwrap();
        let v = read(&file);
        assert_eq!(v["disableAllHooks"], false);
        assert_eq!(v["hooks"]["agentStop"].as_array().unwrap().len(), 2);
        assert_eq!(v["hooks"]["agentStop"][0]["bash"], "say done", "the user's entry stays first");
        assert_eq!(std::fs::read_to_string(dir.join("noble.json.noble-bak")).unwrap(), original);
        uninstall(&file).unwrap();
        let v = read(&file);
        assert_eq!(v["disableAllHooks"], false);
        assert_eq!(v["hooks"], json!({ "agentStop": [ { "type": "command", "bash": "say done" } ] }));
        assert!(!installed_state(&file).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Uninstall deletes the file when only NOBLE's hooks were in it.
    #[test]
    fn uninstall_deletes_the_file_when_it_held_only_nobles_hooks() {
        let dir = temp("delete");
        let file = dir.join("noble.json");
        install(&file, "noble").unwrap();
        uninstall(&file).unwrap();
        assert!(!file.exists());
        // Uninstalling again, or with no file, is harmless.
        uninstall(&file).unwrap();
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An install from an older build gets the events it lacks, with the base it was installed with.
    #[test]
    fn upgrade_adds_the_events_an_older_install_lacks() {
        let dir = temp("upgrade");
        let file = dir.join("noble.json");
        let base = quote_for_shell("/opt/my hook tools/noble");
        install(&file, &base).unwrap();
        let mut v = read(&file);
        for event in ["subagentStop", "errorOccurred"] {
            v["hooks"].as_object_mut().unwrap().remove(event);
        }
        std::fs::write(&file, v.to_string()).unwrap();
        assert!(!installed_state(&file).unwrap());
        upgrade(&file).unwrap();
        assert!(installed_state(&file).unwrap());
        let v = read(&file);
        assert_eq!(v["hooks"]["subagentStop"][0]["bash"], format!("{base} hook subagent-stop --agent copilot"));
        assert_eq!(v["hooks"]["errorOccurred"][0]["powershell"], format!("& {base} hook stop-failure --agent copilot"));
        assert_eq!(v["hooks"]["agentStop"].as_array().unwrap().len(), 1, "existing entries are not duplicated");
        // Without NOBLE's entries nothing is written.
        let other = dir.join("other.json");
        std::fs::write(&other, r#"{ "hooks": {} }"#).unwrap();
        upgrade(&other).unwrap();
        assert_eq!(std::fs::read_to_string(&other).unwrap(), r#"{ "hooks": {} }"#);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Each Copilot payload leaves the record its state needs, and the notifications that ask nothing leave the
    /// state as it was.
    #[test]
    fn each_event_records_the_state_it_means() {
        let data = temp("cli");
        let hook = |arg: &str, stdin: &str| run_cli(Agent::Copilot, arg, stdin, &data, Some("42"), Some("3"));
        let rec = || read_records(&data, 42).remove(&3);
        hook(SESSION_START, r#"{"source":"new"}"#);
        assert_eq!(rec().unwrap().agent(), Agent::Copilot);
        hook(PROMPT, r#"{"prompt":"list the files"}"#);
        assert_eq!(rec().unwrap().event, "prompt");
        hook(NOTIFICATION, r#"{"notification_type":"permission_prompt","message":"Allow reading the repo?"}"#);
        let r = rec().unwrap();
        assert_eq!((r.event.as_str(), r.message.as_deref()), ("notification", Some("Allow reading the repo?")));
        hook(NOTIFICATION, r#"{"notification_type":"elicitation_dialog","message":"Which branch?"}"#);
        assert_eq!(rec().unwrap().message.as_deref(), Some("Copilot asks a question"));
        hook(NOTIFICATION, r#"{"notification_type":"agent_idle","message":"Waiting"}"#);
        assert_eq!(rec().unwrap().message.as_deref(), Some("Copilot asks a question"), "a reminder keeps the state");
        hook(PERMISSION_REQUEST, r#"{"toolName":"bash"}"#);
        assert_eq!(rec().unwrap().message.as_deref(), Some("Copilot asks to run bash"));
        hook(SUBAGENT_START, r#"{"agentId":"sub-7","agentName":"explore"}"#);
        hook(STOP, r#"{"stopReason":"end_turn"}"#);
        let r = rec().unwrap();
        assert_eq!((r.event.as_str(), r.subagents), ("stop", 1), "the subagent still runs");
        hook(SUBAGENT_STOP, r#"{"agentId":"sub-7","agentType":"explore"}"#);
        assert_eq!(rec().unwrap().subagents, 0);
        hook(STOP_FAILURE, r#"{"recoverable":true,"error":{"message":"retrying"}}"#);
        assert_eq!(rec().unwrap().event, "stop", "a recovered error changes nothing");
        hook(STOP_FAILURE, r#"{"recoverable":false,"error":{"message":"model unavailable"}}"#);
        let r = rec().unwrap();
        assert_eq!(r.event, "stop-failure");
        assert_eq!(r.message.as_deref(), Some("Copilot stopped with an error: model unavailable"));
        hook(SESSION_END, r#"{"reason":"complete"}"#);
        assert!(rec().is_none(), "the session ended: no record");
        let _ = std::fs::remove_dir_all(&data);
    }
}
