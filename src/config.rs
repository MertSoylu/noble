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
    /// The startup animation: on the first launch and the first launch after an update (false: never).
    pub boot_animation: bool,
    /// Page transitions, pane zoom, fades and other small animations.
    pub animations: bool,
    /// Keep the animations while running on battery (laptops).
    pub animations_on_battery: bool,
    pub clock_24h: bool,
    pub show_seconds: bool,
    /// Name in the greeting text; the user name is used when empty.
    pub operator: String,
    /// Check GitHub once a day for a new release.
    pub check_updates: bool,
    /// Hints in the status bar, about the current screen (a new one with every screen change).
    pub show_tips: bool,
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
    /// Programs in a pane may set the system clipboard with OSC 52 (false: such requests are ignored).
    pub osc52_write: bool,
    /// A tab is named after the folder its focused pane is in now (false: the folder it opened in).
    pub tab_follows_cwd: bool,
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
    /// In a split tab, the unfocused panes are drawn toward their background (their text fades).
    pub dim_inactive: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct KeysCfg {
    pub prefix: String,
    /// Key pressed after the prefix → action (overrides the defaults).
    pub prefix_bindings: BTreeMap<String, String>,
    /// Global shortcuts without the prefix → action.
    pub direct_bindings: BTreeMap<String, String>,
    /// How keys NOBLE uses reach the app in a pane: "lock" (prefix i toggles a per-pane lock that sends every
    /// shortcut to the app) or "once" (prefix + a shortcut sends that key to the app, prefix i the next key).
    pub passthrough: String,
    /// In a terminal pane the shell keys among the direct shortcuts (`keys::SHELL_KEYS`) go to the app.
    pub shell_first: bool,
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
            animations_on_battery: true,
            clock_24h: true,
            show_seconds: true,
            operator: String::new(),
            check_updates: true,
            show_tips: true,
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
            osc52_write: true,
            tab_follows_cwd: true,
            colors: "windows-terminal".into(),
            background: String::new(),
            foreground: String::new(),
            notify: true,
            notify_after: 10,
            dim_inactive: true,
        }
    }
}

/// `keys.passthrough` values; the first is the default.
pub const PASSTHROUGH_MODES: [&str; 2] = ["lock", "once"];

impl Default for KeysCfg {
    fn default() -> Self {
        Self {
            prefix: "ctrl+a".into(),
            prefix_bindings: BTreeMap::new(),
            direct_bindings: BTreeMap::new(),
            passthrough: PASSTHROUGH_MODES[0].into(),
            shell_first: true,
        }
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
theme = "amber"          # pickable from the Settings tab (22 themes)
transparent = false      # true: the terminal's own background (transparency) shows
boot_animation = true    # first launch and first launch after an update (false: never)
animations = true        # page transitions, zoom, fades and other small animations
animations_on_battery = true  # keep them while a laptop runs on battery (false: off on battery)
clock_24h = true
show_seconds = true
operator = ""            # greeting name; empty = user name
check_updates = true     # check once a day for a new release; notice appears bottom right
show_tips = true         # status bar tips about the current screen (changes with the screen)

[terminal]
shell = ""               # empty = automatic (pwsh → powershell → cmd | $SHELL)
shell_args = []
scrollback = 5000
restore_session = true   # save tabs on exit, restore them at startup
copy_on_select = true    # copy mouse-selected text to the clipboard
osc52_write = true       # programs in a pane may set the clipboard (OSC 52); false ignores them
tab_follows_cwd = true   # name a tab after its current folder (false: the folder it was opened in)
colors = "windows-terminal"  # pane colors: windows-terminal (your PowerShell scheme; the theme elsewhere) | theme | dark-plus | campbell | light-gray …
background = ""          # override the scheme background, e.g. '#c8c8c8' (empty = the scheme's own)
foreground = ""          # override the scheme text color, e.g. '#2e2e2e'
notify = true            # notify when a background tab needs attention (bell, app notification)
notify_after = 10        # report the end of a background command that took at least this many seconds (0 = off)
dim_inactive = true      # in a split tab, fade the text of the panes that do not have the focus

[keys]
prefix = "ctrl+a"        # prefix key; pressed twice it is sent to the shell
# Sending NOBLE's own shortcuts (alt+p, alt+m …) to the app in a pane:
# "lock" = prefix i locks the pane, every shortcut goes to the app until prefix i again
# "once" = prefix + a shortcut (e.g. ctrl+a alt+p) sends just that key; prefix i sends the next key
passthrough = "lock"
# alt+. alt+, alt+t alt+s alt+p are shell keys (last argument, transpose, sudo …): in a terminal
# they go to the shell/app, elsewhere they stay NOBLE's. false = NOBLE takes them everywhere.
shell_first = true
# Key pressed after the prefix → action. Example:
# [keys.prefix_bindings]
# "%" = "split_right"
# Shortcuts without the prefix. Example:
# [keys.direct_bindings]
# "alt+enter" = "zoom"

[projects]
roots = []               # empty = Desktop, Documents, source/repos, projects, Developer, code, dev ...
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
    /// `None` when there is no folder for the files (no home folder and no `NOBLE_HOME`): the caller reports it.
    pub fn resolve(config_override: Option<PathBuf>) -> Option<Paths> {
        let base = std::env::var_os("NOBLE_HOME").filter(|v| !v.is_empty()).map(PathBuf::from);
        let config_dir = dirs::config_dir().map(|d| d.join("noble"));
        let data_dir = dirs::data_local_dir().map(|d| d.join("noble"));
        Paths::from_dirs(base, config_dir, data_dir, config_override)
    }

    /// The paths from the folders found; `base` (`NOBLE_HOME`) takes the place of both. A missing folder is
    /// `None`, never a relative one: `.noble` would be written wherever NOBLE was started from.
    fn from_dirs(
        base: Option<PathBuf>,
        config_dir: Option<PathBuf>,
        data_dir: Option<PathBuf>,
        config_override: Option<PathBuf>,
    ) -> Option<Paths> {
        let config = match config_override {
            Some(path) => path,
            None => base.clone().or(config_dir)?.join("config.toml"),
        };
        let data = base.or(data_dir)?;
        Some(Paths { config, data })
    }

    pub fn data_file(&self, name: &str) -> PathBuf {
        self.data.join(name)
    }
}

/// Load result: the config and (optionally) an error. The error is not fatal.
pub struct Loaded {
    pub config: Config,
    pub error: Option<ConfigError>,
    pub mtime: Option<SystemTime>,
}

/// A config that does not load: the message (one line, shown as a toast) and the 1-based line of the
/// mistake, which lets the toast open the file there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    pub text: String,
    pub line: Option<u32>,
}

/// The error for a TOML parse failure: `config line 5, column 3: <message>` (the position is
/// computed from the byte offset of the error, so it also works for text that is not one line).
fn parse_error(text: &str, e: &toml::de::Error) -> ConfigError {
    let message = e.message().split_whitespace().collect::<Vec<_>>().join(" ");
    let Some(span) = e.span() else {
        return ConfigError { text: format!("config: {message}"), line: None };
    };
    let bytes = text.as_bytes();
    let before = &bytes[..span.start.min(bytes.len())];
    let line = before.iter().filter(|&&b| b == b'\n').count() + 1;
    let line_start = before.iter().rposition(|&b| b == b'\n').map_or(0, |p| p + 1);
    let column = String::from_utf8_lossy(&before[line_start..]).chars().count() + 1;
    ConfigError { text: format!("config line {line}, column {column}: {message}"), line: Some(line as u32) }
}

pub fn parse(text: &str) -> Result<Config, ConfigError> {
    let mut cfg: Config = toml::from_str(text).map_err(|e| parse_error(text, &e))?;
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
        // Atomic like every config write: a crash while the template is written leaves no truncated file.
        let _ = crate::store::write_text(path, DEFAULT_CONFIG);
    }
    let mtime = mtime_of(path);
    match std::fs::read_to_string(path) {
        Ok(text) => match parse(&text) {
            Ok(config) => Loaded { config, error: None, mtime },
            Err(error) => Loaded { config: Config::default(), error: Some(error), mtime },
        },
        // A dangling link (its target is missing): the link is not replaced, so it is reported, not silently defaulted.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && std::fs::symlink_metadata(path).is_ok() => {
            let error = ConfigError {
                text: format!("config: {} is a link to a missing file", crate::util::tilde(path)),
                line: None,
            };
            Loaded { config: Config::default(), error: Some(error), mtime }
        }
        // No file (and none could be written, e.g. a read-only home): the defaults, silently.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Loaded { config: Config::default(), error: None, mtime },
        // Unreadable, or not UTF-8 (e.g. saved in a legacy code page): report it instead of hiding it.
        Err(e) => {
            let error = ConfigError { text: format!("config: {e}"), line: None };
            Loaded { config: Config::default(), error: Some(error), mtime }
        }
    }
}

/// The config text to rewrite from the Settings screen: the template when the file is missing,
/// an error when it exists but cannot be read (it is then left untouched rather than replaced).
pub fn read_for_edit(path: &Path) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DEFAULT_CONFIG.into()),
        Err(e) => Err(format!("config not saved: {e}")),
    }
}

/// Byte index of the `#` that starts a line's comment; a `#` inside a "basic" or 'literal'
/// string does not count.
fn comment_start(line: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        match quote {
            Some('"') if escaped => escaped = false,
            Some('"') if c == '\\' => escaped = true,
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '#' => return Some(i),
            None => {}
        }
    }
    None
}

/// A line without its comment.
fn strip_comment(line: &str) -> &str {
    comment_start(line).map_or(line, |i| &line[..i])
}

pub fn mtime_of(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The part of a `key = value` line after the first `=` outside quotes (the whole line when none).
fn strip_value(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        match quote {
            Some('"') if escaped => escaped = false,
            Some('"') if c == '\\' => escaped = true,
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '=' => return &line[i + 1..],
            None => {}
        }
    }
    line
}

/// Bracket depth (`[` / `{`) after `line`, starting from `depth`; brackets inside strings and
/// comments do not count. Multi-line strings are not supported (the quote state resets per line).
fn bracket_depth(line: &str, mut depth: usize) -> usize {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in line.chars() {
        match quote {
            Some('"') if escaped => escaped = false,
            Some('"') if c == '\\' => escaped = true,
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None => match c {
                '"' | '\'' => quote = Some(c),
                '#' => break,
                '[' | '{' => depth += 1,
                ']' | '}' => depth = depth.saturating_sub(1),
                _ => {}
            },
        }
    }
    depth
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
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
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
            in_section = strip_comment(trimmed).trim_end() == header;
            section_seen |= in_section;
        } else if in_section && !done {
            let lhs = trimmed.split('=').next().unwrap_or("").trim();
            if lhs == key && !trimmed.starts_with('#') {
                let comment = comment_start(line).map(|i| &line[i..]).unwrap_or("");
                let new_line = format!("{key} = {value_toml}");
                let padded = if comment.is_empty() {
                    new_line
                } else {
                    let pad = 25usize.saturating_sub(new_line.len()).max(1);
                    format!("{new_line}{}{}", " ".repeat(pad), comment.trim_start())
                };
                out.push(padded);
                done = true;
                // A multi-line array/table value: drop its continuation lines too.
                let mut depth = bracket_depth(strip_value(line), 0);
                while depth > 0 {
                    let Some(next) = lines.next() else { break };
                    depth = bracket_depth(next, depth);
                }
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
            in_block = strip_comment(trimmed).trim_end() == "[[launchers]]";
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

/// The key of a `key = value` line, `None` for comments, headers, blank lines and anything that is not
/// one single-line pair. Parsed as TOML, so quoted keys such as `"="` or `"\""` come out right.
fn line_key(line: &str) -> Option<String> {
    let t = line.trim();
    if t.is_empty() || t.starts_with('#') || t.starts_with('[') {
        return None;
    }
    let table: toml::Table = toml::from_str(t).ok()?;
    let mut keys = table.keys();
    let key = keys.next()?.clone();
    keys.next().is_none().then_some(key)
}

/// Two spellings of one key chord (`ctrl+a` and `Ctrl+A`) are the same entry.
fn same_chord(a: &str, b: &str) -> bool {
    match (crate::keys::Chord::parse(a), crate::keys::Chord::parse(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

/// A TOML string literal (quoted and escaped).
fn quoted(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

/// Adds `entry` at the end of the table in `out`: before its trailing blank lines, and before a comment
/// that sits right above the next header (with no blank line in between) because that comment titles the
/// next table.
fn push_at_table_end(out: &mut Vec<String>, entry: &str) {
    let mut tail = Vec::new();
    while let Some(l) = out.pop_if(|l| l.trim().is_empty() || l.trim().starts_with('#')) {
        tail.push(l);
    }
    tail.reverse();
    // With a blank line in the run, the entry goes before the first one (comments above it belong to the
    // table); without one, before the whole run.
    let at = tail.iter().position(|l| l.trim().is_empty()).unwrap_or(0);
    out.extend(tail.drain(..at));
    out.push(entry.to_string());
    out.extend(tail);
}

/// The dotted name of the table a header line opens (`[keys.prefix_bindings]`, also spelled
/// `[ keys . "prefix_bindings" ]`); `None` for other lines and for arrays of tables (`[[launchers]]`).
fn header_name(line: &str) -> Option<String> {
    let inner = strip_comment(line.trim()).trim().strip_prefix('[')?.strip_suffix(']')?;
    if inner.starts_with('[') || inner.ends_with(']') {
        return None;
    }
    let parts: Vec<&str> = inner.split('.').map(|p| p.trim().trim_matches(|c| c == '"' || c == '\'')).collect();
    Some(parts.join("."))
}

/// Whether `table` exists in the text without a header line for it: an inline table
/// (`prefix_bindings = { … }`) or dotted keys (`prefix_bindings."x" = …`). Adding a header for it
/// would then make a duplicate table, so the edit is refused instead.
fn defined_without_header(text: &str, table: &str) -> bool {
    let Ok(doc) = text.parse::<toml::Table>() else { return false };
    let mut node: Option<&toml::Table> = Some(&doc);
    for part in table.split('.') {
        node = node.and_then(|t| t.get(part)).and_then(toml::Value::as_table);
    }
    node.is_some() && !text.lines().any(|l| header_name(l).as_deref() == Some(table))
}

/// The one edit behind `set_binding` and `remove_binding`: `Some(value)` writes `"chord" = "value"` in
/// `[table]` (in place of the line of that chord, its comment kept; else at the end of the table; the
/// table is created at the end of the file when missing), `None` deletes the chord's line. Everything
/// else stays as it is; a file without a change is returned untouched. The result must parse as a
/// config: otherwise nothing is written and the error says why. A table written in another form
/// (see `defined_without_header`) is refused rather than duplicated.
fn edit_binding(text: &str, table: &str, chord: &str, value: Option<&str>) -> Result<String, String> {
    if defined_without_header(text, table) {
        return Err(format!(
            "{table} is written in another form in config.toml (inline or dotted keys): edit it by hand"
        ));
    }
    let header = format!("[{table}]");
    let entry = value.map(|v| format!("{} = {}", quoted(chord), quoted(v)));
    let mut out: Vec<String> = Vec::new();
    let mut in_table = false;
    let mut seen_table = false;
    let mut done = false;
    let mut changed = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if in_table && !done {
                if let Some(e) = &entry {
                    push_at_table_end(&mut out, e);
                    changed = true;
                }
                done = true;
            }
            in_table = header_name(line).as_deref() == Some(table);
            seen_table |= in_table;
        } else if in_table && line_key(line).is_some_and(|k| same_chord(&k, chord)) {
            // The first line of the chord is replaced (or dropped); any later duplicate is dropped.
            if !done {
                if let Some(e) = &entry {
                    let comment = comment_start(line).map(|i| format!("  {}", &line[i..])).unwrap_or_default();
                    out.push(format!("{e}{comment}"));
                }
                done = true;
            }
            changed = true;
            continue;
        }
        out.push(line.to_string());
    }
    if !done && let Some(e) = &entry {
        if in_table {
            push_at_table_end(&mut out, e);
            changed = true;
        } else if !seen_table {
            out.push(String::new());
            out.push(header);
            out.push(e.clone());
            changed = true;
        }
    }
    let updated = if changed {
        // `lines()` dropped the `\r` of a CRLF file (Notepad on Windows): the file keeps its line endings.
        let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let mut s = out.join(eol);
        s.push_str(eol);
        s
    } else {
        text.to_string()
    };
    match parse(&updated) {
        Ok(_) => Ok(updated),
        Err(e) => Err(format!("not saved, the edit would break config.toml: {}", e.text)),
    }
}

/// Sets `"chord" = "value"` in the `[table]` of the config text (`value` "none" unbinds the chord),
/// preserving the comments and every other line.
pub fn set_binding(text: &str, table: &str, chord: &str, value: &str) -> Result<String, String> {
    edit_binding(text, table, chord, Some(value))
}

/// Removes the `chord` line from the `[table]` of the config text, so the default binding applies again.
pub fn remove_binding(text: &str, table: &str, chord: &str) -> Result<String, String> {
    edit_binding(text, table, chord, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// OSC 52 clipboard writes from programs are on by default (as before the setting) and can be turned off.
    #[test]
    fn osc52_write_defaults_on_and_can_be_turned_off() {
        assert!(Config::default().terminal.osc52_write);
        assert!(parse(DEFAULT_CONFIG).unwrap().terminal.osc52_write);
        assert!(!parse("[terminal]\nosc52_write = false\n").unwrap().terminal.osc52_write);
    }

    /// Without a folder for the files there is no `Paths` at all: never a relative `.noble` in the working folder.
    #[test]
    fn paths_never_fall_back_to_the_working_folder() {
        let home = PathBuf::from("noble-home");
        let p = Paths::from_dirs(Some(home.clone()), None, None, None).unwrap();
        assert_eq!((p.config.clone(), p.data.clone()), (home.join("config.toml"), home.clone()));
        let split = Paths::from_dirs(None, Some("cfg".into()), Some("data".into()), None).unwrap();
        assert_eq!((split.config, split.data), (PathBuf::from("cfg").join("config.toml"), PathBuf::from("data")));
        // A config file named on the command line needs no config folder, but the data still needs one.
        let named = Paths::from_dirs(None, None, Some("data".into()), Some("my.toml".into())).unwrap();
        assert_eq!(named.config, PathBuf::from("my.toml"));
        assert!(Paths::from_dirs(None, None, Some("data".into()), None).is_none());
        assert!(Paths::from_dirs(None, Some("cfg".into()), None, None).is_none());
        assert!(Paths::from_dirs(None, None, None, Some("my.toml".into())).is_none());
    }

    /// The provider list is spelled out in several places; all of them must match the `ai::providers` registry
    /// (a new provider added to one place only would silently vanish from config, Settings or the defaults).
    #[test]
    fn provider_lists_match_the_registry() {
        let ids: Vec<&str> = crate::ai::providers::registry().iter().map(|p| p.id).collect();
        fn owned(v: &[String]) -> Vec<&str> {
            v.iter().map(String::as_str).collect()
        }
        // `AiCfg::default`
        assert_eq!(owned(&AiCfg::default().providers), ids);
        // `DEFAULT_CONFIG`
        assert_eq!(owned(&parse(DEFAULT_CONFIG).unwrap().ai.providers), ids);
        // The Settings rows (`app::PROVIDER_KEYS`)
        let keys: Vec<&str> = crate::app::PROVIDER_KEYS.iter().map(|k| k.provider_id()).collect();
        assert_eq!(keys, ids);
        // The allow-list in `parse` keeps every registry id and drops unknown ones.
        let quoted: Vec<String> = ids.iter().map(|i| format!("\"{i}\"")).collect();
        let text = format!(
            "[ai]
providers = [{}, \"bogus\"]
",
            quoted.join(", ")
        );
        assert_eq!(owned(&parse(&text).unwrap().ai.providers), ids);
    }

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

    /// Broken files never crash: every unreadable or invalid file yields the defaults plus an error
    /// (shown as a toast), and a missing file gets the template.
    #[test]
    fn load_reports_broken_files() {
        let dir = std::env::temp_dir().join(format!("noble-cfg-load-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let cases: [(&[u8], bool); 7] = [
            (b"[general\ntheme=", true),
            (b"[general]\ntheme = 5\n", true),
            (b"[general]\ntheme = \"\xff\xfe\"\n", true),
            (b"\xef\xbb\xbf[general]\ntheme = \"ice\"\n", false),
            (b"[general]\ntheme = \"ice\"\n[general]\n", true),
            (b"\0\0\0", true),
            (b"", false),
        ];
        for (bytes, broken) in cases {
            std::fs::write(&path, bytes).unwrap();
            let loaded = load(&path);
            assert_eq!(loaded.error.is_some(), broken, "{:?}: {:?}", String::from_utf8_lossy(bytes), loaded.error);
        }
        // The line (and column) of a TOML mistake stay in the message, so the toast can open the file there.
        let at = |text: &str| parse(text).err().map(|e| (e.line, e.text));
        assert_eq!(
            at("[general]\ntheme = \"ice\"\n\n[ai]\nenabled = \"yes\"\n"),
            Some((Some(5), "config line 5, column 11: invalid type: string \"yes\", expected a boolean".into()))
        );
        let (line, text) = at("[general\ntheme=").unwrap();
        assert_eq!(line, Some(1));
        assert!(text.starts_with("config line 1, column ") && !text.contains('\n'), "{text}");
        std::fs::remove_file(&path).unwrap();
        let loaded = load(&path);
        assert!(loaded.error.is_none() && path.is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A " #" inside a quoted value is not a comment: the old value must not leak into the new line.
    #[test]
    fn set_value_hash_inside_quotes() {
        let text = "[general]\noperator = \"team #1\"   # greeting name\nshell = 'C:\\x #y'\n";
        let out = set_value(text, "general", "operator", "\"bob\"");
        assert!(out.contains("operator = \"bob\""), "{out}");
        assert!(out.contains("# greeting name") && !out.contains("#1"), "{out}");
        assert_eq!(parse(&out).unwrap().general.operator, "bob");
        let out = set_value(text, "general", "shell", "\"sh\"");
        assert!(!out.contains("#y"), "{out}");
        assert_eq!(parse(&out).unwrap().terminal.shell, "", "shell lives in [terminal]");
        // No comment at all: only the new value remains.
        let out = set_value("[general]\noperator = \"a #b\"\n", "general", "operator", "\"c\"");
        assert_eq!(out, "[general]\noperator = \"c\"\n");
    }

    /// A section header with a trailing comment is still the section (no duplicate table).
    #[test]
    fn set_value_header_with_comment() {
        let text = "[general]   # look\ntheme = \"ice\"\n\n[ai]\n";
        let out = set_value(text, "general", "theme", "\"synth\"");
        assert_eq!(parse(&out).map(|c| c.general.theme), Ok("synth".into()), "{out}");
        assert_eq!(out.matches("[general]").count(), 1, "{out}");
        let out = set_value(text, "general", "clock_24h", "false");
        assert!(parse(&out).is_ok_and(|c| !c.general.clock_24h), "{out}");
        // The same for launcher blocks: the commented block is replaced, not kept next to the new ones.
        let text = "[[launchers]]  # mine\nkey = \"c\"\nname = \"a\"\ncommand = \"a\"\n";
        let list = default_launchers();
        let out = set_launchers(text, &list);
        assert_eq!(parse(&out).map(|c| c.launchers), Ok(list), "{out}");
    }

    #[test]
    fn read_for_edit_never_replaces_unreadable_files() {
        let dir = std::env::temp_dir().join(format!("noble-cfg-edit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        assert_eq!(read_for_edit(&path).as_deref(), Ok(DEFAULT_CONFIG));
        std::fs::write(&path, b"# caf\xe9\n[general]\n").unwrap();
        assert!(read_for_edit(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Random config texts: `set_value` never panics, and a file that parsed before still parses after.
    #[test]
    fn set_value_fuzz_keeps_valid_files_valid() {
        const LINES: [&str; 18] = [
            "[general]",
            "[general] # c",
            "[terminal]",
            "[keys]",
            "[keys.prefix_bindings]",
            "# comment with = sign",
            "",
            "   ",
            "theme = \"ice\"",
            "theme = \"a #b\" # c",
            "operator = 'x # y'",
            "operator = \"é日🙂\"   # ünïcode",
            "shell = \"\"",
            "\"%\" = \"split_right\"",
            "notify_after = 10",
            "clock_24h = true#tight",
            "\t theme = \"tab\"",
            "[[launchers]]",
        ];
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let targets = [("general", "theme"), ("general", "operator"), ("terminal", "shell"), ("keys", "prefix")];
        for _ in 0..3000 {
            let n = next() % 8;
            let text: Vec<&str> = (0..n).map(|_| LINES[(next() % LINES.len() as u64) as usize]).collect();
            let text = text.join("\n");
            let (section, key) = targets[(next() % targets.len() as u64) as usize];
            let out = set_value(&text, section, key, "\"v #1\"");
            if parse(&text).is_ok() {
                assert!(parse(&out).is_ok(), "{text:?}\n→\n{out}");
            }
        }
    }

    /// A multi-line array value is replaced as a whole: no orphan continuation lines stay behind.
    #[test]
    fn set_value_replaces_multiline_arrays() {
        let text = "[projects]
roots = [
  \"~/a\",   # first ] tricky
  \"~/b]\",
]  # tail
max_depth = 3

[general]
theme = \"ice\"
";
        let out = set_value(text, "projects", "roots", "[\"~/x\"]");
        assert_eq!(out.matches("~/a").count(), 0, "{out}");
        assert!(out.contains("roots = [\"~/x\"]") && out.contains("max_depth = 3"), "{out}");
        assert_eq!(parse(&out).map(|c| c.projects.max_depth), Ok(3), "{out}");
        assert_eq!(strip_value("roots = [\"a=b\"]"), " [\"a=b\"]");
    }

    /// A CRLF file stays CRLF, and a comment titling the next table stays with that table.
    #[test]
    fn set_binding_keeps_line_endings_and_next_section_comment() {
        let crlf = "[keys.prefix_bindings]\r\n\"v\" = \"zoom\"\r\n\r\n[terminal]\r\n";
        let out = set_binding(crlf, "keys.prefix_bindings", "%", "help").unwrap();
        assert_eq!(out.matches('\n').count(), out.matches("\r\n").count(), "{out:?}");
        assert!(out.contains("\"%\" = \"help\"\r\n\r\n[terminal]"), "{out:?}");
        let text = "[keys.prefix_bindings]\n\"v\" = \"zoom\"\n# --- terminal ---\n[terminal]\n";
        let out = set_binding(text, "keys.prefix_bindings", "%", "help").unwrap();
        assert!(out.contains("\"%\" = \"help\"\n# --- terminal ---\n[terminal]"), "{out}");
    }

    #[test]
    fn set_binding_writes_into_its_table_and_keeps_the_rest() {
        let text = "[keys]\nprefix = \"ctrl+a\"   # the prefix\n\n[keys.prefix_bindings]\n# \"%\" = \"zoom\"\n\"v\" = \"split_right\"  # old\n\n[terminal]\nscrollback = 5000\n";
        // A chord with a line is rewritten in place, its comment kept.
        let out = set_binding(text, "keys.prefix_bindings", "v", "none").unwrap();
        assert!(out.contains("\"v\" = \"none\"  # old\n"), "{out}");
        assert!(out.contains("prefix = \"ctrl+a\"   # the prefix\n"), "{out}");
        assert!(out.contains("# \"%\" = \"zoom\"\n"), "a commented example stays: {out}");
        assert!(out.contains("scrollback = 5000\n"), "{out}");
        // A new chord goes to the end of its table, before the blank line that separates the next one.
        let out = set_binding(&out, "keys.prefix_bindings", "%", "split_right").unwrap();
        assert!(out.contains("\"v\" = \"none\"  # old\n\"%\" = \"split_right\"\n\n[terminal]"), "{out}");
        // Another spelling of a line's chord ("Ctrl+A" for "ctrl+a") replaces that line.
        let with_line = "[keys.prefix_bindings]\n\"ctrl+a\" = \"zoom\"\n";
        let out2 = set_binding(with_line, "keys.prefix_bindings", "Ctrl+A", "help").unwrap();
        assert_eq!(out2, "[keys.prefix_bindings]\n\"Ctrl+A\" = \"help\"\n", "{out2}");
        // A missing table is created at the end of the file.
        let out = set_binding("[general]\ntheme = \"amber\"\n", "keys.direct_bindings", "alt+t", "new_tab").unwrap();
        assert_eq!(out, "[general]\ntheme = \"amber\"\n\n[keys.direct_bindings]\n\"alt+t\" = \"new_tab\"\n");
        // Keys that TOML would read as syntax are quoted properly and read back.
        let out = set_binding("", "keys.prefix_bindings", "=", "zoom").unwrap();
        let out = set_binding(&out, "keys.prefix_bindings", "\"", "help").unwrap();
        let parsed = parse(&out).expect("valid config");
        assert_eq!(parsed.keys.prefix_bindings.get("="), Some(&"zoom".to_string()));
        assert_eq!(parsed.keys.prefix_bindings.get("\""), Some(&"help".to_string()));
        // Writing the value a line already has changes nothing.
        assert_eq!(set_binding(text, "keys.prefix_bindings", "v", "split_right").unwrap(), text);
    }

    #[test]
    fn remove_binding_restores_the_default_line_away() {
        let text = "[keys.direct_bindings]\n\"alt+t\" = \"zoom\"\n\"alt+x\" = \"none\"\n\n[ai]\nenabled = true\n";
        let out = remove_binding(text, "keys.direct_bindings", "alt+t").unwrap();
        assert_eq!(out, "[keys.direct_bindings]\n\"alt+x\" = \"none\"\n\n[ai]\nenabled = true\n");
        // Nothing to remove: the text is returned as it was (line endings included).
        let crlf = "[keys.direct_bindings]\r\n\"alt+x\" = \"none\"\r\n";
        assert_eq!(remove_binding(crlf, "keys.direct_bindings", "alt+t").unwrap(), crlf);
        assert_eq!(remove_binding(crlf, "keys.prefix_bindings", "alt+t").unwrap(), crlf);
    }

    #[test]
    fn binding_edits_accept_spaced_headers_and_refuse_other_forms() {
        // Whitespace around the brackets and the dots names the same table.
        let spaced = "[ keys . prefix_bindings ]\n\"v\" = \"zoom\"\n";
        assert_eq!(
            set_binding(spaced, "keys.prefix_bindings", "v", "none").unwrap(),
            "[ keys . prefix_bindings ]\n\"v\" = \"none\"\n"
        );
        // An inline table or dotted keys already hold the table: a second header would duplicate it.
        let inline = "[keys]\nprefix_bindings = { v = \"zoom\" }\n";
        assert!(set_binding(inline, "keys.prefix_bindings", "x", "zoom").is_err());
        let dotted = "[keys]\nprefix_bindings.\"v\" = \"zoom\"\n";
        assert!(remove_binding(dotted, "keys.prefix_bindings", "v").is_err());
        let root_dotted = "keys.direct_bindings.\"alt+t\" = \"zoom\"\n";
        assert!(set_binding(root_dotted, "keys.direct_bindings", "alt+x", "zoom").is_err());
        // An edit that leaves an invalid config is not returned: `keys` is an inline table here.
        let inline_keys = "keys = { prefix = \"ctrl+a\" }\n";
        assert!(set_binding(inline_keys, "keys.prefix_bindings", "v", "zoom").is_err());
        // A file that does not parse is refused too.
        assert!(set_binding("[keys\n", "keys.prefix_bindings", "v", "zoom").is_err());
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
