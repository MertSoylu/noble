//! Cursor CLI's (`cursor-agent`) hooks. The docs name `~/.cursor/hooks.json` for every platform; cursor-agent reads it
//! on Windows too (confirmed in a live run). The file holds `"version": 1` and, under `hooks`, a flat list of command
//! entries per event: Cursor has no matcher groups. NOBLE registers six events: `sessionStart`; `beforeSubmitPrompt`
//! (a prompt starts a turn: working; the CLI's support for this event is not verified); the tool events `postToolUse`,
//! `afterShellExecution` and `afterFileEdit` (the agent is working); and `stop` (the turn ended: idle, or a failed
//! turn when its `status` is `error`). Cursor's permission hooks (`beforeShellExecution`, `beforeMCPExecution`,
//! `beforeReadFile`, `preToolUse` and the rest) are left out on purpose: their reply decides whether the action may
//! run, and NOBLE never answers for it. The state is partial: not every hook event fires in the CLI, and no
//! registered event reports a question, so NOBLE shows no "needs you" for this agent.
//! Every event answers on stdout with a JSON object (see [`reply`]).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{
    Agent, PROMPT, SESSION_START, STOP, STOP_FAILURE, TOOL_USE, field, is_noble_command, noble_base, plain_message,
    read_settings, write_settings,
};

/// Cursor event → `noble hook` argument. Only these six are registered (see the module comment).
pub const EVENTS: [(&str, &str); 6] = [
    ("sessionStart", SESSION_START),
    ("beforeSubmitPrompt", PROMPT),
    ("postToolUse", TOOL_USE),
    ("afterShellExecution", TOOL_USE),
    ("afterFileEdit", TOOL_USE),
    ("stop", STOP),
];

/// Seconds a hook may run; NOBLE's hook only writes a small file.
const TIMEOUT_SECS: u64 = 5;

/// The `cursor-agent` program: on PATH, else the copy the Windows installer leaves in `%LOCALAPPDATA%\cursor-agent`
/// (that folder is not always on PATH). Used by the agent's `on_path`, which decides whether it counts as installed.
pub fn program() -> Option<PathBuf> {
    crate::util::which("cursor-agent").or_else(|| installed_copy(std::env::var_os("LOCALAPPDATA").as_deref()))
}

/// The installer's `cursor-agent.cmd` under `local` (the LOCALAPPDATA folder). The `.cmd` shim is the file that
/// starts from a plain command line (the `.ps1` next to it does not). LOCALAPPDATA is not set on Linux and macOS, so
/// PATH is all that counts there.
fn installed_copy(local: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let local = local.filter(|v| !v.is_empty())?;
    let p = PathBuf::from(local).join("cursor-agent").join("cursor-agent.cmd");
    p.is_file().then_some(p)
}

/// `hooks.json` in Cursor's home folder (`~/.cursor`); `None` when `ai::isolated` is set or there is no home folder.
pub fn settings_path() -> Option<PathBuf> {
    settings_file(crate::ai::isolated(), dirs::home_dir())
}

/// No environment variable is known to move this folder, so only the home folder decides it.
pub(super) fn settings_file(isolated: bool, home: Option<PathBuf>) -> Option<PathBuf> {
    if isolated {
        return None;
    }
    home.map(|h| h.join(".cursor").join("hooks.json"))
}

/// One hook of an event. `base` comes from `command_base`: `noble` when it is on PATH, else a quoted path. Which shell
/// cursor-agent runs a hook command with on Windows is not verified. If it is PowerShell, a quoted absolute path does
/// not run there (PowerShell needs the `&` call operator), the same open question as Antigravity's handlers (see
/// `antigravity::handler`). The usual install with `noble` on PATH does not depend on it.
fn handler(base: &str, arg: &str) -> Value {
    let command = format!("{base} hook {arg} --agent {}", Agent::Cursor.id());
    json!({ "command": command, "timeout": TIMEOUT_SECS })
}

/// Is this entry one of NOBLE's?
fn is_noble_handler(entry: &Value) -> bool {
    entry.get("command").and_then(Value::as_str).is_some_and(is_noble_command)
}

/// The base of the first NOBLE entry in the file (`None` when there is none): an upgrade reuses it.
fn base_of(v: &Value) -> Option<String> {
    EVENTS.iter().find_map(|(event, _)| {
        let list = v.pointer(&format!("/hooks/{event}"))?.as_array()?;
        list.iter().find_map(|h| noble_base(h.get("command")?.as_str()?))
    })
}

/// Is every event installed with one of NOBLE's entries?
fn installed(v: &Value) -> bool {
    EVENTS.iter().all(|(event, _)| {
        v.pointer(&format!("/hooks/{event}"))
            .and_then(Value::as_array)
            .is_some_and(|list| list.iter().any(is_noble_handler))
    })
}

/// Is every event installed with one of NOBLE's hooks? An unreadable file is an error.
pub fn installed_state(path: &Path) -> Result<bool, String> {
    Ok(installed(&read_settings(path)?))
}

/// Adds the missing entries and the file's `"version": 1` when it has none. The user's own entries, keys and any
/// other version are kept.
pub fn install(path: &Path, base: &str) -> Result<(), String> {
    let mut v = read_settings(path)?;
    let root = v.as_object_mut().ok_or("the hooks file is not a JSON object")?;
    root.entry("version").or_insert_with(|| json!(1));
    let hooks = root.entry("hooks").or_insert_with(|| json!({}));
    let hooks = hooks.as_object_mut().ok_or("\"hooks\" is not an object")?;
    for (event, arg) in EVENTS {
        let list = hooks.entry(event).or_insert_with(|| json!([]));
        let list = list.as_array_mut().ok_or(format!("hooks.{event} is not a list"))?;
        if !list.iter().any(is_noble_handler) {
            list.push(handler(base, arg));
        }
    }
    write_settings(path, &v)
}

/// Removes NOBLE's entries and nothing else. An event list emptied this way goes, and so does `hooks` when nothing
/// is left in it; `version` and the user's entries stay.
pub fn uninstall(path: &Path) -> Result<(), String> {
    let mut v = read_settings(path)?;
    let Some(root) = v.as_object_mut() else { return Ok(()) };
    let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) else { return Ok(()) };
    let mut changed = false;
    for (event, _) in EVENTS {
        if let Some(list) = hooks.get_mut(event).and_then(Value::as_array_mut) {
            let before = list.len();
            list.retain(|h| !is_noble_handler(h));
            if list.len() != before {
                changed = true;
                if list.is_empty() {
                    hooks.remove(event);
                }
            }
        }
    }
    if !changed {
        return Ok(());
    }
    if hooks.is_empty() {
        root.remove("hooks");
    }
    write_settings(path, &v)
}

/// Startup: a file with NOBLE's entries from an older build gets the events added since, with the same command.
pub fn upgrade(path: &Path) -> Result<(), String> {
    let v = read_settings(path)?;
    let Some(base) = base_of(&v) else { return Ok(()) };
    if installed(&v) {
        return Ok(());
    }
    install(path, &base)
}

/// The hook argument for a Cursor event. A Stop whose `status` is `error` is a failed turn (the state Claude's
/// `StopFailure` gives); any other Stop is `stop`.
pub fn hook_arg<'a>(arg: &'a str, input: &Value) -> &'a str {
    if arg == STOP && field(input, "status") == Some("error") { STOP_FAILURE } else { arg }
}

/// The text `noble hook` prints on stdout for an event. Every event answers with a JSON object, so each hook's output
/// is valid JSON; the docs do not say what an empty reply means for each event. `beforeSubmitPrompt` says
/// `continue: true`, which lets the prompt through; the others answer `{}`.
pub fn reply(arg: &str) -> &'static str {
    if arg == PROMPT { r#"{"continue":true}"# } else { "{}" }
}

/// The message a Cursor payload leaves on the pane's record (see `claude::record_message`): a failed turn says so;
/// the other events keep their `message` if they have one.
pub fn record_message(event: &str, input: &Value) -> Option<Option<String>> {
    if event != STOP_FAILURE {
        return plain_message(input);
    }
    Some(Some("Cursor stopped with an error".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::{HOOK_ARGS, quote_for_shell, read_records, run_cli};

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("noble-cursor-hooks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn read(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    const CONVERSATION: &str = "5f0c1a2b-9d3e-4c7a-8b1f-0e2d3c4b5a69";

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

    /// Only the events of the module comment are registered: no permission hook, which would decide an action.
    #[test]
    fn permission_hooks_are_not_registered() {
        for native in ["beforeShellExecution", "beforeMCPExecution", "beforeReadFile", "preToolUse", "subagentStart"] {
            assert!(EVENTS.iter().all(|(e, _)| *e != native), "{native}");
        }
    }

    /// Under `ai::isolated` the settings path is off, so no test or isolated run touches `~/.cursor`.
    #[test]
    fn isolation_turns_the_settings_path_off() {
        let home = Some(PathBuf::from("/home/me"));
        assert_eq!(settings_file(true, home.clone()), None);
        assert_eq!(settings_file(false, home), Some(PathBuf::from("/home/me/.cursor/hooks.json")));
        assert_eq!(settings_file(false, None), None, "no home folder");
    }

    /// The installed file has `version` 1, one flat entry per event with the command and a timeout; a second install
    /// adds nothing, and the first backup of a missing file is empty.
    #[test]
    fn install_adds_flat_entries_once_and_is_idempotent() {
        let dir = temp("install");
        let file = dir.join(".cursor").join("hooks.json");
        assert!(!installed_state(&file).unwrap(), "no file, nothing installed");
        install(&file, "noble").unwrap();
        install(&file, "noble").unwrap();
        assert!(installed_state(&file).unwrap());
        let v = read(&file);
        assert_eq!(v["version"], 1);
        assert_eq!(v.as_object().unwrap().len(), 2, "only version and hooks: {v}");
        let stop = v["hooks"]["stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1, "installed once");
        assert_eq!(stop[0], json!({ "command": "noble hook stop --agent cursor", "timeout": 5 }));
        assert_eq!(v["hooks"]["beforeSubmitPrompt"][0]["command"], "noble hook prompt --agent cursor");
        assert_eq!(v["hooks"]["sessionStart"][0]["command"], "noble hook session-start --agent cursor");
        for event in ["postToolUse", "afterShellExecution", "afterFileEdit"] {
            assert_eq!(v["hooks"][event][0]["command"], "noble hook tool-use --agent cursor", "{event}");
        }
        assert_eq!(std::fs::read_to_string(dir.join(".cursor").join("hooks.json.noble-bak")).unwrap(), "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The user's own entries and keys stay, the first backup keeps the file as it was, and uninstall leaves the
    /// file as it was before.
    #[test]
    fn install_keeps_the_users_entries_and_uninstall_leaves_them() {
        let dir = temp("user");
        let file = dir.join("hooks.json");
        let original = r#"{"version":1,"hooks":{"stop":[{"command":"say done"}]},"editor":"kept"}"#;
        std::fs::write(&file, original).unwrap();
        install(&file, "noble").unwrap();
        let v = read(&file);
        let stop = v["hooks"]["stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2, "user entry kept, ours added");
        assert_eq!(stop[0]["command"], "say done", "the user's entry stays first");
        assert_eq!(v["editor"], "kept");
        assert_eq!(std::fs::read_to_string(dir.join("hooks.json.noble-bak")).unwrap(), original);
        uninstall(&file).unwrap();
        assert_eq!(read(&file), read_text(original), "the file is back as it was");
        assert!(!installed_state(&file).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn read_text(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    /// Uninstall removes NOBLE's entries from every list, drops the lists and `hooks` it empties, and keeps the
    /// user's entries and other keys.
    #[test]
    fn uninstall_removes_only_nobles_entries() {
        let dir = temp("uninstall");
        let file = dir.join("hooks.json");
        let user = json!({ "command": "say done" });
        let mine = json!({ "command": "noble hook stop --agent cursor", "timeout": 5 });
        let settings = json!({
            "version": 1,
            "description": "mine",
            "hooks": { "stop": [user.clone(), mine.clone()], "sessionStart": [mine.clone()] },
        });
        std::fs::write(&file, settings.to_string()).unwrap();
        uninstall(&file).unwrap();
        assert_eq!(
            read(&file),
            json!({ "version": 1, "description": "mine", "hooks": { "stop": [user] } }),
            "other keys and the user's entry are kept"
        );
        // Only NOBLE's entries: `hooks` goes with them and the rest of the file stays.
        std::fs::write(&file, json!({ "version": 1, "hooks": { "stop": [mine] } }).to_string()).unwrap();
        uninstall(&file).unwrap();
        assert_eq!(read(&file), json!({ "version": 1 }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `version` is written only when the file has none: another version is the user's and stays.
    #[test]
    fn version_is_added_when_missing_and_kept_otherwise() {
        let dir = temp("version");
        let file = dir.join("hooks.json");
        std::fs::write(&file, r#"{"hooks":{}}"#).unwrap();
        install(&file, "noble").unwrap();
        assert_eq!(read(&file)["version"], 1);
        std::fs::write(&file, r#"{"version":2,"hooks":{}}"#).unwrap();
        install(&file, "noble").unwrap();
        assert_eq!(read(&file)["version"], 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An install from an older build gets the events it lacks, with the base it was installed with. A file without
    /// NOBLE's entries is not touched, and a missing file is not created.
    #[test]
    fn upgrade_adds_the_events_an_older_install_lacks() {
        let dir = temp("upgrade");
        let file = dir.join("hooks.json");
        let base = quote_for_shell("/opt/my hook tools/noble");
        install(&file, &base).unwrap();
        let mut v = read(&file);
        for event in ["afterFileEdit", "sessionStart"] {
            v["hooks"].as_object_mut().unwrap().remove(event);
        }
        std::fs::write(&file, v.to_string()).unwrap();
        assert!(!installed_state(&file).unwrap());
        upgrade(&file).unwrap();
        assert!(installed_state(&file).unwrap());
        let v = read(&file);
        assert_eq!(v["hooks"]["afterFileEdit"][0]["command"], format!("{base} hook tool-use --agent cursor"));
        assert_eq!(v["hooks"]["stop"].as_array().unwrap().len(), 1, "existing entries are not duplicated");

        let other = dir.join("other.json");
        let text = r#"{ "version": 1, "hooks": { "stop": [ { "command": "say done" } ] } }"#;
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

    /// The installer's shim counts as the program when the folder is not on PATH; nothing else does.
    #[test]
    fn the_installed_copy_is_found_under_localappdata() {
        let dir = temp("program");
        assert_eq!(installed_copy(None), None, "no LOCALAPPDATA (Linux and macOS)");
        assert_eq!(installed_copy(Some(std::ffi::OsStr::new(""))), None);
        assert_eq!(installed_copy(Some(dir.as_os_str())), None, "not installed there");
        let shim = dir.join("cursor-agent").join("cursor-agent.cmd");
        std::fs::create_dir_all(shim.parent().unwrap()).unwrap();
        std::fs::write(&shim, "@echo off\r\n").unwrap();
        assert_eq!(installed_copy(Some(dir.as_os_str())), Some(shim));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A Stop with status `error` is a failed turn; any other status, or none, is a plain stop. The other events keep
    /// their state.
    #[test]
    fn hook_arg_reads_the_stop_status() {
        let errored = json!({ "status": "error", "conversation_id": CONVERSATION });
        assert_eq!(hook_arg(STOP, &errored), STOP_FAILURE);
        assert_eq!(hook_arg(STOP, &json!({ "status": "completed" })), STOP);
        assert_eq!(hook_arg(STOP, &json!({})), STOP);
        assert_eq!(hook_arg(PROMPT, &errored), PROMPT, "only a Stop can fail");
    }

    /// The reply on stdout: a prompt continues, the others are an empty object; every reply is valid JSON.
    #[test]
    fn hooks_answer_with_a_json_object() {
        assert_eq!(reply(PROMPT), r#"{"continue":true}"#);
        for arg in [STOP, STOP_FAILURE, SESSION_START, TOOL_USE] {
            assert_eq!(reply(arg), "{}", "{arg}");
        }
        for arg in [PROMPT, STOP, SESSION_START, TOOL_USE] {
            let _: Value = serde_json::from_str(reply(arg)).unwrap();
        }
        assert_eq!(crate::hooks::hook_reply(Agent::Cursor, PROMPT), Some(reply(PROMPT)));
        assert_eq!(crate::hooks::hook_reply(Agent::Claude, PROMPT), None, "Claude's hooks print nothing");
    }

    /// Each Cursor event leaves the record its state needs: working on the prompt and the tools, idle when the turn
    /// stops, failed when the Stop says so. No chat id is kept, since Cursor has no resume (see `resume::session_id`).
    #[test]
    fn each_event_records_the_state_it_means() {
        let data = temp("cli");
        let hook = |arg: &str, stdin: &str| run_cli(Agent::Cursor, arg, stdin, &data, Some("42"), Some("3"));
        let rec = || read_records(&data, 42).remove(&3);
        hook(SESSION_START, &format!(r#"{{"conversation_id":"{CONVERSATION}","session_id":"{CONVERSATION}"}}"#));
        let r = rec().unwrap();
        assert_eq!((r.event.as_str(), r.agent(), r.message.as_deref()), ("session-start", Agent::Cursor, None));
        assert_eq!(r.session, None, "no chat id is kept for Cursor");
        hook(PROMPT, &format!(r#"{{"conversation_id":"{CONVERSATION}","generation_id":"g1","prompt":"fix it"}}"#));
        assert_eq!(rec().unwrap().event, "prompt");
        hook(TOOL_USE, r#"{"tool_name":"Shell","tool_input":{"command":"cargo test"}}"#);
        assert_eq!(rec().unwrap().event, "tool-use");
        hook(STOP, r#"{"status":"completed","loop_count":0}"#);
        let r = rec().unwrap();
        assert_eq!((r.event.as_str(), r.message), ("stop", None));
        hook(STOP, r#"{"status":"error","loop_count":1}"#);
        let r = rec().unwrap();
        assert_eq!(r.event, "stop-failure");
        assert_eq!(r.message.as_deref(), Some("Cursor stopped with an error"));
        assert_eq!(r.session, None, "a payload without a chat id names none");
        hook(PROMPT, "{}");
        assert_eq!(rec().unwrap().event, "prompt", "a new turn is working again");
        let _ = std::fs::remove_dir_all(&data);
    }
}
