//! Üst katmanlar: komut paleti, klavye referansı, onay ve metin girişi.

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
    // Sağ tık menüsü ekranı karartmaz; dışına tıklamak yine de kapatır.
    if !matches!(ov, Overlay::Menu(_)) {
        dim_backdrop(buf, area, th);
    }
    hits.push((area, Hit::Backdrop));
    match ov {
        Overlay::Welcome { prefix } => welcome(buf, area, app, *prefix, hits),
        Overlay::Palette(st) => palette(buf, area, app, st, hits),
        Overlay::Schemes(p) => schemes(buf, area, app, p, hits),
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
                &util::truncate(&c.body, inner.width as usize - 2),
                th.text(),
                inner.width - 2,
            );
            let y = inner.bottom() - 1;
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
                &[(" n ", Style::default().fg(th.fg).bg(th.line).add_modifier(Modifier::BOLD)), (" cancel", th.dim())],
                20,
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
            hud::put_spans(
                buf,
                x,
                y,
                &[("❯ ", th.accent()), (&p.value, th.accent_bold()), (cursor, th.accent())],
                inner.width - 2,
            );
            hud::put_right(buf, inner.right() - 1, inner.bottom() - 1, "⏎ save · esc cancel", th.dim());
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

/// Sağ tık menüsü: tıklanan noktanın yanında, ekrana sığacak şekilde.
fn menu(buf: &mut Buffer, area: Rect, app: &App, m: &crate::app::Menu, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    let label_w = m.items.iter().map(|i| util::width(&i.label)).max().unwrap_or(4) as u16;
    let hint_w = m.items.iter().map(|i| util::width(&i.hint)).max().unwrap_or(0) as u16;
    let title_w = util::width(&m.title) as u16 + 6;
    let w = (label_w + hint_w + 8).max(title_w).clamp(16, 44).min(area.width);
    let h = (m.items.len() as u16 + 2).min(area.height);
    // Sağa/alta taşacaksa sola/yukarı kayar.
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

/// Prefix seçeneklerinin kısa açıklamaları (`PREFIXES` sırası).
const PREFIX_NOTES: [&str; 4] = [
    "tmux style · shell's ctrl+a (line start) takes 2 presses",
    "tmux default · shell's ctrl+b (char back) takes 2 presses",
    "PowerShell's ctrl+space menu takes 2 presses",
    "rarely used by shells · stays out of your way",
];

/// İlk açılış kartı: ne bulundu, en önemli tuşlar ve prefix seçimi.
fn welcome(buf: &mut Buffer, area: Rect, app: &App, sel: usize, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    let w = 66.min(area.width.saturating_sub(2));
    let h = 17.min(area.height.saturating_sub(1));
    let rect = centered(area, w, h);
    hits.push((rect, Hit::Inert));
    let inner = boxed(buf, rect, "WELCOME TO NOBLE", th, th.accent);
    if inner.height < 4 || inner.width < 20 {
        return;
    }
    let x = inner.x + 2;
    let iw = inner.width.saturating_sub(4);
    let mut y = inner.y + 1;
    let bottom = inner.bottom();
    let line = |buf: &mut Buffer, y: u16, spans: &[(&str, Style)]| {
        if y < bottom {
            hud::put_spans(buf, x, y, spans, iw);
        }
    };
    let found = if !app.projects_loaded {
        "looking for your git projects…".to_string()
    } else if app.projects.is_empty() {
        "no git projects found yet — press a on Home to add a folder".to_string()
    } else {
        format!("found {} git projects", app.projects.len())
    };
    line(buf, y, &[("● ", th.accent2()), (&found, th.text())]);
    y += 1;
    let colors = format!("terminal colors: {}", app.scheme_label(&app.cfg.terminal.colors));
    line(buf, y, &[("● ", th.accent2()), (&colors, th.text())]);
    y += 2;
    for (key, what) in [
        ("⏎", "open a terminal in the selected project"),
        ("c  x", "start Claude / Codex right there"),
        ("alt+p", "command palette — everything is in there"),
        ("?", "every shortcut · ctrl+click opens links"),
    ] {
        line(buf, y, &[(&util::pad_right(key, 8), th.accent_bold()), (what, th.text())]);
        y += 1;
    }
    y += 1;
    line(buf, y, &[("PREFIX KEY", th.accent_bold()), ("  splits, tabs and more start with it", th.dim())]);
    y += 1;
    if y < bottom {
        let mut cx = x;
        for (i, p) in crate::app::PREFIXES.iter().enumerate() {
            let label = format!(" {p} ");
            let style = if i == sel {
                Style::default().fg(th.on_accent).bg(th.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(th.fg).bg(th.sel_bg)
            };
            let start = cx;
            cx = hud::put(buf, cx, y, &label, style, (x + iw).saturating_sub(cx));
            hits.push((Rect::new(start, y, cx - start, 1), Hit::WelcomePrefix(i)));
            cx += 1;
        }
        y += 1;
    }
    line(buf, y, &[(PREFIX_NOTES.get(sel).copied().unwrap_or(""), th.dim())]);
    let by = inner.bottom() - 1;
    if by > y {
        let start = x;
        let end = hud::button(buf, x, by, "⏎", "Start", th, true);
        hits.push((Rect::new(start, by, end - start, 1), Hit::WelcomeDone));
        hud::put_right(buf, x + iw, by, "←→ prefix · esc skip", th.dim());
    }
}

/// Terminal renk şeması seçicisi: her satır şemanın kendi zemininde, adı ve
/// 16 ANSI rengiyle çizilir; gezinirken açık terminaller de önizlenir.
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
    // Alt satır: sayaç solda, ipucu sağda; dar kutuda kısalır, sığmazsa atlanır.
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

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 3, w, h)
}

/// Dolgulu, vurgu renkli çerçeveli kutu.
fn boxed(buf: &mut Buffer, rect: Rect, title: &str, th: &Theme, accent: ratatui::style::Color) -> Rect {
    hud::fill(buf, rect, Style::default().bg(th.raised).fg(th.fg));
    let inner = hud::frame(buf, rect, title, "", true, th);
    // Çerçeveyi kutu vurgusuyla boya.
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
    let rows = inner.bottom().saturating_sub(list_y + 1) as usize;
    let offset = if st.selected >= rows { st.selected + 1 - rows } else { 0 };
    for (row, idx) in st.matches.iter().enumerate().skip(offset).take(rows) {
        let item = &st.all[*idx];
        let y = list_y + (row - offset) as u16;
        let selected = row == st.selected;
        let bgc = if selected { th.sel_bg } else { th.raised };
        hud::set_bg_row(buf, inner.x, y, inner.width, bgc);
        let marker = if selected { "▶ " } else { "  " };
        let cx = hud::put(buf, x, y, marker, th.accent().bg(bgc), 2);
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
    hud::put(buf, x, inner.bottom() - 1, "↑↓ select · ⏎ run · esc close", th.line(), iw);
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
    // Sekme numaralarını tek satırda topla.
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
        v.push((k, a.title(), false));
    }

    section(&mut v, "HOME");
    for (k, d) in [
        ("↑ ↓  j k", "select project"),
        ("⏎  double-click", "open a terminal in the project"),
        ("/", "search projects"),
        ("● 3  ✓  ↑ ↓  …", "uncommitted changes · clean · commits to push / pull · checking"),
        ("t", "terminal in home folder"),
        ("o", "open folder in file manager"),
        ("w", "save open tabs as a workspace"),
        ("a", "add a folder to scan for projects"),
        ("r  R", "rescan projects · refresh AI usage"),
        ("m  s", "system · settings"),
        ("p  ?  q", "commands · help · quit"),
    ] {
        v.push((k.into(), d.into(), false));
    }
    for (l, ok) in &app.launchers {
        let note = if *ok { "" } else { "  (not installed)" };
        v.push((l.key.clone(), format!("run {} in the project{note}", l.command), false));
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
        ("┃ ━ ⤢ ✕", "pane buttons: split right · split down · zoom · close"),
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
            hud::put(buf, x, y, k, th.accent_bold().bg(th.raised), inner.width - 2);
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
