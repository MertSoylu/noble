//! Drawing layer: top strip, views, status bar, notifications, overlays.

mod boot;
mod bridge;
pub mod hud;
mod overlay;
mod settings;
mod system;
pub(crate) mod terminal;

pub use settings::settings_width;
pub use terminal::{PaneButton, pane_outline};

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::app::{AgentState, App, Hit, ToastLevel, View};
use crate::keys::Action;
use crate::term::TabAlert;
use crate::util;

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    app.size = (area.width, area.height);
    app.sync_layout();
    let mut hits: Vec<(Rect, Hit)> = Vec::new();
    let buf = f.buffer_mut();
    hud::clear(buf, area, &app.theme);

    let boot_ms = boot::elapsed(app);
    if let Some(ms) = boot_ms
        && boot::covers(ms)
    {
        boot::draw(buf, area, app, ms);
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
            settings::sync_scroll(app, body);
            settings::draw(buf, body, app, &mut hits);
            None
        }
        View::Term(i) => terminal::draw(buf, body, app, i, &mut hits),
    };
    let cursor = slide(buf, body, app).then_some(cursor).flatten();
    if area.height >= 2 {
        status_bar(buf, Rect::new(0, area.height - 1, area.width, 1), app, &hits);
    }
    update_notice(buf, area, app, &mut hits);
    toasts(buf, area, app);
    overlay::draw(buf, area, app, &mut hits);
    hover(buf, app, &hits);
    // The boot's closing iris opens over the finished frame.
    if let Some(ms) = boot_ms {
        boot::draw(buf, area, app, ms);
    }
    app.hits = hits;
    if app.overlay.is_none()
        && boot_ms.is_none()
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
        Hit::Backdrop | Hit::Inert | Hit::Pane { .. } | Hit::PaneTitle(_) | Hit::ScrollTrack { .. } => {}
        Hit::Divider { .. } => {
            // Draggable edge: highlight the shared frame line (not the title text on it).
            hud::tint_frame(buf, hud::Outline::closed(*rect), th.accent);
        }
        Hit::PaneSplit { .. } | Hit::PaneZoom(_) | Hit::PaneClose(_) => {
            // A pane button (" ◫", " ✕ "): the glyph turns into a small chip, close in the error color.
            // Inverted, so the glyph stays readable in every theme (the error or accent color as text
            // on the hover background is too faint in some of them). Many fonts draw these symbols wider
            // than one cell, spilling into the cell on their right, so the chip covers the glyph and the
            // space after it (every button is followed by a space cell).
            let style = match hit {
                Hit::PaneClose(_) => Style::default().fg(th.bg).bg(th.crit),
                _ => Style::default().fg(th.on_accent).bg(th.accent),
            };
            for dx in 1..3u16 {
                if let Some(c) = buf.cell_mut((rect.x.saturating_add(dx), rect.y)) {
                    c.set_style(style);
                    if dx == 1 {
                        c.set_style(Style::default().add_modifier(Modifier::BOLD));
                    }
                }
            }
        }
        Hit::ScrollLive(_) => {
            // The "↓ live" chip keeps its colors (its text is chosen for its own background) and
            // underlines its label.
            for xx in rect.left() + 1..rect.right().saturating_sub(1) {
                if let Some(c) = buf.cell_mut((xx, rect.y)) {
                    c.modifier.insert(Modifier::UNDERLINED);
                }
            }
        }
        Hit::TermScheme(_) | Hit::ThemeOption(_) => {
            // A selector row keeps its own colors; a marker is placed on the left edge.
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

/// One terminal tab of the top bar, measured before it is drawn.
struct TabCell {
    title: String,
    /// The most urgent agent state among the panes (a dot before the title).
    dot: Option<(&'static str, Style)>,
    /// "⊞2": the pane count of a split tab, drawn only when `show_split` (there is room).
    split: Option<String>,
    show_split: bool,
    /// What happened in the background (after the title, in its own cell).
    mark: Option<(&'static str, Style)>,
    active: bool,
}

impl TabCell {
    /// Cells before the title: the edge bar, plus the agent dot and its space.
    fn lead(&self) -> u16 {
        if self.dot.is_some() { 3 } else { 1 }
    }

    /// Width of the tab including its close button and the cell after it.
    fn width(&self) -> u16 {
        let split = self.split.as_ref().filter(|_| self.show_split).map_or(0, |s| 1 + util::width(s) as u16);
        // Title, a space before the close button, "×" and the closing cell.
        self.lead() + util::width(&self.title) as u16 + split + if self.mark.is_some() { 2 } else { 0 } + 3
    }
}

fn tab_cell(app: &App, i: usize, compact: bool, working: &'static str) -> TabCell {
    let th = &app.theme;
    let tab = &app.tabs[i];
    let active = app.view == View::Term(i);
    // The most urgent agent state among the tab's panes leads the title as a one-cell dot
    // (two cells with its space; the title gives them up so the tab keeps its width).
    let state = app.tab_agent_state(i);
    let dot = state.map(|s| terminal::agent_glyph(s, working, th));
    let max = if compact { 12 } else { 24 } - if dot.is_some() { 2 } else { 0 };
    let panes = tab.panes().len();
    TabCell {
        title: util::truncate(&app.tab_title(i), max),
        dot,
        split: (panes > 1).then(|| format!("⊞{panes}")),
        show_split: false,
        // The tab being viewed has seen everything.
        // A ◆ notice is not repeated after the title when the dot already says the agent needs you.
        mark: tab_marker(tab, th)
            .filter(|_| !active && !(tab.alert == Some(TabAlert::Notice) && state == Some(AgentState::NeedsYou)))
            .map(|(_, glyph, style)| (glyph, style)),
        active,
    }
}

/// The marker of a tab that had something happen in the background, with its rank: a notice or a
/// bell (◆), a failed command (✗), a long command that finished (✓), plain output (•).
fn tab_marker(tab: &crate::term::Tab, th: &crate::theme::Theme) -> Option<(u8, &'static str, Style)> {
    let bold = |c| Style::default().fg(c).add_modifier(Modifier::BOLD);
    match tab.alert {
        Some(TabAlert::Notice) => Some((3, "◆", bold(th.warn))),
        Some(TabAlert::Failed) => Some((2, "✗", bold(th.crit))),
        Some(TabAlert::Done) => Some((1, "✓", Style::default().fg(th.ok))),
        None if tab.activity => Some((0, "•", Style::default().fg(th.accent2))),
        None => None,
    }
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
    let mut cells: Vec<TabCell> = (0..app.tabs.len())
        .map(|i| tab_cell(app, i, compact, terminal::working_glyph(app, &app.tabs[i].panes())))
        .collect();
    let mut widths: Vec<u16> = Vec::new();
    let mut used = x;
    for c in &cells {
        let w = c.width();
        if used + w + 4 > limit {
            break;
        }
        used += w;
        widths.push(w);
    }
    let mut shown = widths.len();
    // The "+N" summary carries the most urgent marker of the tabs it hides; a tab gives way
    // when that does not fit next to the count.
    let hidden_mark = |from: usize| {
        (from..app.tabs.len())
            .filter(|i| app.view != View::Term(*i))
            .filter_map(|i| tab_marker(&app.tabs[i], th))
            .max_by_key(|m| m.0)
    };
    while shown > 0 && shown < cells.len() {
        let count = util::width(&format!(" +{} ", cells.len() - shown)) as u16;
        let need = count + if hidden_mark(shown).is_some() { 2 } else { 0 };
        if used + need <= limit {
            break;
        }
        shown -= 1;
        used -= widths[shown];
    }
    let hidden = cells.len() - shown;
    if hidden == 0 {
        // Split counts are the first thing to go: they only appear in what the tabs leave free,
        // the active tab's first (and only that one in compact mode).
        let mut slack = limit.saturating_sub(used + 4);
        let mut order: Vec<usize> = (0..cells.len()).collect();
        order.sort_by_key(|i| !cells[*i].active);
        for i in order {
            let c = &mut cells[i];
            let Some(split) = &c.split else { continue };
            let w = 1 + util::width(split) as u16;
            if (!compact || c.active) && w <= slack {
                c.show_split = true;
                slack -= w;
            }
        }
    }
    for (i, c) in cells.iter().enumerate().take(shown) {
        let bg = if c.active { th.bg } else { th.raised };
        let base = tab_style(c.active);
        let start = x;
        x = hud::put(buf, x, y, &" ".repeat(c.lead() as usize), base, c.lead());
        if c.active {
            // Edge bars: the active tab is more than a color.
            hud::put(buf, start, y, "▌", Style::default().fg(th.accent).bg(th.bg), 1);
        }
        if let Some((glyph, style)) = c.dot {
            hud::put(buf, start + 1, y, glyph, style.bg(bg), 1);
        }
        x = hud::put(buf, x, y, &c.title, base, limit.saturating_sub(x));
        if let Some(split) = c.split.as_ref().filter(|_| c.show_split) {
            x = hud::put(buf, x, y, " ", base, 1);
            x = hud::put(buf, x, y, split, th.dim().bg(bg), limit.saturating_sub(x));
        }
        if let Some((glyph, style)) = c.mark {
            // The marker has its own cell after the title.
            x = hud::put(buf, x, y, " ", base, 1);
            x = hud::put(buf, x, y, glyph, style.bg(bg), 1);
        }
        x = hud::put(buf, x, y, " ", base, 1);
        hits.push((Rect::new(start, y, x - start, 1), Hit::Tab(i)));
        let cx = x;
        x = hud::put(buf, x, y, "×", Style::default().fg(th.dim).bg(bg), 1);
        hits.push((Rect::new(cx, y, 1, 1), Hit::TabClose(i)));
        if c.active {
            x = hud::put(buf, x, y, "▐", Style::default().fg(th.accent).bg(th.bg), 1);
        } else {
            x = hud::put(buf, x, y, " ", Style::default().bg(th.raised), 1);
        }
    }
    if hidden > 0 {
        let mark = hidden_mark(shown);
        let count = format!(" +{hidden}");
        let mut spans = vec![(count.as_str(), th.dim().bg(th.raised))];
        if let Some((_, glyph, style)) = mark {
            spans.push((" ", th.dim().bg(th.raised)));
            spans.push((glyph, style.bg(th.raised)));
        }
        spans.push((" ", th.dim().bg(th.raised)));
        x = hud::put_spans(buf, x, y, &spans, limit.saturating_sub(x));
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

/// The pane button under the mouse, as a status bar hint: its glyph, what it does and the shortcut
/// that does the same in a pane ("◫ split right · ctrl+a v").
fn pane_button_hint(app: &App, hits: &[(Rect, Hit)]) -> Option<(String, String)> {
    let (x, y) = app.hover.filter(|_| app.overlay.is_none() && app.drag_kind().is_none())?;
    let pos = ratatui::layout::Position { x, y };
    let (_, hit) = hits.iter().rev().find(|(r, _)| r.contains(pos))?;
    let (button, pane) = terminal::PaneButton::of_hit(hit)?;
    let zoomed = app.tabs.iter().any(|t| t.zoomed && t.root.contains(pane));
    let what = button.what(zoomed);
    let text = match app.keymap.term_hint(button.action(), app.focused_locked()) {
        Some(key) => format!("{what} · {key}"),
        None => what.to_string(),
    };
    Some((button.glyph(zoomed).to_string(), text))
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
            if app.bridge.proj_act.is_some() {
                return vec![h("←→", "choose"), h("⏎", "apply"), h("esc", "back")];
            }
            vec![
                h("→", "pin / more"),
                h("t", "terminal"),
                h("a A", "add folder / project"),
                h("r", "rescan"),
                h("s", "settings"),
                h("?", "help"),
            ]
        }
        View::System => {
            if app.system.filtering {
                return vec![h("type", "filter"), h("⏎", "done"), h("esc", "clear")];
            }
            vec![h("↑↓", "select"), h("c m p n", "sort"), h("/", "filter"), h("K", "end task"), h("esc", "home")]
        }
        View::Settings => vec![h("↑↓", "move"), h("⏎", "change"), h("←→", "adjust"), h("esc", "back")],
        View::Term(_) if app.search.is_some() => {
            vec![h("type", "search"), h("⏎ ↑", "older"), h("↓", "newer"), h("esc", "close")]
        }
        View::Term(_) if app.pass_next.is_some() && app.pass_next == app.focused_pane() => {
            vec![h("any key", "goes to the app")]
        }
        View::Term(_) if app.focused_locked() => {
            let key = app.keymap.hint(Action::Passthrough).unwrap_or_default();
            vec![(key, "unlock keys".into()), (app.keymap.prefix.to_string(), "menu".into())]
        }
        // Scrolled back: a key typed into the pane returns to the newest output (and goes to the app).
        View::Term(_)
            if app.focused_pane().and_then(|id| app.panes.get(&id)).is_some_and(|p| p.scroll_offset() > 0) =>
        {
            let mut v = Vec::new();
            if let Some(key) = app.keymap.term_hint(Action::ScrollDown, app.focused_locked()) {
                v.push((key, "newer".into()));
            }
            v.extend([h("any key", "back to live"), (app.keymap.prefix.to_string(), "menu".into())]);
            v
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
    "background tab markers: ◆ needs you · ✗ failed · ✓ done",
    "press a on Home to add a folder with your projects",
    "alt+1…9 or prefix n / p switch tabs",
    "Settings → Terminal colors: pick any Windows Terminal scheme",
    "w on Home saves your open tabs as a workspace",
    "prefix z or a double-click on its title zooms a pane",
];

/// The hint to show right now (changes every 20 seconds). Until the prefix key has been
/// used once, the tip is how to reach it, with the configured key.
fn current_tip(app: &App) -> String {
    if !app.ui_state.data.prefix_used {
        return format!("press {}, then ? for all keys", app.keymap.prefix);
    }
    TIPS[(app.started.elapsed().as_secs() / 20) as usize % TIPS.len()].to_string()
}

fn status_bar(buf: &mut Buffer, area: Rect, app: &App, hits: &[(Rect, Hit)]) {
    let th = &app.theme;
    hud::fill(buf, area, Style::default().bg(th.raised));
    let y = area.y;
    // In a terminal a shell key or a locked pane's shortcut goes to the app: show one that works there.
    let palette = if matches!(app.view, View::Term(_)) {
        app.keymap.term_hint(Action::Palette, app.focused_locked())
    } else {
        app.keymap.hint(Action::Palette)
    };
    let right = format!("{} commands ", palette.unwrap_or_else(|| "ctrl+a :".into()));
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
    let hover = if app.prefix_armed { None } else { pane_button_hint(app, hits) };
    let pane_button = hover.is_some();
    for (k, v) in hover.map_or_else(|| hints(app), |h| vec![h]) {
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
    if !app.prefix_armed && !pane_button && x + tip_w + 4 <= limit {
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
