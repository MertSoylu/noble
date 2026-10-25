//! Codex's hooks: `hooks.json` in Codex's home folder (`$CODEX_HOME`, else `~/.codex`) gets one command per event
//! that runs `noble hook <arg> --agent codex`. The file has Claude's layout (matcher groups), so the shared helpers
//! in `super` do the editing. Codex runs user-level hooks only after the user has trusted them once in its `/hooks`
//! view, so an install is followed by [`TRUST_NOTICE`]; NOBLE never writes Codex's trust itself.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{
    Agent, PERMISSION_REQUEST, PROMPT, SESSION_END, SESSION_START, STOP, SUBAGENT_START, SUBAGENT_STOP, field,
    groups_base, groups_install, groups_installed, groups_uninstall, plain_message,
};

/// Codex event → `noble hook` argument. `Interrupt` ends the turn like `Stop` does, so it maps to the same
/// argument: an interrupted turn is no longer working.
pub const EVENTS: [(&str, &str); 8] = [
    ("UserPromptSubmit", PROMPT),
    ("Stop", STOP),
    ("Interrupt", STOP),
    ("PermissionRequest", PERMISSION_REQUEST),
    ("SessionStart", SESSION_START),
    ("SessionEnd", SESSION_END),
    ("SubagentStart", SUBAGENT_START),
    ("SubagentStop", SUBAGENT_STOP),
];

/// Shown once after Codex's hooks were installed: Codex does not run them before the user trusts them.
pub const TRUST_NOTICE: &str = "Codex: open /hooks once to trust NOBLE's hooks";

/// The folder Codex keeps its config in: `CODEX_HOME` when it names an existing folder (Codex requires one),
/// else `~/.codex`. `None` without a home folder.
fn codex_home(env: Option<&OsStr>, home: Option<PathBuf>) -> Option<PathBuf> {
    match env.map(PathBuf::from) {
        Some(dir) if !dir.as_os_str().is_empty() && dir.is_dir() => Some(dir),
        _ => home.map(|h| h.join(".codex")),
    }
}

/// `hooks.json` in the Codex folder; `None` when `ai::isolated` is set or there is no home folder.
pub fn settings_path() -> Option<PathBuf> {
    settings_file(crate::ai::isolated(), std::env::var_os("CODEX_HOME").as_deref(), dirs::home_dir())
}

pub(super) fn settings_file(isolated: bool, env: Option<&OsStr>, home: Option<PathBuf>) -> Option<PathBuf> {
    if isolated {
        return None;
    }
    codex_home(env, home).map(|dir| dir.join("hooks.json"))
}

/// One hook of an event. Codex clamps `SessionEnd` and `Interrupt` to 3 seconds; the others get 5.
fn hook(event: &str, base: &str, arg: &str) -> Value {
    let timeout = if matches!(event, "SessionEnd" | "Interrupt") { 3 } else { 5 };
    let command = format!("{base} hook {arg} --agent {}", Agent::Codex.id());
    json!({ "type": "command", "command": command, "timeout": timeout })
}

/// Is every event installed with one of NOBLE's hooks? An unreadable file is an error.
pub fn installed_state(path: &Path) -> Result<bool, String> {
    groups_installed(path, &EVENTS)
}

/// Adds the missing hooks; touches neither other settings nor the user's hooks.
pub fn install(path: &Path, base: &str) -> Result<(), String> {
    groups_install(path, &EVENTS, |event, arg| hook(event, base, arg))
}

/// Removes NOBLE's hooks and nothing else (see `groups_uninstall`).
pub fn uninstall(path: &Path) -> Result<(), String> {
    groups_uninstall(path, &EVENTS)
}

/// Startup: a file with NOBLE's hooks from an older build gets the events added since, with the same command.
pub fn upgrade(path: &Path) -> Result<(), String> {
    let Some(base) = groups_base(path, &EVENTS)? else { return Ok(()) };
    if installed_state(path)? {
        return Ok(());
    }
    install(path, &base)
}

/// The message a Codex payload leaves on the pane's record (see `claude::record_message`): a permission request
/// names its tool, other events keep their `message` if they have one.
pub fn record_message(event: &str, input: &Value) -> Option<Option<String>> {
    if event != PERMISSION_REQUEST {
        return plain_message(input);
    }
    let text = match field(input, "tool_name").filter(|t| !t.is_empty()) {
        Some(tool) => format!("Codex asks to run {tool}"),
        None => "Codex asks for permission".to_string(),
    };
    Some(Some(crate::util::truncate(&text, 120)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::{HOOK_ARGS, quote_for_shell, read_records, run_cli};

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("noble-codex-hooks-{name}-{}", std::process::id()));
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

    /// `CODEX_HOME` counts only when it names an existing folder; otherwise the default folder under home.
    #[test]
    fn codex_home_is_taken_from_the_environment_when_it_is_a_folder() {
        let dir = temp("home");
        let custom = dir.join("custom");
        std::fs::create_dir_all(&custom).unwrap();
        let default = dir.join("user").join(".codex");
        let home = Some(dir.join("user"));
        assert_eq!(codex_home(None, home.clone()), Some(default.clone()));
        assert_eq!(codex_home(Some(OsStr::new("")), home.clone()), Some(default.clone()));
        assert_eq!(codex_home(Some(dir.join("missing").as_os_str()), home.clone()), Some(default));
        assert_eq!(codex_home(Some(custom.as_os_str()), home), Some(custom.clone()));
        assert_eq!(codex_home(None, None), None, "no home folder");
        assert_eq!(codex_home(Some(custom.as_os_str()), None), Some(custom));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Under `ai::isolated` the settings path is off, so no test or isolated run touches `~/.codex`.
    #[test]
    fn isolation_turns_the_settings_path_off() {
        let home = Some(PathBuf::from("/home/me"));
        assert_eq!(settings_file(true, None, home.clone()), None);
        assert_eq!(settings_file(true, Some(OsStr::new("/srv/codex")), home.clone()), None);
        assert_eq!(settings_file(false, None, home), Some(PathBuf::from("/home/me/.codex/hooks.json")));
        assert_eq!(settings_file(false, None, None), None, "no home folder");
    }

    #[test]
    fn install_adds_every_event_once_and_is_idempotent() {
        let dir = temp("install");
        let file = dir.join("codex-home").join("hooks.json");
        assert!(!installed_state(&file).unwrap(), "no file, nothing installed");
        install(&file, "noble").unwrap();
        install(&file, "noble").unwrap();
        assert!(installed_state(&file).unwrap());
        let v = read(&file);
        assert_eq!(v.as_object().unwrap().len(), 1, "only the hooks key: {v}");
        let stop = v["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1, "installed once");
        assert_eq!(stop[0]["hooks"][0]["command"], "noble hook stop --agent codex");
        assert_eq!(stop[0]["hooks"][0]["timeout"], 5);
        assert_eq!(v["hooks"]["Interrupt"][0]["hooks"][0]["command"], "noble hook stop --agent codex");
        assert_eq!(v["hooks"]["Interrupt"][0]["hooks"][0]["timeout"], 3);
        assert_eq!(v["hooks"]["SessionEnd"][0]["hooks"][0]["timeout"], 3);
        assert_eq!(
            v["hooks"]["PermissionRequest"][0]["hooks"][0]["command"],
            "noble hook permission-request --agent codex"
        );
        let backup = dir.join("codex-home").join("hooks.json.noble-bak");
        assert_eq!(std::fs::read_to_string(backup).unwrap(), "", "an empty backup: the file did not exist");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The user's own hooks and other keys stay; the first backup keeps the file as it was.
    #[test]
    fn install_keeps_the_users_hooks_and_uninstall_leaves_them() {
        let dir = temp("user");
        let file = dir.join("hooks.json");
        let original = r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"say done"}]}]}}"#;
        std::fs::write(&file, original).unwrap();
        install(&file, "noble").unwrap();
        let v = read(&file);
        let stop = v["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2, "user hook kept, ours added");
        assert_eq!(stop[0]["hooks"][0]["command"], "say done", "the user's hook stays first");
        assert_eq!(std::fs::read_to_string(dir.join("hooks.json.noble-bak")).unwrap(), original);
        uninstall(&file).unwrap();
        let v = read(&file);
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], "say done");
        assert!(v["hooks"].get("Interrupt").is_none());
        assert!(!installed_state(&file).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A group that holds the user's hook next to NOBLE's keeps the user's hook; only NOBLE's entries leave.
    #[test]
    fn uninstall_removes_only_nobles_hooks() {
        let dir = temp("uninstall");
        let file = dir.join("hooks.json");
        let user = json!({ "hooks": [ { "type": "command", "command": "say done" } ] });
        let mine = json!({ "hooks": [ { "type": "command", "command": "noble hook stop --agent codex" } ] });
        let shared = json!({ "matcher": "*", "hooks": [
            { "type": "command", "command": "say done" },
            { "type": "command", "command": "noble hook stop --agent codex" },
        ] });
        let settings = json!({ "description": "mine", "hooks": {
            "Stop": [ user.clone(), mine.clone(), shared ],
            "Interrupt": [ mine ],
        } });
        std::fs::write(&file, settings.to_string()).unwrap();
        uninstall(&file).unwrap();
        let v = read(&file);
        assert_eq!(v["description"], "mine", "other keys are kept");
        assert_eq!(
            v["hooks"],
            json!({ "Stop": [ user, { "matcher": "*", "hooks": [ { "type": "command", "command": "say done" } ] } ] })
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An install from an older build gets the events it lacks, with the base it was installed with.
    #[test]
    fn upgrade_adds_the_events_an_older_install_lacks() {
        let dir = temp("upgrade");
        let file = dir.join("hooks.json");
        let base = quote_for_shell("/opt/my hook tools/noble");
        install(&file, &base).unwrap();
        let mut v = read(&file);
        for event in ["Interrupt", "SubagentStop", "SessionEnd"] {
            v["hooks"].as_object_mut().unwrap().remove(event);
        }
        std::fs::write(&file, v.to_string()).unwrap();
        assert!(!installed_state(&file).unwrap());
        upgrade(&file).unwrap();
        assert!(installed_state(&file).unwrap());
        let v = read(&file);
        assert_eq!(
            v["hooks"]["SubagentStop"][0]["hooks"][0]["command"],
            format!("{base} hook subagent-stop --agent codex")
        );
        assert_eq!(v["hooks"]["SessionEnd"][0]["hooks"][0]["timeout"], 3);
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 1, "existing hooks are not duplicated");
        // A file without NOBLE's hooks is left alone, and a missing file is not created.
        let other = dir.join("other.json");
        let text = r#"{ "description": "mine", "hooks": {} }"#;
        std::fs::write(&other, text).unwrap();
        upgrade(&other).unwrap();
        assert_eq!(std::fs::read_to_string(&other).unwrap(), text);
        upgrade(&dir.join("none.json")).unwrap();
        assert!(!dir.join("none.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Each Codex event leaves the record its state needs: working, needs you, idle with running subagents,
    /// and cleared at the end of the session.
    #[test]
    fn each_event_records_the_state_it_means() {
        let data = temp("cli");
        let hook = |arg: &str, stdin: &str| run_cli(Agent::Codex, arg, stdin, &data, Some("42"), Some("3"));
        let rec = || read_records(&data, 42).remove(&3);
        hook(SESSION_START, r#"{"source":"startup"}"#);
        let r = rec().unwrap();
        assert_eq!((r.event.as_str(), r.agent(), r.message), ("session-start", Agent::Codex, None));
        hook(PROMPT, r#"{"prompt":"fix the build"}"#);
        assert_eq!(rec().unwrap().event, "prompt");
        hook(PERMISSION_REQUEST, r#"{"tool_name":"shell","tool_input":{"command":["cargo","test"]}}"#);
        let r = rec().unwrap();
        assert_eq!(r.event, "permission-request");
        assert_eq!(r.message.as_deref(), Some("Codex asks to run shell"));
        hook(PERMISSION_REQUEST, "{}");
        assert_eq!(rec().unwrap().message.as_deref(), Some("Codex asks for permission"));
        hook(SUBAGENT_START, r#"{"agent_id":"sub-1","agent_type":"explorer"}"#);
        hook(STOP, r#"{"last_assistant_message":"done","stop_hook_active":false}"#);
        let r = rec().unwrap();
        assert_eq!(r.event, "stop");
        assert_eq!(r.subagents, 1, "a subagent still runs after the turn ended");
        hook(SUBAGENT_STOP, r#"{"agent_id":"sub-1","agent_type":"explorer"}"#);
        assert_eq!(rec().unwrap().subagents, 0);
        hook(SESSION_END, r#"{"reason":"other"}"#);
        assert!(rec().is_none(), "the session ended: no record");
        let _ = std::fs::remove_dir_all(&data);
    }
}
