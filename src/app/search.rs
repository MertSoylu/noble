//! Terminal geçmişinde arama ve ctrl+tık bağlantıları.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ratatui::layout::Rect;

use super::{App, LinkHover, SearchState, ToastLevel};
use crate::term::layout::PaneId;
use crate::term::link;
use crate::term::pane::find_matches;

impl App {
    /// Odaktaki pane'de arama çubuğunu açar (açıksa sorguyu korur).
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

    /// Eşleşmeleri yeniden hesaplar; seçili eşleşme mümkünse yerinde kalır,
    /// yoksa en yeni (en alttaki) eşleşme seçilir.
    pub fn refresh_search(&mut self) {
        let Some(s) = &self.search else { return };
        let Some(p) = self.panes.get(&s.pane) else {
            self.search = None;
            return;
        };
        let (lines, history) = p.all_lines();
        let matches = find_matches(&lines, &s.query);
        let s = self.search.as_mut().expect("arama açık");
        // Geçmiş dolup eski satırlar düştüyse mutlak satır numaraları kayar.
        let shift = history as isize - s.history as isize;
        let previous = s.current.and_then(|i| s.matches.get(i)).copied();
        s.current = match previous {
            Some(m) => {
                let line = m.line as isize + shift.min(0);
                matches.iter().position(|n| n.line as isize == line && n.col == m.col)
            }
            None => None,
        }
        .or_else(|| matches.len().checked_sub(1));
        s.matches = matches;
        s.history = history;
    }

    /// Önceki (daha eski, `-1`) ya da sonraki (daha yeni, `+1`) eşleşmeye gider.
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

    /// Arama açıkken tuşlar. `false` dönerse tuş normal yoldan işlenir.
    pub(super) fn search_key(&mut self, k: KeyEvent) -> bool {
        let Some(s) = self.search.as_mut() else { return false };
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
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
            // Diğer kısayollar (ör. prefix, alt+1) aramayı kapatıp normal çalışır.
            _ => {
                self.search = None;
                return false;
            }
        }
        true
    }

    /// Pane içinde (satır, sütun) altındaki bağlantı ve sütun aralığı.
    fn link_under(&self, pane: PaneId, inner: Rect, x: u16, y: u16) -> Option<(link::Link, u16, u16, u16)> {
        let p = self.panes.get(&pane)?;
        let (row, col) = (y.checked_sub(inner.y)?, x.checked_sub(inner.x)?);
        // Önce uygulamanın OSC 8 ile işaretlediği bağlantılar (metin adresten farklı olabilir).
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

    /// ctrl+tık: bağlantıyı açar. Bağlantı yoksa `false`.
    pub(super) fn open_link_at(&mut self, pane: PaneId, inner: Rect, m: &MouseEvent) -> bool {
        let Some((target, ..)) = self.link_under(pane, inner, m.column, m.row) else { return false };
        let label = match &target {
            link::Link::Url(u) => u.clone(),
            link::Link::File { path, line, .. } => {
                let base = crate::util::tilde(path);
                line.map(|l| format!("{base}:{l}")).unwrap_or(base)
            }
        };
        match link::open(&target) {
            Ok(()) => self.toast(ToastLevel::Info, format!("opening {}", crate::util::truncate(&label, 60))),
            Err(e) => self.toast(ToastLevel::Error, format!("could not open link: {e}")),
        }
        true
    }

    /// Fare hareketinde ctrl basılıysa altındaki bağlantıyı işaretler.
    pub(super) fn update_link_hover(&mut self, pane: Option<(PaneId, Rect)>, m: &MouseEvent) {
        self.link_hover = match pane {
            Some((pane, inner)) if m.modifiers.contains(KeyModifiers::CONTROL) => self
                .link_under(pane, inner, m.column, m.row)
                .map(|(_, row, from, to)| LinkHover { pane, row, from, to }),
            _ => None,
        };
    }
}
