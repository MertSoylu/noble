//! Overlays: command palette, key reference, confirmation and text input.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::hud;
use crate::app::bindings::{self, BindingsState, Slot};
use crate::app::help::{GAP, KEY_GAP, Line, geometry};
use crate::app::{App, HelpState, Hit, Overlay};
use crate::keys::Action;
use crate::theme::Theme;
use crate::util;

pub fn draw(buf: &mut Buffer, area: Rect, app: &App, hits: &mut Vec<(Rect, Hit)>) {
    let Some(ov) = &app.overlay else { return };
    let th = &app.theme;
    // The right-click menu does not dim the screen; clicking outside still closes it.
    if !matches!(ov, Overlay::Menu(_)) {
        // The backdrop darkens in when the overlay opens.
        let fade = app.fx.progress(crate::app::fx::Fx::Backdrop).unwrap_or(1.0);
        dim_backdrop(buf, area, th, fade);
    }
    hits.push((area, Hit::Backdrop));
    match ov {
        Overlay::Welcome(ws) => welcome(buf, area, app, ws, hits),
        Overlay::Palette(st) => palette(buf, area, app, st, hits),
        Overlay::Schemes(p) => schemes(buf, area, app, p, hits),
        Overlay::Themes(p) => themes(buf, area, app, p, hits),
        Overlay::Launchers { selected } => launchers(buf, area, app, *selected, hits),
        Overlay::AgentHooks { selected } => agent_hooks(buf, area, app, *selected, hits),
        Overlay::Menu(m) => menu(buf, area, app, m, hits),
        Overlay::Help(st) => help(buf, area, app, st, hits),
        Overlay::Bindings(st) => key_bindings(buf, area, app, st, hits),
        Overlay::Confirm(c) => {
            let w = 52.min(area.width.saturating_sub(2));
            // The question wraps over as many rows as it needs: the box grows with it (within the
            // area) and the Yes/No row stays at the bottom.
            let text_w = (w as usize).saturating_sub(4);
            let lines = wrap_text(&c.body, text_w);
            let rows = u16::try_from(lines.len()).unwrap_or(u16::MAX);
            let rect = centered(area, w, rows.saturating_add(6).min(area.height));
            hits.push((rect, Hit::Inert));
            let inner = boxed(buf, rect, &c.title, th, th.crit);
            let x = inner.x + 1;
            // Rows: a top padding row, the text, two blank rows, then the buttons. A short box drops
            // the blank rows first: the text takes every row from the padding down to the buttons.
            let room = inner.height.saturating_sub(2) as usize;
            let cut = lines.len() > room;
            for (i, line) in lines.iter().take(room).enumerate() {
                let row = inner.y + 1 + i as u16;
                let shown = if cut && i + 1 == room { ellipsize(line, text_w) } else { line.clone() };
                hud::put(buf, x, row, &shown, th.text(), inner.width.saturating_sub(2));
            }
            let y = inner.bottom().saturating_sub(1);
            // Chips stay inside the frame: they stop at the content's right edge (one column in).
            let edge = inner.right().saturating_sub(1);
            // The button words drop out when the box is too narrow for them (the keys stay).
            let (yes_word, no_word) =
                if inner.width.saturating_sub(2) >= 28 { (" confirm", " cancel") } else { ("", "") };
            let yes_x = x;
            let yes_spans = [
                (" y ", Style::default().fg(th.on_accent).bg(th.crit).add_modifier(Modifier::BOLD)),
                (yes_word, th.text()),
            ];
            let end = hud::put_spans(buf, yes_x, y, &yes_spans, edge.saturating_sub(yes_x));
            if end > yes_x {
                hits.push((Rect::new(yes_x, y, end - yes_x, 1), Hit::ConfirmYes));
            }
            // The n/esc chip is drawn whole or not at all: a cut key chip would read as a different key.
            let no_x = end.saturating_add(3);
            let no_spans =
                [(" n/esc ", Style::default().fg(th.fg).bg(th.line).add_modifier(Modifier::BOLD)), (no_word, th.dim())];
            let no_w: u16 = no_spans.iter().map(|(s, _)| util::width(s) as u16).sum();
            if no_x.saturating_add(no_w) <= edge {
                let end = hud::put_spans(buf, no_x, y, &no_spans, no_w);
                hits.push((Rect::new(no_x, y, end - no_x, 1), Hit::ConfirmNo));
            }
        }
        Overlay::Prompt(p) => {
            let w = 56.min(area.width.saturating_sub(2));
            let rect = centered(area, w, 5);
            hits.push((rect, Hit::Inert));
            let inner = boxed(buf, rect, &p.title, th, th.accent);
            let x = inner.x + 1;
            let y = inner.y + 1;
            let cursor = if app.cursor_lit() { "▏" } else { " " };
            let split = p.value.char_indices().nth(p.cursor).map_or(p.value.len(), |(i, _)| i);
            let (before, after) = p.value.split_at(split);
            let field_w = inner.width.saturating_sub(2);
            if p.value.is_empty() {
                // Placeholder while empty.
                let hint = match p.purpose {
                    crate::app::PromptPurpose::Add(_) => "path, e.g. ~/code/app",
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
            let add = p.purpose.add_mode();
            if let Some(current) = add {
                // Choice line above the input: auto, a folder to scan or one project (tab / click).
                let mut cx = x;
                let end = inner.right().saturating_sub(1);
                for (mode, label) in [
                    (crate::app::AddMode::Auto, "auto"),
                    (crate::app::AddMode::Folder, "folder to scan"),
                    (crate::app::AddMode::Project, "one project"),
                ] {
                    let on = mode == current;
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
            let hints: &[&str] = if add.is_some() {
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

/// Dims everything under an overlay; `fade` (0..1) is how far the dimming has come.
fn dim_backdrop(buf: &mut Buffer, area: Rect, th: &Theme, fade: f64) {
    let area = area.intersection(buf.area);
    let mix = crate::theme::Theme::mix;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(c) = buf.cell_mut((x, y)) {
                let (fg, bg) = (c.fg, c.bg);
                c.set_fg(if fade < 1.0 { mix(fg, th.line, fade) } else { th.line });
                if th.bg != ratatui::style::Color::Reset {
                    c.set_bg(if fade < 1.0 { mix(bg, th.bg, fade) } else { th.bg });
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
    let glide = app.fx.glide("menu", m.selected);
    for (i, it) in m.items.iter().enumerate().take(inner.height as usize) {
        let ry = inner.y + i as u16;
        let row = Rect::new(inner.x, ry, inner.width, 1);
        let selected = i == m.selected;
        let bg = glide.bg(i, th.raised, th.sel_bg);
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
            "no git projects found yet — press a on Home to add one".to_string()
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
    if let Some(k) = app.keymap.hint(Action::Palette) {
        keys.push((k, "command palette — everything is in there".into()));
    }
    keys.push(("?".into(), "every shortcut · ctrl+click opens links".into()));
    // The key column fits the longest key (a rebound palette key can be "ctrl+space :") and the launcher keys,
    // but takes at most half the card so the text beside it keeps room; a longer key is cut.
    let launcher_keys = launchers.iter().map(|l| l.key.as_str()).collect::<Vec<_>>().join("  ");
    let key_w = keys
        .iter()
        .map(|(k, _)| util::width(k) + 2)
        .chain((!launchers.is_empty()).then(|| util::width(&launcher_keys) + 2))
        .max()
        .unwrap_or(8)
        .clamp(8, ((iw / 2) as usize).max(8));
    if !launchers.is_empty() {
        let names: Vec<String> = launchers.iter().map(|l| super::bridge::capitalize(&l.name)).collect();
        let what = launcher_what(&names, (iw as usize).saturating_sub(key_w));
        keys.insert(1, (launcher_keys, what));
    }
    for (key, what) in &keys {
        line(buf, y, &[(&util::pad_right(key, key_w), th.accent_bold()), (what, th.text())]);
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

/// The welcome card's launcher line, `start Claude / Codex +2 right there`: names are dropped from the end
/// as whole names until the line fits `avail` columns, and the count of those left out follows as `+N`
/// (the same mark the project card uses for ports). A name is never cut; only when not even `+N` fits is
/// the line cut, with "…".
fn launcher_what(names: &[String], avail: usize) -> String {
    let line = |shown: usize| {
        let mut list = names[..shown].join(" / ");
        if shown < names.len() {
            if shown > 0 {
                list.push(' ');
            }
            list.push_str(&format!("+{}", names.len() - shown));
        }
        format!("start {list} right there")
    };
    (0..=names.len())
        .rev()
        .map(&line)
        .find(|text| util::width(text) <= avail)
        .unwrap_or_else(|| util::truncate(&line(0), avail))
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
                // A prompt sample that looks like this OS: PowerShell on Windows, a Unix shell on Linux/macOS.
                let sample = if cfg!(windows) { "PS C:\\>" } else { "~ $" };
                hud::put(buf, cx + 2, y, sample, Style::default().fg(fg).bg(bg), 8);
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
    let glide = app.fx.glide("launchers", selected);
    for (row, (n, &i)) in list.iter().enumerate().skip(offset).take(list_h).enumerate() {
        let Some((l, _)) = app.launchers.get(i) else { continue };
        let y = inner.y + row as u16;
        let line = Rect::new(inner.x, y, inner.width, 1);
        let is_sel = n == selected;
        let bg = glide.bg(n, th.raised, th.sel_bg);
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

/// Agent status hooks popup: one row per agent (`Agent::ALL`) with its hooks on, off, or not found on this
/// machine. Clicking a row, or ⏎ / space on it, turns that agent's hooks on or off.
fn agent_hooks(buf: &mut Buffer, area: Rect, app: &App, selected: usize, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    let agents = crate::hooks::Agent::ALL;
    let w = 46.min(area.width.saturating_sub(2));
    let h = (agents.len() as u16 + 5).min(area.height.saturating_sub(2));
    let rect = centered(area, w, h);
    hits.push((rect, Hit::Inert));
    let inner = boxed(buf, rect, "AGENT STATUS HOOKS", th, th.accent);
    if inner.height < 3 || inner.width < 20 {
        return;
    }
    let x = inner.x + 1;
    let iw = inner.width.saturating_sub(2);
    let list_h = inner.height.saturating_sub(2) as usize;
    let selected = selected.min(agents.len() - 1);
    let offset = (selected + 1).saturating_sub(list_h);
    let glide = app.fx.glide("agent-hooks", selected);
    for (row, (n, &agent)) in agents.iter().enumerate().skip(offset).take(list_h).enumerate() {
        let y = inner.y + row as u16;
        let line = Rect::new(inner.x, y, inner.width, 1);
        let bg = glide.bg(n, th.raised, th.sel_bg);
        hud::fill(buf, line, Style::default().bg(bg));
        if n == selected {
            hud::put(buf, inner.x, y, "▌", Style::default().fg(th.accent).bg(bg), 1);
        }
        let on = app.hooks_on(agent);
        let found = app.agents_found.contains(&agent);
        let (mark, mark_st) = if on {
            ("●", Style::default().fg(th.accent).bg(bg))
        } else {
            ("○", Style::default().fg(th.dim).bg(bg))
        };
        let cx = hud::put(buf, x, y, mark, mark_st, 1) + 1;
        // The state word sits at the right edge; the name fills what is left.
        let (state, state_st) = if on {
            ("on", Style::default().fg(th.accent).bg(bg))
        } else if found {
            ("off", th.dim().bg(bg))
        } else {
            ("not found", th.dim().bg(bg))
        };
        let state_text = format!(" {state}");
        let state_w = util::width(&state_text) as u16;
        let state_x = (x + iw).saturating_sub(state_w);
        let name_w = 18.min(state_x.saturating_sub(cx + 1));
        let name_st = if found { th.text().bg(bg) } else { th.dim().bg(bg) };
        hud::put(buf, cx, y, &util::truncate(agent.name(), name_w as usize), name_st, name_w);
        hud::put(buf, state_x, y, &state_text, state_st, state_w);
        hits.push((line, Hit::AgentHook(n)));
    }
    let by = inner.bottom() - 1;
    let hint = ["space on/off · esc close", "space on/off · esc", "esc close"]
        .into_iter()
        .find(|h| util::width(h) as u16 <= iw);
    if let Some(hint) = hint {
        hud::put_right(buf, x + iw, by, hint, th.dim());
    }
}

/// Word-wraps `text` to lines of at most `width` columns (by display width); a word wider than
/// a line is cut into pieces. Newlines count as spaces.
pub(super) fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    if width == 0 {
        return lines;
    }
    let mut line = String::new();
    for word in text.split_whitespace() {
        if line.is_empty() && util::width(word) <= width {
            line.push_str(word);
            continue;
        }
        if !line.is_empty() && util::width(&line) + 1 + util::width(word) <= width {
            line.push(' ');
            line.push_str(word);
            continue;
        }
        if !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        for c in word.chars() {
            if !line.is_empty() && util::width(&line) + util::width(c.encode_utf8(&mut [0; 4])) > width {
                lines.push(std::mem::take(&mut line));
            }
            line.push(c);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// `line` cut to at most `width` columns and ending with "…" (the text was cut after it).
fn ellipsize(line: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut out = line.to_string();
    while !out.is_empty() && util::width(&out) + 1 > width {
        out.pop();
    }
    format!("{}…", out.trim_end())
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 3, w, h)
}

/// Clears the frame lines of the one-cell ring around an overlay box. Without it a box placed next
/// to the frame of the panel underneath (the Home panel fills the screen) shares its border column
/// or row, and the two frames read as doubled corners and edges. Only box-drawing glyphs
/// (U+2500..=U+257F) are cleared: text, tab chips and block glyphs in the ring stay, so panel titles
/// and tabs behind a menu survive. The ring is clipped to the buffer and never touches the top bar
/// (row `buf.area.y`).
fn halo(buf: &mut Buffer, rect: Rect, th: &Theme) {
    let area = buf.area;
    let x0 = rect.x.saturating_sub(1).max(area.x);
    let x1 = rect.right().saturating_add(1).min(area.right());
    let y0 = rect.y.saturating_sub(1).max(area.y.saturating_add(1));
    let y1 = rect.bottom().saturating_add(1).min(area.bottom());
    for y in y0..y1 {
        for x in x0..x1 {
            let inside = x >= rect.left() && x < rect.right() && y >= rect.top() && y < rect.bottom();
            if inside {
                continue;
            }
            if let Some(c) = buf.cell_mut((x, y)) {
                let is_frame = c.symbol().chars().next().is_some_and(|ch| ('\u{2500}'..='\u{257F}').contains(&ch));
                if !is_frame {
                    continue;
                }
                // Blank in the dimmed backdrop's colors.
                c.reset();
                c.set_fg(th.line);
                if th.bg != ratatui::style::Color::Reset {
                    c.set_bg(th.bg);
                }
            }
        }
    }
}

/// A filled box with an accent-colored frame.
fn boxed(buf: &mut Buffer, rect: Rect, title: &str, th: &Theme, accent: ratatui::style::Color) -> Rect {
    halo(buf, rect, th);
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

/// Splits the drawn title into runs of plain and matched letters. `shown` is `title` as drawn (cut
/// with "…" when it does not fit): marks past the cut are dropped and the "…" is never marked.
fn title_runs(title: &str, shown: &str, marks: &[usize]) -> Vec<(String, bool)> {
    let cut = shown != title;
    let visible = shown.chars().count().saturating_sub(usize::from(cut));
    let mut runs: Vec<(String, bool)> = Vec::new();
    for (i, ch) in shown.chars().enumerate() {
        let hit = i < visible && marks.contains(&i);
        match runs.last_mut() {
            Some((text, run_hit)) if *run_hit == hit => text.push(ch),
            _ => runs.push((ch.to_string(), hit)),
        }
    }
    runs
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
    let cursor = if app.cursor_lit() { "▏" } else { " " };
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
    let glide = app.fx.glide("palette", st.selected);
    for (row, idx) in st.matches.iter().enumerate().skip(offset).take(rows) {
        let item = &st.all[*idx];
        let y = list_y + (row - offset) as u16;
        let selected = row == st.selected;
        let bgc = glide.bg(row, th.raised, th.sel_bg);
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
        // The title as drawn (cut with "…" when it does not fit); the letters the query matched in bold accent.
        let shown = util::truncate(&item.title, title_w as usize);
        let base = if selected { th.text().add_modifier(Modifier::BOLD).bg(bgc) } else { th.text().bg(bgc) };
        let marks: &[usize] = st.positions.get(row).map_or(&[], |m| m.as_slice());
        let runs = title_runs(&item.title, &shown, marks);
        let spans: Vec<(&str, Style)> = runs
            .iter()
            .map(|(text, hit)| (text.as_str(), if *hit { th.accent_bold().bg(bgc) } else { base }))
            .collect();
        hud::put_spans(buf, cx, y, &spans, title_w);
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
    let mut keys = vec![("↑↓", k), (" select  ", d), ("⏎", k)];
    match st.current().map(|i| &i.cmd) {
        // A notification: Enter goes where it leads (or copies it, when nowhere), ctrl+y copies it in any case.
        Some(crate::app::PaletteCmd::Notice { target, .. }) => {
            use crate::app::ToastTarget;
            // A pane's notification whose pane has closed is only copied, like one without a target.
            let target = target.as_ref().filter(|t| app.toast_target_open(t));
            keys.push((
                match target {
                    Some(ToastTarget::Pane { .. }) => " go to  ",
                    Some(ToastTarget::Config { .. }) => " open  ",
                    None => " copy  ",
                },
                d,
            ));
            if target.is_some() {
                keys.extend([("ctrl+y", k), (" copy  ", d)]);
            }
        }
        _ => keys.push((" run  ", d)),
    }
    keys.extend([("esc", k), (" close", d)]);
    hud::put_spans(buf, x, fy, &keys, iw);
}

/// The keyboard reference: one or more columns of sections, a filter in the title and a selection.
fn help(buf: &mut Buffer, area: Rect, app: &App, st: &HelpState, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    let sections = app.help_sections(&st.query);
    let Some(g) = geometry(area, &sections) else {
        // Too small for the key list: a note instead of nothing (as the Settings screen does).
        hud::too_small(buf, area, th);
        return;
    };
    let rect = g.rect;
    hits.push((rect, Hit::Inert));
    let title = if st.query.is_empty() {
        "KEYBOARD REFERENCE".to_string()
    } else {
        format!("KEYBOARD REFERENCE · /{}", st.query)
    };
    let inner = boxed(buf, rect, &title, th, th.accent);
    if sections.is_empty() {
        hud::put(buf, inner.x + 1, inner.y, "no matches", th.dim(), inner.width.saturating_sub(2));
    }
    let scroll = st.scroll.min(g.max_scroll());
    let key_w = g.key_w;
    for (c, col) in g.columns.iter().enumerate() {
        let x = inner.x + 1 + c as u16 * (g.col_w + GAP);
        for (i, line) in col.iter().skip(scroll).take(g.rows).enumerate() {
            let y = inner.y + i as u16;
            match line {
                Line::Gap => {}
                Line::Header(name) => {
                    let end = hud::put(buf, x, y, name, th.accent_bold().bg(th.raised), g.col_w);
                    // A thin rule after the section name.
                    let rule_w = (x + g.col_w).saturating_sub(end + 1);
                    hud::hline(buf, end + 1, y, rule_w, "─", th.line().bg(th.raised));
                }
                Line::Row { key, desc, sel } => {
                    let selected = *sel == Some(st.selected);
                    let bg = if selected { th.sel_bg } else { th.raised };
                    if selected {
                        // Same selection marker as the palette and the other lists.
                        hud::set_bg_row(buf, x - 1, y, g.col_w + 1, bg);
                        hud::put(buf, x - 1, y, "▌", th.accent().bg(bg), 1);
                    }
                    if let Some(s) = sel {
                        hits.push((Rect::new(x, y, g.col_w, 1), Hit::HelpRow(*s)));
                    }
                    // The key is cut to the column less its gap, so the description never starts inside it.
                    let key_text = util::truncate(key, usize::from(key_w).saturating_sub(KEY_GAP));
                    hud::put(
                        buf,
                        x + 1,
                        y,
                        &util::pad_right(&key_text, usize::from(key_w)),
                        Style::default().fg(th.accent2).bg(bg),
                        key_w,
                    );
                    let desc_w = g.col_w.saturating_sub(1 + key_w);
                    let style = if selected { th.text().add_modifier(Modifier::BOLD).bg(bg) } else { th.text().bg(bg) };
                    hud::put(buf, x + 1 + key_w, y, &util::truncate(desc, desc_w as usize), style, desc_w);
                }
            }
        }
    }
    hud::put_right(
        buf,
        inner.right() - 1,
        inner.bottom() - 1,
        "type to filter · ⏎ run · esc close",
        th.dim().bg(th.raised),
    );
}

/// The key bindings editor: every action with its prefix and direct keys, a filter, the status line
/// (the key being awaited, the question about a taken key, a refusal) and the key hints.
fn key_bindings(buf: &mut Buffer, area: Rect, app: &App, st: &BindingsState, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    let Some(g) = bindings::geometry(area) else {
        hud::too_small(buf, area, th);
        return;
    };
    hits.push((g.rect, Hit::Inert));
    let inner = boxed(buf, g.rect, "KEY BINDINGS", th, th.accent);
    let x = inner.x + 1;
    let iw = g.width;
    let rows = app.binding_rows(&st.query);
    // The filter line ("/ text", the cursor while typing) and how many of the actions are listed.
    let count = format!("{}/{}", rows.len(), Action::ALL.len());
    let filter_on = st.filtering || !st.query.is_empty();
    let cursor = if st.filtering && app.cursor_lit() { "▏" } else { "" };
    let filter = if filter_on { format!("/ {}{cursor}", st.query) } else { "/ filter".into() };
    let counter_w = util::width(&count) as u16 + 1;
    let filter_style = if filter_on { th.accent_bold().bg(th.raised) } else { th.dim().bg(th.raised) };
    hud::put(buf, x, inner.y, &filter, filter_style, iw.saturating_sub(counter_w));
    hud::put_right(buf, x + iw, inner.y, &count, th.dim().bg(th.raised));
    // The column titles.
    let head = th.dim().bg(th.raised);
    let hy = inner.y + 1;
    if g.group_w > 0 {
        hud::put(buf, x, hy, "GROUP", head, g.group_w);
    }
    hud::put(buf, x + g.title_at, hy, "ACTION", head, g.title_w);
    hud::put(buf, x + g.prefix_at, hy, "PREFIX", head, g.key_w);
    hud::put(buf, x + g.direct_at, hy, "DIRECT", head, g.key_w);
    // The list, scrolled so that the selected row is on screen.
    let list_y = inner.y + 2;
    if rows.is_empty() {
        hud::put(buf, x, list_y, "no matches", th.dim().bg(th.raised), iw);
    }
    let offset = if st.selected >= g.rows { st.selected + 1 - g.rows } else { 0 };
    for (row, action) in rows.iter().enumerate().skip(offset).take(g.rows) {
        let y = list_y + (row - offset) as u16;
        let selected = row == st.selected;
        let bg = if selected { th.sel_bg } else { th.raised };
        hud::set_bg_row(buf, inner.x, y, inner.width, bg);
        if selected {
            hud::put(buf, inner.x, y, "▌", th.accent().bg(bg), 1);
        }
        if g.group_w > 0 {
            hud::put(buf, x, y, action.group(), th.accent2().bg(bg), g.group_w);
        }
        let title_style = if selected { th.text().add_modifier(Modifier::BOLD).bg(bg) } else { th.text().bg(bg) };
        hud::put(buf, x + g.title_at, y, &util::truncate(&action.title(), g.title_w as usize), title_style, g.title_w);
        for (at, slot) in [(g.prefix_at, Slot::Prefix), (g.direct_at, Slot::Direct)] {
            let label = app.binding_label(*action, slot);
            let style = if label == "—" { th.dim().bg(bg) } else { th.accent2().bg(bg) };
            hud::put(buf, x + at, y, &util::truncate(&label, g.key_w as usize), style, g.key_w);
        }
        hits.push((Rect::new(inner.x, y, inner.width, 1), Hit::BindingsRow(row)));
    }
    // The status line: why a key was refused, the question about a taken key, the change just saved, or
    // what is awaited.
    let warn = Style::default().fg(th.warn).bg(th.raised);
    let (status, style) = if let Some(note) = &st.note {
        // A refused key: the editor keeps waiting, the prompt returns with the next key.
        (note.clone(), warn)
    } else if let Some(r) = st.replace {
        (format!("{} is {} — replace? y/n", app.key_label(r.slot, r.chord), r.other.title()), warn)
    } else if let Some(done) = &st.done {
        (done.clone(), Style::default().fg(th.ok).bg(th.raised))
    } else if let Some((action, slot)) = st.capture {
        let text = match slot {
            Slot::Prefix => format!("press a key for {} after {} · esc cancel", action.title(), app.keymap.prefix),
            Slot::Direct => format!("press a key chord for {} · esc cancel", action.title()),
        };
        (text, th.accent_bold().bg(th.raised))
    } else {
        (String::new(), th.dim().bg(th.raised))
    };
    hud::put(buf, x, inner.bottom() - 2, &util::truncate(&status, iw as usize), style, iw);
    // The key hints: the longest of them that fits.
    const BROWSE: [&str; 4] = [
        "⏎ prefix key · d direct key · x unbind · r reset · / filter · esc close",
        "⏎ prefix key · d direct key · x unbind · r reset · esc",
        "⏎ prefix · d direct · x unbind · r reset · esc",
        "⏎ prefix · d direct · esc",
    ];
    const WAITING: [&str; 1] = ["esc cancel"];
    const REPLACE: [&str; 1] = ["y replace · n cancel"];
    const FILTER: [&str; 2] = ["type to filter · ⏎ done · esc clear", "⏎ done · esc clear"];
    let hints: &[&str] = if st.replace.is_some() {
        &REPLACE
    } else if st.capture.is_some() {
        &WAITING
    } else if st.filtering {
        &FILTER
    } else {
        &BROWSE
    };
    let hint = hints.iter().find(|h| util::width(h) <= iw as usize).copied().unwrap_or("esc");
    hud::put(buf, x, inner.bottom() - 1, hint, th.dim().bg(th.raised), iw);
}

#[cfg(test)]
mod tests {
    use super::wrap_text;
    use crate::util;

    #[test]
    fn wrap_text_breaks_at_spaces_and_cuts_long_words() {
        assert_eq!(wrap_text("close the tab", 20), ["close the tab"]);
        assert_eq!(wrap_text("close the tab", 9), ["close the", "tab"]);
        assert_eq!(wrap_text("a  b\nc", 80), ["a b c"]);
        // A word wider than the line is cut, every piece fits, and no letter is lost.
        let cut = wrap_text(&"ab".repeat(10), 7);
        assert_eq!(cut.concat(), "ab".repeat(10));
        assert!(cut.iter().all(|l| util::width(l) <= 7), "{cut:?}");
        // Wide characters count two columns.
        let wide = wrap_text("日本語 日本語", 5);
        assert!(wide.iter().all(|l| util::width(l) <= 5), "{wide:?}");
        assert!(wrap_text("", 10).is_empty());
        assert!(wrap_text("abc", 0).is_empty());
    }
}
