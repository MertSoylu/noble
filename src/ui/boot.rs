//! Boot sequence: four viewfinder brackets open from the centre, particles
//! scattered over the screen fly into the logo and every cell decodes through
//! random glyphs before it locks in. At the end the logo dissolves and the
//! brackets open out to the screen edges, revealing the app underneath. Any key
//! or click skips it.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use super::hud;
use crate::app::{App, BOOT_DURATION};
use crate::theme::Theme;
use crate::util;

// Timeline in milliseconds; `BOOT_DURATION` is the end of the closing iris.
/// The brackets open from the centre to the frame.
const OPEN_END: f64 = 500.0;
/// A logo particle takes off between these moments, left columns first.
const FLY_FROM: f64 = 150.0;
const FLY_SPREAD: f64 = 650.0;
const FLY_JITTER: f64 = 200.0;
const FLY_TIME: f64 = 650.0;
/// A landed cell shows random glyphs for about this long before it locks.
const SCRAMBLE: f64 = 300.0;
/// A freshly locked cell fades from the second accent into the accent.
const FLASH: f64 = 450.0;
/// Every cell turns into a dot and vanishes, then the iris opens.
const DISSOLVE_FROM: f64 = 2350.0;
const DISSOLVE_SPREAD: f64 = 180.0;
const DISSOLVE_DOT: f64 = 100.0;
const IRIS_FROM: f64 = 2640.0;

/// What a landed logo cell cycles through before it locks.
const BLOCKS: [&str; 10] = ["▓", "▒", "░", "▚", "▞", "▖", "▗", "▘", "▝", "▟"];
/// Particles waiting for take-off: single braille dots.
const SPARKS: [&str; 8] = ["⠁", "⠂", "⠄", "⡀", "⠈", "⠐", "⠠", "⢀"];

struct Cell {
    x: u16,
    y: u16,
    ch: char,
    /// Column position within its row, 0..1: sets the left-to-right order.
    order: f64,
}

struct Scene {
    /// Where the brackets settle and the iris starts from.
    frame: Rect,
    brackets: bool,
    cells: Vec<Cell>,
}

/// Milliseconds since the boot started, `None` when there is no boot.
pub fn elapsed(app: &App) -> Option<f64> {
    app.boot.as_ref().map(|b| b.started.elapsed().as_secs_f64() * 1000.0)
}

/// Does the boot still hide the app completely? After this the iris opens over it.
pub fn covers(ms: f64) -> bool {
    ms < IRIS_FROM
}

/// Draws the frame at `ms`; during the iris it goes over the already drawn app.
pub fn draw(buf: &mut Buffer, area: Rect, app: &App, ms: f64) {
    let Some(boot) = &app.boot else { return };
    let th = &app.theme;
    let seed = boot.seed;
    let scene = scene(area);

    if !covers(ms) {
        let total = BOOT_DURATION.as_secs_f64() * 1000.0;
        let e = ease_in_out((ms - IRIS_FROM) / (total - IRIS_FROM));
        let r = lerp_rect(scene.frame, area, e);
        // Everything outside the opening rectangle is still boot background.
        hud::clear(buf, Rect::new(area.x, area.y, area.width, r.y - area.y), th);
        hud::clear(buf, Rect::new(area.x, r.bottom(), area.width, area.bottom() - r.bottom()), th);
        hud::clear(buf, Rect::new(area.x, r.y, r.x - area.x, r.height), th);
        hud::clear(buf, Rect::new(r.right(), r.y, area.right() - r.right(), r.height), th);
        if scene.brackets {
            brackets(buf, r, Theme::mix(th.accent_dim, th.accent, e));
        }
        return;
    }

    if scene.brackets {
        let e = ease_out(ms / OPEN_END);
        let f = scene.frame;
        let (cx, cy) = (f.x as f64 + f.width as f64 / 2.0, f.y as f64 + f.height as f64 / 2.0);
        let dot = Rect::new((cx as u16).saturating_sub(1), (cy as u16).saturating_sub(1), 2, 2);
        let color = Theme::mix(th.accent2, th.accent_dim, (ms - OPEN_END) / 500.0);
        brackets(buf, lerp_rect(dot, f, e), color);
    }

    // Particles first: a landed cell draws over one still passing by.
    for (i, c) in scene.cells.iter().enumerate() {
        let (take_off, land, _) = logo_times(seed, i, c);
        if ms >= land {
            continue;
        }
        let start =
            (area.x as f64 + rnd(seed, i, 4) * area.width as f64, area.y as f64 + rnd(seed, i, 5) * area.height as f64);
        if ms < take_off {
            // Twinkles in at its start point, then waits.
            let appear = rnd(seed, i, 2) * take_off * 0.7;
            if ms >= appear {
                let color = Theme::mix(th.line, th.dim, (ms - appear) / 150.0);
                let spark = SPARKS[pick(seed, i, 6, SPARKS.len())];
                hud::put(buf, start.0 as u16, start.1 as u16, spark, Style::default().fg(color), 1);
            }
            continue;
        }
        let p = (ms - take_off) / FLY_TIME;
        let at = |e: f64| {
            let x = start.0 + (c.x as f64 - start.0) * e;
            let y = start.1 + (c.y as f64 - start.1) * e;
            (x.round() as u16, y.round() as u16)
        };
        let head = at(ease_out(p));
        let tail = at(ease_out(p - 0.1));
        if tail != head {
            hud::put(buf, tail.0, tail.1, "·", th.line(), 1);
        }
        hud::put(buf, head.0, head.1, "∙", Style::default().fg(Theme::mix(th.dim, th.accent, p)), 1);
    }

    for (i, c) in scene.cells.iter().enumerate() {
        let dissolve = DISSOLVE_FROM + rnd(seed, i, 3) * DISSOLVE_SPREAD;
        if ms >= dissolve {
            if ms < dissolve + DISSOLVE_DOT {
                hud::put(buf, c.x, c.y, "·", Style::default().fg(th.accent_dim), 1);
            }
            continue;
        }
        let (_, land, lock) = logo_times(seed, i, c);
        if ms < land {
            continue;
        }
        if ms < lock {
            // Decoding: a new random glyph every 50 ms.
            let glyph = BLOCKS[pick(seed, i, 100 + (ms / 50.0) as u64, BLOCKS.len())];
            hud::put(buf, c.x, c.y, glyph, Style::default().fg(th.accent_dim), 1);
        } else {
            let color = Theme::mix(th.accent2, th.accent, (ms - lock) / FLASH);
            let mut tmp = [0u8; 4];
            hud::put(buf, c.x, c.y, c.ch.encode_utf8(&mut tmp), Style::default().fg(color), 1);
        }
    }
}

/// Take-off, landing and lock moments of a logo cell.
fn logo_times(seed: u64, i: usize, c: &Cell) -> (f64, f64, f64) {
    let take_off = FLY_FROM + c.order * FLY_SPREAD + rnd(seed, i, 0) * FLY_JITTER;
    let land = take_off + FLY_TIME;
    (take_off, land, land + SCRAMBLE * (0.6 + 0.8 * rnd(seed, i, 1)))
}

/// Puts the logo in the middle of the screen; narrow windows get a one-line "NOBLE".
fn scene(area: Rect) -> Scene {
    let logo_w = util::width(hud::LOGO[0]) as u16;
    let rows: Vec<String> = if area.width >= logo_w + 2 && area.height >= 3 {
        hud::LOGO.iter().map(|l| l.to_string()).collect()
    } else {
        vec![util::truncate("NOBLE", area.width as usize)]
    };
    let content_h = rows.len() as u16;
    let content_w = rows.iter().map(|r| util::width(r) as u16).max().unwrap_or(0);

    // Corner row plus an empty row above and below the content when there is room.
    let pad_y = if area.height >= content_h + 6 {
        2
    } else if area.height >= content_h + 2 {
        1
    } else {
        0
    };
    let brackets = pad_y > 0 && area.width >= content_w + 4;
    let pad_x = match (brackets, area.width >= content_w + 16) {
        (false, _) => 0,
        (true, true) => 6,
        (true, false) => 2,
    };
    let frame_w = (content_w + pad_x * 2).min(area.width);
    let frame_h = (content_h + pad_y * 2).min(area.height);
    let frame = Rect::new(area.x + (area.width - frame_w) / 2, area.y + (area.height - frame_h) / 2, frame_w, frame_h);

    let mut cells = Vec::new();
    for (dy, text) in rows.iter().enumerate() {
        let w = util::width(text) as u16;
        let x0 = area.x + (area.width - w) / 2;
        let mut x = x0;
        let y = frame.y + pad_y + dy as u16;
        for ch in text.chars() {
            let mut tmp = [0u8; 4];
            let cw = util::width(ch.encode_utf8(&mut tmp)) as u16;
            if ch != ' ' && cw > 0 {
                cells.push(Cell { x, y, ch, order: (x - x0) as f64 / w as f64 });
            }
            x += cw;
        }
    }
    Scene { frame, brackets, cells }
}

/// Viewfinder corners: `┌─ ─┐` with a short vertical arm when the rectangle is tall enough.
fn brackets(buf: &mut Buffer, r: Rect, color: Color) {
    if r.width < 2 || r.height < 2 {
        return;
    }
    let st = Style::default().fg(color);
    let (l, t, rt, b) = (r.left(), r.top(), r.right() - 1, r.bottom() - 1);
    let arm = (r.width / 2).clamp(1, 3) - 1;
    for (x, y, corner, dx, dy) in [(l, t, "┌", 1, 1), (rt, t, "┐", -1, 1), (l, b, "└", 1, -1), (rt, b, "┘", -1, -1)]
    {
        for i in 1..=arm {
            hud::put(buf, x.saturating_add_signed(dx * i as i16), y, "─", st, 1);
        }
        if r.height >= 6 {
            hud::put(buf, x, y.saturating_add_signed(dy), "│", st, 1);
        }
        hud::put(buf, x, y, corner, st, 1);
    }
}

fn lerp_rect(a: Rect, b: Rect, e: f64) -> Rect {
    let l = |p: u16, q: u16| (p as f64 + (q as f64 - p as f64) * e).round() as u16;
    let (x, y) = (l(a.left(), b.left()), l(a.top(), b.top()));
    let (r, bt) = (l(a.right(), b.right()).max(x), l(a.bottom(), b.bottom()).max(y));
    Rect::new(x, y, r - x, bt - y)
}

fn ease_out(p: f64) -> f64 {
    1.0 - (1.0 - p.clamp(0.0, 1.0)).powi(3)
}

fn ease_in_out(p: f64) -> f64 {
    let p = p.clamp(0.0, 1.0);
    if p < 0.5 { 4.0 * p * p * p } else { 1.0 - (-2.0 * p + 2.0).powi(3) / 2.0 }
}

/// Deterministic 0..1 noise for (seed, cell, salt): splitmix64.
fn rnd(seed: u64, i: usize, salt: u64) -> f64 {
    let mut z = seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ salt.wrapping_mul(0xD1B5_4A32_D192_ED03);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

fn pick(seed: u64, i: usize, salt: u64, len: usize) -> usize {
    ((rnd(seed, i, salt) * len as f64) as usize).min(len - 1)
}
