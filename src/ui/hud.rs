//! HUD drawing primitives. All of them are bounds-safe against the buffer:
//! nothing overflows or panics in small windows.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::theme::Theme;
use crate::util;

/// Fills an area with the background color.
pub fn clear(buf: &mut Buffer, area: Rect, th: &Theme) {
    fill(buf, area, Style::default().bg(th.bg).fg(th.fg));
}

pub fn fill(buf: &mut Buffer, area: Rect, style: Style) {
    let area = area.intersection(buf.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.reset();
                c.set_style(style);
            }
        }
    }
}

/// Writes text, at most `max` columns; returns the x where it ends.
pub fn put(buf: &mut Buffer, x: u16, y: u16, s: &str, style: Style, max: u16) -> u16 {
    let area = buf.area;
    if y < area.top() || y >= area.bottom() || x >= area.right() || x < area.left() || max == 0 {
        return x;
    }
    let limit = max.min(area.right() - x);
    let (nx, _) = buf.set_stringn(x, y, s, limit as usize, style);
    nx
}

/// Writes the parts one after another.
pub fn put_spans(buf: &mut Buffer, x: u16, y: u16, spans: &[(&str, Style)], max: u16) -> u16 {
    let end = x.saturating_add(max);
    let mut cx = x;
    for (s, st) in spans {
        if cx >= end {
            break;
        }
        cx = put(buf, cx, y, s, *st, end - cx);
    }
    cx
}

/// Right-aligns text to end at column `right` (exclusive); returns the x it starts at.
pub fn put_right(buf: &mut Buffer, right: u16, y: u16, s: &str, style: Style) -> u16 {
    let w = util::width(s) as u16;
    let x = right.saturating_sub(w);
    put(buf, x, y, s, style, w);
    x
}

/// Writes text in the middle of the area.
pub fn put_center(buf: &mut Buffer, area: Rect, y: u16, s: &str, style: Style) {
    let w = (util::width(s) as u16).min(area.width);
    let x = area.x + (area.width - w) / 2;
    put(buf, x, y, s, style, area.width);
}

pub fn hline(buf: &mut Buffer, x: u16, y: u16, w: u16, ch: &str, style: Style) {
    for i in 0..w {
        put(buf, x + i, y, ch, style, 1);
    }
}

pub fn set_bg_row(buf: &mut Buffer, x: u16, y: u16, w: u16, color: Color) {
    for i in 0..w {
        if let Some(c) = buf.cell_mut((x + i, y)) {
            c.set_bg(color);
        }
    }
}

/// Rounded-corner HUD panel. Title on the left, label on the right. Returns the inner area.
pub fn frame(buf: &mut Buffer, area: Rect, title: &str, tag: &str, focused: bool, th: &Theme) -> Rect {
    if area.width < 4 || area.height < 2 {
        return Rect::new(area.x, area.y, 0, 0);
    }
    let line = if focused { Style::default().fg(th.accent_dim) } else { th.line() };
    let corner = line;
    let (l, r, t, b) = (area.left(), area.right() - 1, area.top(), area.bottom() - 1);
    hline(buf, l + 1, t, area.width - 2, "─", line);
    hline(buf, l + 1, b, area.width - 2, "─", line);
    for y in t + 1..b {
        put(buf, l, y, "│", line, 1);
        put(buf, r, y, "│", line, 1);
    }
    put(buf, l, t, "╭", corner, 1);
    put(buf, r, t, "╮", corner, 1);
    put(buf, l, b, "╰", corner, 1);
    put(buf, r, b, "╯", corner, 1);
    let max_title = area.width.saturating_sub(6);
    if !title.is_empty() && max_title > 2 {
        let title_style =
            if focused { th.accent_bold() } else { Style::default().fg(th.fg).add_modifier(Modifier::BOLD) };
        let x = put(buf, l + 1, t, " ", line, 1);
        let x = put(buf, x, t, &util::truncate(title, max_title as usize), title_style, max_title);
        put(buf, x, t, " ", line, 1);
        let used = x - l;
        if !tag.is_empty() && area.width > used + 6 {
            let room = (area.width - used - 5) as usize;
            let tag = format!(" {} ", util::truncate(tag, room));
            put_right(buf, r - 1, t, &tag, th.dim());
        }
    }
    Rect::new(area.x + 1, area.y + 1, area.width.saturating_sub(2), area.height.saturating_sub(2))
}

/// Fill bar on a thin track: the filled part is a colored `━`, the rest of the track is dim.
pub fn bar(buf: &mut Buffer, x: u16, y: u16, w: u16, pct: f64, color: Color, th: &Theme) {
    if w == 0 {
        return;
    }
    let exact = (pct.clamp(0.0, 100.0) / 100.0) * w as f64;
    let full = exact.floor() as u16;
    let half = exact - full as f64 >= 0.5;
    for i in 0..w {
        let (ch, st) = if i < full {
            ("━", Style::default().fg(color))
        } else if i == full && half {
            ("╸", Style::default().fg(color))
        } else {
            ("━", th.line())
        };
        put(buf, x + i, y, ch, st, 1);
    }
}

const SPARK: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Sparkline from 0..1 values.
pub fn spark_char(v: f64) -> char {
    let i = (v.clamp(0.0, 1.0) * 7.0).round() as usize;
    SPARK[i.min(7)]
}

/// Braille area chart: 2 samples × 4 levels per cell. `values` are 0..1, newest last.
/// The chart is right-aligned; old samples flow away to the left.
pub fn braille_area(buf: &mut Buffer, area: Rect, values: &[f64], low: Color, high: Color) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let levels = area.height as f64 * 4.0;
    let n = values.len();
    let cols = area.width as usize * 2;
    const LEFT: [u32; 4] = [0x40, 0x04, 0x02, 0x01];
    const RIGHT: [u32; 4] = [0x80, 0x20, 0x10, 0x08];
    for cx in 0..area.width as usize {
        for cy in 0..area.height as usize {
            let base = (area.height as usize - 1 - cy) * 4;
            let mut bits = 0u32;
            for (side, table) in [(0usize, &LEFT), (1, &RIGHT)] {
                let col = cx * 2 + side;
                // Right-aligned: the newest sample in the last column.
                let idx = (n as isize) - (cols as isize - col as isize);
                if idx < 0 {
                    continue;
                }
                let v = values[idx as usize].clamp(0.0, 1.0);
                let mut dots = (v * levels).round() as usize;
                if v > 0.0 && dots == 0 {
                    dots = 1;
                }
                let count = dots.saturating_sub(base).min(4);
                for bit in table.iter().take(count) {
                    bits |= bit;
                }
            }
            let x = area.x + cx as u16;
            let y = area.y + cy as u16;
            let t = if area.height > 1 { 1.0 - cy as f64 / (area.height as f64 - 1.0) } else { 1.0 };
            let color = Theme::mix(low, high, t);
            if bits != 0 {
                let ch = char::from_u32(0x2800 + bits).unwrap_or(' ');
                let mut tmp = [0u8; 4];
                put(buf, x, y, ch.encode_utf8(&mut tmp), Style::default().fg(color), 1);
            }
        }
    }
}

/// Eighth fill blocks (1/8 … 7/8).
const EIGHTHS: [&str; 7] = ["▏", "▎", "▍", "▌", "▋", "▊", "▉"];

/// Battery icon: body + terminal on the right; the inside fills with eighth
/// blocks according to the percentage, with a dark-to-bright color transition.
/// Framed in 3 rows, a single row in less space. `shimmer`: the cell to shine
/// inside the fill (charging animation).
pub fn battery_icon(buf: &mut Buffer, area: Rect, pct: f64, color: Color, th: &Theme, shimmer: Option<u16>) {
    if area.width < 6 || area.height == 0 {
        return;
    }
    let tall = area.height >= 3;
    let y = if tall { area.y + 1 } else { area.y };
    // The last column is the terminal; two edges of the body are the frame.
    let body_w = area.width - 1;
    let inner_w = body_w - 2;
    let edge = Style::default().fg(Theme::mix(th.line, th.fg, 0.35));
    let (l, r) = (area.x, area.x + body_w - 1);
    if tall {
        put(buf, l, area.y, "╭", edge, 1);
        hline(buf, l + 1, area.y, inner_w, "─", edge);
        put(buf, r, area.y, "╮", edge, 1);
        put(buf, l, y, "│", edge, 1);
        put(buf, r, y, "│", edge, 1);
        put(buf, l, area.y + 2, "╰", edge, 1);
        hline(buf, l + 1, area.y + 2, inner_w, "─", edge);
        put(buf, r, area.y + 2, "╯", edge, 1);
        put(buf, r + 1, y, "▌", edge, 1);
    } else {
        put(buf, l, y, "▕", edge, 1);
        put(buf, r, y, "▏", edge, 1);
        put(buf, r + 1, y, "▍", edge, 1);
    }
    let exact = pct.clamp(0.0, 100.0) / 100.0 * inner_w as f64;
    let mut full = exact.floor() as u16;
    let mut frac = ((exact - full as f64) * 8.0).round() as usize;
    if frac == 8 {
        full += 1;
        frac = 0;
    }
    let dark = Theme::mix(color, th.bg, 0.45);
    for i in 0..inner_w {
        let t = if inner_w > 1 { i as f64 / (inner_w - 1) as f64 } else { 1.0 };
        let mut c = Theme::mix(dark, color, t);
        if shimmer == Some(i) {
            c = Theme::mix(color, th.fg, 0.6);
        }
        let (sym, st) = if i < full {
            ("█", Style::default().fg(c))
        } else if i == full && frac >= 3 {
            // The 1/8 and 2/8 slices blend into the frame line; they are not drawn.
            (EIGHTHS[frac - 1], Style::default().fg(c))
        } else {
            ("░", th.line())
        };
        put(buf, l + 1 + i, y, sym, st, 1);
    }
}

/// 3-row box drawing digits (for the big clock).
fn glyph(c: char) -> [&'static str; 3] {
    match c {
        '0' => ["┏━┓", "┃ ┃", "┗━┛"],
        '1' => ["╺┓ ", " ┃ ", "╺┻╸"],
        '2' => ["╺━┓", "┏━┛", "┗━╸"],
        '3' => ["╺━┓", " ━┫", "╺━┛"],
        '4' => ["╻ ╻", "┗━┫", "  ╹"],
        '5' => ["┏━╸", "┗━┓", "╺━┛"],
        '6' => ["┏━╸", "┣━┓", "┗━┛"],
        '7' => ["╺━┓", "  ┃", "  ╹"],
        '8' => ["┏━┓", "┣━┫", "┗━┛"],
        '9' => ["┏━┓", "┗━┫", "╺━┛"],
        ':' => ["▄", " ", "▀"],
        _ => ["   ", "   ", "   "],
    }
}

pub fn big_width(s: &str) -> u16 {
    s.chars().map(|c| util::width(glyph(c)[0]) as u16 + 1).sum::<u16>().saturating_sub(1)
}

/// Writes the big text; `colon_style` for the colons (blinking).
pub fn big_text(buf: &mut Buffer, x: u16, y: u16, s: &str, style: Style, colon_style: Style) {
    let mut cx = x;
    for c in s.chars() {
        let g = glyph(c);
        let st = if c == ':' { colon_style } else { style };
        for (row, line) in g.iter().enumerate() {
            put(buf, cx, y + row as u16, line, st, 3);
        }
        cx += util::width(g[0]) as u16 + 1;
    }
}

/// Boot logo (3 rows of block letters).
pub const LOGO: [&str; 3] = ["█▄ █ █▀▀█ █▀▀▄ █    █▀▀▀", "█ ▀█ █  █ █▀▀▄ █    █▀▀ ", "▀  ▀ ▀▀▀▀ ▀▀▀  ▀▀▀▀ ▀▀▀▀"];

/// Spinning indicator square.
pub fn spinner(ms: u128) -> &'static str {
    const FRAMES: [&str; 4] = ["◜", "◝", "◞", "◟"];
    FRAMES[((ms / 120) % 4) as usize]
}

/// Clickable button: " ⏎ Open " — the key in the accent color, background slightly raised.
pub fn button(buf: &mut Buffer, x: u16, y: u16, key: &str, label: &str, th: &Theme, enabled: bool) -> u16 {
    let bg = th.sel_bg;
    let key_style = if enabled { th.accent_bold().bg(bg) } else { th.dim().bg(bg) };
    let label_style = if enabled { th.text().bg(bg) } else { th.dim().bg(bg) };
    put_spans(
        buf,
        x,
        y,
        &[(" ", label_style), (key, key_style), (" ", label_style), (label, label_style), (" ", label_style)],
        60,
    )
}

/// Short key badge: " c " on an accent background.
#[allow(clippy::too_many_arguments)]
pub fn key_chip(buf: &mut Buffer, x: u16, y: u16, key: &str, label: &str, th: &Theme, enabled: bool, max: u16) -> u16 {
    let key_style = if enabled {
        Style::default().fg(th.on_accent).bg(th.accent_dim).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(th.dim).bg(th.line)
    };
    let label_style = if enabled { th.text() } else { th.dim() };
    put_spans(buf, x, y, &[(&format!(" {key} "), key_style), (" ", th.text()), (label, label_style)], max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_are_bounds_safe() {
        let th = crate::theme::Theme::by_name("amber", false);
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
        put(&mut buf, 8, 0, "hello", th.text(), 20);
        put(&mut buf, 20, 20, "x", th.text(), 5);
        frame(&mut buf, Rect::new(0, 0, 30, 10), "TITLE", "tag", true, &th);
        bar(&mut buf, 5, 1, 20, 50.0, th.accent, &th);
        braille_area(&mut buf, Rect::new(0, 0, 20, 8), &[0.5; 50], th.accent_dim, th.accent);
        big_text(&mut buf, 0, 2, "12:34", th.text(), th.text());
        assert_eq!(buf.area, Rect::new(0, 0, 10, 4));
    }

    #[test]
    fn braille_fills_from_bottom() {
        let th = crate::theme::Theme::by_name("amber", false);
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        braille_area(&mut buf, Rect::new(0, 0, 1, 1), &[1.0, 0.25], th.accent, th.accent);
        // Left column full (4 dots), right column 1 dot.
        assert_eq!(buf[(0, 0)].symbol(), char::from_u32(0x2800 + 0x47 + 0x80).unwrap().to_string());
    }

    #[test]
    fn battery_icon_fills_by_percent() {
        let th = crate::theme::Theme::by_name("amber", false);
        let mut buf = Buffer::empty(Rect::new(0, 0, 14, 3));
        // Width 12: 1 terminal + 2 edges → 9 inner cells; 50% = 4.5 cells.
        battery_icon(&mut buf, Rect::new(0, 0, 12, 3), 50.0, th.ok, &th, None);
        let row: String = (0..12).map(|x| buf[(x, 1)].symbol().to_string()).collect();
        assert_eq!(row, "│████▌░░░░│▌");
        assert_eq!(buf[(0, 0)].symbol(), "╭");
        let mut one = Buffer::empty(Rect::new(0, 0, 8, 1));
        battery_icon(&mut one, Rect::new(0, 0, 8, 1), 100.0, th.ok, &th, Some(2));
        let row: String = (0..8).map(|x| one[(x, 0)].symbol().to_string()).collect();
        assert_eq!(row, "▕█████▏▍");
        // No overflow.
        battery_icon(&mut one, Rect::new(5, 0, 30, 5), 70.0, th.ok, &th, None);
    }

    #[test]
    fn big_digits_width() {
        assert_eq!(big_width("12:34"), 3 + 1 + 3 + 1 + 1 + 1 + 3 + 1 + 3);
    }
}
