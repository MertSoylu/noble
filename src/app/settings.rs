//! Settings tab: selectable items, value changes and persistent writes to the config.

use crossterm::event::{KeyCode, KeyEvent};

use super::{App, ToastLevel, View};
use crate::theme::THEMES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingKey {
    Transparent,
    Boot,
    Animations,
    Clock24,
    Seconds,
    Shell,
    Restore,
    CopySelect,
    TermColors,
    Notify,
    Prefix,
    Passthrough,
    ShellFirst,
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
    Theme(usize),
    Setting(SettingKey),
    OpenConfig,
    ReloadConfig,
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
            SettingKey::Transparent => "Transparent background",
            SettingKey::Boot => "Boot animation",
            SettingKey::Animations => "Animations",
            SettingKey::Clock24 => "24-hour clock",
            SettingKey::Seconds => "Show seconds",
            SettingKey::Shell => "Shell",
            SettingKey::Restore => "Restore tabs on launch",
            SettingKey::CopySelect => "Copy text on select",
            SettingKey::Notify => "Notify from background tabs",
            SettingKey::TermColors => "Terminal colors",
            SettingKey::Prefix => "Prefix key",
            SettingKey::Passthrough => "Pass shortcuts to apps",
            SettingKey::ShellFirst => "Leave shell keys to the shell",
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

    pub fn is_toggle(&self) -> bool {
        !matches!(
            self,
            SettingKey::Shell
                | SettingKey::Prefix
                | SettingKey::Passthrough
                | SettingKey::TermColors
                | SettingKey::QuickLaunch
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

impl App {
    /// Selectable items on the page, in display order.
    pub fn settings_items(&self) -> Vec<SettingItem> {
        let mut v: Vec<SettingItem> = (0..THEMES.len()).map(SettingItem::Theme).collect();
        v.extend(
            [
                SettingKey::Transparent,
                SettingKey::Boot,
                SettingKey::Animations,
                SettingKey::Clock24,
                SettingKey::Seconds,
                SettingKey::Updates,
                SettingKey::Shell,
                SettingKey::Prefix,
                SettingKey::Passthrough,
                SettingKey::ShellFirst,
                SettingKey::Restore,
                SettingKey::CopySelect,
                SettingKey::TermColors,
                SettingKey::Notify,
                SettingKey::QuickLaunch,
                SettingKey::AiEnabled,
            ]
            .map(SettingItem::Setting),
        );
        // A provider that is not installed is hidden anyway; toggling it is meaningless.
        v.extend(
            PROVIDER_KEYS
                .iter()
                .filter(|k| self.ai_installed.contains(&k.provider_id()))
                .map(|k| SettingItem::Setting(*k)),
        );
        v.extend([SettingKey::AiRefresh, SettingKey::AiWarn].map(SettingItem::Setting));
        if self.claude_hooks_available() {
            v.push(SettingItem::Setting(SettingKey::ClaudeHooks));
        }
        v.extend([SettingItem::OpenConfig, SettingItem::ReloadConfig]);
        v
    }

    pub fn setting_on(&self, key: SettingKey) -> bool {
        let c = &self.cfg;
        let has = |id: &str| c.ai.providers.iter().any(|p| p.eq_ignore_ascii_case(id));
        match key {
            SettingKey::Transparent => c.general.transparent,
            SettingKey::Boot => c.general.boot_animation,
            SettingKey::Animations => c.general.animations,
            SettingKey::Clock24 => c.general.clock_24h,
            SettingKey::Seconds => c.general.show_seconds,
            SettingKey::Updates => c.general.check_updates,
            SettingKey::Restore => c.terminal.restore_session,
            SettingKey::CopySelect => c.terminal.copy_on_select,
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
            SettingKey::Prefix => self.cfg.keys.prefix.clone(),
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
    fn persist(&mut self, section: &str, key: &str, value_toml: &str) {
        if self.services.is_none() {
            return;
        }
        let text = std::fs::read_to_string(&self.paths.config).unwrap_or_else(|_| crate::config::DEFAULT_CONFIG.into());
        let updated = crate::config::set_value(&text, section, key, value_toml);
        if std::fs::write(&self.paths.config, updated).is_ok() {
            self.cfg_mtime = crate::config::mtime_of(&self.paths.config);
        }
    }

    /// Writes the launcher list to the config (`[[launchers]]` blocks are rebuilt).
    fn persist_launchers(&mut self) {
        if self.services.is_none() {
            return;
        }
        let text = std::fs::read_to_string(&self.paths.config).unwrap_or_else(|_| crate::config::DEFAULT_CONFIG.into());
        let updated = crate::config::set_launchers(&text, &self.cfg.launchers);
        if std::fs::write(&self.paths.config, updated).is_ok() {
            self.cfg_mtime = crate::config::mtime_of(&self.paths.config);
        }
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
            SettingItem::Setting(SettingKey::ClaudeHooks) => self.toggle_claude_hooks(),
            SettingItem::Theme(i) => {
                if let Some(t) = THEMES.get(i) {
                    self.set_theme(t.name);
                }
            }
            SettingItem::OpenConfig => self.run(crate::keys::Action::OpenConfig),
            SettingItem::ReloadConfig => self.reload_config(true),
            SettingItem::Setting(key) => {
                let mut c = self.cfg.clone();
                let (section, name, value) = match key {
                    // Set from the popup (handled above).
                    SettingKey::QuickLaunch => return self.open_launcher_picker(),
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

    /// Opens the welcome screen with the current prefix selected.
    pub fn show_welcome(&mut self) {
        let prefix = PREFIXES.iter().position(|p| p.eq_ignore_ascii_case(self.cfg.keys.prefix.trim())).unwrap_or(0);
        self.overlay = Some(super::Overlay::Welcome { prefix });
    }

    /// Closes the welcome; with `apply` the selected prefix takes effect. Never shown again.
    pub fn finish_welcome(&mut self, prefix: usize, apply: bool) {
        self.overlay = None;
        if apply
            && let Some(p) = PREFIXES.get(prefix)
            && !p.eq_ignore_ascii_case(self.cfg.keys.prefix.trim())
        {
            self.set_prefix(p);
            self.toast(ToastLevel::Ok, format!("prefix · {p}"));
        }
        self.ui_state.data.welcomed = true;
        self.ui_state.save();
    }

    /// Adds a root folder to the project scan. If the list is empty (auto mode) the
    /// default folders are written first so they keep being scanned too.
    pub fn add_project_root(&mut self, raw: &str) {
        let raw = raw.trim().trim_matches('"');
        let path = match raw.strip_prefix('~') {
            Some(rest) => dirs::home_dir().unwrap_or_default().join(rest.trim_start_matches(['/', '\\'])),
            None => std::path::PathBuf::from(raw),
        };
        if raw.is_empty() || !path.is_dir() {
            self.toast(ToastLevel::Error, format!("not a folder: {raw}"));
            return;
        }
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
        self.toast(ToastLevel::Ok, format!("added {} · scanning…", crate::util::tilde(&path)));
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
        self.settings_sel = (self.settings_sel as i32 + delta).clamp(0, n - 1) as usize;
    }

    /// Column count of the theme grid (same calculation as the drawing).
    pub fn theme_columns(&self) -> usize {
        let inner = crate::ui::settings_width(self.size.0).saturating_sub(6);
        (inner as usize / crate::ui::THEME_CARD_W as usize).clamp(1, 6)
    }

    pub(super) fn settings_key(&mut self, k: KeyEvent) {
        let items = self.settings_items();
        let sel = self.settings_sel.min(items.len() - 1);
        let cols = self.theme_columns() as i32;
        let in_grid = matches!(items[sel], SettingItem::Theme(_));
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if in_grid {
                    if sel >= cols as usize {
                        self.settings_sel = sel - cols as usize;
                    }
                } else if let Some(SettingItem::Theme(_)) = items.get(sel.saturating_sub(1)) {
                    // Wrap to the start of the grid's last row.
                    let n = THEMES.len();
                    self.settings_sel = n - 1 - (n - 1) % cols as usize;
                } else {
                    self.move_setting(-1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if in_grid {
                    let n = THEMES.len();
                    let next = sel + cols as usize;
                    self.settings_sel = if next < n { next } else { n };
                } else {
                    self.move_setting(1);
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if in_grid {
                    self.move_setting(-1);
                } else if let SettingItem::Setting(key) = items[sel] {
                    self.activate_setting(items[sel], if key.is_toggle() { 1 } else { -1 });
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if in_grid {
                    self.move_setting(1);
                } else if let SettingItem::Setting(_) = items[sel] {
                    self.activate_setting(items[sel], 1);
                }
            }
            KeyCode::Enter | KeyCode::Char(' ') if items[sel] == SettingItem::Setting(SettingKey::TermColors) => {
                self.open_scheme_picker()
            }
            KeyCode::Enter | KeyCode::Char(' ') => self.activate_setting(items[sel], 1),
            KeyCode::Home => self.settings_sel = 0,
            KeyCode::End => self.settings_sel = items.len() - 1,
            KeyCode::Esc | KeyCode::Char('q') => self.view = View::Bridge,
            KeyCode::Char('?') => self.run(crate::keys::Action::Help),
            KeyCode::Char(':' | 'p') => self.run(crate::keys::Action::Palette),
            KeyCode::Char(c @ '1'..='9') => self.go_tab(c as usize - '0' as usize),
            _ => {}
        }
    }
}
