//! NOBLE: rewraps one logical line (the cells of a run of wrapped rows) to a new width. `Grid::reflow` groups the rows
//! into lines, calls this for each and places the cursors through the marks.

use crate::vt100::cell::Cell;
use crate::vt100::row::Row;

/// A position in a line that has to be found again after the rewrap: the index of a cell in the joined line before
/// (`offset`), and where that cell is afterwards (`at`: row within the line and column; the column is the new width
/// for a position just past a full row).
#[derive(Clone, Copy, Debug)]
pub struct Mark {
    pub offset: usize,
    pub at: Option<(usize, u16)>,
}

/// Cuts `cells` into rows of `width` cells and appends them to `out`. All rows but the last are marked as wrapped. A
/// wide character never straddles the edge: when only one cell is left, that cell becomes a leading spacer and the
/// character starts the next row. A line without cells is one blank row. The rows may be shorter than `width` (trailing blanks are not added).
pub fn rewrap(mut cells: Vec<Cell>, width: u16, marks: &mut [Mark], out: &mut Vec<Row>) {
    let width = usize::from(width);
    let n = cells.len();
    let mut row: Vec<Cell> = Vec::with_capacity(width.min(n));
    let mut line_row = 0;
    let mut i = 0;
    while i < n {
        // Fast path: a run of ordinary cells that fits in the row and holds no mark is copied in one go.
        let next_mark = marks.iter().filter(|m| m.at.is_none() && m.offset >= i).map(|m| m.offset).min();
        let limit = (i + (width - row.len())).min(next_mark.unwrap_or(n));
        let mut j = i;
        while j < limit && !cells[j].is_wide() && !cells[j].is_wide_continuation() {
            j += 1;
        }
        if j > i {
            row.extend_from_slice(&cells[i..j]);
            i = j;
            continue;
        }
        // A wide character is its cell plus the continuation after it; anything else that claims to be half of
        // one (a cut pair) is blanked.
        let mut taken = 1;
        let mut unit = 1;
        if cells[i].is_wide() && cells.get(i + 1).is_some_and(Cell::is_wide_continuation) {
            taken = 2;
            unit = 2;
        } else if cells[i].is_wide() || cells[i].is_wide_continuation() {
            let attrs = *cells[i].attrs();
            cells[i].clear(attrs);
        }
        if unit > width {
            // A wide character on a one-column screen: keep its place as a blank.
            let attrs = *cells[i].attrs();
            cells[i].clear(attrs);
            unit = 1;
        }
        if row.len() + unit > width {
            if unit == 2 && row.len() + 1 == width {
                let mut spacer = Cell::new();
                spacer.set_leading_spacer(true);
                row.push(spacer);
            }
            finish(&mut row, true, out);
            line_row += 1;
        }
        let col = u16::try_from(row.len()).unwrap_or(u16::MAX);
        for mark in marks.iter_mut() {
            if mark.at.is_none() && mark.offset >= i && mark.offset < i + taken {
                let inside = u16::from(mark.offset > i && unit == 2);
                mark.at = Some((line_row, col + inside));
            }
        }
        if taken == 2 && unit == 1 {
            row.push(cells[i].clone());
        } else {
            row.extend_from_slice(&cells[i..i + taken]);
        }
        i += taken;
    }
    let end = (line_row, u16::try_from(row.len()).unwrap_or(u16::MAX));
    for mark in marks.iter_mut() {
        mark.at.get_or_insert(end);
    }
    finish(&mut row, false, out);
}

/// Ends a row. It is not padded to the width: the rows that stay on the screen are padded by the caller and the ones
/// that go to the scrollback are trimmed anyway, so a history of many short rows costs no blank cells.
fn finish(row: &mut Vec<Cell>, wrapped: bool, out: &mut Vec<Row>) {
    out.push(Row::from_cells(core::mem::take(row), wrapped));
}

/// How many of the oldest `history` rows can be left out of a reflow to `new_cols` columns because the newest rows
/// alone already fill `keep` rows of the new width, and an estimate of the rows they would have made (for the count of
/// lines that scrolled out). Returns `(rows to skip, estimate)`; the skipped rows end a line, so no line is cut.
///
/// Every cell of a line needs room in the new width, so the cells of the rows behind a point divided by `new_cols` are
/// a lower bound of the rows they make (a wrapped row counts one cell less: it can end in a spacer). Wide characters
/// only add rows. Skipping is exact as long as the bound is met: those rows would have been cut by the capacity.
pub fn skippable(history: &[Row], old_cols: u16, new_cols: u16, keep: usize) -> (usize, usize) {
    let cells_of = |row: &Row| if row.wrapped() { usize::from(old_cols).saturating_sub(1) } else { row.used_len() };
    let need = keep.saturating_mul(usize::from(new_cols));
    let mut behind = 0usize;
    let mut at = history.len();
    while at > 0 && behind < need {
        at -= 1;
        behind += cells_of(&history[at]);
    }
    if behind < need {
        return (0, 0);
    }
    // Back to the start of the line the point is in.
    while at > 0 && history[at - 1].wrapped() {
        at -= 1;
    }
    let skipped: usize = history[..at].iter().map(cells_of).sum();
    (at, skipped / usize::from(new_cols))
}
