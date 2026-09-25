//! `config.toml`: loading, defaults, path resolution and small in-place edits.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub terminal: TerminalCfg,
    pub keys: KeysCfg,
    pub projects: ProjectsCfg,
    pub ai: AiCfg,
    pub launchers: Vec<Launcher>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct General {
    pub theme: String,
    pub transparent: bool,
    pub boot_animation: bool,
    /// Page transition and pane zoom animations.
    pub animations: bool,
    pub clock_24h: bool,
    pub show_seconds: bool,
    /// Name in the greeting text; the user name is used when empty.
    pub operator: String,
    /// Check GitHub once a day for a new release.
    pub check_updates: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TerminalCfg {
    /// Empty = automatic: pwsh → powershell → cmd (Windows), $SHELL (Unix).
    pub shell: String,
    pub shell_args: Vec<String>,
    pub scrollback: usize,
    pub restore_session: bool,
    pub copy_on_select: bool,
    /// Pane color scheme: "windows-terminal" (the PowerShell profile's scheme), "theme"
    /// (follow the UI theme) or a scheme name ("dark-plus", "light-gray", "wt:<name>"…).
    pub colors: String,
    /// "#rrggbb" colors overriding the scheme's background/text (empty = the scheme's own).
    pub background: String,
    pub foreground: String,
    /// Show a notification when a background tab needs attention (+ bell to the outer terminal).
    pub notify: bool,
    /// Report the end of a background command that ran at least this many seconds (0 = off).
    pub notify_after: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct KeysCfg {
    pub prefix: String,
    /// Key pressed after the prefix → action (overrides the defaults).
    pub prefix_bindings: BTreeMap<String, String>,
    /// Global shortcuts without the prefix → action.
    pub direct_bindings: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProjectsCfg {
    /// Empty = common folders are scanned automatically.
    pub roots: Vec<String>,
    pub max_depth: usize,
    pub exclude: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AiCfg {
    pub enabled: bool,
    pub refresh_minutes: u64,
    pub providers: Vec<String>,
    /// Warn when a quota window reaches this percent (0 = off).
    pub warn_at: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Launcher {
    pub key: String,
    pub name: String,
    pub command: String,
    /// Whether it shows as a button/shortcut on Home (already hidden when not installed).
    #[serde(default = "yes")]
    pub show: bool,
}

fn yes() -> bool {
    true
}

impl Default for General {
    fn default() -> Self {
        Self {
            theme: "amber".into(),
            transparent: false,
            boot_animation: true,
            animations: true,
            clock_24h: true,
            show_seconds: true,
            operator: String::new(),
            check_updates: true,
        }
    }
}

impl Default for TerminalCfg {
    fn default() -> Self {
        Self {
            shell: String::new(),
            shell_args: Vec::new(),
            scrollback: 5000,
            restore_session: true,
            copy_on_select: true,
            colors: "windows-terminal".into(),
            background: String::new(),
            foreground: String::new(),
            notify: true,
            notify_after: 10,
        }
    }
}

impl Default for KeysCfg {
    fn default() -> Self {
        Self { prefix: "ctrl+a".into(), prefix_bindings: BTreeMap::new(), direct_bindings: BTreeMap::new() }
    }
}

impl Default for ProjectsCfg {
    fn default() -> Self {
        Self { roots: Vec::new(), max_depth: 4, exclude: Vec::new() }
    }
}

impl Default for AiCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            refresh_minutes: 5,
            providers: ["claude", "codex", "antigravity", "opencode-go", "kilo", "command-code"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            warn_at: 90,
        }
    }
}

/// Home keys assignable to launchers: those that do not clash with Home's own
/// shortcuts (k j t m s p q w r a o).
pub const LAUNCH_KEYS: [&str; 15] = ["c", "x", "l", "g", "d", "e", "f", "b", "n", "u", "v", "y", "z", "i", "h"];

/// Default launchers (key, name, command). Those not installed stay hidden on Home.
const DEFAULT_LAUNCHERS: [(&str, &str, &str); 13] = [
    ("c", "claude", "claude"),
    ("x", "codex", "codex"),
    ("e", "opencode", "opencode"),
    ("i", "copilot", "copilot"),
    ("g", "antigravity", "agy"),
    ("v", "pi", "pi"),
    ("b", "oh-my-pi", "omp"),
    ("f", "freebuff", "freebuff"),
    ("z", "grok build", "grok"),
    ("u", "cursor", "cursor-agent"),
    // The `cmd` alias clashes with Windows' own cmd.exe.
    ("d", "command code", "command-code"),
    ("l", "cline", "cline"),
    ("n", "kilo code", "kilo"),
];

pub fn default_launchers() -> Vec<Launcher> {
    DEFAULT_LAUNCHERS
        .iter()
        .map(|(k, n, c)| Launcher { key: k.to_string(), name: n.to_string(), command: c.to_string(), show: true })
        .collect()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: General::default(),
            terminal: TerminalCfg::default(),
            keys: KeysCfg::default(),
            projects: ProjectsCfg::default(),
            ai: AiCfg::default(),
            launchers: default_launchers(),
        }
    }
}

/// The commented template written on the first run.
pub const DEFAULT_CONFIG: &str = r#"# NOBLE configuration — the app reloads itself when the file is saved.

[general]
theme = "amber"          # pickable from the Settings tab (20 themes)
transparent = false      # true: the terminal's own background (transparency) shows
boot_animation = true
animations = true        # page transition and fullscreen animations
clock_24h = true
show_seconds = true
operator = ""            # greeting name; empty = user name
check_updates = true     # check once a day for a new release; notice appears bottom right

[terminal]
shell = ""               # empty = automatic (pwsh → powershell → cmd | $SHELL)
shell_args = []
scrollback = 5000
restore_session = true   # save tabs on exit, restore them at startup
copy_on_select = true    # copy mouse-selected text to the clipboard
colors = "windows-terminal"  # pane colors: windows-terminal (your PowerShell scheme; the theme elsewhere) | theme | dark-plus | campbell | light-gray …
background = ""          # override the scheme background, e.g. '#c8c8c8' (empty = the scheme's own)
foreground = ""          # override the scheme text color, e.g. '#2e2e2e'
notify = true            # notify when a background tab needs attention (bell, app notification)
notify_after = 10        # report the end of a background command that took at least this many seconds (0 = off)

[keys]
prefix = "ctrl+a"        # prefix key; pressed twice it is sent to the shell
# Key pressed after the prefix → action. Example:
# [keys.prefix_bindings]
# "%" = "split_right"
# Shortcuts without the prefix. Example:
# [keys.direct_bindings]
# "alt+enter" = "zoom"

[projects]
roots = []               # empty = Desktop, Documents, source/repos, projects, code, dev ...
max_depth = 4
exclude = []

[ai]
enabled = true
refresh_minutes = 5      # only while Home is open; refreshed immediately when Home opens
providers = ["claude", "codex", "antigravity", "opencode-go", "kilo", "command-code"]
warn_at = 90             # warn when a quota window reaches this percent (0 = off)

# One-key commands for the selected project on Home. Only those found on PATH
# are visible; `show = false` hides one (Settings → Quick launch).
[[launchers]]
key = "c"
name = "claude"
command = "claude"

[[launchers]]
key = "x"
name = "codex"
command = "codex"

[[launchers]]
key = "e"
name = "opencode"
command = "opencode"

[[launchers]]
key = "i"
name = "copilot"
command = "copilot"

[[launchers]]
key = "g"
name = "antigravity"
command = "agy"

[[launchers]]
key = "v"
name = "pi"
command = "pi"

[[launchers]]
key = "b"
name = "oh-my-pi"
command = "omp"

[[launchers]]
key = "f"
name = "freebuff"
command = "freebuff"

[[launchers]]
key = "z"
name = "grok build"
command = "grok"

[[launchers]]
key = "u"
name = "cursor"
command = "cursor-agent"

[[launchers]]
key = "d"
name = "command code"
command = "command-code"

[[launchers]]
key = "l"
name = "cline"
command = "cline"

[[launchers]]
key = "n"
name = "kilo code"
command = "kilo"
"#;

/// The app's file paths. The `NOBLE_HOME` env var puts them all in one folder.
#[derive(Clone, Debug)]
pub struct Paths {
    pub config: PathBuf,
    pub data: PathBuf,
}

impl Paths {
    pub fn resolve(config_override: Option<PathBuf>) -> Paths {
        let base_override = std::env::var_os("NOBLE_HOME").map(PathBuf::from);
        let config_dir = base_override
            .clone()
            .or_else(|| dirs::config_dir().map(|d| d.join("noble")))
            .unwrap_or_else(|| PathBuf::from(".noble"));
        let data = base_override
            .or_else(|| dirs::data_local_dir().map(|d| d.join("noble")))
            .unwrap_or_else(|| PathBuf::from(".noble"));
        Paths { config: config_override.unwrap_or_else(|| config_dir.join("config.toml")), data }
    }

    pub fn data_file(&self, name: &str) -> PathBuf {
        self.data.join(name)
    }
}

/// Load result: the config and (optionally) an error message. The error is not fatal.
pub struct Loaded {
    pub config: Config,
    pub error: Option<String>,
    pub mtime: Option<SystemTime>,
}

pub fn parse(text: &str) -> Result<Config, String> {
    let mut cfg: Config = toml::from_str(text).map_err(|e| e.message().to_string())?;
    // Removed providers are dropped silently.
    cfg.ai.providers.retain(|p| {
        matches!(
            p.to_ascii_lowercase().as_str(),
            "claude" | "codex" | "antigravity" | "opencode-go" | "kilo" | "command-code"
        )
    });
    if cfg.launchers.is_empty() && !text.contains("[[launchers]]") {
        cfg.launchers = default_launchers();
    } else {
        add_missing_launchers(&mut cfg.launchers);
    }
    Ok(cfg)
}

/// Appends the default launchers missing from older configs (those whose command
/// is not in the list). A busy key gets the first free one. To hide a launcher you
/// use `show = false` instead of deleting it, so re-adding them is harmless.
fn add_missing_launchers(list: &mut Vec<Launcher>) {
    for d in default_launchers() {
        if list.iter().any(|l| l.command.eq_ignore_ascii_case(&d.command)) {
            continue;
        }
        let taken = |k: &str| list.iter().any(|l| l.key == k);
        let key = if taken(&d.key) {
            LAUNCH_KEYS.iter().find(|k| !taken(k)).map(|k| k.to_string())
        } else {
            Some(d.key.clone())
        };
        let Some(key) = key else { continue };
        list.push(Launcher { key, ..d });
    }
}

/// Reads the config; writes the commented template when the file is missing.
pub fn load(path: &Path) -> Loaded {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, DEFAULT_CONFIG);
    }
    let mtime = mtime_of(path);
    match std::fs::read_to_string(path) {
        Ok(text) => match parse(&text) {
            Ok(config) => Loaded { config, error: None, mtime },
            Err(e) => Loaded { config: Config::default(), error: Some(format!("config: {e}")), mtime },
        },
        Err(_) => Loaded { config: Config::default(), error: None, mtime },
    }
}

pub fn mtime_of(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Updates a `key = value` line in a section preserving its comments; appends the
/// line at the end of the section when missing, and creates the section at the end
/// of the file when it does not exist.
pub fn set_value(text: &str, section: &str, key: &str, value_toml: &str) -> String {
    let header = format!("[{section}]");
    let mut out: Vec<String> = Vec::new();
    let mut in_section = false;
    let mut section_seen = false;
    let mut done = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if in_section && !done {
                // Append at the end of the section (before the trailing blank lines).
                let mut tail = Vec::new();
                while let Some(l) = out.pop_if(|l| l.trim().is_empty()) {
                    tail.push(l);
                }
                out.push(format!("{key} = {value_toml}"));
                out.extend(tail);
                done = true;
            }
            in_section = trimmed == header;
            section_seen |= in_section;
        } else if in_section && !done {
            let lhs = trimmed.split('=').next().unwrap_or("").trim();
            if lhs == key && !trimmed.starts_with('#') {
                let comment = line.find(" #").map(|i| &line[i..]).unwrap_or("");
                let new_line = format!("{key} = {value_toml}");
                let padded = if comment.is_empty() {
                    new_line
                } else {
                    let pad = 25usize.saturating_sub(new_line.len()).max(1);
                    format!("{new_line}{}{}", " ".repeat(pad), comment.trim_start())
                };
                out.push(padded);
                done = true;
                continue;
            }
        }
        out.push(line.to_string());
    }
    if !done {
        if in_section {
            out.push(format!("{key} = {value_toml}"));
        } else if !section_seen {
            out.push(String::new());
            out.push(header);
            out.push(format!("{key} = {value_toml}"));
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// Replaces every `[[launchers]]` block with the given list; everything else
/// (comments included) is preserved. The blocks go where the first one was, or at the end.
pub fn set_launchers(text: &str, list: &[Launcher]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut insert_at = None;
    let mut in_block = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_block = trimmed == "[[launchers]]";
            if in_block {
                // The block content (trailing blank lines included) is skipped.
                insert_at.get_or_insert(out.len());
                continue;
            }
        }
        if !in_block {
            out.push(line.to_string());
        }
    }
    let quote = |s: &str| toml::Value::String(s.to_string()).to_string();
    let mut blocks = Vec::new();
    for (i, l) in list.iter().enumerate() {
        if i > 0 {
            blocks.push(String::new());
        }
        blocks.push("[[launchers]]".into());
        blocks.push(format!("key = {}", quote(&l.key)));
        blocks.push(format!("name = {}", quote(&l.name)));
        blocks.push(format!("command = {}", quote(&l.command)));
        if !l.show {
            blocks.push("show = false".into());
        }
    }
    let at = insert_at.unwrap_or_else(|| {
        if out.last().is_some_and(|l| !l.trim().is_empty()) {
            out.push(String::new());
        }
        out.len()
    });
    // Leave a blank line when another section follows.
    if at < out.len() && !out[at].trim().is_empty() {
        blocks.push(String::new());
    }
    out.splice(at..at, blocks);
    let mut s = out.join("\n");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_template_parses() {
        let cfg = parse(DEFAULT_CONFIG).unwrap();
        assert_eq!(cfg.general.theme, "amber");
        assert_eq!(cfg.launchers.len(), DEFAULT_LAUNCHERS.len());
        assert_eq!(cfg.keys.prefix, "ctrl+a");
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn partial_config_uses_defaults() {
        let cfg = parse("[general]\ntheme = \"ice\"\n").unwrap();
        assert_eq!(cfg.general.theme, "ice");
        assert!(cfg.general.boot_animation);
        assert_eq!(cfg.terminal.scrollback, 5000);
        assert_eq!(cfg.launchers.len(), DEFAULT_LAUNCHERS.len());
    }

    #[test]
    fn set_launchers_rewrites_blocks() {
        let mut list = default_launchers();
        list[1].show = false;
        list[0].key = "l".into();
        let out = set_launchers(DEFAULT_CONFIG, &list);
        let cfg = parse(&out).unwrap();
        assert_eq!(cfg.launchers, list);
        assert_eq!(out.matches("[[launchers]]").count(), list.len());
        assert!(out.contains("show = false") && out.contains("# One-key commands for the selected project"));
        // Blocks sitting before another section are changed correctly too.
        let text = "[[launchers]]\nkey = \"c\"\nname = \"a\"\ncommand = \"a\"\n\n[general]\ntheme = \"ice\"\n";
        let cfg = parse(&set_launchers(text, &list)).unwrap();
        assert_eq!(cfg.launchers, list);
        assert_eq!(cfg.general.theme, "ice");
    }

    #[test]
    fn old_config_gets_new_launchers() {
        // Old config: only claude/codex, "e" on another command, codex hidden.
        let text = "[[launchers]]
key = \"c\"
name = \"claude\"
command = \"claude\"

                    [[launchers]]
key = \"x\"
name = \"codex\"
command = \"codex\"
show = false

                    [[launchers]]
key = \"e\"
name = \"aider\"
command = \"aider\"
";
        let cfg = parse(text).unwrap();
        let names: Vec<_> = cfg.launchers.iter().map(|l| (l.key.as_str(), l.name.as_str())).collect();
        assert_eq!(names[..5], [("c", "claude"), ("x", "codex"), ("e", "aider"), ("l", "opencode"), ("i", "copilot")]);
        assert_eq!(cfg.launchers.len(), DEFAULT_LAUNCHERS.len() + 1);
        // The keys do not clash.
        let mut keys: Vec<_> = cfg.launchers.iter().map(|l| l.key.clone()).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), cfg.launchers.len());
        assert!(!cfg.launchers[1].show);
        // The provider list stays as the user chose it.
        let cfg = parse(
            "[ai]
providers = [\"claude\", \"gemini\"]
",
        )
        .unwrap();
        assert_eq!(cfg.ai.providers, ["claude"]);
    }

    #[test]
    fn bad_config_reports_error() {
        assert!(parse("[general\ntheme=").is_err());
    }

    #[test]
    fn set_value_keeps_comments() {
        let out = set_value(DEFAULT_CONFIG, "general", "theme", "\"synth\"");
        assert!(out.contains("theme = \"synth\""));
        assert!(out.contains("# pickable from the Settings tab"));
        assert_eq!(parse(&out).unwrap().general.theme, "synth");
        let added = set_value("[general]\nclock_24h = true\n\n[ai]\n", "general", "theme", "\"ice\"");
        assert_eq!(parse(&added).unwrap().general.theme, "ice");
        let created = set_value("", "general", "theme", "\"ice\"");
        assert_eq!(parse(&created).unwrap().general.theme, "ice");
    }
}
