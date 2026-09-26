//! Home screen: clock + projects on the left, AI usage and system summary on the
//! right. Kept simple: every panel has a single job and everything is clickable.

use std::time::{Duration, SystemTime};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::hud;
use crate::ai::{Presence, ProviderState, Status};
use crate::app::{AgentState, App, Hit, ProjectAct};
use crate::projects::GitInfo;
use crate::theme::Theme;
use crate::util;

pub fn draw(buf: &mut Buffer, area: Rect, app: &App, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    if area.width < 24 || area.height < 6 {
        hud::put_center(buf, area, area.y + area.height / 2, "enlarge window", th.dim());
        return;
    }
    let area = if area.width >= 60 { Rect::new(area.x + 1, area.y, area.width - 2, area.height) } else { area };
    let two_col = area.width >= 92 && area.height >= 16;
    if !two_col {
        let hero_h = if area.height >= 14 { 3 } else { 0 };
        if hero_h > 0 {
            hero_compact(buf, Rect::new(area.x, area.y, area.width, hero_h), app);
        }
        projects(buf, Rect::new(area.x, area.y + hero_h, area.width, area.height - hero_h), app, hits);
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
    projects(buf, Rect::new(left.x, left.y + hero_h, left.width, left.height - hero_h), app, hits);

    let signed_in = signed_in(app);
    let sessions = app.all_agent_sessions();
    let room = right.height.saturating_sub(hero_h + 10);
    let shown = !signed_in.is_empty() || !sessions.is_empty();
    let ai_h = if shown { ai_height(app, &signed_in, sessions.len()).min(room + hero_h) } else { 0 };
    // The right column starts under the clock: in line with the project panel on the left.
    let top = right.y + hero_h;
    let avail = right.height.saturating_sub(hero_h);
    if ai_h >= 5 {
        ai_panel(buf, Rect::new(right.x, top, right.width, ai_h), app, &signed_in, &sessions, hits);
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

/// Big clock + greeting (5 rows).
fn hero(buf: &mut Buffer, area: Rect, app: &App) {
    use chrono::Timelike;
    let th = &app.theme;
    let now = chrono::Local::now();
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
    let greet = format!("{}, {}", greeting(now.hour()), capitalize(&app.operator));
    hud::put(buf, tx, y, &greet, th.text().add_modifier(Modifier::BOLD), w);
    hud::put(buf, tx, y + 1, &now.format("%A, %d %B").to_string(), th.dim(), w);
    hud::put(buf, tx, y + 2, &session_line(app), th.dim(), w);
}

/// One-line clock + greeting (narrow windows).
fn hero_compact(buf: &mut Buffer, area: Rect, app: &App) {
    use chrono::Timelike;
    let th = &app.theme;
    let now = chrono::Local::now();
    let fmt = match (app.cfg.general.clock_24h, app.cfg.general.show_seconds && app.live_clock()) {
        (true, true) => "%H:%M:%S",
        (true, false) => "%H:%M",
        (false, true) => "%I:%M:%S %p",
        (false, false) => "%I:%M %p",
    };
    let y = area.y + area.height.saturating_sub(1) / 2;
    let x = area.x + 1;
    let x = hud::put(buf, x, y, &now.format(fmt).to_string(), th.accent_bold(), area.width);
    let greet = format!("   {}, {} · {}", greeting(now.hour()), capitalize(&app.operator), now.format("%a %d %b"));
    hud::put(buf, x, y, &greet, th.dim(), area.right().saturating_sub(x));
}

fn ago(t: SystemTime) -> String {
    util::fmt_ago(SystemTime::now().duration_since(t).unwrap_or_default())
}

fn ago_ts(ts: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    util::fmt_ago(Duration::from_secs(now.saturating_sub(ts).max(0) as u64))
}

fn projects(buf: &mut Buffer, area: Rect, app: &App, hits: &mut Vec<(Rect, Hit)>) {
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
    // If the list stays short, the selected project's card fills the space below.
    let needed = u16::from(header) + vis.len() as u16;
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
            &[("press ", th.dim()), ("a", th.accent_bold()), (" to add a folder where your code lives", th.dim())],
            w,
        );
    } else if vis.is_empty() {
        hud::put(buf, x, y, &format!("Nothing matches “{}”", app.bridge.filter), th.dim(), w);
    } else {
        // Column headers above the list when there is room, dimmed.
        if header {
            let mut hx = x + 1;
            hud::put(buf, hx, y, "PROJECT", th.dim(), name_w);
            hx += name_w + 1;
            hud::put(buf, hx, y, "BRANCH", th.dim(), branch_w);
            hx += branch_w + 1;
            if status_w > 0 {
                hud::put(buf, hx, y, "STATUS", th.dim(), status_w);
                hx += status_w + 1;
            }
            hud::put_right(buf, hx + ago_w, y, "LAST", th.dim());
            y += 1;
            list_h -= 1;
        }
        let sel = app.bridge.proj_sel.min(vis.len() - 1);
        let offset = if sel >= list_h as usize { sel + 1 - list_h as usize } else { 0 };
        for (row, idx) in vis.iter().enumerate().skip(offset).take(list_h as usize) {
            let p = &app.projects[*idx];
            let ry = y + (row - offset) as u16;
            let selected = row == sel;
            let bgc = if selected { th.sel_bg } else { th.bg };
            if selected {
                hud::set_bg_row(buf, inner.x, ry, inner.width, th.sel_bg);
                hud::put(buf, inner.x, ry, "▌", Style::default().fg(th.accent).bg(bgc), 1);
            }
            let st = |s: Style| s.bg(bgc);
            let mut cx = x + 1;
            let name_style = if selected { th.accent_bold() } else { th.text() };
            // Open Claude/Codex session in the project: next to the name, ◆ when it needs attention, ● while running.
            let sessions = app.agent_sessions(&p.path);
            let marker =
                sessions.iter().map(|(_, st)| *st).max_by_key(|st| state_rank(*st)).map(|st| state_glyph(st, th));
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
                cx = hud::put(buf, cx, ry, &util::pad_right(&branch, branch_w as usize), st(th.dim()), branch_w);
            }
            if status_w > 0 {
                cx = hud::put(buf, cx, ry, " ", st(th.dim()), 1);
                let end = cx + status_w;
                for (text, color) in git_badge(p.git.as_ref(), status_w >= 16, th) {
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
            hud::put(buf, cx, ry, &util::pad_left(&a, ago_w as usize), st(th.dim()), ago_w);
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

    if card_h > 0
        && let Some(p) = app.selected_project()
    {
        project_card(buf, Rect::new(inner.x, y + list_h, inner.width, card_h), app, p);
    }
    if footer_h == 0 {
        return;
    }
    let by = inner.bottom() - 1;
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
            let mut cx = x + 1;
            for (text, color) in git_summary(p.git.as_ref(), th) {
                cx = hud::put(buf, cx, by - 2, &text, Style::default().fg(color), (x + w).saturating_sub(cx));
                cx = hud::put(buf, cx, by - 2, " · ", th.dim(), (x + w).saturating_sub(cx));
            }
            for (kind, state) in app.agent_sessions(&p.path) {
                let (_, color) = state_glyph(state, th);
                let text = format!("{kind} {}", state.label());
                cx = hud::put(buf, cx, by - 2, &text, Style::default().fg(color), (x + w).saturating_sub(cx));
                cx = hud::put(buf, cx, by - 2, " · ", th.dim(), (x + w).saturating_sub(cx));
            }
            let room = (x + w).saturating_sub(cx);
            hud::put(buf, cx, by - 2, &util::truncate(&detail, room as usize), th.dim(), room);
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
    if cx + 12 <= end {
        hud::put_right(buf, end, by, "/ search", th.dim());
    }
}

/// Minimum height of a project card (separator + title + a few rows).
const CARD_MIN: u16 = 6;

/// The selected project's card: recent commits and changed files. The data comes
/// from the `git status` / `git log` output the project scan already runs.
fn project_card(buf: &mut Buffer, area: Rect, app: &App, p: &crate::projects::Project) {
    let th = &app.theme;
    let x = area.x + 1;
    let w = area.width.saturating_sub(2);
    let mut y = area.y;
    // Separator: "── noble-rs ─────────".
    hud::hline(buf, area.x, y, area.width, "─", Style::default().fg(th.line));
    hud::put(buf, x + 1, y, &format!(" {} ", util::truncate(&p.name, w.saturating_sub(4) as usize)), th.dim(), w);
    y += 1;
    let rows = area.bottom().saturating_sub(y);
    let Some(g) = p.git.as_ref() else {
        hud::put(buf, x, y + 1, "checking git status…", th.dim(), w);
        return;
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

    y += 1;
    if commit_rows > 0 {
        hud::put(buf, x, y, "RECENT COMMITS", th.dim(), w);
        y += 1;
        for c in g.commits.iter().take(commit_rows as usize) {
            let right = format!("{:>4}  {}", ago_ts(c.time), util::truncate(&c.author, 12));
            let right_w = util::width(&right) as u16;
            let mut cx = hud::put(buf, x, y, &c.hash, Style::default().fg(th.accent2), 8);
            cx += 1;
            let room = (x + w).saturating_sub(cx + right_w + 2);
            hud::put(buf, cx, y, &util::truncate(&c.subject, room as usize), th.text(), room);
            hud::put_right(buf, x + w, y, &right, th.dim());
            y += 1;
        }
        y += 1;
    }
    if change_rows == 0 || y >= area.bottom() {
        return;
    }
    hud::put(buf, x, y, "CHANGES", th.dim(), w);
    if g.dirty > 0 {
        hud::put(buf, x + 8, y, &g.dirty.to_string(), Style::default().fg(th.warn), 6);
    }
    y += 1;
    if g.dirty == 0 {
        hud::put(buf, x, y, "✓ working tree clean", Style::default().fg(th.ok), w);
        return;
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
fn git_badge(git: Option<&GitInfo>, wide: bool, th: &Theme) -> Vec<(String, ratatui::style::Color)> {
    let Some(g) = git else { return vec![("…".into(), th.dim)] };
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
fn git_summary(git: Option<&GitInfo>, th: &Theme) -> Vec<(String, ratatui::style::Color)> {
    let Some(g) = git else { return vec![("checking git status…".into(), th.dim)] };
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
    if !app.cfg.ai.enabled {
        return Vec::new();
    }
    app.ai.iter().filter(|p| p.presence == Presence::Ready && p.status != Status::SignIn).collect()
}

/// State marker and color: ◆ needs attention, ○ your turn, ● running.
fn state_glyph(state: AgentState, th: &crate::theme::Theme) -> (&'static str, ratatui::style::Color) {
    match state {
        AgentState::NeedsYou => ("◆", th.warn),
        AgentState::Idle => ("○", th.ok),
        AgentState::Working | AgentState::Running => ("●", th.accent2),
    }
}

fn state_rank(state: AgentState) -> u8 {
    match state {
        AgentState::NeedsYou => 3,
        AgentState::Idle => 2,
        AgentState::Working => 1,
        AgentState::Running => 0,
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
        let note = u16::from(matches!(p.status, Status::Error(_)));
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
        if p.status == Status::Loading || p.status == Status::Pending {
            hud::put(buf, cx + 1, y, hud::spinner(ms), th.dim(), 1);
        }
        if let Some(plan) = p.usage.as_ref().and_then(|u| u.plan.clone()) {
            hud::put_right(buf, x + w, y, &capitalize(&plan.to_lowercase()), Style::default().fg(th.accent2));
        }
        y += 1;
        match &p.usage {
            Some(u) if !u.windows.is_empty() => {
                for win in u.windows.iter().take(3) {
                    if y >= bottom {
                        break;
                    }
                    let pct = win.used as f64;
                    let color = if ok { th.level(pct) } else { th.dim };
                    hud::put(buf, x, y, &crate::ai::window_name(&win.label), th.dim(), 5);
                    let reset = win.resets_at.map(reset_in).unwrap_or_default();
                    let bar_w = w.saturating_sub(5 + 5 + 7);
                    hud::bar(buf, x + 5, y, bar_w, pct, color, th);
                    let pct_s = format!("{}{}%", if ok { "" } else { "~" }, win.used);
                    hud::put(buf, x + 5 + bar_w, y, &util::pad_left(&pct_s, 5), Style::default().fg(color), 5);
                    hud::put(buf, x + 10 + bar_w, y, &util::pad_left(&reset, 7), th.dim(), 7);
                    y += 1;
                }
            }
            _ => {
                let msg = match &p.status {
                    Status::Error(e) => e.clone(),
                    _ => "connecting…".into(),
                };
                hud::put(buf, x, y, &msg, th.dim(), w);
                y += 1;
            }
        }
        if let (Status::Error(e), Some(_)) = (&p.status, &p.usage)
            && y < bottom
        {
            let when = p.fetched_at.map(ago_ts).unwrap_or_default();
            hud::put(buf, x, y, &format!("{e} · from {when} ago"), th.dim(), w);
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
        hud::put(buf, x, y, label, th.dim(), 5);
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
                ("NET  ", th.dim()),
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
        hud::put_spans(buf, x, y, &[("UP   ", th.dim()), (&up, th.text())], w);
    }
}
