//! Home screen: clock + projects on the left, AI usage and system summary on the
//! right. Kept simple: every panel has a single job and everything is clickable.

use std::time::{Duration, SystemTime};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::hud;
use crate::ai::{ProviderState, Status};
use crate::app::{AgentState, AgentsByProject, App, Hit, ProjectAct};
use crate::pr::{Checks, PrInfo, PrState};
use crate::projects::Project;
use crate::theme::Theme;
use crate::util;

pub fn draw(buf: &mut Buffer, area: Rect, app: &App, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    if area.width < 24 || area.height < 6 {
        hud::too_small(buf, area, th);
        return;
    }
    let area = if area.width >= 60 { Rect::new(area.x + 1, area.y, area.width - 2, area.height) } else { area };
    // Collected once per frame: the project rows, the AI panel and the quota strip all read it.
    let sessions = app.all_agent_sessions();
    let agents = app.agent_sessions_by_project(&sessions);
    app.fx.observe_agents(sessions.iter().map(|s| (s.pane, s.state)));
    draw_columns(buf, area, app, &sessions, &agents, hits);
    // Coming back to Home: the rows fade in from the top, one after another.
    if app.fx.raw(crate::app::fx::Fx::HomeIn).is_some() {
        let steps = 10u64;
        for (n, y) in (area.top()..area.bottom()).enumerate() {
            let step = n as u64 * steps / area.height.max(1) as u64;
            let shown = app.fx.home_step(step);
            hud::fade_rect(buf, Rect::new(area.x, y, area.width, 1), th.bg, 1.0 - shown);
        }
    }
}

/// Home's panels: one column (clock, quota strip, projects) or two (projects | AI and system).
fn draw_columns(
    buf: &mut Buffer,
    area: Rect,
    app: &App,
    sessions: &[crate::app::AgentSession],
    agents: &AgentsByProject,
    hits: &mut Vec<(Rect, Hit)>,
) {
    let two_col = area.width >= 92 && area.height >= 16;
    if !two_col {
        let hero_h = if area.height >= 14 { 3 } else { 0 };
        if hero_h > 0 {
            hero_compact(buf, Rect::new(area.x, area.y, area.width, hero_h), app);
        }
        // One column has no AI panel: a one-line strip keeps quota and "needs you" in sight.
        let strip_h = u16::from(
            hero_h > 0 && area.height >= 16 && quota_strip(buf, area.x, area.y + hero_h, area.width, app, sessions),
        );
        let top = hero_h + strip_h;
        projects(buf, Rect::new(area.x, area.y + top, area.width, area.height - top), app, agents, hits);
        return;
    }
    let right_w = ((area.width as f32 * 0.34) as u16).clamp(34, 46);
    let left = Rect::new(area.x, area.y, area.width - right_w - 2, area.height);
    let right = Rect::new(area.right() - right_w, area.y, right_w, area.height);

    let hero_h = if left.height >= 20 { 5 } else { 3 };
    if hero_h == 5 {
        hero(buf, Rect::new(left.x, left.y, left.width, hero_h), app);
    } else {
        hero_compact(buf, Rect::new(left.x, left.y, left.width, hero_h), app);
    }
    projects(buf, Rect::new(left.x, left.y + hero_h, left.width, left.height - hero_h), app, agents, hits);

    let signed_in = signed_in(app);
    let room = right.height.saturating_sub(hero_h + 10);
    let shown = !signed_in.is_empty() || !sessions.is_empty();
    let ai_h = if shown { ai_height(app, &signed_in, sessions.len()).min(room + hero_h) } else { 0 };
    // The right column starts under the clock: in line with the project panel on the left.
    let top = right.y + hero_h;
    let avail = right.height.saturating_sub(hero_h);
    if ai_h >= 5 {
        ai_panel(buf, Rect::new(right.x, top, right.width, ai_h), app, &signed_in, sessions, hits);
        system_panel(buf, Rect::new(right.x, top + ai_h, right.width, avail - ai_h), app, hits);
    } else {
        system_panel(buf, Rect::new(right.x, top, right.width, avail), app, hits);
    }
}

fn greeting(hour: u32) -> &'static str {
    match hour {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        18..=22 => "Good evening",
        _ => "Working late",
    }
}

pub(super) fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn session_line(app: &App) -> String {
    let n = app.pane_count();
    match n {
        0 => "no terminals open".into(),
        1 => "1 terminal open".into(),
        n => format!("{n} terminals open"),
    }
}

/// Running version, e.g. "v1.7.0" or "v1.7.0 dev" for the dev build.
fn version_label() -> String {
    let dev = if util::is_dev_build() { " dev" } else { "" };
    format!("v{}{dev}", env!("CARGO_PKG_VERSION"))
}

/// Big clock + greeting (5 rows).
fn hero(buf: &mut Buffer, area: Rect, app: &App) {
    use chrono::Timelike;
    let th = &app.theme;
    let now = app.now();
    let time = now.format(if app.cfg.general.clock_24h { "%H:%M" } else { "%I:%M" }).to_string();
    let x = area.x + 1;
    let y = area.y + 1;
    let live = app.live_clock();
    let colon =
        if !live || now.second().is_multiple_of(2) { th.accent_bold() } else { Style::default().fg(th.accent_dim) };
    hud::big_text(buf, x, y, &time, th.accent_bold(), colon);
    let mut tx = x + hud::big_width(&time) + 1;
    let seconds = app.cfg.general.show_seconds && live;
    if seconds || !app.cfg.general.clock_24h {
        let small = match (seconds, app.cfg.general.clock_24h) {
            (true, true) => now.format("%S").to_string(),
            (true, false) => now.format("%S %p").to_string(),
            _ => now.format("%p").to_string(),
        };
        hud::put(buf, tx, y + 2, &small, Style::default().fg(th.accent_dim), 6);
        tx += util::width(&small) as u16 + 1;
    }
    let tx = tx + 3;
    let w = area.right().saturating_sub(tx);
    let greet = format!("{}, ", greeting(now.hour()));
    hud::put_spans(buf, tx, y, &[(&greet, th.text()), (&capitalize(&app.operator), th.accent_bold())], w);
    hud::put(buf, tx, y + 1, &now.format("%A, %d %B").to_string(), th.dim(), w);
    let sep = Style::default().fg(th.line);
    hud::put_spans(
        buf,
        tx,
        y + 2,
        &[(&session_line(app), th.dim()), ("  ·  ", sep), (&version_label(), Style::default().fg(th.accent_dim))],
        w,
    );
}

/// One-line clock + greeting (narrow windows).
fn hero_compact(buf: &mut Buffer, area: Rect, app: &App) {
    use chrono::Timelike;
    let th = &app.theme;
    let now = app.now();
    let fmt = match (app.cfg.general.clock_24h, app.cfg.general.show_seconds && app.live_clock()) {
        (true, true) => "%H:%M:%S",
        (true, false) => "%H:%M",
        (false, true) => "%I:%M:%S %p",
        (false, false) => "%I:%M %p",
    };
    let y = area.y + area.height.saturating_sub(1) / 2;
    let x = area.x + 1;
    let x = hud::put(buf, x, y, &now.format(fmt).to_string(), th.accent_bold(), area.width);
    let sep = Style::default().fg(th.line);
    let hello = format!("   {}, ", greeting(now.hour()));
    let date = now.format("%a %d %b").to_string();
    hud::put_spans(
        buf,
        x,
        y,
        &[
            (&hello, th.text()),
            (&capitalize(&app.operator), th.accent_bold()),
            ("  ·  ", sep),
            (&date, th.dim()),
            ("  ·  ", sep),
            (&version_label(), Style::default().fg(th.accent_dim)),
        ],
        area.right().saturating_sub(x),
    );
}

/// The one-line quota summary of narrow Home: "◆ 2 need you · claude 71% · codex 92%~". Whole items only,
/// dropped from the end when they do not fit. Returns `false` (drawing nothing) when there is nothing to say.
fn quota_strip(buf: &mut Buffer, x: u16, y: u16, w: u16, app: &App, sessions: &[crate::app::AgentSession]) -> bool {
    let th = &app.theme;
    let needs = sessions.iter().filter(|s| s.state == AgentState::NeedsYou).count();
    let mut items: Vec<(String, Style)> = Vec::new();
    if needs > 0 {
        let verb = if needs == 1 { "needs" } else { "need" };
        items.push((format!("◆ {needs} {verb} you"), Style::default().fg(th.warn).add_modifier(Modifier::BOLD)));
    }
    for p in signed_in(app) {
        let (text, style) = match (&p.status, p.peak()) {
            (Status::Expired, _) => (format!("{} expired", p.id), th.dim()),
            (Status::Ok, Some(pct)) => (format!("{} {pct}%", p.id), Style::default().fg(th.level(pct as f64))),
            (_, Some(pct)) => (format!("{} ~{pct}%", p.id), th.dim()),
            (_, None) => (format!("{} …", p.id), th.dim()),
        };
        items.push((text, style));
    }
    if items.is_empty() || w < 8 {
        return false;
    }
    let sep = "  ·  ";
    let end = x + w;
    let mut cx = x + 1;
    for (i, (text, style)) in items.iter().enumerate() {
        let need = util::width(text) as u16 + if i > 0 { util::width(sep) as u16 } else { 0 };
        if cx + need > end {
            break;
        }
        if i > 0 {
            cx = hud::put(buf, cx, y, sep, Style::default().fg(th.line), end.saturating_sub(cx));
        }
        cx = hud::put(buf, cx, y, text, *style, end.saturating_sub(cx));
    }
    true
}

fn ago(t: SystemTime) -> String {
    util::fmt_ago(SystemTime::now().duration_since(t).unwrap_or_default())
}

fn ago_ts(ts: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    util::fmt_ago(Duration::from_secs(now.saturating_sub(ts).max(0) as u64))
}

fn projects(buf: &mut Buffer, area: Rect, app: &App, agents: &AgentsByProject, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    let vis = app.visible_projects();
    let total = app.projects.len();
    let tag = if !app.projects_loaded {
        "scanning…".to_string()
    } else if !app.bridge.filter.is_empty() {
        format!("{} of {total}", vis.len())
    } else {
        total.to_string()
    };
    let inner = hud::frame(buf, area, "Projects", &tag, true, th);
    search_label(buf, area, &tag, th, hits);
    if inner.height == 0 || inner.width < 10 {
        return;
    }
    let x = inner.x + 1;
    let w = inner.width.saturating_sub(2);
    let footer_h: u16 = if inner.height >= 8 {
        3
    } else if inner.height >= 3 {
        1
    } else {
        0
    };
    let mut y = inner.y;
    if app.bridge.filtering || !app.bridge.filter.is_empty() {
        let cursor = if app.bridge.filtering { "▏" } else { "" };
        hud::put_spans(
            buf,
            x,
            y,
            &[("search ", th.dim()), (&app.bridge.filter, th.accent_bold()), (cursor, th.accent())],
            w,
        );
        y += 1;
    }
    let mut list_h = inner.bottom().saturating_sub(y + footer_h);
    let ago_w = 4u16;
    let status_w = if w >= 76 {
        16u16
    } else if w >= 34 {
        8
    } else {
        0
    };
    let branch_w = if w >= 48 { (w / 5).clamp(6, 18) } else { 0 };
    let gaps = 1 + u16::from(status_w > 0) + u16::from(branch_w > 0);
    let name_w = w.saturating_sub(2 + branch_w + status_w + ago_w + gaps);
    let header = list_h >= 6 && branch_w > 0;
    let rule = header && list_h >= 9;
    // If the list stays short, the selected project's card fills the space below.
    let needed = u16::from(header) + u16::from(rule) + vis.len() as u16;
    let card_h = if app.projects_loaded && !vis.is_empty() && footer_h == 3 && w >= 40 && list_h >= needed + CARD_MIN {
        list_h - needed
    } else {
        0
    };
    list_h -= card_h;

    if !app.projects_loaded {
        hud::put(
            buf,
            x,
            y,
            &format!("{} looking for git repositories…", hud::spinner(app.started.elapsed().as_millis())),
            th.dim(),
            w,
        );
    } else if total == 0 {
        hud::put(buf, x, y, "No git repositories found.", th.text(), w);
        hud::put_spans(
            buf,
            x,
            y + 1,
            &[("press ", th.dim()), ("a", th.accent_bold()), (" to add a project or folder", th.dim())],
            w,
        );
    } else if vis.is_empty() {
        hud::put(buf, x, y, &format!("Nothing matches “{}”", app.bridge.filter), th.dim(), w);
    } else {
        // Column headers above the list when there is room, dimmed.
        if header {
            let head = th.dim().add_modifier(Modifier::BOLD);
            let mut hx = x + 1;
            hud::put(buf, hx, y, "PROJECT", head, name_w);
            hx += name_w + 1;
            hud::put(buf, hx, y, "BRANCH", head, branch_w);
            hx += branch_w + 1;
            if status_w > 0 {
                hud::put(buf, hx, y, "STATUS", head, status_w);
                hx += status_w + 1;
            }
            hud::put_right(buf, hx + ago_w, y, "LAST", head);
            y += 1;
            list_h -= 1;
            if rule {
                hud::hline(buf, x, y, w, "─", th.line());
                y += 1;
                list_h -= 1;
            }
        }
        let sel = app.bridge.proj_sel.min(vis.len() - 1);
        let offset = if sel >= list_h as usize { sel + 1 - list_h as usize } else { 0 };
        let glide = app.fx.glide("projects", sel);
        for (row, idx) in vis.iter().enumerate().skip(offset).take(list_h as usize) {
            let p = &app.projects[*idx];
            let ry = y + (row - offset) as u16;
            let selected = row == sel;
            let bgc = glide.bg(row, th.bg, th.sel_strong());
            if bgc != th.bg {
                hud::set_bg_row(buf, inner.x, ry, inner.width, bgc);
            }
            if selected {
                hud::put(buf, inner.x, ry, "▌", Style::default().fg(th.accent).bg(bgc), 1);
            }
            let st = |s: Style| s.bg(bgc);
            let mut cx = x + 1;
            let name_style = if selected { th.accent_bold() } else { th.text() };
            // Open Claude/Codex session in the project: next to the name, ◆ when it needs attention, ● while running.
            let sessions = agents.get(&p.path).map_or(&[][..], Vec::as_slice);
            let marker =
                sessions.iter().map(|(_, st)| *st).max_by_key(AgentState::urgency).map(|st| state_glyph(st, th));
            let pinned = app.ui_state.is_pinned(&p.path);
            let pin_w = if pinned { 2 } else { 0 };
            let text_w = if marker.is_some() { name_w.saturating_sub(2) } else { name_w }.saturating_sub(pin_w);
            if pinned {
                cx = hud::put(buf, cx, ry, "★ ", st(Style::default().fg(th.accent)), 2);
            }
            let name = util::truncate(&p.name, text_w as usize);
            let end = x + 1 + name_w;
            cx = hud::put(buf, cx, ry, &name, st(name_style), text_w);
            if let Some((glyph, color)) = marker {
                cx = hud::put(buf, cx, ry, " ", st(th.dim()), 1);
                cx = hud::put(buf, cx, ry, glyph, st(Style::default().fg(color)), 1);
            }
            cx = hud::put(
                buf,
                cx,
                ry,
                &" ".repeat(end.saturating_sub(cx) as usize),
                st(th.dim()),
                end.saturating_sub(cx),
            );
            if branch_w > 0 {
                cx = hud::put(buf, cx, ry, " ", st(th.dim()), 1);
                let branch = p.branch.clone().unwrap_or_default();
                cx = hud::put(
                    buf,
                    cx,
                    ry,
                    &util::pad_right(&branch, branch_w as usize),
                    // On the tinted selected row the dim colors would lose contrast.
                    st(Style::default().fg(if selected { th.fg } else { th.accent_dim })),
                    branch_w,
                );
            }
            if status_w > 0 {
                cx = hud::put(buf, cx, ry, " ", st(th.dim()), 1);
                let end = cx + status_w;
                for (text, color) in git_badge(p, status_w >= 16, th) {
                    cx = hud::put(buf, cx, ry, &text, st(Style::default().fg(color)), end.saturating_sub(cx));
                }
                cx = hud::put(
                    buf,
                    cx,
                    ry,
                    &" ".repeat(end.saturating_sub(cx) as usize),
                    st(th.dim()),
                    end.saturating_sub(cx),
                );
            }
            cx = hud::put(buf, cx, ry, " ", st(th.dim()), 1);
            let a = p.last_active.map(ago).unwrap_or_default();
            let ago_style = if selected { Style::default().fg(th.fg) } else { th.dim() };
            hud::put(buf, cx, ry, &util::pad_left(&a, ago_w as usize), st(ago_style), ago_w);
            hits.push((Rect::new(inner.x, ry, inner.width, 1), Hit::Project(row)));
            // Quick actions on the right end of the selected or hovered row. They only cover the
            // "LAST" column (and the padding beside it); the git status stays visible.
            // → focuses them from the keyboard (`BridgeState::proj_act`).
            let hovered = app.hover.is_some_and(|(hx, hy)| hy == ry && hx >= inner.x && hx < inner.right());
            if (hovered || selected) && app.overlay.is_none() && w >= 40 {
                let acts = [(if pinned { " ★ " } else { " ☆ " }, ProjectAct::Pin), (" ⋯ ", ProjectAct::More)];
                let total: u16 = acts.iter().map(|(l, _)| util::width(l) as u16).sum();
                let mut ax = (x + w).saturating_sub(total).max(inner.x);
                for (label, act) in acts {
                    let lw = util::width(label) as u16;
                    let focused = selected && app.bridge.proj_act == Some(act);
                    let style = if focused {
                        Style::default().fg(th.bg).bg(th.accent).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(th.fg).bg(th.raised)
                    };
                    hud::put(buf, ax, ry, label, style, lw);
                    hits.push((Rect::new(ax, ry, lw, 1), Hit::ProjectAct(row, act)));
                    ax += lw;
                }
            }
        }
    }

    let (mut card_used, mut card_top) = (0u16, 0u16);
    if card_h > 0
        && let Some(p) = app.selected_project()
    {
        card_used = project_card(buf, Rect::new(inner.x, y + list_h, inner.width, card_h), app, p, hits);
        card_top = y + list_h;
    }
    if footer_h == 0 {
        return;
    }
    let by = inner.bottom() - 1;
    // A faint rule above the summary anchors it to the action bar when the card leaves a gap.
    if footer_h == 3 && card_h > 0 && by >= 3 && by - 3 > card_used && by - 3 > card_top {
        hud::hline(buf, x, by - 3, w, "─", th.line());
    }
    if footer_h == 3 {
        // Selected project: open AI sessions, the git status sentence and the last
        // commit (or the folder path when the card is open, since the card shows it).
        if let Some(p) = app.selected_project() {
            let detail = match p.git.as_ref().and_then(|g| g.last_subject.clone().map(|s| (s, g.last_commit))) {
                _ if card_h > 0 => util::tilde(&p.path),
                Some((s, Some(t))) => format!("last commit: {s} · {} ago", ago_ts(t)),
                Some((s, None)) => format!("last commit: {s}"),
                None => util::tilde(&p.path),
            };
            let mut parts: Vec<(String, Style, Option<Hit>)> =
                git_summary(p, th).into_iter().map(|(t, c)| (t, Style::default().fg(c), None)).collect();
            for &(kind, state) in agents.get(&p.path).into_iter().flatten() {
                let (_, color) = state_glyph(state, th);
                parts.push((format!("{kind} {}", state.label()), Style::default().fg(color), None));
            }
            // Without the card (a small terminal), the pull request and the ports go here.
            if card_h == 0 {
                parts.extend(footer_live_parts(app, p));
            }
            parts.push((detail, th.dim(), None));
            let mut cx = x + 1;
            let mut drawn = false;
            for (text, style, hit) in &parts {
                let sep = if drawn { 3 } else { 0 };
                let room = (x + w).saturating_sub(cx);
                // A separator only goes before a part that still gets a few readable cells.
                if drawn && room < 3 + 4 {
                    break;
                }
                // A part with a hit (the pull request, a port) is drawn whole or skipped: a cut one
                // would be a clickable stub. Plain text parts are cut to fit.
                if hit.is_some() && util::width(text) as u16 + sep > room {
                    continue;
                }
                if drawn {
                    cx = hud::put(buf, cx, by - 2, " · ", th.dim(), room);
                }
                let room = (x + w).saturating_sub(cx);
                let start = cx;
                cx = hud::put(buf, cx, by - 2, &util::truncate(text, room as usize), *style, room);
                if let Some(hit) = hit
                    && cx > start
                {
                    hits.push((Rect::new(start, by - 2, cx - start, 1), hit.clone()));
                }
                drawn = true;
            }
        }
    }
    let end = x + w;
    let mut cx = x;
    let mut button = |cx: &mut u16, key: &str, label: &str, hit: Hit, enabled: bool| {
        let need = (util::width(key) + util::width(label) + 4) as u16;
        if *cx + need > end {
            return;
        }
        let start = *cx;
        *cx = hud::button(buf, *cx, by, key, label, th, enabled);
        hits.push((Rect::new(start, by, *cx - start, 1), hit));
        *cx += 1;
    };
    button(&mut cx, "⏎", "Open", Hit::OpenSelected, true);
    for (i, l) in app.quick_launchers() {
        button(&mut cx, &l.key, &capitalize(&l.name), Hit::Launcher(i), true);
    }
    button(&mut cx, "o", "Folder", Hit::OpenFiles, true);
}

/// "/ search" on the Projects panel's top border, just left of the count ("/ search · 6"). A click
/// starts the filter, as "/" does. It is left out when the title and the count leave no room.
fn search_label(buf: &mut Buffer, area: Rect, tag: &str, th: &Theme, hits: &mut Vec<(Rect, Hit)>) {
    // The frame's title and count positions (see `hud::frame_outline`).
    let max_title = area.width.saturating_sub(6) as usize;
    let title_end = area.left() + 3 + util::width(&util::truncate("Projects", max_title)) as u16;
    if area.width <= 17 {
        return;
    }
    let count = format!(" {} ", util::truncate(tag, area.width.saturating_sub(16) as usize));
    let count_start = area.right().saturating_sub(2 + util::width(&count) as u16);
    // A space on each side of the label keeps it apart from the border dashes and the count.
    let text = " / search ·";
    let start = count_start.saturating_sub(util::width(text) as u16);
    if start < title_end + 2 {
        return;
    }
    hud::put_right(buf, count_start, area.top(), text, th.dim());
    hits.push((Rect::new(start + 1, area.top(), util::width("/ search") as u16, 1), Hit::HomeSearch));
}

/// Minimum height of a project card (separator + title + a few rows).
const CARD_MIN: u16 = 6;

/// The selected project's card: recent commits and changed files. The data comes
/// from the `git status` / `git log` output the project scan already runs.
fn project_card(buf: &mut Buffer, area: Rect, app: &App, p: &Project, hits: &mut Vec<(Rect, Hit)>) -> u16 {
    let th = &app.theme;
    let x = area.x + 1;
    let w = area.width.saturating_sub(2);
    let mut y = area.y;
    // Separator: "── noble-rs ─────────".
    hud::hline(buf, area.x, y, area.width, "─", Style::default().fg(th.line));
    let title = format!(" {} ", util::truncate(&p.name, w.saturating_sub(4) as usize));
    hud::put(buf, x + 1, y, &title, th.accent_bold(), w);
    y += 1;
    // The spacer row under the title carries the pull request and the ports.
    live_row(buf, x, y, w, app, p, hits);
    // The Claude line comes under the spacer row (one row, when the project has a conversation), then the TASKS
    // section when there is room; the rest of the card follows them.
    let claude_used = claude_line(buf, Rect::new(x, y + 1, w, area.bottom().saturating_sub(y + 1)), app, p, hits);
    let tasks_top = y + 1 + claude_used;
    let tasks_used = task_section(buf, x, tasks_top, w, area.bottom().saturating_sub(tasks_top), app, p, hits);
    let rows = area.bottom().saturating_sub(y).saturating_sub(claude_used + tasks_used);
    y += 1 + claude_used + tasks_used;
    let Some(g) = p.git.as_ref() else {
        let text = if p.repo { "checking git status…" } else { "not a git repository" };
        hud::put(buf, x, y, text, th.dim(), w);
        return y;
    };
    // Files in columns; the column width follows the longest path.
    let longest = g.changes.iter().map(|(_, f)| util::width(f)).max().unwrap_or(0) as u16;
    let col_w = (longest + 5).clamp(16, w.max(16));
    let cols = (w / col_w).clamp(1, 4) as usize;
    // A "+N more" cell for files missing from the list (e.g. the `MAX_CHANGES` cap).
    let entries = g.changes.len() + usize::from(g.dirty as usize > g.changes.len());
    let change_want = entries.div_ceil(cols).max(1) as u16;
    // Give commits room first; files keep at least 3 rows (or as many as needed).
    let commit_rows = (g.commits.len() as u16).min(rows.saturating_sub(4 + change_want.min(3)));
    let change_rows = rows.saturating_sub(if commit_rows > 0 { commit_rows + 2 } else { 0 } + 2).min(change_want);

    if commit_rows > 0 {
        section_header(buf, x, y, w, "RECENT COMMITS", "", th);
        y += 1;
        for c in g.commits.iter().take(commit_rows as usize) {
            let right = format!("{:>4}  {}", ago_ts(c.time), util::truncate(&c.author, 12));
            let right_w = util::width(&right) as u16;
            let mut cx = hud::put(buf, x, y, &c.hash, Style::default().fg(th.accent_dim), 8);
            cx += 1;
            let room = (x + w).saturating_sub(cx + right_w + 2);
            hud::put(buf, cx, y, &util::truncate(&c.subject, room as usize), th.text(), room);
            hud::put_right(buf, x + w, y, &right, th.dim());
            y += 1;
        }
        y += 1;
    }
    if change_rows == 0 || y >= area.bottom() {
        return y;
    }
    let count = if g.dirty > 0 { g.dirty.to_string() } else { String::new() };
    section_header(buf, x, y, w, "CHANGES", &count, th);
    y += 1;
    if g.dirty == 0 {
        hud::put(buf, x, y, "✓ working tree clean", Style::default().fg(th.ok), w);
        return y;
    }
    let slots = change_rows as usize * cols;
    let all_fit = g.changes.len() >= g.dirty as usize && g.changes.len() <= slots;
    let shown = if all_fit { g.changes.len() } else { g.changes.len().min(slots.saturating_sub(1)) };
    let hidden = (g.dirty as usize).saturating_sub(shown);
    for (i, (code, file)) in g.changes.iter().take(shown).enumerate() {
        let (row, col) = (i % change_rows as usize, i / change_rows as usize);
        let cx = x + col as u16 * col_w;
        let cy = y + row as u16;
        let (mark, color) = change_mark(code, th);
        hud::put(buf, cx, cy, mark, Style::default().fg(color), 1);
        hud::put(buf, cx + 2, cy, &util::truncate_left(file, col_w.saturating_sub(4) as usize), th.text(), col_w - 3);
    }
    if hidden > 0 {
        let i = shown;
        let (row, col) = (i % change_rows as usize, i / change_rows as usize);
        let more = format!("+{hidden} more");
        hud::put(buf, x + col as u16 * col_w, y + row as u16, &more, th.dim(), col_w);
    }
    y + change_rows
}

/// The Claude line of the selected project's card: `↻ claude · 2h ago`, when the project has a Claude conversation
/// folder and a `claude` launcher is installed. A click continues that conversation (`Hit::ClaudeContinue`). `line` is
/// the row it would take (drawn only when it has a row). Returns the rows used (0 or 1).
fn claude_line(buf: &mut Buffer, line: Rect, app: &App, p: &Project, hits: &mut Vec<(Rect, Hit)>) -> u16 {
    let Some(when) = p.claude else { return 0 };
    if line.height == 0 || line.width < 12 || app.claude_launcher().is_none() {
        return 0;
    }
    let th = &app.theme;
    let since = ago(when);
    let text = if since == "now" { "↻ claude · just now".to_string() } else { format!("↻ claude · {since} ago") };
    hud::put(buf, line.x, line.y, &text, th.accent(), line.width);
    let width = (util::width(&text) as u16).min(line.width);
    hits.push((Rect::new(line.x, line.y, width, 1), Hit::ClaudeContinue));
    1
}

/// Most rows the TASKS chips take (they wrap onto a second row).
const TASK_ROWS: u16 = 2;

/// The TASKS section of the selected project's card: a header and the quick tasks as clickable chips
/// (`▸ npm run dev`), wrapped to at most `TASK_ROWS` rows. It is drawn only when three rows stay free for
/// the card below it. Returns the rows it used (0 when not drawn).
#[allow(clippy::too_many_arguments)]
fn task_section(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    w: u16,
    room: u16,
    app: &App,
    p: &Project,
    hits: &mut Vec<(Rect, Hit)>,
) -> u16 {
    let chip_rows = room.saturating_sub(4).min(TASK_ROWS);
    if p.tasks.is_empty() || chip_rows == 0 || w < 12 {
        return 0;
    }
    let th = &app.theme;
    section_header(buf, x, y, w, "TASKS", "", th);
    let right = x + w;
    let (mut cx, mut row) = (x, 0u16);
    let mut shown = 0;
    for (i, command) in p.tasks.iter().enumerate() {
        // A chip is " ▸ command ": the command is cut to the width, and a chip that does not fit wraps.
        let label = util::truncate(command, w.saturating_sub(4) as usize);
        let need = util::width(&label) as u16 + 4;
        if cx + need > right {
            if row + 1 >= chip_rows {
                break;
            }
            row += 1;
            cx = x;
        }
        let ry = y + 1 + row;
        let start = cx;
        cx = hud::button(buf, cx, ry, "▸", &label, th, true);
        hits.push((Rect::new(start, ry, cx - start, 1), Hit::ProjectTask(i)));
        cx += 1;
        shown += 1;
    }
    let hidden = p.tasks.len() - shown;
    if hidden > 0 {
        let more = format!("+{hidden} more");
        if cx + util::width(&more) as u16 <= right {
            hud::put(buf, cx, y + 1 + row, &more, th.dim(), right - cx);
        }
    }
    row + 2
}

/// The spacer row under a card's title: the selected project's pull request with its CI (`PR #42 open ·
/// ✓ CI 12/12`), then its listening ports (`⇄ :3000 :5173 +1`). Each part is drawn whole or left out; the
/// `+N` counts the ports that did not fit. Both are clickable.
fn live_row(buf: &mut Buffer, x: u16, y: u16, w: u16, app: &App, p: &Project, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    let right = x + w;
    let mut cx = x;
    let mut room = w as usize;
    if let Some(pr) = app.project_pr(p) {
        let head = pr_head(pr, th);
        let head_w = spans_width(&head);
        let mut segment = head.clone();
        let mut seg_w = head_w;
        if let Some((text, color)) = ci_part(pr.checks, th) {
            let ci_w = 3 + util::width(&text);
            if head_w + ci_w <= room {
                segment.push((" · ".into(), th.dim()));
                segment.push((text, Style::default().fg(color)));
                seg_w += ci_w;
            }
        }
        if seg_w <= room {
            let refs: Vec<(&str, Style)> = segment.iter().map(|(t, st)| (t.as_str(), *st)).collect();
            cx = hud::put_spans(buf, cx, y, &refs, seg_w as u16);
            hits.push((Rect::new(x, y, seg_w as u16, 1), Hit::ProjectPr));
            room -= seg_w;
        }
    }
    let ports = app.project_ports(p);
    if !ports.is_empty() {
        let sep = if cx > x { 3 } else { 0 };
        if let Some(shown) = ports_shown(ports, room.saturating_sub(sep)) {
            if sep > 0 {
                cx = hud::put(buf, cx, y, " · ", th.dim(), right.saturating_sub(cx));
            }
            cx = hud::put(buf, cx, y, "⇄", th.accent2(), right.saturating_sub(cx));
            for port in &ports[..shown] {
                let token = format!(":{port}");
                let start = cx + 1;
                cx = hud::put(buf, cx, y, &format!(" {token}"), th.accent(), right.saturating_sub(cx));
                hits.push((Rect::new(start, y, util::width(&token) as u16, 1), Hit::Port(*port)));
            }
            if shown < ports.len() {
                cx = hud::put(buf, cx, y, &format!(" +{}", ports.len() - shown), th.dim(), right.saturating_sub(cx));
            }
        }
    }
    let _ = cx;
}

/// The ports as `⇄ :3000 :5173` with the first `shown` ports and `+N` for the rest.
fn ports_text(ports: &[u16], shown: usize) -> String {
    let mut text = String::from("⇄");
    for port in &ports[..shown] {
        text.push_str(&format!(" :{port}"));
    }
    if shown < ports.len() {
        text.push_str(&format!(" +{}", ports.len() - shown));
    }
    text
}

/// How many ports fit in `avail` columns (the most that do); `None` when not even the mark and `+N` fit.
fn ports_shown(ports: &[u16], avail: usize) -> Option<usize> {
    (0..=ports.len()).rev().find(|&shown| util::width(&ports_text(ports, shown)) <= avail)
}

/// The pull request's state and number: `PR #42 open`; the state is dim unless the pull request is open.
fn pr_head(pr: &PrInfo, th: &Theme) -> Vec<(String, Style)> {
    let (label, dim) = match pr.state {
        PrState::Open => ("open", false),
        PrState::Draft => ("draft", true),
        PrState::Merged => ("merged", true),
        PrState::Closed => ("closed", true),
    };
    let label_style = if dim { th.dim() } else { th.text() };
    vec![(format!("PR #{}", pr.number), th.accent_bold()), (format!(" {label}"), label_style)]
}

/// The CI part of a pull request: `✗ CI 2 failing`, `● CI 3 running` or `✓ CI 12/12`; `None` without checks.
fn ci_part(checks: Checks, th: &Theme) -> Option<(String, ratatui::style::Color)> {
    let total = checks.passed + checks.failed + checks.pending;
    if total == 0 {
        return None;
    }
    Some(if checks.failed > 0 {
        (format!("✗ CI {} failing", checks.failed), th.crit)
    } else if checks.pending > 0 {
        (format!("● CI {} running", checks.pending), th.accent2)
    } else {
        (format!("✓ CI {}/{}", checks.passed, total), th.ok)
    })
}

/// Columns of styled text parts written one after another.
fn spans_width(parts: &[(String, Style)]) -> usize {
    parts.iter().map(|(t, _)| util::width(t)).sum()
}

/// The pull request and the ports as footer parts, for a screen without the card: `PR #42 ✓`, then up to
/// three ports (each clickable) and `+N`.
fn footer_live_parts(app: &App, p: &Project) -> Vec<(String, Style, Option<Hit>)> {
    let th = &app.theme;
    let mut parts = Vec::new();
    if let Some(pr) = app.project_pr(p) {
        let checks = pr.checks;
        let (mark, color) = if checks.failed > 0 {
            (" ✗", th.crit)
        } else if checks.pending > 0 {
            (" ●", th.accent2)
        } else if checks.passed > 0 {
            (" ✓", th.ok)
        } else {
            ("", th.accent)
        };
        parts.push((format!("PR #{}{mark}", pr.number), Style::default().fg(color), Some(Hit::ProjectPr)));
    }
    let ports = app.project_ports(p);
    for port in ports.iter().take(3) {
        parts.push((format!(":{port}"), th.accent(), Some(Hit::Port(*port))));
    }
    if ports.len() > 3 {
        parts.push((format!("+{}", ports.len() - 3), th.dim(), None));
    }
    parts
}

/// Section header: bold dim label, an optional warn count and a faint rule to the edge (as in Settings).
fn section_header(buf: &mut Buffer, x: u16, y: u16, w: u16, label: &str, count: &str, th: &Theme) {
    let mut cx = hud::put(buf, x, y, label, th.dim().add_modifier(Modifier::BOLD), w);
    if !count.is_empty() {
        cx = hud::put(buf, cx + 1, y, count, Style::default().fg(th.warn).add_modifier(Modifier::BOLD), 6);
    }
    let start = cx + 1;
    hud::hline(buf, start, y, (x + w).saturating_sub(start), "─", th.line());
}

/// Single letter and color from a porcelain status code: M changed, A added, D deleted, ? new.
fn change_mark(code: &str, th: &Theme) -> (&'static str, ratatui::style::Color) {
    match code.trim().chars().next().unwrap_or(' ') {
        '?' => ("?", th.accent),
        'A' => ("A", th.ok),
        'D' => ("D", th.crit),
        'R' => ("R", th.accent2),
        'U' => ("U", th.crit),
        _ => ("M", th.warn),
    }
}

fn plural(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Short git status on a project row: "● 3 changed ↑1 ↓2", "✓ clean". In a
/// narrow column the words are dropped ("● 3 ↑1").
fn git_badge(p: &crate::projects::Project, wide: bool, th: &Theme) -> Vec<(String, ratatui::style::Color)> {
    if !p.repo {
        // A folder added by hand that is not a git repository: nothing to check.
        return vec![((if wide { "no git" } else { "–" }).into(), th.dim)];
    }
    let Some(g) = p.git.as_ref() else { return vec![("…".into(), th.dim)] };
    let mut v = Vec::new();
    if g.dirty > 0 {
        v.push((if wide { format!("● {} changed", g.dirty) } else { format!("● {}", g.dirty) }, th.warn));
    } else if wide && g.ahead == 0 && g.behind == 0 {
        v.push(("✓ clean".into(), th.ok));
    } else {
        v.push(("✓".into(), th.ok));
    }
    if g.ahead > 0 {
        v.push((format!(" ↑{}", g.ahead), th.accent2));
    }
    if g.behind > 0 {
        v.push((format!(" ↓{}", g.behind), th.accent));
    }
    v
}

/// The selected project's git status, as a sentence.
fn git_summary(p: &crate::projects::Project, th: &Theme) -> Vec<(String, ratatui::style::Color)> {
    if !p.repo {
        return vec![("not a git repository".into(), th.dim)];
    }
    let Some(g) = p.git.as_ref() else { return vec![("checking git status…".into(), th.dim)] };
    let mut v = Vec::new();
    if g.dirty > 0 {
        let mut text = plural(g.dirty, "uncommitted change", "uncommitted changes");
        if g.untracked > 0 {
            text.push_str(&format!(" ({} new)", g.untracked));
        }
        v.push((text, th.warn));
    } else {
        v.push(("clean".into(), th.ok));
    }
    if g.ahead > 0 {
        v.push((plural(g.ahead, "commit to push", "commits to push"), th.accent2));
    }
    if g.behind > 0 {
        v.push((plural(g.behind, "commit to pull", "commits to pull"), th.accent));
    }
    v
}

/// Providers with a live session (credentials found).
fn signed_in(app: &App) -> Vec<&ProviderState> {
    app.quota_providers().collect()
}

/// State marker and color: ◆ needs attention, ✗ failed, ○ your turn, ● running.
fn state_glyph(state: AgentState, th: &crate::theme::Theme) -> (&'static str, ratatui::style::Color) {
    match state {
        AgentState::NeedsYou => ("◆", th.warn),
        AgentState::Error => ("✗", th.crit),
        AgentState::Idle => ("○", th.ok),
        AgentState::Working | AgentState::Running => ("●", th.accent2),
    }
}

/// Maximum rows shown in the session list.
const MAX_SESSION_ROWS: usize = 4;

fn ai_height(app: &App, list: &[&ProviderState], sessions: usize) -> u16 {
    let mut h: u16 = 2;
    if sessions > 0 {
        h += sessions.min(MAX_SESSION_ROWS) as u16 + 1 + u16::from(sessions > MAX_SESSION_ROWS);
    }
    for p in list {
        let rows = p.usage.as_ref().map(|u| u.windows.len().min(3) as u16).unwrap_or(1).max(1);
        let note = u16::from(matches!(p.status, Status::Error(_) | Status::Expired));
        let pace = u16::from(pace_line(app, p).is_some());
        h += 1 + rows + note + pace + 1;
    }
    h
}

fn reset_in(ts: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    if ts <= now {
        return "now".into();
    }
    util::fmt_duration(Duration::from_secs((ts - now) as u64)).replace(' ', "")
}

/// "At this pace 5h fills in 1h20m": the warning text shown when the 5 hour
/// window would fill before it resets. No estimate is made for the weekly window.
fn pace_line(app: &App, p: &ProviderState) -> Option<String> {
    let u = p.usage.as_ref()?;
    let now = chrono::Utc::now().timestamp();
    let w = u.windows.iter().find(|w| w.label == "5H")?;
    let reset = w.resets_at?;
    let key = crate::store::UsageHistory::key(p.id, &w.label);
    let eta = app.usage_history.pace_eta(&key, reset.saturating_sub(5 * 3600), now, w.used)?;
    (now.saturating_add(eta.min(i64::MAX as u64) as i64) < reset).then(|| {
        let name = crate::ai::window_name(&w.label);
        format!("▲ {name} full in ~{} at this pace", util::fmt_duration(Duration::from_secs(eta)))
    })
}

/// Age of cached quota numbers ("~5m", "~2h"); `None` while they are fresh, missing or
/// younger than a minute.
fn cache_age(p: &ProviderState) -> Option<String> {
    if !p.is_stale() || p.usage.as_ref().is_none_or(|u| u.windows.is_empty()) {
        return None;
    }
    let age = ago_ts(p.fetched_at?);
    (age != "now").then(|| format!("~{age}"))
}

/// One line for an expired login that fits `w` cells: "expired · run claude to refresh", or without
/// "to refresh" when the panel is narrow.
fn expired_hint(p: &ProviderState, w: u16) -> String {
    let full = format!("expired · run {} to refresh", p.login_hint);
    if util::width(&full) as u16 <= w { full } else { format!("expired · run {}", p.login_hint) }
}

fn ai_panel(
    buf: &mut Buffer,
    area: Rect,
    app: &App,
    list: &[&ProviderState],
    sessions: &[crate::app::AgentSession],
    hits: &mut Vec<(Rect, Hit)>,
) {
    let th = &app.theme;
    hits.push((area, Hit::AiRefresh));
    let newest = list.iter().filter(|p| p.status == Status::Ok).filter_map(|p| p.fetched_at).max();
    let tag = newest.map(|t| format!("{} ago", ago_ts(t))).unwrap_or_default();
    let inner = hud::frame(buf, area, "AI usage", &tag, false, th);
    if inner.height == 0 || inner.width < 16 {
        return;
    }
    let ms = app.started.elapsed().as_millis();
    let x = inner.x + 1;
    let w = inner.width.saturating_sub(2);
    let bottom = inner.bottom();
    let mut y = inner.y;
    // Open AI sessions: clicking one jumps to that tab.
    if !sessions.is_empty() {
        for sess in sessions.iter().take(MAX_SESSION_ROWS) {
            if y >= bottom {
                break;
            }
            let (glyph, color) = state_glyph(sess.state, th);
            let label = sess.state.label();
            // A session that just changed state lights its row up in its color, fading back.
            if let Some(t) = app.fx.raw(crate::app::fx::Fx::Agent(sess.pane)) {
                let glow = crate::theme::Theme::mix(th.bg, color, 0.35 * (1.0 - t));
                hud::set_bg_row(buf, inner.x, y, inner.width, glow);
            }
            let cx = hud::put(buf, x, y, glyph, Style::default().fg(color), 1);
            let cx = hud::put(buf, cx + 1, y, sess.kind, th.text().add_modifier(Modifier::BOLD), w);
            let room = (x + w).saturating_sub(cx + util::width(label) as u16 + 4);
            hud::put(buf, cx, y, &format!(" · {}", util::truncate(&sess.place, room as usize)), th.dim(), room + 3);
            hud::put_right(buf, x + w, y, label, Style::default().fg(color));
            hits.push((Rect::new(inner.x, y, inner.width, 1), Hit::Tab(sess.tab)));
            y += 1;
        }
        if sessions.len() > MAX_SESSION_ROWS && y < bottom {
            hud::put(buf, x, y, &format!("+{} more", sessions.len() - MAX_SESSION_ROWS), th.dim(), w);
            y += 1;
        }
        y += 1;
    }
    for p in list {
        if y + 2 > bottom {
            break;
        }
        let ok = p.status == Status::Ok;
        let name_style = th.text().add_modifier(Modifier::BOLD);
        let cx = hud::put(buf, x, y, p.name, name_style, w);
        let spinning = p.status == Status::Loading || p.status == Status::Pending;
        if spinning {
            hud::put(buf, cx + 1, y, hud::spinner(ms), th.dim(), 1);
        }
        let plan_x = match p.usage.as_ref().and_then(|u| u.plan.clone()) {
            Some(plan) => {
                let chip = format!(" {} ", capitalize(&plan.to_lowercase()));
                hud::put_right(buf, x + w, y, &chip, Style::default().fg(th.accent2).bg(th.raised))
            }
            None => x + w,
        };
        // Cached numbers ("~" below): how old they are, e.g. "~5m". Home redraws at least
        // once a minute, so the age keeps up without extra wakeups.
        if let Some(age) = cache_age(p) {
            let ax = cx + if spinning { 3 } else { 1 };
            hud::put(buf, ax, y, &age, th.dim(), plan_x.saturating_sub(ax + 1));
        }
        y += 1;
        match &p.usage {
            Some(u) if !u.windows.is_empty() => {
                for win in u.windows.iter().take(3) {
                    if y >= bottom {
                        break;
                    }
                    // The bar and the number slide to a new value.
                    let pct = app.fx.tween(&format!("ai:{}:{}", p.id, win.label), win.used as f64, 1.0);
                    let color = if ok { th.level(pct) } else { th.dim };
                    hud::put(buf, x, y, &crate::ai::window_name(&win.label), th.dim().add_modifier(Modifier::BOLD), 5);
                    let reset = win.resets_at.map(reset_in).unwrap_or_default();
                    let bar_w = w.saturating_sub(5 + 5 + 7);
                    hud::bar(buf, x + 5, y, bar_w, pct, color, th);
                    let pct_s = format!("{}{}%", if ok { "" } else { "~" }, pct.round() as u64);
                    hud::put(buf, x + 5 + bar_w, y, &util::pad_left(&pct_s, 5), Style::default().fg(color), 5);
                    hud::put(buf, x + 10 + bar_w, y, &util::pad_left(&reset, 7), th.dim(), 7);
                    y += 1;
                }
            }
            _ => {
                let msg = match &p.status {
                    Status::Error(e) => e.clone(),
                    Status::Expired => expired_hint(p, w),
                    _ => "connecting…".into(),
                };
                hud::put(buf, x, y, &msg, th.dim(), w);
                y += 1;
            }
        }
        let has_bars = p.usage.as_ref().is_some_and(|u| !u.windows.is_empty());
        let cached_note = match (&p.status, &p.usage) {
            (Status::Error(e), Some(_)) => Some(e.clone()),
            (Status::Expired, Some(_)) if has_bars => Some(expired_hint(p, w)),
            _ => None,
        };
        if let Some(note) = cached_note
            && y < bottom
        {
            // The age of the cached numbers is on the name row.
            hud::put(buf, x, y, &note, th.dim(), w);
            y += 1;
        }
        if let Some(text) = pace_line(app, p)
            && y < bottom
        {
            hud::put(buf, x, y, &text, Style::default().fg(th.warn), w);
            y += 1;
        }
        y += 1;
    }
}

fn system_panel(buf: &mut Buffer, area: Rect, app: &App, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    hits.push((area, Hit::TabSystem));
    let s = &app.sensors;
    let inner = hud::frame(buf, area, "System", "details ›", false, th);
    if inner.height == 0 || inner.width < 16 {
        return;
    }
    let x = inner.x + 1;
    let w = inner.width.saturating_sub(2);
    // Battery at the very bottom; the other rows stay above it.
    let bat_h = super::battery_rows(app, inner.height);
    if bat_h > 0 {
        let by = inner.bottom() - bat_h;
        super::battery_block(buf, Rect::new(inner.x + 1, by, inner.width.saturating_sub(2), bat_h), app);
    }
    let bottom = inner.bottom() - if bat_h > 0 { bat_h + 1 } else { 0 };
    let mut y = inner.y;
    let Some(last) = &s.last else {
        hud::put(
            buf,
            x,
            y,
            &format!("{} reading sensors…", hud::spinner(app.started.elapsed().as_millis())),
            th.dim(),
            w,
        );
        return;
    };
    let metric = |buf: &mut Buffer, y: u16, label: &str, pct: f64| {
        // The bar and the number slide to a new value.
        let pct = app.fx.tween(&format!("home:{label}"), pct, 1.0);
        hud::put(buf, x, y, label, th.dim().add_modifier(Modifier::BOLD), 5);
        let bar_w = w.saturating_sub(5 + 5);
        hud::bar(buf, x + 5, y, bar_w, pct, th.level(pct), th);
        hud::put_right(buf, x + w, y, &format!("{pct:.0}%"), Style::default().fg(th.level(pct)));
    };
    // The graphs share the remaining height: CPU up to 10 rows, RAM (thin, since it
    // barely changes) up to 3. Fixed rows: CPU, gap, RAM, info, gap, disks, gap, NET,
    // UP. In tight spaces a 1-2 row CPU graph comes before NET/UP.
    let disks = last.disks.len().min(2) as u16;
    let avail = bottom.saturating_sub(y);
    let spare = avail.saturating_sub(8 + disks);
    let mem_g = if spare >= 8 { (spare / 4).clamp(2, 3) } else { 0 };
    let cpu_g = match spare - mem_g {
        n if n >= 2 => n.min(10),
        _ => avail.saturating_sub(5 + disks).min(2),
    };
    if y < bottom {
        metric(buf, y, "CPU", last.cpu as f64);
        y += 1;
    }
    if cpu_g > 0 && y + cpu_g <= bottom {
        let vals: Vec<f64> = s.cpu_hist.iter().map(|v| *v as f64 / 100.0).collect();
        hud::braille_area(buf, Rect::new(x, y, w, cpu_g), &vals, th.accent_dim, th.accent);
        y += cpu_g;
    }
    y += 1;
    if y < bottom {
        metric(buf, y, "RAM", s.mem_pct() as f64);
        y += 1;
    }
    if mem_g > 0 && y + mem_g <= bottom {
        let vals: Vec<f64> = s.mem_hist.iter().map(|v| *v as f64 / 100.0).collect();
        hud::braille_area(buf, Rect::new(x, y, w, mem_g), &vals, th.accent_dim, th.accent2);
        y += mem_g;
    }
    if y < bottom {
        let info = format!("{} of {}", util::fmt_bytes(last.mem_used), util::fmt_bytes(last.mem_total));
        hud::put(buf, x + 5, y, &info, th.dim(), w.saturating_sub(5));
        y += 2;
    }
    for d in last.disks.iter().take(2) {
        if y >= bottom {
            break;
        }
        let pct = if d.total > 0 { d.used as f64 / d.total as f64 * 100.0 } else { 0.0 };
        metric(buf, y, &util::truncate(&d.mount, 4), pct);
        y += 1;
    }
    y += 1;
    if y < bottom {
        hud::put_spans(
            buf,
            x,
            y,
            &[
                ("NET  ", th.dim().add_modifier(Modifier::BOLD)),
                ("↓ ", Style::default().fg(th.accent2)),
                (&util::fmt_rate(last.rx_rate), th.text()),
                ("   ↑ ", Style::default().fg(th.accent)),
                (&util::fmt_rate(last.tx_rate), th.text()),
            ],
            w,
        );
        y += 1;
    }
    if y < bottom {
        let up = util::fmt_duration(Duration::from_secs(last.uptime));
        hud::put_spans(buf, x, y, &[("UP   ", th.dim().add_modifier(Modifier::BOLD)), (&up, th.text())], w);
    }
}
