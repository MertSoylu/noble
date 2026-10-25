//! Claude Code's hooks: `~/.claude/settings.json` gets one command per event that runs `noble hook <arg>`.
//! This module also reads Claude's payloads (the JSON it passes on stdin) into the pane's record.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{
    NOTIFICATION, PERMISSION_REQUEST, PROMPT, SESSION_START, STOP, STOP_FAILURE, SUBAGENT_START, SUBAGENT_STOP, field,
    groups_base, groups_install, groups_installed, groups_uninstall, quote_for_shell,
};

/// Claude Code event → `noble hook` argument. The events added since the first release come last, so an older
/// install lacks only the tail (see [`upgrade`]).
pub const EVENTS: [(&str, &str); 9] = [
    ("UserPromptSubmit", PROMPT),
    ("Stop", STOP),
    ("Notification", NOTIFICATION),
    ("SessionStart", SESSION_START),
    ("SessionEnd", super::SESSION_END),
    ("SubagentStart", SUBAGENT_START),
    ("SubagentStop", SUBAGENT_STOP),
    ("PermissionRequest", PERMISSION_REQUEST),
    ("StopFailure", STOP_FAILURE),
];

/// Notifications that ask nothing of the user: the idle reminder Claude sends a while after its answer (it is
/// already "your turn", and it also comes while background subagents still run) and a background subagent
/// finishing (the subagent hooks track that).
const QUIET_NOTIFICATIONS: [&str; 2] = ["idle_prompt", "agent_completed"];
/// The idle reminder's text, for Claude versions that do not send `notification_type`.
const IDLE_MESSAGE: &str = "Claude is waiting for your input";

/// `~/.claude/settings.json`; `None` without a home directory or when `ai::ISOLATED_ENV` is set
/// (tests that run the real binary must never install, upgrade or remove the user's hooks).
pub fn settings_path() -> Option<PathBuf> {
    if crate::ai::isolated() {
        return None;
    }
    settings_in(dirs::home_dir())
}

/// `<home>/.claude/settings.json` for a given home folder (`settings_path` without the isolation check).
pub(super) fn settings_in(home: Option<PathBuf>) -> Option<PathBuf> {
    Some(home?.join(".claude").join("settings.json"))
}

/// The message a Claude payload leaves on the pane's record: `None` when the event is not recorded at all (the
/// reminders that ask nothing of the user). A permission prompt names its tool ("Claude asks to run Bash"), a
/// question is "Claude asks a question", a failed turn says why; other texts are kept as Claude wrote them.
pub fn record_message(event: &str, input: &Value) -> Option<Option<String>> {
    let text = field(input, "message");
    let message = match event {
        NOTIFICATION => {
            let kind = field(input, "notification_type");
            if kind.is_some_and(|k| QUIET_NOTIFICATIONS.contains(&k)) || text == Some(IDLE_MESSAGE) {
                return None;
            }
            match kind {
                Some("permission_prompt") => Some(match tool_name(input, text) {
                    Some(tool) => format!("Claude asks to run {tool}"),
                    None => text.unwrap_or_default().to_string(),
                })
                .filter(|m| !m.is_empty()),
                Some("elicitation_dialog") => Some("Claude asks a question".to_string()),
                _ => text.map(str::to_string),
            }
        }
        PERMISSION_REQUEST => Some(match tool_name(input, text) {
            Some(tool) => format!("Claude asks to run {tool}"),
            None => "Claude asks for permission".to_string(),
        }),
        STOP_FAILURE => {
            field(input, "error").filter(|e| !e.is_empty()).map(|e| format!("Claude stopped with an error: {e}"))
        }
        _ => text.map(str::to_string),
    };
    Some(message.map(|m| crate::util::truncate(&m, 120)))
}

/// The tool a permission prompt asks about: the payload's `tool_name`, else the name the text gives
/// ("Claude needs your permission to use Bash").
fn tool_name<'a>(input: &'a Value, text: Option<&'a str>) -> Option<&'a str> {
    field(input, "tool_name")
        .filter(|t| !t.is_empty())
        .or_else(|| text?.split_once("permission to use ").map(|(_, t)| t.trim()).filter(|t| !t.is_empty()))
}

/// Are the hooks installed for every event? An unreadable or unparseable settings file counts as not installed.
pub fn is_installed(path: &Path) -> bool {
    installed_state(path).unwrap_or(false)
}

/// `is_installed`, but a settings file that cannot be read or parsed is an error (`noble doctor` reports it as
/// unknown). Read-only: the file is never written here.
pub fn installed_state(path: &Path) -> Result<bool, String> {
    groups_installed(path, &EVENTS)
}

/// Adds the missing hooks; touches neither other settings nor other hooks.
pub fn install(path: &Path, base: &str) -> Result<(), String> {
    groups_install(path, &EVENTS, |_, arg| json!({ "type": "command", "command": format!("{base} hook {arg}") }))
}

/// Startup: when the hooks of an older NOBLE are installed, adds the events added since
/// (with the same command) so existing users need not turn the setting off and on.
/// Does nothing when no NOBLE hook is installed or all of them already are.
pub fn upgrade(path: &Path) -> Result<(), String> {
    let Some(base) = groups_base(path, &EVENTS)? else { return Ok(()) };
    if is_installed(path) {
        return Ok(());
    }
    install(path, &requote(&base))
}

/// The base of an older install, quoted the way this build quotes it (an install from before the quoting
/// fix used double quotes, which expand `$`). A bare `noble` from PATH stays bare.
fn requote(base: &str) -> String {
    let raw = base.trim().trim_matches(['"', '\'']);
    if raw == "noble" { raw.to_string() } else { quote_for_shell(raw) }
}

/// Removes the hooks NOBLE added; also deletes lists left empty. A matcher group that holds NOBLE's hook
/// next to the user's own hooks keeps the user's hooks: only NOBLE's entries leave it, and the group goes
/// when nothing is left in it.
pub fn uninstall(path: &Path) -> Result<(), String> {
    groups_uninstall(path, &EVENTS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::{Agent, HOOK_ARGS, is_noble_hook, read_records, run_cli};

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("noble-claude-hooks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// The first backup (the user's original file) survives later writes.
    #[test]
    fn backup_is_written_only_once() {
        let dir = temp("backup");
        let file = dir.join("settings.json");
        std::fs::write(&file, "{\"theme\": \"dark\"}").unwrap();
        install(&file, "noble").unwrap();
        uninstall(&file).unwrap();
        let bak = std::fs::read_to_string(dir.join("settings.json.noble-bak")).unwrap();
        assert_eq!(bak, "{\"theme\": \"dark\"}");
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
        assert_eq!(v["hooks"]["PermissionRequest"][0]["hooks"][0]["command"], "noble hook permission-request");
        assert_eq!(v["hooks"]["StopFailure"][0]["hooks"][0]["command"], "noble hook stop-failure");
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

    /// The reminders are never recorded; the other notifications keep a text that says what they ask.
    #[test]
    fn only_real_notifications_are_recorded() {
        let data = temp("quiet");
        let hook = |event: &str, stdin: &str| run_cli(Agent::Claude, event, stdin, &data, Some("42"), Some("3"));
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
        assert_eq!(read_records(&data, 42)[&3].message.as_deref(), Some("Claude asks to run Bash"));
        hook("notification", r#"{"notification_type":"elicitation_dialog","message":"Which branch?"}"#);
        assert_eq!(read_records(&data, 42)[&3].message.as_deref(), Some("Claude asks a question"));
        let _ = std::fs::remove_dir_all(&data);
    }

    /// A permission request names its tool, a failed turn says why, and each keeps its own event.
    #[test]
    fn permission_requests_and_failed_turns_are_recorded() {
        let data = temp("a2");
        let hook = |event: &str, stdin: &str| run_cli(Agent::Claude, event, stdin, &data, Some("42"), Some("3"));
        let rec = || read_records(&data, 42)[&3].clone();
        hook("permission-request", r#"{"tool_name":"Bash","tool_input":{"command":"ls"}}"#);
        let r = rec();
        assert_eq!(r.event, "permission-request");
        assert_eq!(r.message.as_deref(), Some("Claude asks to run Bash"));
        hook("permission-request", "{}");
        assert_eq!(rec().message.as_deref(), Some("Claude asks for permission"));
        hook("stop-failure", r#"{"error":"rate_limit"}"#);
        let r = rec();
        assert_eq!(r.event, "stop-failure");
        assert_eq!(r.message.as_deref(), Some("Claude stopped with an error: rate_limit"));
        hook("stop-failure", "{}");
        assert_eq!(rec().message, None, "no reason given: the notice falls back to its own text");
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn record_message_table() {
        let msg = |event: &str, json: &str| record_message(event, &serde_json::from_str(json).unwrap());
        // The permission prompt's tool comes from the payload, else from the text.
        assert_eq!(
            msg("notification", r#"{"notification_type":"permission_prompt","tool_name":"Edit"}"#),
            Some(Some("Claude asks to run Edit".into()))
        );
        assert_eq!(
            msg(
                "notification",
                r#"{"notification_type":"permission_prompt","message":"Claude needs your permission to use Read"}"#
            ),
            Some(Some("Claude asks to run Read".into()))
        );
        // A prompt without a tool keeps its text; without any text it has none.
        assert_eq!(
            msg("notification", r#"{"notification_type":"permission_prompt","message":"Approve this?"}"#),
            Some(Some("Approve this?".into()))
        );
        assert_eq!(msg("notification", r#"{"notification_type":"permission_prompt"}"#), Some(None));
        // Other events keep their message as before.
        assert_eq!(msg("prompt", r#"{"message":"  hi  "}"#), Some(Some("hi".into())));
        assert_eq!(msg("stop", "{}"), Some(None));
        // The reminders are not recorded at all.
        assert_eq!(msg("notification", r#"{"notification_type":"idle_prompt"}"#), None);
        assert_eq!(msg("notification", r#"{"message":"Claude is waiting for your input"}"#), None);
        // A long message is cut.
        let long = format!(r#"{{"message":"{}"}}"#, "x".repeat(300));
        assert_eq!(msg("prompt", &long).flatten().map(|m| m.chars().count()), Some(120));
    }

    /// The events added since the first release are in the table and the install; `upgrade` adds them to an
    /// older install with the same command.
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
        // The added event is written with this build's quoting (`requote`); the existing ones are untouched.
        let added = format!("{} hook subagent-stop", quote_for_shell(r"C:\Tools\noble.exe"));
        assert_eq!(v["hooks"]["SubagentStop"][0]["hooks"][0]["command"], added);
        let failure = format!("{} hook stop-failure", quote_for_shell(r"C:\Tools\noble.exe"));
        assert_eq!(v["hooks"]["StopFailure"][0]["hooks"][0]["command"], failure);
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
        // The events it adds carry the whole path (not cut at its " hook "), quoted for the shell.
        let added = format!("{} hook subagent-start", quote_for_shell("/opt/my hook tools/noble"));
        assert_eq!(v["hooks"]["SubagentStart"][0]["hooks"][0]["command"], added);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The base is one word for the shell that runs hooks: nothing in it expands, and a `'` survives.
    #[test]
    fn hook_base_is_quoted_for_the_shell() {
        if cfg!(windows) {
            // Windows: double quotes, as before (cmd.exe and Git Bash read them).
            assert_eq!(quote_for_shell(r"C:\my tools\noble.exe"), r#""C:\my tools\noble.exe""#);
        } else {
            // Unix: single quotes, so `$`, backticks and backslashes stay literal.
            assert_eq!(quote_for_shell("/opt/a b/$X/`y`/noble"), "'/opt/a b/$X/`y`/noble'");
            assert_eq!(quote_for_shell("/o/it's/noble"), r"'/o/it'\''s/noble'");
        }
        let base = quote_for_shell("/o/it's/$HOME/noble");
        assert!(is_noble_hook(&json!({ "hooks": [ { "type": "command", "command": format!("{base} hook stop") } ] })));
        assert_eq!(requote("noble"), "noble");
        assert_eq!(requote(r#""/opt/$X/noble""#), quote_for_shell("/opt/$X/noble"));
    }

    /// An install written with double quotes (before this fix) gets the events it lacks in this build's form.
    #[test]
    fn upgrade_quotes_the_base_of_the_events_it_adds() {
        let dir = temp("upgrade-quote");
        let file = dir.join("settings.json");
        let legacy = r#""/opt/$X/noble""#;
        let mut hooks = serde_json::Map::new();
        for (event, arg) in &EVENTS[..5] {
            hooks.insert(
                event.to_string(),
                json!([{ "hooks": [ { "type": "command", "command": format!("{legacy} hook {arg}") } ] }]),
            );
        }
        std::fs::write(&file, serde_json::to_string(&json!({ "hooks": hooks })).unwrap()).unwrap();
        upgrade(&file).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        for (event, arg) in &EVENTS[5..] {
            let command = &v["hooks"][event][0]["hooks"][0]["command"];
            assert_eq!(command, &format!("{} hook {arg}", quote_for_shell("/opt/$X/noble")), "{event}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Uninstall removes NOBLE's hook from a matcher group and nothing else: a group that also holds the user's
    /// own hook keeps it, and only a group left empty goes.
    #[test]
    fn uninstall_keeps_the_users_hooks_in_a_shared_group() {
        let dir = temp("shared-group");
        let file = dir.join("settings.json");
        let user = json!({ "hooks": [ { "type": "command", "command": "say done" } ] });
        let mine = json!({ "hooks": [ { "type": "command", "command": "noble hook stop" } ] });
        let shared = json!({ "matcher": "Bash", "hooks": [
            { "type": "command", "command": "say done" },
            { "type": "command", "command": "noble hook stop" },
        ] });
        let settings = json!({ "hooks": {
            "Stop": [ user.clone(), mine.clone(), shared ],
            "Notification": [ mine ],
        } });
        std::fs::write(&file, settings.to_string()).unwrap();
        uninstall(&file).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        let stop = v["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(
            stop,
            &vec![user, json!({ "matcher": "Bash", "hooks": [ { "type": "command", "command": "say done" } ] })]
        );
        assert!(v["hooks"].get("Notification").is_none(), "an empty list is removed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A backup cut short by an interrupted plain copy (an older build) is taken again from the settings file
    /// while that file is still the original; the new backup is written in one step (no temp file left).
    #[test]
    fn a_truncated_backup_is_taken_again() {
        let dir = temp("backup-repair");
        let file = dir.join("settings.json");
        let original = "{ \"model\": \"opus\" }";
        std::fs::write(&file, original).unwrap();
        std::fs::write(dir.join("settings.json.noble-bak"), "{ \"mo").unwrap();
        install(&file, "noble").unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("settings.json.noble-bak")).unwrap(), original);
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
        assert!(left.iter().all(|n| !n.to_string_lossy().ends_with(".tmp")), "{left:?}");
        // A good backup is never overwritten by a later write.
        install(&file, "noble").unwrap();
        uninstall(&file).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("settings.json.noble-bak")).unwrap(), original);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every event of the table is a `noble hook` argument, and the table has no event twice.
    #[test]
    fn every_event_is_a_known_hook_argument() {
        for (_, arg) in EVENTS {
            assert!(HOOK_ARGS.contains(&arg), "{arg}");
        }
        let mut names: Vec<&str> = EVENTS.iter().map(|(e, _)| *e).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), EVENTS.len());
    }
}
