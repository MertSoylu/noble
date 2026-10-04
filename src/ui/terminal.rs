//! Terminal tab: every pane of the split tree is drawn cell by cell from the vt100 screen.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use super::hud;
use crate::app::{AgentBadge, AgentState, App, Hit, LinkHover, SearchState};
use crate::keys::Action;
use crate::term::layout::{Dir, PaneId, frame_rect};
use crate::term::pane::Pane;
use crate::theme::{TermPalette, Theme};
use crate::ui::hud::Outline;
use crate::util;

/// Draws the visible tab; returns the focused pane's cursor position.
pub fn draw(
    buf: &mut Buffer,
    area: Rect,
    app: &App,
    tab_idx: usize,
    hits: &mut Vec<(Rect, Hit)>,
) -> Option<(u16, u16)> {
    let tab = app.tabs.get(tab_idx)?;
    let th = &app.theme;
    // During the fullscreen animation the normal layout stays visible behind it.
    let anim = app.zoom_anim.as_ref().filter(|z| tab.root.contains(z.pane));
    app.fx.observe_focus(tab.id, tab.focus, &tab.panes());
    // A pane just split off unfolds from the divider.
    let split = app.fx.progress(crate::app::fx::Fx::Split(tab.focus)).map(|t| tab.root.growing(tab.focus, t as f32));
    let (rects, dividers) = if tab.zoomed && anim.is_none() {
        (vec![(tab.focus, area)], Vec::new())
    } else {
        split.as_ref().unwrap_or(&tab.root).layout(area)
    };
    let multi = tab.panes().len() > 1;
    let hover = app.hover.filter(|_| app.overlay.is_none());
    let ctx = FrameCtx { multi, zoomed: tab.zoomed, on_battery: app.on_battery(), hover: None };
    let t = &app.cfg.terminal;
    let pal = TermPalette::resolve(&app.term_schemes, &t.colors, &t.background, &t.foreground, th);
    let mut cursor = None;
    // Neighbouring panes share one border line; the frames are joined after they are all drawn.
    let frames: Vec<(PaneId, Outline)> = rects.iter().map(|(id, tile)| (*id, pane_outline(*tile, area))).collect();
    let ctx = FrameCtx { hover: hover.and_then(|at| hovered_pane(&rects, &frames, at)), ..ctx };
    // Title text and buttons go on top of the dividers: a horizontal divider runs along the lower pane's title row.
    let mut on_top = Vec::new();
    for (id, rect) in &frames {
        let Some(pane) = app.panes.get(id) else { continue };
        let focused = *id == tab.focus;
        let agent = app.agent_badge(*id).map(|b| (b, working_glyph(app, &[*id])));
        let inner = pane_frame(buf, *rect, pane, focused, ctx, agent, th, hits, &mut on_top);
        if inner.width == 0 || inner.height == 0 {
            continue;
        }
        hits.push((inner, Hit::Pane { pane: *id, inner }));
        let marks = Marks {
            search: app.search.as_ref().filter(|s| s.pane == *id),
            link: app.link_hover.filter(|l| l.pane == *id),
        };
        let c = pane_content(buf, inner, pane, th, &pal, focused, &marks);
        if let Some(s) = marks.search {
            search_bar(buf, inner, s, th);
        }
        scroll_helpers(buf, inner, pane, marks.search, th, &pal, hits);
        if focused && marks.search.is_none() {
            cursor = c;
        }
    }
    if frames.len() > 1 {
        let all: Vec<Outline> = frames.iter().map(|(_, o)| *o).collect();
        hud::join_frames(buf, &all);
        // The focused frame is drawn in its color all around, also where it shares a line.
        if let Some((_, r)) = frames.iter().find(|(id, _)| *id == tab.focus) {
            // A pane that just took the focus lights its frame up, fading back to the usual tint.
            let color = app
                .fx
                .progress(crate::app::fx::Fx::Focus(tab.focus))
                .map_or(th.accent_dim, |t| crate::theme::Theme::mix(th.accent, th.accent_dim, t));
            hud::tint_frame(buf, *r, color);
        }
    }
    cluster_brackets(buf, &on_top);
    if let Some(z) = anim {
        // The growing/shrinking pane: an intermediate rect between its place and fullscreen.
        let t = crate::app::ease(z.started, crate::app::ZOOM_DURATION);
        let rect = lerp_rect(z.from, z.to, t);
        if let Some(pane) = app.panes.get(&z.pane) {
            hud::clear(buf, rect, th);
            let o = Outline { rect, open_left: rect.x <= area.x, open_right: rect.right() >= area.right() };
            let agent = app.agent_badge(z.pane).map(|b| (b, working_glyph(app, &[z.pane])));
            let ctx = FrameCtx { hover: None, ..ctx };
            let mut buttons = Vec::new();
            let inner = pane_frame(buf, o, pane, true, ctx, agent, th, &mut Vec::new(), &mut buttons);
            cluster_brackets(buf, &buttons);
            if inner.width > 0 && inner.height > 0 {
                pane_content(buf, inner, pane, th, &pal, false, &Marks::default());
            }
        }
        return None;
    }
    for div in dividers {
        hits.push((div.hit, Hit::Divider { tab: tab_idx, div: div.clone() }));
    }
    hits.extend(on_top);
    cursor
}

/// The pane the mouse at `at` is over, for its buttons. Neighbouring frames share a line, so a
/// pane's title row wins (it is also the bottom line of the pane above), then the tile the point is
/// in: a divider belongs to one pane only.
fn hovered_pane(tiles: &[(PaneId, Rect)], frames: &[(PaneId, Outline)], (x, y): (u16, u16)) -> Option<PaneId> {
    let title = frames.iter().find(|(_, o)| o.rect.y == y && x >= o.rect.x && x < o.rect.right());
    let tile = || tiles.iter().find(|(_, t)| t.contains(ratatui::layout::Position { x, y }));
    title.map(|(id, _)| *id).or_else(|| tile().map(|(id, _)| *id))
}

/// A pane's frame: it reaches one cell into a neighbour on its left or top (one shared line),
/// and the window's outer left and right sides stay open (the lines only go between panes).
pub fn pane_outline(tile: Rect, area: Rect) -> Outline {
    let rect = frame_rect(tile, area);
    Outline { rect, open_left: rect.x <= area.x, open_right: rect.right() >= area.right() }
}

/// What the pane's title row shows about the command at its prompt: the live timer while one runs
/// (after `TIMER_AFTER`), otherwise how the last one ended (`✓ 2.4s`, `✗ 1 · 12s`, or `· 12s` for
/// cmd.exe, which reports no exit code). The result stays until the next command starts. Only the
/// ✓ glyph is in the theme's `ok` color: as text it is too faint on the light themes.
pub fn result_badge(pane: &Pane, on_battery: bool, th: &Theme) -> Option<Vec<(String, Style)>> {
    if let Some(elapsed) = pane.running_for() {
        return crate::term::pane::live_timer(elapsed, on_battery).map(|t| vec![(t, th.dim())]);
    }
    let r = pane.last_result?;
    let took = util::fmt_took(r.took);
    Some(match r.code {
        Some(0) => vec![
            ("✓".to_string(), Style::default().fg(th.ok).add_modifier(Modifier::BOLD)),
            (format!(" {took}"), th.text()),
        ],
        Some(code) => vec![(format!("✗ {code} · {took}"), Style::default().fg(th.crit))],
        None => vec![(format!("· {took}"), th.dim())],
    })
}

/// The glyph of a working agent in one of `panes`: the braille spinner while agents spin
/// (`App::agent_spinning`: a live one works in a visible pane and the laptop is not on battery) and
/// one of these panes is live itself (`App::agent_live`), otherwise a static "…".
pub fn working_glyph(app: &App, panes: &[PaneId]) -> &'static str {
    if app.agent_spinning() && panes.iter().any(|p| app.agent_live(*p)) {
        hud::agent_spinner(app.started.elapsed().as_millis())
    } else {
        "…"
    }
}

/// The state glyph of an agent and its style (the tab strip's dot, the start of a pane title).
pub fn agent_glyph(state: AgentState, working: &'static str, th: &Theme) -> (&'static str, Style) {
    match state {
        AgentState::NeedsYou => ("◆", Style::default().fg(th.warn).add_modifier(Modifier::BOLD)),
        AgentState::Idle => ("●", Style::default().fg(th.ok).add_modifier(Modifier::BOLD)),
        AgentState::Working => (working, Style::default().fg(th.accent2).add_modifier(Modifier::BOLD)),
        AgentState::Running => ("○", th.dim()),
    }
}

/// How long an agent has been working, by the minute ("2m", "1h05m"); nothing in the first minute.
pub fn working_for(secs: i64) -> Option<String> {
    let m = secs.max(0) / 60;
    match m {
        0 => None,
        1..60 => Some(format!("{m}m")),
        _ => Some(format!("{}h{:02}m", m / 60, m % 60)),
    }
}

/// The agent part at the start of a pane title in three widths: the whole state
/// ("⠋ claude · working 2m +2", "◆ codex · needs you", "● claude · your turn", "○ claude"), then
/// the glyph and kind, then the glyph alone. `title` is the style of the rest of the title.
pub fn agent_title(
    b: &AgentBadge,
    working: &'static str,
    now: i64,
    title: Style,
    th: &Theme,
) -> [Vec<(String, Style)>; 3] {
    let (glyph, glyph_style) = agent_glyph(b.state, working, th);
    // Only the glyph carries the warning or ok color: as text those are too faint on the light
    // themes. "needs you" stands out in bold instead.
    let (kind_style, state_style) = match b.state {
        AgentState::NeedsYou => (title, th.text().add_modifier(Modifier::BOLD)),
        AgentState::Idle => (title, th.text()),
        AgentState::Working => (title, Style::default().fg(th.accent2)),
        AgentState::Running => (th.dim(), th.dim()),
    };
    let short = vec![(glyph.to_string(), glyph_style), (format!(" {}", b.kind), kind_style)];
    let mut full = short.clone();
    if b.state != AgentState::Running {
        let took = b.working_since.and_then(|t| working_for(now - t)).map(|t| format!(" {t}")).unwrap_or_default();
        full.push((format!(" · {}{took}", b.state.label()), state_style));
    }
    if b.subagents > 0 {
        full.push((format!(" +{}", b.subagents), th.dim()));
    }
    [full, short, vec![(glyph.to_string(), glyph_style)]]
}

fn spans_width(spans: &[(String, Style)]) -> usize {
    spans.iter().map(|(t, _)| util::width(t)).sum()
}

/// The buttons at the right end of a pane's title row, in the order they are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PaneButton {
    SplitRight,
    SplitDown,
    Zoom,
    Close,
}

impl PaneButton {
    /// One cell wide, not emoji: ◫ is a box split by a vertical line (a new pane on the right),
    /// ⊟ one split by a horizontal line (a new pane below; ⊞ counts a split tab's panes on the tab
    /// strip), ⤢ grows the pane to the whole tab and ◱ puts it back.
    pub fn glyph(self, zoomed: bool) -> &'static str {
        match self {
            PaneButton::SplitRight => "◫",
            PaneButton::SplitDown => "⊟",
            PaneButton::Zoom if zoomed => "◱",
            PaneButton::Zoom => "⤢",
            PaneButton::Close => "✕",
        }
    }

    /// What the button does (the status bar while the mouse is on it).
    pub fn what(self, zoomed: bool) -> &'static str {
        match self {
            PaneButton::SplitRight => "split right",
            PaneButton::SplitDown => "split down",
            PaneButton::Zoom if zoomed => "restore pane",
            PaneButton::Zoom => "zoom pane",
            PaneButton::Close => "close pane",
        }
    }

    /// The action with the same effect, whose shortcut the status bar shows.
    pub fn action(self) -> Action {
        match self {
            PaneButton::SplitRight => Action::SplitRight,
            PaneButton::SplitDown => Action::SplitDown,
            PaneButton::Zoom => Action::Zoom,
            PaneButton::Close => Action::ClosePane,
        }
    }

    fn hit(self, pane: PaneId) -> Hit {
        match self {
            PaneButton::SplitRight => Hit::PaneSplit { pane, dir: Dir::Row },
            PaneButton::SplitDown => Hit::PaneSplit { pane, dir: Dir::Col },
            PaneButton::Zoom => Hit::PaneZoom(pane),
            PaneButton::Close => Hit::PaneClose(pane),
        }
    }

    /// The button a click target stands for, and its pane.
    pub fn of_hit(hit: &Hit) -> Option<(PaneButton, PaneId)> {
        match *hit {
            Hit::PaneSplit { pane, dir: Dir::Row } => Some((PaneButton::SplitRight, pane)),
            Hit::PaneSplit { pane, dir: Dir::Col } => Some((PaneButton::SplitDown, pane)),
            Hit::PaneZoom(pane) => Some((PaneButton::Zoom, pane)),
            Hit::PaneClose(pane) => Some((PaneButton::Close, pane)),
            _ => None,
        }
    }
}

/// Widths in cells of what a pane title row would like to show (0 = nothing): `lead` the key lock
/// and the agent's state glyph, `label` the program or agent name (with its leading space after a
/// glyph), `state` the agent's " · working 2m +2", `badge` the command result or timer, `tag`
/// KEYS / ↑N / ZOOM, and `cwd` the folder (drawn after " · ").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TitleParts {
    pub lead: usize,
    pub label: usize,
    pub state: usize,
    pub badge: usize,
    pub tag: usize,
    pub cwd: usize,
}

/// What a title row keeps: the number of buttons, the label's and the folder's widths (either may
/// be cut short with "…"; 0 = dropped) and whether the other parts show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TitleFit {
    pub buttons: usize,
    pub lead: bool,
    pub label: usize,
    pub state: bool,
    pub badge: bool,
    pub tag: bool,
    pub cwd: usize,
}

/// Cells of a title row that are always there: the line and a space before the title, a space and
/// at least one line cell after it, and the corner.
const TITLE_FIXED: usize = 5;

/// Fits a pane title row into `width` cells. The priority is defined here, once: room goes first to
/// the close button (the first of `groups`, the buttons' sizes, most important first: close, then
/// zoom, then the two splits), then the key lock and agent glyph (`lead`), the label's first three
/// cells ("po…"), the other buttons, the rest of the label, the agent's state and the command result
/// badge, the tag, and last the folder, which gets what is left. So a pane always keeps close, its
/// lock and a bit of its name before the other buttons. A part that does not fit leaves its room to
/// the smaller ones after it.
///
/// Row layout: `─ lead label state · cwd ─… badge ─ tag ─┤ ◫ ⊟ ⤢ ✕ ├─╮`: the button cluster
/// takes 2 cells per button plus 4 (`┤`, ` ├` and a line cell), each chip its text plus 3.
pub fn fit_title(width: u16, groups: &[usize], p: TitleParts) -> TitleFit {
    /// Takes `cost` cells for a part that wants to show, if they are left.
    fn claim(room: &mut usize, want: bool, cost: usize) -> bool {
        let ok = want && cost <= *room;
        if ok {
            *room -= cost;
        }
        ok
    }
    let mut room = (width as usize).saturating_sub(TITLE_FIXED);
    let mut fit = TitleFit::default();
    if let Some(&close) = groups.first()
        && claim(&mut room, true, 2 * close + 4)
    {
        fit.buttons = close;
    }
    fit.lead = claim(&mut room, p.lead > 0, p.lead);
    // The label follows the glyph and may be cut short, but not below three cells ("pw…").
    let min = p.label.min(3);
    if claim(&mut room, p.label > 0 && (p.lead == 0 || fit.lead), min) {
        fit.label = min;
    }
    if fit.buttons > 0 {
        for &g in &groups[1..] {
            if !claim(&mut room, true, 2 * g) {
                break;
            }
            fit.buttons += g;
        }
    }
    // The rest of the label: nothing is left for the parts after a cut label.
    if fit.label > 0 {
        let more = (p.label - fit.label).min(room);
        fit.label += more;
        room -= more;
    }
    let whole = fit.label == p.label;
    fit.state = claim(&mut room, whole && p.state > 0, p.state);
    fit.badge = claim(&mut room, p.badge > 0, p.badge + 3);
    fit.tag = claim(&mut room, p.tag > 0, p.tag + 3);
    // The folder follows the whole label after " · " and needs a few cells to say anything.
    if whole && p.cwd > 0 && room >= 3 + p.cwd.min(4) {
        fit.cwd = p.cwd.min(room - 3);
    }
    fit
}

/// What a pane frame needs to know besides the pane: the tab has several panes, the tab is zoomed,
/// running on battery, and the pane under the mouse (`hovered_pane`; `None` while an overlay is open).
#[derive(Clone, Copy, Default)]
struct FrameCtx {
    multi: bool,
    zoomed: bool,
    on_battery: bool,
    hover: Option<PaneId>,
}

/// The title row and lines of a pane; returns the area inside. `agent`: the agent running in the
/// pane and the glyph of a working one (`working_glyph`). The buttons show on the focused pane and
/// on the one under the mouse; elsewhere their place stays a plain line with no click target, so
/// the row never shifts when they appear. Their brackets are drawn by `cluster_brackets` once the
/// frames are joined.
#[allow(clippy::too_many_arguments)]
fn pane_frame(
    buf: &mut Buffer,
    outline: Outline,
    pane: &Pane,
    focused: bool,
    ctx: FrameCtx,
    agent: Option<(AgentBadge, &'static str)>,
    th: &Theme,
    hits: &mut Vec<(Rect, Hit)>,
    on_top: &mut Vec<(Rect, Hit)>,
) -> Rect {
    let rect = outline.rect;
    if rect.width < 4 || rect.height < 3 {
        return Rect::new(rect.x, rect.y, 0, 0);
    }
    let inner = hud::frame_outline(buf, outline, "", "", focused, th);
    let y = rect.y;
    let cwd = util::tilde(&pane.cwd());
    // The lock leads the title so it shows even when the pane is too narrow for the tag or all its
    // buttons (`fit_title` ranks it right after close).
    let lock = if pane.passthrough { "🔒 " } else { "" };
    let scroll = pane.scroll_offset();
    let tag = if pane.passthrough {
        // Key lock: NOBLE's shortcuts go to the app (`keys.passthrough = "lock"`).
        "KEYS".to_string()
    } else if scroll > 0 {
        format!("↑{scroll}")
    } else if ctx.zoomed {
        "ZOOM".to_string()
    } else {
        String::new()
    };
    let title_style = if focused { th.accent_bold() } else { Style::default().fg(th.fg).add_modifier(Modifier::BOLD) };
    // An agent's state replaces the program name: glyph, kind, then " · working 2m +2".
    let mut lead: Vec<(String, Style)> = Vec::new();
    if !lock.is_empty() {
        lead.push((lock.to_string(), title_style));
    }
    let (label, state) = match agent {
        Some((b, working)) => {
            let [full, ..] = agent_title(&b, working, chrono::Utc::now().timestamp(), title_style, th);
            let mut parts = full.into_iter();
            lead.extend(parts.next());
            (parts.next().unwrap_or_default(), parts.collect::<Vec<_>>())
        }
        None => ((pane.label(), title_style), Vec::new()),
    };
    // A pane running an agent shows its state rather than the command's result or live timer.
    let badge = if agent.is_none() { result_badge(pane, ctx.on_battery, th) } else { None };
    let mut groups = vec![1];
    if ctx.multi || ctx.zoomed {
        groups.push(1);
    }
    groups.push(2);
    let parts = TitleParts {
        lead: spans_width(&lead),
        label: util::width(&label.0),
        state: spans_width(&state),
        badge: badge.as_ref().map_or(0, |b| spans_width(b)),
        tag: util::width(&tag),
        cwd: util::width(&cwd),
    };
    let fit = fit_title(rect.width, &groups, parts);

    // The title, left.
    let mut title: Vec<(String, Style)> = Vec::new();
    if fit.lead {
        title.extend(lead);
    }
    if fit.label > 0 {
        title.push((util::truncate(&label.0, fit.label), label.1));
    }
    if fit.state {
        title.extend(state);
    }
    if fit.cwd > 0 {
        title.push((format!(" · {}", util::truncate_left(&cwd, fit.cwd)), title_style));
    }
    // The lock alone keeps no space after it.
    if let Some((t, _)) = title.last_mut() {
        t.truncate(t.trim_end().len());
    }
    let title_w = spans_width(&title) as u16;
    let line = if focused { Style::default().fg(th.accent_dim) } else { th.line() };
    if title_w > 0 {
        let x = hud::put(buf, rect.x + 1, y, " ", line, 1);
        let spans: Vec<(&str, Style)> = title.iter().map(|(t, s)| (t.as_str(), *s)).collect();
        let x = hud::put_spans(buf, x, y, &spans, title_w);
        hud::put(buf, x, y, " ", line, 1);
    }
    // Title row: click to focus, double-click to zoom, right click for the menu (buttons stay on
    // top). The title text itself also stays on top of a divider running along this row.
    hits.push((Rect::new(rect.x, y, rect.width, 1), Hit::PaneTitle(pane.id)));
    if title_w > 0 {
        on_top.push((Rect::new(rect.x + 1, y, title_w + 2, 1), Hit::PaneTitle(pane.id)));
    }

    // The button cluster, right: kept buttons in drawing order, close always last.
    let mut buttons: Vec<PaneButton> =
        [PaneButton::Close, PaneButton::Zoom, PaneButton::SplitRight, PaneButton::SplitDown]
            .into_iter()
            .filter(|b| *b != PaneButton::Zoom || ctx.multi || ctx.zoomed)
            .take(fit.buttons)
            .collect();
    buttons.sort();
    let end = rect.right() - 1;
    let mut left = end;
    if !buttons.is_empty() {
        let n = buttons.len() as u16;
        let cx = end - 1 - (2 * n + 3);
        left = cx;
        if focused || ctx.hover == Some(pane.id) {
            let glyph_style = if focused { th.accent() } else { th.dim() };
            for (i, b) in buttons.iter().enumerate() {
                let bx = cx + 1 + 2 * i as u16;
                hud::put(buf, bx, y, " ", line, 1);
                hud::put(buf, bx + 1, y, b.glyph(ctx.zoomed), glyph_style, 1);
                // Each target is the space before the glyph and the glyph; close also takes the
                // space after it, so every cell between the brackets is a button.
                let w = if i + 1 == buttons.len() { 3 } else { 2 };
                on_top.push((Rect::new(bx, y, w, 1), b.hit(pane.id)));
            }
            hud::put(buf, cx + 1 + 2 * n, y, " ", line, 1);
        }
    }
    // The chips between the title and the buttons, each followed by a line cell: tag, then badge.
    if fit.tag {
        left = hud::put_right(buf, left.saturating_sub(1), y, &format!(" {tag} "), Style::default().fg(th.accent2));
    }
    if fit.badge
        && let Some(spans) = badge
    {
        let w = spans_width(&spans) as u16 + 2;
        let x = left.saturating_sub(1).saturating_sub(w);
        let pad = (" ", spans[0].1);
        let spans: Vec<(&str, Style)> =
            [pad].into_iter().chain(spans.iter().map(|(t, s)| (t.as_str(), *s))).chain([pad]).collect();
        hud::put_spans(buf, x, y, &spans, w);
    }
    inner
}

/// Brackets the button cluster of each pane (`┤ ◫ ⊟ ⤢ ✕ ├`) on its title line. Drawn after the
/// frames are joined, which would turn them back into plain line; each keeps its cell's color, and a
/// junction with another pane's line stays as it is.
fn cluster_brackets(buf: &mut Buffer, on_top: &[(Rect, Hit)]) {
    let mut ends: Vec<(PaneId, u16, u16, u16)> = Vec::new();
    for (r, hit) in on_top {
        let Some((_, pane)) = PaneButton::of_hit(hit) else { continue };
        match ends.iter_mut().find(|e| e.0 == pane) {
            Some(e) => {
                e.2 = e.2.min(r.x);
                e.3 = e.3.max(r.right());
            }
            None => ends.push((pane, r.y, r.x, r.right())),
        }
    }
    for (_, y, from, to) in ends {
        for (x, sym) in [(from.checked_sub(1), "┤"), (Some(to), "├")] {
            if let Some(c) = x.and_then(|x| buf.cell_mut((x, y)))
                && c.symbol() == "─"
            {
                c.set_symbol(sym);
            }
        }
    }
}

/// Marks overlaid on a pane: search matches and the link.
#[derive(Default)]
struct Marks<'a> {
    search: Option<&'a SearchState>,
    link: Option<LinkHover>,
}

fn pane_content(
    buf: &mut Buffer,
    inner: Rect,
    pane: &Pane,
    th: &Theme,
    pal: &TermPalette,
    focused: bool,
    marks: &Marks<'_>,
) -> Option<(u16, u16)> {
    // The themed background also shows in the edge cells the screen does not cover.
    hud::fill(buf, inner, Style::default().bg(pal.bg).fg(pal.fg));
    let parser = pane.parser();
    let screen = parser.screen();
    let (rows, cols) = screen.size();
    let selection = pane.selection;
    let default_bg = pal.bg;
    // Matches in the visible rows: (screen row, start, end, selected).
    let found: Vec<(u16, u16, u16, bool)> = match marks.search {
        Some(s) => {
            let top = s.history.saturating_sub(screen.scrollback());
            s.matches
                .iter()
                .enumerate()
                .filter(|(_, m)| m.line >= top && m.line < top + rows as usize)
                .map(|(i, m)| ((m.line - top) as u16, m.col, m.col + m.width, s.current == Some(i)))
                .collect()
        }
        None => Vec::new(),
    };
    for r in 0..inner.height.min(rows) {
        for c in 0..inner.width.min(cols) {
            let Some(cell) = screen.cell(r, c) else { continue };
            if cell.is_wide_continuation() {
                continue;
            }
            let mut fg = pal.color(cell.fgcolor(), pal.fg);
            let mut bg = pal.color(cell.bgcolor(), default_bg);
            if cell.inverse() {
                std::mem::swap(&mut fg, &mut bg);
                if fg == Color::Reset {
                    fg = th.bg;
                }
                if bg == Color::Reset {
                    bg = th.fg;
                }
            }
            let mut m = Modifier::empty();
            if cell.bold() {
                m |= Modifier::BOLD;
            }
            if cell.dim() {
                m |= Modifier::DIM;
            }
            if cell.italic() {
                m |= Modifier::ITALIC;
            }
            if cell.underline() {
                m |= Modifier::UNDERLINED;
            }
            if selection.is_some_and(|s| !s.is_empty() && s.contains(r, c)) {
                bg = pal.sel_bg;
                fg = pal.fg;
            }
            if let Some(&(.., current)) = found.iter().find(|(fr, a, b, _)| *fr == r && c >= *a && c < *b) {
                if current {
                    bg = th.accent;
                    fg = th.on_accent;
                    m |= Modifier::BOLD;
                } else {
                    bg = pal.match_bg;
                    fg = pal.fg;
                }
            }
            if marks.link.is_some_and(|l| l.row == r && c >= l.from && c < l.to) {
                m |= Modifier::UNDERLINED;
                fg = th.accent2;
            }
            let x = inner.x + c;
            let y = inner.y + r;
            if let Some(out) = buf.cell_mut((x, y)) {
                let sym = cell.contents();
                out.set_symbol(if sym.is_empty() { " " } else { sym });
                out.set_style(Style::default().fg(fg).bg(bg).add_modifier(m));
            }
        }
    }
    if !focused || screen.hide_cursor() || screen.scrollback() > 0 {
        return None;
    }
    let (cr, cc) = screen.cursor_position();
    (cr < inner.height && cc < inner.width).then(|| (inner.x + cc, inner.y + cr))
}

/// The pane is big enough for the search bar on its bottom row.
fn search_bar_fits(inner: Rect) -> bool {
    inner.height >= 2 && inner.width >= 12
}

/// The position bar needs at least this much room (width, height); a smaller pane only gets the
/// "↓ live" chip.
const TRACK_MIN: (u16, u16) = (20, 4);

/// The thumb of a position bar `h` cells high: (top row, length). `history` lines lie above the
/// screen's `rows`, and the view is `offset` lines up from the bottom. Scrolled back (offset > 0),
/// the thumb never touches the bottom row.
pub fn track_thumb(h: u16, history: usize, rows: usize, offset: usize) -> (u16, u16) {
    let h = h.max(1) as usize;
    let total = (history + rows).max(1);
    let len = (h * rows).div_ceil(total).clamp(1, h);
    let top = history.saturating_sub(offset);
    let pos = ((h - len) * top).checked_div(history).unwrap_or(h - len);
    (pos as u16, len as u16)
}

/// The absolute line (0 = oldest) that row `y` of a position bar `track` stands for, out of `total`
/// lines (scrollback and screen): the middle of the part of the scrollback that cell covers.
pub fn track_line(track: Rect, y: u16, total: usize) -> usize {
    let h = track.height.max(1) as usize;
    let row = (y.saturating_sub(track.y) as usize).min(h - 1);
    ((2 * row + 1) * total / (2 * h)).min(total.saturating_sub(1))
}

/// The "↓ live" chip's text for `new` lines below the view, in the widest form that fits `room`.
pub fn live_chip(new: usize, room: u16) -> Option<String> {
    let full = (new > 0).then(|| format!(" ↓ live · {new} "));
    full.into_iter().chain([" ↓ live ".to_string(), " ↓ ".to_string()]).find(|t| util::width(t) as u16 <= room)
}

/// Helpers while a pane is scrolled back (nothing otherwise): a position bar over the content's last
/// column with the search matches on it, and a "↓ live" chip at the bottom right (above the search
/// bar) with the number of lines that arrived meanwhile. A small pane only gets the chip.
fn scroll_helpers(
    buf: &mut Buffer,
    inner: Rect,
    pane: &Pane,
    search: Option<&SearchState>,
    th: &Theme,
    pal: &TermPalette,
    hits: &mut Vec<(Rect, Hit)>,
) {
    let offset = pane.scroll_offset();
    if offset == 0 || inner.width == 0 {
        return;
    }
    let bottom = inner.height.saturating_sub(u16::from(search.is_some() && search_bar_fits(inner)));
    if bottom == 0 {
        return;
    }
    let chip_y = inner.y + bottom - 1;
    let track_h = bottom - 1;
    if inner.width >= TRACK_MIN.0 && inner.height >= TRACK_MIN.1 && track_h >= 2 {
        let x = inner.right() - 1;
        let track = Rect::new(x, inner.y, 1, track_h);
        let rows = pane.size.0 as usize;
        let (pos, len) = track_thumb(track_h, pane.history_len(), rows, offset);
        for r in 0..track_h {
            clear_wide_before(buf, x, inner.y + r);
            let thumb = r >= pos && r < pos + len;
            let (sym, fg) = if thumb { ("┃", th.accent) } else { ("│", th.dim) };
            if let Some(c) = buf.cell_mut((x, inner.y + r)) {
                c.set_symbol(sym);
                c.set_style(Style::default().fg(fg).bg(pal.bg));
            }
        }
        // Search matches: one mark per row of the bar, the selected match drawn last.
        if let Some(s) = search {
            let total = (s.history + rows).max(1);
            let row_of = |line: usize| ((line * track_h as usize / total) as u16).min(track_h - 1);
            let mut marks: Vec<(u16, bool)> =
                s.matches.iter().enumerate().map(|(i, m)| (row_of(m.line), s.current == Some(i))).collect();
            marks.sort_unstable_by_key(|&(row, current)| (current, row));
            marks.dedup();
            for (row, current) in marks {
                // `match_bg` is a faint background tint; as a mark it would barely show.
                let (sym, fg) = if current { ("●", th.accent) } else { ("•", th.accent2) };
                if let Some(c) = buf.cell_mut((x, inner.y + row)) {
                    c.set_symbol(sym);
                    c.set_fg(fg);
                }
            }
        }
        hits.push((track, Hit::ScrollTrack { pane: pane.id, track }));
    }
    let new = pane.new_lines();
    let Some(chip) = live_chip(new, inner.width) else { return };
    let w = util::width(&chip) as u16;
    let x = inner.right() - w;
    let style = if new > 0 {
        Style::default().fg(th.on_accent).bg(th.accent2).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(th.accent).bg(th.raised).add_modifier(Modifier::BOLD)
    };
    clear_wide_before(buf, x, chip_y);
    hud::put(buf, x, chip_y, &chip, style, w);
    hits.push((Rect::new(x, chip_y, w, 1), Hit::ScrollLive(pane.id)));
}

/// Blanks a wide character (CJK, emoji) that starts in the cell left of `x` and so also covers `x`,
/// before `x` is drawn over: the terminal would otherwise never be sent the cell at `x` (a wide
/// symbol's second cell is skipped when the screen is updated).
fn clear_wide_before(buf: &mut Buffer, x: u16, y: u16) {
    if let Some(c) = x.checked_sub(1).and_then(|px| buf.cell_mut((px, y)))
        && util::width(c.symbol()) > 1
    {
        c.set_symbol(" ");
    }
}

/// Search bar overlaid on the pane's bottom row: query, counter and hints.
fn search_bar(buf: &mut Buffer, inner: Rect, s: &SearchState, th: &Theme) {
    if !search_bar_fits(inner) {
        return;
    }
    let y = inner.bottom() - 1;
    let bar = Rect::new(inner.x, y, inner.width, 1);
    hud::fill(buf, bar, Style::default().bg(th.raised).fg(th.fg));
    let count = match (s.current, s.matches.len()) {
        (_, 0) if s.query.is_empty() => String::new(),
        (_, 0) => "no matches".to_string(),
        (Some(i), n) => format!("{}/{n}", i + 1),
        (None, n) => format!("{n}"),
    };
    // The hint's keys in the accent color, the words dim.
    let hint: [(&str, bool); 6] =
        [("⏎↑", true), (" older  ", false), ("↓", true), (" newer  ", false), ("esc", true), (" close", false)];
    let hint_w: u16 = hint.iter().map(|(t, _)| util::width(t) as u16).sum();
    let wide = inner.width >= 60;
    let count_w = util::width(&count) as u16;
    // The counter, then " │ " and the hint.
    let right_w = count_w + if wide { hint_w + 3 } else { 0 };
    let bg = th.raised;
    let x = hud::put(buf, inner.x + 1, y, "find ", th.dim().bg(bg), inner.width);
    let room = inner.right().saturating_sub(x + right_w + 2);
    let query = util::truncate_left(&s.query, room.saturating_sub(1) as usize);
    let x = hud::put(buf, x, y, &query, th.accent_bold().bg(bg), room);
    hud::put(buf, x, y, "▏", th.accent().bg(bg), 1);
    let mut rx = inner.right().saturating_sub(1);
    if wide {
        let hx = rx.saturating_sub(hint_w);
        let spans: Vec<(&str, Style)> =
            hint.iter().map(|&(t, key)| (t, if key { th.accent().bg(bg) } else { th.dim().bg(bg) })).collect();
        hud::put_spans(buf, hx, y, &spans, hint_w);
        // A thin divider between the counter and the hint (only when a counter is shown).
        if count_w > 0 {
            hud::put(buf, hx.saturating_sub(2), y, "│", th.line().bg(bg), 1);
        }
        rx = hx.saturating_sub(3);
    }
    let color = if s.matches.is_empty() && !s.query.is_empty() { th.warn } else { th.accent2 };
    hud::put_right(buf, rx, y, &count, Style::default().fg(color).bg(bg).add_modifier(Modifier::BOLD));
}

/// Linear interpolation between two rectangles.
pub fn lerp_rect(a: Rect, b: Rect, t: f64) -> Rect {
    let l = |x: u16, y: u16| (x as f64 + (y as f64 - x as f64) * t).round() as u16;
    Rect::new(l(a.x, b.x), l(a.y, b.y), l(a.width, b.width), l(a.height, b.height))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(spans: &[(String, Style)]) -> String {
        spans.iter().map(|(t, _)| t.as_str()).collect()
    }

    /// The thumb's size follows the visible share, its place the offset; scrolled back it never sits
    /// on the bottom row, and a click on a row maps back into the scrollback.
    #[test]
    fn scroll_track_geometry() {
        // 20 rows of 100 lines on a 10-cell bar: a 2-cell thumb.
        assert_eq!(track_thumb(10, 80, 20, 80), (0, 2));
        assert_eq!(track_thumb(10, 80, 20, 40), (4, 2));
        let (pos, len) = track_thumb(10, 80, 20, 1);
        assert!(pos + len < 10, "{pos} {len}");
        // A short scrollback: most of the bar is thumb, still above the bottom.
        let (pos, len) = track_thumb(10, 3, 20, 1);
        assert!(len >= 8 && pos + len < 10, "{pos} {len}");
        // Never longer than the bar, never empty.
        assert_eq!(track_thumb(4, 100_000, 20, 50_000).1, 1);
        assert_eq!(track_thumb(0, 10, 20, 5), (0, 1));
        let track = Rect::new(5, 2, 1, 10);
        assert_eq!(track_line(track, 2, 100), 5);
        assert_eq!(track_line(track, 11, 100), 95);
        assert_eq!(track_line(track, 40, 100), 95);
        assert_eq!(track_line(track, 0, 0), 0);
    }

    #[test]
    fn live_chip_shrinks_to_fit() {
        assert_eq!(live_chip(12, 40).as_deref(), Some(" ↓ live · 12 "));
        assert_eq!(live_chip(0, 40).as_deref(), Some(" ↓ live "));
        assert_eq!(live_chip(12, 10).as_deref(), Some(" ↓ live "));
        assert_eq!(live_chip(12, 4).as_deref(), Some(" ↓ "));
        assert_eq!(live_chip(12, 2), None);
        for g in ["↓", "│", "┃", "●", "•"] {
            assert_eq!(util::width(g), 1, "{g}");
        }
    }

    /// A title row gives its room in the order `fit_title` defines: the buttons (close, zoom, the
    /// splits), the glyph, the label, the agent state / badge, the tag, then the folder. A part is
    /// only left out for a less important one when it needs more room than that one had left, the
    /// used cells never pass the width, and a wide row shows everything.
    #[test]
    fn title_row_priority() {
        let groups = [1, 1, 2];
        let check = |p: TitleParts| {
            let wide = fit_title(200, &groups, p);
            assert_eq!(
                wide,
                TitleFit {
                    buttons: 4,
                    lead: p.lead > 0,
                    label: p.label,
                    state: p.state > 0,
                    badge: p.badge > 0,
                    tag: p.tag > 0,
                    cwd: p.cwd
                }
            );
            for w in 0..=200u16 {
                let f = fit_title(w, &groups, p);
                let cluster = if f.buttons > 0 { 2 * f.buttons + 4 } else { 0 };
                // (shown, cells it takes or would take), most important first.
                let min = p.label.min(3);
                let parts = [
                    (f.buttons >= 1, 6),
                    (f.lead || p.lead == 0, p.lead),
                    (f.label >= min, min),
                    (f.buttons >= 2, 2),
                    (f.buttons >= 4, 4),
                    (f.label == p.label, p.label - min),
                    (f.state || p.state == 0, p.state),
                    (f.badge || p.badge == 0, if p.badge > 0 { p.badge + 3 } else { 0 }),
                    (f.tag || p.tag == 0, if p.tag > 0 { p.tag + 3 } else { 0 }),
                ];
                let used = TITLE_FIXED
                    + cluster
                    + if f.lead { p.lead } else { 0 }
                    + f.label
                    + if f.state { p.state } else { 0 }
                    + if f.badge { p.badge + 3 } else { 0 }
                    + if f.tag { p.tag + 3 } else { 0 }
                    + if f.cwd > 0 { 3 + f.cwd } else { 0 };
                assert!(w < TITLE_FIXED as u16 || used <= w as usize, "{w}: {used} cells for {f:?}");
                let left = (w as usize).saturating_sub(used);
                for (i, (shown, cost)) in parts.iter().enumerate() {
                    // The buttons go in order (zoom needs close, the splits zoom), the label needs
                    // the glyph, and its rest its first cells.
                    let gated = match i {
                        2 | 4 => !parts[i - 1].0,
                        3 => !parts[0].0,
                        5 => !parts[2].0,
                        _ => false,
                    };
                    if !shown && !gated {
                        let smaller: usize = parts[i + 1..].iter().filter(|(s, _)| *s).map(|(_, c)| c).sum();
                        assert!(*cost > left + smaller, "{w}: part {i} left out for less: {f:?}");
                    }
                }
                // The buttons go in order: never zoom without close, never the splits without zoom.
                assert!(matches!(f.buttons, 0 | 1 | 2 | 4), "{w}: {f:?}");
                // The folder only shows after the whole label, and only goes when it cannot fit.
                assert!(f.cwd == 0 || f.label == p.label, "{w}: {f:?}");
                assert!(f.cwd > 0 || f.label < p.label || left < 3 + p.cwd.min(4), "{w}: {f:?}");
            }
        };
        // An agent pane: "⠋ claude · working 2m +2 · ~/proj ─ ↑12 ─┤ ◫ ⊟ ⤢ ✕ ├─".
        check(TitleParts { lead: 1, label: 7, state: 16, badge: 0, tag: 3, cwd: 30 });
        // A shell: "pwsh · ~/proj ─ ✓ 2.4s ─ ↑12 ─┤ … ├─".
        let shell = TitleParts { lead: 0, label: 4, state: 0, badge: 6, tag: 3, cwd: 20 };
        check(shell);
        // Narrowing drops the folder first (after shrinking it), then the tag, then the badge.
        let at = |w: u16| fit_title(w, &groups, shell);
        assert_eq!(at(59).cwd, 20);
        assert_eq!(at(50).cwd, 11);
        assert!(at(40).cwd == 0 && at(40).tag && at(40).badge);
        assert!(!at(33).tag && at(33).badge && at(33).label == 4);
        assert!(!at(26).badge && !at(26).tag && at(26).label == 4 && at(26).buttons == 4);
        // The badge's room, too small for it, still holds the smaller tag.
        assert!(at(28).tag && !at(28).badge);
        // Everything fits exactly: 5 fixed cells, 12 for four buttons, the chips and the title.
        assert_eq!(fit_title(5 + 12 + 4 + 9 + 6 + 3 + 20, &groups, shell).cwd, 20);
        // A long label is cut short, not below three cells.
        let long = TitleParts { label: 20, ..TitleParts::default() };
        assert_eq!(fit_title(5 + 12 + 10, &groups, long).label, 10);
        // The label's first cells outrank zoom and the splits: a small pane keeps its name.
        let small = fit_title(5 + 12 + 2, &groups, long);
        assert_eq!((small.buttons, small.label), (2, 6));
        let tiny = fit_title(15, &groups, TitleParts { label: 10, ..TitleParts::default() });
        assert_eq!((tiny.buttons, tiny.label), (1, 4));
        assert_eq!(fit_title(3, &groups, long), TitleFit::default());
        // The key lock outranks every button but close.
        let locked = fit_title(16, &[1, 2], TitleParts { lead: 3, label: 10, tag: 4, ..TitleParts::default() });
        assert!(locked.lead && locked.buttons == 1, "{locked:?}");
    }

    /// The buttons' glyphs are one cell wide (and not emoji), and each maps back from its hit.
    #[test]
    fn pane_buttons_are_single_cells() {
        let pane: PaneId = 7;
        for b in [PaneButton::SplitRight, PaneButton::SplitDown, PaneButton::Zoom, PaneButton::Close] {
            for zoomed in [false, true] {
                assert_eq!(util::width(b.glyph(zoomed)), 1, "{b:?}");
            }
            assert_eq!(PaneButton::of_hit(&b.hit(pane)), Some((b, pane)));
        }
        for g in ["┤", "├"] {
            assert_eq!(util::width(g), 1);
        }
        assert_eq!(PaneButton::of_hit(&Hit::PaneTitle(pane)), None);
    }

    #[test]
    fn working_time_is_by_the_minute() {
        assert_eq!(working_for(-5), None);
        assert_eq!(working_for(59), None);
        assert_eq!(working_for(60), Some("1m".into()));
        assert_eq!(working_for(59 * 60 + 59), Some("59m".into()));
        assert_eq!(working_for(3600 + 2 * 60 + 30), Some("1h02m".into()));
    }

    /// Each state's title in its three widths; the text only uses single-width glyphs.
    #[test]
    fn agent_title_per_state() {
        let th = Theme::by_name("amber", false);
        let now = 10_000;
        let badge = |kind, state, since: Option<i64>, subagents| AgentBadge {
            kind,
            state,
            working_since: since.map(|s| now - s),
            subagents,
        };
        let titles = |b: AgentBadge, working| agent_title(&b, working, now, th.text(), &th).map(|v| text(&v));
        let working = titles(badge("claude", AgentState::Working, Some(125), 2), "⠋");
        assert_eq!(working, ["⠋ claude · working 2m +2", "⠋ claude", "⠋"]);
        let fresh = titles(badge("claude", AgentState::Working, Some(20), 0), "…");
        assert_eq!(fresh[0], "… claude · working");
        let waiting = titles(badge("codex", AgentState::NeedsYou, None, 0), "⠋");
        assert_eq!(waiting, ["◆ codex · needs you", "◆ codex", "◆"]);
        assert_eq!(titles(badge("claude", AgentState::Idle, None, 0), "⠋")[0], "● claude · your turn");
        assert_eq!(titles(badge("claude", AgentState::Running, None, 0), "⠋"), ["○ claude", "○ claude", "○"]);
        // "Needs you": the glyph in the warning color, the words bold in readable text colors (the
        // warning and ok colors are too faint as text on the light themes).
        let spans = agent_title(&badge("codex", AgentState::NeedsYou, None, 0), "⠋", now, th.text(), &th);
        assert_eq!(spans[0][0].1.fg, Some(th.warn));
        let (glyph, state) = (&spans[0][0].1, &spans[0][spans[0].len() - 1].1);
        assert!(glyph.add_modifier.contains(Modifier::BOLD) && state.add_modifier.contains(Modifier::BOLD));
        assert!(spans[0][1..].iter().all(|(_, s)| s.fg == Some(th.fg)), "{:?}", spans[0]);
        let idle = agent_title(&badge("claude", AgentState::Idle, None, 0), "⠋", now, th.text(), &th);
        assert_eq!(idle[0][0].1.fg, Some(th.ok));
        assert!(idle[0][1..].iter().all(|(_, s)| s.fg != Some(th.ok)), "{:?}", idle[0]);
        for t in working.iter().chain(&waiting) {
            assert_eq!(util::width(t), t.chars().count(), "{t}");
        }
    }
}
