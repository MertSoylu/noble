//! Tests for the SGR (`CSI … m`) attributes: the extended colors in both their semicolon and colon forms, the
//! underline styles, the underline color (consumed, not stored), and strikethrough, conceal and blink.

use crate::vt100::{Color, Parser};
use core::num::NonZeroU16;

fn nz(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n).unwrap()
}

/// The cell at column 0 of a 3 by 40 screen after `bytes`.
fn first_cell(bytes: &[u8]) -> crate::vt100::Cell {
    let mut parser = Parser::new(nz(3), nz(40), 0);
    parser.process(bytes);
    parser.screen().cell(0, 0).unwrap().clone()
}

#[test]
fn semicolon_underline_color_takes_its_parameters() {
    // Read as attributes, 40..42 would set a background color and the 2 would set dim.
    let cell = first_cell(b"\x1b[58;2;40;41;42mX");
    assert_eq!(cell.bgcolor(), Color::Default);
    assert!(!cell.dim());
    // Read as attributes, the 1 would set bold.
    let cell = first_cell(b"\x1b[58;5;1mX");
    assert!(!cell.bold());
    // What follows the color still applies, and 59 resets the underline color without parameters.
    let cell = first_cell(b"\x1b[58;2;40;41;42;1;4;59mX");
    assert!(cell.bold() && cell.underline());
    assert_eq!(cell.bgcolor(), Color::Default);
}

#[test]
fn colon_underline_color_and_extended_colors_have_no_side_effects() {
    // A colon form is one parameter, so nothing is left over to read as an attribute. The colorspace id
    // (empty in `58:2::r:g:b`) is part of that parameter.
    let cell = first_cell(b"\x1b[58:2::40:41:42mX");
    assert_eq!(
        (cell.bgcolor(), cell.fgcolor(), cell.bold(), cell.dim()),
        (Color::Default, Color::Default, false, false)
    );
    let cell = first_cell(b"\x1b[58:5:1mX");
    assert!(!cell.bold() && cell.bgcolor() == Color::Default);
    assert_eq!(first_cell(b"\x1b[38:2::255:0:0mX").fgcolor(), Color::Rgb(255, 0, 0));
    assert_eq!(first_cell(b"\x1b[48:2::0:255:0mX").bgcolor(), Color::Rgb(0, 255, 0));
    assert_eq!(first_cell(b"\x1b[38:2:1:2:3mX").fgcolor(), Color::Rgb(1, 2, 3));
    assert_eq!(first_cell(b"\x1b[48;5;196mX").bgcolor(), Color::Idx(196));
}

#[test]
fn underline_styles_underline_and_4_0_clears_it() {
    // `4:3` (curly underline) is an underline; the parameters after it still apply.
    let cell = first_cell(b"\x1b[4:3;1mX");
    assert!(cell.underline() && cell.bold());
    let mut parser = Parser::new(nz(3), nz(40), 0);
    parser.process(b"\x1b[4:1mA\x1b[4:0mB");
    assert!(parser.screen().cell(0, 0).unwrap().underline());
    assert!(!parser.screen().cell(0, 1).unwrap().underline());
}

#[test]
fn strikethrough_conceal_and_blink_set_and_reset() {
    let mut parser = Parser::new(nz(3), nz(40), 0);
    parser.process(b"\x1b[9mA\x1b[29mB\x1b[8mC\x1b[28mD\x1b[5mE\x1b[6mF\x1b[25mG");
    let screen = parser.screen();
    let at = |col| screen.cell(0, col).unwrap();
    assert!(at(0).strikethrough() && !at(1).strikethrough());
    assert!(at(2).conceal() && !at(3).conceal());
    assert!(at(4).blink() && at(5).blink() && !at(6).blink());
}

#[test]
fn sgr_zero_clears_the_new_attributes() {
    let cell = first_cell(b"\x1b[9;8;5mX\x1b[0m");
    assert!(cell.strikethrough() && cell.conceal() && cell.blink());
    let mut parser = Parser::new(nz(3), nz(40), 0);
    parser.process(b"\x1b[9;8;5mA\x1b[0mB");
    let screen = parser.screen();
    assert!(!screen.cell(0, 1).unwrap().strikethrough());
    assert!(!screen.cell(0, 1).unwrap().conceal());
    assert!(!screen.cell(0, 1).unwrap().blink());
}

#[test]
fn formatted_output_keeps_the_new_attributes() {
    let mut parser = Parser::new(nz(3), nz(40), 0);
    parser.process(b"\x1b[9mA\x1b[29m\x1b[8mB\x1b[28m\x1b[5mC\x1b[25m");
    let formatted = parser.screen().contents_formatted();
    let mut copy = Parser::new(nz(3), nz(40), 0);
    copy.process(formatted.as_bytes());
    let (original, copied) = (parser.screen(), copy.screen());
    for col in 0..3 {
        let (a, b) = (original.cell(0, col).unwrap(), copied.cell(0, col).unwrap());
        assert_eq!(
            (a.strikethrough(), a.conceal(), a.blink()),
            (b.strikethrough(), b.conceal(), b.blink()),
            "column {col}"
        );
    }
    assert!(copied.cell(0, 0).unwrap().strikethrough());
    assert!(copied.cell(0, 1).unwrap().conceal());
    assert!(copied.cell(0, 2).unwrap().blink());
}

#[test]
fn a_reflow_keeps_the_new_attributes() {
    let mut parser = Parser::new(nz(3), nz(10), 0);
    parser.process(b"\x1b[9;5mabcdefgh");
    // Narrowing rewraps the line; the second row starts with "e".
    parser.set_size(nz(3), nz(4));
    let cell = parser.screen().cell(1, 0).unwrap();
    assert_eq!(cell.contents(), "e");
    assert!(cell.strikethrough() && cell.blink());
}
