//! Antigravity CLI's (`agy`) hooks. agy reads `~/.gemini/config/hooks.json`, a file of hook names: each top-level key
//! is one hook and maps its events to handler lists. NOBLE owns the `noble` key. Under each event it uses, the key
//! holds a flat list of handlers (agy ignores matchers for these events), and `enabled` switches the whole entry.
//! NOBLE registers three events: `SessionStart`, `PreInvocation` (before each model call, so the agent is working) and
//! `Stop` (the loop ended; a Stop that reports an error is a failed turn). `PreToolUse` is left out on purpose: its
//! reply decides whether the tool may run, and an `allow` skips agy's own permission prompt. agy has no event for a
//! permission prompt, a notification, the end of a session or a subagent, so NOBLE shows none of those for it.
//! `enabled: false` (set in agy's `/hooks` view or by hand) counts as not installed, and startup leaves it off; an
//! install turns it on again.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{Agent, PROMPT, SESSION_START, STOP, STOP_FAILURE, field, is_noble_command, noble_base, plain_message};
use super::{read_settings, write_settings};

/// agy event → `noble hook` argument. Only these three are registered (see the module comment).
pub const EVENTS: [(&str, &str); 3] = [("SessionStart", SESSION_START), ("PreInvocation", PROMPT), ("Stop", STOP)];

/// The top-level key NOBLE's hooks live under. The user's own hooks use their own keys.
const HOOK_NAME: &str = "noble";

/// Seconds a handler may run; NOBLE's hook only writes a small file.
const TIMEOUT_SECS: u64 = 5;

/// `hooks.json` in agy's global config folder; `None` when `ai::isolated` is set or there is no home folder.
pub fn settings_path() -> Option<PathBuf> {
    settings_file(crate::ai::isolated(), dirs::home_dir())
}

/// agy has no environment variable that moves this folder, so only the home folder decides it.
pub(super) fn settings_file(isolated: bool, home: Option<PathBuf>) -> Option<PathBuf> {
    if isolated {
        return None;
    }
    home.map(|h| h.join(".gemini").join("config").join("hooks.json"))
}

/// One handler of an event: agy's handler is `type`, `command` and `timeout` (seconds). `base` comes from
/// `command_base`: `noble` when it is on PATH (every shell runs that), else a path quoted by `quote_for_shell`, which
/// is double quotes on Windows. Which shell agy runs a handler's command with on Windows is not verified. If it is
/// PowerShell, the quoted form does not run there (PowerShell needs the `&` call operator before a quoted path), and
/// one `command` cannot carry both forms (Copilot writes a separate `powershell` key for that). The usual install,
/// with `noble` on PATH, does not depend on it. Unix shells read the single quotes as they are.
fn handler(base: &str, arg: &str) -> Value {
    let command = format!("{base} hook {arg} --agent {}", Agent::Antigravity.id());
    json!({ "type": "command", "command": command, "timeout": TIMEOUT_SECS })
}

/// Is this handler one of NOBLE's?
fn is_noble_handler(handler: &Value) -> bool {
    handler.get("command").and_then(Value::as_str).is_some_and(is_noble_command)
}

/// The base of the first NOBLE handler in the file (`None` when there is none): an upgrade reuses it.
fn base_of(v: &Value) -> Option<String> {
    EVENTS.iter().find_map(|(event, _)| {
        let list = v.pointer(&format!("/{HOOK_NAME}/{event}"))?.as_array()?;
        list.iter().find_map(|h| noble_base(h.get("command")?.as_str()?))
    })
}

/// Did the user turn NOBLE's entry off (`enabled: false`)?
fn disabled(v: &Value) -> bool {
    v.pointer(&format!("/{HOOK_NAME}/enabled")).and_then(Value::as_bool) == Some(false)
}

/// Is NOBLE's entry on, with one of NOBLE's handlers under every event?
fn installed(v: &Value) -> bool {
    !disabled(v)
        && EVENTS.iter().all(|(event, _)| {
            v.pointer(&format!("/{HOOK_NAME}/{event}"))
                .and_then(Value::as_array)
                .is_some_and(|list| list.iter().any(is_noble_handler))
        })
}

/// Is every event installed with one of NOBLE's handlers, and the entry on? An unreadable file is an error.
pub fn installed_state(path: &Path) -> Result<bool, String> {
    Ok(installed(&read_settings(path)?))
}

/// Adds the missing handlers to the `noble` entry and turns the entry on. The user's other keys, handlers and
/// entry settings are kept.
pub fn install(path: &Path, base: &str) -> Result<(), String> {
    let mut v = read_settings(path)?;
    let root = v.as_object_mut().ok_or("the hooks file is not a JSON object")?;
    let entry = root.entry(HOOK_NAME).or_insert_with(|| json!({}));
    let entry = entry.as_object_mut().ok_or(format!("\"{HOOK_NAME}\" is not an object"))?;
    // An install is the user asking for the hooks, so a disabled entry is enabled again.
    entry.insert("enabled".into(), json!(true));
    for (event, arg) in EVENTS {
        let list = entry.entry(event).or_insert_with(|| json!([]));
        let list = list.as_array_mut().ok_or(format!("{HOOK_NAME}.{event} is not a list"))?;
        if !list.iter().any(is_noble_handler) {
            list.push(handler(base, arg));
        }
    }
    write_settings(path, &v)
}

/// Removes NOBLE's handlers from the `noble` entry. The entry goes when nothing but its `enabled` flag is left; the
/// user's handlers in it and every other key stay.
pub fn uninstall(path: &Path) -> Result<(), String> {
    let mut v = read_settings(path)?;
    let Some(root) = v.as_object_mut() else { return Ok(()) };
    let Some(entry) = root.get_mut(HOOK_NAME).and_then(Value::as_object_mut) else { return Ok(()) };
    let mut changed = false;
    for (event, _) in EVENTS {
        if let Some(list) = entry.get_mut(event).and_then(Value::as_array_mut) {
            let before = list.len();
            list.retain(|h| !is_noble_handler(h));
            let removed = list.len() != before;
            changed |= removed;
            if removed && list.is_empty() {
                entry.remove(event);
            }
        }
    }
    if !changed {
        return Ok(());
    }
    if entry.keys().all(|k| k == "enabled") {
        root.remove(HOOK_NAME);
    }
    write_settings(path, &v)
}

/// Startup: an entry from an older build gets the events added since, with the same command. An entry the user
/// turned off stays off, and a file without NOBLE's handlers is left as it is.
pub fn upgrade(path: &Path) -> Result<(), String> {
    let v = read_settings(path)?;
    let Some(base) = base_of(&v) else { return Ok(()) };
    if disabled(&v) || installed(&v) {
        return Ok(());
    }
    install(path, &base)
}

/// The hook argument for an agy event. A Stop whose payload reports an error (`terminationReason` `error`, or an
/// `error` text) is a failed turn, the state Claude's `StopFailure` gives; any other Stop is `stop`.
pub fn hook_arg<'a>(arg: &'a str, input: &Value) -> &'a str {
    let errored = input.get("terminationReason").and_then(Value::as_str) == Some("error")
        || field(input, "error").is_some_and(|e| !e.is_empty());
    if arg == STOP && errored { STOP_FAILURE } else { arg }
}

/// The text `noble hook` prints on stdout for an event. agy reads a JSON object back from each hook. For a Stop,
/// `continue` re-enters the loop and any other `decision` lets the agent stop, so `allow` is said. The other events'
/// output fields are all optional, so an empty object is enough.
pub fn reply(arg: &str) -> &'static str {
    if arg == STOP { r#"{"decision":"allow"}"# } else { "{}" }
}

/// The message an agy payload leaves on the pane's record (see `claude::record_message`): a failed turn names the
/// error when it has one; the other events keep their `message` if they have one.
pub fn record_message(event: &str, input: &Value) -> Option<Option<String>> {
    if event != STOP_FAILURE {
        return plain_message(input);
    }
    let text = match field(input, "error").filter(|e| !e.is_empty()) {
        Some(error) => format!("Antigravity stopped with an error: {error}"),
        None => "Antigravity stopped with an error".to_string(),
    };
    Some(Some(crate::util::truncate(&text, 120)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::{HOOK_ARGS, PERMISSION_REQUEST, quote_for_shell, read_records, run_cli};

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("noble-antigravity-hooks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn read(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    const CONVERSATION: &str = "ec33ebf9-0cba-4100-8142-c61503f6c587";

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

    /// Under `ai::isolated` the settings path is off, so no test or isolated run touches `~/.gemini`.
    #[test]
    fn isolation_turns_the_settings_path_off() {
        let home = Some(PathBuf::from("/home/me"));
        assert_eq!(settings_file(true, home.clone()), None);
        assert_eq!(settings_file(false, home), Some(PathBuf::from("/home/me/.gemini/config/hooks.json")));
        assert_eq!(settings_file(false, None), None, "no home folder");
    }

    /// The `noble` entry holds flat handlers (no matcher groups), turned on, and only NOBLE's events; the first
    /// backup of a missing file is empty.
    #[test]
    fn install_adds_flat_handlers_under_one_key_and_is_idempotent() {
        let dir = temp("install");
        let file = dir.join("config").join("hooks.json");
        assert!(!installed_state(&file).unwrap(), "no file, nothing installed");
        install(&file, "noble").unwrap();
        install(&file, "noble").unwrap();
        assert!(installed_state(&file).unwrap());
        let v = read(&file);
        assert_eq!(v.as_object().unwrap().keys().map(String::as_str).collect::<Vec<_>>(), ["noble"], "{v}");
        let entry = &v["noble"];
        assert_eq!(entry["enabled"], true);
        let stop = entry["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1, "installed once");
        assert_eq!(
            stop[0],
            json!({ "type": "command", "command": "noble hook stop --agent antigravity", "timeout": 5 }),
            "a flat handler, no matcher group"
        );
        assert_eq!(entry["PreInvocation"][0]["command"], "noble hook prompt --agent antigravity");
        assert_eq!(entry["SessionStart"][0]["command"], "noble hook session-start --agent antigravity");
        assert!(entry.get("PreToolUse").is_none() && entry.get("PostToolUse").is_none(), "{v}");
        assert_eq!(std::fs::read_to_string(file.with_extension("json.noble-bak")).unwrap(), "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The user's own hook names stay, the first backup keeps the file as it was, and uninstall removes the
    /// `noble` key and nothing else.
    #[test]
    fn install_keeps_the_users_hooks_and_uninstall_leaves_them() {
        let dir = temp("user");
        let file = dir.join("hooks.json");
        let original = r#"{"my-linter-hook":{"PostToolUse":[{"matcher":"run_command","hooks":[{"type":"command","command":"./lint.sh"}]}]}}"#;
        std::fs::write(&file, original).unwrap();
        install(&file, "noble").unwrap();
        let v = read(&file);
        assert_eq!(v["my-linter-hook"], read_text(original)["my-linter-hook"], "the user's hook name stays");
        assert_eq!(std::fs::read_to_string(file.with_extension("json.noble-bak")).unwrap(), original);
        uninstall(&file).unwrap();
        assert_eq!(read(&file), read_text(original), "only the noble key was removed");
        assert!(!installed_state(&file).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn read_text(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    /// Uninstall removes NOBLE's handlers from the `noble` entry and keeps the user's handler in it.
    #[test]
    fn uninstall_removes_only_nobles_handlers() {
        let dir = temp("uninstall");
        let file = dir.join("hooks.json");
        let user = json!({ "type": "command", "command": "say done" });
        let mine = json!({ "type": "command", "command": "noble hook stop --agent antigravity", "timeout": 5 });
        let settings = json!({
            "description": "mine",
            "noble": { "enabled": true, "Stop": [user.clone(), mine.clone()], "SessionStart": [mine] },
        });
        std::fs::write(&file, settings.to_string()).unwrap();
        uninstall(&file).unwrap();
        assert_eq!(
            read(&file),
            json!({ "description": "mine", "noble": { "enabled": true, "Stop": [user] } }),
            "other keys and the user's handler are kept"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An entry turned off in agy is not installed, and an install turns it on again.
    #[test]
    fn a_disabled_entry_is_not_installed_until_an_install_turns_it_on() {
        let dir = temp("disabled");
        let file = dir.join("hooks.json");
        install(&file, "noble").unwrap();
        let mut v = read(&file);
        v["noble"]["enabled"] = json!(false);
        std::fs::write(&file, v.to_string()).unwrap();
        assert!(!installed_state(&file).unwrap(), "a disabled entry is not installed");
        install(&file, "noble").unwrap();
        assert!(installed_state(&file).unwrap());
        assert_eq!(read(&file)["noble"]["enabled"], true);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An install from an older build gets the events it lacks, with the command it was installed with. An entry the
    /// user turned off stays off, and a file without NOBLE's handlers is not touched.
    #[test]
    fn upgrade_adds_the_events_an_older_install_lacks() {
        let dir = temp("upgrade");
        let file = dir.join("hooks.json");
        let base = quote_for_shell("/opt/my hook tools/noble");
        install(&file, &base).unwrap();
        let mut v = read(&file);
        v["noble"].as_object_mut().unwrap().remove("PreInvocation");
        std::fs::write(&file, v.to_string()).unwrap();
        assert!(!installed_state(&file).unwrap());
        upgrade(&file).unwrap();
        assert!(installed_state(&file).unwrap());
        let v = read(&file);
        assert_eq!(v["noble"]["PreInvocation"][0]["command"], format!("{base} hook prompt --agent antigravity"));
        assert_eq!(v["noble"]["Stop"].as_array().unwrap().len(), 1, "existing handlers are not duplicated");

        // A disabled entry is left disabled.
        let mut v = read(&file);
        v["noble"].as_object_mut().unwrap().remove("SessionStart");
        v["noble"]["enabled"] = json!(false);
        std::fs::write(&file, v.to_string()).unwrap();
        upgrade(&file).unwrap();
        let v = read(&file);
        assert_eq!(v["noble"]["enabled"], false, "startup does not turn the hooks on");
        assert!(v["noble"].get("SessionStart").is_none());

        let other = dir.join("other.json");
        let text = r#"{ "my-hook": { "Stop": [] } }"#;
        std::fs::write(&other, text).unwrap();
        upgrade(&other).unwrap();
        assert_eq!(std::fs::read_to_string(&other).unwrap(), text);
        upgrade(&dir.join("none.json")).unwrap();
        assert!(!dir.join("none.json").exists(), "a missing file is not created");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file that cannot be parsed is an error for the state, not "not installed".
    #[test]
    fn an_unreadable_file_is_an_error() {
        let dir = temp("broken");
        let file = dir.join("hooks.json");
        std::fs::write(&file, "{ not json").unwrap();
        assert!(installed_state(&file).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The reply on stdout: Stop says `allow` (any decision other than `continue` lets it stop), the other events an
    /// empty object.
    #[test]
    fn hooks_answer_with_a_json_object() {
        assert_eq!(reply(STOP), r#"{"decision":"allow"}"#);
        for arg in [SESSION_START, PROMPT, PERMISSION_REQUEST] {
            assert_eq!(reply(arg), "{}", "{arg}");
        }
        for arg in [STOP, SESSION_START, PROMPT] {
            let _: Value = serde_json::from_str(reply(arg)).unwrap();
        }
    }

    /// Each agy event leaves the record its state needs: working before a model call, idle after the loop, failed
    /// when the Stop reports an error, and the conversation id to resume.
    #[test]
    fn each_event_records_the_state_it_means() {
        let data = temp("cli");
        let hook = |arg: &str, stdin: &str| run_cli(Agent::Antigravity, arg, stdin, &data, Some("42"), Some("3"));
        let rec = || read_records(&data, 42).remove(&3);
        hook(SESSION_START, &format!(r#"{{"conversationId":"{CONVERSATION}","modelName":"gemini"}}"#));
        let r = rec().unwrap();
        assert_eq!((r.event.as_str(), r.agent(), r.message.as_deref()), ("session-start", Agent::Antigravity, None));
        assert_eq!(r.session.as_deref(), Some(CONVERSATION), "the conversation to resume");
        hook(PROMPT, r#"{"invocationNum":0,"initialNumSteps":0}"#);
        assert_eq!(rec().unwrap().event, "prompt");
        hook(STOP, r#"{"executionNum":1,"terminationReason":"model_stop","error":"","fullyIdle":true}"#);
        let r = rec().unwrap();
        assert_eq!((r.event.as_str(), r.message), ("stop", None));
        hook(STOP, r#"{"executionNum":2,"terminationReason":"error","error":"model unavailable","fullyIdle":true}"#);
        let r = rec().unwrap();
        assert_eq!(r.event, "stop-failure");
        assert_eq!(r.message.as_deref(), Some("Antigravity stopped with an error: model unavailable"));
        hook(STOP, r#"{"terminationReason":"error","error":""}"#);
        assert_eq!(rec().unwrap().message.as_deref(), Some("Antigravity stopped with an error"));
        hook(PROMPT, "{}");
        assert_eq!(rec().unwrap().event, "prompt", "a new turn is working again");
        let _ = std::fs::remove_dir_all(&data);
    }
}
