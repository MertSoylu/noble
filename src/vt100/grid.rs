use crate::vt100::term::BufWrite as _;
use alloc::{string::String, vec::Vec};

#[derive(Clone, Debug)]
pub struct Grid {
    size: Size,
    pos: Pos,
    saved_pos: Pos,
    rows: Vec<crate::vt100::row::Row>,
    scroll_top: u16,
    scroll_bottom: u16,
    origin_mode: bool,
    saved_origin_mode: bool,
    scrollback: Option<ScrollbackState>,
    /// NOBLE: rewrap the rows on a width change and keep rows below the cursor on a height change (set for the
    /// primary grid only).
    reflow: bool,
    /// NOBLE: a taller screen pulls the newest history rows down (the default); off, it keeps its top row and gets
    /// blank rows at the bottom (see `set_pull_history`).
    pull_history: bool,
    /// NOBLE: counts the reflows (and screen resets) that renumbered the lines; see `epoch`.
    epoch: u64,
}

#[derive(Clone, Debug)]
struct ScrollbackState {
    rows: alloc::collections::VecDeque<crate::vt100::row::Row>,
    capacity: usize,
    offset: usize,
    /// The number of rows of scrollback there *would* be if the scrollback
    /// capacity were unlimited.
    unbounded_len: usize,
    /// NOBLE: the rows above the oldest kept scrollback row: scrolled out of a full scrollback, cleared, or
    /// reflowed away. `dropped + index` numbers a line the same way for as long as the epoch stays.
    dropped: usize,
}

#[derive(Clone, Copy, Debug)]
pub enum Scrollback {
    Disabled,
    Enabled { capacity: usize },
}

impl Grid {
    pub fn new(size: Size, scrollback: Scrollback) -> Self {
        let scrollback = match scrollback {
            Scrollback::Disabled => None,
            Scrollback::Enabled { capacity } => Some(ScrollbackState {
                rows: alloc::collections::VecDeque::new(),
                capacity,
                offset: 0,
                unbounded_len: 0,
                dropped: 0,
            }),
        };
        Self {
            size,
            pos: Pos::default(),
            saved_pos: Pos::default(),
            rows: Vec::new(),
            scroll_top: 0,
            scroll_bottom: size.rows() - 1,
            origin_mode: false,
            saved_origin_mode: false,
            scrollback,
            reflow: false,
            pull_history: true,
            epoch: 0,
        }
    }

    /// NOBLE: the rows that left the top of the scrollback (see `ScrollbackState::dropped`); `0` without one.
    pub fn history_dropped(&self) -> usize {
        self.scrollback.as_ref().map_or(0, |s| s.dropped)
    }

    /// NOBLE: changes when a reflow or a reset renumbered the lines, so a stored line number no longer points at the
    /// same text.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// NOBLE: a screen reset starts a new grid; its epoch continues after the old one's.
    pub fn continue_epoch(&mut self, previous: u64) {
        self.epoch = previous.wrapping_add(1);
    }

    pub fn set_reflow(&mut self, reflow: bool) {
        self.reflow = reflow;
    }

    pub fn reflow(&self) -> bool {
        self.reflow
    }

    pub fn set_pull_history(&mut self, pull: bool) {
        self.pull_history = pull;
    }

    pub fn pull_history(&self) -> bool {
        self.pull_history
    }

    pub fn allocate_rows(&mut self) {
        if self.rows.is_empty() {
            self.rows.resize_with(usize::from(self.size.rows()), || crate::vt100::row::Row::new(self.size.cols()));
        }
    }

    pub fn clear(&mut self) {
        self.pos = Pos::default();
        self.saved_pos = Pos::default();
        for row in self.drawing_rows_mut() {
            row.reset();
        }
        self.scroll_top = 0;
        self.scroll_bottom = self.size.rows() - 1;
        self.origin_mode = false;
        self.saved_origin_mode = false;
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn set_size<F>(&mut self, size: Size, on_row: F)
    where
        F: FnMut(&crate::vt100::row::Row),
    {
        if size == self.size {
            return;
        }

        // xterm always resets the scroll region and scrollback offset when the
        // screen is resized.
        self.scroll_top = 0;
        self.scroll_bottom = self.size.rows() - 1;
        if let Some(scrollback) = &mut self.scrollback {
            scrollback.offset = 0;
        }

        // `rows` might be empty if this is the alternate screen and the rows
        // haven't been allocated (Self::allocate_rows) yet.
        let is_empty = self.rows.is_empty();
        if self.reflow && !is_empty && size.cols() != self.size.cols() {
            self.reflow_rows(size, on_row);
            return;
        }

        let new_row = || crate::vt100::row::Row::new(size.cols());
        let diff = self.size.rows().abs_diff(size.rows());
        let pos_diff = if is_empty {
            0
        } else if size.rows() < self.size.rows() {
            // NOBLE: rows with text below the cursor are pushed to the scrollback instead of being cut off (only
            // with `reflow`; the alternate screen has no scrollback and keeps truncating).
            let bottom_used = if self.reflow { self.last_used_row() } else { self.pos.row };
            let num_excess_bottom = self.size.rows() - bottom_used - 1;
            let num_pushed = diff.saturating_sub(num_excess_bottom);
            self.scroll_up(num_pushed, on_row);
            self.rows.truncate(size.rows().into());
            num_pushed
        } else if let Some(scrollback) = self.scrollback.as_mut().filter(|_| self.pull_history) {
            // NOBLE: a taller screen pulls history down only with `pull_history`. Without it (Windows: ConPTY keeps the
            // top row of its viewport where it was and repaints from there) the new rows are added at the bottom, or
            // the repaint would write over the rows pulled down.
            let num_pulled = u16::try_from(scrollback.unbounded_len).unwrap_or(u16::MAX).min(diff);
            // Number of rows actually pulled from scrollback.
            let num_pulled_scrollback = scrollback.rows.len().min(num_pulled.into());
            // Number of blank rows inserted at the top, because our scrollback
            // buffer isn't as large as the theoretical amount of scrollback
            // an unbounded terminal would have.
            let num_pulled_blank = usize::from(num_pulled) - num_pulled_scrollback;
            let pulled = core::iter::repeat_with(new_row)
                .take(num_pulled_blank)
                .chain(scrollback.rows.drain(scrollback.rows.len() - num_pulled_scrollback..));
            self.rows.splice(0..0, pulled);
            self.rows.resize_with(size.rows().into(), new_row);
            scrollback.unbounded_len -= usize::from(num_pulled);
            // The blank rows stand in for dropped ones, which are on the screen now.
            scrollback.dropped = scrollback.dropped.saturating_sub(num_pulled_blank);
            num_pulled
        } else {
            self.rows.resize_with(size.rows().into(), new_row);
            0
        };

        if let Some(row) = self.rows.last_mut() {
            // The last row is maintained to always be a valid row
            row.wrap(false);
        }
        for row in &mut self.rows {
            row.resize(size.cols(), crate::vt100::Cell::new());
        }

        // Maintain the cursor position relative to the bottom-left corner of
        // the screen.
        if size.rows() < self.size.rows() {
            self.pos.row = self.pos.row.saturating_sub(pos_diff);
            self.saved_pos.row = self.saved_pos.row.saturating_sub(pos_diff).min(size.rows() - 1);
        } else {
            self.pos.row += pos_diff;
            self.saved_pos.row += pos_diff;
        }

        self.pos.col = self.pos.col.min(size.cols() - 1);
        self.saved_pos.col = self.saved_pos.col.min(size.cols() - 1);
        self.scroll_bottom = size.rows() - 1;
        self.size = size;

        if !is_empty {
            debug_assert_eq!(self.rows.len(), usize::from(self.size.rows()));
        }
        debug_assert!(self.pos.row < self.size.rows());
        debug_assert!(self.pos.col <= self.size.cols());
        debug_assert!(self.saved_pos.row < self.size.rows());
        debug_assert!(self.saved_pos.col < self.size.cols());
    }

    /// The last row that shows something or holds the cursor, following a wrapped row onto the next one.
    fn last_used_row(&self) -> u16 {
        let mut last = usize::from(self.pos.row);
        if let Some(i) = self.rows.iter().rposition(|row| !row.is_blank()) {
            last = last.max(i);
        }
        while last + 1 < self.rows.len() && self.rows[last].wrapped() {
            last += 1;
        }
        u16::try_from(last).unwrap_or(u16::MAX)
    }

    /// NOBLE: resizes the primary grid when the width changes. The scrollback and the rows up to the last used one
    /// are joined into logical lines (a run of wrapped rows), cut to the new width again (`reflow::rewrap`) and
    /// split into the new scrollback and the new screen. The screen keeps the top of the old one where it can, pulls
    /// history back when it grows and pushes rows into the scrollback when the text would not fit; blank rows below
    /// the last used one are dropped and padded again. The cursor and the saved cursor follow their cells; a cursor
    /// right after a full row stays in its pending-wrap column. `unbounded_len` keeps counting the rows the
    /// capacity already dropped.
    fn reflow_rows<F>(&mut self, size: Size, mut on_row: F)
    where
        F: FnMut(&crate::vt100::row::Row),
    {
        use crate::vt100::reflow::{Mark, rewrap};

        let last_used = usize::from(self.last_used_row());
        let old_cols = self.size.cols();
        self.rows.truncate(last_used + 1);
        let (mut old, dropped, capacity) = match &mut self.scrollback {
            Some(sb) => {
                let dropped = sb.unbounded_len.saturating_sub(sb.rows.len());
                (sb.rows.drain(..).collect::<Vec<_>>(), dropped, sb.capacity)
            }
            None => (Vec::new(), 0, 0),
        };
        self.epoch = self.epoch.wrapping_add(1);
        // The oldest history that the new width would push past the capacity is not rewrapped, only counted.
        let (skip, lost) =
            crate::vt100::reflow::skippable(&old, old_cols, size.cols(), capacity + usize::from(size.rows()));
        old.drain(..skip);
        let sb_len = old.len();
        old.append(&mut self.rows);
        let total = old.len();

        // Positions to carry over, as (row in `old`, column): the top of the old screen, the cursor and, when it is
        // inside the used part, the saved cursor.
        let saved_inside = usize::from(self.saved_pos.row) <= last_used;
        let wanted = [
            (sb_len, 0),
            (sb_len + usize::from(self.pos.row), self.pos.col),
            (sb_len + usize::from(self.saved_pos.row), self.saved_pos.col),
        ];
        let mut found: [Option<usize>; 3] = [None; 3];
        let mut found_col = [0u16; 3];

        let mut out: Vec<crate::vt100::row::Row> = Vec::with_capacity(total);
        let mut group: Vec<crate::vt100::row::Row> = Vec::new();
        // Without `pull_history` (Windows: ConPTY) a wider screen is repainted by conhost from its first row, which
        // conhost does not join to the history above it. The repaint starts with the rest of a line whose head is in
        // the scrollback and would write over the joined line, so the screen's first row begins a line of its own.
        let split_at_screen = !self.pull_history && size.cols() > old_cols && sb_len > 0;
        for (idx, row) in old.into_iter().enumerate() {
            let wrapped = row.wrapped() && !(split_at_screen && idx + 1 == sb_len);
            group.push(row);
            if wrapped && idx + 1 < total {
                continue;
            }
            let first = idx + 1 - group.len();
            let count = group.len();
            // A single hard line that fits the new width as it is (most rows of a history) is moved over as it is.
            if count == 1
                && !wrapped
                && !wanted.iter().enumerate().any(|(w, (row_at, _))| *row_at == first && (w < 2 || saved_inside))
                && group[0].fits(size.cols())
            {
                // Blank cells past the new width go (the screen rows are padded again below).
                for mut row in group.drain(..) {
                    row.trim();
                    out.push(row);
                }
                continue;
            }
            let mut cells: Vec<crate::vt100::Cell> = Vec::new();
            let mut marks = Vec::new();
            let mut which = Vec::new();
            for (k, row) in group.drain(..).enumerate() {
                let mut row_cells = row.into_cells();
                if k + 1 < count && row_cells.len() < usize::from(old_cols) {
                    // Scrollback rows are trimmed; a wrapped one must still be a full row when joined.
                    row_cells.resize(usize::from(old_cols), crate::vt100::Cell::new());
                }
                if k + 1 < count && row_cells.last().is_some_and(crate::vt100::Cell::is_leading_spacer) {
                    row_cells.pop();
                }
                for (w, (row_at, col)) in wanted.iter().enumerate() {
                    if *row_at == first + k && (w < 2 || saved_inside) {
                        let col = usize::from(*col);
                        let col = if k + 1 < count { col.min(row_cells.len()) } else { col };
                        marks.push(Mark { offset: cells.len() + col, at: None });
                        which.push(w);
                    }
                }
                cells.extend(row_cells);
            }
            // Trailing blank cells go, but not the ones a cursor stands on.
            let keep = marks.iter().map(|m| m.offset).max().unwrap_or(0);
            while cells.len() > keep && cells.last().is_some_and(|c| *c == crate::vt100::cell::Cell::DEFAULT) {
                cells.pop();
            }
            if cells.len() < keep {
                cells.resize(keep, crate::vt100::Cell::new());
            }
            let start = out.len();
            rewrap(cells, size.cols(), &mut marks, &mut out);
            for (mark, w) in marks.iter().zip(which) {
                let (line_row, col) = mark.at.unwrap_or_default();
                found[w] = Some(start + line_row);
                found_col[w] = col;
            }
        }

        let rows_old = usize::from(self.size.rows());
        let rows_new = usize::from(size.rows());
        let mut top = found[0].unwrap_or(0);
        let mut cur = found[1].unwrap_or(0);
        let mut saved = found[2].unwrap_or(0);

        // Growing: pull history back (not with `pull_history` off). Where the scrollback capacity already dropped rows, blank rows stand in for
        // them at the top, as without reflow.
        let want = if self.pull_history { rows_new.saturating_sub(rows_old) } else { 0 };
        let blanks = want.saturating_sub(top).min(dropped);
        if blanks > 0 {
            out.splice(0..0, core::iter::repeat_with(|| crate::vt100::row::Row::new(size.cols())).take(blanks));
            top += blanks;
            cur += blanks;
            saved += blanks;
        }
        let used = out.len();
        if !saved_inside {
            // A saved cursor below the used part keeps its distance from the last used row.
            saved = used - 1 + (usize::from(self.saved_pos.row) - last_used);
            found_col[2] = self.saved_pos.col;
        }
        // The screen starts where the old one did (less what a growth pulls back), but low enough to hold the
        // last used row.
        let start = top.saturating_sub(want).max(used.saturating_sub(rows_new));

        let mut screen = out.split_off(start);
        screen.truncate(rows_new);
        screen.resize_with(rows_new, || crate::vt100::row::Row::new(size.cols()));
        for row in &mut screen {
            // Rows from the rewrap (or moved over) can be shorter than the screen: they are padded to its width.
            row.grow(size.cols(), crate::vt100::Cell::new());
        }
        if let Some(last) = screen.last_mut() {
            last.wrap(false);
        }
        let history = out.len();
        // A cursor above the screen (text below it was pushed up) sits on the first row.
        let screen_row = |abs: usize| u16::try_from(abs.saturating_sub(start).min(rows_new - 1)).unwrap_or(0);
        self.pos = Pos { row: screen_row(cur), col: found_col[1].min(size.cols()) };
        self.saved_pos = Pos { row: screen_row(saved), col: found_col[2].min(size.cols() - 1) };

        // Without a scrollback (or past its capacity) the rows are gone.
        let overflow = history.saturating_sub(if self.scrollback.is_some() { capacity } else { 0 });
        for row in out.drain(..overflow) {
            on_row(&row);
        }
        if let Some(sb) = &mut self.scrollback {
            for row in &mut out {
                row.trim();
            }
            sb.rows = out.into();
            sb.unbounded_len = history + dropped + lost - blanks;
            // Rows past the capacity are gone; the blank stand-ins are on the screen now. Reflow also changed the
            // line numbering (the epoch), so the old count is only a base for the new one.
            sb.dropped = (sb.dropped + overflow + lost).saturating_sub(blanks);
            sb.offset = 0;
        }
        self.rows = screen;
        self.scroll_top = 0;
        self.scroll_bottom = size.rows() - 1;
        self.size = size;
    }

    pub fn pos(&self) -> Pos {
        self.pos
    }

    pub fn set_pos(&mut self, mut pos: Pos) {
        if self.origin_mode {
            pos.row = pos.row.saturating_add(self.scroll_top);
        }
        self.pos = pos;
        self.row_clamp_top(self.origin_mode);
        self.row_clamp_bottom(self.origin_mode);
        self.col_clamp();
    }

    pub fn save_cursor(&mut self) {
        self.saved_pos = self.pos;
        self.saved_origin_mode = self.origin_mode;
    }

    pub fn restore_cursor(&mut self) {
        self.pos = self.saved_pos;
        self.origin_mode = self.saved_origin_mode;
    }

    pub fn visible_rows(&self) -> impl Iterator<Item = &crate::vt100::row::Row> {
        self.scrollback
            .as_ref()
            .map(|s| s.rows.iter().skip(s.rows.len() - s.offset))
            .into_iter()
            .flatten()
            .chain(&self.rows)
            .take(self.rows.len())
    }

    pub fn drawing_rows(&self) -> impl Iterator<Item = &crate::vt100::row::Row> {
        self.rows.iter()
    }

    pub fn drawing_rows_mut(&mut self) -> impl Iterator<Item = &mut crate::vt100::row::Row> {
        self.rows.iter_mut()
    }

    pub fn visible_row(&self, row: u16) -> Option<&crate::vt100::row::Row> {
        self.visible_rows().nth(usize::from(row))
    }

    pub fn drawing_row(&self, row: u16) -> Option<&crate::vt100::row::Row> {
        self.drawing_rows().nth(usize::from(row))
    }

    pub fn drawing_row_mut(&mut self, row: u16) -> Option<&mut crate::vt100::row::Row> {
        self.drawing_rows_mut().nth(usize::from(row))
    }

    pub fn current_row_mut(&mut self) -> &mut crate::vt100::row::Row {
        self.drawing_row_mut(self.pos.row)
            // we assume self.pos.row is always valid
            .unwrap()
    }

    pub fn visible_cell(&self, pos: Pos) -> Option<&crate::vt100::Cell> {
        self.visible_row(pos.row).and_then(|r| r.get(pos.col))
    }

    pub fn drawing_cell(&self, pos: Pos) -> Option<&crate::vt100::Cell> {
        self.drawing_row(pos.row).and_then(|r| r.get(pos.col))
    }

    pub fn drawing_cell_mut(&mut self, pos: Pos) -> Option<&mut crate::vt100::Cell> {
        self.drawing_row_mut(pos.row).and_then(|r| r.get_mut(pos.col))
    }

    pub fn scrollback_capacity(&self) -> usize {
        self.scrollback.as_ref().map_or(0, |s| s.capacity)
    }

    pub fn scrollback_len(&self) -> usize {
        self.scrollback.as_ref().map_or(0, |s| s.rows.len())
    }

    pub fn scrollback(&self) -> usize {
        self.scrollback.as_ref().map_or(0, |s| s.offset)
    }

    /// Ensures every visible row of scrollback is at least the width of the
    /// screen.
    fn grow_visible_scrollback(scrollback: &mut ScrollbackState, size: Size) {
        let scrollback_len = scrollback.rows.len();
        scrollback.rows.iter_mut().skip(scrollback_len - scrollback.offset).take(size.rows().into()).for_each(|row| {
            row.grow(size.cols(), crate::vt100::cell::Cell::new());
        });
    }

    pub fn set_scrollback(&mut self, rows: usize) {
        let Some(scrollback) = &mut self.scrollback else {
            return;
        };
        scrollback.offset = rows.min(scrollback.rows.len());
        Self::grow_visible_scrollback(scrollback, self.size);
    }

    pub fn write_contents(&self, contents: &mut String) {
        let mut wrapping = false;
        for row in self.visible_rows() {
            row.write_contents(contents, 0, self.size.cols(), wrapping);
            if !row.wrapped() {
                contents.push('\n');
            }
            wrapping = row.wrapped();
        }

        while contents.ends_with('\n') {
            contents.truncate(contents.len() - 1);
        }
    }

    pub fn write_contents_formatted(&self, contents: &mut String) -> crate::vt100::attrs::Attrs {
        crate::vt100::term::ClearAttrs.write_buf(contents);
        crate::vt100::term::ClearScreen.write_buf(contents);

        let mut prev_attrs = crate::vt100::attrs::Attrs::default();
        let mut prev_pos = Pos::default();
        let mut wrapping = false;

        for (i, row) in self.visible_rows().enumerate() {
            // we limit the number of cols to a u16 (see Size), so
            // visible_rows() can never return more rows than will fit
            let i = i.try_into().unwrap();
            let (new_pos, new_attrs) = row.write_contents_formatted(
                contents,
                0,
                self.size.cols(),
                i,
                wrapping,
                Some(prev_pos),
                Some(prev_attrs),
            );
            prev_pos = new_pos;
            prev_attrs = new_attrs;
            wrapping = row.wrapped();
        }

        self.write_cursor_position_formatted(contents, Some(prev_pos), Some(prev_attrs));

        prev_attrs
    }

    pub fn write_contents_formatted_basic(
        &self,
        writer: &mut impl alloc::fmt::Write,
        range: crate::vt100::capture::BasicFormattedCaptureRange<'_>,
    ) -> alloc::fmt::Result {
        fn write<'a>(
            rows: impl Iterator<Item = &'a crate::vt100::row::Row>,
            writer: &mut impl alloc::fmt::Write,
            state: &mut crate::vt100::capture::BasicFormattedCaptureState,
        ) -> alloc::fmt::Result {
            rows.map(crate::vt100::capture::RowContents).try_for_each(|row| row.write_formatted_basic(writer, state))
        }
        match range {
            crate::vt100::capture::BasicFormattedCaptureRange::Full(state) => {
                if let Some(scrollback) = &self.scrollback {
                    write(scrollback.rows.iter(), writer, state)?;
                }
                write(self.rows.iter(), writer, state)
            }
            crate::vt100::capture::BasicFormattedCaptureRange::Visible => {
                write(self.visible_rows(), writer, &mut crate::vt100::capture::BasicFormattedCaptureState::new())
            }
        }
    }

    pub fn write_contents_diff(
        &self,
        contents: &mut String,
        prev: &Self,
        mut prev_attrs: crate::vt100::attrs::Attrs,
    ) -> crate::vt100::attrs::Attrs {
        let mut prev_pos = prev.pos;
        let mut wrapping = false;
        let mut prev_wrapping = false;
        for (i, (row, prev_row)) in self.visible_rows().zip(prev.visible_rows()).enumerate() {
            // we limit the number of cols to a u16 (see Size), so
            // visible_rows() can never return more rows than will fit
            let i = i.try_into().unwrap();
            let (new_pos, new_attrs) = row.write_contents_diff(
                contents,
                prev_row,
                0,
                self.size.cols(),
                i,
                wrapping,
                prev_wrapping,
                prev_pos,
                prev_attrs,
            );
            prev_pos = new_pos;
            prev_attrs = new_attrs;
            wrapping = row.wrapped();
            prev_wrapping = prev_row.wrapped();
        }

        self.write_cursor_position_formatted(contents, Some(prev_pos), Some(prev_attrs));

        prev_attrs
    }

    pub fn write_cursor_position_formatted(
        &self,
        contents: &mut String,
        prev_pos: Option<Pos>,
        prev_attrs: Option<crate::vt100::attrs::Attrs>,
    ) {
        let prev_attrs = prev_attrs.unwrap_or_default();
        // writing a character to the last column of a row doesn't wrap the
        // cursor immediately - it waits until the next character is actually
        // drawn. it is only possible for the cursor to have this kind of
        // position after drawing a character though, so if we end in this
        // position, we need to redraw the character at the end of the row.
        if prev_pos != Some(self.pos) && self.pos.col >= self.size.cols() {
            let mut pos = Pos { row: self.pos.row, col: self.size.cols() - 1 };
            if self
                .drawing_cell(pos)
                // we assume self.pos.row is always valid, and self.size.cols
                // - 1 is always a valid column
                .unwrap()
                .is_wide_continuation()
            {
                pos.col = self.size.cols() - 2;
            }
            let cell =
                // we assume self.pos.row is always valid, and self.size.cols
                // - 2 must be a valid column because self.size.cols - 1 is
                // always valid and we just checked that the cell at
                // self.size.cols - 1 is a wide continuation character, which
                // means that the first half of the wide character must be
                // before it
                self.drawing_cell(pos).unwrap();
            if cell.has_contents() {
                if let Some(prev_pos) = prev_pos {
                    crate::vt100::term::MoveFromTo::new(prev_pos, pos).write_buf(contents);
                } else {
                    crate::vt100::term::MoveTo::new(pos).write_buf(contents);
                }
                cell.attrs().write_escape_code_diff(contents, &prev_attrs);
                contents.push_str(cell.contents());
                prev_attrs.write_escape_code_diff(contents, cell.attrs());
            } else {
                // if the cell doesn't have contents, we can't have gotten
                // here by drawing a character in the last column. this means
                // that as far as i'm aware, we have to have reached here from
                // a newline when we were already after the end of an earlier
                // row. in the case where we are already after the end of an
                // earlier row, we can just write a few newlines, otherwise we
                // also need to do the same as above to get ourselves to after
                // the end of a row.
                let mut found = false;
                for i in (0..self.pos.row).rev() {
                    pos.row = i;
                    pos.col = self.size.cols() - 1;
                    if self
                        .drawing_cell(pos)
                        // i is always less than self.pos.row, which we assume
                        // to be always valid, so it must also be valid.
                        // self.size.cols - 1 is always a valid col.
                        .unwrap()
                        .is_wide_continuation()
                    {
                        pos.col = self.size.cols() - 2;
                    }
                    let cell = self
                        .drawing_cell(pos)
                        // i is always less than self.pos.row, which we assume
                        // to be always valid, so it must also be valid.
                        // self.size.cols - 2 is valid because self.size.cols
                        // - 1 is always valid, and col gets set to
                        // self.size.cols - 2 when the cell at self.size.cols
                        // - 1 is a wide continuation character, meaning that
                        // the first half of the wide character must be before
                        // it
                        .unwrap();
                    if cell.has_contents() {
                        if let Some(prev_pos) = prev_pos {
                            if prev_pos.row != i || prev_pos.col < self.size.cols() {
                                crate::vt100::term::MoveFromTo::new(prev_pos, pos).write_buf(contents);
                                cell.attrs().write_escape_code_diff(contents, &prev_attrs);
                                contents.push_str(cell.contents());
                                prev_attrs.write_escape_code_diff(contents, cell.attrs());
                            }
                        } else {
                            crate::vt100::term::MoveTo::new(pos).write_buf(contents);
                            cell.attrs().write_escape_code_diff(contents, &prev_attrs);
                            contents.push_str(cell.contents());
                            prev_attrs.write_escape_code_diff(contents, cell.attrs());
                        }
                        contents.push_str(&"\n".repeat(usize::from(self.pos.row - i)));
                        found = true;
                        break;
                    }
                }

                // this can happen if you get the cursor off the end of a row,
                // and then do something to clear the end of the current row
                // without moving the cursor (IL, DL, ED, EL, etc). we know
                // there can't be something in the last column because we
                // would have caught that above, so it should be safe to
                // overwrite it.
                if !found {
                    pos = Pos { row: self.pos.row, col: self.size.cols() - 1 };
                    if let Some(prev_pos) = prev_pos {
                        crate::vt100::term::MoveFromTo::new(prev_pos, pos).write_buf(contents);
                    } else {
                        crate::vt100::term::MoveTo::new(pos).write_buf(contents);
                    }
                    contents.push(' ');
                    // we know that the cell has no contents, but it still may
                    // have drawing attributes (background color, etc)
                    let end_cell = self
                        .drawing_cell(pos)
                        // we assume self.pos.row is always valid, and
                        // self.size.cols - 1 is always a valid column
                        .unwrap();
                    end_cell.attrs().write_escape_code_diff(contents, &prev_attrs);
                    crate::vt100::term::SaveCursor.write_buf(contents);
                    crate::vt100::term::Backspace.write_buf(contents);
                    crate::vt100::term::EraseChar::new(1).write_buf(contents);
                    crate::vt100::term::RestoreCursor.write_buf(contents);
                    prev_attrs.write_escape_code_diff(contents, end_cell.attrs());
                }
            }
        } else if let Some(prev_pos) = prev_pos {
            crate::vt100::term::MoveFromTo::new(prev_pos, self.pos).write_buf(contents);
        } else {
            crate::vt100::term::MoveTo::new(self.pos).write_buf(contents);
        }
    }

    pub fn erase_all(&mut self, attrs: crate::vt100::attrs::Attrs) {
        for row in self.drawing_rows_mut() {
            row.clear(attrs);
        }
    }

    pub fn erase_all_forward(&mut self, attrs: crate::vt100::attrs::Attrs) {
        let pos = self.pos;
        for row in self.drawing_rows_mut().skip(usize::from(pos.row) + 1) {
            row.clear(attrs);
        }

        self.erase_row_forward(attrs);
    }

    pub fn erase_all_backward(&mut self, attrs: crate::vt100::attrs::Attrs) {
        let pos = self.pos;
        for row in self.drawing_rows_mut().take(usize::from(pos.row)) {
            row.clear(attrs);
        }

        self.erase_row_backward(attrs);
    }

    pub fn erase_all_scrollback<F>(&mut self, mut on_row: F)
    where
        F: FnMut(&crate::vt100::row::Row),
    {
        if let Some(scrollback) = &mut self.scrollback {
            scrollback.dropped += scrollback.rows.len();
            for row in scrollback.rows.drain(..) {
                on_row(&row);
            }
            scrollback.unbounded_len = 0;
            scrollback.offset = 0;
        }
    }

    pub fn erase_row(&mut self, attrs: crate::vt100::attrs::Attrs) {
        self.current_row_mut().clear(attrs);
    }

    pub fn erase_row_forward(&mut self, attrs: crate::vt100::attrs::Attrs) {
        let size = self.size;
        let pos = self.pos;
        let row = self.current_row_mut();
        for col in pos.col..size.cols() {
            row.erase(col, attrs);
        }
    }

    pub fn erase_row_backward(&mut self, attrs: crate::vt100::attrs::Attrs) {
        let size = self.size;
        let pos = self.pos;
        let row = self.current_row_mut();
        for col in 0..=pos.col.min(size.cols() - 1) {
            row.erase(col, attrs);
        }
    }

    pub fn insert_cells(&mut self, count: u16) {
        let pos = self.pos;
        let row = self.current_row_mut();
        row.insert(pos.col, count);
    }

    pub fn delete_cells(&mut self, count: u16) {
        let size = self.size;
        let pos = self.pos;
        let row = self.current_row_mut();
        row.remove(pos.col..pos.col.saturating_add(count).min(size.cols()));
    }

    pub fn erase_cells(&mut self, count: u16, attrs: crate::vt100::attrs::Attrs) {
        let size = self.size;
        let pos = self.pos;
        let row = self.current_row_mut();
        for col in pos.col..((pos.col.saturating_add(count)).min(size.cols())) {
            row.erase(col, attrs);
        }
    }

    pub fn insert_lines(&mut self, count: u16) {
        if count == 0 {
            return;
        }
        if !(self.scroll_top..=self.scroll_bottom).contains(&self.pos.row) {
            return;
        }
        let between = &mut self.rows[usize::from(self.pos.row)..=usize::from(self.scroll_bottom)];
        let num_between = between.len();
        let num_removed = usize::from(count).min(num_between);
        between.rotate_right(num_removed);
        for row in &mut between[..num_removed] {
            row.reset();
        }
        // self.scroll_bottom is maintained to always be a valid row
        self.rows[usize::from(self.scroll_bottom)].wrap(false);
    }

    pub fn delete_lines(&mut self, count: u16) {
        if !(self.scroll_top..=self.scroll_bottom).contains(&self.pos.row) {
            return;
        }
        let between = &mut self.rows[usize::from(self.pos.row)..=usize::from(self.scroll_bottom)];
        let num_between = between.len();
        let num_removed = usize::from(count).min(num_between);
        for row in &mut between[..num_removed] {
            row.reset();
        }
        between.rotate_left(num_removed);
    }

    pub fn scroll_up<F>(&mut self, count: u16, mut on_row: F)
    where
        F: FnMut(&crate::vt100::row::Row),
    {
        // This can be true even when `self.scrollback` is `None`; it controls
        // whether we call the `on_row` callback.
        let scrollback_enabled = self.scroll_top == 0;
        let between = &mut self.rows[usize::from(self.scroll_top)..=usize::from(self.scroll_bottom)];
        let num_between = between.len();
        let count = usize::from(count).min(num_between);

        // Number of to-be-removed rows.
        let num_removed = count.min(num_between);
        let new_row = || crate::vt100::row::Row::new(self.size.cols());

        if let Some(scrollback) = self.scrollback.as_mut().filter(|_| scrollback_enabled) {
            scrollback.unbounded_len = scrollback.unbounded_len.saturating_add(num_removed);

            // Number of rows that will be pushed to the scrollback.
            let num_pushed = count.min(scrollback.capacity);

            // Scrollback length is capped at `scrollback.capacity`. These rows
            // will be lost, so just remove them now to avoid allocating.
            let scrollback_overflow = (scrollback.rows.len() + num_pushed).saturating_sub(scrollback.capacity);
            scrollback.dropped += scrollback_overflow + (count - num_pushed);
            for row in scrollback.rows.drain(0..scrollback_overflow) {
                on_row(&row);
            }

            // Number of rows that will simply be reset rather than pushed to
            // the scrollback.
            let num_reset = num_removed - num_pushed;

            for row in &mut between[..num_reset] {
                on_row(row);
                row.reset();
            }

            scrollback.rows.extend(between[num_reset..num_removed].iter_mut().map(|row| {
                row.trim();
                core::mem::replace(row, new_row())
            }));

            if scrollback.offset > 0 {
                scrollback.offset = scrollback.offset.saturating_add(count).min(scrollback.rows.len());
            }
            Self::grow_visible_scrollback(scrollback, self.size);
        } else {
            for row in &mut between[..num_removed] {
                if scrollback_enabled {
                    on_row(row);
                }
                row.reset();
            }
        }

        // Finally, rotate the rows so the "removed" ones (which have now been
        // reset to empty) are at the bottom.
        between.rotate_left(num_removed);
    }

    pub fn scroll_down(&mut self, count: u16) {
        if count == 0 {
            return;
        }
        let between = &mut self.rows[usize::from(self.scroll_top)..=usize::from(self.scroll_bottom)];
        let num_between = between.len();
        let num_removed = usize::from(count).min(num_between);
        between.rotate_right(num_removed);
        for row in &mut between[..num_removed] {
            row.reset();
        }
        // self.scroll_bottom is maintained to always be a valid row
        self.rows[usize::from(self.scroll_bottom)].wrap(false);
    }

    pub fn set_scroll_region(&mut self, top: u16, bottom: u16) {
        let bottom = bottom.min(self.size().rows() - 1);
        if top < bottom {
            self.scroll_top = top;
            self.scroll_bottom = bottom;
        } else {
            self.scroll_top = 0;
            self.scroll_bottom = self.size().rows() - 1;
        }
        self.pos.row = self.scroll_top;
        self.pos.col = 0;
    }

    fn in_scroll_region(&self) -> bool {
        self.pos.row >= self.scroll_top && self.pos.row <= self.scroll_bottom
    }

    pub fn set_origin_mode(&mut self, mode: bool) {
        self.origin_mode = mode;
        self.set_pos(Pos { row: 0, col: 0 });
    }

    pub fn row_inc_clamp(&mut self, count: u16) {
        let in_scroll_region = self.in_scroll_region();
        self.pos.row = self.pos.row.saturating_add(count);
        self.row_clamp_bottom(in_scroll_region);
    }

    pub fn row_inc_scroll<F>(&mut self, count: u16, on_row: F) -> u16
    where
        F: FnMut(&crate::vt100::row::Row),
    {
        let in_scroll_region = self.in_scroll_region();
        self.pos.row = self.pos.row.saturating_add(count);
        let lines = self.row_clamp_bottom(in_scroll_region);
        if in_scroll_region {
            self.scroll_up(lines, on_row);
            lines
        } else {
            0
        }
    }

    pub fn row_dec_clamp(&mut self, count: u16) {
        let in_scroll_region = self.in_scroll_region();
        self.pos.row = self.pos.row.saturating_sub(count);
        self.row_clamp_top(in_scroll_region);
    }

    pub fn row_dec_scroll(&mut self, count: u16) {
        let in_scroll_region = self.in_scroll_region();
        // need to account for clamping by both row_clamp_top and by
        // saturating_sub
        let extra_lines = count.saturating_sub(self.pos.row);
        self.pos.row = self.pos.row.saturating_sub(count);
        let lines = self.row_clamp_top(in_scroll_region);
        self.scroll_down(lines + extra_lines);
    }

    pub fn row_set(&mut self, i: u16) {
        self.pos.row = i;
        self.row_clamp();
    }

    pub fn col_inc(&mut self, count: u16) {
        self.pos.col = self.pos.col.saturating_add(count);
    }

    pub fn col_inc_clamp(&mut self, count: u16) {
        self.pos.col = self.pos.col.saturating_add(count);
        self.col_clamp();
    }

    pub fn col_dec(&mut self, count: u16) {
        self.pos.col = self.pos.col.saturating_sub(count);
    }

    pub fn col_tab(&mut self) {
        self.pos.col -= self.pos.col % 8;
        self.pos.col = self.pos.col.saturating_add(8);
        self.col_clamp();
    }

    pub fn col_set(&mut self, i: u16) {
        self.pos.col = i;
        self.col_clamp();
    }

    pub fn col_wrap<F>(&mut self, width: u16, wrap: bool, on_row: F)
    where
        F: FnMut(&crate::vt100::row::Row),
    {
        if self.pos.col <= self.size.cols() - width {
            // we haven't reached the end of the row yet
            return;
        }

        let mut prev_pos = self.pos;
        self.pos.col = 0;
        let scrolled = self.row_inc_scroll(1, on_row);
        if prev_pos.row == 0 && scrolled > 0 {
            // we're trying to scroll but we're already at the top of the
            // grid - this can happen when the grid height is set to 1. in
            // this case, we want to do the scrolling (via row_inc_scroll)
            // but don't want to set the wrap state (since it never makes
            // sense on a 1-height grid).
            return;
        }

        prev_pos.row -= scrolled;
        let new_pos = self.pos;
        let wrapped = wrap && prev_pos.row + 1 == new_pos.row;
        let cols = self.size.cols();
        let row = self
            .drawing_row_mut(prev_pos.row)
            // we assume self.pos.row is always valid, and so prev_pos.row
            // must be valid because it is always less than or equal to
            // self.pos.row
            .unwrap();
        row.wrap(wrapped);
        if wrapped && width > 1 && prev_pos.col + 1 == cols {
            // NOBLE: a wide character wrapped because only one cell was left; that empty last cell marks the
            // gap so the row can be joined with the next one without a stray space.
            if let Some(cell) = row.get_mut(prev_pos.col) {
                if !cell.has_contents() && !cell.is_wide_continuation() {
                    cell.set_leading_spacer(true);
                }
            }
        }
    }

    fn row_clamp_top(&mut self, limit_to_scroll_region: bool) -> u16 {
        if limit_to_scroll_region && self.pos.row < self.scroll_top {
            let rows = self.scroll_top - self.pos.row;
            self.pos.row = self.scroll_top;
            rows
        } else {
            0
        }
    }

    fn row_clamp_bottom(&mut self, limit_to_scroll_region: bool) -> u16 {
        let bottom = if limit_to_scroll_region { self.scroll_bottom } else { self.size.rows() - 1 };
        if self.pos.row > bottom {
            let rows = self.pos.row - bottom;
            self.pos.row = bottom;
            rows
        } else {
            0
        }
    }

    fn row_clamp(&mut self) {
        if self.pos.row > self.size.rows() - 1 {
            self.pos.row = self.size.rows() - 1;
        }
    }

    fn col_clamp(&mut self) {
        if self.pos.col > self.size.cols() - 1 {
            self.pos.col = self.size.cols() - 1;
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Size {
    pub rows: core::num::NonZeroU16,
    pub cols: core::num::NonZeroU16,
}

impl Size {
    pub fn rows(self) -> u16 {
        self.rows.get()
    }

    pub fn cols(self) -> u16 {
        self.cols.get()
    }
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct Pos {
    pub row: u16,
    pub col: u16,
}
