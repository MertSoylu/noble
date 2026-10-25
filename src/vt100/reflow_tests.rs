//! Tests for reflow on a width change, and for the leading spacer of wrapped wide characters.
//!
//! The reference model is deliberately simple: a buffer is a list of logical lines, and the screen it should show
//! at width N is each line cut into rows of at most N cells (a wide character never straddles the edge). The tests
//! that need the real reflow are `#[ignore]`d until `Grid::set_size` reflows the primary grid.
//!
//! Wide characters are written as escapes: ZH is U+4E2D (a double-width CJK character), WEN U+6587, RI U+65E5,
//! BEN U+672C and GO U+8A9E.

use crate::vt100::Parser;
use core::num::NonZeroU16;
use unicode_width::UnicodeWidthChar as _;

const ZH: &str = "\u{4e2d}";
const WEN: &str = "\u{6587}";
const RI: &str = "\u{65e5}";
const BEN: &str = "\u{672c}";
const GO: &str = "\u{8a9e}";

fn nz(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n).unwrap()
}

/// One physical row: its text without trailing blanks, and whether it continues on the next row.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RefRow {
    text: String,
    wrapped: bool,
}

fn row(text: &str, wrapped: bool) -> RefRow {
    RefRow { text: text.to_string(), wrapped }
}

/// Reference wrap: cuts every logical line into rows of at most `width` cells. A wide character that does not fit
/// moves to the next row. An empty line is one empty row.
fn wrap_to_width(lines: &[&str], width: u16) -> Vec<RefRow> {
    let width = usize::from(width);
    let mut rows = Vec::new();
    for line in lines {
        let mut text = String::new();
        let mut used = 0;
        for c in line.chars() {
            let w = c.width().unwrap_or(0);
            if w > 0 && used + w > width {
                rows.push(RefRow { text: std::mem::take(&mut text), wrapped: true });
                used = 0;
            }
            text.push(c);
            used += w;
        }
        rows.push(RefRow { text, wrapped: false });
    }
    rows
}

/// Every row of the buffer, scrollback first, with trailing blank unwrapped rows (the unused part of the screen)
/// dropped. The scrollback position is restored.
fn all_rows(parser: &mut Parser) -> Vec<RefRow> {
    let (screen_rows, cols) = {
        let (r, c) = parser.screen().size();
        (usize::from(r.get()), c.get())
    };
    let total = parser.screen().scrollback_len();
    let before = parser.screen().scrollback();
    let mut rows = Vec::new();
    for a in 0..total + screen_rows {
        // Show the buffer so that absolute row `a` is the first visible row (or a live row, past the history).
        let (offset, i) = if a < total { (total - a, 0) } else { (0, a - total) };
        parser.screen_mut().set_scrollback(offset);
        let screen = parser.screen();
        let text = screen.rows(0, cols).nth(i).unwrap_or_default();
        rows.push(RefRow { text, wrapped: screen.row_wrapped(u16::try_from(i).unwrap()) });
    }
    parser.screen_mut().set_scrollback(before);
    while rows.last().is_some_and(|r| r.text.is_empty() && !r.wrapped) {
        rows.pop();
    }
    rows
}

/// A wide character is always followed by its continuation cell, and a continuation always follows a wide one.
fn assert_wide_cells_whole(parser: &Parser) {
    let (rows, cols) = parser.screen().size();
    for r in 0..rows.get() {
        for c in 0..cols.get() {
            let cell = parser.screen().cell(r, c).unwrap();
            if cell.is_wide() {
                let next = parser.screen().cell(r, c + 1);
                assert!(next.is_some_and(|n| n.is_wide_continuation()), "wide cell without continuation at {r},{c}");
            }
            if cell.is_wide_continuation() {
                let prev = c.checked_sub(1).and_then(|p| parser.screen().cell(r, p));
                assert!(prev.is_some_and(|p| p.is_wide()), "orphan continuation at {r},{c}");
            }
        }
    }
}

/// A parser fed with the lines, one per logical line (CR LF between them, none after the last).
fn fed(rows: u16, cols: u16, lines: &[&str]) -> Parser {
    let mut parser = Parser::new(nz(rows), nz(cols), 10_000);
    parser.process(lines.join("\r\n").as_bytes());
    parser
}

/// A mixed line of narrow and wide characters.
fn mixed() -> String {
    format!("ab{ZH}{WEN}cd{RI}{BEN}{GO}ef")
}

// reference model

#[test]
fn reference_wraps_and_keeps_wide_characters_whole() {
    assert_eq!(wrap_to_width(&["abcde"], 5), vec![row("abcde", false)]);
    assert_eq!(wrap_to_width(&["abcdef"], 5), vec![row("abcde", true), row("f", false)]);
    assert_eq!(wrap_to_width(&["", "a"], 5), vec![row("", false), row("a", false)]);
    assert_eq!(wrap_to_width(&[&format!("abcd{ZH}")], 5), vec![row("abcd", true), row(ZH, false)]);
    assert_eq!(wrap_to_width(&[&format!("abc{ZH}")], 5), vec![row(&format!("abc{ZH}"), false)]);
}

#[test]
fn the_emulator_agrees_with_the_reference_without_a_resize() {
    let mixed = mixed();
    for width in [3, 4, 5, 7, 10] {
        let lines = ["hello world", "", &mixed, "x", "0123456789abcdef"];
        let mut parser = fed(6, width, &lines);
        assert_eq!(all_rows(&mut parser), wrap_to_width(&lines, width), "width {width}");
        assert_wide_cells_whole(&parser);
    }
}

// reflow

#[test]
fn shrink_then_grow_keeps_text() {
    let lines = ["0123456789abcdefghijklmnopqrst", "short", "the quick brown fox jumps over the lazy dog again"];
    let mut parser = fed(8, 80, &lines);
    assert_eq!(all_rows(&mut parser), wrap_to_width(&lines, 80));

    parser.set_size(nz(8), nz(20));
    assert_eq!(all_rows(&mut parser), wrap_to_width(&lines, 20));
    parser.set_size(nz(8), nz(7));
    assert_eq!(all_rows(&mut parser), wrap_to_width(&lines, 7));
    parser.set_size(nz(8), nz(80));
    assert_eq!(all_rows(&mut parser), wrap_to_width(&lines, 80));
    // the text itself (what a copy gives) is the same throughout
    assert!(parser.screen().contents().contains("the quick brown fox jumps over the lazy dog again"));
}

#[test]
fn wide_characters_stay_together_at_the_new_edge() {
    let mixed = mixed();
    let all_wide = ZH.repeat(5);
    let lines = [mixed.as_str(), all_wide.as_str()];
    let mut parser = fed(6, 40, &lines);
    for width in [5, 4, 3, 2, 9, 40] {
        parser.set_size(nz(6), nz(width));
        assert_eq!(all_rows(&mut parser), wrap_to_width(&lines, width), "width {width}");
        assert_wide_cells_whole(&parser);
    }
}

#[test]
fn a_hard_newline_is_never_merged() {
    let lines = ["abc", "def", "ghijkl"];
    let mut parser = fed(6, 4, &lines);
    parser.set_size(nz(6), nz(20));
    assert_eq!(all_rows(&mut parser), vec![row("abc", false), row("def", false), row("ghijkl", false)]);
    assert_eq!(parser.screen().contents().trim_end(), "abc\ndef\nghijkl");
}

#[test]
fn a_short_row_before_a_hard_break_is_not_merged_with_the_next() {
    // "abcde" fills a 5-column row exactly and ends with a hard break: it must stay apart from "fg" at any width.
    let lines = ["ab", "cd", "abcde", "fg"];
    let mut parser = fed(6, 5, &lines);
    for width in [10, 3, 5, 80] {
        parser.set_size(nz(6), nz(width));
        assert_eq!(all_rows(&mut parser), wrap_to_width(&lines, width), "width {width}");
    }
}

// leading spacer

#[test]
fn a_wide_character_that_wraps_leaves_a_spacer() {
    let mut parser = Parser::new(nz(4), nz(5), 100);
    let text = format!("aaaa{ZH}b");
    parser.process(text.as_bytes());
    let screen = parser.screen();
    assert!(screen.cell(0, 4).unwrap().is_leading_spacer());
    assert!(!screen.cell(0, 4).unwrap().has_contents());
    assert!(screen.row_wrapped(0));
    assert_eq!(screen.cell(1, 0).unwrap().contents(), ZH);
    assert!(screen.cell(1, 0).unwrap().is_wide());
    // the spacer is never text: the logical line reads straight through
    assert_eq!(screen.contents().trim_end(), text);
    assert_eq!(screen.contents_between(0, 0, 1, 5), text);
    assert_eq!(all_rows(&mut parser), wrap_to_width(&[&text], 5));
}

#[test]
fn a_wide_character_in_the_last_two_columns_needs_no_spacer() {
    let mut parser = Parser::new(nz(4), nz(5), 100);
    parser.process(format!("aaa{ZH}").as_bytes());
    let screen = parser.screen();
    assert!(!screen.cell(0, 4).unwrap().is_leading_spacer());
    assert!(!screen.row_wrapped(0));
    assert_eq!(screen.cell(0, 3).unwrap().contents(), ZH);
}

#[test]
fn a_narrow_character_that_wraps_has_no_spacer() {
    let mut parser = Parser::new(nz(4), nz(5), 100);
    parser.process(b"aaaaabb");
    assert!(parser.screen().row_wrapped(0));
    assert!(!parser.screen().cell(0, 4).unwrap().is_leading_spacer());
}

#[test]
fn a_cell_widened_by_an_emoji_selector_wraps_with_a_spacer() {
    let mut parser = Parser::new(nz(4), nz(5), 100);
    parser.process("aaaa\u{2764}\u{FE0F}".as_bytes());
    let screen = parser.screen();
    assert!(screen.cell(0, 4).unwrap().is_leading_spacer());
    assert!(screen.row_wrapped(0));
    assert_eq!(screen.cell(1, 0).unwrap().contents(), "\u{2764}\u{FE0F}");
    assert_eq!(screen.contents().trim_end(), "aaaa\u{2764}\u{FE0F}");
}

#[test]
fn the_spacer_survives_scrolling_and_goes_when_the_cell_is_written() {
    let mut parser = Parser::new(nz(2), nz(5), 100);
    parser.process(format!("xxxxx\r\nbbbb{ZH}").as_bytes());
    // the wide character wrapped at the bottom row, so the grid scrolled once
    assert!(parser.screen().cell(0, 4).unwrap().is_leading_spacer());
    assert_eq!(parser.screen().cell(1, 0).unwrap().contents(), ZH);
    parser.process(b"\x1b[1;5Hz");
    assert!(!parser.screen().cell(0, 4).unwrap().is_leading_spacer());
    assert_eq!(parser.screen().cell(0, 4).unwrap().contents(), "z");
}

#[test]
fn erasing_the_last_cell_clears_the_spacer() {
    let mut parser = Parser::new(nz(4), nz(5), 100);
    parser.process(format!("aaaa{ZH}").as_bytes());
    parser.process(b"\x1b[1;5H\x1b[K");
    assert!(!parser.screen().cell(0, 4).unwrap().is_leading_spacer());
}

#[test]
fn a_width_change_without_reflow_drops_the_spacer_with_the_wrapped_flag() {
    // The alternate screen (and the primary one until it reflows) truncates and pads rows, which clears `wrapped`.
    for new_cols in [4, 9] {
        let mut parser = Parser::new(nz(4), nz(5), 100);
        parser.process(format!("aaaa{ZH}").as_bytes());
        parser.set_size(nz(4), nz(new_cols));
        for c in 0..new_cols {
            assert!(!parser.screen().cell(0, c).unwrap().is_leading_spacer(), "cols {new_cols} col {c}");
        }
    }
}

// cursor, history and rows

#[test]
fn the_cursor_follows_its_cell_through_a_width_change() {
    // "abcdefghij" at 10 columns, cursor in the middle of it.
    let mut parser = fed(5, 10, &["abcdefghij"]);
    parser.process(b"\x1b[1;4H");
    assert_eq!(parser.screen().cursor_position(), (0, 3));
    parser.set_size(nz(5), nz(4));
    // the cell 'd' is the last of the first row
    assert_eq!(parser.screen().cursor_position(), (0, 3));
    parser.set_size(nz(5), nz(3));
    assert_eq!(parser.screen().cursor_position(), (1, 0));
    parser.set_size(nz(5), nz(10));
    assert_eq!(parser.screen().cursor_position(), (0, 3));
    assert_eq!(parser.screen().cell(0, 3).unwrap().contents(), "d");
}

#[test]
fn a_cursor_at_the_end_of_a_line_stays_at_the_end() {
    let mut parser = fed(5, 20, &["$ ls"]);
    assert_eq!(parser.screen().cursor_position(), (0, 4));
    parser.set_size(nz(5), nz(3));
    // "$ l" / "s": the cursor is after the 's'
    assert_eq!(parser.screen().cursor_position(), (1, 1));
    parser.set_size(nz(5), nz(20));
    assert_eq!(parser.screen().cursor_position(), (0, 4));
    // typing continues right after the text
    parser.process(b"x");
    assert_eq!(parser.screen().contents().trim_end(), "$ lsx");
}

#[test]
fn a_cursor_after_a_full_row_waits_to_wrap() {
    let mut parser = fed(5, 8, &["abcdefgh"]);
    assert_eq!(parser.screen().cursor_position(), (0, 8));
    parser.set_size(nz(5), nz(4));
    assert_eq!(parser.screen().cursor_position(), (1, 4));
    parser.process(b"i");
    assert_eq!(all_rows(&mut parser), wrap_to_width(&["abcdefghi"], 4));
}

#[test]
fn the_saved_cursor_is_remapped() {
    let mut parser = fed(6, 10, &["0123456789", "xy"]);
    parser.process(b"\x1b[1;8H\x1b7\x1b[2;1H");
    parser.set_size(nz(6), nz(4));
    // "0123" "4567" "89" "xy": the saved cell '7' is the last of the second row
    parser.process(b"\x1b8");
    assert_eq!(parser.screen().cursor_position(), (1, 3));
    parser.set_size(nz(6), nz(10));
    parser.process(b"\x1b8");
    assert_eq!(parser.screen().cursor_position(), (0, 7));
}

#[test]
fn the_scrollback_is_reflowed() {
    let lines: Vec<String> = (0..12).map(|i| format!("line {i} with some words in it")).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut parser = fed(4, 40, &refs);
    assert!(parser.screen().scrollback_len() >= 8);
    for width in [10, 7, 25, 40] {
        parser.set_size(nz(4), nz(width));
        assert_eq!(all_rows(&mut parser), wrap_to_width(&refs, width), "width {width}");
    }
}

#[test]
fn the_reflowed_scrollback_respects_its_capacity() {
    let lines: Vec<String> = (0..30).map(|i| format!("row {i:02}")).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut parser = Parser::new(nz(4), nz(20), 10);
    parser.process(refs.join("\r\n").as_bytes());
    assert_eq!(parser.screen().scrollback_len(), 10);
    // each line now takes two rows: the history is cut to its capacity and keeps the newest rows
    parser.set_size(nz(4), nz(4));
    assert_eq!(parser.screen().scrollback_len(), 10);
    let rows = all_rows(&mut parser);
    let expected = wrap_to_width(&refs, 4);
    assert_eq!(rows, expected[expected.len() - rows.len()..]);
    assert!(parser.screen().contents().contains("29"));
}

#[test]
fn unbounded_history_is_pulled_back_after_a_reflow() {
    // The capacity drops old rows; a taller screen still shows blank rows in their place, as it does without a
    // reflow, and a screen as tall as before shows the same lines.
    let lines: Vec<String> = (0..40).map(|i| format!("r{i:02}")).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut parser = Parser::new(nz(5), nz(20), 6);
    parser.process(refs.join("\r\n").as_bytes());
    parser.set_size(nz(5), nz(10));
    parser.set_size(nz(5), nz(20));
    let before = all_rows(&mut parser);
    assert_eq!(before.last().unwrap().text, "r39");
    parser.set_size(nz(12), nz(20));
    // 12 rows: the last 5 lines plus 6 pulled-back ones and one blank stand-in for a dropped one
    assert_eq!(parser.screen().cursor_position().0, 11);
    let screen = parser.screen().rows(0, 20).collect::<Vec<_>>();
    assert_eq!(screen[0], "");
    assert_eq!(screen[1], "r29");
    assert_eq!(screen[11], "r39");
    parser.set_size(nz(5), nz(20));
    assert_eq!(all_rows(&mut parser), before);
}

#[test]
fn the_alternate_screen_still_cuts_and_pads() {
    let mut parser = Parser::new(nz(4), nz(10), 100);
    parser.process(b"\x1b[?1049habcdefghij");
    assert!(parser.screen().alternate_screen());
    parser.set_size(nz(4), nz(5));
    assert_eq!(parser.screen().contents().trim_end(), "abcde");
    parser.set_size(nz(4), nz(10));
    assert_eq!(parser.screen().contents().trim_end(), "abcde");
    parser.process(b"\x1b[?1049l");
    assert_eq!(parser.screen().scrollback_len(), 0);
}

#[test]
fn a_resize_resets_the_scroll_region() {
    let mut parser = fed(6, 10, &["one", "two"]);
    parser.process(b"\x1b[2;4r");
    parser.set_size(nz(6), nz(5));
    // with a region left over, a line feed at the last row would not scroll the whole screen
    parser.process(b"\x1b[6;1H\r\nz");
    assert_eq!(parser.screen().scrollback_len(), 1);
    assert_eq!(parser.screen().contents().trim_end().lines().last(), Some("z"));
}

#[test]
fn blank_rows_below_the_text_are_dropped_and_padded_again() {
    let mut parser = fed(10, 10, &["abcdefghijklmno"]);
    parser.set_size(nz(10), nz(5));
    assert_eq!(parser.screen().scrollback_len(), 0);
    assert_eq!(parser.screen().cursor_position(), (2, 5));
    parser.set_size(nz(10), nz(15));
    assert_eq!(parser.screen().scrollback_len(), 0);
    assert_eq!(all_rows(&mut parser), vec![row("abcdefghijklmno", false)]);
}

#[test]
fn text_below_the_cursor_goes_to_the_scrollback_on_a_row_shrink() {
    let lines = ["a", "b", "c", "d", "e", "f"];
    let mut parser = fed(6, 10, &lines);
    // the cursor goes up to the second row; the text below it must not be cut off
    parser.process(b"\x1b[2;1H");
    parser.set_size(nz(3), nz(10));
    assert_eq!(parser.screen().scrollback_len(), 3);
    assert_eq!(all_rows(&mut parser), wrap_to_width(&lines, 10));
    assert!(parser.screen().contents().contains('f'));
    parser.set_size(nz(6), nz(10));
    assert_eq!(parser.screen().scrollback_len(), 0);
    assert_eq!(all_rows(&mut parser), wrap_to_width(&lines, 10));
}

#[test]
fn the_bottom_line_survives_a_row_shrink_and_grow() {
    let lines = ["1", "2", "3", "4", "5", "6", "7", "8"];
    let mut parser = fed(8, 10, &lines);
    for rows in [4, 2, 8, 3, 12, 8] {
        parser.set_size(nz(rows), nz(10));
        assert_eq!(all_rows(&mut parser), wrap_to_width(&lines, 10), "rows {rows}");
    }
    // the same with a width change in the same step
    for (rows, cols) in [(4, 5), (2, 3), (8, 10)] {
        parser.set_size(nz(rows), nz(cols));
        assert_eq!(all_rows(&mut parser), wrap_to_width(&lines, cols), "{rows}x{cols}");
    }
}

#[test]
fn without_reflow_a_width_change_cuts_and_pads() {
    let mut parser = fed(4, 10, &["abcdefghij"]);
    parser.screen_mut().set_reflow(false);
    parser.set_size(nz(4), nz(4));
    assert_eq!(parser.screen().contents().trim_end(), "abcd");
    parser.set_size(nz(4), nz(10));
    assert_eq!(parser.screen().contents().trim_end(), "abcd");
}

#[test]
fn many_random_resizes_lose_nothing() {
    // A small deterministic generator: widths and heights jump around, nothing is written in between.
    let mixed = mixed();
    let lines = ["hello world this is a long line", "", mixed.as_str(), "x", "0123456789abcdef0123456789abcdef"];
    let mut parser = fed(6, 30, &lines);
    let mut seed = 0x2545_f491_u32;
    for _ in 0..200 {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let cols = u16::try_from((seed >> 16) % 40).unwrap() + 2;
        let rows = u16::try_from((seed >> 8) % 12).unwrap() + 1;
        parser.set_size(nz(rows), nz(cols));
        assert_wide_cells_whole(&parser);
        assert_eq!(all_rows(&mut parser), wrap_to_width(&lines, cols), "{rows}x{cols}");
        let (r, c) = parser.screen().cursor_position();
        assert!(r < rows && c <= cols);
    }
}

/// The stable number of the first live row: `history_dropped() + scrollback_len()`.
fn base(parser: &Parser) -> usize {
    parser.screen().history_dropped() + parser.screen().scrollback_len()
}

#[test]
fn history_dropped_counts_lines_that_left_a_full_scrollback() {
    let mut parser = Parser::new(nz(3), nz(10), 4);
    assert_eq!(parser.screen().history_dropped(), 0);
    for i in 0..4 {
        parser.process(format!("l{i}\r\n").as_bytes());
    }
    // Four lines in the scrollback at most; `l0` is its oldest line and nothing has left yet.
    assert_eq!((parser.screen().scrollback_len(), parser.screen().history_dropped()), (2, 0));
    let first_live = base(&parser);
    for i in 4..14 {
        parser.process(format!("l{i}\r\n").as_bytes());
    }
    assert_eq!(parser.screen().scrollback_len(), 4);
    // 14 lines were written, the cursor is on the 15th: 12 rows scrolled up, 4 are kept.
    assert_eq!(parser.screen().history_dropped(), 8);
    // The numbers never move: the first live row of before is `first_live` lines from the top, still.
    assert_eq!(base(&parser) - first_live, 10);
    assert_eq!(parser.screen().reflow_epoch(), 0);
}

#[test]
fn history_dropped_counts_an_erased_scrollback() {
    let mut parser = Parser::new(nz(3), nz(10), 100);
    for i in 0..8 {
        parser.process(format!("l{i}\r\n").as_bytes());
    }
    let before = base(&parser);
    parser.process(b"\x1b[3J");
    assert_eq!(parser.screen().scrollback_len(), 0);
    assert_eq!(base(&parser), before, "the live rows keep their numbers");
}

/// The stable number of the live row that shows `text`.
fn number_of(parser: &Parser, text: &str) -> usize {
    let row = parser.screen().rows(0, 10).position(|r| r.trim_end() == text).unwrap();
    base(parser) + row
}

/// A row grow pulls history back onto the screen; the lines keep their numbers, and so does a shrink.
#[test]
fn line_numbers_survive_a_row_grow_and_shrink() {
    let mut parser = Parser::new(nz(3), nz(10), 100);
    for i in 0..10 {
        parser.process(format!("l{i}\r\n").as_bytes());
    }
    let at = number_of(&parser, "l9");
    assert_eq!(at, 9);
    parser.set_size(nz(7), nz(10));
    assert_eq!(number_of(&parser, "l9"), at, "rows pulled back from the scrollback are numbered as before");
    assert_eq!(base(&parser), 4);
    parser.set_size(nz(2), nz(10));
    assert_eq!(number_of(&parser, "l9"), at);
    assert_eq!(parser.screen().reflow_epoch(), 0, "a height change does not renumber");

    // Where the capacity had dropped lines, blank stand-ins are pulled back and the count follows.
    let mut parser = Parser::new(nz(3), nz(10), 2);
    for i in 0..10 {
        parser.process(format!("l{i}\r\n").as_bytes());
    }
    let at = number_of(&parser, "l9");
    parser.set_size(nz(8), nz(10));
    assert_eq!(number_of(&parser, "l9"), at);
    assert_eq!(
        parser.screen().history_dropped(),
        3,
        "8 scrolled up, 2 kept, 5 came back: 2 kept and 3 blank stand-ins"
    );
}

#[test]
fn the_epoch_moves_with_a_reflow_and_a_reset_only() {
    let mut parser = fed(4, 10, &["abcdefghij", "klm"]);
    assert_eq!(parser.screen().reflow_epoch(), 0);
    parser.set_size(nz(6), nz(10));
    parser.process(b"more\r\n\r\n\r\n\r\n\r\n\r\n");
    assert_eq!(parser.screen().reflow_epoch(), 0);
    parser.set_size(nz(6), nz(5));
    assert_eq!(parser.screen().reflow_epoch(), 1);
    parser.set_size(nz(6), nz(5));
    assert_eq!(parser.screen().reflow_epoch(), 1, "the same size changes nothing");
    parser.process(b"\x1bc");
    assert_eq!(parser.screen().reflow_epoch(), 2, "a full reset starts the numbering over");
    assert_eq!(parser.screen().history_dropped(), 0);
    // The main screen behind the alternate one reflows all the same (its epoch moves); the alternate one never does.
    parser.process(b"\x1b[?1049h");
    parser.set_size(nz(6), nz(8));
    assert_eq!(parser.screen().reflow_epoch(), 3);
}

/// Selecting across the whole screen after a width change copies a soft-wrapped line as one line, not split where
/// the new width cut it.
#[test]
fn contents_between_does_not_break_a_soft_wrapped_line() {
    let mut parser = fed(6, 30, &["the quick brown fox jumps", "next"]);
    for cols in [12, 7, 20] {
        parser.set_size(nz(8), nz(cols));
        let (rows, cols) = parser.screen().size();
        let text = parser.screen().contents_between(0, 0, rows.get() - 1, cols.get());
        let lines: Vec<&str> = text.lines().map(str::trim_end).filter(|l| !l.is_empty()).collect();
        assert_eq!(lines, ["the quick brown fox jumps", "next"], "{cols} columns");
    }
}

/// A blank row that continues on the next one is trimmed to nothing in the scrollback; showing it (scrolling back
/// grows it to the screen's width again) must not turn it into a hard line break. Found by the reflow fuzz test: a
/// line with leading blanks lost its first row's flag and came back as two lines.
#[test]
fn showing_a_trimmed_wrapped_row_keeps_it_wrapped() {
    let mut parser = Parser::new(nz(4), nz(30), 100);
    parser.process(b"first\r\n\x1b[8Cx\r\n\r\n\r\n\r\n\r\nlast");
    parser.set_size(nz(4), nz(5));
    let before = all_rows(&mut parser);
    assert!(before.iter().any(|r| r.wrapped && r.text.is_empty()), "{before:?}");
    // Look at every part of the history.
    for offset in [1, 3, 9, 100, 0] {
        parser.screen_mut().set_scrollback(offset);
    }
    assert_eq!(all_rows(&mut parser), before);
    parser.set_size(nz(4), nz(30));
    let rows = all_rows(&mut parser);
    assert_eq!(rows[1], row("        x", false), "{rows:?}");
}

/// A long history is not rewrapped as far back as the new capacity cuts it off: what stays is the same as if all of it
/// had been, and the count of lines that left the scrollback never claims more than really did.
#[test]
fn the_part_of_the_history_the_capacity_cuts_off_is_not_rewrapped() {
    let lines: Vec<String> = (0..3000).map(|i| format!("{i:04} {}", "word ".repeat(i % 17))).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut parser = Parser::new(nz(6), nz(60), 200);
    parser.process(refs.join("\r\n").as_bytes());
    for width in [40, 25, 13, 9, 7] {
        parser.set_size(nz(6), nz(width));
        assert_eq!(parser.screen().scrollback_len(), 200, "width {width}");
        let rows = all_rows(&mut parser);
        let expected = wrap_to_width(&refs, width);
        assert_eq!(rows, expected[expected.len() - rows.len()..], "width {width}");
        let claimed = parser.screen().history_dropped() + rows.len();
        assert!(claimed <= expected.len(), "width {width}: {claimed} rows claimed of {}", expected.len());
    }
}

/// With `pull_history` off (Windows panes: ConPTY keeps the top of its viewport and repaints from there) a taller
/// screen keeps its top row and gets blank rows at the bottom, also when the width changes in the same resize; on
/// by default the history is pulled down.
#[test]
fn a_taller_screen_is_not_filled_from_the_history_when_pulling_is_off() {
    for pull in [false, true] {
        for cols in [20, 7] {
            let mut parser = Parser::new(nz(4), nz(20), 100);
            parser.screen_mut().set_pull_history(pull);
            assert_eq!(parser.screen().pull_history(), pull);
            parser.process(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\nsix");
            assert_eq!(parser.screen().scrollback_len(), 2);
            parser.set_size(nz(6), nz(cols));
            let top = parser.screen().rows(0, cols).next().unwrap();
            if pull {
                assert_eq!((top.as_str(), parser.screen().scrollback_len()), ("one", 0), "{cols} columns");
            } else {
                assert_eq!((top.as_str(), parser.screen().scrollback_len()), ("three", 2), "{cols} columns");
                assert_eq!(parser.screen().cursor_position(), (3, 3), "the cursor stays on its text");
            }
        }
    }
}

/// A full reset keeps what the embedding program set.
#[test]
fn a_full_reset_keeps_the_resize_settings() {
    let mut parser = Parser::new(nz(4), nz(20), 100);
    parser.screen_mut().set_reflow(false);
    parser.screen_mut().set_pull_history(false);
    parser.process(b"\x1bc");
    assert!(!parser.screen().reflow() && !parser.screen().pull_history());
}
