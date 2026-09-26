//! Terminal tab: every pane of the split tree is drawn cell by cell from the vt100 screen.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use super::hud;
use crate::app::{App, Hit, LinkHover, SearchState};
use crate::term::layout::{Dir, PaneId, frame_rect};
use crate::term::pane::Pane;
use crate::theme::{TermPalette, Theme};
use crate::ui::hud::Outline;
use crate::util;

/// Draws the visible tab; returns the focused pane's cursor position.
pub fn draw(
    buf: &mut Buffer,
    area: Rect,
    app: &App,
    tab_idx: usize,
    hits: &mut Vec<(Rect, Hit)>,
) -> Option<(u16, u16)> {
    let tab = app.tabs.get(tab_idx)?;
    let th = &app.theme;
    // During the fullscreen animation the normal layout stays visible behind it.
    let anim = app.zoom_anim.as_ref().filter(|z| tab.root.contains(z.pane));
    let (rects, dividers) =
        if tab.zoomed && anim.is_none() { (vec![(tab.focus, area)], Vec::new()) } else { tab.root.layout(area) };
    let multi = tab.panes().len() > 1;
    let t = &app.cfg.terminal;
    let pal = TermPalette::resolve(&app.term_schemes, &t.colors, &t.background, &t.foreground, th);
    let mut cursor = None;
    // Neighbouring panes share one border line; the frames are joined after they are all drawn.
    let frames: Vec<(PaneId, Outline)> = rects.iter().map(|(id, tile)| (*id, pane_outline(*tile, area))).collect();
    // Title text and buttons go on top of the dividers: a horizontal divider runs along the lower pane's title row.
    let mut on_top = Vec::new();
    for (id, rect) in &frames {
        let Some(pane) = app.panes.get(id) else { continue };
        let focused = *id == tab.focus;
        let inner = pane_frame(buf, *rect, pane, focused, multi, tab.zoomed, th, hits, &mut on_top);
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
    if frames.len() > 1 {
        let all: Vec<Outline> = frames.iter().map(|(_, o)| *o).collect();
        hud::join_frames(buf, &all);
        // The focused frame is drawn in its color all around, also where it shares a line.
        if let Some((_, r)) = frames.iter().find(|(id, _)| *id == tab.focus) {
            hud::tint_frame(buf, *r, th.accent_dim);
        }
    }
    if let Some(z) = anim {
        // The growing/shrinking pane: an intermediate rect between its place and fullscreen.
        let t = crate::app::ease(z.started, crate::app::ZOOM_DURATION);
        let rect = lerp_rect(z.from, z.to, t);
        if let Some(pane) = app.panes.get(&z.pane) {
            hud::clear(buf, rect, th);
            let o = Outline { rect, open_left: rect.x <= area.x, open_right: rect.right() >= area.right() };
            let inner = pane_frame(buf, o, pane, true, multi, tab.zoomed, th, &mut Vec::new(), &mut Vec::new());
            if inner.width > 0 && inner.height > 0 {
                pane_content(buf, inner, pane, th, &pal, false, &Marks::default());
            }
        }
        return None;
    }
    for div in dividers {
        hits.push((div.hit, Hit::Divider { tab: tab_idx, div: div.clone() }));
    }
    hits.extend(on_top);
    cursor
}

/// A pane's frame: it reaches one cell into a neighbour on its left or top (one shared line),
/// and the window's outer left and right sides stay open (the lines only go between panes).
pub fn pane_outline(tile: Rect, area: Rect) -> Outline {
    let rect = frame_rect(tile, area);
    Outline { rect, open_left: rect.x <= area.x, open_right: rect.right() >= area.right() }
}

#[allow(clippy::too_many_arguments)]
fn pane_frame(
    buf: &mut Buffer,
    outline: Outline,
    pane: &Pane,
    focused: bool,
    multi: bool,
    zoomed: bool,
    th: &Theme,
    hits: &mut Vec<(Rect, Hit)>,
    on_top: &mut Vec<(Rect, Hit)>,
) -> Rect {
    let rect = outline.rect;
    if rect.width < 4 || rect.height < 3 {
        return Rect::new(rect.x, rect.y, 0, 0);
    }
    let label = pane.label();
    let cwd = util::tilde(&pane.cwd());
    // The lock leads the title so it shows even when the pane is too narrow for the corner tag.
    let lock = if pane.passthrough { "🔒 " } else { "" };
    let title =
        format!("{lock}{label} · {}", util::truncate_left(&cwd, (rect.width as usize).saturating_sub(24).max(8)));
    let scroll = pane.scroll_offset();
    let tag = if pane.passthrough {
        // Key lock: NOBLE's shortcuts go to the app (`keys.passthrough = "lock"`).
        "KEYS".to_string()
    } else if scroll > 0 {
        format!("↑{scroll}")
    } else if zoomed {
        "ZOOM".to_string()
    } else {
        String::new()
    };
    let inner = hud::frame_outline(buf, outline, &title, "", focused, th);
    // Title row: click to focus, right click for the menu (buttons stay on top). The title
    // text itself also stays on top of a divider running along this row.
    hits.push((Rect::new(rect.x, rect.y, rect.width, 1), Hit::PaneTitle(pane.id)));
    let title_w = (util::width(&util::truncate(&title, rect.width.saturating_sub(6) as usize)) as u16 + 2)
        .min(rect.width.saturating_sub(2));
    on_top.push((Rect::new(rect.x + 1, rect.y, title_w, 1), Hit::PaneTitle(pane.id)));
    // Top-right corner buttons: split (right / down), zoom, close.
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
            on_top.push((Rect::new(bx, y, 3, 1), hit));
            bx += 3;
        }
        if !tag.is_empty() {
            let left = rect.right().saturating_sub(1 + total);
            hud::put_right(buf, left, y, &format!(" {tag}"), Style::default().fg(th.accent2));
        }
    }
    inner
}

/// Marks overlaid on a pane: search matches and the link.
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
    // The themed background also shows in the edge cells the screen does not cover.
    hud::fill(buf, inner, Style::default().bg(pal.bg).fg(pal.fg));
    let parser = pane.parser();
    let screen = parser.screen();
    let (rows, cols) = screen.size();
    let selection = pane.selection;
    let default_bg = pal.bg;
    // Matches in the visible rows: (screen row, start, end, selected).
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

/// Search bar overlaid on the pane's bottom row: query, counter and hints.
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

/// Linear interpolation between two rectangles.
pub fn lerp_rect(a: Rect, b: Rect, t: f64) -> Rect {
    let l = |x: u16, y: u16| (x as f64 + (y as f64 - x as f64) * t).round() as u16;
    Rect::new(l(a.x, b.x), l(a.y, b.y), l(a.width, b.width), l(a.height, b.height))
}
