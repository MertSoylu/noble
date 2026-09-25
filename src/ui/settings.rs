//! Settings tab: theme cards (each previews its own colors), on/off toggles
//! and cycling choices. Every row is clickable.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use super::hud;
use crate::app::{App, Hit, PROVIDER_KEYS, SettingItem, SettingKey};
use crate::theme::THEMES;
use crate::util;

/// Theme card width (including 1 column of spacing).
pub const THEME_CARD_W: u16 = 22;

/// Outer width of the settings frame.
pub fn settings_width(term_w: u16) -> u16 {
    term_w.saturating_sub(2).min(96)
}

enum Line {
    Header(&'static str),
    Themes(usize, usize),
    Item(usize),
    Buttons(Vec<usize>),
    Blank,
}

fn build_lines(items: &[SettingItem], cols: usize) -> Vec<Line> {
    let pos = |target: SettingItem| items.iter().position(|i| *i == target).unwrap_or(0);
    let key = |k: SettingKey| Line::Item(pos(SettingItem::Setting(k)));
    let mut v = vec![Line::Header("Theme")];
    let n = THEMES.len();
    let mut i = 0;
    while i < n {
        v.push(Line::Themes(i, (i + cols).min(n)));
        i += cols;
    }
    v.push(Line::Blank);
    v.push(Line::Header("Display"));
    v.extend(
        [SettingKey::Transparent, SettingKey::Boot, SettingKey::Animations, SettingKey::Clock24, SettingKey::Seconds]
            .map(key),
    );
    v.push(key(SettingKey::Updates));
    v.push(Line::Blank);
    v.push(Line::Header("Terminal"));
    v.extend(
        [
            SettingKey::Shell,
            SettingKey::Prefix,
            SettingKey::Passthrough,
            SettingKey::Restore,
            SettingKey::CopySelect,
            SettingKey::TermColors,
            SettingKey::Notify,
            SettingKey::QuickLaunch,
        ]
        .map(key),
    );
    v.push(Line::Blank);
    v.push(Line::Header("AI usage"));
    // Providers that are not installed get no row (`settings_items` filters them out).
    let present = |k: &SettingKey| items.contains(&SettingItem::Setting(*k));
    v.push(key(SettingKey::AiEnabled));
    v.extend(PROVIDER_KEYS.iter().filter(|k| present(k)).map(|k| key(*k)));
    v.extend([SettingKey::AiRefresh, SettingKey::AiWarn].map(key));
    if present(&SettingKey::ClaudeHooks) {
        v.push(key(SettingKey::ClaudeHooks));
    }
    v.push(Line::Blank);
    v.push(Line::Buttons(vec![pos(SettingItem::OpenConfig), pos(SettingItem::ReloadConfig)]));
    v
}

fn line_has(line: &Line, idx: usize) -> bool {
    match line {
        Line::Themes(a, b) => idx >= *a && idx < *b,
        Line::Item(i) => *i == idx,
        Line::Buttons(v) => v.contains(&idx),
        _ => false,
    }
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &App, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    if area.width < 30 || area.height < 8 {
        hud::put_center(buf, area, area.y + area.height / 2, "enlarge window", th.dim());
        return;
    }
    let w = settings_width(area.width).min(area.width);
    let frame = Rect::new(area.x + (area.width - w) / 2, area.y, w, area.height);
    let inner = hud::frame(buf, frame, "Settings", "saved automatically", true, th);
    let x = inner.x + 2;
    let iw = inner.width.saturating_sub(4);
    let items = app.settings_items();
    let sel = app.settings_sel.min(items.len() - 1);
    let cols = app.theme_columns();
    let lines = build_lines(&items, cols);

    // Keep the selected row visible.
    let rows = inner.height.saturating_sub(2) as usize;
    let sel_line = lines.iter().position(|l| line_has(l, sel)).unwrap_or(0);
    let offset = (sel_line + 2).saturating_sub(rows);
    let offset = if sel_line < 3 { 0 } else { offset };

    for (y, line) in (inner.y + 1..).zip(lines.iter().skip(offset)) {
        if y >= inner.bottom().saturating_sub(1) {
            break;
        }
        match line {
            Line::Blank => {}
            Line::Header(h) => {
                hud::put(buf, x, y, h, th.accent_bold(), iw);
            }
            Line::Themes(a, b) => {
                for (col, ti) in (*a..*b).enumerate() {
                    let cx = x + col as u16 * THEME_CARD_W;
                    let card = Rect::new(cx, y, THEME_CARD_W - 1, 1);
                    theme_card(buf, card, app, ti, ti == sel);
                    hits.push((card, Hit::Setting(ti)));
                }
            }
            Line::Item(i) => {
                // The row's label and value: on/off (Some(bool)) or cycling text.
                let (label, toggle, text) = match items[*i] {
                    SettingItem::Setting(key) => {
                        let toggle = key.is_toggle().then(|| app.setting_on(key));
                        (key.label().to_string(), toggle, app.setting_value(key))
                    }
                    _ => continue,
                };
                let selected = *i == sel;
                let row = Rect::new(inner.x, y, inner.width, 1);
                if selected {
                    hud::set_bg_row(buf, row.x, y, row.width, th.sel_bg);
                    hud::put(buf, row.x, y, "▌", Style::default().fg(th.accent).bg(th.sel_bg), 1);
                }
                let bg = if selected { th.sel_bg } else { th.bg };
                hud::put(buf, x, y, &label, th.text().bg(bg), iw);
                hits.push((row, Hit::Setting(*i)));
                if items[*i] == SettingItem::Setting(SettingKey::TermColors) {
                    let label_end = x + util::width(&label) as u16 + 2;
                    scheme_chip(buf, label_end, x + iw, y, app, bg);
                    continue;
                }
                let (value, style) = match toggle {
                    Some(true) => {
                        ("● On".to_string(), Style::default().fg(th.accent).bg(bg).add_modifier(Modifier::BOLD))
                    }
                    Some(false) => ("○ Off".to_string(), th.dim().bg(bg)),
                    None => (format!("‹ {text} ›"), Style::default().fg(th.accent2).bg(bg)),
                };
                hud::put_right(buf, x + iw, y, &value, style);
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
    let hint = "click or ⏎ to change · ←→ adjust · esc back";
    hud::put_right(buf, inner.right() - 1, inner.bottom() - 1, hint, th.dim());
}

/// The selected terminal scheme: a chip with its own background/text colors and
/// three color swatches, with a hint to open the selector on its left.
fn scheme_chip(buf: &mut Buffer, left: u16, right: u16, y: u16, app: &App, row_bg: Color) {
    let th = &app.theme;
    let (label, bg, fg, dots) = scheme_look(app, &app.cfg.terminal.colors);
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

/// How a scheme looks: name, background, text and color swatches (red, green, blue, …).
pub(super) fn scheme_look(app: &App, name: &str) -> (String, Color, Color, Vec<Color>) {
    let th = &app.theme;
    match crate::theme::find_scheme(&app.term_schemes, name) {
        Some(s) => (s.label.clone(), s.bg, s.fg, s.ansi.to_vec()),
        None => ("Follow theme".into(), th.bg, th.fg, vec![th.accent, th.accent2, th.ok]),
    }
}

/// Theme card: drawn with the theme's own background, text and accent colors.
fn theme_card(buf: &mut Buffer, card: Rect, app: &App, ti: usize, selected: bool) {
    let t = &THEMES[ti];
    let active = t.name == app.theme.name;
    let bg = t.bg;
    hud::fill(buf, card, Style::default().bg(bg));
    let marker = if selected { "▌" } else { " " };
    let mut cx = hud::put(buf, card.x, card.y, marker, Style::default().fg(app.theme.accent).bg(bg), 1);
    cx = hud::put(buf, cx, card.y, "██", Style::default().fg(t.accent).bg(bg), 2);
    cx = hud::put(buf, cx, card.y, "█", Style::default().fg(t.accent2).bg(bg), 1);
    cx = hud::put(buf, cx, card.y, " ", Style::default().bg(bg), 1);
    let name_w = card.right().saturating_sub(cx + 2);
    let mut st = Style::default().fg(t.fg).bg(bg);
    if selected || active {
        st = st.add_modifier(Modifier::BOLD);
    }
    hud::put(buf, cx, card.y, &util::truncate(t.label, name_w as usize), st, name_w);
    if active {
        hud::put(buf, card.right() - 2, card.y, "✓", Style::default().fg(t.ok).bg(bg), 1);
    }
}
