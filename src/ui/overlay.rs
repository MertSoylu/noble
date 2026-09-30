//! Overlays: command palette, key reference, confirmation and text input.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::hud;
use crate::app::{App, Hit, Overlay};
use crate::keys::Action;
use crate::theme::Theme;
use crate::util;

pub fn draw(buf: &mut Buffer, area: Rect, app: &App, hits: &mut Vec<(Rect, Hit)>) {
    let Some(ov) = &app.overlay else { return };
    let th = &app.theme;
    // The right-click menu does not dim the screen; clicking outside still closes it.
    if !matches!(ov, Overlay::Menu(_)) {
        dim_backdrop(buf, area, th);
    }
    hits.push((area, Hit::Backdrop));
    match ov {
        Overlay::Welcome(ws) => welcome(buf, area, app, ws, hits),
        Overlay::Palette(st) => palette(buf, area, app, st, hits),
        Overlay::Schemes(p) => schemes(buf, area, app, p, hits),
        Overlay::Themes(p) => themes(buf, area, app, p, hits),
        Overlay::Launchers { selected } => launchers(buf, area, app, *selected, hits),
        Overlay::Menu(m) => menu(buf, area, app, m, hits),
        Overlay::Help { scroll } => help(buf, area, app, *scroll, hits),
        Overlay::Confirm(c) => {
            let w = 52.min(area.width.saturating_sub(2));
            let rect = centered(area, w, 7);
            hits.push((rect, Hit::Inert));
            let inner = boxed(buf, rect, &c.title, th, th.crit);
            let x = inner.x + 1;
            hud::put(
                buf,
                x,
                inner.y + 1,
                &util::truncate(&c.body, (inner.width as usize).saturating_sub(2)),
                th.text(),
                inner.width.saturating_sub(2),
            );
            let y = inner.bottom().saturating_sub(1);
            let yes_x = x;
            let end = hud::put_spans(
                buf,
                yes_x,
                y,
                &[
                    (" y ", Style::default().fg(th.on_accent).bg(th.crit).add_modifier(Modifier::BOLD)),
                    (" confirm", th.text()),
                ],
                20,
            );
            hits.push((Rect::new(yes_x, y, end - yes_x, 1), Hit::ConfirmYes));
            let no_x = end + 3;
            let end = hud::put_spans(
                buf,
                no_x,
                y,
                &[
                    (" n/esc ", Style::default().fg(th.fg).bg(th.line).add_modifier(Modifier::BOLD)),
                    (" cancel", th.dim()),
                ],
                24,
            );
            hits.push((Rect::new(no_x, y, end - no_x, 1), Hit::ConfirmNo));
        }
        Overlay::Prompt(p) => {
            let w = 56.min(area.width.saturating_sub(2));
            let rect = centered(area, w, 5);
            hits.push((rect, Hit::Inert));
            let inner = boxed(buf, rect, &p.title, th, th.accent);
            let x = inner.x + 1;
            let y = inner.y + 1;
            let ms = app.started.elapsed().as_millis();
            let cursor = if (ms / 500).is_multiple_of(2) { "▏" } else { " " };
            let split = p.value.char_indices().nth(p.cursor).map_or(p.value.len(), |(i, _)| i);
            let (before, after) = p.value.split_at(split);
            let field_w = inner.width.saturating_sub(2);
            if p.value.is_empty() {
                // Placeholder while empty.
                let hint = match p.purpose {
                    crate::app::PromptPurpose::AddRoot | crate::app::PromptPurpose::AddProject => {
                        "path, e.g. ~/code/app"
                    }
                    crate::app::PromptPurpose::RenameTab(_) => "tab name (empty = automatic)",
                    crate::app::PromptPurpose::SaveWorkspace => "workspace name",
                };
                hud::put_spans(buf, x, y, &[("❯ ", th.accent()), (cursor, th.accent()), (hint, th.dim())], field_w);
            } else {
                // Long values scroll: the part before the cursor keeps its tail visible.
                let room = (field_w as usize).saturating_sub(3);
                let mut kept: Vec<char> = Vec::new();
                let mut used = 0usize;
                for c in before.chars().rev() {
                    let w = util::width(&c.to_string());
                    if used + w > room {
                        break;
                    }
                    used += w;
                    kept.push(c);
                }
                kept.reverse();
                let before_shown: String = kept.into_iter().collect();
                let after_shown = util::truncate(after, room.saturating_sub(used).max(1));
                hud::put_spans(
                    buf,
                    x,
                    y,
                    &[
                        ("❯ ", th.accent()),
                        (&before_shown, th.accent_bold()),
                        (cursor, th.accent()),
                        (&after_shown, th.accent_bold()),
                    ],
                    field_w,
                );
            }
            if let Some(err) = &p.error {
                hud::put(
                    buf,
                    x,
                    inner.y + 2,
                    &util::truncate(err, field_w as usize),
                    Style::default().fg(th.crit),
                    field_w,
                );
            }
            let add = p.purpose.is_add();
            if add {
                // Choice line above the input: one project or a folder to scan (tab / click).
                let project = matches!(p.purpose, crate::app::PromptPurpose::AddProject);
                let mut cx = x;
                let end = inner.right().saturating_sub(1);
                for (mode, label) in [(true, "one project"), (false, "folder of projects (scan)")] {
                    let on = mode == project;
                    let start = cx;
                    let (dot, style) = if on { ("● ", th.accent_bold()) } else { ("○ ", th.dim()) };
                    cx = hud::put_spans(buf, cx, inner.y, &[(dot, style), (label, style)], end.saturating_sub(cx));
                    if cx > start {
                        hits.push((Rect::new(start, inner.y, cx - start, 1), Hit::PromptMode(mode)));
                    }
                    cx = hud::put(buf, cx, inner.y, "   ", th.dim(), end.saturating_sub(cx));
                }
            }
            // The longest hint that fits (a narrow window drops "tab switch" first).
            let hints: &[&str] = if add {
                &["tab switch · ⏎ add · esc cancel", "⏎ add · esc cancel"]
            } else {
                &["⏎ save · esc cancel"]
            };
            if p.error.is_none()
                && let Some(hint) = hints.iter().find(|h| util::width(h) + 2 <= inner.width as usize)
            {
                hud::put_right(buf, inner.right().saturating_sub(1), inner.bottom().saturating_sub(1), hint, th.dim());
            }
        }
    }
}

fn dim_backdrop(buf: &mut Buffer, area: Rect, th: &Theme) {
    let area = area.intersection(buf.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_fg(th.line);
                if th.bg != ratatui::style::Color::Reset {
                    c.set_bg(th.bg);
                }
            }
        }
    }
}

/// Right-click menu: next to the clicked point, kept inside the screen.
fn menu(buf: &mut Buffer, area: Rect, app: &App, m: &crate::app::Menu, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    let label_w = m.items.iter().map(|i| util::width(&i.label)).max().unwrap_or(4) as u16;
    let hint_w = m.items.iter().map(|i| util::width(&i.hint)).max().unwrap_or(0) as u16;
    let title_w = util::width(&m.title) as u16 + 6;
    let w = (label_w + hint_w + 8).max(title_w).clamp(16, 44).min(area.width);
    let h = (m.items.len() as u16 + 2).min(area.height);
    // Slides left/up when it would overflow right/bottom.
    let x = m.x.min(area.right().saturating_sub(w));
    let y = if m.y + h > area.bottom() { m.y.saturating_sub(h + 1).max(area.y) } else { m.y };
    let rect = Rect::new(x, y, w, h);
    hits.push((rect, Hit::Inert));
    let inner = boxed(buf, rect, &util::truncate(&m.title, (w as usize).saturating_sub(6)), th, th.accent_dim);
    for (i, it) in m.items.iter().enumerate().take(inner.height as usize) {
        let ry = inner.y + i as u16;
        let row = Rect::new(inner.x, ry, inner.width, 1);
        let selected = i == m.selected;
        let bg = if selected { th.sel_bg } else { th.raised };
        hud::fill(buf, row, Style::default().bg(bg));
        let marker = if selected { "▌" } else { " " };
        hud::put(buf, row.x, ry, marker, Style::default().fg(th.accent).bg(bg), 1);
        let style = if !it.enabled {
            th.dim().bg(bg)
        } else if selected {
            th.accent_bold().bg(bg)
        } else {
            th.text().bg(bg)
        };
        hud::put(buf, row.x + 2, ry, &it.label, style, row.width.saturating_sub(3));
        if !it.hint.is_empty() && row.width > label_w + 6 {
            hud::put_right(buf, row.right() - 1, ry, &it.hint, th.dim().bg(bg));
        }
        hits.push((row, Hit::MenuItem(i)));
    }
}

/// Short descriptions of the prefix options (`PREFIXES` order).
const PREFIX_NOTES: [&str; 4] = [
    "tmux style · shell's ctrl+a takes 2 presses",
    "tmux default · shell's ctrl+b takes 2 presses",
    "PowerShell's ctrl+space menu takes 2 presses",
    "rarely used by shells · stays out of your way",
];

/// First launch card: what was found, a short setup (theme, terminal colors, shell, prefix)
/// and the most important keys. At small sizes the key list is cut before the setup rows.
fn welcome(buf: &mut Buffer, area: Rect, app: &App, ws: &crate::app::WelcomeSetup, hits: &mut Vec<(Rect, Hit)>) {
    use crate::app::WelcomeRow;
    let th = &app.theme;
    let w = 66.min(area.width.saturating_sub(2));
    let h = 18.min(area.height.saturating_sub(1));
    let rect = centered(area, w, h);
    hits.push((rect, Hit::Inert));
    let inner = boxed(buf, rect, "WELCOME TO NOBLE", th, th.accent);
    if inner.height < 4 || inner.width < 20 {
        return;
    }
    let x = inner.x + 2;
    let iw = inner.width.saturating_sub(4);
    // A short window skips the padding and the projects line so the setup rows stay visible.
    let roomy = inner.height >= 14;
    let mut y = inner.y + u16::from(roomy);
    // The last row belongs to the Start button.
    let bottom = inner.bottom().saturating_sub(1);
    let line = |buf: &mut Buffer, y: u16, spans: &[(&str, Style)]| {
        if y < bottom {
            hud::put_spans(buf, x, y, spans, iw);
        }
    };
    if roomy {
        let found = if !app.projects_loaded {
            "looking for your git projects…".to_string()
        } else if app.projects.is_empty() {
            "no git projects found yet — press a on Home to add a folder".to_string()
        } else {
            format!("found {} git projects", app.projects.len())
        };
        line(buf, y, &[("● ", th.accent2()), (&found, th.text())]);
        y += 2;
    }
    let rows = ws.rows();
    // Too short for the heading and every row (30×8): the rows win, the heading goes.
    if bottom.saturating_sub(y) as usize > rows.len() {
        line(buf, y, &[("SET UP", th.accent_bold()), ("  ↑↓ choose · ←→ change", th.dim())]);
        y += 1;
    }
    // Setup rows: label column, then the value between ‹ › (the prefix as chips).
    // A narrow card uses short labels so the values keep some room.
    let short = iw < 44;
    let label_w: u16 = if short { 8 } else { 17 };
    let vx = x + label_w;
    let vend = x + iw;
    let theme_label = crate::theme::THEMES.get(ws.theme).map_or("", |t| t.label);
    // The prefix chips need room for all four; otherwise the prefix is a ‹ value › row too.
    let chips_w: u16 = crate::app::PREFIXES.iter().map(|p| util::width(p) as u16 + 3).sum();
    let chips = vend.saturating_sub(vx) >= chips_w;
    for (i, row) in rows.into_iter().enumerate() {
        if y >= bottom {
            break;
        }
        let on = i == ws.row;
        hits.push((Rect::new(inner.x, y, inner.width, 1), Hit::WelcomeRow(i)));
        if on {
            hud::put(buf, x - 1, y, "▌", th.accent(), 1);
        }
        let label = match (row, short) {
            (WelcomeRow::Theme, _) => "Theme",
            (WelcomeRow::Colors, false) => "Terminal colors",
            (WelcomeRow::Colors, true) => "Colors",
            (WelcomeRow::Shell, _) => "Shell",
            (WelcomeRow::Prefix, false) => "Prefix key",
            (WelcomeRow::Prefix, true) => "Prefix",
        };
        hud::put(buf, x, y, label, if on { th.accent_bold() } else { th.text() }, label_w.min(iw));
        if row == WelcomeRow::Prefix && chips {
            let mut cx = vx;
            for (pi, p) in crate::app::PREFIXES.iter().enumerate() {
                let style = if pi == ws.prefix {
                    Style::default().fg(th.on_accent).bg(th.accent).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(th.fg).bg(th.sel_bg)
                };
                let start = cx;
                cx = hud::put(buf, cx, y, &format!(" {p} "), style, vend.saturating_sub(cx));
                if cx > start {
                    hits.push((Rect::new(start, y, cx - start, 1), Hit::WelcomePrefix(pi)));
                }
                cx += 1;
            }
            y += 1;
            if y < bottom {
                hud::put(
                    buf,
                    vx,
                    y,
                    PREFIX_NOTES.get(ws.prefix).copied().unwrap_or(""),
                    th.dim(),
                    vend.saturating_sub(vx),
                );
            }
        } else {
            let value = match row {
                WelcomeRow::Theme => theme_label.to_string(),
                WelcomeRow::Colors => ws.schemes.get(ws.colors).map(|s| app.scheme_label(s)).unwrap_or_default(),
                WelcomeRow::Shell => ws.shells.get(ws.shell).map(|s| app.shell_option_label(s)).unwrap_or_default(),
                WelcomeRow::Prefix => crate::app::PREFIXES.get(ws.prefix).copied().unwrap_or_default().to_string(),
            };
            let arrow = if on { th.accent_bold() } else { th.dim() };
            // ‹ value ›, the value padded so the right arrow stays put while cycling.
            let value_w = 24.min(vend.saturating_sub(vx + 4) as usize);
            let left_end = hud::put(buf, vx, y, "‹ ", arrow, vend.saturating_sub(vx));
            let text_end = hud::put(
                buf,
                left_end,
                y,
                &util::pad_right(&util::truncate(&value, value_w), value_w),
                if on { th.accent_bold() } else { th.text() },
                vend.saturating_sub(left_end),
            );
            let right_end = hud::put(buf, text_end, y, " ›", arrow, vend.saturating_sub(text_end));
            // Clicking the value itself steps forward, like the right arrow.
            if text_end > left_end {
                hits.push((Rect::new(left_end, y, text_end - left_end, 1), Hit::WelcomeStep(i, 1)));
            }
            if left_end > vx {
                hits.push((Rect::new(vx, y, left_end - vx, 1), Hit::WelcomeStep(i, -1)));
            }
            if right_end > text_end {
                hits.push((Rect::new(text_end, y, right_end - text_end, 1), Hit::WelcomeStep(i, 1)));
            }
        }
        y += 1;
    }
    y += 1;
    // The launcher line lists only the CLIs that are installed (and shown); none installed, no line.
    let launchers: Vec<_> = app.quick_launchers().map(|(_, l)| l).collect();
    let mut keys = vec![("⏎".to_string(), "open a terminal in the selected project".to_string())];
    if !launchers.is_empty() {
        let keys_text = launchers.iter().map(|l| l.key.as_str()).collect::<Vec<_>>().join("  ");
        let names = launchers.iter().map(|l| super::bridge::capitalize(&l.name)).collect::<Vec<_>>().join(" / ");
        keys.push((keys_text, format!("start {names} right there")));
    }
    keys.push(("alt+p".into(), "command palette — everything is in there".into()));
    keys.push(("?".into(), "every shortcut · ctrl+click opens links".into()));
    for (key, what) in &keys {
        line(buf, y, &[(&util::pad_right(key, 8), th.accent_bold()), (what, th.text())]);
        y += 1;
    }
    // Always drawn (the rows above stop short of it): the way out must stay visible.
    let by = bottom;
    if by > inner.y {
        let end = hud::button(buf, x, by, "⏎", "Start", th, true);
        hits.push((Rect::new(x, by, end - x, 1), Hit::WelcomeDone));
        let hint = "esc keep defaults";
        if end + 2 + util::width(hint) as u16 <= x + iw {
            hud::put_right(buf, x + iw, by, hint, th.dim());
        }
    }
}

/// Theme selector: each row is drawn in its theme's own colors (name, accents, status
/// colors and a line sample); the whole UI previews the theme under the cursor.
fn themes(buf: &mut Buffer, area: Rect, app: &App, p: &crate::app::ThemePicker, hits: &mut Vec<(Rect, Hit)>) {
    use crate::theme::THEMES;
    let th = &app.theme;
    let w = 56.min(area.width.saturating_sub(2));
    let h = (THEMES.len() as u16 + 4).min(area.height.saturating_sub(2));
    let rect = centered(area, w, h);
    hits.push((rect, Hit::Inert));
    let inner = boxed(buf, rect, "THEME", th, th.accent);
    if inner.height < 3 || inner.width < 16 {
        return;
    }
    let x = inner.x + 1;
    let iw = inner.width.saturating_sub(2);
    let list_h = inner.height.saturating_sub(2) as usize;
    let offset = (p.selected + 1).saturating_sub(list_h);
    for (row, (i, t)) in THEMES.iter().enumerate().skip(offset).take(list_h).enumerate() {
        let y = inner.y + row as u16;
        let line = Rect::new(x, y, iw, 1);
        let bg = Style::default().bg(t.bg);
        hud::fill(buf, line, bg.fg(t.fg));
        let selected = i == p.selected;
        let marker = if selected { "▌" } else { " " };
        let mut cx = hud::put(buf, x, y, marker, bg.fg(th.accent), 1);
        let mut st = bg.fg(t.fg);
        if selected {
            st = st.add_modifier(Modifier::BOLD);
        }
        let name_w = 18.min(iw.saturating_sub(4));
        cx = hud::put(buf, cx, y, &util::pad_right(t.label, name_w as usize), st, name_w);
        // Accents and status colors, then the frame line and dim text as they look in the theme.
        if cx + 24 <= line.right() {
            for c in [t.accent, t.accent2, t.ok, t.warn, t.crit] {
                cx = hud::put(buf, cx, y, "██", bg.fg(c), 2);
                cx = hud::put(buf, cx, y, " ", bg, 1);
            }
            cx = hud::put(buf, cx, y, "── ", bg.fg(t.line), 3);
            hud::put(buf, cx, y, "dim", bg.fg(t.dim), 3);
        }
        if t.name == p.original {
            hud::put(buf, line.right() - 2, y, "✓", bg.fg(t.ok), 1);
        }
        hits.push((line, Hit::ThemeOption(i)));
    }
    let by = inner.bottom() - 1;
    let count = format!("{}/{}", p.selected + 1, THEMES.len());
    let counter_w = if THEMES.len() > list_h { util::width(&count) as u16 + 2 } else { 0 };
    if counter_w > 0 {
        hud::put(buf, x, by, &count, th.dim(), iw);
    }
    let hint = ["↑↓ preview · ⏎ apply · esc cancel", "⏎ apply · esc cancel", "⏎ apply"]
        .into_iter()
        .find(|h| util::width(h) as u16 + counter_w <= iw);
    if let Some(hint) = hint {
        hud::put_right(buf, x + iw, by, hint, th.dim());
    }
}

/// Terminal color scheme selector: each row is drawn on the scheme's own background
/// with its name and 16 ANSI colors; open terminals preview as you navigate.
fn schemes(buf: &mut Buffer, area: Rect, app: &App, p: &crate::app::SchemePicker, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    let options = app.scheme_options();
    let w = 78.min(area.width.saturating_sub(2));
    let h = (options.len() as u16 + 5).min(area.height.saturating_sub(2));
    let rect = centered(area, w, h);
    hits.push((rect, Hit::Inert));
    let inner = boxed(buf, rect, "TERMINAL COLORS", th, th.accent);
    if inner.height < 3 || inner.width < 16 {
        return;
    }
    let x = inner.x + 1;
    let iw = inner.width.saturating_sub(2);
    let list_h = inner.height.saturating_sub(2) as usize;
    let offset = (p.selected + 1).saturating_sub(list_h);
    for (row, (i, name)) in options.iter().enumerate().skip(offset).take(list_h).enumerate() {
        let y = inner.y + row as u16;
        let (label, bg, fg, colors) = super::settings::scheme_look(app, name);
        let line = Rect::new(x, y, iw, 1);
        hud::fill(buf, line, Style::default().bg(bg).fg(fg));
        let selected = i == p.selected;
        let saved = name.eq_ignore_ascii_case(p.original.trim())
            || crate::theme::find_scheme(&app.term_schemes, &p.original).is_some_and(|s| &s.name == name);
        let marker = if selected { "▌" } else { " " };
        let mut cx = hud::put(buf, x, y, marker, Style::default().fg(th.accent).bg(bg), 1);
        let mut st = Style::default().fg(fg).bg(bg);
        if selected {
            st = st.add_modifier(Modifier::BOLD);
        }
        let name_w = 30.min(iw.saturating_sub(4));
        cx = hud::put(
            buf,
            cx,
            y,
            &util::pad_right(&util::truncate(&label, name_w as usize - 1), name_w as usize),
            st,
            name_w,
        );
        if colors.len() == 16 && cx + 18 <= line.right() {
            for (n, c) in colors.iter().enumerate() {
                if n == 8 {
                    cx = hud::put(buf, cx, y, " ", Style::default().bg(bg), 1);
                }
                cx = hud::put(buf, cx, y, "█", Style::default().fg(*c).bg(bg), 1);
            }
            if cx + 10 <= line.right() {
                hud::put(buf, cx + 2, y, "PS C:\\>", Style::default().fg(fg).bg(bg), 8);
            }
        } else if colors.len() != 16 {
            hud::put(
                buf,
                cx,
                y,
                "uses the NOBLE theme",
                Style::default().fg(th.dim).bg(bg),
                line.right().saturating_sub(cx),
            );
        }
        if saved {
            hud::put(buf, line.right() - 2, y, "✓", Style::default().fg(th.ok).bg(bg), 1);
        }
        hits.push((line, Hit::TermScheme(i)));
    }
    // Bottom row: counter left, hint right; shortened in narrow boxes, skipped when it does not fit.
    let by = inner.bottom() - 1;
    let count = format!("{}/{}", p.selected + 1, options.len());
    let counter_w = if options.len() > list_h { util::width(&count) as u16 + 2 } else { 0 };
    if counter_w > 0 {
        hud::put(buf, x, by, &count, th.dim(), iw);
    }
    let hint = ["↑↓ preview · ⏎ apply · esc cancel", "⏎ apply · esc cancel", "⏎ apply"]
        .into_iter()
        .find(|h| util::width(h) as u16 + counter_w <= iw);
    if let Some(hint) = hint {
        hud::put_right(buf, x + iw, by, hint, th.dim());
    }
}

/// Quick launch popup: for each installed launcher its Home visibility and shortcut
/// key. Clicking a row toggles it, clicking the key chip changes it.
fn launchers(buf: &mut Buffer, area: Rect, app: &App, selected: usize, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    let list = app.installed_launchers();
    let w = 56.min(area.width.saturating_sub(2));
    let h = (list.len() as u16 + 5).min(area.height.saturating_sub(2));
    let rect = centered(area, w, h);
    hits.push((rect, Hit::Inert));
    let inner = boxed(buf, rect, "QUICK LAUNCH", th, th.accent);
    if inner.height < 3 || inner.width < 20 {
        return;
    }
    let x = inner.x + 1;
    let iw = inner.width.saturating_sub(2);
    let list_h = inner.height.saturating_sub(2) as usize;
    let selected = selected.min(list.len().saturating_sub(1));
    let offset = (selected + 1).saturating_sub(list_h);
    for (row, (n, &i)) in list.iter().enumerate().skip(offset).take(list_h).enumerate() {
        let Some((l, _)) = app.launchers.get(i) else { continue };
        let y = inner.y + row as u16;
        let line = Rect::new(inner.x, y, inner.width, 1);
        let is_sel = n == selected;
        let bg = if is_sel { th.sel_bg } else { th.raised };
        hud::fill(buf, line, Style::default().bg(bg));
        if is_sel {
            hud::put(buf, inner.x, y, "▌", Style::default().fg(th.accent).bg(bg), 1);
        }
        let (mark, mark_st) = if l.show {
            ("●", Style::default().fg(th.accent).bg(bg))
        } else {
            ("○", Style::default().fg(th.dim).bg(bg))
        };
        let mut cx = hud::put(buf, x, y, mark, mark_st, 1) + 1;
        let name_st = if l.show { th.text().bg(bg) } else { th.dim().bg(bg) };
        let name_w = 18.min(iw.saturating_sub(12));
        let name = util::truncate(&super::bridge::capitalize(&l.name), name_w as usize);
        hud::put(buf, cx, y, &name, name_st, name_w);
        cx += name_w;
        // The key chip on the right; the command name in between (when there is room).
        let chip = format!(" ‹ {} › ", l.key);
        let chip_w = util::width(&chip) as u16;
        let chip_x = (x + iw).saturating_sub(chip_w);
        let cmd_w = chip_x.saturating_sub(cx + 2);
        if cmd_w >= 6 {
            hud::put(buf, cx + 1, y, &util::truncate(&l.command, cmd_w as usize), th.dim().bg(bg), cmd_w);
        }
        hud::put(buf, chip_x, y, &chip, Style::default().fg(th.accent2).bg(bg), chip_w);
        hits.push((line, Hit::LaunchShow(i)));
        hits.push((Rect::new(chip_x, y, chip_w, 1), Hit::LaunchKey(i)));
    }
    let by = inner.bottom() - 1;
    let hint = ["space show/hide · ←→ shortcut · esc close", "space show · ←→ key · esc", "esc close"]
        .into_iter()
        .find(|h| util::width(h) as u16 <= iw);
    if let Some(hint) = hint {
        hud::put_right(buf, x + iw, by, hint, th.dim());
    }
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 3, w, h)
}

/// A filled box with an accent-colored frame.
fn boxed(buf: &mut Buffer, rect: Rect, title: &str, th: &Theme, accent: ratatui::style::Color) -> Rect {
    hud::fill(buf, rect, Style::default().bg(th.raised).fg(th.fg));
    let inner = hud::frame(buf, rect, title, "", true, th);
    // Paint the frame with the box accent.
    for x in rect.left()..rect.right() {
        for y in [rect.top(), rect.bottom().saturating_sub(1)] {
            if let Some(c) = buf.cell_mut((x, y)) {
                if matches!(c.symbol(), "─" | "╭" | "╮" | "╰" | "╯") {
                    c.set_fg(accent);
                }
                c.set_bg(th.raised);
            }
        }
    }
    for y in rect.top()..rect.bottom() {
        for x in [rect.left(), rect.right().saturating_sub(1)] {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_fg(accent);
                c.set_bg(th.raised);
            }
        }
    }
    inner
}

fn palette(buf: &mut Buffer, area: Rect, app: &App, st: &crate::app::PaletteState, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    let w = 76.min(area.width.saturating_sub(2));
    let h = 24.min(area.height.saturating_sub(2));
    if w < 20 || h < 6 {
        return;
    }
    let rect = centered(area, w, h);
    hits.push((rect, Hit::Inert));
    let inner = boxed(buf, rect, "COMMAND", th, th.accent);
    let x = inner.x + 1;
    let iw = inner.width.saturating_sub(2);
    let ms = app.started.elapsed().as_millis();
    let cursor = if (ms / 500).is_multiple_of(2) { "▏" } else { " " };
    let placeholder = st.query.is_empty();
    hud::put_spans(
        buf,
        x,
        inner.y,
        &[
            ("❯ ", th.accent_bold()),
            (if placeholder { "" } else { &st.query }, th.accent_bold()),
            (cursor, th.accent()),
            (if placeholder { "type a command, project, tab or theme" } else { "" }, th.dim()),
        ],
        iw,
    );
    hud::put_right(buf, x + iw, inner.y, &format!("{}/{}", st.matches.len(), st.all.len()), th.dim());
    hud::hline(buf, inner.x, inner.y + 1, inner.width, "─", th.line());
    let list_y = inner.y + 2;
    // A rule above the key hints when there is room for it.
    let footer_rule = inner.height >= 8;
    let rows = inner.bottom().saturating_sub(list_y + 1 + u16::from(footer_rule)) as usize;
    let offset = if st.selected >= rows { st.selected + 1 - rows } else { 0 };
    for (row, idx) in st.matches.iter().enumerate().skip(offset).take(rows) {
        let item = &st.all[*idx];
        let y = list_y + (row - offset) as u16;
        let selected = row == st.selected;
        let bgc = if selected { th.sel_bg } else { th.raised };
        hud::set_bg_row(buf, inner.x, y, inner.width, bgc);
        // Same selection marker as every other list.
        if selected {
            hud::put(buf, inner.x, y, "▌", th.accent().bg(bgc), 1);
        }
        let cx = x + 2;
        let group_w = 6u16;
        let cx = hud::put(
            buf,
            cx,
            y,
            &util::pad_right(item.group, group_w as usize),
            Style::default().fg(th.accent2).bg(bgc),
            group_w,
        );
        let hint_w = util::width(&item.hint) as u16;
        let title_w = iw.saturating_sub(2 + group_w + hint_w + 2);
        let title_style = if selected { th.accent_bold().bg(bgc) } else { th.text().bg(bgc) };
        hud::put(buf, cx, y, &util::truncate(&item.title, title_w as usize), title_style, title_w);
        if !item.hint.is_empty() {
            hud::put_right(buf, x + iw, y, &item.hint, th.dim().bg(bgc));
        }
        hits.push((Rect::new(inner.x, y, inner.width, 1), Hit::PaletteItem(row)));
    }
    if st.matches.is_empty() {
        hud::put(buf, x, list_y, "no matches", th.dim(), iw);
    }
    let fy = inner.bottom() - 1;
    if footer_rule {
        hud::hline(buf, inner.x, fy - 1, inner.width, "─", th.line());
    }
    // Keys in the accent color, their meaning dimmed.
    let (k, d) = (th.accent2(), th.dim());
    hud::put_spans(buf, x, fy, &[("↑↓", k), (" select  ", d), ("⏎", k), (" run  ", d), ("esc", k), (" close", d)], iw);
}

fn help_lines(app: &App) -> Vec<(String, String, bool)> {
    let mut v: Vec<(String, String, bool)> = Vec::new();
    let section = |v: &mut Vec<(String, String, bool)>, s: &str| {
        if !v.is_empty() {
            v.push((String::new(), String::new(), false));
        }
        v.push((s.to_string(), String::new(), true));
    };
    let prefix = app.keymap.prefix.to_string();
    section(&mut v, &format!("PREFIX · press {prefix}, then"));
    let mut by_action: Vec<(Action, Vec<String>)> = Vec::new();
    for a in Action::ALL {
        let mut keys: Vec<String> =
            app.keymap.prefix_map.iter().filter(|(_, x)| **x == a).map(|(k, _)| k.to_string()).collect();
        if keys.is_empty() {
            continue;
        }
        keys.sort_by_key(|k| (k.len(), k.clone()));
        by_action.push((a, keys));
    }
    // Collect the tab numbers on a single line.
    let mut tabs_done = false;
    for (a, keys) in &by_action {
        if matches!(a, Action::GoTab(_)) {
            if !tabs_done {
                v.push(("1 … 9".into(), "go to tab".into(), false));
                tabs_done = true;
            }
            continue;
        }
        v.push((keys.join("  "), a.title(), false));
    }
    v.push((prefix.clone(), "send prefix to the shell".into(), false));

    section(&mut v, "GLOBAL");
    let mut direct: Vec<(String, Action)> = app.keymap.direct_map.iter().map(|(k, a)| (k.to_string(), *a)).collect();
    direct.sort_by_key(|(k, _)| k.clone());
    let mut tabs_done = false;
    for (k, a) in direct {
        if matches!(a, Action::GoTab(_)) {
            if !tabs_done {
                v.push(("alt+1 … 9".into(), "go to tab".into(), false));
                tabs_done = true;
            }
            continue;
        }
        // Shell keys reach the shell in a terminal (`keys.shell_first`).
        let title = if app.keymap.shell_first.iter().any(|c| c.to_string() == k) {
            format!("{} (not in terminals)", a.title())
        } else {
            a.title()
        };
        v.push((k, title, false));
    }

    section(&mut v, "HOME");
    for (k, d) in [
        ("↑ ↓  j k", "select project"),
        ("⏎  double-click", "open a terminal in the project"),
        ("→  ⏎", "pin to top ★ · more actions ⋯ (← → choose)"),
        ("/", "search projects"),
        ("● 3  ✓  ↑ ↓  …", "uncommitted changes · clean · commits to push / pull · checking"),
        ("t", "terminal in home folder"),
        ("o", "open folder in file manager"),
        ("w", "save open tabs as a workspace"),
        ("a  A", "add a folder to scan for projects · add one project folder"),
        ("⋯  remove", "Remove from list: hide a project for good (A adds it back)"),
        ("r  R", "rescan projects · refresh AI usage"),
        ("m  s", "system · settings"),
        ("p  ?  q", "commands · help · quit"),
    ] {
        v.push((k.into(), d.into(), false));
    }
    for (_, l) in app.quick_launchers() {
        v.push((l.key.clone(), format!("run {} in the project", l.command), false));
    }

    section(&mut v, "SYSTEM");
    for (k, d) in [
        ("↑ ↓  pgup pgdn", "select process"),
        ("c m p n", "sort by cpu · mem · pid · name"),
        ("/", "filter"),
        ("K  del", "terminate (asks first)"),
        ("esc", "back"),
    ] {
        v.push((k.into(), d.into(), false));
    }

    section(&mut v, "MOUSE");
    for (k, d) in [
        ("click", "everything: tabs, rows, buttons, settings"),
        ("◫ ⊟ ⤢ ✕", "pane buttons: split right · split down · zoom · close (focused or hovered pane)"),
        ("double-click title", "zoom / restore the pane"),
        ("middle-click tab", "close the tab"),
        ("drag border", "resize panes"),
        ("drag text", "select (copied on release)"),
        ("right-click", "paste into terminal"),
        ("ctrl+click", "open a link or file:line (underlined while ctrl is held)"),
        ("wheel", "scroll history / lists"),
        ("shift+drag", "select even when the app captures the mouse"),
    ] {
        v.push((k.into(), d.into(), false));
    }
    v.push((String::new(), String::new(), false));
    v.push(("config".into(), util::tilde(&app.paths.config), false));
    v
}

fn help(buf: &mut Buffer, area: Rect, app: &App, scroll: u16, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    let w = 78.min(area.width.saturating_sub(2));
    let h = area.height.saturating_sub(2).min(40);
    if w < 30 || h < 6 {
        return;
    }
    let rect = centered(area, w, h);
    hits.push((rect, Hit::Inert));
    let inner = boxed(buf, rect, "KEYBOARD REFERENCE", th, th.accent);
    let lines = help_lines(app);
    let rows = inner.height.saturating_sub(1) as usize;
    let max_scroll = lines.len().saturating_sub(rows);
    let scroll = (scroll as usize).min(max_scroll);
    let key_w = 18u16;
    let x = inner.x + 1;
    for (i, (k, d, header)) in lines.iter().skip(scroll).take(rows).enumerate() {
        let y = inner.y + i as u16;
        if *header {
            let end = hud::put(buf, x, y, k, th.accent_bold().bg(th.raised), inner.width - 2);
            // A thin rule after the section name.
            let rule_w = (inner.right().saturating_sub(1)).saturating_sub(end + 1);
            hud::hline(buf, end + 1, y, rule_w, "─", th.line().bg(th.raised));
            continue;
        }
        hud::put(
            buf,
            x + 1,
            y,
            &util::pad_right(k, key_w as usize),
            Style::default().fg(th.accent2).bg(th.raised),
            key_w,
        );
        hud::put(buf, x + 1 + key_w, y, d, th.text().bg(th.raised), inner.width.saturating_sub(key_w + 3));
    }
    let footer = if max_scroll > 0 {
        format!("↑↓ scroll {}/{} · esc close", scroll, max_scroll)
    } else {
        "esc close".to_string()
    };
    hud::put_right(buf, inner.right() - 1, inner.bottom() - 1, &footer, th.dim().bg(th.raised));
}
