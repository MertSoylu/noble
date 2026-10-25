//! Agent hooks. An agent's hooks run `noble hook <event> [--agent <id>]`, which writes the pane's state to a small
//! file in the data folder; an open NOBLE reads those files and shows the session's real state (working, waiting
//! for you, finished, failed). Each agent has a module (`claude`) that knows where its hooks are configured and how
//! they are installed; this module holds what they share: the record files, the `noble hook` command and its shape.
//! Hooks are added to an agent's settings only when the user turns them on in Settings; the settings file is backed
//! up the first time.

pub mod claude;
pub mod codex;
pub mod copilot;
pub mod opencode;

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::term::layout::PaneId;

/// The agents NOBLE reads hook records from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Claude,
    Codex,
    Copilot,
    OpenCode,
}

impl Agent {
    /// Every agent, in the order Settings lists them.
    pub const ALL: [Agent; 4] = [Agent::Claude, Agent::Codex, Agent::Copilot, Agent::OpenCode];

    /// The id of `noble hook --agent <id>`, the record's `agent` field and the pane's agent name (the same
    /// names `ai::agent_kind` gives the agent's command).
    pub fn id(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
            Agent::Copilot => "copilot",
            Agent::OpenCode => "opencode",
        }
    }

    /// The agent's name as its product writes it.
    pub fn name(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
            Agent::Copilot => "GitHub Copilot",
            Agent::OpenCode => "OpenCode",
        }
    }

    /// The agent with this id; `None` for an unknown one.
    pub fn from_id(id: &str) -> Option<Agent> {
        Agent::ALL.into_iter().find(|a| a.id() == id)
    }
}

/// Where each agent keeps its hooks and how they are written. The agent modules do the work.
/// The environment variables that move an agent's settings folder. `from_process` reads them for the running
/// app; `noble doctor` gets them through its `Scope`, so its tests never read the real environment.
#[derive(Clone, Debug, Default)]
pub struct HookEnv {
    pub codex_home: Option<OsString>,
    pub copilot_home: Option<OsString>,
    pub xdg_config_home: Option<OsString>,
}

impl HookEnv {
    pub fn from_process() -> HookEnv {
        HookEnv {
            codex_home: std::env::var_os("CODEX_HOME"),
            copilot_home: std::env::var_os("COPILOT_HOME"),
            xdg_config_home: std::env::var_os("XDG_CONFIG_HOME"),
        }
    }
}

impl Agent {
    /// The agent's hook settings file (OpenCode's is its plugin file); `None` when `ai::isolated` is set or there
    /// is no home folder.
    pub fn settings_path(self) -> Option<PathBuf> {
        match self {
            Agent::Claude => claude::settings_path(),
            Agent::Codex => codex::settings_path(),
            Agent::Copilot => copilot::settings_path(),
            Agent::OpenCode => opencode::settings_path(),
        }
    }

    /// The settings file for a given home folder and environment (the isolation check is left to the caller).
    pub fn settings_path_in(self, env: &HookEnv, home: Option<PathBuf>) -> Option<PathBuf> {
        match self {
            Agent::Claude => claude::settings_in(home),
            Agent::Codex => codex::settings_file(false, env.codex_home.as_deref(), home),
            Agent::Copilot => copilot::settings_file(false, env.copilot_home.as_deref(), home),
            Agent::OpenCode => opencode::settings_file(false, env.xdg_config_home.as_deref(), home),
        }
    }

    /// Whether the agent's command is on PATH, which is how NOBLE tells the agent is installed here.
    pub fn on_path(self) -> bool {
        crate::util::which(self.id()).is_some()
    }

    /// Whether every hook is installed in `path`; a file that cannot be read or parsed is an error.
    pub fn installed_state(self, path: &Path) -> Result<bool, String> {
        match self {
            Agent::Claude => claude::installed_state(path),
            Agent::Codex => codex::installed_state(path),
            Agent::Copilot => copilot::installed_state(path),
            Agent::OpenCode => opencode::installed_state(path),
        }
    }

    /// Adds the missing hooks to `path`; each runs `<base> hook <arg>` (see [`command_base`]).
    pub fn install(self, path: &Path, base: &str) -> Result<(), String> {
        match self {
            Agent::Claude => claude::install(path, base),
            Agent::Codex => codex::install(path, base),
            Agent::Copilot => copilot::install(path, base),
            Agent::OpenCode => opencode::install(path, base),
        }
    }

    /// Removes NOBLE's hooks from `path` and nothing else.
    pub fn uninstall(self, path: &Path) -> Result<(), String> {
        match self {
            Agent::Claude => claude::uninstall(path),
            Agent::Codex => codex::uninstall(path),
            Agent::Copilot => copilot::uninstall(path),
            Agent::OpenCode => opencode::uninstall(path),
        }
    }

    /// Gives the hooks of an older NOBLE the events added since (with the same command); does nothing when no
    /// NOBLE hook is installed or all of them already are.
    pub fn upgrade(self, path: &Path) -> Result<(), String> {
        match self {
            Agent::Claude => claude::upgrade(path),
            Agent::Codex => codex::upgrade(path),
            Agent::Copilot => copilot::upgrade(path),
            Agent::OpenCode => opencode::upgrade(path),
        }
    }
}

/// Startup: every agent whose hooks are installed gets the events added since (see [`Agent::upgrade`]).
pub fn upgrade_installed() {
    for agent in Agent::ALL {
        if let Some(path) = agent.settings_path() {
            let _ = agent.upgrade(&path);
        }
    }
}

// The `noble hook` arguments. Every agent maps its own event names to these (see each agent module's table).

pub const PROMPT: &str = "prompt";
pub const STOP: &str = "stop";
/// The agent's turn ended with an error (Claude's `StopFailure`): the session is not working any more.
pub const STOP_FAILURE: &str = "stop-failure";
pub const NOTIFICATION: &str = "notification";
/// The agent asks for permission to run a tool, before its dialog is shown.
pub const PERMISSION_REQUEST: &str = "permission-request";
pub const SESSION_START: &str = "session-start";
/// The session is over (the agent exited, or `/clear` right before a new `session-start`).
pub const SESSION_END: &str = "session-end";
/// A subagent started / finished. Claude's `Stop` fires when the main answer ends, even while background
/// subagents keep working, so NOBLE counts the running ones: each has a marker file next to the pane's record
/// (`<instance>-<pane>.<agent_id>.sub`).
pub const SUBAGENT_START: &str = "subagent-start";
pub const SUBAGENT_STOP: &str = "subagent-stop";

/// Every argument `noble hook` takes; a hook command with any other one is not NOBLE's.
pub const HOOK_ARGS: [&str; 9] = [
    PROMPT,
    STOP,
    STOP_FAILURE,
    NOTIFICATION,
    PERMISSION_REQUEST,
    SESSION_START,
    SESSION_END,
    SUBAGENT_START,
    SUBAGENT_STOP,
];

/// A subagent marker older than this no longer counts, in case the agent never reported the end (a crash, a
/// killed process), so a session cannot stay "working" forever.
const SUBAGENT_MAX_AGE_SECS: u64 = 3 * 3600;

/// The shared marker used to recognize our hook commands.
const MARKER: &str = " hook ";

/// The last event for a pane.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HookRecord {
    /// The `noble hook` argument (one of the constants above).
    pub event: String,
    #[serde(default)]
    pub message: Option<String>,
    pub ts: i64,
    /// The agent that wrote the record. A record without one (written before other agents had hooks) is Claude's.
    #[serde(default)]
    pub agent: Option<Agent>,
    /// Subagents of the session still running (counted from the marker files, not stored).
    #[serde(skip)]
    pub subagents: usize,
}

impl HookRecord {
    /// The agent this record belongs to.
    pub fn agent(&self) -> Agent {
        self.agent.unwrap_or(Agent::Claude)
    }
}

/// The folder of the record files (`<instance>-<pane>.json` and the subagent markers).
pub fn agents_dir(data: &Path) -> PathBuf {
    data.join("agents")
}

/// Base of the hook command: `noble` from PATH if present, else the running file's full path, quoted for the
/// shell that runs hooks (see [`quote_for_shell`]).
pub fn command_base() -> String {
    if crate::util::which("noble").is_some() {
        return "noble".into();
    }
    match std::env::current_exe() {
        Ok(p) => quote_for_shell(&p.display().to_string()),
        Err(_) => "noble".into(),
    }
}

/// `text` as one word for the shell an agent runs hook commands with. Unix (`sh`): single quotes, so
/// nothing in the path expands (`$`, backticks, `\`); a `'` is closed, escaped and reopened. Windows: double
/// quotes, the form cmd.exe and Git Bash read (a path cannot hold a `"` there).
pub fn quote_for_shell(text: &str) -> String {
    if cfg!(windows) {
        // Windows: cmd.exe and Git Bash read double quotes (a path cannot hold a `"` there).
        format!("\"{text}\"")
    } else {
        // Unix: single quotes, where nothing expands. Windows uses double quotes (see above).
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

/// Is `command` NOBLE's own hook command (see [`noble_base`])?
pub(crate) fn is_noble_command(command: &str) -> bool {
    noble_base(command).is_some()
}

/// The base of NOBLE's hook command `<base> hook <arg> [--agent <id>]` (the `--agent` part is left out for
/// Claude): `<arg>` is one of [`HOOK_ARGS`] and `<base>` is `noble`, `noble-dev` or a (possibly quoted) path to a
/// `noble` executable. `None` for any other command.
pub(crate) fn noble_base(command: &str) -> Option<String> {
    // The last " hook " is ours: the executable's path may contain one too (`/opt/my hook tools/noble`).
    let (base, rest) = command.rsplit_once(MARKER)?;
    let mut words = rest.split_whitespace();
    if !HOOK_ARGS.contains(&words.next()?) {
        return None;
    }
    let tail: Vec<&str> = words.collect();
    let tail_ok = match tail.as_slice() {
        [] => true,
        ["--agent", id] => Agent::from_id(id).is_some(),
        _ => false,
    };
    if !tail_ok {
        return None;
    }
    let base = base.trim();
    let name = base.trim_matches(['"', '\'']);
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name).to_lowercase();
    let stem = file.strip_suffix(".exe").unwrap_or(&file);
    (stem == "noble" || stem == "noble-dev").then(|| base.to_string())
}

/// Is this one hook (`{"type": "command", "command": …}`) NOBLE's?
fn is_noble_command_hook(hook: &Value) -> bool {
    hook.get("command").and_then(Value::as_str).is_some_and(is_noble_command)
}

/// Does this matcher group hold one of NOBLE's hooks?
fn is_noble_hook(entry: &Value) -> bool {
    entry.get("hooks").and_then(Value::as_array).is_some_and(|hooks| hooks.iter().any(is_noble_command_hook))
}

/// A file of matcher groups (`{"hooks": {"<Event>": [{"hooks": [{"type": "command", "command": …}]}]}}`), as
/// Claude's `settings.json` and Codex's `hooks.json` are. `events` pairs each native event name with its hook
/// argument. Is every event installed with one of NOBLE's hooks?
pub(crate) fn groups_installed(path: &Path, events: &[(&str, &str)]) -> Result<bool, String> {
    let v = read_settings(path)?;
    Ok(events.iter().all(|(event, _)| {
        v.pointer(&format!("/hooks/{event}")).and_then(Value::as_array).is_some_and(|l| l.iter().any(is_noble_hook))
    }))
}

/// Adds NOBLE's hook for every event that lacks one; `hook` builds the hook object from the native event and its
/// argument. Other settings, the user's own hooks and groups that hold them are kept.
pub(crate) fn groups_install(
    path: &Path,
    events: &[(&str, &str)],
    hook: impl Fn(&str, &str) -> Value,
) -> Result<(), String> {
    let mut v = read_settings(path)?;
    let root = v.as_object_mut().ok_or("the file is not a JSON object")?;
    let hooks = root.entry("hooks").or_insert_with(|| json!({}));
    let hooks = hooks.as_object_mut().ok_or("\"hooks\" is not an object")?;
    for &(event, arg) in events {
        let list = hooks.entry(event).or_insert_with(|| json!([]));
        let list = list.as_array_mut().ok_or(format!("hooks.{event} is not a list"))?;
        if !list.iter().any(is_noble_hook) {
            list.push(json!({ "hooks": [ hook(event, arg) ] }));
        }
    }
    write_settings(path, &v)
}

/// Removes NOBLE's hooks from a file of matcher groups and writes it back when it has a `hooks` object. A group
/// that also holds the user's own hook keeps it: only NOBLE's entry leaves it, and the group goes when nothing
/// is left in it. Lists emptied this way are removed, and so is `hooks` when nothing is left in it.
pub(crate) fn groups_uninstall(path: &Path, events: &[(&str, &str)]) -> Result<(), String> {
    let mut v = read_settings(path)?;
    let Some(hooks) = v.get_mut("hooks").and_then(Value::as_object_mut) else { return Ok(()) };
    for (event, _) in events {
        if let Some(list) = hooks.get_mut(*event).and_then(Value::as_array_mut) {
            let before = list.len();
            list.retain_mut(|group| {
                if !is_noble_hook(group) {
                    return true;
                }
                match group.get_mut("hooks").and_then(Value::as_array_mut) {
                    Some(inner) => {
                        inner.retain(|h| !is_noble_command_hook(h));
                        !inner.is_empty()
                    }
                    None => true,
                }
            });
            if list.is_empty() && list.len() != before {
                hooks.remove(*event);
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

/// The base of the first NOBLE hook in a file of matcher groups (`None` when there is none): an upgrade reuses it.
pub(crate) fn groups_base(path: &Path, events: &[(&str, &str)]) -> Result<Option<String>, String> {
    let v = read_settings(path)?;
    Ok(events.iter().find_map(|(event, _)| {
        let list = v.pointer(&format!("/hooks/{event}"))?.as_array()?;
        let entry = list.iter().find(|e| is_noble_hook(e))?;
        entry.pointer("/hooks")?.as_array()?.iter().find_map(|h| noble_base(h.get("command")?.as_str()?))
    }))
}

/// The message of an agent whose payload has no special fields: its `message`, cut to 120 characters.
pub(crate) fn plain_message(input: &Value) -> Option<Option<String>> {
    Some(field(input, "message").filter(|m| !m.is_empty()).map(|m| crate::util::truncate(m, 120)))
}

/// Reads a JSON settings file of an agent; a missing or empty file is an empty object.
fn read_settings(path: &Path) -> Result<Value, String> {
    match std::fs::read_to_string(path) {
        Ok(text) if crate::util::strip_bom(&text).trim().is_empty() => Ok(json!({})),
        Ok(text) => serde_json::from_str(crate::util::strip_bom(&text))
            .map_err(|e| format!("cannot parse {}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

/// Writes a JSON settings file, keeping a backup (`<name>.json.noble-bak`) of the file as it was first.
fn write_settings(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // The backup keeps the file as it was before NOBLE first touched it; later writes must not
    // overwrite it with an already edited copy. A backup that is damaged (cut short by an older build's
    // plain copy) is taken again: the file is still the original while nothing was written yet.
    // A file that did not exist yet gets an empty backup, so the next write does not copy NOBLE's own edits.
    let backup = path.with_extension("json.noble-bak");
    if !backup_is_intact(&backup) {
        let original =
            if path.is_file() { std::fs::read(path).map_err(|e| format!("backup failed: {e}"))? } else { Vec::new() };
        crate::store::write_bytes(&backup, original).map_err(|e| format!("backup failed: {e}"))?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    // Atomic, through a symlinked file, keeping its permissions; no temp file is left on failure.
    crate::store::write_text(path, &(text + "\n")).map_err(|e| e.to_string())
}

/// Is the backup there and usable: it exists and is empty (an empty settings file was backed up) or holds
/// JSON. A missing backup, or one cut short by a plain copy that was interrupted, is not.
fn backup_is_intact(backup: &Path) -> bool {
    std::fs::read_to_string(backup).is_ok_and(|text| {
        let text = crate::util::strip_bom(&text);
        text.trim().is_empty() || serde_json::from_str::<Value>(text).is_ok()
    })
}

/// The `noble hook` arguments of `noble hook <event> [--agent <id>]` (`args` are the words after the event):
/// Claude without a `--agent` (hooks installed before other agents existed have none); `None` for an unknown
/// id, which records nothing.
pub fn agent_arg(args: &[String]) -> Option<Agent> {
    match args.iter().position(|a| a == "--agent") {
        None => Some(Agent::Claude),
        Some(i) => args.get(i + 1).and_then(|id| Agent::from_id(id)),
    }
}

/// A string field of a hook's JSON payload, trimmed.
fn field<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str).map(str::trim)
}

/// `noble hook <event>`: takes the message from the JSON the agent passes on stdin and writes the pane's state
/// file. Does nothing when run outside NOBLE. Errors never surface to the agent (always silent).
pub fn run_cli(agent: Agent, event: &str, stdin: &str, data: &Path, instance: Option<&str>, pane: Option<&str>) {
    let (Some(instance), Some(pane)) = (instance, pane) else { return };
    if !instance.bytes().all(|b| b.is_ascii_digit()) || !pane.bytes().all(|b| b.is_ascii_digit()) {
        return;
    }
    let dir = agents_dir(data);
    let file = dir.join(format!("{instance}-{pane}.json"));
    let input = serde_json::from_str::<Value>(stdin).unwrap_or(Value::Null);
    // OpenCode's plugin forwards its own event names with the event as JSON; they become hook arguments here, and
    // a name with no state (or one that is not forwarded) records nothing.
    let event = match agent {
        Agent::OpenCode => match opencode::hook_arg(event, &input) {
            Some(arg) => arg,
            None => return,
        },
        _ => event,
    };
    // The session is over (the agent exited, or `/clear` right before a new `session-start`):
    // the pane no longer runs the agent, so its record and subagent markers go.
    if event == SESSION_END {
        remove_pane_files(&dir, instance, pane);
        return;
    }
    if event == SUBAGENT_START || event == SUBAGENT_STOP {
        // Only file-name-safe characters of the id (Copilot's payloads are camelCase); without an id the
        // subagent is not tracked.
        let id: String = field(&input, "agent_id")
            .or_else(|| field(&input, "agentId"))
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
            let _ = crate::util::create_private_dir(&dir);
            let _ = crate::store::create_new_file(&marker, "");
        } else {
            let _ = std::fs::remove_file(marker);
        }
        return;
    }
    // Reminders that ask nothing of the user leave the state as it is (`None`).
    let message = match agent {
        Agent::Claude => claude::record_message(event, &input),
        Agent::Codex => codex::record_message(event, &input),
        Agent::Copilot => copilot::record_message(event, &input),
        Agent::OpenCode => opencode::record_message(event, &input),
    };
    let Some(message) = message else { return };
    let rec = HookRecord {
        event: event.to_string(),
        message,
        ts: chrono::Utc::now().timestamp(),
        agent: Some(agent),
        subagents: 0,
    };
    let _ = crate::util::create_private_dir(&dir);
    if let Ok(text) = serde_json::to_string(&rec) {
        let tmp = dir.join(format!("{instance}-{pane}.tmp"));
        // A stale temp file (a crash) is replaced: the record is written as a new file each time.
        let _ = std::fs::remove_file(&tmp);
        // Windows: the reader may have the record open, so the rename is retried (`replace_file`).
        if crate::store::create_new_file(&tmp, &text).and_then(|()| crate::store::replace_file(&tmp, &file)).is_err() {
            let _ = std::fs::remove_file(&tmp);
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

/// Records belonging to this NOBLE instance (pane → last event and running subagents). A record of an agent this
/// build does not know is skipped.
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
/// fresh pane as running an agent. Records of other running instances stay.
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

    /// Only NOBLE's own command shape counts: another tool that merely mentions "noble" and " hook " does not.
    #[test]
    fn noble_command_shape_is_strict() {
        for ok in [
            "noble hook stop",
            "noble-dev hook prompt",
            "noble hook permission-request",
            "noble hook stop-failure",
            "noble hook stop --agent codex",
            "noble hook subagent-stop --agent copilot",
            "\"C:/Users/me/.cargo/bin/noble.exe\" hook session-end",
            "\"/opt/my hook tools/noble\" hook notification",
            "& \"C:/Users/me/.cargo/bin/noble.exe\" hook session-end --agent copilot",
        ] {
            assert!(is_noble_command(ok), "{ok}");
        }
        for bad in [
            "my-noble-notifier hook stop",
            "noblex hook stop",
            "noble hook unknown-event",
            "noble hook stop --agent cursor",
            "noble hook stop --agent",
            "noble hook stop --verbose",
            "noble hook stop --agent codex extra",
            "echo noble hook stop; rm -rf ~",
            "python noble.py hook stop",
            "noble stop",
        ] {
            assert!(!is_noble_command(bad), "{bad}");
        }
    }

    /// A failed write leaves no temporary file next to the settings (a directory in the way
    /// makes the rename fail on every platform).
    #[test]
    fn write_settings_cleans_up_its_temp_file() {
        let dir = temp("tmp");
        let blocked = dir.join("settings.json");
        std::fs::create_dir_all(blocked.join("inside")).unwrap();
        assert!(write_settings(&blocked, &serde_json::json!({})).is_err());
        assert!(blocked.is_dir(), "the directory in the way is untouched");
        // The (empty) backup written before the failed rename is not a temp file.
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .filter(|n| !n.to_string_lossy().ends_with(".noble-bak"))
            .collect();
        assert_eq!(left, vec![std::ffi::OsString::from("settings.json")], "temp file left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn agent_arg_reads_the_agent_flag() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(agent_arg(&args(&[])), Some(Agent::Claude), "hooks without a flag are Claude's");
        assert_eq!(agent_arg(&args(&["--agent", "codex"])), Some(Agent::Codex));
        assert_eq!(agent_arg(&args(&["--agent", "opencode"])), Some(Agent::OpenCode));
        assert_eq!(agent_arg(&args(&["--agent"])), None, "a flag without an id");
        assert_eq!(agent_arg(&args(&["--agent", "cursor"])), None, "an unknown agent records nothing");
        for agent in Agent::ALL {
            assert_eq!(Agent::from_id(agent.id()), Some(agent));
        }
    }

    #[test]
    fn records_name_their_agent() {
        let data = temp("agent");
        run_cli(Agent::Codex, PROMPT, "{}", &data, Some("42"), Some("3"));
        run_cli(Agent::Claude, STOP, "{}", &data, Some("42"), Some("4"));
        let recs = read_records(&data, 42);
        assert_eq!(recs[&3].agent(), Agent::Codex);
        assert_eq!(recs[&4].agent(), Agent::Claude);
        // A record written before other agents existed has no agent: it is Claude's.
        std::fs::write(agents_dir(&data).join("42-5.json"), r#"{"event":"prompt","ts":1}"#).unwrap();
        assert_eq!(read_records(&data, 42)[&5].agent(), Agent::Claude);
        // A record of an agent this build does not know is not read.
        std::fs::write(agents_dir(&data).join("42-6.json"), r#"{"event":"prompt","ts":1,"agent":"cursor"}"#).unwrap();
        assert!(!read_records(&data, 42).contains_key(&6));
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn cli_writes_records_only_inside_noble() {
        let data = temp("cli");
        run_cli(Agent::Claude, "stop", "{}", &data, None, Some("3"));
        assert!(read_records(&data, 42).is_empty());
        run_cli(
            Agent::Claude,
            "notification",
            r#"{"message":"Claude needs your permission to use Bash"}"#,
            &data,
            Some("42"),
            Some("3"),
        );
        run_cli(Agent::Claude, "prompt", "", &data, Some("7"), Some("1"));
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
        run_cli(Agent::Claude, "prompt", "{}", &data, Some("42"), Some("3"));
        run_cli(Agent::Claude, "prompt", "{}", &data, Some("42"), Some("4"));
        run_cli(Agent::Claude, SESSION_END, "{}", &data, Some("42"), Some("3"));
        let recs = read_records(&data, 42);
        assert!(!recs.contains_key(&3), "{recs:?}");
        assert_eq!(recs[&4].event, "prompt", "other panes keep their record");
        // Ending a session that has no record is harmless.
        run_cli(Agent::Claude, SESSION_END, "{}", &data, Some("42"), Some("9"));
        run_cli(Agent::Claude, "session-start", "{}", &data, Some("42"), Some("3"));
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
        let hook = |event: &str, stdin: &str| run_cli(Agent::Claude, event, stdin, &data, Some("42"), Some("3"));
        let count = || read_records(&data, 42).get(&3).map(|r| (r.event.clone(), r.subagents));
        hook("prompt", "{}");
        hook(SUBAGENT_START, r#"{"agent_id":"agent-001","agent_type":"Explore"}"#);
        hook(SUBAGENT_START, r#"{"agent_id":"agent-002","agent_type":"general-purpose"}"#);
        hook(SUBAGENT_START, r#"{"agent_id":"agent-001"}"#); // reported twice: still one
        hook(SUBAGENT_START, "{}"); // no id: not tracked
        hook(SUBAGENT_START, r#"{"agent_id":"../.."}"#); // nothing file-name-safe left
        run_cli(Agent::Claude, SUBAGENT_START, r#"{"agent_id":"agent-009"}"#, &data, Some("42"), Some("4")); // another pane
        hook("stop", "{}");
        assert_eq!(count(), Some(("stop".into(), 2)), "the main answer ended, two subagents run");
        hook(SUBAGENT_STOP, r#"{"agent_id":"agent-001","last_assistant_message":"done"}"#);
        assert_eq!(count(), Some(("stop".into(), 1)));
        // A marker left behind hours ago (the agent killed) no longer counts.
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

    /// Startup cleanup: records a day old (crashed instances) and leftovers of an earlier
    /// process with this instance's id go; another running instance's records stay.
    #[test]
    fn prune_removes_stale_and_reused_instance_records() {
        let data = temp("prune");
        run_cli(Agent::Claude, "prompt", "{}", &data, Some("42"), Some("1")); // an earlier process with our id
        run_cli(Agent::Claude, "stop", "{}", &data, Some("420"), Some("1")); // another instance (shares the prefix digits)
        run_cli(Agent::Claude, "notification", "{}", &data, Some("7"), Some("2")); // crashed long ago
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
