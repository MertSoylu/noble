//! Agent duel overlays: the start form, the compare view of the two sides and the end dialog (`app/duel.rs`).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::hud;
use super::overlay::{boxed, centered, wrap_text};
use crate::app::{App, CompareState, DuelField, DuelForm, DuelHit, DuelView, EndState, EndStep, Hit};
use crate::duel::{DeleteBranches, FileChange, Side};
use crate::util;

pub fn draw(buf: &mut Buffer, area: Rect, app: &App, view: &DuelView, hits: &mut Vec<(Rect, Hit)>) {
    match view {
        DuelView::Form(f) => form(buf, area, app, f, hits),
        DuelView::Compare(st) => compare(buf, area, app, st, hits),
        DuelView::End(st) => end(buf, area, app, st, hits),
    }
}

fn hit(hits: &mut Vec<(Rect, Hit)>, rect: Rect, what: DuelHit) {
    if rect.width > 0 && rect.height > 0 {
        hits.push((rect, Hit::Duel(what)));
    }
}

/// The footer of an overlay: the chip of its main key (a click target), then the other keys.
fn footer(
    buf: &mut Buffer,
    app: &App,
    inner: Rect,
    chips: &[(&str, &str, DuelHit)],
    pairs: &[(&str, &str)],
    hits: &mut Vec<(Rect, Hit)>,
) {
    let th = &app.theme;
    let y = inner.bottom().saturating_sub(1);
    let end = inner.right().saturating_sub(1);
    let mut x = inner.x + 1;
    for (key, label, what) in chips {
        // A cut key chip would read as another key: it is drawn whole or not at all.
        let need = (util::width(key) + 2 + 1 + util::width(label)) as u16;
        if need > end.saturating_sub(x) {
            break;
        }
        let start = x;
        x = hud::key_chip(buf, x, y, key, label, th, true, end.saturating_sub(x));
        hit(hits, Rect::new(start, y, x.saturating_sub(start), 1), *what);
        x = hud::put(buf, x, y, "  ", th.dim(), end.saturating_sub(x));
    }
    hud::key_hints(buf, x, y, end.saturating_sub(x), pairs, th);
}

// ─── The form ────────────────────────────────────────────────────────────────────────────────────

fn form(buf: &mut Buffer, area: Rect, app: &App, f: &DuelForm, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    let w = 74.min(area.width.saturating_sub(2));
    let avail = area.height.saturating_sub(2);
    // Project line, two agent rows, the task label, the message row and the footer around the task rows.
    let task_rows = avail.saturating_sub(8).clamp(1, 6);
    let rect = centered(area, w, (8 + task_rows).min(avail));
    hits.push((rect, Hit::Inert));
    let title = format!("AGENT DUEL · {}", f.name);
    let inner = boxed(buf, rect, &util::truncate(&title, (w as usize).saturating_sub(6)), th, th.accent);
    if inner.height < 3 || inner.width < 12 {
        return;
    }
    let x = inner.x + 1;
    let iw = inner.width.saturating_sub(2);
    let foot_y = inner.bottom() - 1;
    let roomy = inner.height >= 8;
    let mut y = inner.y;
    if roomy {
        let text = format!("both start from {}", f.from);
        hud::put(buf, x, y, &util::truncate(&text, iw as usize), th.dim(), iw);
        y += 1;
    }
    for (side, field, chosen) in [(Side::A, DuelField::A, f.a), (Side::B, DuelField::B, f.b)] {
        agent_row(buf, app, f, (side, field, chosen), (x, y, iw), hits);
        y += 1;
    }
    // The message row (an error, or what is going on); a short box has none and shows an error over the footer.
    let msg_y = (inner.height >= 7).then(|| foot_y - 1);
    let label_y = (inner.height >= 6).then_some(y);
    if let Some(ly) = label_y {
        let focused = f.field == DuelField::Task;
        let style = if focused { th.accent_bold() } else { th.dim() };
        let count = format!("{}/{}", f.task.chars().count(), crate::duel::MAX_PROMPT);
        hud::put(buf, x, ly, "task", style, iw);
        hud::put_right(buf, x + iw, ly, &count, th.dim());
        y += 1;
    }
    let task_end = msg_y.unwrap_or(foot_y);
    let task_area = Rect::new(inner.x, y, inner.width, task_end.saturating_sub(y));
    task_rows_draw(buf, app, f, task_area, hits);
    let note = if f.busy {
        Some(("creating the worktrees and branches…".to_string(), th.accent()))
    } else {
        f.error.as_ref().map(|e| (e.clone(), Style::default().fg(th.crit)))
    };
    match (note, msg_y) {
        (Some((text, style)), Some(my)) => {
            hud::put(buf, x, my, &util::truncate(&text, iw as usize), style, iw);
        }
        (Some((text, style)), None) => {
            hud::put(buf, x, foot_y, &util::truncate(&text, iw as usize), style, iw);
            return;
        }
        (None, Some(my)) => {
            let text = "worktrees under NOBLE's data folder · branches noble/duel-…";
            hud::put(buf, x, my, &util::truncate(text, iw as usize), th.dim(), iw);
        }
        (None, None) => {}
    }
    if f.busy {
        footer(buf, app, inner, &[], &[("esc", "hide")], hits);
        return;
    }
    let task_keys: &[(&str, &str)] = &[("ctrl+j", "new line"), ("tab", "next"), ("esc", "cancel")];
    let agent_keys: &[(&str, &str)] = &[("←→", "agent"), ("tab", "next"), ("esc", "cancel")];
    let keys = if f.field == DuelField::Task { task_keys } else { agent_keys };
    footer(buf, app, inner, &[("⏎", "start", DuelHit::Start)], keys, hits);
}

/// One agent row: the side's letter and a chip per agent; the chosen one is lit. Too many chips for the row show
/// only the chosen one (a click on it steps to the next).
fn agent_row(
    buf: &mut Buffer,
    app: &App,
    f: &DuelForm,
    (side, field, chosen): (Side, DuelField, usize),
    (x, y, iw): (u16, u16, u16),
    hits: &mut Vec<(Rect, Hit)>,
) {
    let th = &app.theme;
    let focused = f.field == field;
    let row = Rect::new(x.saturating_sub(1), y, iw + 2, 1);
    hit(hits, row, DuelHit::Field(field));
    if focused {
        hud::put(buf, row.x, y, "▌", Style::default().fg(th.accent), 1);
    }
    let letter = side.letter().to_uppercase();
    let label_style = if focused { th.accent_bold() } else { th.dim() };
    let mut cx = hud::put(buf, x, y, &letter, label_style, iw) + 2;
    let end = x + iw;
    let names: Vec<String> = f.agents.iter().map(|a| format!(" {} ", a.name)).collect();
    let total: usize = names.iter().map(|n| util::width(n)).sum::<usize>() + names.len().saturating_sub(1);
    let lit = Style::default().fg(th.on_accent).bg(th.accent).add_modifier(Modifier::BOLD);
    if total as u16 <= end.saturating_sub(cx) {
        for (i, name) in names.iter().enumerate() {
            let start = cx;
            cx = hud::put(
                buf,
                cx,
                y,
                name,
                if i == chosen { lit } else { th.dim().bg(th.sel_bg) },
                end.saturating_sub(cx),
            );
            hit(hits, Rect::new(start, y, cx - start, 1), DuelHit::Pick(side, i));
            cx = hud::put(buf, cx, y, " ", th.dim(), end.saturating_sub(cx));
        }
    } else if let Some(name) = names.get(chosen) {
        let start = cx;
        cx = hud::put(buf, cx, y, name, lit, end.saturating_sub(cx));
        let more = format!(" {}/{}", chosen + 1, names.len());
        let next = (chosen + 1) % names.len();
        cx = hud::put(buf, cx, y, &more, th.dim(), end.saturating_sub(cx));
        hit(hits, Rect::new(start, y, cx - start, 1), DuelHit::Pick(side, next));
    }
}

/// The task text: wrapped rows ending at the cursor's row (a longer text scrolls), the cursor drawn as a bar.
fn task_rows_draw(buf: &mut Buffer, app: &App, f: &DuelForm, area: Rect, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    if area.height == 0 || area.width < 6 {
        return;
    }
    hit(hits, area, DuelHit::Field(DuelField::Task));
    let focused = f.field == DuelField::Task;
    let x = area.x + 3;
    // One column stays free for the cursor at the end of a full row.
    let text_w = area.width.saturating_sub(5);
    let room = text_w + 1;
    let rows = f.rows(text_w as usize);
    let cursor_row = f.cursor_row(&rows);
    let top = (cursor_row + 1).saturating_sub(usize::from(area.height));
    if focused {
        for dy in 0..area.height {
            hud::put(buf, area.x, area.y + dy, "▌", Style::default().fg(th.accent), 1);
        }
    }
    if f.task.is_empty() {
        let hint = "what should both agents do? ⏎ starts them";
        let (cursor, st) = (if app.cursor_lit() && focused { "▏" } else { " " }, th.accent());
        hud::put_spans(buf, area.x + 1, area.y, &[("❯ ", th.accent()), (cursor, st), (hint, th.dim())], area.width - 1);
        return;
    }
    for (n, range) in rows.iter().enumerate().skip(top).take(area.height as usize) {
        let y = area.y + (n - top) as u16;
        let chars: Vec<char> = f.task.chars().collect();
        let end = range.end.min(chars.len());
        let text: String = chars[range.start.min(end)..end].iter().collect();
        let mark = if n == top && top > 0 {
            "↑ "
        } else if n == 0 {
            "❯ "
        } else {
            "  "
        };
        hud::put(buf, area.x + 1, y, mark, th.accent(), 2);
        if focused && n == cursor_row {
            let at = f.cursor.saturating_sub(range.start).min(end - range.start.min(end));
            let before: String = text.chars().take(at).collect();
            let after: String = text.chars().skip(at).collect();
            let cursor = if app.cursor_lit() { "▏" } else { " " };
            hud::put_spans(
                buf,
                x,
                y,
                &[(&before, th.accent_bold()), (cursor, th.accent()), (&after, th.accent_bold())],
                room,
            );
        } else {
            hud::put(buf, x, y, &text, th.text(), room);
        }
    }
}

// ─── Compare ─────────────────────────────────────────────────────────────────────────────────────

fn compare(buf: &mut Buffer, area: Rect, app: &App, st: &CompareState, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    let w = 108.min(area.width.saturating_sub(2));
    let avail = area.height.saturating_sub(2);
    let rect = centered(area, w, 32.min(avail));
    hits.push((rect, Hit::Inert));
    let title = format!("COMPARE DUEL · {} · {}", st.rec.name, st.rec.id);
    let inner = boxed(buf, rect, &util::truncate(&title, (w as usize).saturating_sub(6)), th, th.accent);
    if inner.height < 4 || inner.width < 16 {
        return;
    }
    let x = inner.x + 1;
    let iw = inner.width.saturating_sub(2);
    let foot_y = inner.bottom() - 1;
    let mut y = inner.y;
    // What both started from, and the task.
    let base = match &st.rec.base_branch {
        Some(b) => format!("base {} on {b}", st.rec.base_short()),
        None => format!("base {}", st.rec.base_short()),
    };
    hud::put(buf, x, y, &util::truncate(&base, iw as usize), th.dim(), iw);
    y += 1;
    if inner.height >= 14 {
        let first = st.rec.prompt.lines().next().unwrap_or("");
        let text = util::truncate(&format!("task  {first}"), iw as usize);
        hud::put(buf, x, y, &text, th.text(), iw);
        y += 1;
    }
    // Commands (of the focused side) below the lists, when there is room.
    let show_commands = inner.height >= 19;
    let commands_h: u16 = if show_commands { 4 } else { 0 };
    let list_bottom = foot_y.saturating_sub(commands_h);
    let two = inner.width >= 62;
    let col_w = if two { (iw.saturating_sub(3)) / 2 } else { iw };
    let sides: Vec<(Side, u16)> = if two { vec![(Side::A, x), (Side::B, x + col_w + 3)] } else { vec![(st.side, x)] };
    for (side, sx) in &sides {
        side_block(buf, app, st, *side, Rect::new(*sx, y, col_w, list_bottom.saturating_sub(y)), hits);
    }
    if two {
        let sep_x = x + col_w + 1;
        for sy in y..list_bottom {
            hud::put(buf, sep_x, sy, "│", th.line(), 1);
        }
    }
    if show_commands {
        let cy = list_bottom;
        let label = format!(" {} · inspect or merge (NOBLE never runs these) ", st.side.letter().to_uppercase());
        hud::hline(buf, inner.x + 1, cy, iw, "─", th.line());
        hud::put(buf, x + 2, cy, &util::truncate(&label, iw.saturating_sub(4) as usize), th.dim(), iw);
        for (i, cmd) in crate::duel::side_commands(&st.rec, st.side).iter().enumerate() {
            let line = util::truncate(cmd, iw as usize);
            hud::put(buf, x, cy + 1 + i as u16, &line, th.accent2(), iw);
        }
    }
    let side_name = st.side.letter().to_uppercase();
    let copy_label = format!("copy {side_name} commands");
    let mut keys = vec![("←→", "side"), ("↑↓", "scroll"), ("esc", "close")];
    if inner.width < 60 {
        keys.remove(1);
    }
    footer(
        buf,
        app,
        inner,
        &[("c", copy_label.as_str(), DuelHit::Copy), ("r", "refresh", DuelHit::Refresh)],
        &keys,
        hits,
    );
}

/// One side's column: the agent and branch, the totals, the file list.
fn side_block(buf: &mut Buffer, app: &App, st: &CompareState, side: Side, area: Rect, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    if area.height == 0 || area.width < 8 {
        return;
    }
    let focused = st.side == side;
    let s = st.rec.side(side);
    let (x, w) = (area.x, area.width);
    let letter = side.letter().to_uppercase();
    let head_style = if focused { th.accent_bold() } else { th.text() };
    let head = format!("{letter} · {}", s.agent);
    let mut y = area.y;
    let end = hud::put(buf, x, y, &head, head_style, w);
    hud::put(buf, end + 1, y, &util::truncate(&s.branch, (x + w).saturating_sub(end + 1) as usize), th.dim(), w);
    hit(hits, Rect::new(x, y, w, 1), DuelHit::Side(side));
    y += 1;
    let Some(report) = &st.report else {
        if y < area.bottom() {
            hud::put(buf, x, y, "reading the worktree…", th.dim(), w);
        }
        return;
    };
    let r = report.side(side);
    if y < area.bottom() {
        let files = if r.file_count == 1 { "1 file".to_string() } else { format!("{} files", r.file_count) };
        let spans = [
            (files.as_str(), th.text()),
            ("  ", th.dim()),
            (&format!("+{}", r.added), Style::default().fg(th.ok)),
            (" ", th.dim()),
            (&format!("−{}", r.removed), Style::default().fg(th.crit)),
        ];
        hud::put_spans(buf, x, y, &spans, w);
        y += 1;
    }
    if y < area.bottom() {
        let (text, style) = match (&r.error, r.worktree_exists) {
            (Some(e), _) => (e.clone(), Style::default().fg(th.crit)),
            (None, false) => {
                ("the worktree folder is gone — only the branch is read".into(), Style::default().fg(th.warn))
            }
            (None, true) => {
                let commits = if r.commits == 1 { "1 commit".to_string() } else { format!("{} commits", r.commits) };
                (format!("{commits} · {} uncommitted", r.uncommitted), th.dim())
            }
        };
        hud::put(buf, x, y, &util::truncate(&text, w as usize), style, w);
        y += 1;
    }
    if y < area.bottom() {
        hud::hline(buf, x, y, w, "─", th.line());
        let legend = "✓ committed  ~ uncommitted  + new";
        if w as usize >= util::width(legend) + 6 {
            hud::put_right(buf, x + w - 1, y, legend, th.dim());
        }
        y += 1;
    }
    let list_h = area.bottom().saturating_sub(y) as usize;
    if list_h == 0 {
        return;
    }
    if r.files.is_empty() {
        let text = if r.error.is_some() { "" } else { "no changes yet" };
        hud::put(buf, x, y, text, th.dim(), w);
        return;
    }
    let slot = usize::from(side == Side::B);
    let top = st.scroll[slot].min(r.files.len().saturating_sub(list_h.min(r.files.len())));
    let more = r.file_count.saturating_sub(r.files.len());
    for (i, file) in r.files.iter().enumerate().skip(top).take(list_h) {
        let row_y = y + (i - top) as u16;
        // The last row says how many more there are, instead of the next file.
        let last = i - top + 1 == list_h && (i + 1 < r.files.len() || more > 0);
        if last {
            let left = r.files.len() - i + more;
            hud::put(buf, x, row_y, &format!("… {left} more (↑↓ scrolls)"), th.dim(), w);
            break;
        }
        file_row(buf, app, file, (x, row_y, w));
    }
}

fn file_row(buf: &mut Buffer, app: &App, file: &FileChange, (x, y, w): (u16, u16, u16)) {
    let th = &app.theme;
    let (mark, mark_style) = if file.untracked {
        ("+", Style::default().fg(th.ok))
    } else if file.uncommitted {
        ("~", Style::default().fg(th.warn))
    } else {
        ("✓", Style::default().fg(th.accent2))
    };
    hud::put(buf, x, y, mark, mark_style, 1);
    let count = |n: Option<u32>| n.map_or("·".to_string(), |n| n.to_string());
    let (plus, minus) = (format!("+{}", count(file.added)), format!("−{}", count(file.removed)));
    let nums_w = (util::width(&plus) + 1 + util::width(&minus)) as u16;
    // The numbers sit at the right edge, the path (its tail, the file name) takes the rest.
    let path_w = w.saturating_sub(nums_w + 3) as usize;
    hud::put(buf, x + 2, y, &util::truncate_left(&file.path, path_w), th.text(), w.saturating_sub(2));
    if w > nums_w + 3 {
        let nx = x + w - nums_w;
        let after = hud::put(buf, nx, y, &plus, Style::default().fg(th.ok), nums_w);
        hud::put(buf, after + 1, y, &minus, Style::default().fg(th.crit), (x + w).saturating_sub(after + 1));
    }
}

// ─── End ─────────────────────────────────────────────────────────────────────────────────────────

fn end(buf: &mut Buffer, area: Rect, app: &App, st: &EndState, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    let w = 70.min(area.width.saturating_sub(2));
    let text_w = (w as usize).saturating_sub(4);
    let rec = &st.rec;
    let mut lines: Vec<(String, Style)> = Vec::new();
    // Wrapped text; a path or branch name goes on a line of its own, cut from the left when it is too long.
    let mut say = |text: String, style: Style| {
        for l in wrap_text(&text, text_w) {
            lines.push((l, style));
        }
    };
    match st.step {
        EndStep::Ask | EndStep::Checking | EndStep::Removing => {
            say(
                format!("Closes this duel's panes (both agents stop) and removes its two worktrees of {}:", rec.name),
                th.text(),
            );
        }
        EndStep::Dirty(a, b) => {
            let files = |n: usize| if n == 1 { "1 file".to_string() } else { format!("{n} files") };
            say(
                format!(
                    "Uncommitted changes: A has {}, B has {}. Removing the worktrees discards them; the commits stay on the branches.",
                    files(a),
                    files(b)
                ),
                Style::default().fg(th.warn),
            );
            say("Nothing was removed yet. Commit them first, or force the removal.".into(), th.text());
        }
    }
    if matches!(st.step, EndStep::Ask | EndStep::Checking | EndStep::Removing) {
        for side in Side::BOTH {
            let path = util::tilde(std::path::Path::new(&rec.side(side).worktree));
            let text =
                format!("{}  {}", side.letter().to_uppercase(), util::truncate_left(&path, text_w.saturating_sub(3)));
            lines.push((text, th.dim()));
        }
        let mut say = |text: String| {
            for l in wrap_text(&text, text_w) {
                lines.push((l, th.text()));
            }
        };
        say(format!("The branches {} and {} are kept, so no work is lost.", rec.a.branch, rec.b.branch));
    }
    if let Some(e) = &st.error {
        lines.push((util::truncate(e, text_w), Style::default().fg(th.crit)));
    }
    let want_choices = st.step == EndStep::Ask;
    // Rows: a padding row, the text, a blank row, the choices (a question and four rows), the footer.
    let choice_h: u16 = if want_choices { 5 } else { 0 };
    let full = lines.len() as u16 + 1 + if want_choices { choice_h + 1 } else { 0 } + 1;
    let avail = area.height.saturating_sub(2);
    // The question comes first: the branch choice is shown only when the box can hold it under all of the text.
    let choices = want_choices && full + 2 <= avail;
    let height = if want_choices && !choices { lines.len() as u16 + 1 + 1 } else { full } + 2;
    let rect = centered(area, w, height.min(avail));
    hits.push((rect, Hit::Inert));
    let inner = boxed(buf, rect, &format!("END DUEL · {}", rec.id), th, th.crit);
    if inner.height < 3 || inner.width < 16 {
        return;
    }
    let x = inner.x + 1;
    let iw = inner.width.saturating_sub(2);
    let foot_y = inner.bottom() - 1;
    let first = inner.y + 1;
    let rows = foot_y.saturating_sub(first);
    let text_rows = if choices { lines.len() as u16 } else { rows.min(lines.len() as u16) };
    for (i, (text, style)) in lines.iter().take(usize::from(text_rows)).enumerate() {
        let last_cut = i + 1 == usize::from(text_rows) && lines.len() > usize::from(text_rows);
        let shown = if last_cut { util::truncate(&format!("{text} …"), iw as usize) } else { text.clone() };
        hud::put(buf, x, first + i as u16, &shown, *style, iw);
    }
    if choices {
        let cy = foot_y - choice_h;
        hud::put(buf, x, cy, "Delete a branch as well? Only if you pick it:", th.dim(), iw);
        for (i, choice) in DeleteBranches::ALL.iter().enumerate() {
            let row_y = cy + 1 + i as u16;
            let on = st.delete == *choice;
            let label = match choice {
                DeleteBranches::None => "keep both branches".to_string(),
                DeleteBranches::A => format!("also delete {} (unmerged commits are lost)", rec.a.branch),
                DeleteBranches::B => format!("also delete {} (unmerged commits are lost)", rec.b.branch),
                DeleteBranches::Both => "also delete both branches (unmerged commits are lost)".to_string(),
            };
            let (dot, style) = if on { ("● ", th.accent_bold()) } else { ("○ ", th.dim()) };
            let n = format!("{} ", i + 1);
            let label = util::truncate(&label, text_w.saturating_sub(5));
            hud::put_spans(buf, x, row_y, &[(&n, th.accent2()), (dot, style), (&label, style)], iw);
            hit(hits, Rect::new(inner.x, row_y, inner.width, 1), DuelHit::Delete(*choice));
        }
    }
    match st.step {
        EndStep::Ask => footer(
            buf,
            app,
            inner,
            &[("y", "remove worktrees", DuelHit::Confirm)],
            &[("↑↓", "branches"), ("esc", "cancel")],
            hits,
        ),
        EndStep::Dirty(..) => {
            footer(buf, app, inner, &[("f", "force remove", DuelHit::Force)], &[("esc", "cancel")], hits)
        }
        EndStep::Checking | EndStep::Removing => {
            let text = if st.step == EndStep::Checking { "looking for uncommitted changes…" } else { "removing…" };
            hud::put(buf, x, foot_y, text, th.accent(), iw);
        }
    }
}
