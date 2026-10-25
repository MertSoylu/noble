//! Scrollback search and ctrl+click links.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

use super::input::ctrl_held;
use super::{App, Hit, LinkHover, SearchState, ToastLevel};
use crate::term::layout::PaneId;
use crate::term::link;
use crate::term::pane::{Match, find_matches};

/// Output while the search is open recomputes the matches at most this often: a burst of output costs one
/// refresh per interval, and the last output always gets its refresh (`App::refresh_search_when_due`).
pub(super) const SEARCH_REFRESH: Duration = Duration::from_millis(100);

/// The selection after the matches were computed again on a text with `dropped` lines above it and numbering `epoch`
/// (`new`, `capped`): `(current, detached)`, the newest match when nothing is kept.
///
/// Lines keep a stable number (`dropped + line`) while output scrolls and while the screen grows or shrinks, so the
/// selection moves by how far the first line moved. A reflow renumbers the lines: then the selection is the same
/// place counted from the newest match (newer lines are the last to leave the scrollback), clamped.
fn carry_selection(
    s: &SearchState,
    new: &[Match],
    capped: bool,
    dropped: usize,
    epoch: u64,
) -> (Option<usize>, Option<Match>) {
    // The selected match: the one selected, or the one the selection fell out of the window on.
    let selected = match s.current {
        Some(i) => s.matches.get(i).copied(),
        None => s.detached,
    };
    let mut current = None;
    let mut detached = None;
    if let Some(m) = selected {
        if epoch != s.epoch {
            if let Some(i) = s.current {
                let newer = s.matches.len().saturating_sub(i + 1);
                current = new.len().checked_sub(1 + newer);
            }
        } else if let Some(line) = (m.line + s.dropped).checked_sub(dropped) {
            match new.iter().position(|n| n.line == line && n.col == m.col) {
                Some(i) => current = Some(i),
                // Older than every kept match: the window moved past it (a capped search keeps the newest only).
                // The selection stays where it was, and the newest match is not selected instead.
                None if capped && new.first().is_some_and(|f| line < f.line) => {
                    detached = Some(Match { line, ..m });
                }
                None => {}
            }
        }
    }
    let current = current.or_else(|| match detached {
        Some(_) => None,
        None => new.len().checked_sub(1),
    });
    (current, detached)
}

impl App {
    /// Opens the search bar in the focused pane (keeps the query if already open).
    pub fn open_search(&mut self) {
        let Some(pane) = self.focused_pane() else {
            self.toast(ToastLevel::Info, "open a terminal tab first");
            return;
        };
        if self.search.as_ref().is_some_and(|s| s.pane == pane) {
            return;
        }
        self.search = Some(SearchState {
            pane,
            query: String::new(),
            matches: Vec::new(),
            current: None,
            detached: None,
            history: 0,
            dropped: 0,
            epoch: 0,
            text: None,
            capped: false,
            stale: false,
            refreshed: None,
        });
    }

    pub fn close_search(&mut self) {
        if let Some(s) = self.search.take()
            && let Some(p) = self.panes.get(&s.pane)
        {
            p.scroll_reset();
        }
    }

    /// Recomputes the matches; the selected match stays put if possible,
    /// otherwise the newest (bottom-most) match is selected.
    pub fn refresh_search(&mut self) {
        let Some(s) = &self.search else { return };
        let Some(p) = self.panes.get(&s.pane) else {
            self.search = None;
            return;
        };
        // The copy of the scrollback is reused while the pane has no new output (typing a query).
        let text = p.scroll_text(s.text.as_ref());
        let history = text.history;
        let (matches, capped) = find_matches(&text.lines, &s.query);
        let Some(s) = self.search.as_mut() else { return };
        let (dropped, epoch) = (text.dropped, text.epoch);
        let (current, detached) = carry_selection(s, &matches, capped, dropped, epoch);
        s.current = current;
        s.detached = detached;
        s.matches = matches;
        s.capped = capped;
        s.history = history;
        s.dropped = dropped;
        s.epoch = epoch;
        s.text = Some(text);
        s.stale = false;
        s.refreshed = Some(Instant::now());
    }

    /// Output in the search's pane makes the matches stale. They are recomputed right away when the last
    /// refresh is older than `SEARCH_REFRESH`; otherwise `refresh_search_when_due` does it once it is.
    pub(super) fn search_output(&mut self, pane: PaneId) {
        if let Some(s) = self.search.as_mut().filter(|s| s.pane == pane) {
            s.stale = true;
            self.refresh_search_when_due();
        }
    }

    /// The search's pane was resized: its lines re-wrap, so the matches are stale. They are recomputed by the
    /// throttled refresh, on the next tick (`redraw_after` schedules it), never while the frame is drawn: a drag
    /// of the divider would otherwise read the whole scrollback on every frame.
    pub(super) fn search_resized(&mut self) {
        if let Some(s) = self.search.as_mut() {
            s.stale = true;
        }
    }

    /// Recomputes stale matches once the refresh interval has passed (called on every tick and on output).
    /// `true` when it refreshed them.
    pub(super) fn refresh_search_when_due(&mut self) -> bool {
        let due =
            self.search.as_ref().is_some_and(|s| s.stale && s.refreshed.is_none_or(|t| t.elapsed() >= SEARCH_REFRESH));
        if due {
            self.refresh_search();
        }
        due
    }

    /// Goes to the previous (older, `-1`) or next (newer, `+1`) match. From a selection that fell out of the kept
    /// window (`detached`), the nearest kept match in that direction: the oldest kept one going newer.
    fn search_step(&mut self, dir: i32) {
        let Some(s) = self.search.as_mut() else { return };
        let n = s.matches.len();
        if n == 0 {
            return;
        }
        let next = match (s.current, s.detached) {
            (Some(i), _) => (i as i32 + dir).rem_euclid(n as i32) as usize,
            (None, Some(m)) => {
                // The first kept match after the detached line: going newer lands on it, going older on the one
                // before it (wrapping to the newest when there is none).
                let after = s.matches.partition_point(|x| x.line < m.line);
                if dir > 0 { after % n } else { after.checked_sub(1).unwrap_or(n - 1) }
            }
            (None, None) => (n as i32 - 1 + dir).rem_euclid(n as i32) as usize,
        };
        s.current = Some(next);
        s.detached = None;
        self.reveal_match();
    }

    fn reveal_match(&mut self) {
        let Some(s) = &self.search else { return };
        let Some(m) = s.current.and_then(|i| s.matches.get(i)) else { return };
        if let Some(p) = self.panes.get(&s.pane) {
            p.scroll_to_line(m.line, s.history);
        }
    }

    /// Keys while the search is open. `false` means the key is handled normally.
    pub(super) fn search_key(&mut self, k: KeyEvent) -> bool {
        let Some(s) = self.search.as_mut() else { return false };
        let ctrl = ctrl_held(&k);
        match k.code {
            KeyCode::Esc => self.close_search(),
            KeyCode::Enter if k.modifiers.contains(KeyModifiers::SHIFT) => self.search_step(1),
            KeyCode::Enter | KeyCode::Up => self.search_step(-1),
            KeyCode::Char('p') if ctrl => self.search_step(-1),
            KeyCode::Down => self.search_step(1),
            KeyCode::Char('n') if ctrl => self.search_step(1),
            KeyCode::Backspace => {
                s.query.pop();
                s.current = None;
                s.detached = None;
                self.refresh_search();
                self.reveal_match();
            }
            KeyCode::Char('u') if ctrl => {
                s.query.clear();
                s.current = None;
                s.detached = None;
                self.refresh_search();
            }
            KeyCode::Char(c) if !ctrl => {
                if s.query.chars().count() < 80 {
                    s.query.push(c);
                }
                s.current = None;
                s.detached = None;
                self.refresh_search();
                self.reveal_match();
            }
            // Other shortcuts (e.g. prefix, alt+1) close the search and run normally.
            _ => {
                self.search = None;
                return false;
            }
        }
        true
    }

    /// The link and its column span below (row, col) inside the pane.
    fn link_under(&self, pane: PaneId, inner: Rect, x: u16, y: u16) -> Option<(link::Link, u16, u16, u16)> {
        let p = self.panes.get(&pane)?;
        let (row, col) = (y.checked_sub(inner.y)?, x.checked_sub(inner.x)?);
        // First the links the app marked with OSC 8 (their text may differ from the address).
        if let Some((url, from, to)) = p.hyperlink_at(row, col) {
            let target = if url.to_ascii_lowercase().starts_with("file://") {
                link::classify(&url, &p.cwd())?
            } else {
                link::Link::Url(url)
            };
            return Some((target, row, from, to));
        }
        let text = p.visible_row(row)?;
        let (token, from, to) = link::token_at(&text, col)?;
        let target = link::classify(&token, &p.cwd())?;
        Some((target, row, from, to))
    }

    /// ctrl+click: opens the link the highlight shows under the pointer (`on_mouse` keeps that current). `false`
    /// if there is none.
    pub(super) fn open_link_at(&mut self, pane: PaneId) -> bool {
        let Some(hover) = self.link_hover.clone().filter(|h| h.pane == pane) else { return false };
        self.open_link(hover.target);
        true
    }

    /// Every link on the pane's screen, the bottom row (the newest output) first, each once. A bare name
    /// (`src`, `README.md`) is left out: in a list it is noise, while ctrl+click still opens it.
    fn visible_links(&self, pane: PaneId) -> Vec<link::Link> {
        let Some(p) = self.panes.get(&pane) else { return Vec::new() };
        let cwd = p.cwd();
        let mut rows = Vec::new();
        for row in 0u16.. {
            let Some(text) = p.visible_row(row) else { break };
            let mut found = Vec::new();
            let width = crate::util::width(&text) as u16;
            let mut col = 0;
            while col < width {
                if let Some((url, _, to)) = p.hyperlink_at(row, col) {
                    let target = if url.to_ascii_lowercase().starts_with("file://") {
                        link::classify(&url, &cwd)
                    } else {
                        Some(link::Link::Url(url))
                    };
                    found.extend(target);
                    col = to.max(col + 1);
                } else if let Some((token, _, to)) = link::token_at(&text, col) {
                    // Only what looks like a link is checked on disk (a URL, a path with a separator or a
                    // `:line`), not every word: on a slow or network drive each check is a stat call.
                    let lower = token.to_ascii_lowercase();
                    let is_url = ["http://", "https://", "file://"].iter().any(|s| lower.starts_with(s));
                    let has_line =
                        token.split(':').skip(1).any(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
                    if is_url || has_line || token.contains(['/', '\\']) {
                        found.extend(link::classify(&token, &cwd));
                    }
                    col = to.max(col + 1);
                } else {
                    col += 1;
                }
            }
            rows.push(found);
        }
        let mut links: Vec<link::Link> = Vec::new();
        for target in rows.into_iter().rev().flat_map(|r| r.into_iter()) {
            if !links.contains(&target) {
                links.push(target);
            }
        }
        links
    }

    /// The keyboard way to ctrl+click (macOS terminals such as iTerm2 and Terminal.app keep ctrl+click for
    /// their own menu): a menu of the links on the focused pane's screen, opened with ⏎ or the item's digit.
    pub(super) fn open_link_menu(&mut self) {
        let Some(pane) = self.focused_pane() else {
            self.toast(ToastLevel::Info, "open a terminal tab first");
            return;
        };
        let links = self.visible_links(pane);
        if links.is_empty() {
            self.toast(ToastLevel::Info, "no links on screen");
            return;
        }
        // Digits 1…9 pick an item, so the menu holds the nine newest, fewer when the window is too short to
        // show them all (a digit must not run a hidden item); the title says when some were left out.
        let max = (self.size.1 as usize).saturating_sub(2).clamp(1, 9);
        let title = if links.len() > max { format!("LINKS · {max} of {}", links.len()) } else { "LINKS".to_string() };
        let items = links
            .into_iter()
            .take(max)
            .enumerate()
            .map(|(i, target)| super::menu::MenuItem {
                // A path keeps its end (file name and line), an address its start (host); 36 columns fit the
                // widest menu with its number.
                label: format!(
                    "{} {}",
                    i + 1,
                    match &target {
                        link::Link::Url(_) => crate::util::truncate(&link_label(&target), 36),
                        link::Link::File { .. } => crate::util::truncate_left(&link_label(&target), 36),
                    }
                ),
                hint: String::new(),
                cmd: super::menu::MenuCmd::OpenLink(target),
                enabled: true,
            })
            .collect();
        // Below the pane title, like the pane menu.
        let at = self.hits.iter().find(|(_, h)| *h == super::Hit::PaneTitle(pane)).map(|(r, _)| (r.x + 1, r.y + 1));
        let (x, y) = at.unwrap_or((0, 1));
        self.open_menu(title, x, y, items);
    }

    /// Opens a link from terminal output: a URL in the browser, a file in an editor (see `link::open`).
    pub(super) fn open_link(&mut self, target: link::Link) {
        let label = link_label(&target);
        // A scheme the OS could hand to any registered app (`ms-msdt:`, `vscode:`, custom handlers) is never
        // opened from terminal output: the address is copied instead.
        if let link::Link::Url(url) = &target
            && !link::is_openable_url(url)
        {
            let url = url.clone();
            self.set_clipboard(&url, false);
            self.toast(ToastLevel::Info, format!("copied, not opened — {}", crate::util::truncate(&url, 50)));
            return;
        }
        // Without a graphical session: URLs go to the clipboard, files to a terminal editor.
        if !crate::util::has_desktop() {
            match &target {
                link::Link::Url(url) => {
                    let url = url.clone();
                    self.set_clipboard(&url, false);
                    self.toast(
                        ToastLevel::Info,
                        format!("no browser here — copied {}", crate::util::truncate(&url, 50)),
                    );
                    return;
                }
                link::Link::File { path, line, .. } => {
                    let (path, line) = (path.clone(), *line);
                    let title = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if path.is_file() && self.edit_in_tab(&path, line, &title) {
                        return;
                    }
                }
            }
        }
        match link::open(&target) {
            Ok(()) => self.toast(ToastLevel::Info, format!("opening {}", crate::util::truncate(&label, 60))),
            // A program behind a link (its text may say something else) is never run: the path is copied.
            Err(link::OpenError::Program) => {
                if let link::Link::File { path, .. } = &target {
                    let text = path.display().to_string();
                    self.set_clipboard(&text, false);
                }
                self.toast(
                    ToastLevel::Warn,
                    format!("a program — copied, not run: {}", crate::util::truncate(&label, 50)),
                );
            }
            Err(link::OpenError::Failed(e)) => self.toast(ToastLevel::Error, format!("could not open link: {e}")),
        }
    }

    /// Updates the link highlight from the pointer (when given) and the modifiers of the input event. The
    /// highlight shows the link a ctrl+click would open, so it is recomputed on every mouse and key event (a
    /// key or click without ctrl ends it: a released ctrl is not reported by every terminal), on focus loss and
    /// on output (the link under the pointer may have moved).
    pub(super) fn update_link_hover(&mut self, pointer: Option<(u16, u16)>, modifiers: KeyModifiers) {
        if let Some(p) = pointer {
            self.pointer = Some(p);
        }
        self.link_ctrl = modifiers.contains(KeyModifiers::CONTROL);
        self.refresh_link_hover();
    }

    /// Recomputes the link highlight on the screen as it is now (see `update_link_hover`). `true` when it changed.
    pub(super) fn refresh_link_hover(&mut self) -> bool {
        let next = self.link_under_pointer();
        let changed = next != self.link_hover;
        self.link_hover = next;
        changed
    }

    /// Called after a frame is drawn, when the hit list of that layout is known: a resize, a split or a config
    /// change may have moved the link under the pointer, so it is found again. A changed highlight asks for one
    /// more frame, since the frame just drawn shows the old one.
    pub fn relink_after_layout(&mut self) {
        if self.refresh_link_hover() {
            self.dirty = true;
        }
    }

    /// The link under the pointer, while ctrl is held and no overlay covers the terminal.
    fn link_under_pointer(&self) -> Option<LinkHover> {
        let (x, y) = self.pointer.filter(|_| self.link_ctrl && self.overlay.is_none())?;
        let Some(Hit::Pane { pane, inner }) = self.hit_at(x, y) else { return None };
        let (target, row, from, to) = self.link_under(pane, inner, x, y)?;
        Some(LinkHover { pane, row, from, to, target })
    }
}

/// How a link reads in a toast or menu: the address, or the path (with `~`) and its line.
fn link_label(target: &link::Link) -> String {
    match target {
        link::Link::Url(u) => u.clone(),
        link::Link::File { path, line, .. } => {
            let base = crate::util::tilde(path);
            line.map(|l| format!("{base}:{l}")).unwrap_or(base)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(line: usize, col: u16) -> Match {
        Match { line, col, width: 3 }
    }

    fn state(matches: Vec<Match>, current: Option<usize>, dropped: usize, epoch: u64) -> SearchState {
        SearchState {
            pane: PaneId::default(),
            query: String::new(),
            matches,
            current,
            detached: None,
            history: 0,
            dropped,
            epoch,
            text: None,
            capped: false,
            stale: false,
            refreshed: None,
        }
    }

    /// Lines scrolled out of a full scrollback: the counter says how many, the matches need not line up.
    #[test]
    fn selection_follows_the_lines_that_scrolled_out() {
        let s = state(vec![m(10, 0), m(40, 4), m(41, 0), m(90, 2)], Some(2), 0, 0);
        // Seven lines dropped: matches moved up, a new one appeared at the bottom.
        let new = [m(3, 0), m(33, 4), m(34, 0), m(83, 2), m(95, 0)];
        assert_eq!(carry_selection(&s, &new, false, 7, 0), (Some(2), None));
        // The selected line itself scrolled out: the newest match is selected.
        assert_eq!(carry_selection(&s, &[m(83, 2), m(95, 0)], false, 42, 0), (Some(1), None));
    }

    /// A taller screen pulls history back: the first line moved (`dropped` shrinks) and the selection follows it.
    #[test]
    fn selection_follows_a_row_grow() {
        let s = state(vec![m(10, 0), m(20, 0), m(30, 0)], Some(1), 6, 0);
        // Three blank stand-ins came back to the top: every index is three higher, `dropped` three lower.
        let new = [m(13, 0), m(23, 0), m(33, 0)];
        assert_eq!(carry_selection(&s, &new, false, 3, 0), (Some(1), None));
        // Without dropped lines the same text keeps its indexes.
        assert_eq!(carry_selection(&s, &s.matches.clone(), false, 6, 0), (Some(1), None));
    }

    /// After a reflow the old numbers mean nothing: the selection keeps its place counted from the newest match.
    #[test]
    fn selection_survives_a_new_epoch_by_position_from_the_end() {
        let s = state(vec![m(1, 0), m(2, 0), m(3, 0), m(4, 0)], Some(1), 0, 0);
        // Two matches below the selected one; the reflow made more matches above.
        let new = [m(1, 0), m(2, 0), m(3, 0), m(5, 0), m(8, 0), m(9, 0)];
        assert_eq!(carry_selection(&s, &new, false, 0, 1), (Some(3), None));
        // Fewer matches than before: clamped to "none", so the newest is selected.
        assert_eq!(carry_selection(&s, &[m(9, 0)], false, 0, 1), (Some(0), None));
        // A search without a selection selects the newest one.
        let s = state(vec![m(1, 0)], None, 0, 0);
        assert_eq!(carry_selection(&s, &new, false, 0, 1), (Some(5), None));
    }

    /// A capped search keeps the newest matches only: a selection older than all of them is detached.
    #[test]
    fn selection_older_than_the_kept_matches_is_detached() {
        let s = state(vec![m(10, 0), m(20, 0), m(30, 0)], Some(0), 0, 0);
        let new = [m(20, 0), m(30, 0), m(40, 0)];
        assert_eq!(carry_selection(&s, &new, true, 0, 0), (None, Some(m(10, 0))));
        // Without the cap it is simply gone.
        assert_eq!(carry_selection(&s, &new, false, 0, 0), (Some(2), None));
    }
}
