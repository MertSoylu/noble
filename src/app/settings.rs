//! Settings tab: selectable items, value changes and persistent writes to the config.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use std::path::{Path, PathBuf};

use super::ops::DIR_PROBE_TIMEOUT;
use super::{AddMode, App, ToastLevel, View};
use crate::event::AppEvent;
use crate::theme::THEMES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingKey {
    Theme,
    Transparent,
    Boot,
    Animations,
    AnimBattery,
    Clock24,
    Seconds,
    Tips,
    Shell,
    Restore,
    CopySelect,
    TabFollowsCwd,
    DimInactive,
    TermColors,
    Notify,
    Prefix,
    Passthrough,
    ShellFirst,
    KeyBindings,
    AiEnabled,
    Claude,
    Codex,
    Antigravity,
    QuickLaunch,
    OpenCodeGo,
    Kilo,
    CommandCode,
    AiRefresh,
    AiWarn,
    ClaudeHooks,
    Updates,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingItem {
    Setting(SettingKey),
    OpenConfig,
    ReloadConfig,
}

impl SettingItem {
    /// The row's name; the Settings filter matches it.
    pub fn label(&self) -> &'static str {
        match self {
            SettingItem::Setting(key) => key.label(),
            SettingItem::OpenConfig => "Edit config file",
            SettingItem::ReloadConfig => "Reload config",
        }
    }
}

pub const PREFIXES: [&str; 4] = ["ctrl+a", "ctrl+b", "ctrl+space", "ctrl+g"];
pub const REFRESH_MINUTES: [u64; 4] = [1, 5, 15, 30];
/// Quota warning thresholds (0 = off).
pub const WARN_PERCENTS: [u8; 4] = [0, 80, 90, 95];
pub use crate::config::{LAUNCH_KEYS, PASSTHROUGH_MODES};

/// AI provider ids (Settings order).
pub const PROVIDER_KEYS: [SettingKey; 6] = [
    SettingKey::Claude,
    SettingKey::Codex,
    SettingKey::Antigravity,
    SettingKey::OpenCodeGo,
    SettingKey::Kilo,
    SettingKey::CommandCode,
];

impl SettingKey {
    pub fn label(&self) -> &'static str {
        match self {
            SettingKey::Theme => "Theme",
            SettingKey::Transparent => "Transparent background",
            SettingKey::Boot => "Boot animation",
            SettingKey::Animations => "Animations",
            SettingKey::AnimBattery => "Animations on battery",
            SettingKey::Clock24 => "24-hour clock",
            SettingKey::Tips => "Show tips",
            SettingKey::Seconds => "Show seconds",
            SettingKey::Shell => "Shell",
            SettingKey::Restore => "Restore tabs on launch",
            SettingKey::CopySelect => "Copy text on select",
            SettingKey::TabFollowsCwd => "Tab name follows the folder",
            SettingKey::DimInactive => "Dim inactive panes",
            SettingKey::Notify => "Notify from background tabs",
            SettingKey::TermColors => "Terminal colors",
            SettingKey::Prefix => "Prefix key",
            SettingKey::Passthrough => "Pass keys to the app",
            SettingKey::ShellFirst => "Leave shell keys to the shell",
            SettingKey::KeyBindings => "Key bindings",
            SettingKey::AiEnabled => "Show AI usage",
            SettingKey::Claude => "Claude Code",
            SettingKey::Codex => "Codex",
            SettingKey::Antigravity => "Antigravity",
            SettingKey::QuickLaunch => "Quick launch",
            SettingKey::OpenCodeGo => "OpenCode Go",
            SettingKey::Kilo => "Kilo Code",
            SettingKey::CommandCode => "Command Code",
            SettingKey::AiRefresh => "Refresh every",
            SettingKey::AiWarn => "Warn when usage reaches",
            SettingKey::ClaudeHooks => "Claude Code status hooks",
            SettingKey::Updates => "Check for updates",
        }
    }

    /// One short line about the setting, shown under the list while it is selected.
    pub fn description(&self) -> &'static str {
        match self {
            SettingKey::Theme => "Color theme of the whole interface",
            SettingKey::Transparent => "Let the terminal's own background show through",
            SettingKey::Boot => "Startup animation on the first launch and after an update",
            SettingKey::Animations => "Slide, zoom, fade and pulse effects",
            SettingKey::AnimBattery => "Keep animations while running on battery",
            SettingKey::Clock24 => "24-hour instead of AM/PM clock on Home",
            SettingKey::Seconds => "Show seconds on the Home clock (off on battery)",
            SettingKey::Tips => "Hints about the current screen in the status bar",
            SettingKey::Shell => "Program that new terminal panes start",
            SettingKey::Restore => "Reopen the tabs of the last session on launch",
            SettingKey::CopySelect => "Copy a mouse selection as soon as you release it",
            SettingKey::TabFollowsCwd => "Name tabs after the folder the shell is in",
            SettingKey::DimInactive => "Fade the text of the split panes without the focus",
            SettingKey::Notify => "Alert when a long command ends in a background tab",
            SettingKey::TermColors => "Color scheme used inside terminal panes",
            SettingKey::Prefix => "Key that starts NOBLE's command chords",
            SettingKey::Passthrough => "Which shortcuts full-screen apps receive",
            SettingKey::ShellFirst => "Keep editing keys such as ctrl+w for the shell",
            SettingKey::KeyBindings => "Change the prefix and direct keys of every action",
            SettingKey::AiEnabled => "Show AI quota cards on Home",
            SettingKey::Claude | SettingKey::Codex | SettingKey::Antigravity => "Show this provider's usage on Home",
            SettingKey::OpenCodeGo | SettingKey::Kilo | SettingKey::CommandCode => "Show this provider's usage on Home",
            SettingKey::QuickLaunch => "Choose the AI CLIs offered as launchers",
            SettingKey::AiRefresh => "How often usage is fetched while Home is open",
            SettingKey::AiWarn => "Warn when a quota reaches this percentage",
            SettingKey::ClaudeHooks => "Show Claude Code's live state on its tab",
            SettingKey::Updates => "Look for a newer NOBLE release once a day",
        }
    }

    /// The provider id in `ai::providers` ("" when it is not a provider).
    pub fn provider_id(&self) -> &'static str {
        match self {
            SettingKey::Claude => "claude",
            SettingKey::Codex => "codex",
            SettingKey::Antigravity => "antigravity",
            SettingKey::OpenCodeGo => "opencode-go",
            SettingKey::Kilo => "kilo",
            SettingKey::CommandCode => "command-code",
            _ => "",
        }
    }

    /// The on/off setting this one only matters under: drawn indented, and dimmed while it is off.
    pub fn parent(&self) -> Option<SettingKey> {
        let under_ai = PROVIDER_KEYS.contains(self) || matches!(self, SettingKey::AiRefresh | SettingKey::AiWarn);
        if *self == SettingKey::AnimBattery {
            return Some(SettingKey::Animations);
        }
        under_ai.then_some(SettingKey::AiEnabled)
    }

    /// Set in a popup window (⏎); ←→ do not change it.
    pub fn opens_popup(&self) -> bool {
        matches!(self, SettingKey::QuickLaunch | SettingKey::KeyBindings)
    }

    pub fn is_toggle(&self) -> bool {
        !matches!(
            self,
            SettingKey::Theme
                | SettingKey::Shell
                | SettingKey::Prefix
                | SettingKey::Passthrough
                | SettingKey::TermColors
                | SettingKey::QuickLaunch
                | SettingKey::KeyBindings
                | SettingKey::AiRefresh
                | SettingKey::AiWarn
        )
    }
}

/// Shell options found on this machine ("" = automatic).
pub fn shell_options() -> Vec<String> {
    let names: &[&str] =
        if cfg!(windows) { &["pwsh", "powershell", "cmd"] } else { &["bash", "zsh", "fish", "pwsh", "nu"] };
    let mut v = vec![String::new()];
    v.extend(names.iter().filter(|n| crate::util::which(n).is_some()).map(|n| n.to_string()));
    if cfg!(windows) {
        // Git Bash is rarely on PATH (and `bash` there may be the WSL launcher): use its full path.
        let git = std::env::var_os("ProgramFiles").map(|p| std::path::PathBuf::from(p).join(r"Git\bin\bash.exe"));
        if let Some(git) = git.filter(|p| p.is_file()) {
            v.push(git.display().to_string().replace('\\', "/"));
        }
    }
    v
}

fn cycle<T: PartialEq + Clone>(options: &[T], current: &T, dir: i32) -> T {
    let i = options.iter().position(|o| o == current).unwrap_or(0) as i32;
    options[(i + dir).rem_euclid(options.len() as i32) as usize].clone()
}

/// Whether `path` holds repositories (`probe`), answered within `DIR_PROBE_TIMEOUT`: a folder that does
/// not answer in time (a network share that stopped responding) counts as holding none, so it is added
/// as a project. The probe runs on its own thread, which is left to finish when the answer comes late
/// (as `util::usable_dirs` does).
fn holds_repos_in_time(path: PathBuf, probe: fn(&Path, &[String]) -> bool, exclude: Vec<String>) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("add-probe".into()).spawn(move || {
        let _ = tx.send(probe(&path, &exclude));
    });
    spawned.is_ok() && rx.recv_timeout(DIR_PROBE_TIMEOUT).unwrap_or(false)
}

impl App {
    /// The page's sections and their items, in display order, narrowed by the filter (`/`); a
    /// section with no matching row is left out. The drawing and the selection use the same list.
    pub fn settings_sections(&self) -> Vec<(&'static str, Vec<SettingItem>)> {
        let q = self.settings_filter.trim().to_lowercase();
        self.settings_layout()
            .into_iter()
            .map(|(title, items)| {
                let items: Vec<SettingItem> =
                    items.into_iter().filter(|i| q.is_empty() || i.label().to_lowercase().contains(&q)).collect();
                (title, items)
            })
            .filter(|(_, items)| !items.is_empty())
            .collect()
    }

    /// Every selectable item of the page, ignoring the filter (the palette lists them all).
    pub fn all_settings_items(&self) -> Vec<SettingItem> {
        self.settings_layout().into_iter().flat_map(|(_, items)| items).collect()
    }

    /// The page's sections and their items before filtering, in display order.
    fn settings_layout(&self) -> Vec<(&'static str, Vec<SettingItem>)> {
        use SettingKey::*;
        let rows = |keys: &[SettingKey]| keys.iter().map(|k| SettingItem::Setting(*k)).collect::<Vec<_>>();
        let mut terminal =
            rows(&[Shell, TermColors, Restore, CopySelect, TabFollowsCwd, DimInactive, Notify, QuickLaunch]);
        if self.claude_hooks_available() {
            terminal.push(SettingItem::Setting(ClaudeHooks));
        }
        let mut ai = rows(&[AiEnabled]);
        // A provider that is not installed is hidden anyway; toggling it is meaningless.
        ai.extend(rows(&PROVIDER_KEYS).into_iter().filter(|i| match i {
            SettingItem::Setting(k) => self.ai_installed.contains(&k.provider_id()),
            _ => false,
        }));
        ai.extend(rows(&[AiRefresh, AiWarn]));
        let mut general = rows(&[Updates]);
        general.extend([SettingItem::OpenConfig, SettingItem::ReloadConfig]);
        let mut display = rows(&[Theme, Transparent, Boot, Animations]);
        // Only a laptop runs on battery.
        if self.sensors.battery().is_some() {
            display.push(SettingItem::Setting(AnimBattery));
        }
        display.extend(rows(&[Clock24, Seconds, Tips]));
        vec![
            ("Display", display),
            ("Terminal", terminal),
            ("Keys", rows(&[Prefix, Passthrough, ShellFirst, KeyBindings])),
            ("AI usage", ai),
            ("General", general),
        ]
    }

    /// Selectable items on the page, in display order (after the filter).
    pub fn settings_items(&self) -> Vec<SettingItem> {
        self.settings_sections().into_iter().flat_map(|(_, items)| items).collect()
    }

    /// Opens Settings; esc goes back to the page it was opened from.
    pub fn open_settings(&mut self) {
        if self.view != View::Settings {
            let pane = match self.view {
                View::Term(i) => self.tabs.get(i).map(|t| t.focus),
                _ => None,
            };
            self.settings_back = Some((self.view, pane));
        }
        self.view = View::Settings;
        self.settings_follow = true;
        self.settings_filter.clear();
        self.settings_filtering = false;
    }

    /// Leaves Settings for the page it was opened from (a tab is found again by its pane,
    /// since tabs may have moved or closed meanwhile), Home otherwise.
    pub fn close_settings(&mut self) {
        self.settings_filter.clear();
        self.settings_filtering = false;
        self.view = match self.settings_back.take() {
            Some((View::System, _)) => View::System,
            Some((View::Term(_), Some(pane))) => self.tab_of(pane).map_or(View::Bridge, View::Term),
            _ => View::Bridge,
        };
    }

    pub fn setting_on(&self, key: SettingKey) -> bool {
        let c = &self.cfg;
        let has = |id: &str| c.ai.providers.iter().any(|p| p.eq_ignore_ascii_case(id));
        match key {
            SettingKey::Transparent => c.general.transparent,
            SettingKey::Boot => c.general.boot_animation,
            SettingKey::Animations => c.general.animations,
            SettingKey::AnimBattery => c.general.animations_on_battery,
            SettingKey::Clock24 => c.general.clock_24h,
            SettingKey::Seconds => c.general.show_seconds,
            SettingKey::Tips => c.general.show_tips,
            SettingKey::Updates => c.general.check_updates,
            SettingKey::Restore => c.terminal.restore_session,
            SettingKey::CopySelect => c.terminal.copy_on_select,
            SettingKey::TabFollowsCwd => c.terminal.tab_follows_cwd,
            SettingKey::DimInactive => c.terminal.dim_inactive,
            SettingKey::ShellFirst => c.keys.shell_first,
            SettingKey::Notify => c.terminal.notify,
            SettingKey::ClaudeHooks => self.hooks_installed,
            SettingKey::AiEnabled => c.ai.enabled,
            SettingKey::Claude => has("claude"),
            SettingKey::Codex => has("codex"),
            SettingKey::Antigravity => has("antigravity"),
            SettingKey::OpenCodeGo => has("opencode-go"),
            SettingKey::Kilo => has("kilo"),
            SettingKey::CommandCode => has("command-code"),
            _ => false,
        }
    }

    /// Displayed value of a cycling setting.
    pub fn setting_value(&self, key: SettingKey) -> String {
        match key {
            SettingKey::Shell => {
                let shell = self.cfg.terminal.shell.trim();
                if shell.is_empty() {
                    format!("auto ({})", self.shell.label())
                } else if shell.contains(['/', '\\']) {
                    // A full path (Git Bash): the program name is enough.
                    self.shell.label()
                } else {
                    shell.to_string()
                }
            }
            SettingKey::Theme => self.theme.label.to_string(),
            SettingKey::Prefix => self.cfg.keys.prefix.clone(),
            SettingKey::KeyBindings => {
                let custom = !self.cfg.keys.prefix_bindings.is_empty() || !self.cfg.keys.direct_bindings.is_empty();
                if custom { "custom".into() } else { "defaults".into() }
            }
            SettingKey::Passthrough => {
                let key = self.keymap.hint(crate::keys::Action::Passthrough).unwrap_or_default();
                if self.pass_once() { format!("once ({} + key)", self.keymap.prefix) } else { format!("lock ({key})") }
            }
            SettingKey::AiRefresh => format!("{} min", self.cfg.ai.refresh_minutes),
            SettingKey::TermColors => self.scheme_label(&self.cfg.terminal.colors),
            SettingKey::QuickLaunch => {
                let installed = self.installed_launchers();
                let shown = installed.iter().filter(|i| self.launchers[**i].0.show).count();
                if installed.is_empty() {
                    "none installed".into()
                } else {
                    format!("{shown} of {} shown", installed.len())
                }
            }
            SettingKey::AiWarn => match self.cfg.ai.warn_at {
                0 => "Off".into(),
                p => format!("{p}%"),
            },
            k => if self.setting_on(k) { "On" } else { "Off" }.to_string(),
        }
    }

    /// Updates a single value in the config file, preserving its comments.
    pub(super) fn persist(&mut self, section: &str, key: &str, value_toml: &str) {
        if self.services.is_none() {
            return;
        }
        let text = match crate::config::read_for_edit(&self.paths.config) {
            Ok(text) => text,
            Err(e) => return self.toast(ToastLevel::Error, e),
        };
        let updated = crate::config::set_value(&text, section, key, value_toml);
        self.write_config(updated);
    }

    /// Writes the edited config; a failure is reported (the change then only lasts this session).
    /// `false` when the file could not be written (the error is shown as a toast).
    pub(super) fn write_config(&mut self, text: String) -> bool {
        match crate::store::write_text(&self.paths.config, &text) {
            Ok(()) => {
                self.cfg_mtime = crate::config::mtime_of(&self.paths.config);
                true
            }
            Err(e) => {
                let path = crate::util::tilde(&self.paths.config);
                self.toast(ToastLevel::Error, format!("could not save {path}: {e}"));
                false
            }
        }
    }

    /// Writes the launcher list to the config (`[[launchers]]` blocks are rebuilt).
    fn persist_launchers(&mut self) {
        if self.services.is_none() {
            return;
        }
        let text = match crate::config::read_for_edit(&self.paths.config) {
            Ok(text) => text,
            Err(e) => return self.toast(ToastLevel::Error, e),
        };
        let updated = crate::config::set_launchers(&text, &self.cfg.launchers);
        self.write_config(updated);
    }

    /// Shows/hides a launcher or flips its shortcut to the next key
    /// that no other launcher uses.
    pub fn change_launcher(&mut self, idx: usize, key: bool, dir: i32) {
        let mut c = self.cfg.clone();
        if idx >= c.launchers.len() {
            return;
        }
        if key {
            let taken: Vec<String> =
                c.launchers.iter().enumerate().filter(|(j, _)| *j != idx).map(|(_, l)| l.key.clone()).collect();
            let free: Vec<String> = LAUNCH_KEYS.iter().map(|k| k.to_string()).filter(|k| !taken.contains(k)).collect();
            let cur = c.launchers[idx].key.clone();
            let next = if free.contains(&cur) { cycle(&free, &cur, dir) } else { free.first().cloned().unwrap_or(cur) };
            c.launchers[idx].key = next;
        } else {
            c.launchers[idx].show ^= true;
        }
        self.apply_config(c);
        self.persist_launchers();
    }

    /// Applies an item: picks a theme, toggles a key or cycles a value.
    pub fn activate_setting(&mut self, item: SettingItem, dir: i32) {
        match item {
            SettingItem::Setting(SettingKey::QuickLaunch) => self.open_launcher_picker(),
            SettingItem::Setting(SettingKey::KeyBindings) => self.open_bindings(),
            SettingItem::Setting(SettingKey::ClaudeHooks) => self.toggle_claude_hooks(),
            SettingItem::OpenConfig => self.run(crate::keys::Action::OpenConfig),
            SettingItem::ReloadConfig => self.reload_config(true),
            SettingItem::Setting(key) => {
                let mut c = self.cfg.clone();
                let (section, name, value) = match key {
                    // Set from the popup (handled above).
                    SettingKey::QuickLaunch => return self.open_launcher_picker(),
                    SettingKey::KeyBindings => return self.open_bindings(),
                    // ←→ step through the themes; `set_theme` saves it.
                    SettingKey::Theme => {
                        let names: Vec<&str> = THEMES.iter().map(|t| t.name).collect();
                        return self.set_theme(cycle(&names, &self.theme.name, dir));
                    }
                    SettingKey::Transparent => {
                        c.general.transparent ^= true;
                        ("general", "transparent", c.general.transparent.to_string())
                    }
                    SettingKey::Boot => {
                        c.general.boot_animation ^= true;
                        ("general", "boot_animation", c.general.boot_animation.to_string())
                    }
                    SettingKey::Animations => {
                        c.general.animations ^= true;
                        ("general", "animations", c.general.animations.to_string())
                    }
                    SettingKey::AnimBattery => {
                        c.general.animations_on_battery ^= true;
                        ("general", "animations_on_battery", c.general.animations_on_battery.to_string())
                    }
                    SettingKey::Tips => {
                        c.general.show_tips ^= true;
                        ("general", "show_tips", c.general.show_tips.to_string())
                    }
                    SettingKey::Clock24 => {
                        c.general.clock_24h ^= true;
                        ("general", "clock_24h", c.general.clock_24h.to_string())
                    }
                    SettingKey::Seconds => {
                        c.general.show_seconds ^= true;
                        ("general", "show_seconds", c.general.show_seconds.to_string())
                    }
                    SettingKey::Updates => {
                        c.general.check_updates ^= true;
                        ("general", "check_updates", c.general.check_updates.to_string())
                    }
                    SettingKey::Restore => {
                        c.terminal.restore_session ^= true;
                        ("terminal", "restore_session", c.terminal.restore_session.to_string())
                    }
                    SettingKey::CopySelect => {
                        c.terminal.copy_on_select ^= true;
                        ("terminal", "copy_on_select", c.terminal.copy_on_select.to_string())
                    }
                    SettingKey::TabFollowsCwd => {
                        c.terminal.tab_follows_cwd ^= true;
                        ("terminal", "tab_follows_cwd", c.terminal.tab_follows_cwd.to_string())
                    }
                    SettingKey::DimInactive => {
                        c.terminal.dim_inactive ^= true;
                        ("terminal", "dim_inactive", c.terminal.dim_inactive.to_string())
                    }
                    // Written to Claude's own settings file, not the config (handled above).
                    SettingKey::ClaudeHooks => return self.toggle_claude_hooks(),
                    SettingKey::TermColors => {
                        let names = self.scheme_options();
                        let cur = names
                            .iter()
                            .find(|n| n.eq_ignore_ascii_case(c.terminal.colors.trim()))
                            .cloned()
                            .unwrap_or_default();
                        c.terminal.colors = cycle(&names, &cur, dir);
                        ("terminal", "colors", format!("\"{}\"", c.terminal.colors))
                    }
                    SettingKey::Notify => {
                        c.terminal.notify ^= true;
                        ("terminal", "notify", c.terminal.notify.to_string())
                    }
                    SettingKey::AiWarn => {
                        c.ai.warn_at = cycle(&WARN_PERCENTS, &c.ai.warn_at, dir);
                        ("ai", "warn_at", c.ai.warn_at.to_string())
                    }
                    SettingKey::AiEnabled => {
                        c.ai.enabled ^= true;
                        ("ai", "enabled", c.ai.enabled.to_string())
                    }
                    SettingKey::Claude
                    | SettingKey::Codex
                    | SettingKey::Antigravity
                    | SettingKey::OpenCodeGo
                    | SettingKey::Kilo
                    | SettingKey::CommandCode => {
                        let id = key.provider_id();
                        if c.ai.providers.iter().any(|p| p.eq_ignore_ascii_case(id)) {
                            c.ai.providers.retain(|p| !p.eq_ignore_ascii_case(id));
                        } else {
                            c.ai.providers.push(id.into());
                        }
                        let list = c.ai.providers.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join(", ");
                        ("ai", "providers", format!("[{list}]"))
                    }
                    SettingKey::AiRefresh => {
                        c.ai.refresh_minutes = cycle(&REFRESH_MINUTES, &c.ai.refresh_minutes, dir);
                        ("ai", "refresh_minutes", c.ai.refresh_minutes.to_string())
                    }
                    SettingKey::Shell => {
                        c.terminal.shell = cycle(&shell_options(), &c.terminal.shell, dir);
                        let escaped = c.terminal.shell.replace('\\', "\\\\").replace('"', "\\\"");
                        ("terminal", "shell", format!("\"{escaped}\""))
                    }
                    SettingKey::Prefix => {
                        let cur = c.keys.prefix.clone();
                        c.keys.prefix = cycle(&PREFIXES.map(String::from), &cur, dir);
                        ("keys", "prefix", format!("\"{}\"", c.keys.prefix))
                    }
                    SettingKey::ShellFirst => {
                        c.keys.shell_first ^= true;
                        ("keys", "shell_first", c.keys.shell_first.to_string())
                    }
                    SettingKey::Passthrough => {
                        let cur = if self.pass_once() { "once" } else { "lock" }.to_string();
                        c.keys.passthrough = cycle(&PASSTHROUGH_MODES.map(String::from), &cur, dir);
                        // A lock would stay on with no way to see it change back: release them all.
                        for p in self.panes.values_mut() {
                            p.passthrough = false;
                        }
                        ("keys", "passthrough", format!("\"{}\"", c.keys.passthrough))
                    }
                };
                self.apply_config(c);
                self.persist(section, name, &value);
                if key == SettingKey::Shell {
                    self.toast(ToastLevel::Info, "shell applies to new tabs");
                }
            }
        }
    }

    /// The hooks row is shown only when Claude Code is installed, or when our hooks
    /// are still in its settings (so they can be removed after an uninstall).
    pub fn claude_hooks_available(&self) -> bool {
        self.hooks_installed || self.ai_installed.contains(&"claude")
    }

    /// Adds or removes the Claude Code hooks in `~/.claude/settings.json`.
    /// In headless mode (tests) the real file is never touched.
    pub fn toggle_claude_hooks(&mut self) {
        let Some(path) = crate::hooks::settings_path().filter(|_| self.services.is_some()) else {
            self.toast(ToastLevel::Warn, "Claude Code settings are not available here");
            return;
        };
        let result = if self.hooks_installed {
            crate::hooks::uninstall(&path)
        } else {
            crate::hooks::install(&path, &crate::hooks::command_base())
        };
        match result {
            Ok(()) => {
                self.hooks_installed = crate::hooks::is_installed(&path);
                let msg = if self.hooks_installed {
                    "Claude Code hooks installed · applies to new Claude sessions"
                } else {
                    "Claude Code hooks removed"
                };
                self.toast(ToastLevel::Ok, msg);
            }
            Err(e) => self.toast(ToastLevel::Error, e),
        }
    }

    /// Changes the prefix key and writes it to the config.
    pub fn set_prefix(&mut self, prefix: &str) {
        let mut c = self.cfg.clone();
        c.keys.prefix = prefix.to_string();
        self.apply_config(c);
        self.persist("keys", "prefix", &format!("\"{prefix}\""));
    }

    /// Opens the welcome screen with the current settings selected.
    pub fn show_welcome(&mut self) {
        self.overlay = Some(super::Overlay::Welcome(self.welcome_setup()));
    }

    /// The welcome setup for the current settings (also rebuilt when the config file is
    /// reloaded while the card is open, so its previews never outlive the file's values).
    fn welcome_setup(&self) -> super::WelcomeSetup {
        let prefix = PREFIXES.iter().position(|p| p.eq_ignore_ascii_case(self.cfg.keys.prefix.trim())).unwrap_or(0);
        let theme = THEMES.iter().position(|t| t.name == self.theme.name).unwrap_or(0);
        let current = self.cfg.terminal.colors.clone();
        // Same row as the scheme selector: 0 is "Follow theme".
        let colors = self
            .term_schemes
            .iter()
            .position(|s| crate::theme::find_scheme(std::slice::from_ref(s), &current).is_some())
            .map(|i| i + 1)
            .unwrap_or(0);
        let mut shells = shell_options();
        // A shell set in the config that is not in the list stays selectable (and selected), so
        // ⏎ never replaces it with "auto".
        if !shells.contains(&self.cfg.terminal.shell) {
            shells.push(self.cfg.terminal.shell.clone());
        }
        let shell = shells.iter().position(|s| *s == self.cfg.terminal.shell).unwrap_or(0);
        super::WelcomeSetup {
            row: 0,
            theme,
            colors,
            shell,
            prefix,
            schemes: self.scheme_options(),
            shells,
            original_theme: self.theme.name.to_string(),
            original_colors: current,
            colors_at_open: colors,
            prefix_at_open: prefix,
        }
    }

    /// The config file changed while the welcome card is open: the card starts over from the
    /// new values (its previews are dropped), keeping the selected row.
    pub(super) fn refresh_welcome(&mut self) {
        if let Some(super::Overlay::Welcome(old)) = &self.overlay {
            let row = old.row;
            let mut w = self.welcome_setup();
            w.row = row.min(w.rows().len() - 1);
            self.overlay = Some(super::Overlay::Welcome(w));
        }
    }

    /// Display name of a shell option (`shell_options`): "" is auto, a full path is Git Bash.
    pub fn shell_option_label(&self, shell: &str) -> String {
        let path = std::path::Path::new(shell);
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned());
        if shell.is_empty() {
            format!("auto ({})", self.shell.label())
        } else if cfg!(windows) && shell.contains(['/', '\\']) && stem.as_deref() == Some("bash") {
            // Only Windows lists a full path (Git Bash); Linux/macOS list program names.
            "Git Bash".into()
        } else if shell.contains(['/', '\\']) {
            // Any other full path (set in the config by hand): its program name.
            stem.unwrap_or_else(|| shell.to_string())
        } else {
            shell.to_string()
        }
    }

    /// Changes a welcome setup row by `dir` (wrapping); theme and terminal colors preview at once.
    pub(super) fn welcome_step(&mut self, w: &mut super::WelcomeSetup, row: super::WelcomeRow, dir: i32) {
        let step = |i: usize, n: usize| (i as i32 + dir).rem_euclid(n.max(1) as i32) as usize;
        match row {
            super::WelcomeRow::Theme => {
                w.theme = step(w.theme, THEMES.len());
                self.preview_theme(w.theme);
            }
            super::WelcomeRow::Colors => {
                w.colors = step(w.colors, w.schemes.len());
                if let Some(name) = w.schemes.get(w.colors) {
                    self.cfg.terminal.colors = name.clone();
                }
            }
            super::WelcomeRow::Shell => w.shell = step(w.shell, w.shells.len()),
            super::WelcomeRow::Prefix => w.prefix = step(w.prefix, PREFIXES.len()),
        }
    }

    /// Closes the welcome. With `apply` the choices are saved to the config, otherwise the
    /// previews are undone. Either way it is never shown again.
    pub fn finish_welcome(&mut self, w: &super::WelcomeSetup, apply: bool) {
        self.overlay = None;
        // Undo the previews first: `cfg` then holds what the config file says.
        self.cfg.terminal.colors = w.original_colors.clone();
        self.theme = crate::theme::Theme::by_name(&w.original_theme, self.cfg.general.transparent);
        if apply {
            let mut c = self.cfg.clone();
            let mut edits: Vec<(&str, &str, String)> = Vec::new();
            if let Some(t) = THEMES.get(w.theme)
                && t.name != w.original_theme
            {
                c.general.theme = t.name.to_string();
                edits.push(("general", "theme", format!("\"{}\"", t.name)));
            }
            if w.colors != w.colors_at_open
                && let Some(name) = w.schemes.get(w.colors)
            {
                c.terminal.colors = name.clone();
                edits.push(("terminal", "colors", format!("\"{name}\"")));
            }
            if let Some(shell) = w.shells.get(w.shell)
                && *shell != c.terminal.shell
            {
                c.terminal.shell = shell.clone();
                let escaped = shell.replace('\\', "\\\\").replace('"', "\\\"");
                edits.push(("terminal", "shell", format!("\"{escaped}\"")));
            }
            if let Some(p) = PREFIXES.get(w.prefix)
                && w.prefix != w.prefix_at_open
                && !p.eq_ignore_ascii_case(c.keys.prefix.trim())
            {
                c.keys.prefix = p.to_string();
                edits.push(("keys", "prefix", format!("\"{p}\"")));
            }
            if !edits.is_empty() {
                self.apply_config(c);
                for (section, key, value) in &edits {
                    self.persist(section, key, value);
                }
                self.toast(ToastLevel::Ok, "all set · change any of it later in Settings");
            }
        }
        self.edit_ui_state(|d| d.welcomed = true);
    }

    /// Adds a root folder to the project scan. If the list is empty (auto mode) the
    /// default folders are written first so they keep being scanned too.
    pub fn add_project_root(&mut self, raw: &str) {
        let Some(path) = self.prompt_folder(raw) else { return };
        let mut c = self.cfg.clone();
        if c.projects.roots.is_empty() {
            c.projects.roots = crate::projects::default_roots().iter().map(|p| p.display().to_string()).collect();
        }
        let text = path.display().to_string();
        if c.projects.roots.iter().any(|r| r.eq_ignore_ascii_case(&text)) {
            self.toast(ToastLevel::Info, format!("{} is already scanned", crate::util::tilde(&path)));
            return;
        }
        c.projects.roots.push(text);
        let list = toml::Value::Array(c.projects.roots.iter().cloned().map(toml::Value::String).collect());
        self.apply_config(c);
        self.persist("projects", "roots", &list.to_string());
        self.toast(ToastLevel::Ok, format!("scanning {} for projects", crate::util::tilde(&path)));
    }

    /// A folder typed into the add prompt (`~` expands to the home folder, a relative path is
    /// taken from the home folder); the error text when it is not a folder.
    pub fn resolve_folder(raw: &str) -> Result<std::path::PathBuf, String> {
        let raw = raw.trim().trim_matches('"');
        let home = dirs::home_dir().unwrap_or_default();
        let path = match raw.strip_prefix('~') {
            Some(rest) => home.join(rest.trim_start_matches(['/', '\\'])),
            None => std::path::PathBuf::from(raw),
        };
        let path = if path.is_relative() && !raw.is_empty() { home.join(path) } else { path };
        if raw.is_empty() || !path.is_dir() {
            return Err(format!("not a folder: {raw}"));
        }
        // Rebuilt from its components: a trailing separator or "." parts do not make a new path.
        Ok(path.components().collect())
    }

    /// Like `resolve_folder`, with an error toast and `None` when it is not a folder.
    fn prompt_folder(&mut self, raw: &str) -> Option<std::path::PathBuf> {
        match Self::resolve_folder(raw) {
            Ok(p) => Some(p),
            Err(e) => {
                self.toast(ToastLevel::Error, e);
                None
            }
        }
    }

    /// The add prompt's action. Auto mode: a git repository, or a folder with no repository
    /// within two levels (`projects::holds_repos`), is added as a project; a folder that holds
    /// repositories is scanned for them. The toast names what happened. The folder walk runs on a
    /// helper thread: its answer arrives as `AppEvent::AddDecided`, so the UI keeps running meanwhile.
    pub fn add_by_mode(&mut self, mode: AddMode, raw: &str) {
        match mode {
            AddMode::Project => self.add_project(raw),
            AddMode::Folder => self.add_project_root(raw),
            AddMode::Auto => {
                let Ok(path) = Self::resolve_folder(raw) else { return };
                if crate::projects::git_dir(&path).is_some() {
                    return self.add_project(raw);
                }
                let (tx, probe, exclude) = (self.tx.clone(), self.repo_probe, self.cfg.projects.exclude.clone());
                let raw_owned = raw.to_string();
                let spawned = std::thread::Builder::new().name("add-decide".into()).spawn(move || {
                    let scan = holds_repos_in_time(path, probe, exclude);
                    let _ = tx.send(AppEvent::AddDecided { raw: raw_owned, scan });
                });
                // No thread to spare: the folder is added as a project, the answer that cannot be checked.
                if spawned.is_err() {
                    self.add_project(raw);
                }
            }
        }
    }

    /// Adds one folder as a project (a git repo or any other folder), kept in `state.json`
    /// so it stays listed after rescans and restarts even when the scan would not find it.
    /// A project removed from the list earlier comes back this way.
    pub fn add_project(&mut self, raw: &str) {
        let Some(path) = self.prompt_folder(raw) else { return };
        let Some(project) = crate::projects::project_at(&path) else {
            self.toast(ToastLevel::Error, format!("not a folder: {}", path.display()));
            return;
        };
        let (name, repo) = (project.name.clone(), project.repo);
        self.ui_state.add_project(&path);
        self.sync_manual_projects();
        let listed = self.projects.iter().position(|p| crate::util::same_path(&p.path, &path));
        let target = match listed {
            Some(i) => self.projects[i].path.clone(),
            None => {
                self.projects.push(project);
                self.projects_loaded = true;
                crate::projects::sort_projects(&mut self.projects);
                path.clone()
            }
        };
        self.sort_pinned();
        // Select it (clearing a search that would hide it) so the user sees where it landed.
        self.bridge.filter.clear();
        self.bridge.filtering = false;
        self.bridge.proj_act = None;
        if let Some(pos) = self.visible_projects().iter().position(|i| self.projects[*i].path == target) {
            self.bridge.proj_sel = pos;
        }
        if repo {
            self.request_git(target, std::time::Duration::ZERO);
        }
        if listed.is_some() {
            self.toast(ToastLevel::Info, format!("{name} is already in Projects"));
        } else {
            self.toast(ToastLevel::Ok, format!("added project {name}"));
        }
    }

    /// Removes a project from the list for good (until it is added again): it stays hidden
    /// across rescans and restarts, and is unpinned. The folder itself is not touched.
    pub fn remove_project(&mut self, path: &std::path::Path) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let selected = self.bridge.proj_sel;
        self.ui_state.hide_project(path);
        self.sync_manual_projects();
        self.projects.retain(|p| !crate::util::same_path(&p.path, path));
        self.bridge.proj_act = None;
        let n = self.visible_projects().len();
        self.bridge.proj_sel = selected.min(n.saturating_sub(1));
        self.toast(ToastLevel::Ok, format!("removed {name} from Projects · A on Home adds it back"));
    }

    /// Changes `state.json` (see `UiState::modify`). The write also reads back what other
    /// windows changed, so when the hidden/added projects differ afterwards the scan thread
    /// gets the new lists.
    pub(crate) fn edit_ui_state<T>(&mut self, f: impl FnOnce(&mut crate::store::UiStateData) -> T) -> T {
        let before = self.ui_state.manual_projects();
        let out = self.ui_state.modify(f);
        if self.ui_state.manual_projects() != before {
            self.sync_manual_projects();
        }
        out
    }

    /// Hands the hidden/added projects to the scan thread (the next scan applies them).
    pub(super) fn sync_manual_projects(&self) {
        if let Some(s) = &self.services
            && let Ok(mut m) = s.proj_manual.lock()
        {
            *m = self.ui_state.manual_projects();
        }
    }

    /// Scheme ids in the selector: "follow theme" first, then `term_schemes`.
    pub fn scheme_options(&self) -> Vec<String> {
        std::iter::once(crate::theme::FOLLOW_THEME.to_string())
            .chain(self.term_schemes.iter().map(|s| s.name.clone()))
            .collect()
    }

    /// Display name of a scheme; unknown ones (e.g. Windows Terminal not found) fall back to the theme.
    pub fn scheme_label(&self, name: &str) -> String {
        match crate::theme::find_scheme(&self.term_schemes, name) {
            Some(s) => s.label.clone(),
            None => "Follow theme".into(),
        }
    }

    /// Installed launchers (`launchers` order); the popup lists these.
    pub fn installed_launchers(&self) -> Vec<usize> {
        self.launchers.iter().enumerate().filter(|(_, (_, ok))| *ok).map(|(i, _)| i).collect()
    }

    /// Opens the quick launch popup (warns when no launcher is installed).
    pub fn open_launcher_picker(&mut self) {
        if self.installed_launchers().is_empty() {
            self.toast(ToastLevel::Warn, "none of the launcher commands are installed");
            return;
        }
        self.overlay = Some(super::Overlay::Launchers { selected: 0 });
    }

    /// Opens the scheme selector; the selected row is the current scheme.
    pub fn open_scheme_picker(&mut self) {
        self.reload_schemes();
        let current = self.cfg.terminal.colors.clone();
        let selected = self
            .term_schemes
            .iter()
            .position(|s| crate::theme::find_scheme(std::slice::from_ref(s), &current).is_some())
            .map(|i| i + 1)
            .unwrap_or(0);
        self.overlay = Some(super::Overlay::Schemes(super::SchemePicker { selected, original: current }));
    }

    /// Opens the theme selector on the theme in use.
    pub fn open_theme_picker(&mut self) {
        let original = self.theme.name.to_string();
        let selected = THEMES.iter().position(|t| t.name == original).unwrap_or(0);
        self.overlay = Some(super::Overlay::Themes(super::ThemePicker { selected, original }));
    }

    /// Shows the whole UI in a theme while navigating the selector, without saving it.
    pub(super) fn preview_theme(&mut self, idx: usize) {
        if let Some(t) = THEMES.get(idx) {
            self.theme = crate::theme::Theme::by_name(t.name, self.cfg.general.transparent);
        }
    }

    /// Closes a selector without choosing: its preview is undone.
    pub(super) fn cancel_picker(&mut self, ov: &super::Overlay) {
        match ov {
            super::Overlay::Themes(p) => {
                self.theme = crate::theme::Theme::by_name(&p.original, self.cfg.general.transparent)
            }
            super::Overlay::Schemes(p) => self.cfg.terminal.colors = p.original.clone(),
            _ => {}
        }
    }

    /// Previews a scheme while navigating the selector, without saving it.
    pub(super) fn preview_scheme(&mut self, idx: usize) {
        if let Some(name) = self.scheme_options().get(idx) {
            self.cfg.terminal.colors = name.clone();
        }
    }

    /// Picks the terminal color scheme and writes it to the config.
    pub fn set_term_colors(&mut self, name: &str) {
        let mut c = self.cfg.clone();
        c.terminal.colors = name.to_string();
        self.apply_config(c);
        self.persist("terminal", "colors", &format!("\"{name}\""));
        if let Some(i) = self.settings_items().iter().position(|it| *it == SettingItem::Setting(SettingKey::TermColors))
        {
            self.settings_sel = i;
        }
    }

    pub(super) fn move_setting(&mut self, delta: i32) {
        let n = self.settings_items().len() as i32;
        if n == 0 {
            return;
        }
        self.settings_sel = (self.settings_sel as i32 + delta).clamp(0, n - 1) as usize;
    }

    /// What ⏎ does on a row of the page (also the palette's "Setting:" commands): a popup opens for
    /// the terminal colors and the theme, every other row changes one step.
    pub fn enter_setting(&mut self, item: SettingItem) {
        match item {
            SettingItem::Setting(SettingKey::TermColors) => self.open_scheme_picker(),
            SettingItem::Setting(SettingKey::Theme) => self.open_theme_picker(),
            other => self.activate_setting(other, 1),
        }
    }

    pub(super) fn settings_key(&mut self, k: KeyEvent) {
        // The page scrolls back to the selection (the wheel may have moved it away).
        self.settings_follow = true;
        if self.settings_filtering {
            return self.settings_filter_key(k);
        }
        let items = self.settings_items();
        let sel = self.settings_sel.min(items.len().saturating_sub(1));
        let chosen = items.get(sel).copied();
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_setting(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_setting(1),
            // ← turns a switch off and → on (pressing again changes nothing); a choice steps back/forward.
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Right | KeyCode::Char('l') => {
                let right = matches!(k.code, KeyCode::Right | KeyCode::Char('l'));
                if let Some(item @ SettingItem::Setting(key)) = chosen {
                    if key.opens_popup() {
                    } else if key.is_toggle() {
                        if self.setting_on(key) != right {
                            self.activate_setting(item, 1);
                        }
                    } else {
                        self.activate_setting(item, if right { 1 } else { -1 });
                    }
                }
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(item) = chosen {
                    self.enter_setting(item);
                }
            }
            KeyCode::Home => self.settings_sel = 0,
            KeyCode::End => self.settings_sel = items.len().saturating_sub(1),
            // "/" starts a filter on the row names (like Home's); esc clears a kept one, then leaves.
            KeyCode::Char('/') => {
                self.settings_filtering = true;
                self.settings_filter.clear();
                self.settings_sel = 0;
            }
            KeyCode::Esc if !self.settings_filter.is_empty() => {
                self.settings_filter.clear();
                self.settings_sel = 0;
            }
            KeyCode::Esc | KeyCode::Char('q') => self.close_settings(),
            KeyCode::Char('?') => self.run(crate::keys::Action::Help),
            KeyCode::Char(':' | 'p') => self.run(crate::keys::Action::Palette),
            KeyCode::Char(c @ '1'..='9') => self.go_tab(c as usize - '0' as usize),
            _ => {}
        }
    }

    /// Keys while the filter takes text: letters narrow the rows, ↑↓ move, ⏎ keeps the filter and
    /// returns to the rows, esc clears it.
    fn settings_filter_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => {
                self.settings_filtering = false;
                self.settings_filter.clear();
                self.settings_sel = 0;
            }
            KeyCode::Enter => self.settings_filtering = false,
            KeyCode::Up => self.move_setting(-1),
            KeyCode::Down => self.move_setting(1),
            KeyCode::Backspace => {
                if self.settings_filter.pop().is_none() {
                    self.settings_filtering = false;
                }
                self.settings_sel = 0;
            }
            KeyCode::Char(c) if !super::input::ctrl_held(&k) && !k.modifiers.contains(KeyModifiers::ALT) => {
                self.settings_filter.push(c);
                self.settings_sel = 0;
            }
            _ => {}
        }
    }
}
