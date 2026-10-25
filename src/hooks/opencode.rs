//! OpenCode's hooks: a plugin file NOBLE owns, `plugins/noble.js` in OpenCode's config folder (`$XDG_CONFIG_HOME/opencode`,
//! else `~/.config/opencode`). OpenCode loads every plugin in that folder at startup. The plugin forwards the session
//! and permission events to `noble hook <event> --agent opencode` with the event as JSON on stdin. The event names are
//! OpenCode's own, so [`hook_arg`] maps them to the shared hook arguments (`session.status` carries busy or idle in its
//! payload). Install writes the file only when it is missing or already NOBLE's; uninstall deletes only NOBLE's file.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{PERMISSION_REQUEST, PROMPT, STOP, STOP_FAILURE, field, plain_message};

/// OpenCode event → the hook argument it maps to. `session.status` is mapped by its payload (see [`hook_arg`]); the
/// plugin forwards exactly these names. `message.part.updated` fires for every token, so it is not forwarded.
pub const EVENTS: [(&str, &str); 6] = [
    // busy → prompt (working), idle → stop; `retry` keeps the state (see `hook_arg`).
    ("session.status", PROMPT),
    ("session.idle", STOP),
    ("session.error", STOP_FAILURE),
    ("permission.asked", PERMISSION_REQUEST),
    ("permission.updated", PERMISSION_REQUEST),
    // The user answered the permission: the agent carries on.
    ("permission.replied", PROMPT),
];

/// The first line of the plugin file: how NOBLE recognizes its own file. A file without it is never touched.
const MARKER: &str = "// NOBLE OpenCode plugin";

/// The plugin. `__MARKER__`, `__FORWARD__` and `__NOBLE_BIN__` are replaced (see [`render`]). It starts `noble`
/// without a shell (the path is a JS string), never throws, and does nothing outside NOBLE's panes.
const TEMPLATE: &str = r##"__MARKER__: written by NOBLE when the hooks are installed and at every start. Edits are
// overwritten; NOBLE's Settings turns the hooks off and removes this file.
// It forwards session and permission events to `noble hook <event> --agent opencode`, with the event as JSON on stdin.
// Outside NOBLE (no NOBLE_PANE or NOBLE_INSTANCE in the environment) it does nothing. It never throws or waits.
import { spawn } from "node:child_process";

const NOBLE_BIN = __NOBLE_BIN__;
const FORWARD = new Set(__FORWARD__);

function forward(event) {
  try {
    const child = spawn(NOBLE_BIN, ["hook", event.type, "--agent", "opencode"], {
      stdio: ["pipe", "ignore", "ignore"],
      windowsHide: true,
    });
    child.on("error", () => {}); // a missing noble arrives here; OpenCode is not affected
    child.stdin.on("error", () => {}); // noble may exit before it reads the event
    child.stdin.end(JSON.stringify(event));
    child.unref(); // OpenCode does not wait for the hook
  } catch {
    // never throw into OpenCode
  }
}

export const NoblePlugin = async () => ({
  event: async ({ event }) => {
    try {
      const env = globalThis.process?.env ?? {};
      if (!env.NOBLE_PANE || !env.NOBLE_INSTANCE) return;
      if (event && FORWARD.has(event.type)) forward(event);
    } catch {
      // never throw into OpenCode
    }
  },
});
"##;

/// `noble.js` in OpenCode's plugin folder; `None` when `ai::isolated` is set or there is no home folder.
pub fn settings_path() -> Option<PathBuf> {
    settings_file(crate::ai::isolated(), std::env::var_os("XDG_CONFIG_HOME").as_deref(), dirs::home_dir())
}

pub(super) fn settings_file(isolated: bool, xdg: Option<&OsStr>, home: Option<PathBuf>) -> Option<PathBuf> {
    if isolated {
        return None;
    }
    config_home(xdg, home).map(|dir| dir.join("plugins").join("noble.js"))
}

/// OpenCode's config folder: `$XDG_CONFIG_HOME/opencode` when that is set, else `~/.config/opencode`.
fn config_home(xdg: Option<&OsStr>, home: Option<PathBuf>) -> Option<PathBuf> {
    match xdg {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir).join("opencode")),
        _ => home.map(|h| h.join(".config").join("opencode")),
    }
}

/// The hook argument for an OpenCode event (the name the plugin forwards, and the event JSON). `None` for an event
/// that changes nothing (a retry, a token) or that NOBLE does not forward.
pub fn hook_arg(native: &str, input: &Value) -> Option<&'static str> {
    if native == "session.status" {
        return match input.pointer("/properties/status/type").and_then(Value::as_str)? {
            "busy" => Some(PROMPT),
            "idle" => Some(STOP),
            // A retry after an error: the agent is still working.
            _ => None,
        };
    }
    EVENTS.iter().find(|(name, _)| *name == native).map(|(_, arg)| *arg)
}

/// The message an OpenCode event leaves on the pane's record (see `claude::record_message`). A permission request
/// names the permission (`bash`, `edit` …); a failed turn names the error when it has one.
pub fn record_message(event: &str, input: &Value) -> Option<Option<String>> {
    let props = input.get("properties");
    let text = match event {
        PERMISSION_REQUEST => match props.and_then(|p| field(p, "type").or_else(|| field(p, "permission"))) {
            Some(kind) if !kind.is_empty() => format!("OpenCode asks to use {kind}"),
            _ => "OpenCode asks for permission".to_string(),
        },
        STOP_FAILURE => {
            let error = props.and_then(|p| p.get("error"));
            let reason = match error {
                Some(Value::String(s)) => Some(s.as_str()),
                Some(e) => e
                    .pointer("/data/message")
                    .or_else(|| e.get("message"))
                    .or_else(|| e.get("name"))
                    .and_then(Value::as_str),
                None => None,
            };
            match reason.map(str::trim).filter(|r| !r.is_empty()) {
                Some(reason) => format!("OpenCode stopped with an error: {reason}"),
                None => "OpenCode stopped with an error".to_string(),
            }
        }
        _ => return plain_message(input),
    };
    Some(Some(crate::util::truncate(&text, 120)))
}

/// The plugin file's text when it is NOBLE's (it starts with [`MARKER`]); `None` when there is no file.
fn read_file(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

fn is_ours(text: &str) -> bool {
    text.starts_with(MARKER)
}

/// Is NOBLE's plugin file there? A file that cannot be read is an error; another file of that name is not NOBLE's.
pub fn installed_state(path: &Path) -> Result<bool, String> {
    Ok(read_file(path)?.is_some_and(|t| is_ours(&t)))
}

/// The program of a hook command base (`command_base`, shell-quoted by `quote_for_shell`). The plugin starts it
/// without a shell, so the quotes come off: the exact inverse of `quote_for_shell`.
fn program_of(base: &str) -> String {
    if let Some(inner) = base.strip_prefix('"').and_then(|b| b.strip_suffix('"')) {
        return inner.to_string();
    }
    if let Some(inner) = base.strip_prefix('\'').and_then(|b| b.strip_suffix('\'')) {
        return inner.replace(r"'\''", "'");
    }
    base.to_string()
}

/// The plugin text for `program`. The program is a JSON string, which is also a valid JS string literal, so any
/// path (backslashes, quotes) reaches the plugin unchanged.
fn render(program: &str) -> String {
    let forward: Vec<&str> = EVENTS.iter().map(|(name, _)| *name).collect();
    TEMPLATE
        .replace("__MARKER__", MARKER)
        .replace("__FORWARD__", &json!(forward).to_string())
        .replace("__NOBLE_BIN__", &json!(program).to_string())
}

/// The program a NOBLE plugin file starts (its `const NOBLE_BIN` line); `None` when the line is not there.
fn program_in(text: &str) -> Option<String> {
    let literal = text.lines().find_map(|l| l.strip_prefix("const NOBLE_BIN = "))?;
    serde_json::from_str::<String>(literal.strip_suffix(';')?).ok()
}

/// Writes the plugin for `base` (the hook command base). A file that is not NOBLE's is never overwritten; a file
/// that already has this content is left alone.
pub fn install(path: &Path, base: &str) -> Result<(), String> {
    let current = read_file(path)?;
    if current.as_deref().is_some_and(|t| !is_ours(t)) {
        return Err(format!(
            "{} exists and was not written by NOBLE; move it aside to install the hooks",
            path.display()
        ));
    }
    let content = render(&program_of(base));
    if current.as_deref() == Some(content.as_str()) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    crate::store::write_text(path, &content).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Deletes NOBLE's plugin file. Another file of that name, and a missing one, are left alone.
pub fn uninstall(path: &Path) -> Result<(), String> {
    match read_file(path)? {
        Some(text) if is_ours(&text) => {
            std::fs::remove_file(path).map_err(|e| format!("cannot remove {}: {e}", path.display()))
        }
        _ => Ok(()),
    }
}

/// Startup: NOBLE's plugin from an older build is rewritten with the same program when its text differs. A missing
/// file is not created here, and another file is never touched.
pub fn upgrade(path: &Path) -> Result<(), String> {
    let Some(text) = read_file(path)?.filter(|t| is_ours(t)) else { return Ok(()) };
    let Some(program) = program_in(&text) else { return Ok(()) };
    let content = render(&program);
    if content == text {
        return Ok(());
    }
    crate::store::write_text(path, &content).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::{Agent, HOOK_ARGS, quote_for_shell, read_records, run_cli};

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("noble-opencode-hooks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
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
    fn config_home_follows_xdg_then_the_home_folder() {
        let home = Some(PathBuf::from("/home/me"));
        assert_eq!(config_home(None, home.clone()), Some(PathBuf::from("/home/me/.config/opencode")));
        assert_eq!(config_home(Some(OsStr::new("")), home.clone()), Some(PathBuf::from("/home/me/.config/opencode")));
        assert_eq!(config_home(Some(OsStr::new("/srv/xdg")), home), Some(PathBuf::from("/srv/xdg/opencode")));
        assert_eq!(config_home(None, None), None, "no home folder");
    }

    /// Under `ai::isolated` the plugin path is off, so no test or isolated run touches the real OpenCode folder.
    #[test]
    fn isolation_turns_the_settings_path_off() {
        let home = Some(PathBuf::from("/home/me"));
        assert_eq!(settings_file(true, None, home.clone()), None);
        assert_eq!(settings_file(true, Some(OsStr::new("/srv/xdg")), home.clone()), None);
        assert_eq!(settings_file(false, None, home), Some(PathBuf::from("/home/me/.config/opencode/plugins/noble.js")));
        assert_eq!(
            settings_file(false, Some(OsStr::new("/srv/xdg")), None),
            Some(PathBuf::from("/srv/xdg/opencode/plugins/noble.js")),
            "XDG_CONFIG_HOME moves the file even without a home folder"
        );
    }

    /// The plugin carries the Windows path as a JS string: backslashes are escaped, and the command runs
    /// `noble hook <event> --agent opencode` without a shell.
    #[test]
    fn install_writes_the_plugin_with_the_escaped_program() {
        let dir = temp("install");
        let file = dir.join("plugins").join("noble.js");
        let program = r"C:\Users\Mert\.cargo\bin\noble.exe";
        assert!(!installed_state(&file).unwrap());
        install(&file, &quote_for_shell(program)).unwrap();
        install(&file, &quote_for_shell(program)).unwrap();
        assert!(installed_state(&file).unwrap());
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.starts_with(MARKER), "the NOBLE marker comes first");
        assert!(!text.contains("__"), "every placeholder is replaced");
        assert!(text.contains(r#"const NOBLE_BIN = "C:\\Users\\Mert\\.cargo\\bin\\noble.exe";"#), "{text}");
        assert_eq!(program_in(&text).as_deref(), Some(program));
        assert!(text.contains(r#"spawn(NOBLE_BIN, ["hook", event.type, "--agent", "opencode"]"#));
        assert!(text.contains(r#""session.status""#) && text.contains(r#""permission.replied""#));
        assert!(!text.contains(r#""message.part.updated""#), "per-token events are not forwarded");
        assert!(text.contains("NOBLE_PANE") && text.contains("unref()"), "silent outside NOBLE, never waits");
        // Nothing but the plugin file is written next to it.
        let names: Vec<String> = std::fs::read_dir(file.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["noble.js".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Paths with quotes, apostrophes and spaces survive the shell quoting and the JS string unchanged.
    #[test]
    fn awkward_paths_reach_the_plugin_unchanged() {
        for program in ["/opt/it's here/noble", "/opt/q\"uote/noble", r"C:\my tools\noble.exe", "/opt/a\\b/noble"] {
            assert_eq!(program_of(&quote_for_shell(program)), program);
            assert_eq!(program_in(&render(program)).as_deref(), Some(program), "{program}");
        }
        assert_eq!(program_of("noble"), "noble", "a bare name from PATH has no quotes");
    }

    #[test]
    fn install_moves_the_program_when_it_changes() {
        let dir = temp("moved");
        let file = dir.join("noble.js");
        install(&file, &quote_for_shell("/opt/old/noble")).unwrap();
        install(&file, &quote_for_shell("/opt/new/noble")).unwrap();
        assert_eq!(program_in(&std::fs::read_to_string(&file).unwrap()).as_deref(), Some("/opt/new/noble"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file of that name that NOBLE did not write is kept: install refuses, uninstall and upgrade leave it.
    #[test]
    fn another_file_of_that_name_is_never_touched() {
        let dir = temp("foreign");
        let file = dir.join("noble.js");
        std::fs::write(&file, "export const Mine = () => ({});\n").unwrap();
        assert!(!installed_state(&file).unwrap());
        assert!(install(&file, "noble").is_err());
        uninstall(&file).unwrap();
        upgrade(&file).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "export const Mine = () => ({});\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn uninstall_deletes_only_nobles_file() {
        let dir = temp("uninstall");
        let file = dir.join("noble.js");
        install(&file, "noble").unwrap();
        uninstall(&file).unwrap();
        assert!(!file.exists());
        // Again, with no file: harmless.
        uninstall(&file).unwrap();
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An older NOBLE's plugin is rewritten with the same program; a missing file is not created.
    #[test]
    fn upgrade_rewrites_an_older_plugin_with_its_program() {
        let dir = temp("upgrade");
        let file = dir.join("noble.js");
        upgrade(&file).unwrap();
        assert!(!file.exists(), "no plugin, nothing to upgrade");
        let old = "// NOBLE OpenCode plugin: written by an older build\nconst NOBLE_BIN = \"/opt/noble\";\n";
        std::fs::write(&file, old).unwrap();
        upgrade(&file).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(text, render("/opt/noble"));
        upgrade(&file).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), text, "an up-to-date plugin is left alone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hook_arg_maps_the_forwarded_events() {
        let status =
            |t: &str| json!({ "type": "session.status", "properties": { "sessionID": "s1", "status": { "type": t } } });
        assert_eq!(hook_arg("session.status", &status("busy")), Some(PROMPT));
        assert_eq!(hook_arg("session.status", &status("idle")), Some(STOP));
        assert_eq!(hook_arg("session.status", &status("retry")), None, "a retry is still working");
        assert_eq!(hook_arg("session.status", &json!({})), None, "no status: nothing to map");
        assert_eq!(hook_arg("session.idle", &json!({})), Some(STOP));
        assert_eq!(hook_arg("session.error", &json!({})), Some(STOP_FAILURE));
        assert_eq!(hook_arg("permission.asked", &json!({})), Some(PERMISSION_REQUEST));
        assert_eq!(hook_arg("permission.updated", &json!({})), Some(PERMISSION_REQUEST));
        assert_eq!(hook_arg("permission.replied", &json!({})), Some(PROMPT));
        assert_eq!(hook_arg("message.part.updated", &json!({})), None, "not forwarded");
        assert_eq!(hook_arg("stop", &json!({})), None, "a hook argument is not an OpenCode event");
    }

    #[test]
    fn record_messages_name_the_permission_and_the_error() {
        let asked = json!({ "type": "permission.asked", "properties": { "type": "bash" } });
        assert_eq!(record_message(PERMISSION_REQUEST, &asked), Some(Some("OpenCode asks to use bash".into())));
        assert_eq!(
            record_message(PERMISSION_REQUEST, &json!({ "properties": {} })),
            Some(Some("OpenCode asks for permission".into()))
        );
        let failed =
            json!({ "properties": { "error": { "name": "APIError", "data": { "message": " rate limited " } } } });
        assert_eq!(
            record_message(STOP_FAILURE, &failed),
            Some(Some("OpenCode stopped with an error: rate limited".into()))
        );
        let named = json!({ "properties": { "error": { "name": "ProviderAuthError" } } });
        assert_eq!(
            record_message(STOP_FAILURE, &named),
            Some(Some("OpenCode stopped with an error: ProviderAuthError".into()))
        );
        assert_eq!(record_message(STOP_FAILURE, &json!({})), Some(Some("OpenCode stopped with an error".into())));
        assert_eq!(record_message(PROMPT, &json!({})), Some(None), "a prompt has no message");
    }

    /// The events the plugin forwards are recorded as the states they mean, under the OpenCode agent; other names
    /// and the hook arguments themselves record nothing.
    #[test]
    fn cli_records_opencode_events_as_their_states() {
        let data = temp("cli");
        let hook = |native: &str, stdin: &str| run_cli(Agent::OpenCode, native, stdin, &data, Some("42"), Some("3"));
        let rec = || read_records(&data, 42).remove(&3);
        hook("session.status", r#"{"type":"session.status","properties":{"status":{"type":"busy"}}}"#);
        let r = rec().unwrap();
        assert_eq!((r.event.as_str(), r.agent()), ("prompt", Agent::OpenCode));
        hook("permission.asked", r#"{"type":"permission.asked","properties":{"type":"bash"}}"#);
        let r = rec().unwrap();
        assert_eq!((r.event.as_str(), r.message.as_deref()), ("permission-request", Some("OpenCode asks to use bash")));
        hook("permission.replied", r#"{"type":"permission.replied","properties":{}}"#);
        assert_eq!(rec().unwrap().event, "prompt", "the answer lets the agent carry on");
        hook("session.status", r#"{"type":"session.status","properties":{"status":{"type":"idle"}}}"#);
        assert_eq!(rec().unwrap().event, "stop");
        hook("session.status", r#"{"type":"session.status","properties":{"status":{"type":"retry"}}}"#);
        assert_eq!(rec().unwrap().event, "stop", "a retry keeps the state");
        hook("session.error", r#"{"type":"session.error","properties":{"error":{"data":{"message":"quota"}}}}"#);
        let r = rec().unwrap();
        assert_eq!(
            (r.event.as_str(), r.message.as_deref()),
            ("stop-failure", Some("OpenCode stopped with an error: quota"))
        );
        hook("message.part.updated", r#"{"type":"message.part.updated","properties":{}}"#);
        hook("stop", "{}");
        hook("session.unknown", "{}");
        let r = rec().unwrap();
        assert_eq!(r.event, "stop-failure", "unforwarded names change nothing");
        let _ = std::fs::remove_dir_all(&data);
    }
}
