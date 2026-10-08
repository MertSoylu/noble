//! Scrollback search and ctrl+click links.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ratatui::layout::Rect;

use super::input::ctrl_held;
use super::{App, LinkHover, SearchState, ToastLevel};
use crate::term::layout::PaneId;
use crate::term::link;
use crate::term::pane::{Match, find_matches};

/// How many old matches on each side of the selected one decide the shift in `scrolled_out`.
const SHIFT_WINDOW: usize = 64;
/// How many shifts `scrolled_out` tries at most.
const SHIFT_CANDIDATES: usize = 256;

/// How many lines dropped off the top since `old` was computed: the shift that makes most old matches
/// around the selected one (`old[selected]`) reappear (same column, `shift` lines higher) in `new`.
/// `0` when nothing lines up better than no shift.
///
/// Runs for every output batch while the search is open, so only a window of old matches around the
/// selection is scored (the one the search has to keep), not all of them for every candidate shift.
fn scrolled_out(old: &[Match], new: &[Match], selected: usize) -> usize {
    use std::collections::HashSet;
    let Some(first) = new.first() else { return 0 };
    if selected >= old.len() {
        return 0;
    }
    let window = &old[selected.saturating_sub(SHIFT_WINDOW)..(selected + SHIFT_WINDOW + 1).min(old.len())];
    let known: HashSet<(usize, u16)> = new.iter().map(|m| (m.line, m.col)).collect();
    let score =
        |k: usize| window.iter().filter(|m| m.line.checked_sub(k).is_some_and(|l| known.contains(&(l, m.col)))).count();
    // The oldest surviving match is the first new one, so the shift is one of these distances. The
    // matches are sorted by line, so the smallest distances come first.
    let mut candidates: Vec<usize> = Vec::with_capacity(SHIFT_CANDIDATES);
    for k in old.iter().filter_map(|m| m.line.checked_sub(first.line)) {
        if candidates.last() != Some(&k) {
            if candidates.len() == SHIFT_CANDIDATES {
                break;
            }
            candidates.push(k);
        }
    }
    let mut best = (score(0), 0);
    for k in candidates.into_iter().filter(|&k| k > 0) {
        let s = score(k);
        if s > best.0 {
            best = (s, k);
        }
    }
    best.1
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
        self.search = Some(SearchState { pane, query: String::new(), matches: Vec::new(), current: None, history: 0 });
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
        let (lines, history) = p.all_lines();
        let matches = find_matches(&lines, &s.query);
        let s = self.search.as_mut().expect("search open");
        // When old lines drop off the top (scrollback full or cleared), absolute line numbers shift. A full
        // scrollback keeps its length, so the drop is recovered from the matches themselves; a shorter
        // scrollback is the fallback when the matches do not line up.
        // Without a selection nothing has to be kept, so the shift is not worked out at all.
        let previous = s.current.and_then(|i| Some((i, *s.matches.get(i)?)));
        s.current = match previous {
            Some((i, m)) => {
                let dropped = match scrolled_out(&s.matches, &matches, i) {
                    0 => (s.history as isize - history as isize).max(0) as usize,
                    n => n,
                };
                let line = m.line.checked_sub(dropped);
                line.and_then(|line| matches.iter().position(|n| n.line == line && n.col == m.col))
            }
            None => None,
        }
        .or_else(|| matches.len().checked_sub(1));
        s.matches = matches;
        s.history = history;
    }

    /// Goes to the previous (older, `-1`) or next (newer, `+1`) match.
    fn search_step(&mut self, dir: i32) {
        let Some(s) = self.search.as_mut() else { return };
        let n = s.matches.len();
        if n == 0 {
            return;
        }
        let cur = s.current.unwrap_or(n - 1) as i32;
        s.current = Some((cur + dir).rem_euclid(n as i32) as usize);
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
                self.refresh_search();
                self.reveal_match();
            }
            KeyCode::Char('u') if ctrl => {
                s.query.clear();
                s.current = None;
                self.refresh_search();
            }
            KeyCode::Char(c) if !ctrl => {
                if s.query.chars().count() < 80 {
                    s.query.push(c);
                }
                s.current = None;
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

    /// ctrl+click: opens the link. `false` if there is none.
    pub(super) fn open_link_at(&mut self, pane: PaneId, inner: Rect, m: &MouseEvent) -> bool {
        let Some((target, ..)) = self.link_under(pane, inner, m.column, m.row) else { return false };
        self.open_link(target);
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

    /// Highlights the link under the pointer while ctrl is held during mouse movement.
    pub(super) fn update_link_hover(&mut self, pane: Option<(PaneId, Rect)>, m: &MouseEvent) {
        self.link_hover = match pane {
            Some((pane, inner)) if m.modifiers.contains(KeyModifiers::CONTROL) => self
                .link_under(pane, inner, m.column, m.row)
                .map(|(_, row, from, to)| LinkHover { pane, row, from, to }),
            _ => None,
        };
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

    /// A full scrollback keeps its length while lines drop off the top: the shift comes from the matches.
    #[test]
    fn scrolled_out_recovers_the_dropped_lines() {
        let old = [m(10, 0), m(40, 4), m(41, 0), m(90, 2)];
        // Seven lines dropped: matches moved up, one new match appeared at the bottom.
        let new = [m(3, 4), m(34, 0), m(83, 2), m(95, 0)];
        assert_eq!(scrolled_out(&old, &new, 3), 7);
        // The oldest match itself scrolled out.
        let new = [m(33, 4), m(34, 0), m(83, 2)];
        assert_eq!(scrolled_out(&old, &new, 2), 7);
    }

    #[test]
    fn scrolled_out_is_zero_when_nothing_moved() {
        let old = [m(10, 0), m(20, 0), m(30, 0)];
        assert_eq!(scrolled_out(&old, &old, 1), 0);
        // Growing history only appends matches.
        assert_eq!(scrolled_out(&old, &[m(10, 0), m(20, 0), m(30, 0), m(50, 1)], 2), 0);
        assert_eq!(scrolled_out(&[], &old, 0), 0);
        assert_eq!(scrolled_out(&old, &[], 0), 0);
        // A selection that is not an old match (none to keep) never scores anything.
        assert_eq!(scrolled_out(&old, &old, 3), 0);
    }

    /// Many matches: only the window around the selection is scored, so matches far from it
    /// that line up with another shift do not outvote the ones next to it.
    #[test]
    fn scrolled_out_scores_only_around_the_selection() {
        // 5000 matches, one every 3 lines, with varying columns so a wrong shift does not line up.
        let old: Vec<Match> = (0..5000).map(|i| m(i * 3, (i % 5) as u16)).collect();
        // Around the selection four matches (12 lines) dropped off the top; the many matches below
        // line up with a shift of 3 instead.
        let new: Vec<Match> = (0..4999)
            .map(|i| if i < 1000 { (old[i + 4], 12) } else { (old[i + 1], 3) })
            .map(|(x, k)| m(x.line - k, x.col))
            .collect();
        assert!(new.windows(2).all(|w| w[0].line < w[1].line));
        assert_eq!(scrolled_out(&old, &new, 100), 12);
        assert_eq!(scrolled_out(&old, &new, 4000), 3);
    }
}
