//! Boot sequence: the logo appears scanline by scanline, the subsystems come
//! "online" one by one with their real status. Any key or click skips it.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::hud;
use crate::ai::{Presence, Status};
use crate::app::{App, BOOT_DURATION};
use crate::theme::Theme;
use crate::util;

pub fn draw(buf: &mut Buffer, area: Rect, app: &App) {
    let th = &app.theme;
    let Some(boot) = &app.boot else { return };
    let t = boot.started.elapsed().as_millis() as f64 / BOOT_DURATION.as_millis() as f64;
    let t = t.clamp(0.0, 1.0);
    let ms = boot.started.elapsed().as_millis();

    // Background grid: sparse dots, the scan line moves downwards.
    let scan_y = area.y + ((area.height as f64) * (t * 1.4).min(1.0)) as u16;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if x % 6 == 0 && y % 3 == 0 {
                let near = (y as i32 - scan_y as i32).abs();
                let color = if near <= 1 { th.accent_dim } else { th.line };
                hud::put(buf, x, y, "·", Style::default().fg(color), 1);
            }
        }
    }
    if scan_y < area.bottom() {
        for x in area.left()..area.right() {
            hud::put(buf, x, scan_y, "─", Style::default().fg(Theme::mix(th.line, th.accent_dim, 0.6)), 1);
        }
    }

    let logo_w = util::width(hud::LOGO[0]) as u16;
    let lines = checklist(app);
    let block_h = 3 + 2 + lines.len() as u16 + 3;
    let top = area.y + area.height.saturating_sub(block_h) / 2;
    let content_w = 46u16.min(area.width.saturating_sub(2));
    let left = area.x + area.width.saturating_sub(content_w) / 2;
    // Keep the back of the content block clean (grid dots must not mix into the text).
    let pad_w = content_w.max(logo_w).saturating_add(6).min(area.width);
    let block = Rect::new(area.x + (area.width - pad_w) / 2, top.saturating_sub(1).max(area.y), pad_w, block_h + 2);
    hud::clear(buf, block.intersection(area), th);

    // Logo: reveals left to right, the scan head is bright.
    if area.width > logo_w + 2 && area.height >= 12 {
        let lx = area.x + (area.width - logo_w) / 2;
        let reveal = ((t / 0.45).min(1.0) * logo_w as f64) as u16;
        for (row, line) in hud::LOGO.iter().enumerate() {
            for (i, ch) in line.chars().enumerate() {
                let i = i as u16;
                if i > reveal {
                    break;
                }
                let color = if reveal.saturating_sub(i) <= 1 && reveal < logo_w { th.accent2 } else { th.accent };
                let mut tmp = [0u8; 4];
                hud::put(buf, lx + i, top + row as u16, ch.encode_utf8(&mut tmp), Style::default().fg(color), 1);
            }
        }
        hud::put_center(
            buf,
            area,
            top + 4,
            &format!("HUD TERMINAL WORKSPACE · v{}", env!("CARGO_PKG_VERSION")),
            th.dim(),
        );
    } else {
        hud::put_center(buf, area, top + 1, "◢◤ NOBLE", th.accent_bold());
    }

    // Checklist: each row appears in turn.
    let list_y = top + 6;
    let dots_w = content_w.saturating_sub(8 + 18);
    for (i, (label, value, ready)) in lines.iter().enumerate() {
        let appear = 0.25 + i as f64 * 0.11;
        if t < appear {
            break;
        }
        let y = list_y + i as u16;
        let (badge, badge_style) = if *ready {
            ("[ OK ]", Style::default().fg(th.ok).add_modifier(Modifier::BOLD))
        } else {
            ("[ .. ]", Style::default().fg(th.warn))
        };
        let x = hud::put(buf, left, y, badge, badge_style, 6);
        let x = hud::put(buf, x + 1, y, &util::pad_right(label, 14), th.text(), 14);
        let dots = ".".repeat(dots_w.saturating_sub(14).max(2) as usize);
        let x = hud::put(buf, x, y, &dots, th.line(), dots_w);
        hud::put(buf, x + 1, y, &util::truncate(value, 22), th.accent2(), 22);
    }

    // Progress bar.
    let py = list_y + lines.len() as u16 + 1;
    hud::bar(buf, left, py, content_w, t * 100.0, th.accent, th);
    let blink = (ms / 400) % 2 == 0;
    if blink {
        hud::put_center(buf, area, py + 1, "PRESS ANY KEY", th.dim());
    }
}

fn checklist(app: &App) -> Vec<(String, String, bool)> {
    let shell = app.shell.label();
    let pty = if cfg!(windows) { format!("CONPTY · {shell}") } else { format!("PTY · {shell}") };
    let sensors = if app.sensors.info.cores > 0 {
        (format!("{} CORES · {}", app.sensors.info.cores, util::fmt_bytes(app.sensors.info.total_mem)), true)
    } else {
        ("CALIBRATING".into(), false)
    };
    let projects =
        if app.projects_loaded { (format!("{} REPOS", app.projects.len()), true) } else { ("INDEXING".into(), false) };
    let linked = app.ai.iter().filter(|p| p.presence == Presence::Ready && p.status != Status::SignIn).count();
    let ai = if !app.cfg.ai.enabled {
        ("DISABLED".into(), true)
    } else if app.ai.is_empty() {
        ("HANDSHAKE".into(), false)
    } else {
        (format!("{linked} PROVIDER{}", if linked == 1 { "" } else { "S" }), true)
    };
    let session =
        if app.restored_tabs > 0 { format!("{} TABS RESTORED", app.restored_tabs) } else { "CLEAN START".to_string() };
    vec![
        ("PSEUDO-TTY".into(), pty, true),
        ("SENSOR ARRAY".into(), sensors.0, sensors.1),
        ("PROJECT INDEX".into(), projects.0, projects.1),
        ("AI LINK".into(), ai.0, ai.1),
        ("SESSION".into(), session, true),
    ]
}
