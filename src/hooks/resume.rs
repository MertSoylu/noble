//! Resuming agent conversations: the session id an agent's hook payload carries, and the command line that picks
//! that conversation up again when its pane is restored. The id ends up on a shell command line, so it is checked
//! (letters, digits, `-` and `_`, bounded length) before anything else sees it, including when it is read back
//! from a session file.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Agent;

/// The longest session id NOBLE keeps (real ids are UUIDs or short `ses_…` names).
pub const MAX_ID_LEN: usize = 128;

/// A conversation an agent pane can pick up again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resume {
    pub agent: Agent,
    pub id: String,
}

/// The id when it is safe on a command line: non-empty, at most [`MAX_ID_LEN`] characters, and only ASCII letters,
/// digits, `-` and `_`.
pub fn valid_id(raw: &str) -> Option<String> {
    let safe = !raw.is_empty()
        && raw.len() <= MAX_ID_LEN
        && raw.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    safe.then(|| raw.to_string())
}

/// The session id in an agent's hook payload, `None` when it has none or it is not a safe id.
pub fn session_id(agent: Agent, input: &Value) -> Option<String> {
    let raw = match agent {
        // Claude Code and Codex send `session_id` on every event.
        Agent::Claude | Agent::Codex => input.get("session_id").and_then(Value::as_str),
        // Copilot names it `sessionId` on camelCase events and `session_id` on PascalCase ones.
        Agent::Copilot => input.get("sessionId").or_else(|| input.get("session_id")).and_then(Value::as_str),
        // OpenCode's plugin forwards its own events, which carry the session in `properties`.
        Agent::OpenCode => input
            .pointer("/properties/sessionID")
            .or_else(|| input.pointer("/properties/info/id"))
            .and_then(Value::as_str),
    }?;
    valid_id(raw.trim())
}

/// The words of a launcher that already pick a conversation (such a launcher is left as it is).
fn taken_flags(agent: Agent) -> &'static [&'static str] {
    match agent {
        Agent::Claude => &["--resume", "-r", "--continue", "-c", "--session-id"],
        Agent::Codex => &["resume"],
        Agent::Copilot => &["--resume", "--continue"],
        Agent::OpenCode => &["--session", "-s", "--continue", "-c", "--fork"],
    }
}

/// Whether the launcher line already picks a conversation for `agent`. A flag also matches in its `--flag=value` form.
fn picks_conversation(launch: &str, agent: Agent) -> bool {
    launch.split_whitespace().any(|word| {
        let flag = word.split('=').next().unwrap_or(word);
        taken_flags(agent).contains(&flag)
    })
}

impl Resume {
    /// The arguments that resume this conversation with the agent's CLI; `None` when the id is not safe.
    pub fn args(&self) -> Option<String> {
        let id = valid_id(&self.id)?;
        Some(match self.agent {
            Agent::Claude => format!("--resume {id}"),
            Agent::Codex => format!("resume {id}"),
            // Copilot's documented form takes the id after `=`.
            Agent::Copilot => format!("--resume={id}"),
            Agent::OpenCode => format!("--session {id}"),
        })
    }
}

/// The command line that resumes `resume` from a launcher command (e.g. `claude --model opus` becomes
/// `claude --model opus --resume <id>`). `None` when the launcher does not run this agent, the launcher already
/// picks a conversation, or the id is not safe. The caller runs the result through the shell as usual.
pub fn resume_command(launch: &str, resume: &Resume) -> Option<String> {
    let launch = launch.trim();
    if launch.is_empty() || crate::ai::agent_kind(Some(launch), "") != Some(resume.agent.id()) {
        return None;
    }
    let args = resume.args()?;
    (!picks_conversation(launch, resume.agent)).then(|| format!("{launch} {args}"))
}

/// The command line that continues the most recent Claude conversation of the folder (`claude --continue`) from a
/// launcher command. `None` when the launcher does not run Claude or already picks a conversation. The caller runs
/// the result through the shell as usual.
pub fn continue_command(launch: &str) -> Option<String> {
    let launch = launch.trim();
    if launch.is_empty() || crate::ai::agent_kind(Some(launch), "") != Some(Agent::Claude.id()) {
        return None;
    }
    (!picks_conversation(launch, Agent::Claude)).then(|| format!("{launch} --continue"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn res(agent: Agent, id: &str) -> Resume {
        Resume { agent, id: id.into() }
    }

    #[test]
    fn ids_must_be_safe_on_a_command_line() {
        assert_eq!(
            valid_id("06aab473-8596-46e1-90b3-2092043a6fd4").as_deref(),
            Some("06aab473-8596-46e1-90b3-2092043a6fd4")
        );
        assert_eq!(valid_id("ses_2f1A-b_9").as_deref(), Some("ses_2f1A-b_9"));
        assert_eq!(valid_id(""), None);
        assert_eq!(valid_id("a b"), None, "a space would split the argument");
        assert_eq!(valid_id("x;rm -rf ~"), None);
        assert_eq!(valid_id("a&b"), None);
        assert_eq!(valid_id("a\"b"), None);
        assert_eq!(valid_id("a%PATH%"), None, "cmd.exe would expand it");
        assert_eq!(valid_id("é"), None, "ASCII only");
        assert_eq!(valid_id(&"a".repeat(MAX_ID_LEN)).map(|s| s.len()), Some(MAX_ID_LEN));
        assert_eq!(valid_id(&"a".repeat(MAX_ID_LEN + 1)), None, "bounded length");
    }

    #[test]
    fn session_id_is_read_per_agent() {
        let id = "06aab473-8596-46e1-90b3-2092043a6fd4";
        assert_eq!(session_id(Agent::Claude, &json!({ "session_id": id, "cwd": "C:\\x" })).as_deref(), Some(id));
        assert_eq!(session_id(Agent::Codex, &json!({ "session_id": id })).as_deref(), Some(id));
        assert_eq!(session_id(Agent::Copilot, &json!({ "sessionId": id })).as_deref(), Some(id));
        assert_eq!(session_id(Agent::Copilot, &json!({ "session_id": id })).as_deref(), Some(id));
        assert_eq!(
            session_id(Agent::OpenCode, &json!({ "type": "session.idle", "properties": { "sessionID": "ses_1" } }))
                .as_deref(),
            Some("ses_1")
        );
        assert_eq!(
            session_id(Agent::OpenCode, &json!({ "properties": { "info": { "id": "ses_2" } } })).as_deref(),
            Some("ses_2")
        );
    }

    #[test]
    fn missing_or_unsafe_session_ids_are_none() {
        assert_eq!(session_id(Agent::Claude, &json!({})), None);
        assert_eq!(session_id(Agent::Claude, &json!({ "session_id": 5 })), None, "not a string");
        assert_eq!(session_id(Agent::Claude, &json!({ "session_id": "a b; calc" })), None);
        assert_eq!(session_id(Agent::Copilot, &json!({ "session_id": "  " })), None);
        // Claude does not read another agent's field names.
        assert_eq!(session_id(Agent::Claude, &json!({ "sessionId": "abc" })), None);
        assert_eq!(session_id(Agent::OpenCode, &json!({ "session_id": "abc" })), None);
    }

    #[test]
    fn resume_arguments_per_agent() {
        assert_eq!(res(Agent::Claude, "abc").args().as_deref(), Some("--resume abc"));
        assert_eq!(res(Agent::Codex, "abc").args().as_deref(), Some("resume abc"));
        assert_eq!(res(Agent::Copilot, "abc").args().as_deref(), Some("--resume=abc"));
        assert_eq!(res(Agent::OpenCode, "abc").args().as_deref(), Some("--session abc"));
        assert_eq!(res(Agent::Claude, "a b").args(), None, "an unsafe id builds no command");
    }

    #[test]
    fn command_keeps_the_launchers_own_arguments() {
        assert_eq!(
            resume_command("claude --model opus", &res(Agent::Claude, "abc")).as_deref(),
            Some("claude --model opus --resume abc")
        );
        assert_eq!(resume_command("claude", &res(Agent::Claude, "abc")).as_deref(), Some("claude --resume abc"));
        assert_eq!(
            resume_command("  codex --sandbox read-only ", &res(Agent::Codex, "abc")).as_deref(),
            Some("codex --sandbox read-only resume abc")
        );
        assert_eq!(
            resume_command("copilot --allow-all", &res(Agent::Copilot, "abc")).as_deref(),
            Some("copilot --allow-all --resume=abc")
        );
        assert_eq!(
            resume_command("opencode", &res(Agent::OpenCode, "ses_1")).as_deref(),
            Some("opencode --session ses_1")
        );
        // A path to the program still names the agent.
        assert_eq!(
            resume_command("C:\\Tools\\claude.exe --verbose", &res(Agent::Claude, "abc")).as_deref(),
            Some("C:\\Tools\\claude.exe --verbose --resume abc")
        );
    }

    #[test]
    fn command_is_not_built_for_other_programs_or_taken_flags() {
        assert_eq!(resume_command("pwsh", &res(Agent::Claude, "abc")), None, "a shell launcher is not the agent");
        assert_eq!(resume_command("", &res(Agent::Claude, "abc")), None);
        assert_eq!(resume_command("claude --resume other", &res(Agent::Claude, "abc")), None);
        assert_eq!(resume_command("claude --resume=other", &res(Agent::Claude, "abc")), None);
        assert_eq!(resume_command("claude -c", &res(Agent::Claude, "abc")), None);
        assert_eq!(resume_command("codex resume --last", &res(Agent::Codex, "abc")), None);
        assert_eq!(resume_command("copilot --continue", &res(Agent::Copilot, "abc")), None);
        assert_eq!(resume_command("opencode --session x", &res(Agent::OpenCode, "abc")), None);
        // An unsafe id never reaches the command line.
        assert_eq!(resume_command("claude", &res(Agent::Claude, "x;y")), None);
    }

    /// cmd.exe takes the line as text: the resume arguments stay at its end, and only the program is quoted.
    #[test]
    fn cmd_invocation_keeps_the_resume_arguments() {
        let cmd = crate::term::pane::ShellSpec::new("cmd", Vec::new());
        let command = resume_command("cmd /c echo claude --model opus", &res(Agent::Claude, "abc-1")).unwrap();
        let line = cmd.invocation(&command);
        assert!(line.ends_with("/c echo claude --model opus --resume abc-1"), "{line}");
        if cfg!(windows) {
            assert!(line.starts_with('"'), "the program is quoted for cmd: {line}");
        }
    }

    #[test]
    fn resume_survives_a_round_trip_and_rejects_bad_ids_in_files() {
        let good = res(Agent::Claude, "abc-1");
        let text = serde_json::to_string(&good).unwrap();
        assert_eq!(serde_json::from_str::<Resume>(&text).unwrap(), good);
        // A hand-edited file with an unsafe id still deserializes, but builds no command.
        let bad: Resume = serde_json::from_str(r#"{"agent":"claude","id":"a\"; rm"}"#).unwrap();
        assert_eq!(resume_command("claude", &bad), None);
    }

    #[test]
    fn continue_command_adds_the_flag_to_a_claude_launcher() {
        assert_eq!(continue_command("claude").as_deref(), Some("claude --continue"));
        assert_eq!(continue_command(" claude --model opus ").as_deref(), Some("claude --model opus --continue"));
        assert_eq!(continue_command("C:\\Tools\\claude.exe").as_deref(), Some("C:\\Tools\\claude.exe --continue"));
        // Other programs, empty lines and launchers that already pick a conversation get nothing.
        assert_eq!(continue_command("codex"), None);
        assert_eq!(continue_command("pwsh"), None);
        assert_eq!(continue_command(""), None);
        assert_eq!(continue_command("claude -c"), None);
        assert_eq!(continue_command("claude --resume=abc"), None);
        assert_eq!(continue_command("claude --session-id abc"), None);
    }
}
