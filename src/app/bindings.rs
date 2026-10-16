//! The key bindings editor (Settings → Key bindings): every action with its prefix and direct keys,
//! the capture of a new key, the question when a key belongs to another action, and the writes to
//! `config.toml`. Drawing is in `ui/overlay.rs`; keys and clicks are in `app/input.rs`. Both read the
//! same rows, so a click or ⏎ acts on the row that is drawn.

use std::collections::{HashMap, HashSet};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

use super::{App, Overlay, ToastLevel};
use crate::config::{self, KeysCfg};
use crate::keys::{Action, Chord, chords_of, default_direct_bindings, default_prefix_bindings};
use crate::term::input::is_altgr_char;

/// The table a binding belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// Pressed after the prefix key (`keys.prefix_bindings`).
    Prefix,
    /// Pressed on its own (`keys.direct_bindings`).
    Direct,
}

impl Slot {
    /// The config table of the slot.
    pub fn table(self) -> &'static str {
        match self {
            Slot::Prefix => "keys.prefix_bindings",
            Slot::Direct => "keys.direct_bindings",
        }
    }
}

/// Typed state of the overlay.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BindingsState {
    pub query: String,
    /// `/` opened the filter: typed letters go to the query, not to the keys.
    pub filtering: bool,
    /// Index into the rows (`App::binding_rows`).
    pub selected: usize,
    /// A key is awaited for this action in this slot.
    pub capture: Option<(Action, Slot)>,
    /// The captured key belongs to another action: waiting for y or n.
    pub replace: Option<Replace>,
    /// Why the last captured key was refused (shown until the next key).
    pub note: Option<String>,
    /// The last change, as saved (shown until the next key: a toast would be under this box).
    pub done: Option<String>,
}

/// A captured key that `other` has now: `action` takes it when confirmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Replace {
    pub action: Action,
    pub slot: Slot,
    pub chord: Chord,
    pub other: Action,
}

/// One change to the bindings. It is written to the config and applied to the in-memory keys alike.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Binds `chord` to `value`: an action id, or "none" to unbind it.
    Set { slot: Slot, chord: Chord, value: String },
    /// Deletes the user's line of `chord`, so the default binding applies again.
    Remove { slot: Slot, chord: Chord },
}

impl Edit {
    fn apply_text(&self, text: &str) -> String {
        match self {
            Edit::Set { slot, chord, value } => config::set_binding(text, slot.table(), &chord.config_name(), value),
            Edit::Remove { slot, chord } => config::remove_binding(text, slot.table(), &chord.config_name()),
        }
    }

    fn apply_cfg(&self, keys: &mut KeysCfg) {
        let (slot, chord, value) = match self {
            Edit::Set { slot, chord, value } => (*slot, *chord, Some(value)),
            Edit::Remove { slot, chord } => (*slot, *chord, None),
        };
        let map = match slot {
            Slot::Prefix => &mut keys.prefix_bindings,
            Slot::Direct => &mut keys.direct_bindings,
        };
        // The same chord written in another spelling is the same entry.
        map.retain(|k, _| Chord::parse(k) != Some(chord));
        if let Some(value) = value {
            map.insert(chord.config_name(), value.clone());
        }
    }
}

/// The chord a captured key stands for, or why NOBLE cannot bind it. The checks mirror the config
/// parser (`Chord::parse`) and add the rules a key table needs: not the prefix itself, and no plain
/// letter as a direct key (every shell would lose it).
pub fn capture_chord(k: &KeyEvent, slot: Slot, prefix: Chord) -> Result<Chord, String> {
    // Windows reports AltGr as ctrl+alt: such a key is a character, not a chord.
    if let KeyCode::Char(c) = k.code
        && is_altgr_char(k.modifiers, c)
    {
        return Err("ctrl+alt with a non-letter is AltGr on Windows; use another key".into());
    }
    let chord = Chord::from_event(k);
    if Chord::parse(&chord.config_name()) != Some(chord) {
        return Err("NOBLE cannot bind this key".into());
    }
    if chord == prefix {
        return Err(format!("{prefix} is the prefix key itself"));
    }
    if slot == Slot::Direct && matches!(chord.code, KeyCode::Char(_)) && chord.mods.is_empty() {
        return Err("a plain letter would be taken from every shell: add ctrl, alt or shift".into());
    }
    Ok(chord)
}

/// The box and its columns for a screen; the columns are offsets from the left edge of the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub rect: Rect,
    /// Text width inside the box (one column of padding on each side).
    pub width: u16,
    /// Lines of the list.
    pub rows: usize,
    /// Width of the group column (0 when the box is too narrow for it).
    pub group_w: u16,
    pub title_at: u16,
    pub title_w: u16,
    pub prefix_at: u16,
    pub direct_at: u16,
    pub key_w: u16,
}

/// The box for `area`; `None` when the list does not fit (the overlay then asks for a bigger window).
pub fn geometry(area: Rect) -> Option<Geometry> {
    let w = 100.min(area.width.saturating_sub(2));
    let h = area.height.saturating_sub(2).min(34);
    if w < 40 || h < 10 {
        return None;
    }
    let rect = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 3, w, h);
    let width = w - 4;
    let wide = width >= 80;
    let (group_w, title_at, key_w) = if wide { (7, 8, 16) } else { (0, 0, 9) };
    let title_w = width.saturating_sub(title_at + 2 * key_w + 2);
    let prefix_at = title_at + title_w + 1;
    let direct_at = prefix_at + key_w + 1;
    // Inside the frame: the filter line, the column titles, the list, a status line and the key hints.
    let rows = usize::from((h - 2).saturating_sub(4));
    Some(Geometry { rect, width, rows, group_w, title_at, title_w, prefix_at, direct_at, key_w })
}

impl App {
    /// Opens the editor (Settings → Key bindings).
    pub fn open_bindings(&mut self) {
        self.overlay = Some(Overlay::Bindings(BindingsState::default()));
    }

    /// The chords of an action in one slot, the shortest first.
    pub fn bound_chords(&self, action: Action, slot: Slot) -> Vec<Chord> {
        let map = match slot {
            Slot::Prefix => &self.keymap.prefix_map,
            Slot::Direct => &self.keymap.direct_map,
        };
        chords_of(map, action)
    }

    /// The rows of the list: every action, or those whose name, group, id or keys contain the filter.
    pub fn binding_rows(&self, query: &str) -> Vec<Action> {
        let q = query.trim().to_lowercase();
        Action::ALL
            .iter()
            .copied()
            .filter(|a| {
                q.is_empty() || {
                    let keys: Vec<String> = [Slot::Prefix, Slot::Direct]
                        .into_iter()
                        .flat_map(|s| self.bound_chords(*a, s))
                        .map(|c| c.to_string())
                        .collect();
                    format!("{} {} {} {}", a.title(), a.group(), a.id(), keys.join(" ")).to_lowercase().contains(&q)
                }
            })
            .collect()
    }

    /// The keys of an action in one slot as the list shows them ("—" for none).
    pub fn binding_label(&self, action: Action, slot: Slot) -> String {
        let keys = self.bound_chords(action, slot);
        if keys.is_empty() {
            return "—".into();
        }
        keys.iter().map(|c| c.to_string()).collect::<Vec<_>>().join("  ")
    }

    /// A chord as the user types it in a slot: the prefix chord is written after the prefix key.
    pub fn key_label(&self, slot: Slot, chord: Chord) -> String {
        match slot {
            Slot::Prefix => format!("{} {chord}", self.keymap.prefix),
            Slot::Direct => chord.to_string(),
        }
    }

    /// Keys in the editor; `false` closes it.
    pub(super) fn bindings_key(&mut self, st: &mut BindingsState, k: KeyEvent) -> bool {
        st.note = None;
        st.done = None;
        if let Some((action, slot)) = st.capture {
            if k.code == KeyCode::Esc {
                st.capture = None;
            } else {
                match capture_chord(&k, slot, self.keymap.prefix) {
                    Err(why) => st.note = Some(why),
                    Ok(chord) => {
                        st.capture = None;
                        self.bind_captured(st, action, slot, chord);
                    }
                }
            }
            return true;
        }
        if let Some(r) = st.replace {
            match k.code {
                KeyCode::Char('y' | 'Y') => {
                    st.replace = None;
                    let edits = self.rebind_edits(r.slot, r.action, r.chord);
                    let done = format!("saved · {} is {}", self.key_label(r.slot, r.chord), r.action.title());
                    self.save_bindings(st, &edits, done);
                }
                KeyCode::Char('n' | 'N') | KeyCode::Esc => st.replace = None,
                _ => {}
            }
            return true;
        }
        if st.filtering {
            self.filter_key(st, k);
            return true;
        }
        let rows = self.binding_rows(&st.query);
        let last = rows.len().saturating_sub(1);
        st.selected = st.selected.min(last);
        let sel = rows.get(st.selected).copied();
        // Letters are actions only here; while the filter takes text (`/`) they are typed.
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        match k.code {
            KeyCode::Esc => {
                if st.query.is_empty() {
                    return false;
                }
                st.query.clear();
                st.selected = 0;
            }
            KeyCode::Up => st.selected = st.selected.saturating_sub(1),
            KeyCode::Down => st.selected = (st.selected + 1).min(last),
            KeyCode::PageUp => st.selected = st.selected.saturating_sub(8),
            KeyCode::PageDown => st.selected = (st.selected + 8).min(last),
            KeyCode::Home => st.selected = 0,
            KeyCode::End => st.selected = last,
            KeyCode::Char('/') if !alt => st.filtering = true,
            KeyCode::Enter => {
                if let Some(a) = sel {
                    st.capture = Some((a, Slot::Prefix));
                }
            }
            KeyCode::Char('d') if !alt => {
                if let Some(a) = sel {
                    st.capture = Some((a, Slot::Direct));
                }
            }
            KeyCode::Char('x') | KeyCode::Delete if !alt => {
                if let Some(a) = sel {
                    let edits = self.unbind_edits(a);
                    self.save_bindings(st, &edits, format!("saved · {} unbound", a.title()));
                }
            }
            KeyCode::Char('r') if !alt => {
                if let Some(a) = sel {
                    let edits = self.reset_edits(a);
                    self.save_bindings(st, &edits, format!("saved · {} back to its default keys", a.title()));
                }
            }
            _ => {}
        }
        true
    }

    /// Keys while the filter takes text: letters narrow the rows, ↑↓ move, ⏎ keeps the filter and goes
    /// back to the rows, esc clears it.
    fn filter_key(&mut self, st: &mut BindingsState, k: KeyEvent) {
        let ctrl_or_alt = k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        match k.code {
            KeyCode::Esc => {
                st.filtering = false;
                st.query.clear();
                st.selected = 0;
            }
            KeyCode::Enter => st.filtering = false,
            KeyCode::Backspace => {
                if st.query.pop().is_none() {
                    st.filtering = false;
                }
                st.selected = 0;
            }
            KeyCode::Up => st.selected = st.selected.saturating_sub(1),
            KeyCode::Down => {
                let last = self.binding_rows(&st.query).len().saturating_sub(1);
                st.selected = (st.selected + 1).min(last);
            }
            KeyCode::Char(c) if !ctrl_or_alt => {
                st.query.push(c);
                st.selected = 0;
            }
            _ => {}
        }
    }

    /// A captured key: taken by another action → asked first; free (or already this action's) → bound.
    fn bind_captured(&mut self, st: &mut BindingsState, action: Action, slot: Slot, chord: Chord) {
        match self.slot_map(slot).get(&chord).copied().filter(|other| *other != action) {
            Some(other) => st.replace = Some(Replace { action, slot, chord, other }),
            None => {
                let edits = self.rebind_edits(slot, action, chord);
                let done = format!("saved · {} is {}", self.key_label(slot, chord), action.title());
                self.save_bindings(st, &edits, done);
            }
        }
    }

    fn slot_map(&self, slot: Slot) -> &HashMap<Chord, Action> {
        match slot {
            Slot::Prefix => &self.keymap.prefix_map,
            Slot::Direct => &self.keymap.direct_map,
        }
    }

    /// The action gets exactly `chord` in the slot: its other keys there are unbound.
    fn rebind_edits(&self, slot: Slot, action: Action, chord: Chord) -> Vec<Edit> {
        let mut edits: Vec<Edit> = self
            .bound_chords(action, slot)
            .into_iter()
            .filter(|old| *old != chord)
            .map(|old| Edit::Set { slot, chord: old, value: "none".into() })
            .collect();
        edits.push(Edit::Set { slot, chord, value: action.id() });
        edits
    }

    /// Every key of the action, in both slots, is unbound.
    fn unbind_edits(&self, action: Action) -> Vec<Edit> {
        let mut edits = Vec::new();
        for slot in [Slot::Prefix, Slot::Direct] {
            for chord in self.bound_chords(action, slot) {
                edits.push(Edit::Set { slot, chord, value: "none".into() });
            }
        }
        edits
    }

    /// Back to the defaults: the user's lines for the action, and the default chords of it, are deleted.
    fn reset_edits(&self, action: Action) -> Vec<Edit> {
        let mut edits = Vec::new();
        for slot in [Slot::Prefix, Slot::Direct] {
            let (user, defaults) = match slot {
                Slot::Prefix => (&self.cfg.keys.prefix_bindings, default_prefix_bindings()),
                Slot::Direct => (&self.cfg.keys.direct_bindings, default_direct_bindings()),
            };
            let mut chords: Vec<Chord> = user
                .iter()
                .filter(|(_, value)| Action::from_id(value) == Some(action))
                .filter_map(|(key, _)| Chord::parse(key))
                .collect();
            chords.extend(defaults.iter().filter(|(_, a)| *a == action).map(|(c, _)| *c));
            let mut seen = HashSet::new();
            for chord in chords {
                if seen.insert(chord) {
                    edits.push(Edit::Remove { slot, chord });
                }
            }
        }
        edits
    }

    /// Writes the edits to `config.toml` and reloads the keys from the new settings. Without background
    /// services (tests) the file is left alone and only the keys change. The result shows on the editor's
    /// status line and as a toast (which is only seen once the editor is closed).
    fn save_bindings(&mut self, st: &mut BindingsState, edits: &[Edit], done: String) {
        if self.services.is_some() {
            let text = match config::read_for_edit(&self.paths.config) {
                Ok(text) => text,
                Err(e) => return self.toast(ToastLevel::Error, e),
            };
            let updated = edits.iter().fold(text, |acc, e| e.apply_text(&acc));
            if !self.write_config(updated) {
                return;
            }
        }
        let mut cfg = self.cfg.clone();
        for e in edits {
            e.apply_cfg(&mut cfg.keys);
        }
        self.apply_config(cfg);
        st.done = Some(done.clone());
        self.toast(ToastLevel::Ok, done);
    }

    /// A click on a row selects it; a double click also starts the prefix key of that action.
    pub(super) fn bindings_click(&mut self, row: usize, double: bool) {
        let Some(Overlay::Bindings(st)) = &self.overlay else { return };
        let Some(action) = self.binding_rows(&st.query).get(row).copied() else { return };
        if let Some(Overlay::Bindings(st)) = &mut self.overlay {
            st.selected = row;
            st.capture = double.then_some((action, Slot::Prefix));
            st.replace = None;
            st.note = None;
        }
    }

    /// The wheel moves the selection.
    pub(super) fn bindings_wheel(&mut self, dir: i32) {
        let Some(Overlay::Bindings(st)) = &self.overlay else { return };
        let last = self.binding_rows(&st.query).len().saturating_sub(1);
        if let Some(Overlay::Bindings(st)) = &mut self.overlay {
            st.selected = if dir > 0 { st.selected.saturating_sub(1) } else { (st.selected + 1).min(last) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    fn press(app: &mut App, code: KeyCode, mods: KeyModifiers) {
        app.on_key(key(code, mods));
    }

    fn bindings(app: &App) -> &BindingsState {
        match &app.overlay {
            Some(Overlay::Bindings(st)) => st,
            other => panic!("the editor is open: {}", other.is_some()),
        }
    }

    fn select(app: &mut App, action: Action) {
        let row = app.binding_rows("").iter().position(|a| *a == action).expect("listed");
        if let Some(Overlay::Bindings(st)) = &mut app.overlay {
            st.selected = row;
        }
    }

    #[test]
    fn captured_keys_are_checked_before_they_are_bound() {
        let prefix = Chord::parse("ctrl+a").unwrap();
        let ctrl_alt = KeyModifiers::CONTROL | KeyModifiers::ALT;
        // ctrl+alt on a symbol is AltGr on Windows: refused, as the config parser refuses it.
        assert!(capture_chord(&key(KeyCode::Char('1'), ctrl_alt), Slot::Prefix, prefix).is_err());
        assert_eq!(
            capture_chord(&key(KeyCode::Char('x'), ctrl_alt), Slot::Direct, prefix),
            Ok(Chord::parse("ctrl+alt+x").unwrap())
        );
        // The prefix itself is no key of its own, in either table.
        assert!(capture_chord(&key(KeyCode::Char('a'), KeyModifiers::CONTROL), Slot::Prefix, prefix).is_err());
        assert!(capture_chord(&key(KeyCode::Char('a'), KeyModifiers::CONTROL), Slot::Direct, prefix).is_err());
        // A plain letter cannot be a direct key; after the prefix it can, and so can arrows as direct keys.
        assert!(capture_chord(&key(KeyCode::Char('q'), KeyModifiers::NONE), Slot::Direct, prefix).is_err());
        assert_eq!(
            capture_chord(&key(KeyCode::Char('q'), KeyModifiers::NONE), Slot::Prefix, prefix),
            Ok(Chord::new(KeyCode::Char('q'), KeyModifiers::NONE))
        );
        assert!(capture_chord(&key(KeyCode::Up, KeyModifiers::SHIFT), Slot::Direct, prefix).is_ok());
    }

    #[test]
    fn taken_keys_are_asked_about_before_they_move() {
        let mut app = App::headless(Config::default(), (110, 30));
        app.open_bindings();
        select(&mut app, Action::SplitRight);
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(bindings(&app).capture, Some((Action::SplitRight, Slot::Prefix)));
        // "n" is the prefix key of Next Tab: the question comes first.
        press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
        let n = Chord::parse("n").unwrap();
        assert_eq!(
            bindings(&app).replace,
            Some(Replace { action: Action::SplitRight, slot: Slot::Prefix, chord: n, other: Action::NextTab })
        );
        assert_eq!(app.keymap.prefix_map.get(&n), Some(&Action::NextTab), "nothing moved yet");
        // No keeps everything as it was.
        press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
        assert_eq!(bindings(&app).replace, None);
        assert_eq!(app.keymap.prefix_map.get(&n), Some(&Action::NextTab));
        // Yes: Split Pane Right gets "n" and its old keys (v and |) are unbound; Next Tab loses "n".
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
        press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
        assert_eq!(app.keymap.prefix_map.get(&n), Some(&Action::SplitRight));
        assert_eq!(app.keymap.prefix_map.get(&Chord::parse("v").unwrap()), None);
        assert_eq!(app.keymap.prefix_map.get(&Chord::parse("|").unwrap()), None);
        assert!(app.overlay.is_some(), "the editor stays open");
        assert_eq!(app.binding_label(Action::SplitRight, Slot::Prefix), "n");
    }

    #[test]
    fn refused_keys_say_why_and_keep_waiting() {
        let mut app = App::headless(Config::default(), (110, 30));
        app.open_bindings();
        select(&mut app, Action::NewTab);
        press(&mut app, KeyCode::Char('d'), KeyModifiers::NONE);
        press(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);
        let st = bindings(&app);
        assert!(st.capture.is_some() && st.note.as_deref().is_some_and(|n| n.contains("plain letter")), "{st:?}");
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(bindings(&app).capture.is_none());
        // Typed letters are not actions while the filter takes text.
        press(&mut app, KeyCode::Char('/'), KeyModifiers::NONE);
        press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
        assert_eq!(bindings(&app).query, "x");
        assert!(app.keymap.prefix_map.contains_key(&Chord::parse("t").unwrap()), "x did not unbind");
    }

    #[test]
    fn unbind_and_reset_round_trip() {
        let mut app = App::headless(Config::default(), (110, 30));
        app.open_bindings();
        select(&mut app, Action::NewTab);
        let t = Chord::parse("t").unwrap();
        let alt_t = Chord::parse("alt+t").unwrap();
        press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
        assert_eq!(app.keymap.prefix_map.get(&t), None);
        assert_eq!(app.keymap.direct_map.get(&alt_t), None);
        assert_eq!(app.binding_label(Action::NewTab, Slot::Prefix), "—");
        press(&mut app, KeyCode::Char('r'), KeyModifiers::NONE);
        assert_eq!(app.keymap.prefix_map.get(&t), Some(&Action::NewTab));
        assert_eq!(app.keymap.direct_map.get(&alt_t), Some(&Action::NewTab));
    }

    #[test]
    fn edits_write_the_same_result_to_text_and_to_the_keys() {
        let edits = [
            Edit::Set { slot: Slot::Prefix, chord: Chord::parse("%").unwrap(), value: "split_right".into() },
            Edit::Set { slot: Slot::Direct, chord: Chord::parse("alt+enter").unwrap(), value: "zoom".into() },
            Edit::Remove { slot: Slot::Direct, chord: Chord::parse("alt+enter").unwrap() },
        ];
        let text = edits.iter().fold(String::new(), |acc, e| e.apply_text(&acc));
        let parsed = config::parse(&text).expect("valid config");
        let mut cfg = Config::default().keys;
        for e in &edits {
            e.apply_cfg(&mut cfg);
        }
        assert_eq!(parsed.keys.prefix_bindings, cfg.prefix_bindings);
        assert_eq!(parsed.keys.direct_bindings, cfg.direct_bindings);
        assert_eq!(cfg.prefix_bindings.get("%").map(String::as_str), Some("split_right"));
        assert!(cfg.direct_bindings.is_empty());
    }

    #[test]
    fn edits_keep_the_config_template_valid() {
        let edits = [
            Edit::Set { slot: Slot::Prefix, chord: Chord::parse("%").unwrap(), value: "split_right".into() },
            Edit::Set { slot: Slot::Prefix, chord: Chord::parse("v").unwrap(), value: "none".into() },
            Edit::Set { slot: Slot::Direct, chord: Chord::parse("alt+enter").unwrap(), value: "zoom".into() },
            Edit::Remove { slot: Slot::Direct, chord: Chord::parse("alt+enter").unwrap() },
        ];
        // The first run's file: comments, the launcher blocks and the commented examples all stay.
        let text = edits.iter().fold(config::DEFAULT_CONFIG.to_string(), |acc, e| e.apply_text(&acc));
        assert!(text.contains("# \"%\" = \"split_right\"") && text.contains("[[launchers]]"), "{text}");
        let parsed = config::parse(&text).expect("valid config");
        let mut cfg = config::parse(config::DEFAULT_CONFIG).expect("template").keys;
        for e in &edits {
            e.apply_cfg(&mut cfg);
        }
        assert_eq!(parsed.keys.prefix_bindings, cfg.prefix_bindings);
        assert_eq!(parsed.keys.direct_bindings, cfg.direct_bindings);
        assert_eq!(parsed.keys.prefix_bindings.get("v").map(String::as_str), Some("none"));
    }

    #[test]
    fn the_filter_matches_names_groups_and_keys() {
        let app = App::headless(Config::default(), (110, 30));
        let all = app.binding_rows("").len();
        assert_eq!(all, Action::ALL.len());
        assert!(app.binding_rows("split pane").contains(&Action::SplitRight));
        assert!(app.binding_rows("ctrl+a").is_empty(), "the prefix is not a row's key");
        assert!(app.binding_rows("alt+t").contains(&Action::NewTab));
        assert!(app.binding_rows("NOTHING-LIKE-THIS").is_empty());
    }
}
