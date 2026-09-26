//! Key chords, actions and the key map (prefix + direct shortcuts).

use std::collections::{HashMap, HashSet};
use std::fmt;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::config::KeysCfg;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl Chord {
    pub fn new(code: KeyCode, mods: KeyModifiers) -> Self {
        Self { code, mods }.normalized()
    }

    pub fn from_event(ev: &KeyEvent) -> Self {
        Self::new(ev.code, ev.modifiers)
    }

    /// For character keys the SHIFT info is carried by the character itself
    /// ("A", "|"), so it is dropped when matching.
    fn normalized(mut self) -> Self {
        self.mods &= KeyModifiers::SHIFT | KeyModifiers::CONTROL | KeyModifiers::ALT;
        if let KeyCode::Char(c) = self.code {
            self.mods.remove(KeyModifiers::SHIFT);
            // A symbol produced with AltGr (Ctrl+Alt) counts as a plain character.
            if crate::term::input::is_altgr_char(self.mods, c) {
                self.mods = KeyModifiers::NONE;
            }
            if self.mods.contains(KeyModifiers::CONTROL) && c.is_ascii_uppercase() {
                self.code = KeyCode::Char(c.to_ascii_lowercase());
            }
        }
        if self.code == KeyCode::Tab && self.mods.contains(KeyModifiers::SHIFT) {
            self.code = KeyCode::BackTab;
        }
        if self.code == KeyCode::BackTab {
            self.mods.remove(KeyModifiers::SHIFT);
        }
        self
    }

    /// Parses texts like "ctrl+a", "alt+1", "shift+up", "|", "space", "f5".
    pub fn parse(text: &str) -> Option<Chord> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        // "+" alone can be a key; also support spellings like "ctrl++".
        let (mod_part, key_part) = if text == "+" {
            ("", "+")
        } else if let Some(stripped) = text.strip_suffix("++") {
            (stripped, "+")
        } else {
            match text.rfind('+') {
                Some(i) => (&text[..i], &text[i + 1..]),
                None => ("", text),
            }
        };
        let mut mods = KeyModifiers::NONE;
        for m in mod_part.split('+').filter(|m| !m.is_empty()) {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "c" => mods |= KeyModifiers::CONTROL,
                "alt" | "meta" | "m" | "opt" => mods |= KeyModifiers::ALT,
                "shift" | "s" => mods |= KeyModifiers::SHIFT,
                _ => return None,
            }
        }
        let lower = key_part.to_ascii_lowercase();
        let code = match lower.as_str() {
            "space" => KeyCode::Char(' '),
            "enter" | "return" => KeyCode::Enter,
            "esc" | "escape" => KeyCode::Esc,
            "tab" => KeyCode::Tab,
            "backtab" => KeyCode::BackTab,
            "backspace" | "bs" => KeyCode::Backspace,
            "delete" | "del" => KeyCode::Delete,
            "insert" | "ins" => KeyCode::Insert,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "pageup" | "pgup" => KeyCode::PageUp,
            "pagedown" | "pgdn" => KeyCode::PageDown,
            f if f.len() >= 2 && f.starts_with('f') && f[1..].parse::<u8>().is_ok() => KeyCode::F(f[1..].parse().ok()?),
            _ => {
                let mut chars = key_part.chars();
                let c = chars.next()?;
                if chars.next().is_some() {
                    return None;
                }
                KeyCode::Char(c)
            }
        };
        Some(Chord::new(code, mods))
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts: Vec<String> = Vec::new();
        if self.mods.contains(KeyModifiers::CONTROL) {
            parts.push("ctrl".into());
        }
        if self.mods.contains(KeyModifiers::ALT) {
            parts.push("alt".into());
        }
        if self.mods.contains(KeyModifiers::SHIFT) {
            parts.push("shift".into());
        }
        let key = match self.code {
            KeyCode::Char(' ') => "space".into(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Enter => "enter".into(),
            KeyCode::Esc => "esc".into(),
            KeyCode::Tab => "tab".into(),
            KeyCode::BackTab => "shift+tab".into(),
            KeyCode::Backspace => "bksp".into(),
            KeyCode::Delete => "del".into(),
            KeyCode::Insert => "ins".into(),
            KeyCode::Up => "↑".into(),
            KeyCode::Down => "↓".into(),
            KeyCode::Left => "←".into(),
            KeyCode::Right => "→".into(),
            KeyCode::Home => "home".into(),
            KeyCode::End => "end".into(),
            KeyCode::PageUp => "pgup".into(),
            KeyCode::PageDown => "pgdn".into(),
            KeyCode::F(n) => format!("f{n}"),
            _ => "?".into(),
        };
        parts.push(key);
        write!(f, "{}", parts.join("+"))
    }
}

/// All actions of the app. Known in the config by their snake_case names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Bridge,
    System,
    Settings,
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    GoTab(u8),
    SplitRight,
    SplitDown,
    ClosePane,
    Zoom,
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    FocusNext,
    ResizeLeft,
    ResizeRight,
    ResizeUp,
    ResizeDown,
    Palette,
    Help,
    Quit,
    ReloadConfig,
    OpenConfig,
    CycleTheme,
    RefreshAi,
    RescanProjects,
    RenameTab,
    SaveWorkspace,
    ScrollUp,
    ScrollDown,
    Search,
    AddProjectFolder,
    SendPrefix,
    MoveTabLeft,
    MoveTabRight,
    PaneMenu,
    Update,
    DismissUpdate,
    Passthrough,
}

impl Action {
    pub const ALL: [Action; 50] = [
        Action::Bridge,
        Action::System,
        Action::Settings,
        Action::NewTab,
        Action::CloseTab,
        Action::NextTab,
        Action::PrevTab,
        Action::GoTab(1),
        Action::GoTab(2),
        Action::GoTab(3),
        Action::GoTab(4),
        Action::GoTab(5),
        Action::GoTab(6),
        Action::GoTab(7),
        Action::GoTab(8),
        Action::GoTab(9),
        Action::SplitRight,
        Action::SplitDown,
        Action::ClosePane,
        Action::Zoom,
        Action::FocusLeft,
        Action::FocusRight,
        Action::FocusUp,
        Action::FocusDown,
        Action::FocusNext,
        Action::ResizeLeft,
        Action::ResizeRight,
        Action::ResizeUp,
        Action::ResizeDown,
        Action::Palette,
        Action::Help,
        Action::Quit,
        Action::ReloadConfig,
        Action::OpenConfig,
        Action::CycleTheme,
        Action::RefreshAi,
        Action::RescanProjects,
        Action::RenameTab,
        Action::SaveWorkspace,
        Action::ScrollUp,
        Action::ScrollDown,
        Action::Search,
        Action::AddProjectFolder,
        Action::SendPrefix,
        Action::MoveTabLeft,
        Action::MoveTabRight,
        Action::PaneMenu,
        Action::Update,
        Action::DismissUpdate,
        Action::Passthrough,
    ];

    pub fn id(&self) -> String {
        match self {
            Action::GoTab(n) => format!("tab_{n}"),
            other => {
                let s = match other {
                    Action::Bridge => "bridge",
                    Action::System => "system",
                    Action::Settings => "settings",
                    Action::NewTab => "new_tab",
                    Action::CloseTab => "close_tab",
                    Action::NextTab => "next_tab",
                    Action::PrevTab => "prev_tab",
                    Action::SplitRight => "split_right",
                    Action::SplitDown => "split_down",
                    Action::ClosePane => "close_pane",
                    Action::Zoom => "zoom",
                    Action::FocusLeft => "focus_left",
                    Action::FocusRight => "focus_right",
                    Action::FocusUp => "focus_up",
                    Action::FocusDown => "focus_down",
                    Action::FocusNext => "focus_next",
                    Action::ResizeLeft => "resize_left",
                    Action::ResizeRight => "resize_right",
                    Action::ResizeUp => "resize_up",
                    Action::ResizeDown => "resize_down",
                    Action::Palette => "palette",
                    Action::Help => "help",
                    Action::Quit => "quit",
                    Action::ReloadConfig => "reload_config",
                    Action::OpenConfig => "open_config",
                    Action::CycleTheme => "cycle_theme",
                    Action::RefreshAi => "refresh_ai",
                    Action::RescanProjects => "rescan_projects",
                    Action::RenameTab => "rename_tab",
                    Action::SaveWorkspace => "save_workspace",
                    Action::ScrollUp => "scroll_up",
                    Action::ScrollDown => "scroll_down",
                    Action::Search => "search",
                    Action::AddProjectFolder => "add_project_folder",
                    Action::SendPrefix => "send_prefix",
                    Action::MoveTabLeft => "move_tab_left",
                    Action::MoveTabRight => "move_tab_right",
                    Action::PaneMenu => "pane_menu",
                    Action::Update => "update",
                    Action::DismissUpdate => "dismiss_update",
                    Action::Passthrough => "passthrough",
                    Action::GoTab(_) => unreachable!(),
                };
                s.to_string()
            }
        }
    }

    pub fn from_id(id: &str) -> Option<Action> {
        let id = id.trim().to_ascii_lowercase().replace('-', "_");
        Action::ALL.iter().copied().find(|a| a.id() == id)
    }

    /// The title shown in the palette and the help screen.
    pub fn title(&self) -> String {
        match self {
            Action::Bridge => "Go Home".into(),
            Action::System => "Open System Monitor".into(),
            Action::Settings => "Open Settings".into(),
            Action::NewTab => "New Terminal Tab".into(),
            Action::CloseTab => "Close Tab".into(),
            Action::NextTab => "Next Tab".into(),
            Action::PrevTab => "Previous Tab".into(),
            Action::GoTab(n) => format!("Go to Tab {n}"),
            Action::SplitRight => "Split Pane Right".into(),
            Action::SplitDown => "Split Pane Down".into(),
            Action::ClosePane => "Close Pane".into(),
            Action::Zoom => "Zoom / Restore Pane".into(),
            Action::FocusLeft => "Focus Pane Left".into(),
            Action::FocusRight => "Focus Pane Right".into(),
            Action::FocusUp => "Focus Pane Up".into(),
            Action::FocusDown => "Focus Pane Down".into(),
            Action::FocusNext => "Focus Next Pane".into(),
            Action::ResizeLeft => "Move Divider Left".into(),
            Action::ResizeRight => "Move Divider Right".into(),
            Action::ResizeUp => "Move Divider Up".into(),
            Action::ResizeDown => "Move Divider Down".into(),
            Action::Palette => "Command Palette".into(),
            Action::Help => "Keyboard Reference".into(),
            Action::Quit => "Quit NOBLE".into(),
            Action::ReloadConfig => "Reload Config".into(),
            Action::OpenConfig => "Edit Config File".into(),
            Action::CycleTheme => "Cycle Theme".into(),
            Action::RefreshAi => "Refresh AI Quotas".into(),
            Action::RescanProjects => "Rescan Projects".into(),
            Action::RenameTab => "Rename Tab".into(),
            Action::SaveWorkspace => "Save Workspace Snapshot".into(),
            Action::ScrollUp => "Scroll Up (page)".into(),
            Action::ScrollDown => "Scroll Down (page)".into(),
            Action::Search => "Search Scrollback".into(),
            Action::AddProjectFolder => "Add Project Folder…".into(),
            Action::SendPrefix => "Send Prefix Key to Shell".into(),
            Action::MoveTabLeft => "Move Tab Left".into(),
            Action::MoveTabRight => "Move Tab Right".into(),
            Action::PaneMenu => "Pane Menu (copy path, open folder…)".into(),
            Action::Update => "Update NOBLE".into(),
            Action::DismissUpdate => "Dismiss Update Notice".into(),
            Action::Passthrough => "Pass Keys to the App".into(),
        }
    }

    /// Palette'teki grup etiketi.
    pub fn group(&self) -> &'static str {
        match self {
            Action::Bridge | Action::System | Action::Settings | Action::Palette | Action::Help | Action::Quit => "NAV",
            Action::NewTab
            | Action::CloseTab
            | Action::NextTab
            | Action::PrevTab
            | Action::GoTab(_)
            | Action::RenameTab
            | Action::MoveTabLeft
            | Action::MoveTabRight => "TAB",
            Action::SplitRight
            | Action::SplitDown
            | Action::ClosePane
            | Action::Zoom
            | Action::FocusLeft
            | Action::FocusRight
            | Action::FocusUp
            | Action::FocusDown
            | Action::FocusNext
            | Action::ResizeLeft
            | Action::ResizeRight
            | Action::ResizeUp
            | Action::ResizeDown
            | Action::ScrollUp
            | Action::ScrollDown
            | Action::Search
            | Action::SendPrefix
            | Action::PaneMenu
            | Action::Passthrough => "PANE",
            _ => "SYS",
        }
    }
}

/// Default direct shortcuts that bash, zsh and fish (and readline in general) use at the prompt:
/// alt+. last argument, alt+t transpose words, alt+s sudo (fish) / spelling (zsh), alt+p pager (fish) /
/// history search (bash). With `keys.shell_first` they reach the app in a pane; elsewhere they still work.
pub const SHELL_KEYS: [&str; 5] = ["alt+.", "alt+,", "alt+t", "alt+s", "alt+p"];

pub struct Keymap {
    pub prefix: Chord,
    pub prefix_map: HashMap<Chord, Action>,
    pub direct_map: HashMap<Chord, Action>,
    /// Direct shortcuts left to the app in a terminal pane (`SHELL_KEYS` not rebound by the user).
    pub shell_first: HashSet<Chord>,
    /// Invalid entries in the config (shown to the user).
    pub warnings: Vec<String>,
}

fn c(s: &str) -> Chord {
    Chord::parse(s).expect("valid default chord")
}

pub fn default_prefix_bindings() -> Vec<(Chord, Action)> {
    let mut v = vec![
        (c("t"), Action::NewTab),
        (c("c"), Action::NewTab),
        (c("X"), Action::CloseTab),
        (c("n"), Action::NextTab),
        (c("p"), Action::PrevTab),
        (c("0"), Action::Bridge),
        (c("b"), Action::Bridge),
        (c("m"), Action::System),
        (c("S"), Action::Settings),
        (c("v"), Action::SplitRight),
        (c("|"), Action::SplitRight),
        (c("s"), Action::SplitDown),
        (c("-"), Action::SplitDown),
        (c("x"), Action::ClosePane),
        (c("z"), Action::Zoom),
        (c("left"), Action::FocusLeft),
        (c("right"), Action::FocusRight),
        (c("up"), Action::FocusUp),
        (c("down"), Action::FocusDown),
        (c("o"), Action::FocusNext),
        (c("shift+left"), Action::ResizeLeft),
        (c("shift+right"), Action::ResizeRight),
        (c("shift+up"), Action::ResizeUp),
        (c("shift+down"), Action::ResizeDown),
        (c("H"), Action::ResizeLeft),
        (c("L"), Action::ResizeRight),
        (c("K"), Action::ResizeUp),
        (c("J"), Action::ResizeDown),
        (c(":"), Action::Palette),
        (c("space"), Action::Palette),
        (c("?"), Action::Help),
        (c("q"), Action::Quit),
        (c("r"), Action::ReloadConfig),
        (c(","), Action::RenameTab),
        (c("<"), Action::MoveTabLeft),
        (c(">"), Action::MoveTabRight),
        (c("."), Action::PaneMenu),
        (c("w"), Action::SaveWorkspace),
        (c("pgup"), Action::ScrollUp),
        (c("["), Action::ScrollUp),
        (c("pgdn"), Action::ScrollDown),
        (c("]"), Action::ScrollDown),
        (c("/"), Action::Search),
        (c("f"), Action::Search),
        (c("i"), Action::Passthrough),
    ];
    for n in 1..=9u8 {
        v.push((Chord::new(KeyCode::Char((b'0' + n) as char), KeyModifiers::NONE), Action::GoTab(n)));
    }
    v
}

pub fn default_direct_bindings() -> Vec<(Chord, Action)> {
    let mut v = vec![
        (c("alt+0"), Action::Bridge),
        (c("alt+t"), Action::NewTab),
        (c("alt+p"), Action::Palette),
        (c("alt+m"), Action::System),
        (c("alt+s"), Action::Settings),
        (c("alt+z"), Action::Zoom),
        (c("alt+o"), Action::FocusNext),
        (c("alt+."), Action::NextTab),
        (c("alt+,"), Action::PrevTab),
        (c("shift+pgup"), Action::ScrollUp),
        (c("shift+pgdn"), Action::ScrollDown),
    ];
    for n in 1..=9u8 {
        v.push((Chord::new(KeyCode::Char((b'0' + n) as char), KeyModifiers::ALT), Action::GoTab(n)));
    }
    v
}

impl Keymap {
    pub fn from_config(cfg: &KeysCfg) -> Keymap {
        let mut warnings = Vec::new();
        let prefix = Chord::parse(&cfg.prefix).unwrap_or_else(|| {
            warnings.push(format!("invalid prefix '{}', using ctrl+a", cfg.prefix));
            c("ctrl+a")
        });
        let mut prefix_map: HashMap<Chord, Action> = default_prefix_bindings().into_iter().collect();
        let mut direct_map: HashMap<Chord, Action> = default_direct_bindings().into_iter().collect();
        for (map, user, label) in
            [(&mut prefix_map, &cfg.prefix_bindings, "prefix"), (&mut direct_map, &cfg.direct_bindings, "direct")]
        {
            for (key, action) in user {
                match (Chord::parse(key), action.trim()) {
                    (Some(chord), "none" | "") => {
                        map.remove(&chord);
                    }
                    (Some(chord), id) => match Action::from_id(id) {
                        Some(a) => {
                            map.insert(chord, a);
                        }
                        None => warnings.push(format!("{label} binding '{key}': unknown action '{id}'")),
                    },
                    (None, _) => warnings.push(format!("{label} binding: cannot parse key '{key}'")),
                }
            }
        }
        // The prefix chord cannot be in the direct map, or the prefix could never be set.
        direct_map.remove(&prefix);
        // A shell key the user bound on purpose stays NOBLE's in terminals too.
        let user: HashSet<Chord> = cfg.direct_bindings.keys().filter_map(|k| Chord::parse(k)).collect();
        let shell_first = if cfg.shell_first {
            SHELL_KEYS.iter().map(|k| c(k)).filter(|k| !user.contains(k)).collect()
        } else {
            HashSet::new()
        };
        Keymap { prefix, prefix_map, direct_map, shell_first, warnings }
    }

    /// The shortest shortcut of an action (for the palette hint).
    pub fn hint(&self, action: Action) -> Option<String> {
        self.hint_where(action, |_| true)
    }

    /// The shortest shortcut that works in a terminal pane: no shell key, and with the pane's keys
    /// `locked` no direct shortcut at all.
    pub fn term_hint(&self, action: Action, locked: bool) -> Option<String> {
        self.hint_where(action, |k| !locked && !self.shell_first.contains(k))
    }

    fn hint_where(&self, action: Action, direct_ok: impl Fn(&Chord) -> bool) -> Option<String> {
        let direct = self
            .direct_map
            .iter()
            .filter(|(k, a)| **a == action && direct_ok(k))
            .map(|(k, _)| k.to_string())
            .min_by_key(|s| s.len());
        if let Some(d) = direct {
            return Some(d);
        }
        self.prefix_map
            .iter()
            .filter(|(_, a)| **a == action)
            .map(|(k, _)| k.to_string())
            .min_by_key(|s| (s.len(), s.clone()))
            .map(|k| format!("{} {k}", self.prefix))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_chords() {
        assert_eq!(Chord::parse("ctrl+a"), Some(Chord::new(KeyCode::Char('a'), KeyModifiers::CONTROL)));
        assert_eq!(Chord::parse("Ctrl+A"), Some(Chord::new(KeyCode::Char('a'), KeyModifiers::CONTROL)));
        assert_eq!(Chord::parse("alt+1"), Some(Chord::new(KeyCode::Char('1'), KeyModifiers::ALT)));
        assert_eq!(Chord::parse("shift+up"), Some(Chord::new(KeyCode::Up, KeyModifiers::SHIFT)));
        assert_eq!(Chord::parse("|"), Some(Chord::new(KeyCode::Char('|'), KeyModifiers::NONE)));
        assert_eq!(Chord::parse("+"), Some(Chord::new(KeyCode::Char('+'), KeyModifiers::NONE)));
        assert_eq!(Chord::parse("ctrl++"), Some(Chord::new(KeyCode::Char('+'), KeyModifiers::CONTROL)));
        assert_eq!(Chord::parse("f5"), Some(Chord::new(KeyCode::F(5), KeyModifiers::NONE)));
        assert_eq!(Chord::parse("hyper+x"), None);
        assert_eq!(Chord::parse("abc"), None);
    }

    #[test]
    fn shift_is_folded_into_chars() {
        let from_terminal = Chord::new(KeyCode::Char('X'), KeyModifiers::SHIFT);
        assert_eq!(Some(from_terminal), Chord::parse("X"));
        let pipe = Chord::new(KeyCode::Char('|'), KeyModifiers::SHIFT);
        assert_eq!(Some(pipe), Chord::parse("|"));
        let altgr_pipe = Chord::new(KeyCode::Char('|'), KeyModifiers::CONTROL | KeyModifiers::ALT);
        assert_eq!(Some(altgr_pipe), Chord::parse("|"));
    }

    #[test]
    fn action_ids_roundtrip() {
        for a in Action::ALL {
            assert_eq!(Action::from_id(&a.id()), Some(a), "{a:?}");
        }
        assert_eq!(Action::from_id("split-right"), Some(Action::SplitRight));
    }

    #[test]
    fn user_overrides() {
        let mut cfg = KeysCfg { prefix: "ctrl+b".into(), ..KeysCfg::default() };
        cfg.prefix_bindings.insert("%".into(), "split_right".into());
        cfg.prefix_bindings.insert("v".into(), "none".into());
        cfg.prefix_bindings.insert("y".into(), "bogus".into());
        let km = Keymap::from_config(&cfg);
        assert_eq!(km.prefix, c("ctrl+b"));
        assert_eq!(km.prefix_map.get(&c("%")), Some(&Action::SplitRight));
        assert!(!km.prefix_map.contains_key(&c("v")));
        assert_eq!(km.warnings.len(), 1);
        assert_eq!(km.hint(Action::Palette).as_deref(), Some("alt+p"));
        assert_eq!(km.direct_map.get(&c("alt+.")), Some(&Action::NextTab));
        assert_eq!(km.direct_map.get(&c("alt+,")), Some(&Action::PrevTab));
    }
}
