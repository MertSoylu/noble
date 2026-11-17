//! The Agent Inbox overlay: a row per agent that needs you and, under the list, the selected agent's request and the
//! last lines of its pane's screen. State and keys are in `app/inbox.rs`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::hud;
use super::overlay::{boxed, centered};
use crate::app::inbox::{InboxItem, InboxState, preview_lines, resolve, wait_label};
use crate::app::{App, Hit, QuickAnswer};
use crate::util;

/// Most agent rows the list shows at once (it scrolls past that).
const LIST_MAX: usize = 6;
/// Most screen lines the preview shows.
const PREVIEW_MAX: usize = 10;
/// Widest the box gets.
const BOX_MAX_W: u16 = 100;
/// Columns from which the wait reads "waiting 5m" instead of "5m".
const WAIT_WORD_FROM: u16 = 56;
/// Columns the detail row needs before the answer chips are drawn on it.
const CHIPS_FROM: u16 = 40;

/// How the box's rows are shared, from the rows inside the frame and the number of agents: the rows of the list
/// and of the preview. The last row is the key hints; with a preview there is also the detail row between the two.
/// A box too short for a preview is all list.
fn rows_for(inner_h: u16, n: usize) -> (usize, usize) {
    let room = usize::from(inner_h.saturating_sub(1));
    if n == 0 {
        return (0, 0);
    }
    if room >= 5 {
        let list = n.min(LIST_MAX).min(room - 4).max(1);
        (list, (room - list - 1).min(PREVIEW_MAX))
    } else {
        (n.min(room), 0)
    }
}

pub(super) fn draw(buf: &mut Buffer, area: Rect, app: &App, st: &InboxState, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    let items = app.inbox_items();
    let at = resolve(&items, st);
    let n = items.len();
    let w = BOX_MAX_W.min(area.width.saturating_sub(2));
    if w < 12 || area.height < 5 {
        return;
    }
    // The box is as tall as its content wants (a list, the detail row, the preview, the hints), or what the window has.
    let want = if n == 0 { 5 } else { n.min(LIST_MAX) + 1 + PREVIEW_MAX.min(8) + 1 };
    let h = (want as u16 + 2).min(area.height.saturating_sub(2));
    let rect = centered(area, w, h);
    hits.push((rect, Hit::Inert));
    let title = if n > 1 { format!("AGENT INBOX · {}/{n}", at + 1) } else { "AGENT INBOX".to_string() };
    let inner = boxed(buf, rect, &title, th, th.warn);
    if inner.height < 2 || inner.width < 10 {
        return;
    }
    let x = inner.x + 1;
    let iw = inner.width.saturating_sub(2);
    let footer = inner.bottom() - 1;
    let selected = items.get(at);
    let options = selected.and_then(|i| app.inbox_quick(i));

    let mut pairs: Vec<(&str, &str)> = vec![("↑↓", "select"), ("⏎", "jump")];
    if options.is_some() {
        pairs.extend([("y", "yes"), ("n", "no")]);
    }
    pairs.push(("esc", "close"));
    hud::key_hints(buf, x, footer, iw, &pairs, th);

    if n == 0 {
        // The sentence wraps in a narrow box and stays in the middle of the rows above the hints.
        let lines = super::overlay::wrap_text("No agent is waiting for you", usize::from(iw));
        let room = usize::from(inner.height - 1);
        let first = inner.y + (room.saturating_sub(lines.len()) / 2) as u16;
        for (k, line) in lines.iter().take(room).enumerate() {
            hud::put_center(buf, Rect::new(x, 0, iw, 1), first + k as u16, line, th.dim());
        }
        return;
    }

    let (list_h, preview_h) = rows_for(inner.height, n);
    let offset = (at + 1).saturating_sub(list_h);
    let glide = app.fx.glide("agent-inbox", at);
    let now = crate::util::unix_now();
    for (row, (i, item)) in items.iter().enumerate().skip(offset).take(list_h).enumerate() {
        let y = inner.y + row as u16;
        let line = Rect::new(inner.x, y, inner.width, 1);
        let bg = glide.bg(i, th.raised, th.sel_bg);
        hud::fill(buf, line, Style::default().bg(bg));
        if i == at {
            hud::put(buf, inner.x, y, "▌", Style::default().fg(th.warn).bg(bg), 1);
        }
        agent_row(buf, x, y, iw, item, now, app, bg);
        hits.push((line, Hit::InboxRow(item.pane)));
    }

    let Some(item) = selected.filter(|_| preview_h > 0) else { return };
    // The detail row: what the agent asks, where it is, and the answer chips of a permission prompt.
    let dy = inner.y + list_h as u16;
    let chips = options.is_some() && iw >= CHIPS_FROM;
    let chips_w: u16 = if chips { 7 + 2 + 6 } else { 0 };
    let text_w = iw.saturating_sub(chips_w + u16::from(chips));
    let msg_w = util::width(&item.message).min(text_w as usize);
    let at_x =
        hud::put(buf, x, dy, &util::truncate(&item.message, msg_w), th.text().add_modifier(Modifier::BOLD), text_w);
    // Where it runs, when the row has room left after the message.
    if usize::from(text_w) > msg_w + 6 {
        let place = format!("{} · {}", item.tab_title, item.cwd);
        let place = util::truncate(&place, usize::from(text_w) - msg_w - 5);
        hud::put_spans(buf, at_x, dy, &[("  ·  ", th.dim()), (&place, th.dim())], text_w - msg_w as u16);
    }
    if chips {
        let no_x = x + iw - 6;
        let yes_x = no_x - 2 - 7;
        let end = hud::key_chip(buf, yes_x, dy, "y", "yes", th, true, 7);
        hits.push((Rect::new(yes_x, dy, end.saturating_sub(yes_x), 1), Hit::InboxAnswer(QuickAnswer::Yes)));
        let end = hud::key_chip(buf, no_x, dy, "n", "no", th, true, 6);
        hits.push((Rect::new(no_x, dy, end.saturating_sub(no_x), 1), Hit::InboxAnswer(QuickAnswer::No)));
    }

    // The pane's screen: its last lines, as the agent shows them now.
    let rows = app.panes.get(&item.pane).map(|p| p.live_rows()).unwrap_or_default();
    let lines = preview_lines(&rows, preview_h);
    let top = dy + 1;
    if lines.is_empty() {
        hud::put(buf, x, top, "▏ (nothing on the screen)", th.dim(), iw);
    }
    for (k, text) in lines.iter().enumerate() {
        let y = top + k as u16;
        if y >= footer {
            break;
        }
        hud::put(buf, x, y, "▏", th.line(), 1);
        hud::put(buf, x + 2, y, text, th.text(), iw.saturating_sub(2));
    }
}

/// One agent's row: its mark, name, where it runs, what it asks (when there is room) and how long it has waited.
#[allow(clippy::too_many_arguments)]
fn agent_row(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    iw: u16,
    item: &InboxItem,
    now: i64,
    app: &App,
    bg: ratatui::style::Color,
) {
    let th = &app.theme;
    let end = x + iw;
    let wait = item.since.map(|s| wait_label(now - s)).unwrap_or_default();
    let wait = if !wait.is_empty() && iw >= WAIT_WORD_FROM { format!("waiting {wait}") } else { wait };
    let wait_w = util::width(&wait) as u16;
    hud::put(buf, x, y, "◆", Style::default().fg(th.warn).bg(bg).add_modifier(Modifier::BOLD), 1);
    if wait_w > 0 {
        hud::put_right(buf, end, y, &wait, Style::default().fg(th.warn).bg(bg));
    }
    // The text between the mark and the wait, with a blank before the wait.
    let left_end = end.saturating_sub(if wait_w > 0 { wait_w + 2 } else { 0 });
    let start = x + 2;
    if left_end <= start {
        return;
    }
    let room = |cx: u16| usize::from(left_end.saturating_sub(cx));
    let kind = util::truncate(item.kind, room(start));
    let mut cx = hud::put(buf, start, y, &kind, th.text().bg(bg).add_modifier(Modifier::BOLD), left_end - start);
    let mut identity = format!("tab {}", item.tab + 1);
    if !item.place.is_empty() {
        identity.push_str(&format!(" · {}", item.place));
    }
    if item.panes_in_tab > 1 {
        identity.push_str(&format!(" · pane {}/{}", item.pane_no, item.panes_in_tab));
    }
    if room(cx) > 4 {
        let text = util::truncate(&format!("  {identity}"), room(cx));
        cx = hud::put(buf, cx, y, &text, th.text().bg(bg), left_end - cx);
    }
    if room(cx) > 8 {
        let text = util::truncate(&format!("  {}", item.message), room(cx));
        hud::put(buf, cx, y, &text, th.dim().bg(bg), left_end - cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rows_are_shared_between_list_and_preview() {
        // Roomy: the list keeps what it needs, the preview gets the rest (up to its cap).
        assert_eq!(rows_for(20, 2), (2, 10));
        assert_eq!(rows_for(12, 2), (2, 8));
        assert_eq!(rows_for(14, 9), (6, 6));
        // A short box shrinks the list before the preview is lost.
        assert_eq!(rows_for(8, 5), (3, 3));
        // Too short for a preview: all list, and the hints row stays.
        assert_eq!(rows_for(4, 5), (3, 0));
        assert_eq!(rows_for(2, 5), (1, 0));
        assert_eq!(rows_for(1, 5), (0, 0));
        assert_eq!(rows_for(10, 0), (0, 0));
        for h in 0..30u16 {
            for n in 0..9usize {
                let (list, preview) = rows_for(h, n);
                let used = list + preview + usize::from(preview > 0) + usize::from(h > 0);
                assert!(used <= usize::from(h).max(usize::from(h > 0)), "h {h} n {n}: {list} + {preview}");
            }
        }
    }
}
