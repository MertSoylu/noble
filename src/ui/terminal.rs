//! Terminal sekmesi: bölme ağacındaki her pane, vt100 ekranından hücre hücre çizilir.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use super::hud;
use crate::app::{App, Hit, LinkHover, SearchState};
use crate::term::layout::Dir;
use crate::term::pane::Pane;
use crate::theme::{TermPalette, Theme};
use crate::util;

/// Görünen sekmeyi çizer; odaktaki pane'in imleç konumunu döndürür.
pub fn draw(
    buf: &mut Buffer,
    area: Rect,
    app: &App,
    tab_idx: usize,
    hits: &mut Vec<(Rect, Hit)>,
) -> Option<(u16, u16)> {
    let tab = app.tabs.get(tab_idx)?;
    let th = &app.theme;
    // Tam ekran animasyonu sırasında arka planda normal düzen görünür.
    let anim = app.zoom_anim.as_ref().filter(|z| tab.root.contains(z.pane));
    let (rects, dividers) =
        if tab.zoomed && anim.is_none() { (vec![(tab.focus, area)], Vec::new()) } else { tab.root.layout(area) };
    let multi = tab.panes().len() > 1;
    let t = &app.cfg.terminal;
    let pal = TermPalette::resolve(&app.term_schemes, &t.colors, &t.background, &t.foreground, th);
    let mut cursor = None;
    for (id, rect) in &rects {
        let Some(pane) = app.panes.get(id) else { continue };
        let focused = *id == tab.focus;
        let inner = pane_frame(buf, *rect, pane, focused, multi, tab.zoomed, th, hits);
        if inner.width == 0 || inner.height == 0 {
            continue;
        }
        hits.push((inner, Hit::Pane { pane: *id, inner }));
        let marks = Marks {
            search: app.search.as_ref().filter(|s| s.pane == *id),
            link: app.link_hover.filter(|l| l.pane == *id),
        };
        let c = pane_content(buf, inner, pane, th, &pal, focused, &marks);
        if let Some(s) = marks.search {
            search_bar(buf, inner, s, th);
        }
        if focused && marks.search.is_none() {
            cursor = c;
        }
    }
    if let Some(z) = anim {
        // Büyüyen/küçülen pane: bulunduğu yerle tam ekran arasında ara dikdörtgen.
        let t = crate::app::ease(z.started, crate::app::ZOOM_DURATION);
        let rect = lerp_rect(z.from, z.to, t);
        if let Some(pane) = app.panes.get(&z.pane) {
            hud::clear(buf, rect, th);
            let inner = pane_frame(buf, rect, pane, true, multi, tab.zoomed, th, &mut Vec::new());
            if inner.width > 0 && inner.height > 0 {
                pane_content(buf, inner, pane, th, &pal, false, &Marks::default());
            }
        }
        return None;
    }
    for div in dividers {
        hits.push((div.hit, Hit::Divider { tab: tab_idx, div: div.clone() }));
    }
    cursor
}

#[allow(clippy::too_many_arguments)]
fn pane_frame(
    buf: &mut Buffer,
    rect: Rect,
    pane: &Pane,
    focused: bool,
    multi: bool,
    zoomed: bool,
    th: &Theme,
    hits: &mut Vec<(Rect, Hit)>,
) -> Rect {
    if rect.width < 4 || rect.height < 3 {
        return Rect::new(rect.x, rect.y, 0, 0);
    }
    let label = pane.label();
    let cwd = util::tilde(&pane.cwd());
    let title = format!("{label} · {}", util::truncate_left(&cwd, (rect.width as usize).saturating_sub(24).max(8)));
    let scroll = pane.scroll_offset();
    let tag = if scroll > 0 {
        format!("↑{scroll}")
    } else if zoomed {
        "ZOOM".to_string()
    } else {
        String::new()
    };
    let inner = hud::frame(buf, rect, &title, "", focused, th);
    // Başlık satırı: tıklayınca odak, sağ tıkla menü (düğmeler üstte kalır).
    hits.push((Rect::new(rect.x, rect.y, rect.width, 1), Hit::PaneTitle(pane.id)));
    // Sağ üst köşe düğmeleri: böl (sağa / aşağı), büyüt, kapat.
    let y = rect.y;
    let style = if focused { th.accent() } else { th.dim() };
    let mut buttons: Vec<(&str, Hit)> = Vec::new();
    if rect.width >= 30 {
        buttons.push(("┃", Hit::PaneSplit { pane: pane.id, dir: Dir::Row }));
        buttons.push(("━", Hit::PaneSplit { pane: pane.id, dir: Dir::Col }));
    }
    if (multi || zoomed) && rect.width >= 22 {
        buttons.push((if zoomed { "◱" } else { "⤢" }, Hit::PaneZoom(pane.id)));
    }
    if rect.width >= 14 {
        buttons.push(("✕", Hit::PaneClose(pane.id)));
    }
    let total = buttons.len() as u16 * 3 + 1;
    let mut bx = rect.right().saturating_sub(1 + total);
    if !buttons.is_empty() && bx > rect.x + 4 {
        hud::put(buf, bx, y, " ", style, 1);
        bx += 1;
        for (glyph, hit) in buttons {
            hud::put(buf, bx, y, &format!(" {glyph} "), style, 3);
            hits.push((Rect::new(bx, y, 3, 1), hit));
            bx += 3;
        }
        if !tag.is_empty() {
            let left = rect.right().saturating_sub(1 + total);
            hud::put_right(buf, left, y, &format!(" {tag}"), Style::default().fg(th.accent2));
        }
    }
    inner
}

/// Pane üzerine bindirilen işaretler: arama eşleşmeleri ve bağlantı.
#[derive(Default)]
struct Marks<'a> {
    search: Option<&'a SearchState>,
    link: Option<LinkHover>,
}

fn pane_content(
    buf: &mut Buffer,
    inner: Rect,
    pane: &Pane,
    th: &Theme,
    pal: &TermPalette,
    focused: bool,
    marks: &Marks<'_>,
) -> Option<(u16, u16)> {
    // Şemalı zemin, ekranın kaplamadığı kenar hücrelerde de görünsün.
    hud::fill(buf, inner, Style::default().bg(pal.bg).fg(pal.fg));
    let parser = pane.parser();
    let screen = parser.screen();
    let (rows, cols) = screen.size();
    let selection = pane.selection;
    let default_bg = pal.bg;
    // Görünen satırlardaki eşleşmeler: (ekran satırı, başlangıç, bitiş, seçili mi).
    let found: Vec<(u16, u16, u16, bool)> = match marks.search {
        Some(s) => {
            let top = s.history.saturating_sub(screen.scrollback());
            s.matches
                .iter()
                .enumerate()
                .filter(|(_, m)| m.line >= top && m.line < top + rows as usize)
                .map(|(i, m)| ((m.line - top) as u16, m.col, m.col + m.width, s.current == Some(i)))
                .collect()
        }
        None => Vec::new(),
    };
    for r in 0..inner.height.min(rows) {
        for c in 0..inner.width.min(cols) {
            let Some(cell) = screen.cell(r, c) else { continue };
            if cell.is_wide_continuation() {
                continue;
            }
            let mut fg = pal.color(cell.fgcolor(), pal.fg);
            let mut bg = pal.color(cell.bgcolor(), default_bg);
            if cell.inverse() {
                std::mem::swap(&mut fg, &mut bg);
                if fg == Color::Reset {
                    fg = th.bg;
                }
                if bg == Color::Reset {
                    bg = th.fg;
                }
            }
            let mut m = Modifier::empty();
            if cell.bold() {
                m |= Modifier::BOLD;
            }
            if cell.dim() {
                m |= Modifier::DIM;
            }
            if cell.italic() {
                m |= Modifier::ITALIC;
            }
            if cell.underline() {
                m |= Modifier::UNDERLINED;
            }
            if selection.is_some_and(|s| !s.is_empty() && s.contains(r, c)) {
                bg = pal.sel_bg;
                fg = pal.fg;
            }
            if let Some(&(.., current)) = found.iter().find(|(fr, a, b, _)| *fr == r && c >= *a && c < *b) {
                if current {
                    bg = th.accent;
                    fg = th.on_accent;
                    m |= Modifier::BOLD;
                } else {
                    bg = pal.match_bg;
                    fg = pal.fg;
                }
            }
            if marks.link.is_some_and(|l| l.row == r && c >= l.from && c < l.to) {
                m |= Modifier::UNDERLINED;
                fg = th.accent2;
            }
            let x = inner.x + c;
            let y = inner.y + r;
            if let Some(out) = buf.cell_mut((x, y)) {
                let sym = cell.contents();
                out.set_symbol(if sym.is_empty() { " " } else { sym });
                out.set_style(Style::default().fg(fg).bg(bg).add_modifier(m));
            }
        }
    }
    if !focused || screen.hide_cursor() || screen.scrollback() > 0 {
        return None;
    }
    let (cr, cc) = screen.cursor_position();
    (cr < inner.height && cc < inner.width).then(|| (inner.x + cc, inner.y + cr))
}

/// Pane'in alt satırına bindirilen arama çubuğu: sorgu, sayaç ve ipuçları.
fn search_bar(buf: &mut Buffer, inner: Rect, s: &SearchState, th: &Theme) {
    if inner.height < 2 || inner.width < 12 {
        return;
    }
    let y = inner.bottom() - 1;
    let bar = Rect::new(inner.x, y, inner.width, 1);
    hud::fill(buf, bar, Style::default().bg(th.raised).fg(th.fg));
    let count = match (s.current, s.matches.len()) {
        (_, 0) if s.query.is_empty() => String::new(),
        (_, 0) => "no matches".to_string(),
        (Some(i), n) => format!("{}/{n}", i + 1),
        (None, n) => format!("{n}"),
    };
    let hint = "⏎↑ older  ↓ newer  esc close";
    let right_w = util::width(&count) as u16 + if inner.width >= 60 { util::width(hint) as u16 + 3 } else { 0 };
    let x = hud::put(buf, inner.x + 1, y, "find ", th.dim().bg(th.raised), inner.width);
    let room = inner.right().saturating_sub(x + right_w + 2);
    let query = util::truncate_left(&s.query, room.saturating_sub(1) as usize);
    let x = hud::put(buf, x, y, &query, th.accent_bold().bg(th.raised), room);
    hud::put(buf, x, y, "▏", th.accent().bg(th.raised), 1);
    let mut rx = inner.right().saturating_sub(1);
    if inner.width >= 60 {
        rx = hud::put_right(buf, rx, y, hint, th.dim().bg(th.raised)).saturating_sub(3);
    }
    let color = if s.matches.is_empty() && !s.query.is_empty() { th.warn } else { th.accent2 };
    hud::put_right(buf, rx, y, &count, Style::default().fg(color).bg(th.raised));
}

/// İki dikdörtgen arasında doğrusal ara değer.
pub fn lerp_rect(a: Rect, b: Rect, t: f64) -> Rect {
    let l = |x: u16, y: u16| (x as f64 + (y as f64 - x as f64) * t).round() as u16;
    Rect::new(l(a.x, b.x), l(a.y, b.y), l(a.width, b.width), l(a.height, b.height))
}
