//! Settings tab: on/off toggles, cycling choices and rows that open a selector (theme,
//! terminal colors, quick launch). Every row is clickable.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use super::hud;
use crate::app::{App, Hit, SettingItem, SettingKey};
use crate::util;

/// Outer width of the settings frame.
pub fn settings_width(term_w: u16) -> u16 {
    term_w.saturating_sub(2).min(96)
}

enum Line {
    Header(&'static str),
    Item(usize),
    Buttons(Vec<usize>),
    Blank,
}

/// The page's lines: each section's header and its rows (items are indices into `settings_items`).
fn build_lines(app: &App) -> Vec<Line> {
    let mut v = Vec::new();
    let mut idx = 0;
    for (n, (title, items)) in app.settings_sections().into_iter().enumerate() {
        if n > 0 {
            v.push(Line::Blank);
        }
        v.push(Line::Header(title));
        let mut buttons = Vec::new();
        for (k, item) in items.iter().enumerate() {
            match item {
                SettingItem::Setting(_) => v.push(Line::Item(idx + k)),
                _ => buttons.push(idx + k),
            }
        }
        if !buttons.is_empty() {
            v.push(Line::Buttons(buttons));
        }
        idx += items.len();
    }
    v
}

fn line_has(line: &Line, idx: usize) -> bool {
    match line {
        Line::Item(i) => *i == idx,
        Line::Buttons(v) => v.contains(&idx),
        _ => false,
    }
}

/// The frame of the page inside the body, and how many lines of the page fit in it (the
/// first and last inner rows hold the "more" markers).
fn page_frame(area: Rect) -> (Rect, usize) {
    let w = settings_width(area.width).min(area.width);
    let frame = Rect::new(area.x + (area.width - w) / 2, area.y, w, area.height);
    (frame, area.height.saturating_sub(4) as usize)
}

/// Keeps the scroll position valid and, after a key press, the selected row (with its section
/// header) on screen. Runs before drawing, which only reads the state.
pub fn sync_scroll(app: &mut App, area: Rect) {
    let lines = build_lines(app);
    let (_, rows) = page_frame(area);
    if app.settings_follow {
        let sel = app.settings_sel.min(app.settings_items().len().saturating_sub(1));
        let sel_line = lines.iter().position(|l| line_has(l, sel)).unwrap_or(0);
        let top = if sel_line > 0 && matches!(lines[sel_line - 1], Line::Header(_)) { sel_line - 1 } else { sel_line };
        if top < app.settings_scroll {
            app.settings_scroll = top;
        } else if rows > 0 && sel_line >= app.settings_scroll + rows {
            app.settings_scroll = sel_line + 1 - rows;
        }
        app.settings_follow = false;
    }
    app.settings_scroll = app.settings_scroll.min(lines.len().saturating_sub(rows));
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &App, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    if area.width < 30 || area.height < 8 {
        hud::put_center(buf, area, area.y + area.height / 2, "enlarge window", th.dim());
        return;
    }
    let (frame, rows) = page_frame(area);
    let inner = hud::frame(buf, frame, "Settings", "saved automatically", true, th);
    let x = inner.x + 2;
    let iw = inner.width.saturating_sub(4);
    let items = app.settings_items();
    let sel = app.settings_sel.min(items.len() - 1);
    let lines = build_lines(app);
    let offset = app.settings_scroll.min(lines.len().saturating_sub(rows));
    let visible = &lines[offset..(offset + rows).min(lines.len())];

    for (n, (y, line)) in (inner.y + 1..).zip(visible).enumerate() {
        match line {
            Line::Blank => {}
            // A header cut off from its first row stays hidden.
            Line::Header(_) if n + 1 == visible.len() => {}
            Line::Header(h) => {
                hud::put(buf, x, y, h, th.accent_bold(), iw);
            }
            Line::Item(i) => {
                let SettingItem::Setting(key) = items[*i] else { continue };
                setting_row(buf, Rect::new(inner.x, y, inner.width, 1), app, key, *i == sel);
                hits.push((Rect::new(inner.x, y, inner.width, 1), Hit::Setting(*i)));
            }
            Line::Buttons(list) => {
                let mut cx = x;
                for i in list {
                    let label = match items[*i] {
                        SettingItem::OpenConfig => "Edit config file",
                        _ => "Reload config",
                    };
                    let start = cx;
                    let key = if *i == sel { "▶" } else { "·" };
                    cx = hud::button(buf, cx, y, key, label, th, true);
                    hits.push((Rect::new(start, y, cx - start, 1), Hit::Setting(*i)));
                    cx += 2;
                }
                let path = util::tilde(&app.paths.config);
                let room = (x + iw).saturating_sub(cx + 1);
                if room > 10 {
                    hud::put_right(buf, x + iw, y, &util::truncate_left(&path, room as usize), th.dim());
                }
            }
        }
    }
    // More of the page above / below (the wheel scrolls it).
    if offset > 0 {
        hud::put_right(buf, x + iw, inner.y, "↑ more", th.dim());
    }
    if offset + rows < lines.len() {
        hud::put_right(buf, x + iw, inner.bottom() - 1, "↓ more", th.dim());
    }
}

/// A setting's row: its label and value (on/off, a choice, or what a popup sets). A setting
/// that only matters under another one is indented, and dimmed while that one is off.
fn setting_row(buf: &mut Buffer, row: Rect, app: &App, key: SettingKey, selected: bool) {
    let th = &app.theme;
    let (x, y, iw) = (row.x + 2, row.y, row.width.saturating_sub(4));
    let bg = if selected { th.sel_bg } else { th.bg };
    if selected {
        hud::set_bg_row(buf, row.x, y, row.width, th.sel_bg);
        hud::put(buf, row.x, y, "▌", Style::default().fg(th.accent).bg(th.sel_bg), 1);
    }
    let parent = key.parent();
    let active = parent.is_none_or(|p| app.setting_on(p));
    let indent = if parent.is_some() { "  " } else { "" };
    let label = format!("{indent}{}", key.label());
    let text = if active { th.text() } else { th.dim() };
    hud::put(buf, x, y, &label, text.bg(bg), iw);
    // Rows that open a selector show a chip in the chosen colors.
    let look = match key {
        SettingKey::Theme => Some(theme_look(&app.theme)),
        SettingKey::TermColors => Some(scheme_look(app, &app.cfg.terminal.colors)),
        _ => None,
    };
    if let Some(look) = look {
        let label_end = x + util::width(&label) as u16 + 2;
        chip(buf, label_end, x + iw, y, app, bg, look);
        return;
    }
    let dim = th.dim().bg(bg);
    if key.opens_popup() {
        // Changed in a popup: no ‹ › (←→ do nothing here).
        let value = app.setting_value(key);
        let vx = hud::put_right(buf, x + iw, y, &value, Style::default().fg(th.accent2).bg(bg));
        hud::put_right(buf, vx, y, "⏎ choose  ", dim);
        return;
    }
    let (value, style) = match key.is_toggle().then(|| app.setting_on(key)) {
        Some(true) if active => {
            ("● On".to_string(), Style::default().fg(th.accent).bg(bg).add_modifier(Modifier::BOLD))
        }
        Some(true) => ("● On".to_string(), dim),
        Some(false) => ("○ Off".to_string(), dim),
        None if active => (format!("‹ {} ›", app.setting_value(key)), Style::default().fg(th.accent2).bg(bg)),
        None => (format!("‹ {} ›", app.setting_value(key)), dim),
    };
    hud::put_right(buf, x + iw, y, &value, style);
}

/// The chosen theme or terminal scheme: a chip in its own background/text colors with
/// three color swatches, and a hint to open the selector on its left.
fn chip(
    buf: &mut Buffer,
    left: u16,
    right: u16,
    y: u16,
    app: &App,
    row_bg: Color,
    (label, bg, fg, dots): (String, Color, Color, Vec<Color>),
) {
    let th = &app.theme;
    let w = (util::width(&label) as u16 + 7).min(right.saturating_sub(left));
    if w < 6 {
        return;
    }
    let x = right - w;
    let hint = "⏎ choose  ";
    if x >= left + util::width(hint) as u16 {
        hud::put_right(buf, x, y, hint, th.dim().bg(row_bg));
    }
    let rect = Rect::new(x, y, w, 1);
    hud::fill(buf, rect, Style::default().bg(bg).fg(fg));
    let mut px = hud::put(buf, x, y, "▌", Style::default().fg(th.accent).bg(bg), 1);
    for d in dots.iter().take(3) {
        px = hud::put(buf, px, y, "●", Style::default().fg(*d).bg(bg), 1);
    }
    px = hud::put(buf, px, y, " ", Style::default().bg(bg), 1);
    let room = rect.right().saturating_sub(px + 1);
    hud::put(
        buf,
        px,
        y,
        &util::truncate(&label, room as usize),
        Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
        room,
    );
}

/// How a theme looks: name, background, text and its accent colors.
pub(super) fn theme_look(t: &crate::theme::Theme) -> (String, Color, Color, Vec<Color>) {
    (t.label.to_string(), t.bg, t.fg, vec![t.accent, t.accent2, t.ok])
}

/// How a scheme looks: name, background, text and color swatches (red, green, blue, …).
pub(super) fn scheme_look(app: &App, name: &str) -> (String, Color, Color, Vec<Color>) {
    let th = &app.theme;
    match crate::theme::find_scheme(&app.term_schemes, name) {
        Some(s) => (s.label.clone(), s.bg, s.fg, s.ansi.to_vec()),
        None => ("Follow theme".into(), th.bg, th.fg, vec![th.accent, th.accent2, th.ok]),
    }
}
