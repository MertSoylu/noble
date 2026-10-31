//! `noble doctor`: a plain-text report of this machine, the terminal NOBLE runs in and NOBLE's setup, meant to be
//! pasted into a bug report. It only reads: it writes no file, never changes `~/.claude/settings.json`, and never
//! prints a token, a key or the content of a credential file (an AI provider is reported as detected or not).
//! Every probe is bounded in time and never panics; a probe that cannot answer says "unknown".

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::ai::{self, Presence};
use crate::config::{self, Config, Paths};
use crate::hooks::Agent;
use crate::term::pane::{ShellSpec, resolve_shell};
use crate::util;

/// Longest wait for `gh auth status`; past it the login state is "unknown".
const GH_TIMEOUT: Duration = Duration::from_secs(5);

/// Shells looked up on PATH, in the order the report lists them. Git Bash is looked up next to `git` on Windows.
const SHELLS: [&str; 6] = ["pwsh", "powershell", "cmd", "bash", "zsh", "fish"];

/// The reason shown for every machine-wide probe that is not run.
const SKIPPED: &str = "skipped (NOBLE_NO_SYSTEM_INTEGRATIONS is set)";

/// What `gather` may look at. `run_cli` takes it from the real environment; tests point `home` at a fake folder.
pub struct Scope {
    /// The user's home folder, where Claude Code's settings and the AI logins are. `None` when there is none.
    pub home: Option<PathBuf>,
    /// Whether the machine-wide integrations are probed (hooks, AI logins, `gh`). Off under
    /// `NOBLE_NO_SYSTEM_INTEGRATIONS` (`ai::isolated`).
    pub system: bool,
    /// Whether `gh auth status` runs. It starts a process, so tests leave it off.
    pub gh_auth: bool,
    /// The variables that move the agents' settings folders (`HookEnv::from_process` in `run_cli`).
    pub env: crate::hooks::HookEnv,
}

/// The answers the report shows: `gather` fills them in, `render` turns them into text.
#[derive(Debug, Clone, PartialEq)]
pub struct Facts {
    pub version: String,
    pub dev_build: bool,
    pub os: String,
    pub arch: String,
    pub noble_home: bool,
    pub system: bool,
    pub terminal: Terminal,
    pub default_shell: Shell,
    pub shells: Vec<Shell>,
    /// Each agent's hook state, in `Agent::ALL` order.
    pub hooks: Vec<AgentHooks>,
    pub providers: Providers,
    pub config_path: PathBuf,
    pub config: ConfigState,
    pub data_dir: PathBuf,
    pub data_exists: bool,
    pub gh: Gh,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Terminal {
    pub term: Option<String>,
    pub term_program: Option<String>,
    pub colorterm: Option<String>,
    pub truecolor: bool,
    /// Whether `WT_SESSION` (Windows Terminal) is set; `None` on the other platforms.
    pub wt_session: Option<bool>,
    pub over_ssh: bool,
    pub desktop: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Shell {
    pub name: String,
    /// `None`: not found.
    pub program: Option<PathBuf>,
    /// Whether NOBLE's shell integration reaches it (`ShellSpec::integration_applies`).
    pub integration: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hooks {
    Skipped,
    NoHome,
    Unreadable,
    Installed,
    NotInstalled,
    /// Not on this machine: no settings file and the agent's command is not on PATH.
    NotFound,
}

/// One agent's hook state and the settings file it is read from (`None` when there is none to read).
#[derive(Debug, Clone, PartialEq)]
pub struct AgentHooks {
    pub agent: Agent,
    pub state: Hooks,
    pub settings: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Providers {
    Skipped,
    NoHome,
    /// Each provider's display name and what was found for it. Only the presence is kept, never a credential.
    Checked(Vec<(&'static str, Presence)>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigState {
    Missing,
    Parses,
    /// Does not parse; the line of the mistake when known. The message itself is not kept: it can quote a value.
    Broken(Option<u32>),
    Unreadable,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Gh {
    /// `None`: `gh` is not on PATH.
    pub path: Option<PathBuf>,
    pub auth: GhAuth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GhAuth {
    Skipped,
    NotChecked,
    LoggedIn,
    NotLoggedIn,
    Unknown,
}

/// Probes the machine and collects the report's answers. Reads only: a missing config file is not created and the
/// data folder is not made. Every probe is bounded in time.
pub fn gather(paths: &Paths, scope: &Scope) -> Facts {
    let (config, parsed) = read_config(&paths.config);
    let terminal_cfg = parsed.map(|c| c.terminal).unwrap_or_else(|| Config::default().terminal);
    let spec = resolve_shell(&terminal_cfg, &paths.data);
    let default_shell = Shell {
        name: spec.label(),
        integration: spec.integration_applies(),
        program: Some(PathBuf::from(&spec.program)),
    };
    let hooks = hooks(scope);
    Facts {
        version: env!("CARGO_PKG_VERSION").into(),
        dev_build: util::is_dev_build(),
        os: os_line(),
        arch: std::env::consts::ARCH.into(),
        noble_home: std::env::var_os("NOBLE_HOME").is_some_and(|v| !v.is_empty()),
        system: scope.system,
        terminal: terminal(),
        default_shell,
        shells: probe_shells(&paths.data),
        hooks,
        providers: providers(scope),
        config_path: paths.config.clone(),
        config,
        data_dir: paths.data.clone(),
        data_exists: paths.data.is_dir(),
        gh: gh(scope),
    }
}

/// The config's state, and the parsed config when it parses. A missing file is not an error: NOBLE uses the defaults.
fn read_config(path: &Path) -> (ConfigState, Option<Config>) {
    match std::fs::read_to_string(path) {
        Ok(text) => match config::parse(util::strip_bom(&text)) {
            Ok(cfg) => (ConfigState::Parses, Some(cfg)),
            Err(e) => (ConfigState::Broken(e.line), None),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (ConfigState::Missing, None),
        Err(_) => (ConfigState::Unreadable, None),
    }
}

fn os_line() -> String {
    let version = sysinfo::System::long_os_version().unwrap_or_else(|| "version unknown".into());
    format!("{} ({version})", std::env::consts::OS)
}

fn terminal() -> Terminal {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    Terminal {
        term: var("TERM"),
        term_program: var("TERM_PROGRAM"),
        colorterm: var("COLORTERM"),
        truecolor: !util::lacks_truecolor(),
        // Windows Terminal sets it in every tab; the other platforms have no such variable to report.
        wt_session: cfg!(windows).then(|| var("WT_SESSION").is_some()),
        over_ssh: util::over_ssh(),
        desktop: util::has_desktop(),
    }
}

fn probe_shells(data: &Path) -> Vec<Shell> {
    let mut found: Vec<(String, Option<PathBuf>)> =
        SHELLS.iter().map(|name| (name.to_string(), util::which(name))).collect();
    // Windows only: Git Bash is not on PATH by itself, so it is found through `git`. Other platforms have bash.
    if cfg!(windows) {
        found.push(("git bash".into(), git_bash()));
    }
    found
        .into_iter()
        .map(|(name, program)| Shell {
            name,
            integration: program.as_deref().is_some_and(|p| integration_of(p, data)),
            program,
        })
        .collect()
}

/// `bin\bash.exe` of the Git installation that owns `git` on PATH (`...\Git\cmd\git.exe`).
fn git_bash() -> Option<PathBuf> {
    let git = util::which("git")?;
    let bash = git.parent()?.parent()?.join("bin").join("bash.exe");
    bash.is_file().then_some(bash)
}

/// Whether NOBLE's shell integration would reach the shell at `program`, started with no arguments.
fn integration_of(program: &Path, data: &Path) -> bool {
    let spec = ShellSpec {
        integration: Some(data.join("shell")),
        ..ShellSpec::new(program.display().to_string(), Vec::new())
    };
    spec.integration_applies()
}

/// Every agent's hook state (`Agent::ALL` order).
fn hooks(scope: &Scope) -> Vec<AgentHooks> {
    Agent::ALL.into_iter().map(|agent| agent_hooks(scope, agent)).collect()
}

fn agent_hooks(scope: &Scope, agent: Agent) -> AgentHooks {
    let unknown = |state| AgentHooks { agent, state, settings: None };
    if !scope.system {
        return unknown(Hooks::Skipped);
    }
    let Some(home) = &scope.home else { return unknown(Hooks::NoHome) };
    // The agent's own settings path for this home and environment (CODEX_HOME and the like are honored).
    let Some(path) = agent.settings_path_in(&scope.env, Some(home.clone())) else { return unknown(Hooks::NoHome) };
    let state = match agent.installed_state(&path) {
        Ok(true) => Hooks::Installed,
        Ok(false) if !path.exists() && !agent.on_path() => Hooks::NotFound,
        Ok(false) => Hooks::NotInstalled,
        Err(_) => Hooks::Unreadable,
    };
    AgentHooks { agent, state, settings: Some(path) }
}

fn providers(scope: &Scope) -> Providers {
    if !scope.system {
        return Providers::Skipped;
    }
    let Some(home) = &scope.home else { return Providers::NoHome };
    let env = ai::Env::new(home.clone());
    Providers::Checked(ai::providers::registry().into_iter().map(|d| (d.name, (d.detect)(&env))).collect())
}

fn gh(scope: &Scope) -> Gh {
    let path = util::which("gh");
    let auth = match &path {
        _ if !scope.system => GhAuth::Skipped,
        Some(gh) if scope.gh_auth => gh_auth(gh),
        _ => GhAuth::NotChecked,
    };
    Gh { path, auth }
}

/// `gh auth status` exits 0 when signed in and non-zero when not; anything that cannot answer in time is "unknown".
/// Its output (which can show a token) is discarded, never read.
fn gh_auth(gh: &Path) -> GhAuth {
    let mut cmd = util::command_for(gh);
    cmd.args(["auth", "status"]).env("GH_PROMPT_DISABLED", "1");
    match ai::run_succeeds(cmd, GH_TIMEOUT) {
        Ok(true) => GhAuth::LoggedIn,
        Ok(false) => GhAuth::NotLoggedIn,
        Err(_) => GhAuth::Unknown,
    }
}

/// The report text. Plain ASCII lines, so it pastes cleanly into any issue or console.
pub fn render(f: &Facts) -> String {
    let mut out =
        String::from("noble doctor\nPaste the whole output into a bug report. It holds no tokens or credentials.\n");

    section(&mut out, "NOBLE");
    line(&mut out, "version", &f.version);
    line(&mut out, "build", if f.dev_build { "dev" } else { "release" });
    let integrations = if f.system { "checked" } else { SKIPPED };
    line(&mut out, "system integrations", integrations);
    line(&mut out, "NOBLE_HOME", if f.noble_home { "set" } else { "not set" });

    section(&mut out, "System");
    line(&mut out, "os", &f.os);
    line(&mut out, "arch", &f.arch);

    section(&mut out, "Terminal");
    let t = &f.terminal;
    line(&mut out, "TERM", &opt_text(&t.term));
    line(&mut out, "TERM_PROGRAM", &opt_text(&t.term_program));
    line(&mut out, "COLORTERM", &opt_text(&t.colorterm));
    line(&mut out, "truecolor", yes_no(t.truecolor));
    if let Some(wt) = t.wt_session {
        line(&mut out, "WT_SESSION", if wt { "set" } else { "not set" });
    }
    line(&mut out, "over SSH", yes_no(t.over_ssh));
    line(&mut out, "desktop", yes_no(t.desktop));

    section(&mut out, "Shells");
    line(&mut out, &format!("default ({})", f.default_shell.name), &shell_text(&f.default_shell));
    for shell in &f.shells {
        line(&mut out, &shell.name, &shell_text(shell));
    }

    section(&mut out, "Agent hooks");
    for h in &f.hooks {
        let status = match h.state {
            Hooks::Installed => "installed",
            Hooks::NotInstalled => "not installed",
            Hooks::NotFound => "not found",
            Hooks::Unreadable => "unknown (the settings file cannot be read or parsed)",
            Hooks::NoHome => "unknown (no home folder)",
            Hooks::Skipped => SKIPPED,
        };
        line(&mut out, h.agent.name(), status);
        if let Some(path) = &h.settings {
            line(&mut out, "  settings", &util::tilde(path));
        }
    }

    section(&mut out, "AI providers");
    match &f.providers {
        Providers::Skipped => line(&mut out, "status", SKIPPED),
        Providers::NoHome => line(&mut out, "status", "unknown (no home folder)"),
        Providers::Checked(list) => {
            for (name, presence) in list {
                line(&mut out, name, presence_text(*presence));
            }
        }
    }

    section(&mut out, "Paths");
    line(&mut out, "config", &util::tilde(&f.config_path));
    let parses = match f.config {
        ConfigState::Missing => "not created yet (the defaults are used)".to_string(),
        ConfigState::Parses => "yes".into(),
        ConfigState::Broken(Some(n)) => format!("no (line {n})"),
        ConfigState::Broken(None) => "no".into(),
        ConfigState::Unreadable => "unknown (the file cannot be read)".into(),
    };
    line(&mut out, "config.toml parses", &parses);
    let data_state = if f.data_exists { "exists" } else { "missing" };
    line(&mut out, "data", &format!("{} ({data_state})", util::tilde(&f.data_dir)));

    section(&mut out, "GitHub CLI");
    line(&mut out, "gh", &f.gh.path.as_deref().map_or("not found".into(), util::tilde));
    let auth = match f.gh.auth {
        GhAuth::Skipped => SKIPPED,
        GhAuth::NotChecked => "not checked",
        GhAuth::LoggedIn => "logged in",
        GhAuth::NotLoggedIn => "not logged in",
        GhAuth::Unknown => "unknown",
    };
    line(&mut out, "gh auth", auth);
    out
}

fn section(out: &mut String, title: &str) {
    out.push_str(&format!("\n[{title}]\n"));
}

fn line(out: &mut String, key: &str, value: &str) {
    out.push_str(&format!("{key}: {value}\n"));
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn opt_text(value: &Option<String>) -> String {
    value.clone().unwrap_or_else(|| "not set".into())
}

fn shell_text(shell: &Shell) -> String {
    match &shell.program {
        Some(path) => format!("{}, integration: {}", util::tilde(path), yes_no(shell.integration)),
        None => "not found".into(),
    }
}

fn presence_text(presence: Presence) -> &'static str {
    match presence {
        Presence::Ready => "detected (signed in)",
        Presence::NoLogin => "detected, not signed in",
        Presence::NotInstalled => "not found",
    }
}

/// `noble doctor [--config <path>]`: prints the report to stdout and returns the exit code. The terminal is never
/// opened, and `NOBLE_HOME` moves the config and data folders as it does for NOBLE itself.
pub fn run_cli(args: &[String]) -> i32 {
    let mut config_override = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!(
                    "USAGE: noble doctor [--config <path>]\n\n\
                     Prints a report on this machine, the terminal and NOBLE's setup, for bug reports.\n  \
                       --config <path>   check an alternate config file"
                );
                return 0;
            }
            "--config" => match args.next() {
                Some(path) => config_override = Some(PathBuf::from(path)),
                None => {
                    eprintln!("noble: --config needs a path");
                    return 2;
                }
            },
            other => {
                eprintln!("noble: unknown argument '{other}' (try noble doctor --help)");
                return 2;
            }
        }
    }
    let Some(paths) = Paths::resolve(config_override) else {
        eprintln!("noble: no home folder for NOBLE's files: set NOBLE_HOME to the folder to use");
        return 1;
    };
    let scope = Scope {
        home: dirs::home_dir(),
        system: !ai::isolated(),
        gh_auth: true,
        env: crate::hooks::HookEnv::from_process(),
    };
    let text = render(&gather(&paths, &scope));
    // A closed pipe (`noble doctor | head`) is not an error worth a panic.
    let _ = std::io::stdout().lock().write_all(text.as_bytes());
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder for one test, removed when the test ends (also on a panic).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let path = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn shell(name: &str, program: Option<&str>, integration: bool) -> Shell {
        Shell { name: name.into(), program: program.map(PathBuf::from), integration }
    }

    /// Every section filled in, as on a Windows machine with Windows Terminal.
    fn sample() -> Facts {
        let pwsh = "C:/Program Files/PowerShell/7/pwsh.exe";
        Facts {
            version: "1.10.0".into(),
            dev_build: false,
            os: "windows (Windows 11 Home 10.0.26300)".into(),
            arch: "x86_64".into(),
            noble_home: false,
            system: true,
            terminal: Terminal {
                term: Some("xterm-256color".into()),
                term_program: Some("WindowsTerminal".into()),
                colorterm: Some("truecolor".into()),
                truecolor: true,
                wt_session: Some(true),
                over_ssh: false,
                desktop: true,
            },
            default_shell: shell("pwsh", Some(pwsh), true),
            shells: vec![shell("pwsh", Some(pwsh), true), shell("bash", None, false)],
            hooks: vec![
                AgentHooks {
                    agent: Agent::Claude,
                    state: Hooks::Installed,
                    settings: Some(PathBuf::from("C:/elsewhere/.claude/settings.json")),
                },
                AgentHooks {
                    agent: Agent::Codex,
                    state: Hooks::NotInstalled,
                    settings: Some(PathBuf::from("C:/elsewhere/.codex/hooks.json")),
                },
                AgentHooks { agent: Agent::Copilot, state: Hooks::NotFound, settings: None },
                AgentHooks { agent: Agent::OpenCode, state: Hooks::NotFound, settings: None },
                AgentHooks {
                    agent: Agent::Antigravity,
                    state: Hooks::Installed,
                    settings: Some(PathBuf::from("C:/elsewhere/.gemini/config/hooks.json")),
                },
                AgentHooks {
                    agent: Agent::Cursor,
                    state: Hooks::NotInstalled,
                    settings: Some(PathBuf::from("C:/elsewhere/.cursor/hooks.json")),
                },
            ],
            providers: Providers::Checked(vec![
                ("Claude Code", Presence::Ready),
                ("Codex", Presence::NoLogin),
                ("Kilo Code", Presence::NotInstalled),
            ]),
            config_path: PathBuf::from("C:/elsewhere/noble/config.toml"),
            config: ConfigState::Parses,
            data_dir: PathBuf::from("C:/elsewhere/noble-data"),
            data_exists: true,
            gh: Gh { path: Some(PathBuf::from("C:/elsewhere/gh.exe")), auth: GhAuth::LoggedIn },
        }
    }

    /// Every agent in one hook state and with no settings file (the report's hook lines).
    fn hooks_all(state: Hooks) -> Vec<AgentHooks> {
        Agent::ALL.into_iter().map(|agent| AgentHooks { agent, state, settings: None }).collect()
    }

    /// Whether the report has this exact line (a substring match would let "not found" pass for "not found here").
    fn has_line(text: &str, line: &str) -> bool {
        text.lines().any(|l| l == line)
    }

    #[test]
    fn report_has_every_section_and_line() {
        let text = render(&sample());
        for section in [
            "[NOBLE]",
            "[System]",
            "[Terminal]",
            "[Shells]",
            "[Agent hooks]",
            "[AI providers]",
            "[Paths]",
            "[GitHub CLI]",
        ] {
            assert!(has_line(&text, section), "missing {section}:\n{text}");
        }
        for line in [
            "version: 1.10.0",
            "build: release",
            "system integrations: checked",
            "truecolor: yes",
            "WT_SESSION: set",
            "over SSH: no",
            "desktop: yes",
            "default (pwsh): C:/Program Files/PowerShell/7/pwsh.exe, integration: yes",
            "pwsh: C:/Program Files/PowerShell/7/pwsh.exe, integration: yes",
            "bash: not found",
            "Claude Code: installed",
            "Claude Code: detected (signed in)",
            "Codex: detected, not signed in",
            "Kilo Code: not found",
            "config.toml parses: yes",
            "gh: C:/elsewhere/gh.exe",
            "gh auth: logged in",
        ] {
            assert!(has_line(&text, line), "missing `{line}`:\n{text}");
        }
        // Paste-friendly: plain ASCII only.
        assert!(text.is_ascii(), "{text}");
    }

    #[test]
    fn dev_build_and_platform_specific_lines() {
        let mut facts = sample();
        facts.dev_build = true;
        facts.terminal.truecolor = false;
        facts.terminal.wt_session = None;
        facts.terminal.over_ssh = true;
        let text = render(&facts);
        assert!(has_line(&text, "build: dev"), "{text}");
        assert!(has_line(&text, "truecolor: no"), "{text}");
        assert!(has_line(&text, "over SSH: yes"), "{text}");
        // No Windows Terminal line where the platform has no such variable.
        assert!(!text.contains("WT_SESSION"), "{text}");
    }

    #[test]
    fn skipped_probes_say_why() {
        let mut facts = sample();
        facts.system = false;
        facts.hooks = hooks_all(Hooks::Skipped);
        facts.providers = Providers::Skipped;
        facts.gh.auth = GhAuth::Skipped;
        let text = render(&facts);
        assert!(has_line(&text, "system integrations: skipped (NOBLE_NO_SYSTEM_INTEGRATIONS is set)"), "{text}");
        for agent in Agent::ALL {
            assert!(has_line(&text, &format!("{}: {SKIPPED}", agent.name())), "{text}");
        }
        assert_eq!(text.matches("status: skipped (NOBLE_NO_SYSTEM_INTEGRATIONS is set)").count(), 1, "{text}");
        assert!(has_line(&text, "gh auth: skipped (NOBLE_NO_SYSTEM_INTEGRATIONS is set)"), "{text}");
        assert!(!text.contains("settings:"), "{text}");
        assert!(!text.contains("detected"), "{text}");
    }

    #[test]
    fn hook_states_are_plain_words() {
        let cases = [
            (Hooks::Installed, "Claude Code: installed"),
            (Hooks::NotInstalled, "Claude Code: not installed"),
            (Hooks::NotFound, "Claude Code: not found"),
            (Hooks::Unreadable, "Claude Code: unknown (the settings file cannot be read or parsed)"),
            (Hooks::NoHome, "Claude Code: unknown (no home folder)"),
        ];
        for (state, expected) in cases {
            let mut facts = sample();
            facts.hooks = hooks_all(state);
            assert!(has_line(&render(&facts), expected), "{state:?}");
        }
    }

    #[test]
    fn no_home_folder_is_unknown_for_hooks_and_providers() {
        let mut facts = sample();
        facts.hooks = hooks_all(Hooks::NoHome);
        facts.providers = Providers::NoHome;
        let text = render(&facts);
        for agent in Agent::ALL {
            assert!(has_line(&text, &format!("{}: unknown (no home folder)", agent.name())), "{text}");
        }
        assert_eq!(text.matches("status: unknown (no home folder)").count(), 1, "{text}");
        assert!(!text.contains("settings:"), "{text}");
    }

    #[test]
    fn config_states() {
        let cases = [
            (ConfigState::Missing, "config.toml parses: not created yet (the defaults are used)"),
            (ConfigState::Parses, "config.toml parses: yes"),
            (ConfigState::Broken(Some(3)), "config.toml parses: no (line 3)"),
            (ConfigState::Broken(None), "config.toml parses: no"),
            (ConfigState::Unreadable, "config.toml parses: unknown (the file cannot be read)"),
        ];
        for (state, expected) in cases {
            let mut facts = sample();
            facts.config = state;
            assert!(has_line(&render(&facts), expected), "{state:?}");
        }
    }

    #[test]
    fn gh_states_and_missing_gh() {
        let cases = [
            (GhAuth::NotLoggedIn, "gh auth: not logged in"),
            (GhAuth::Unknown, "gh auth: unknown"),
            (GhAuth::NotChecked, "gh auth: not checked"),
        ];
        for (auth, expected) in cases {
            let mut facts = sample();
            facts.gh.auth = auth;
            assert!(has_line(&render(&facts), expected), "{auth:?}");
        }
        let mut facts = sample();
        facts.gh = Gh { path: None, auth: GhAuth::NotChecked };
        assert!(has_line(&render(&facts), "gh: not found"));
    }

    #[test]
    fn shells_show_path_and_integration() {
        let mut facts = sample();
        facts.shells = vec![shell("cmd", Some("C:/Windows/system32/cmd.exe"), false), shell("zsh", None, false)];
        let text = render(&facts);
        assert!(has_line(&text, "cmd: C:/Windows/system32/cmd.exe, integration: no"), "{text}");
        assert!(has_line(&text, "zsh: not found"), "{text}");
    }

    #[test]
    fn home_folder_is_shortened() {
        let Some(home) = dirs::home_dir() else { return };
        let mut facts = sample();
        facts.config_path = home.join("noble").join("config.toml");
        let text = render(&facts);
        assert!(!text.contains(&home.display().to_string()), "{text}");
        assert!(text.contains("~"), "{text}");
    }

    #[test]
    fn read_config_states() {
        let dir = TempDir::new("noble-doctor-config");
        let path = dir.0.join("config.toml");
        assert_eq!(read_config(&path).0, ConfigState::Missing);
        assert!(!path.exists(), "the report must not create the config file");
        std::fs::write(&path, "[general]\n").unwrap();
        assert_eq!(read_config(&path).0, ConfigState::Parses);
        std::fs::write(&path, "ok = true\n[general\n").unwrap();
        assert!(matches!(read_config(&path).0, ConfigState::Broken(Some(_))));
        std::fs::write(&path, "\u{feff}[general]\n").unwrap();
        assert_eq!(read_config(&path).0, ConfigState::Parses, "a byte order mark is not an error");
    }

    /// A fake home with credential files in it. The tokens must never reach the report.
    #[test]
    fn report_never_prints_a_token() {
        let dir = TempDir::new("noble-doctor-token");
        let claude_token = "sk-ant-oat01-DOCTORTEST-0123456789abcdef";
        let codex_token = "codex-access-DOCTORTEST-fedcba9876543210";
        std::fs::create_dir_all(dir.0.join(".claude")).unwrap();
        std::fs::write(
            dir.0.join(".claude").join(".credentials.json"),
            format!(r#"{{"claudeAiOauth":{{"accessToken":"{claude_token}","refreshToken":"{claude_token}"}}}}"#),
        )
        .unwrap();
        std::fs::create_dir_all(dir.0.join(".codex")).unwrap();
        std::fs::write(
            dir.0.join(".codex").join("auth.json"),
            format!(r#"{{"tokens":{{"access_token":"{codex_token}"}}}}"#),
        )
        .unwrap();
        let paths = Paths { config: dir.0.join("config.toml"), data: dir.0.join("data") };
        let scope = Scope { home: Some(dir.0.clone()), system: true, gh_auth: false, env: Default::default() };
        let text = render(&gather(&paths, &scope));
        // The detection did read the fake login, so the absence of the token below means something.
        // (CLAUDE_CONFIG_DIR, when set on the machine running the test, moves where Claude Code is looked for.)
        if std::env::var_os("CLAUDE_CONFIG_DIR").is_none() {
            assert!(has_line(&text, "Claude Code: detected (signed in)"), "{text}");
        }
        for secret in [claude_token, codex_token, "DOCTORTEST", "sk-ant", "access_token", "refreshToken"] {
            assert!(!text.contains(secret), "the report shows `{secret}`:\n{text}");
        }
    }

    #[test]
    fn isolation_skips_the_home_probes() {
        let dir = TempDir::new("noble-doctor-isolated");
        std::fs::create_dir_all(dir.0.join(".claude")).unwrap();
        std::fs::write(
            dir.0.join(".claude").join(".credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"sk-ant-ISOLATED-token"}}"#,
        )
        .unwrap();
        let paths = Paths { config: dir.0.join("config.toml"), data: dir.0.join("data") };
        let scope = Scope { home: Some(dir.0.clone()), system: false, gh_auth: true, env: Default::default() };
        let facts = gather(&paths, &scope);
        assert!(facts.hooks.iter().all(|h| h.state == Hooks::Skipped));
        assert_eq!(facts.providers, Providers::Skipped);
        assert_eq!(facts.gh.auth, GhAuth::Skipped);
        assert!(facts.hooks.iter().all(|h| h.settings.is_none()));
        let text = render(&facts);
        assert!(!text.contains("ISOLATED"), "{text}");
    }

    #[test]
    fn gather_reads_only() {
        let dir = TempDir::new("noble-doctor-readonly");
        let paths = Paths { config: dir.0.join("config.toml"), data: dir.0.join("data") };
        let scope = Scope { home: Some(dir.0.clone()), system: true, gh_auth: false, env: Default::default() };
        let facts = gather(&paths, &scope);
        assert_eq!(facts.config, ConfigState::Missing);
        assert!(!dir.0.join("config.toml").exists());
        assert!(!dir.0.join("data").exists(), "the data folder is not created");
        // Claude's command may or may not be on this machine's PATH: either way no hook is installed.
        let claude = facts.hooks.iter().find(|h| h.agent == Agent::Claude).expect("claude row");
        assert!(matches!(claude.state, Hooks::NotInstalled | Hooks::NotFound), "no settings file means no hooks");
        assert!(!dir.0.join(".claude").exists(), "the settings file is not created");
    }
}
