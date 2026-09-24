//! Drawing layer: top strip, views, status bar, notifications, overlays.

mod boot;
mod bridge;
pub mod hud;
mod overlay;
mod settings;
mod system;
mod terminal;

pub use settings::{THEME_CARD_W, settings_width};

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::app::{App, Hit, ToastLevel, View};
use crate::keys::Action;
use crate::util;

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    app.size = (area.width, area.height);
    app.sync_layout();
    let mut hits: Vec<(Rect, Hit)> = Vec::new();
    let buf = f.buffer_mut();
    hud::clear(buf, area, &app.theme);

    if app.boot.is_some() {
        boot::draw(buf, area, app);
        app.hits = hits;
        return;
    }

    top_bar(buf, Rect::new(0, 0, area.width, 1.min(area.height)), app, &mut hits);
    let body = app.body();
    let cursor = match app.view {
        View::Bridge => {
            bridge::draw(buf, body, app, &mut hits);
            None
        }
        View::System => {
            system::draw(buf, body, app, &mut hits);
            None
        }
        View::Settings => {
            settings::draw(buf, body, app, &mut hits);
            None
        }
        View::Term(i) => terminal::draw(buf, body, app, i, &mut hits),
    };
    let cursor = slide(buf, body, app).then_some(cursor).flatten();
    if area.height >= 2 {
        status_bar(buf, Rect::new(0, area.height - 1, area.width, 1), app);
    }
    update_notice(buf, area, app, &mut hits);
    toasts(buf, area, app);
    overlay::draw(buf, area, app, &mut hits);
    hover(buf, app, &hits);
    app.hits = hits;
    if app.overlay.is_none()
        && let Some((x, y)) = cursor
    {
        f.set_cursor_position((x, y));
    }
}

/// The page's order in the tab strip: determines the transition direction.
fn view_order(v: View) -> usize {
    match v {
        View::Bridge => 0,
        View::System => 1,
        View::Term(i) => 2 + i,
        View::Settings => usize::MAX,
    }
}

/// Starts the transition and applies the scroll when the page changed. `true` when there is none.
fn slide(buf: &mut Buffer, body: Rect, app: &mut App) -> bool {
    let body = body.intersection(buf.area);
    let current = buf_region(buf, body);
    if let (Some(prev), Some(from)) = (app.drawn_view, app.last_body.take())
        && prev != app.view
        && app.cfg.general.animations
        && from.area == body
    {
        let dir = if view_order(app.view) > view_order(prev) { 1 } else { -1 };
        app.slide = Some(crate::app::Slide { from, dir, started: std::time::Instant::now() });
    }
    app.drawn_view = Some(app.view);
    app.last_body = Some(current.clone());
    let Some(sl) = &app.slide else { return true };
    if sl.from.area != body {
        app.slide = None;
        return true;
    }
    let t = crate::app::ease(sl.started, crate::app::SLIDE_DURATION);
    let w = body.width as i32;
    // The new page comes from direction `dir`; the old one leaves the same way at the same speed.
    let shift = (sl.dir as f64 * (1.0 - t) * w as f64).round() as i32;
    for y in body.top()..body.bottom() {
        for x in body.left()..body.right() {
            let col = (x - body.x) as i32;
            let new_col = col - shift;
            let (src, sc) =
                if (0..w).contains(&new_col) { (&current, new_col) } else { (&sl.from, new_col + sl.dir * w) };
            if (0..w).contains(&sc)
                && let (Some(from), Some(to)) = (src.cell((body.x + sc as u16, y)), buf.cell_mut((x, y)))
            {
                *to = from.clone();
            }
        }
    }
    false
}

fn buf_region(buf: &Buffer, area: Rect) -> Buffer {
    let mut out = Buffer::empty(area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let (Some(src), Some(dst)) = (buf.cell((x, y)), out.cell_mut((x, y))) {
                *dst = src.clone();
            }
        }
    }
    out
}

/// Highlights the clickable item under the mouse: background on rows and buttons,
/// frame color on panels, line color on dividers.
fn hover(buf: &mut Buffer, app: &App, hits: &[(Rect, Hit)]) {
    let Some((x, y)) = app.hover else { return };
    if app.drag.is_some() && !matches!(app.drag_kind(), Some("divider")) {
        return;
    }
    let pos = ratatui::layout::Position { x, y };
    let Some((rect, hit)) = hits.iter().rev().find(|(r, _)| r.contains(pos)) else { return };
    let th = &app.theme;
    match hit {
        Hit::Backdrop | Hit::Inert | Hit::Pane { .. } | Hit::PaneTitle(_) => {}
        Hit::Divider { .. } => {
            // Draggable edge: highlight the shared frame line.
            for yy in rect.top()..rect.bottom() {
                for xx in rect.left()..rect.right() {
                    if let Some(c) = buf.cell_mut((xx, yy)) {
                        c.set_fg(th.accent);
                    }
                }
            }
        }
        Hit::Setting(i) if *i < crate::theme::THEMES.len() => {
            // The theme card keeps its own colors; a marker is placed on the left edge.
            if let Some(c) = buf.cell_mut((rect.x, rect.y)) {
                c.set_symbol("▌");
                c.set_fg(th.accent);
            }
        }
        Hit::TermScheme(_) => {
            // The scheme chip also keeps its own colors.
            if let Some(c) = buf.cell_mut((rect.x, rect.y)) {
                c.set_symbol("▌");
                c.set_fg(th.accent);
            }
        }
        _ if rect.height == 1 => {
            let bg = th.hover();
            for xx in rect.left()..rect.right() {
                if let Some(c) = buf.cell_mut((xx, rect.y)) {
                    c.set_bg(bg);
                }
            }
        }
        _ => {
            // Large clickable panels: the frame turns to the accent color.
            let edge = |xx: u16, yy: u16| {
                xx == rect.left() || xx == rect.right() - 1 || yy == rect.top() || yy == rect.bottom() - 1
            };
            for yy in rect.top()..rect.bottom() {
                for xx in rect.left()..rect.right() {
                    if edge(xx, yy)
                        && let Some(c) = buf.cell_mut((xx, yy))
                        && c.fg == th.line
                    {
                        c.set_fg(th.accent);
                    }
                }
            }
        }
    }
}

/// Height of the battery block: a 3-row icon when there is room, one row otherwise; 0 without a battery.
pub(crate) fn battery_rows(app: &App, avail: u16) -> u16 {
    match app.sensors.battery() {
        Some(_) if avail >= 16 => 3,
        Some(_) if avail >= 6 => 1,
        _ => 0,
    }
}

/// Battery block: icon on the left, percentage and time left on the right. While
/// charging a shimmer slides inside the icon and ↯ shows next to the percentage.
pub(crate) fn battery_block(buf: &mut Buffer, area: Rect, app: &App) {
    use crate::battery::PowerState;
    let Some((b, eta)) = app.sensors.battery() else { return };
    if area.width < 16 || area.height == 0 {
        return;
    }
    let th = &app.theme;
    let color = battery_color(b, th);
    let charging = b.state == PowerState::Charging;
    let pct = format!("{:.0}%", b.percent);
    let bolt = if charging { " ↯" } else { "" };
    let status = crate::battery::short(b, eta);
    let tall = area.height >= 3;
    let text_w = if tall {
        [util::width(&status), util::width(&pct) + 2, 7].into_iter().max().unwrap_or(7) as u16
    } else {
        (util::width(&pct) + util::width(bolt) + 1 + util::width(&status)) as u16
    }
    .min(area.width / 2);
    let icon_w = area.width.saturating_sub(text_w + 2).min(30);
    // The shimmer slides through the fill from left to right and pauses briefly at the end.
    let cells = icon_w.saturating_sub(3);
    let filled = (b.percent as f64 / 100.0 * cells as f64).floor() as u16;
    let shimmer = (charging && filled > 1).then(|| {
        // Slow step: an idle screen must not change every frame and burn CPU.
        let step = (app.started.elapsed().as_millis() / 450) as u16;
        step % (filled + 4)
    });
    hud::battery_icon(buf, Rect::new(area.x, area.y, icon_w, area.height.min(3)), b.percent as f64, color, th, shimmer);
    let tx = area.x + icon_w + 2;
    let room = area.right().saturating_sub(tx);
    let pct_style = Style::default().fg(color).add_modifier(Modifier::BOLD);
    let bolt_style = Style::default().fg(th.accent2).add_modifier(Modifier::BOLD);
    if tall {
        hud::put(buf, tx, area.y, "BATTERY", th.dim(), room);
        hud::put_spans(buf, tx, area.y + 1, &[(&pct, pct_style), (bolt, bolt_style)], room);
        hud::put(buf, tx, area.y + 2, &util::truncate(&status, room as usize), th.dim(), room);
    } else {
        let status = format!(" {status}");
        hud::put_spans(buf, tx, area.y, &[(&pct, pct_style), (bolt, bolt_style), (&status, th.dim())], room);
    }
}

/// Battery color: accent2 while charging, warning → critical as it drains on battery.
pub(crate) fn battery_color(b: &crate::battery::Battery, th: &crate::theme::Theme) -> ratatui::style::Color {
    use crate::battery::PowerState;
    match b.state {
        PowerState::Charging => th.accent2,
        PowerState::Full | PowerState::PluggedIn => th.ok,
        PowerState::Discharging if b.percent <= 15.0 => th.crit,
        PowerState::Discharging if b.percent <= 30.0 => th.warn,
        PowerState::Discharging => th.ok,
    }
}

fn clock_text(app: &App, seconds: bool) -> String {
    let now = chrono::Local::now();
    let fmt = match (app.cfg.general.clock_24h, seconds) {
        (true, true) => "%H:%M:%S",
        (true, false) => "%H:%M",
        (false, true) => "%I:%M:%S %p",
        (false, false) => "%I:%M %p",
    };
    now.format(fmt).to_string()
}

fn top_bar(buf: &mut Buffer, area: Rect, app: &App, hits: &mut Vec<(Rect, Hit)>) {
    if area.height == 0 {
        return;
    }
    let th = &app.theme;
    hud::fill(buf, area, Style::default().bg(th.raised));
    let y = area.y;
    let clock = format!(" {} ", clock_text(app, false));
    let compact = area.width < 80;
    let settings_label = if compact { " ⚙ " } else { " ⚙ Settings " };
    let right_w = util::width(&clock) as u16 + util::width(settings_label) as u16 + 1;
    let limit = area.right().saturating_sub(right_w);

    let tab_style = |active: bool| {
        if active {
            Style::default().fg(th.accent).bg(th.bg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(th.dim).bg(th.raised)
        }
    };
    let brand = if app.dev { " NOBLE dev " } else { " NOBLE " };
    let mut x = hud::put(buf, area.x, y, brand, th.chip(), limit);
    x += 1;
    let fixed = [(View::Bridge, "Home", Hit::TabBridge), (View::System, "System", Hit::TabSystem)];
    for (view, label, hit) in fixed {
        let start = x;
        x = hud::put(buf, x, y, &format!(" {label} "), tab_style(app.view == view), limit.saturating_sub(x));
        hits.push((Rect::new(start, y, x - start, 1), hit));
    }
    x = hud::put(buf, x, y, " │", th.line().bg(th.raised), limit.saturating_sub(x));
    // Terminal tabs: those that do not fit are summarized as "+N".
    let mut hidden = 0;
    for i in 0..app.tabs.len() {
        let active = app.view == View::Term(i);
        let title = util::truncate(&app.tab_title(i), if compact { 12 } else { 24 });
        let text = format!(" {title} ");
        let tw = util::width(&text) as u16 + 2;
        if x + tw + 4 > limit {
            hidden = app.tabs.len() - i;
            break;
        }
        let start = x;
        x = hud::put(buf, x, y, &text, tab_style(active), tw);
        if app.tabs[i].alert && !active {
            // Dikkat isteyen sekme: vurgulu elmas (tamamlanan komut, zil, bildirim).
            hud::put(buf, x - 1, y, "◆", Style::default().fg(th.warn).bg(th.raised).add_modifier(Modifier::BOLD), 1);
        } else if app.tabs[i].activity && !active {
            hud::put(buf, x - 1, y, "•", Style::default().fg(th.accent2).bg(th.raised), 1);
        }
        hits.push((Rect::new(start, y, x - start, 1), Hit::Tab(i)));
        let cx = x;
        let bg = if active { th.bg } else { th.raised };
        x = hud::put(buf, x, y, "× ", Style::default().fg(th.dim).bg(bg), 2);
        hits.push((Rect::new(cx, y, 1, 1), Hit::TabClose(i)));
    }
    if hidden > 0 {
        x = hud::put(buf, x, y, &format!(" +{hidden} "), th.dim().bg(th.raised), limit.saturating_sub(x));
    }
    let start = x;
    x = hud::put(buf, x, y, " + ", Style::default().fg(th.accent).bg(th.raised), limit.saturating_sub(x));
    if x > start {
        hits.push((Rect::new(start, y, x - start, 1), Hit::NewTab));
    }

    let rx = hud::put_right(buf, area.right(), y, &clock, Style::default().fg(th.fg).bg(th.raised));
    let sx = rx.saturating_sub(util::width(settings_label) as u16 + 1);
    hud::put(buf, sx, y, settings_label, tab_style(app.view == View::Settings), util::width(settings_label) as u16);
    hits.push((Rect::new(sx, y, util::width(settings_label) as u16, 1), Hit::TabSettings));
}

/// A few short hints for the context.
fn hints(app: &App) -> Vec<(String, String)> {
    let h = |k: &str, v: &str| (k.to_string(), v.to_string());
    if app.prefix_armed {
        // Pane commands (split, close, zoom, focus) only make sense in a terminal.
        if matches!(app.view, View::Term(_)) {
            return vec![
                h("t", "new tab"),
                h("v", "split right"),
                h("s", "split down"),
                h("x", "close"),
                h("z", "zoom"),
                h("arrows", "move focus"),
                h("0", "home"),
                h("?", "all keys"),
            ];
        }
        let mut v = vec![h("t", "new tab")];
        if !app.tabs.is_empty() {
            v.push(h("1…9", "go to tab"));
        }
        if app.view != View::Bridge {
            v.push(h("0", "home"));
        }
        if app.view != View::Settings {
            v.push(h("S", "settings"));
        }
        v.extend([h(":", "commands"), h("?", "all keys")]);
        return v;
    }
    match app.view {
        View::Bridge => {
            // ⏎, the launchers (c, x) and "/ search" are already on the Projects
            // panel's buttons; only shortcuts not in the panel appear here.
            if app.bridge.filtering {
                return vec![h("type", "search"), h("↑↓", "select"), h("esc", "clear")];
            }
            vec![h("t", "terminal"), h("a", "add folder"), h("r", "rescan"), h("s", "settings"), h("?", "help")]
        }
        View::System => {
            if app.system.filtering {
                return vec![h("type", "filter"), h("⏎", "done"), h("esc", "clear")];
            }
            vec![h("↑↓", "select"), h("c m p n", "sort"), h("/", "filter"), h("K", "end task"), h("esc", "home")]
        }
        View::Settings => vec![h("↑↓", "move"), h("⏎", "change"), h("←→", "adjust"), h("esc", "home")],
        View::Term(_) if app.search.is_some() => {
            vec![h("type", "search"), h("⏎ ↑", "older"), h("↓", "newer"), h("esc", "close")]
        }
        View::Term(_) => vec![
            (app.keymap.prefix.to_string(), "menu".into()),
            h("alt+0", "home"),
            h("drag border", "resize"),
            h("right-click", "paste"),
        ],
    }
}

/// Hints shown in turn on the status bar (lesser-known features).
const TIPS: [&str; 10] = [
    "ctrl+click a URL or file:line in a terminal to open it",
    "prefix / searches a terminal's scrollback",
    "drag a tab to reorder it · double-click to rename",
    "right-click a tab, a pane title or a project for more",
    "a background tab shows ◆ when it needs you",
    "press a on Home to add a folder with your projects",
    "alt+. and alt+, switch to the next / previous tab",
    "Settings → Terminal colors: pick any Windows Terminal scheme",
    "w on Home saves your open tabs as a workspace",
    "prefix z zooms the focused pane",
];

/// The hint to show right now (changes every 20 seconds).
fn current_tip(app: &App) -> &'static str {
    TIPS[(app.started.elapsed().as_secs() / 20) as usize % TIPS.len()]
}

fn status_bar(buf: &mut Buffer, area: Rect, app: &App) {
    let th = &app.theme;
    hud::fill(buf, area, Style::default().bg(th.raised));
    let y = area.y;
    let right = format!("{} commands ", app.keymap.hint(Action::Palette).unwrap_or_else(|| "ctrl+a :".into()));
    let right_w = util::width(&right) as u16;
    let limit = area.right().saturating_sub(right_w + 1);
    let mut x = area.x + 1;
    if app.prefix_armed {
        x = hud::put(
            buf,
            x,
            y,
            &format!(" {} ", app.keymap.prefix),
            Style::default().fg(th.on_accent).bg(th.warn).add_modifier(Modifier::BOLD),
            limit.saturating_sub(x),
        );
        x += 1;
    }
    for (k, v) in hints(app) {
        let need = (util::width(&k) + util::width(&v) + 3) as u16;
        if x + need > limit {
            break;
        }
        x = hud::put(buf, x, y, &k, Style::default().fg(th.accent).bg(th.raised), need);
        x = hud::put(buf, x, y, &format!(" {v}   "), Style::default().fg(th.dim).bg(th.raised), need);
    }
    if area.width >= 50 {
        hud::put_right(buf, area.right(), y, &right, th.dim().bg(th.raised));
    }
    // The hint shows only if it fits in the space left after the shortcut hints.
    let tip = format!("tip: {}", current_tip(app));
    let tip_w = util::width(&tip) as u16;
    if !app.prefix_armed && x + tip_w + 4 <= limit {
        hud::put_right(buf, limit.saturating_sub(2), y, &tip, Style::default().fg(th.accent_dim).bg(th.raised));
    }
}

/// New version notice: bottom right, just above the status bar. On terminal tabs
/// it drops to the right of the status bar so it never covers the shell's last line.
fn update_notice(buf: &mut Buffer, area: Rect, app: &App, hits: &mut Vec<(Rect, Hit)>) {
    let Some(version) = app.update_notice() else { return };
    if area.height < 6 || area.width < 30 || app.overlay.is_some() {
        return;
    }
    let th = &app.theme;
    let in_term = matches!(app.view, View::Term(_));
    let y = area.bottom() - if in_term { 1 } else { 2 };
    let (button, close) = (" update ", " × ");
    let fixed = (1 + util::width(button) + util::width(close) + 1) as u16;
    let long = format!(" ↑ NOBLE {version} is available ");
    let short = format!(" ↑ {version} ");
    let room = area.width.saturating_sub(fixed + 2) / 2;
    let text = if !in_term && util::width(&long) as u16 <= room { long } else { short };
    let w = fixed + util::width(&text) as u16;
    let x0 = area.right().saturating_sub(w);
    let bg = Style::default().bg(th.raised);
    let mut x = hud::put(buf, x0, y, "▌", Style::default().fg(th.accent2).bg(th.raised), 1);
    x = hud::put(buf, x, y, &text, bg.fg(th.fg).add_modifier(Modifier::BOLD), w);
    x = hud::put(buf, x, y, button, Style::default().fg(th.on_accent).bg(th.accent2).add_modifier(Modifier::BOLD), w);
    hits.push((Rect::new(x0, y, x - x0, 1), Hit::Update));
    let cx = x;
    x = hud::put(buf, x, y, close, bg.fg(th.dim), w);
    hud::put(buf, x, y, " ", bg, 1);
    hits.push((Rect::new(cx, y, x - cx, 1), Hit::UpdateDismiss));
}

fn toasts(buf: &mut Buffer, area: Rect, app: &App) {
    let th = &app.theme;
    let mut y = area.bottom().saturating_sub(3);
    for t in app.toasts.iter().rev() {
        if y <= area.y + 1 {
            break;
        }
        let color = match t.level {
            ToastLevel::Info => th.accent2,
            ToastLevel::Ok => th.ok,
            ToastLevel::Warn => th.warn,
            ToastLevel::Error => th.crit,
        };
        let max = area.width.saturating_sub(4).min(70) as usize;
        let text = format!(" {} ", util::truncate(&t.text, max.saturating_sub(3)));
        let w = util::width(&text) as u16 + 1;
        let x = area.right().saturating_sub(w + 1);
        hud::put(buf, x, y, "▌", Style::default().fg(color).bg(th.raised), 1);
        hud::put(buf, x + 1, y, &text, Style::default().fg(th.fg).bg(th.raised), w);
        y -= 1;
    }
}
