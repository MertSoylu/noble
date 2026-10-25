//! Color queries: what a pane's program is told about the colors it is drawn with. OSC 10 / 11 / 12 (text,
//! background, cursor) and OSC 4 (palette entry) with `?` are answered in xterm's format, `COLORFGBG` is set in the
//! environment, and a program that set mode 2031 is told when the scheme turns light or dark (`CSI ? 997 ; Ps n`).
//!
//! The snapshot (`ColorReport`) is built from `TermPalette::resolve`, the same colors `ui/terminal.rs` draws with, and
//! shared with the pane's reader thread, which answers the queries itself (like DSR/DA). Only the main thread
//! writes it (`App::sync_color_reports`).

use std::sync::{Arc, RwLock};

use ratatui::style::Color;

use crate::theme::{CAMPBELL, TermPalette};

/// Private mode 2031: the program wants `CSI ? 997 ; Ps n` when the color scheme turns dark or light.
pub const SCHEME_NOTICES: u16 = 2031;

/// The report every pane's reader thread reads and the main thread replaces.
pub type SharedColors = Arc<RwLock<ColorReport>>;

type Rgb = [u8; 3];

/// The pane colors as a program can ask for them.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorReport {
    pub bg: Rgb,
    pub fg: Rgb,
    /// The cursor is the outer terminal's own; the text color is the closest thing NOBLE can name.
    pub cursor: Rgb,
    /// ANSI 0–15. Without a scheme (theme mode) the outer terminal decides, so the standard Campbell values stand in.
    pub ansi: [Rgb; 16],
}

fn split(hex: u32) -> Rgb {
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8]
}

/// The RGB of a drawing color; `None` for the ones that name no fixed color (`Reset`, indexed).
fn rgb_of(c: Color) -> Option<Rgb> {
    match c {
        Color::Rgb(r, g, b) => Some([r, g, b]),
        _ => None,
    }
}

/// The xterm default for palette entries 16–255 (the 6×6×6 cube and the gray ramp); below 16 `ansi` decides.
fn xterm_extended(i: u8) -> Rgb {
    match i {
        16..=231 => {
            let level = |n: u8| if n == 0 { 0 } else { 55 + 40 * n };
            let n = i - 16;
            [level(n / 36), level(n / 6 % 6), level(n % 6)]
        }
        _ => {
            let v = 8 + 10 * i.saturating_sub(232);
            [v, v, v]
        }
    }
}

impl ColorReport {
    /// The report for a pane drawn with `pal`. A color that is not a fixed RGB (a transparent theme's background)
    /// is read from `fallback`, the same palette resolved with the opaque theme.
    pub fn new(pal: &TermPalette, fallback: &TermPalette) -> ColorReport {
        let bg = rgb_of(pal.bg).or_else(|| rgb_of(fallback.bg)).unwrap_or([0, 0, 0]);
        let fg = rgb_of(pal.fg).or_else(|| rgb_of(fallback.fg)).unwrap_or([0xcc, 0xcc, 0xcc]);
        let ansi = match pal.ansi {
            Some(a) => std::array::from_fn(|i| rgb_of(a[i]).unwrap_or_else(|| split(CAMPBELL[i]))),
            None => CAMPBELL.map(split),
        };
        ColorReport { bg, fg, cursor: fg, ansi }
    }

    /// A plain dark report, for panes that exist before any palette was resolved.
    pub fn dark() -> ColorReport {
        ColorReport { bg: [0, 0, 0], fg: [0xcc; 3], cursor: [0xcc; 3], ansi: CAMPBELL.map(split) }
    }

    /// Whether the background is light: its luma is at least half.
    pub fn is_light(&self) -> bool {
        let [r, g, b] = self.bg;
        // Rec. 601 weights scaled by 1000; 127_500 is half of the full range (255 * 1000).
        299 * r as u32 + 587 * g as u32 + 114 * b as u32 >= 127_500
    }

    /// `COLORFGBG` as rxvt sets it: ANSI indexes of the text and the background, `15;0` on a dark background and
    /// `0;15` on a light one (programs read the second number to tell the two apart).
    pub fn colorfgbg(&self) -> &'static str {
        if self.is_light() { "0;15" } else { "15;0" }
    }

    /// The answer to `CSI ? 996 n`: `CSI ? 997 ; 1 n` dark, `CSI ? 997 ; 2 n` light. Also the notice a program
    /// that set mode 2031 gets when the scheme changes.
    pub fn scheme_notice(&self) -> &'static [u8] {
        if self.is_light() { b"\x1b[?997;2n" } else { b"\x1b[?997;1n" }
    }

    /// The color of palette entry `i`.
    pub fn palette(&self, i: u8) -> Rgb {
        if i < 16 { self.ansi[i as usize] } else { xterm_extended(i) }
    }

    /// The body (without terminator) of the answer to a query for OSC `number` (10, 11 or 12, or 4 with `index`).
    fn body(&self, number: u8, index: Option<u8>) -> String {
        let (color, prefix) = match (number, index) {
            (4, Some(i)) => (self.palette(i), format!("4;{i}")),
            (10, _) => (self.fg, "10".to_string()),
            (11, _) => (self.bg, "11".to_string()),
            _ => (self.cursor, "12".to_string()),
        };
        format!("\x1b]{prefix};{}", xterm_rgb(color))
    }

    /// The answers (bodies without terminator) to the `?` queries of one OSC string: `number` is its first part,
    /// `params` the parts after it. A color being set is ignored, and so is anything not understood: a program that
    /// sets colors must not break.
    ///
    /// OSC 10 / 11 take one part per color and the next part is the next color (`10;?;?` asks for text and
    /// background); OSC 4 takes `index;spec` pairs.
    pub fn answers(&self, number: &[u8], params: &[&[u8]]) -> Vec<String> {
        let mut out = Vec::new();
        match number {
            b"4" => {
                for pair in params.chunks(2) {
                    if let [index, b"?"] = pair
                        && let Some(i) = std::str::from_utf8(index).ok().and_then(|s| s.trim().parse::<u8>().ok())
                    {
                        out.push(self.body(4, Some(i)));
                    }
                }
            }
            b"10" | b"11" | b"12" => {
                let first = match number {
                    b"10" => 10,
                    b"11" => 11,
                    _ => 12,
                };
                for (n, spec) in (first..=12).zip(params) {
                    if *spec == b"?" {
                        out.push(self.body(n, None));
                    }
                }
            }
            _ => {}
        }
        out
    }
}

impl Default for ColorReport {
    fn default() -> ColorReport {
        ColorReport::dark()
    }
}

/// xterm's `rgb:RRRR/GGGG/BBBB`: each 8-bit value repeated into 16 bits.
pub fn xterm_rgb([r, g, b]: Rgb) -> String {
    format!("rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}")
}

/// What ends an OSC string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Terminator {
    Bel,
    /// `ESC \`
    St,
}

impl Terminator {
    pub fn bytes(self) -> &'static [u8] {
        match self {
            Terminator::Bel => b"\x07",
            Terminator::St => b"\x1b\\",
        }
    }
}

/// Follows the OSC strings in the output to find where a color OSC (4, 10, 11, 12) ends and how, because the
/// emulator does not say how a string was terminated and the answer must end the same way. State carries over
/// from one chunk to the next.
#[derive(Default)]
pub struct OscScan {
    /// Inside an OSC string: its number so far, and whether the number is complete (a `;` came).
    open: Option<(Vec<u8>, bool)>,
    esc: bool,
}

impl OscScan {
    /// Whether the output stopped between the `ESC` and the `\` of a string's terminator. The emulator has already
    /// ended the string at the `ESC`, so its answer waits for the `\` to learn that it is an ST.
    pub fn awaiting_st(&self) -> bool {
        self.open.is_some() && self.esc
    }

    /// Takes the next output byte; `Some` when it ends a color OSC string.
    pub fn push(&mut self, b: u8) -> Option<Terminator> {
        let after_esc = std::mem::replace(&mut self.esc, b == 0x1b);
        let Some((number, done)) = &mut self.open else {
            if after_esc && b == b']' {
                self.open = Some((Vec::new(), false));
            }
            return None;
        };
        let end = match b {
            0x07 => Some(Terminator::Bel),
            b'\\' if after_esc => Some(Terminator::St),
            _ => None,
        };
        if let Some(end) = end {
            let color = matches!(number.as_slice(), b"4" | b"10" | b"11" | b"12");
            self.open = None;
            return color.then_some(end);
        }
        if after_esc && b == b']' {
            self.open = Some((Vec::new(), false));
        } else if after_esc || b == 0x18 || b == 0x1a {
            // Another escape sequence (or CAN / SUB) cuts the string off.
            self.open = None;
        } else if !*done {
            if b == b';' || number.len() >= 4 {
                *done = true;
            } else {
                number.push(b);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{FOLLOW_THEME, Theme, builtin_schemes};

    fn report(scheme: &str, theme: &str) -> ColorReport {
        let th = Theme::by_name(theme, false);
        let pal = TermPalette::resolve(&builtin_schemes(), scheme, "", "", &th);
        ColorReport::new(&pal, &pal)
    }

    #[test]
    fn xterm_format_repeats_each_byte() {
        assert_eq!(xterm_rgb([0x1e, 0x2f, 0xff]), "rgb:1e1e/2f2f/ffff");
        assert_eq!(xterm_rgb([0, 0, 0]), "rgb:0000/0000/0000");
    }

    #[test]
    fn answers_follow_the_query() {
        let r = report("light-gray", FOLLOW_THEME);
        assert_eq!(r.answers(b"11", &[b"?"]), vec!["\x1b]11;rgb:c8c8/c8c8/c8c8"]);
        assert_eq!(r.answers(b"10", &[b"?"]), vec!["\x1b]10;rgb:1e1e/1e1e/1e1e"]);
        assert_eq!(r.answers(b"12", &[b"?"]), vec!["\x1b]12;rgb:1e1e/1e1e/1e1e"]);
        // `10;?;?` asks for the text and the background in turn.
        assert_eq!(r.answers(b"10", &[b"?", b"?"]), vec!["\x1b]10;rgb:1e1e/1e1e/1e1e", "\x1b]11;rgb:c8c8/c8c8/c8c8"]);
        assert_eq!(
            r.answers(b"4", &[b"1", b"?", b"2", b"?"]),
            vec!["\x1b]4;1;rgb:a3a3/2626/1b1b", "\x1b]4;2;rgb:2e2e/6b6b/2828"]
        );
        // A color being set answers nothing.
        assert!(r.answers(b"11", &[b"#102030"]).is_empty());
        assert!(r.answers(b"4", &[b"1", b"rgb:ff/00/00"]).is_empty());
        assert!(r.answers(b"4", &[b"x", b"?"]).is_empty());
    }

    #[test]
    fn palette_beyond_sixteen_is_the_xterm_default() {
        let r = ColorReport::dark();
        assert_eq!(r.palette(16), [0, 0, 0]);
        assert_eq!(r.palette(196), [255, 0, 0]);
        assert_eq!(r.palette(231), [255, 255, 255]);
        assert_eq!(r.palette(232), [8, 8, 8]);
        assert_eq!(r.palette(255), [238, 238, 238]);
    }

    #[test]
    fn light_and_dark_follow_the_background() {
        // The light UI themes in theme mode.
        for name in ["latte", "paper", "solarlight"] {
            let r = report(FOLLOW_THEME, name);
            assert!(r.is_light(), "{name}");
            assert_eq!(r.colorfgbg(), "0;15", "{name}");
            assert_eq!(r.scheme_notice(), b"\x1b[?997;2n", "{name}");
        }
        // The light schemes (Solarized Light, Paper's and Light Gray's own, Windows Terminal's) over a dark theme.
        for (scheme, light) in [
            ("solarized-light", true),
            ("paper", true),
            ("light-gray", true),
            ("one-half-light", true),
            ("tango-light", true),
            ("solarized-dark", false),
            ("campbell", false),
            ("graphite", false),
        ] {
            let r = report(scheme, "amber");
            assert_eq!(r.is_light(), light, "{scheme}");
            assert_eq!(r.colorfgbg(), if light { "0;15" } else { "15;0" }, "{scheme}");
        }
        for name in ["amber", "nord", "dracula", "solarized"] {
            let r = report(FOLLOW_THEME, name);
            assert_eq!(r.colorfgbg(), "15;0", "{name}");
            assert_eq!(r.scheme_notice(), b"\x1b[?997;1n", "{name}");
        }
    }

    #[test]
    fn transparent_background_is_read_from_the_opaque_theme() {
        let schemes = builtin_schemes();
        let clear = Theme::by_name("latte", true);
        let solid = Theme::by_name("latte", false);
        let pal = TermPalette::resolve(&schemes, FOLLOW_THEME, "", "", &clear);
        let fallback = TermPalette::resolve(&schemes, FOLLOW_THEME, "", "", &solid);
        let r = ColorReport::new(&pal, &fallback);
        assert_eq!(Some(r.bg), rgb_of(solid.bg));
        assert!(r.is_light());
    }

    fn ends(bytes: &[u8]) -> Vec<(usize, Terminator)> {
        let mut scan = OscScan::default();
        bytes.iter().enumerate().filter_map(|(i, b)| scan.push(*b).map(|t| (i, t))).collect()
    }

    #[test]
    fn scanner_finds_color_osc_terminators() {
        assert_eq!(ends(b"\x1b]11;?\x07"), vec![(6, Terminator::Bel)]);
        assert_eq!(ends(b"\x1b]11;?\x1b\\"), vec![(7, Terminator::St)]);
        assert_eq!(ends(b"\x1b]10;?\x07\x1b]4;1;?\x1b\\"), vec![(6, Terminator::Bel), (15, Terminator::St)]);
        // Other strings end without a report.
        assert!(ends(b"\x1b]0;title\x07\x1b]7;file:///x\x1b\\\x1b]110\x07").is_empty());
        // An aborted string, then a real one.
        assert_eq!(ends(b"\x1b]11;?\x1b[0m\x1b]11;?\x07"), vec![(16, Terminator::Bel)]);
        // Split between chunks: the state carries over.
        let mut scan = OscScan::default();
        assert!(b"\x1b]1".iter().all(|b| scan.push(*b).is_none()));
        assert!(b"1;?\x1b".iter().all(|b| scan.push(*b).is_none()));
        assert_eq!(scan.push(b'\\'), Some(Terminator::St));
    }
}
