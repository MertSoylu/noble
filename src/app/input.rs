//! Interpreting keyboard and mouse input.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use super::{App, Drag, Hit, Overlay, ProjectAct, SortKey, ToastLevel, View};
use crate::keys::{Action, Chord};
use crate::sensors::ProcInfo;
use crate::term::input::{encode_key, encode_mouse};
use crate::term::layout::{PaneId, ratio_from_point};
use crate::term::pane::Selection;
use crate::theme::THEMES;

/// Ctrl held as a shortcut modifier. A character typed with AltGr (Ctrl+Alt on
/// Windows: '@', '\', '{' …) is text, not a shortcut.
pub(super) fn ctrl_held(k: &KeyEvent) -> bool {
    k.modifiers.contains(KeyModifiers::CONTROL)
        && !matches!(k.code, KeyCode::Char(c) if crate::term::input::is_altgr_char(k.modifiers, c))
}

impl App {
    /// Filtered and sorted process list.
    pub fn visible_procs(&self) -> Vec<ProcInfo> {
        let Some(s) = &self.sensors.last else { return Vec::new() };
        let q = self.system.filter.trim().to_lowercase();
        let mut list: Vec<ProcInfo> = s
            .procs
            .iter()
            .filter(|p| q.is_empty() || p.name.to_lowercase().contains(&q) || p.pid.to_string().starts_with(&q))
            .cloned()
            .collect();
        let desc = self.system.desc;
        list.sort_by(|a, b| {
            let ord = match self.system.sort {
                SortKey::Cpu => a.cpu.partial_cmp(&b.cpu).unwrap_or(std::cmp::Ordering::Equal),
                SortKey::Mem => a.mem.cmp(&b.mem),
                SortKey::Pid => a.pid.cmp(&b.pid),
                SortKey::Name => b.name.to_lowercase().cmp(&a.name.to_lowercase()),
            };
            if desc { ord.reverse() } else { ord }
        });
        list
    }

    pub(super) fn on_input(&mut self, e: Event) {
        match e {
            Event::Key(k) if k.kind != KeyEventKind::Release => self.on_key(k),
            Event::Mouse(m) => self.on_mouse(m),
            Event::Resize(w, h) => self.size = (w, h),
            Event::Paste(text) => self.on_paste(&text),
            _ => {}
        }
    }

    fn on_paste(&mut self, text: &str) {
        match &mut self.overlay {
            Some(Overlay::Palette(st)) => {
                st.query.push_str(text.lines().next().unwrap_or(""));
                st.refilter();
                return;
            }
            Some(Overlay::Prompt(p)) => {
                let line: String = text.lines().next().unwrap_or("").chars().take(p.purpose.max_len()).collect();
                p.value.push_str(&line);
                return;
            }
            Some(_) => return,
            None => {}
        }
        if let Some(id) = self.focused_pane().filter(|id| self.panes.contains_key(id)) {
            self.paste_into(id, text.to_string());
        } else if self.view == View::Bridge && self.bridge.filtering {
            self.bridge.filter.push_str(text.lines().next().unwrap_or(""));
            self.bridge.proj_sel = 0;
        }
    }

    pub fn on_key(&mut self, k: KeyEvent) {
        if self.boot.take().is_some() {
            return;
        }
        if self.overlay.is_some() {
            self.overlay_key(k);
            return;
        }
        if self.search.is_some() && !self.prefix_armed && self.search_key(k) {
            return;
        }
        let in_term = matches!(self.view, View::Term(_));
        // Only while the same pane is focused in a terminal; anything else drops it.
        if self.pass_next.take().is_some_and(|id| in_term && self.focused_pane() == Some(id)) {
            self.term_key(k);
            return;
        }
        let chord = Chord::from_event(&k);
        if self.prefix_armed {
            self.prefix_armed = false;
            if chord == self.keymap.prefix {
                self.run(Action::SendPrefix);
            } else if k.code != KeyCode::Esc {
                match self.keymap.prefix_map.get(&chord).copied() {
                    Some(a) => self.run(a),
                    // "once": prefix + a direct shortcut sends that key to the app.
                    None if in_term && self.pass_once() && self.keymap.direct_map.contains_key(&chord) => {
                        self.term_key(k)
                    }
                    None => self
                        .toast(ToastLevel::Warn, format!("{} {chord} is not bound — ? for help", self.keymap.prefix)),
                }
            }
            return;
        }
        if chord == self.keymap.prefix {
            self.prefix_armed = true;
            return;
        }
        if let Some(a) = self.keymap.direct_map.get(&chord).copied()
            && !(in_term && (self.focused_locked() || self.keymap.shell_first.contains(&chord)))
        {
            self.run(a);
            return;
        }
        match self.view {
            View::Bridge => self.bridge_key(k),
            View::System => self.system_key(k),
            View::Settings => self.settings_key(k),
            View::Term(_) => self.term_key(k),
        }
    }

    fn term_key(&mut self, k: KeyEvent) {
        let Some(id) = self.focused_pane() else { return };
        let Some(p) = self.panes.get_mut(&id) else { return };
        p.selection = None;
        p.scroll_reset();
        let (app_cursor, alt) = {
            let parser = p.parser();
            (parser.screen().application_cursor(), parser.screen().alternate_screen())
        };
        // In a shell Enter starts a command: we time its end (prompt return).
        if k.code == KeyCode::Enter && !alt {
            p.command_started = Some(Instant::now());
        }
        p.write(&encode_key(&k, app_cursor));
    }

    fn overlay_key(&mut self, k: KeyEvent) {
        if matches!(self.overlay, Some(Overlay::Menu(_))) {
            self.menu_key(k);
            return;
        }
        let Some(mut ov) = self.overlay.take() else { return };
        let ctrl = ctrl_held(&k);
        let keep = match &mut ov {
            Overlay::Palette(st) => match k.code {
                KeyCode::Esc => false,
                KeyCode::Enter => {
                    if let Some(item) = st.current().cloned() {
                        self.run_palette(item.cmd);
                    }
                    // The command may have opened a new overlay (e.g. rename).
                    return;
                }
                KeyCode::Up => {
                    st.selected = st.selected.saturating_sub(1);
                    true
                }
                KeyCode::Char('p' | 'k') if ctrl => {
                    st.selected = st.selected.saturating_sub(1);
                    true
                }
                KeyCode::Down | KeyCode::Tab => {
                    st.selected = (st.selected + 1).min(st.matches.len().saturating_sub(1));
                    true
                }
                KeyCode::Char('n' | 'j') if ctrl => {
                    st.selected = (st.selected + 1).min(st.matches.len().saturating_sub(1));
                    true
                }
                KeyCode::PageUp => {
                    st.selected = st.selected.saturating_sub(8);
                    true
                }
                KeyCode::PageDown => {
                    st.selected = (st.selected + 8).min(st.matches.len().saturating_sub(1));
                    true
                }
                KeyCode::Home => {
                    st.selected = 0;
                    true
                }
                KeyCode::End => {
                    st.selected = st.matches.len().saturating_sub(1);
                    true
                }
                KeyCode::Backspace => {
                    st.query.pop();
                    st.refilter();
                    true
                }
                KeyCode::Char('u') if ctrl => {
                    st.query.clear();
                    st.refilter();
                    true
                }
                KeyCode::Char(c) if !ctrl => {
                    st.query.push(c);
                    st.selected = 0;
                    st.refilter();
                    true
                }
                _ => true,
            },
            // Menu keys are handled in `menu_key`; they never reach here.
            Overlay::Menu(_) => true,
            Overlay::Welcome { prefix } => {
                let n = super::settings::PREFIXES.len();
                match k.code {
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        let p = *prefix;
                        self.finish_welcome(p, true);
                        return;
                    }
                    KeyCode::Esc => {
                        let p = *prefix;
                        self.finish_welcome(p, false);
                        return;
                    }
                    KeyCode::Left | KeyCode::Up | KeyCode::Char('h' | 'k') => *prefix = (*prefix + n - 1) % n,
                    KeyCode::Right | KeyCode::Down | KeyCode::Tab | KeyCode::Char('l' | 'j') => {
                        *prefix = (*prefix + 1) % n
                    }
                    _ => {}
                }
                true
            }
            Overlay::Themes(p) => {
                let n = THEMES.len();
                let to = match k.code {
                    KeyCode::Up | KeyCode::Char('k') => Some(p.selected.saturating_sub(1)),
                    KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => Some((p.selected + 1).min(n - 1)),
                    KeyCode::PageUp => Some(p.selected.saturating_sub(8)),
                    KeyCode::PageDown => Some((p.selected + 8).min(n - 1)),
                    KeyCode::Home => Some(0),
                    KeyCode::End => Some(n - 1),
                    _ => None,
                };
                match k.code {
                    KeyCode::Esc => {
                        self.cancel_picker(&ov);
                        false
                    }
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        self.set_theme(THEMES[p.selected].name);
                        false
                    }
                    _ => {
                        if let Some(to) = to {
                            p.selected = to;
                            self.preview_theme(to);
                        }
                        true
                    }
                }
            }
            Overlay::Schemes(p) => {
                let n = self.scheme_options().len();
                let step = |p: &mut super::SchemePicker, d: i32| {
                    p.selected = (p.selected as i32 + d).clamp(0, n as i32 - 1) as usize;
                };
                match k.code {
                    KeyCode::Esc => {
                        // Undo the preview.
                        self.cfg.terminal.colors = p.original.clone();
                        false
                    }
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        if let Some(name) = self.scheme_options().get(p.selected).cloned() {
                            self.set_term_colors(&name);
                        }
                        false
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        step(p, -1);
                        self.preview_scheme(p.selected);
                        true
                    }
                    KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                        step(p, 1);
                        self.preview_scheme(p.selected);
                        true
                    }
                    KeyCode::PageUp => {
                        step(p, -8);
                        self.preview_scheme(p.selected);
                        true
                    }
                    KeyCode::PageDown => {
                        step(p, 8);
                        self.preview_scheme(p.selected);
                        true
                    }
                    KeyCode::Home => {
                        p.selected = 0;
                        self.preview_scheme(0);
                        true
                    }
                    KeyCode::End => {
                        p.selected = n.saturating_sub(1);
                        self.preview_scheme(p.selected);
                        true
                    }
                    _ => true,
                }
            }
            Overlay::Launchers { selected } => {
                let list = self.installed_launchers();
                let sel = (*selected).min(list.len().saturating_sub(1));
                match k.code {
                    KeyCode::Esc | KeyCode::Char('q') => false,
                    KeyCode::Up | KeyCode::Char('k') => {
                        *selected = sel.saturating_sub(1);
                        true
                    }
                    KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                        *selected = (sel + 1).min(list.len().saturating_sub(1));
                        true
                    }
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        if let Some(&i) = list.get(sel) {
                            self.change_launcher(i, false, 1);
                        }
                        true
                    }
                    KeyCode::Left | KeyCode::Right | KeyCode::Char('h' | 'l') => {
                        let dir = if matches!(k.code, KeyCode::Left | KeyCode::Char('h')) { -1 } else { 1 };
                        if let Some(&i) = list.get(sel) {
                            self.change_launcher(i, true, dir);
                        }
                        true
                    }
                    _ => true,
                }
            }
            Overlay::Help { scroll } => match k.code {
                KeyCode::Esc | KeyCode::Char('q' | '?') | KeyCode::Enter => false,
                KeyCode::Up | KeyCode::Char('k') => {
                    *scroll = scroll.saturating_sub(1);
                    true
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    *scroll = scroll.saturating_add(1);
                    true
                }
                KeyCode::PageUp => {
                    *scroll = scroll.saturating_sub(10);
                    true
                }
                KeyCode::PageDown | KeyCode::Char(' ') => {
                    *scroll = scroll.saturating_add(10);
                    true
                }
                KeyCode::Home => {
                    *scroll = 0;
                    true
                }
                _ => true,
            },
            Overlay::Confirm(_) => match k.code {
                KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
                    if let Overlay::Confirm(c) = ov {
                        self.confirm(c.action);
                    }
                    return;
                }
                KeyCode::Char('n' | 'N') | KeyCode::Esc => false,
                _ => true,
            },
            Overlay::Prompt(p) => match k.code {
                KeyCode::Esc => false,
                KeyCode::Enter => {
                    if let Overlay::Prompt(p) = ov {
                        self.submit_prompt(p);
                    }
                    return;
                }
                KeyCode::Backspace => {
                    p.value.pop();
                    true
                }
                KeyCode::Char('u') if ctrl => {
                    p.value.clear();
                    true
                }
                KeyCode::Char(c) if !ctrl => {
                    if p.value.chars().count() < p.purpose.max_len() {
                        p.value.push(c);
                    }
                    true
                }
                _ => true,
            },
        };
        if keep && self.overlay.is_none() {
            self.overlay = Some(ov);
        }
    }

    fn move_project(&mut self, delta: i32) {
        let n = self.visible_projects().len();
        if n == 0 {
            return;
        }
        self.bridge.proj_sel = (self.bridge.proj_sel as i32 + delta).clamp(0, n as i32 - 1) as usize;
    }

    /// Directory of the selected project (for launchers and "open folder").
    fn bridge_target_dir(&self) -> Option<PathBuf> {
        self.selected_project().map(|p| p.path.clone())
    }

    fn open_selected(&mut self) {
        match self.selected_project().map(|p| p.path.clone()) {
            Some(p) => self.open_project_shell(p),
            None => self.run(Action::NewTab),
        }
    }

    fn bridge_key(&mut self, k: KeyEvent) {
        let ctrl = ctrl_held(&k);
        if self.bridge.filtering {
            match k.code {
                KeyCode::Esc => {
                    self.bridge.filtering = false;
                    self.bridge.filter.clear();
                }
                KeyCode::Enter => {
                    self.bridge.filtering = false;
                    if self.selected_project().is_some() {
                        self.open_selected();
                        self.bridge.filter.clear();
                    }
                }
                KeyCode::Backspace => {
                    if self.bridge.filter.pop().is_none() {
                        self.bridge.filtering = false;
                    }
                    self.bridge.proj_sel = 0;
                }
                KeyCode::Up => self.move_project(-1),
                KeyCode::Down => self.move_project(1),
                KeyCode::Char(c) if !ctrl => {
                    self.bridge.filter.push(c);
                    self.bridge.proj_sel = 0;
                }
                _ => {}
            }
            return;
        }
        // The focused quick action lasts only while ←/→/⏎ act on it; any other key leaves it.
        let act = self.bridge.proj_act.take();
        match (k.code, act) {
            (KeyCode::Right, None) if self.selected_project().is_some() => {
                self.bridge.proj_act = Some(ProjectAct::Pin);
                return;
            }
            (KeyCode::Right, Some(_)) => {
                self.bridge.proj_act = Some(ProjectAct::More);
                return;
            }
            (KeyCode::Left, Some(ProjectAct::More)) => {
                self.bridge.proj_act = Some(ProjectAct::Pin);
                return;
            }
            (KeyCode::Left | KeyCode::Esc, Some(_)) => return,
            (KeyCode::Enter, Some(ProjectAct::Pin)) => {
                if let Some(p) = self.bridge_target_dir() {
                    self.toggle_pin(&p);
                }
                self.bridge.proj_act = Some(ProjectAct::Pin);
                return;
            }
            (KeyCode::Enter, Some(ProjectAct::More)) => {
                let row = self.bridge.proj_sel;
                // The menu opens under the ⋯ button, as with a click.
                let at = self.hits.iter().find(|(_, h)| *h == Hit::ProjectAct(row, ProjectAct::More)).map(|(r, _)| r);
                let (x, y) = at.map(|r| (r.x, r.y)).unwrap_or((0, 0));
                self.project_action(row, ProjectAct::More, x, y + 1);
                return;
            }
            _ => {}
        }
        let launcher = match k.code {
            KeyCode::Char(c) if !ctrl && !k.modifiers.contains(KeyModifiers::ALT) => {
                self.quick_launchers().find(|(_, l)| l.key.starts_with(c)).map(|(i, _)| i)
            }
            _ => None,
        };
        if let Some(i) = launcher {
            let dir = self.bridge_target_dir();
            self.launch(i, dir);
            return;
        }
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_project(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_project(1),
            KeyCode::PageUp => self.move_project(-8),
            KeyCode::PageDown => self.move_project(8),
            KeyCode::Home => self.move_project(-10_000),
            KeyCode::End => self.move_project(10_000),
            KeyCode::Enter => self.open_selected(),
            KeyCode::Char('/') => {
                self.bridge.filtering = true;
                self.bridge.filter.clear();
                self.bridge.proj_sel = 0;
            }
            KeyCode::Esc => self.bridge.filter.clear(),
            KeyCode::Char('t') => self.run(Action::NewTab),
            KeyCode::Char('m') => self.run(Action::System),
            KeyCode::Char('s') => self.run(Action::Settings),
            KeyCode::Char('p' | ':') => self.run(Action::Palette),
            KeyCode::Char('?') => self.run(Action::Help),
            KeyCode::Char('q') => self.request_quit(),
            KeyCode::Char('c') if ctrl => self.request_quit(),
            KeyCode::Char('w') => self.run(Action::SaveWorkspace),
            KeyCode::Char('r') => self.rescan_projects(),
            KeyCode::Char('R') => self.refresh_ai(),
            KeyCode::Char('a') => self.run(Action::AddProjectFolder),
            KeyCode::Char('o') => {
                if let Some(p) = self.bridge_target_dir() {
                    self.open_in_explorer(&p);
                }
            }
            KeyCode::Char(c @ '1'..='9') => self.go_tab(c as usize - '0' as usize),
            _ => {}
        }
    }

    fn system_key(&mut self, k: KeyEvent) {
        let ctrl = ctrl_held(&k);
        if self.system.filtering {
            match k.code {
                KeyCode::Esc => {
                    self.system.filtering = false;
                    self.system.filter.clear();
                }
                KeyCode::Enter => self.system.filtering = false,
                KeyCode::Backspace => {
                    if self.system.filter.pop().is_none() {
                        self.system.filtering = false;
                    }
                }
                KeyCode::Up => self.move_proc(-1),
                KeyCode::Down => self.move_proc(1),
                KeyCode::Char(c) if !ctrl => self.system.filter.push(c),
                _ => {}
            }
            return;
        }
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_proc(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_proc(1),
            KeyCode::PageUp => self.move_proc(-15),
            KeyCode::PageDown => self.move_proc(15),
            KeyCode::Home => self.move_proc(-100_000),
            KeyCode::End => self.move_proc(100_000),
            KeyCode::Char('c') if !ctrl => self.set_sort(SortKey::Cpu),
            KeyCode::Char('m') => self.set_sort(SortKey::Mem),
            KeyCode::Char('p') => self.set_sort(SortKey::Pid),
            KeyCode::Char('n') => self.set_sort(SortKey::Name),
            KeyCode::Char('/') => {
                self.system.filtering = true;
                self.system.filter.clear();
            }
            KeyCode::Char('K') | KeyCode::Delete => {
                let procs = self.visible_procs();
                if let Some(p) = self.system.selected_pid.and_then(|pid| procs.iter().find(|p| p.pid == pid)) {
                    self.request_kill(p.pid, p.name.clone());
                } else {
                    self.toast(ToastLevel::Info, "select a process first (↑/↓)");
                }
            }
            KeyCode::Esc => {
                if self.system.filter.is_empty() {
                    self.view = View::Bridge;
                } else {
                    self.system.filter.clear();
                }
            }
            KeyCode::Char('q') => self.view = View::Bridge,
            KeyCode::Char('c') if ctrl => self.view = View::Bridge,
            KeyCode::Char('?') => self.run(Action::Help),
            KeyCode::Char(':') => self.run(Action::Palette),
            KeyCode::Char(c @ '1'..='9') => self.go_tab(c as usize - '0' as usize),
            _ => {}
        }
    }

    fn set_sort(&mut self, key: SortKey) {
        if self.system.sort == key {
            self.system.desc = !self.system.desc;
        } else {
            self.system.sort = key;
            self.system.desc = !matches!(key, SortKey::Name | SortKey::Pid);
        }
    }

    fn move_proc(&mut self, delta: i32) {
        let procs = self.visible_procs();
        if procs.is_empty() {
            return;
        }
        let cur = self.system.selected_pid.and_then(|pid| procs.iter().position(|p| p.pid == pid));
        let next = match cur {
            Some(i) => (i as i32 + delta).clamp(0, procs.len() as i32 - 1) as usize,
            None => 0,
        };
        self.system.selected_pid = Some(procs[next].pid);
    }

    // ─── Fare ───────────────────────────────────────────────────────────────

    fn hit_at(&self, x: u16, y: u16) -> Option<Hit> {
        self.hits.iter().rev().find(|(r, _)| r.contains(Position { x, y })).map(|(_, h)| h.clone())
    }

    pub(super) fn tab_of(&self, pane: PaneId) -> Option<usize> {
        self.tabs.iter().position(|t| t.root.contains(pane))
    }

    fn forward_mouse(&self, pane: PaneId, inner: Rect, m: &MouseEvent) -> bool {
        let Some(p) = self.panes.get(&pane) else { return false };
        let (mode, enc) = {
            let parser = p.parser();
            (parser.screen().mouse_protocol_mode(), parser.screen().mouse_protocol_encoding())
        };
        let col = m.column.saturating_sub(inner.x).min(inner.width.saturating_sub(1));
        let row = m.row.saturating_sub(inner.y).min(inner.height.saturating_sub(1));
        match encode_mouse(m, col, row, mode, enc) {
            Some(bytes) => {
                p.write(&bytes);
                true
            }
            None => false,
        }
    }

    fn pane_wants_mouse(&self, pane: PaneId) -> bool {
        self.panes
            .get(&pane)
            .map(|p| p.parser().screen().mouse_protocol_mode() != vt100::MouseProtocolMode::None)
            .unwrap_or(false)
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        self.hover = Some((m.column, m.row));
        if self.boot.is_some() {
            if matches!(m.kind, MouseEventKind::Down(_)) {
                self.boot = None;
            }
            return;
        }
        match m.kind {
            MouseEventKind::Down(btn) => self.mouse_down(btn, &m),
            MouseEventKind::Drag(MouseButton::Left) => self.mouse_drag(&m),
            MouseEventKind::Drag(_) => {
                if let Some(Drag::Forward { pane, inner }) = &self.drag {
                    self.forward_mouse(*pane, *inner, &m);
                }
            }
            MouseEventKind::Up(_) => self.mouse_up(&m),
            MouseEventKind::ScrollUp => self.mouse_wheel(&m, 1),
            MouseEventKind::ScrollDown => self.mouse_wheel(&m, -1),
            MouseEventKind::Moved => {
                // Moving without a button: the release was lost (e.g. outside the window), end a divider
                // drag so the deferred PTY resize happens.
                if matches!(self.drag, Some(Drag::Divider { .. })) {
                    self.drag = None;
                }
                let over = match self.hit_at(m.column, m.row) {
                    Some(Hit::Pane { pane, inner }) if self.overlay.is_none() => Some((pane, inner)),
                    _ => None,
                };
                self.update_link_hover(over, &m);
                if let Some((pane, inner)) = over
                    && Some(pane) == self.focused_pane()
                {
                    self.forward_mouse(pane, inner, &m);
                }
            }
            _ => {}
        }
    }

    fn mouse_down(&mut self, btn: MouseButton, m: &MouseEvent) {
        let (x, y) = (m.column, m.row);
        let double =
            self.last_click.is_some_and(|(t, lx, ly)| t.elapsed() < Duration::from_millis(450) && lx == x && ly == y);
        self.last_click = Some((Instant::now(), x, y));
        // A click ends the keyboard focus on a project's quick actions.
        self.bridge.proj_act = None;
        let Some(hit) = self.hit_at(x, y) else { return };
        let shift = m.modifiers.contains(KeyModifiers::SHIFT);
        // Right click: context menu for a tab, a pane title and a project.
        if btn == MouseButton::Right && self.overlay.is_none() {
            match &hit {
                Hit::Tab(i) => return self.open_tab_menu(*i, x, y + 1),
                Hit::PaneTitle(p) => return self.open_pane_menu(*p, x, y + 1),
                Hit::Project(row) => {
                    self.bridge.proj_sel = *row;
                    if let Some(p) = self.selected_project().map(|p| p.path.clone()) {
                        self.open_project_menu(p, x, y + 1);
                    }
                    return;
                }
                _ => {}
            }
        }
        match hit {
            Hit::Backdrop if matches!(self.overlay, Some(Overlay::Welcome { .. })) => {}
            Hit::Backdrop => {
                // Clicking outside a selector cancels it, preview included.
                if let Some(ov) = self.overlay.take() {
                    self.cancel_picker(&ov);
                }
            }
            Hit::Inert => {}
            Hit::TabBridge => self.view = View::Bridge,
            Hit::TabSystem => self.view = View::System,
            Hit::TabSettings => self.open_settings(),
            Hit::Tab(i) => {
                if double && self.view == View::Term(i) {
                    self.rename_tab_prompt(i);
                } else {
                    self.go_tab(i + 1);
                    self.drag = Some(Drag::Tab { index: i });
                }
            }
            Hit::PaneTitle(p) => {
                self.focus_pane(p);
            }
            Hit::MenuItem(i) => {
                let cmd = match &self.overlay {
                    Some(Overlay::Menu(m)) => m.items.get(i).filter(|it| it.enabled).map(|it| it.cmd.clone()),
                    _ => None,
                };
                if let Some(cmd) = cmd {
                    self.run_menu(cmd);
                }
            }
            Hit::ProjectAct(row, act) => self.project_action(row, act, x, y + 1),
            Hit::Update => self.start_update(),
            Hit::UpdateDismiss => self.dismiss_update(),
            Hit::TabClose(i) => self.request_close_tab(i),
            Hit::NewTab => self.run(Action::NewTab),
            Hit::Pane { pane, inner } => {
                if let Some(ti) = self.tab_of(pane) {
                    self.tabs[ti].focus = pane;
                }
                let ctrl = m.modifiers.contains(KeyModifiers::CONTROL);
                if btn == MouseButton::Left && ctrl && self.open_link_at(pane, inner, m) {
                    return;
                }
                let wants = self.pane_wants_mouse(pane) && !shift;
                match btn {
                    MouseButton::Left => {
                        if wants {
                            self.forward_mouse(pane, inner, m);
                            self.drag = Some(Drag::Forward { pane, inner });
                        } else {
                            for p in self.panes.values_mut() {
                                p.selection = None;
                            }
                            if let Some(p) = self.panes.get_mut(&pane) {
                                let at = (y.saturating_sub(inner.y), x.saturating_sub(inner.x));
                                p.selection = Some(Selection { anchor: at, head: at });
                            }
                            self.drag = Some(Drag::Select { pane, inner });
                        }
                    }
                    MouseButton::Right | MouseButton::Middle => {
                        if wants {
                            self.forward_mouse(pane, inner, m);
                        } else {
                            self.paste_clipboard(pane);
                        }
                    }
                }
            }
            Hit::PaneZoom(id) => {
                if let Some(ti) = self.tab_of(id) {
                    self.toggle_zoom(ti, id);
                }
            }
            Hit::PaneClose(id) => self.request_close_pane(id),
            Hit::Divider { tab, div } => self.drag = Some(Drag::Divider { tab, div }),
            Hit::Project(i) => {
                if self.bridge.proj_sel == i && double {
                    self.open_selected();
                } else {
                    self.bridge.proj_sel = i;
                }
            }
            Hit::OpenSelected => self.open_selected(),
            Hit::AiRefresh => self.refresh_ai(),
            Hit::Setting(i)
                if self.settings_items().get(i)
                    == Some(&super::SettingItem::Setting(super::SettingKey::TermColors)) =>
            {
                self.settings_sel = i;
                self.open_scheme_picker();
            }
            Hit::Setting(i)
                if self.settings_items().get(i) == Some(&super::SettingItem::Setting(super::SettingKey::Theme)) =>
            {
                self.settings_sel = i;
                self.open_theme_picker();
            }
            Hit::Setting(i) => {
                self.settings_sel = i;
                if let Some(item) = self.settings_items().get(i).copied() {
                    self.activate_setting(item, 1);
                }
            }
            Hit::LaunchShow(i) | Hit::LaunchKey(i) => {
                if let Some(Overlay::Launchers { selected }) = &mut self.overlay
                    && let Some(row) =
                        self.launchers.iter().enumerate().filter(|(_, (_, ok))| *ok).position(|(j, _)| j == i)
                {
                    *selected = row;
                }
                self.change_launcher(i, matches!(hit, Hit::LaunchKey(_)), 1);
            }
            Hit::ThemeOption(i) => {
                if let Some(t) = THEMES.get(i) {
                    self.overlay = None;
                    self.set_theme(t.name);
                }
            }
            Hit::TermScheme(i) => {
                if let Some(name) = self.scheme_options().get(i).cloned() {
                    self.overlay = None;
                    self.set_term_colors(&name);
                }
            }
            Hit::PaneSplit { pane, dir } => {
                if let Some(ti) = self.tab_of(pane) {
                    self.tabs[ti].focus = pane;
                    self.view = View::Term(ti);
                    self.split(dir);
                }
            }
            Hit::Launcher(i) => {
                let dir = self.bridge_target_dir();
                self.launch(i, dir);
            }
            Hit::OpenFiles => {
                if let Some(p) = self.bridge_target_dir() {
                    self.open_in_explorer(&p);
                }
            }
            Hit::Proc(pid) => self.system.selected_pid = Some(pid),
            Hit::SortCol(k) => self.set_sort(k),
            Hit::PaletteItem(i) => {
                if let Some(Overlay::Palette(st)) = &mut self.overlay {
                    st.selected = i;
                    if let Some(item) = st.current().cloned() {
                        self.overlay = None;
                        self.run_palette(item.cmd);
                    }
                }
            }
            Hit::ConfirmYes => {
                if let Some(Overlay::Confirm(c)) = self.overlay.take() {
                    self.confirm(c.action);
                }
            }
            Hit::ConfirmNo => self.overlay = None,
            Hit::WelcomePrefix(i) => {
                if let Some(Overlay::Welcome { prefix }) = &mut self.overlay {
                    *prefix = i;
                }
            }
            Hit::WelcomeDone => {
                if let Some(Overlay::Welcome { prefix }) = &self.overlay {
                    let p = *prefix;
                    self.finish_welcome(p, true);
                }
            }
        }
    }

    fn mouse_drag(&mut self, m: &MouseEvent) {
        let (x, y) = (m.column, m.row);
        match &self.drag {
            Some(Drag::Divider { tab, div }) => {
                let ratio = ratio_from_point(div, x, y);
                if let Some(t) = self.tabs.get_mut(*tab) {
                    t.root.set_ratio(&div.path, ratio);
                }
            }
            Some(Drag::Select { pane, inner }) => {
                let (pane, inner) = (*pane, *inner);
                let row = y.clamp(inner.y, inner.bottom().saturating_sub(1)) - inner.y;
                let col = x.clamp(inner.x, inner.right().saturating_sub(1)) - inner.x;
                if let Some(p) = self.panes.get_mut(&pane)
                    && let Some(sel) = &mut p.selection
                {
                    sel.head = (row, col);
                }
            }
            Some(Drag::Forward { pane, inner }) => {
                let (pane, inner) = (*pane, *inner);
                self.forward_mouse(pane, inner, m);
            }
            Some(Drag::Tab { index }) => {
                let from = *index;
                // Each tab's slot on the bar: its hit rect plus the "× " after it.
                let slot = |i: usize| {
                    self.hits.iter().find(|(_, h)| *h == Hit::Tab(i)).map(|(r, _)| (r.x, r.right() + 2, r.y))
                };
                let Some((f0, f1, bar_y)) = slot(from) else { return };
                let target = (0..self.tabs.len())
                    .filter(|&i| i != from)
                    .find_map(|i| slot(i).filter(|&(a, b, _)| x >= a && x < b).map(|(a, b, _)| (i, a, b)));
                let Some((to, t0, t1)) = target else { return };
                // Only move once the pointer would be over the dragged tab after the move; with tabs of
                // different widths it would otherwise land on the other tab and swap back (jitter).
                let w = f1 - f0;
                let past = if to > from { x >= t1.saturating_sub(w) } else { x < t0 + w };
                if past && y <= bar_y + 1 {
                    self.move_tab(from, to);
                    self.drag = Some(Drag::Tab { index: to });
                    // The tab rects are stale until the next frame; several drag events can arrive
                    // before it, so no further move until then.
                    self.hits.retain(|(_, h)| !matches!(h, Hit::Tab(_) | Hit::TabClose(_)));
                }
            }
            None => {}
        }
    }

    fn mouse_up(&mut self, m: &MouseEvent) {
        match self.drag.take() {
            Some(Drag::Select { pane, .. }) => {
                let text = self.panes.get(&pane).and_then(|p| p.selected_text());
                match text {
                    Some(t) if self.cfg.terminal.copy_on_select => self.set_clipboard(&t, true),
                    Some(_) => {}
                    None => {
                        if let Some(p) = self.panes.get_mut(&pane) {
                            p.selection = None;
                        }
                    }
                }
            }
            Some(Drag::Forward { pane, inner }) => {
                self.forward_mouse(pane, inner, m);
            }
            _ => {}
        }
    }

    fn mouse_wheel(&mut self, m: &MouseEvent, dir: i32) {
        if let Some(Overlay::Help { scroll }) = &mut self.overlay {
            *scroll = if dir > 0 { scroll.saturating_sub(3) } else { scroll.saturating_add(3) };
            return;
        }
        if let Some(Overlay::Themes(p)) = &mut self.overlay {
            p.selected = if dir > 0 { p.selected.saturating_sub(1) } else { (p.selected + 1).min(THEMES.len() - 1) };
            let sel = p.selected;
            self.preview_theme(sel);
            return;
        }
        if let Some(Overlay::Schemes(p)) = &mut self.overlay {
            let n = self.term_schemes.len() + 1;
            p.selected = if dir > 0 { p.selected.saturating_sub(1) } else { (p.selected + 1).min(n - 1) };
            let sel = p.selected;
            self.preview_scheme(sel);
            return;
        }
        if let Some(Overlay::Palette(st)) = &mut self.overlay {
            st.selected = if dir > 0 {
                st.selected.saturating_sub(1)
            } else {
                (st.selected + 1).min(st.matches.len().saturating_sub(1))
            };
            return;
        }
        if self.overlay.is_some() {
            return;
        }
        if self.view == View::Settings {
            // The wheel scrolls the page (clamped when drawn); the selection stays where it is.
            self.settings_scroll =
                if dir > 0 { self.settings_scroll.saturating_sub(3) } else { self.settings_scroll.saturating_add(3) };
            self.settings_follow = false;
            return;
        }
        match self.hit_at(m.column, m.row) {
            Some(Hit::Pane { pane, inner }) => {
                if self.pane_wants_mouse(pane) {
                    self.forward_mouse(pane, inner, m);
                    return;
                }
                let Some(p) = self.panes.get(&pane) else { return };
                let (alt, app_cursor) = {
                    let parser = p.parser();
                    (parser.screen().alternate_screen(), parser.screen().application_cursor())
                };
                if alt {
                    // On the alternate screen (less, man…) the wheel becomes arrow keys.
                    let code = if dir > 0 { KeyCode::Up } else { KeyCode::Down };
                    let bytes = encode_key(&KeyEvent::new(code, KeyModifiers::NONE), app_cursor);
                    for _ in 0..3 {
                        p.write(&bytes);
                    }
                } else {
                    p.scroll(dir * 3);
                }
            }
            Some(Hit::Project(_)) => self.move_project(-dir),
            Some(Hit::Proc(_)) => self.move_proc(-dir * 3),
            _ => {}
        }
    }
}
