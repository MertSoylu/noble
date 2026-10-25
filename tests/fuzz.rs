//! Fuzz tests for the parsers that read text or JSON from outside NOBLE (git output, shell titles
//! and OSC payloads, terminal lines, Windows Terminal settings, provider and GitHub API replies,
//! Claude hook payloads): random and hostile input must never panic.
//! Deterministic: `NOBLE_FUZZ_SEED` picks another seed, `NOBLE_FUZZ_SCALE` runs longer.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;

struct Rng(u64);

impl Rng {
    fn seeded(default: u64) -> Rng {
        let seed = std::env::var("NOBLE_FUZZ_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(default);
        eprintln!("fuzz seed {seed}");
        Rng(seed.max(1))
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    /// Text glued from `pieces`, with now and then a random character from any Unicode plane.
    fn text(&mut self, pieces: &[&str], max: usize) -> String {
        let mut s = String::new();
        for _ in 0..self.below(max + 1) {
            if self.below(5) == 0 {
                s.extend(char::from_u32(self.next() as u32 % 0x11_0000));
            } else {
                s.push_str(self.pick::<&str>(pieces));
            }
        }
        s
    }
}

fn steps(base: usize) -> usize {
    base * std::env::var("NOBLE_FUZZ_SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or(1)
}

/// Pieces of the formats under test plus multi-byte and control characters.
const PIECES: [&str; 44] = [
    "## ",
    "...",
    " [",
    "]",
    "ahead ",
    "behind ",
    ", ",
    "?? ",
    " M ",
    "R  ",
    " -> ",
    "\"",
    "\n",
    "\r\n",
    "\t",
    " ",
    "\u{1f}",
    "file://",
    "/",
    "\\",
    "C:",
    "%",
    "%e9",
    "%zz",
    ":",
    "(",
    ")",
    ",",
    "12",
    "4294967296",
    "-",
    ".exe",
    " - ",
    " | ",
    "é",
    "日本",
    "🙂",
    "e\u{301}",
    "İ",
    "\u{202e}",
    "\u{0}",
    "#",
    "https://",
    "~/",
];

#[test]
fn text_parsers_never_panic() {
    let mut rng = Rng::seeded(0x5151_7a7a_0303_0909);
    let cwd = std::env::temp_dir();
    for _ in 0..steps(20_000) {
        let s = rng.text(&PIECES, 12);
        let _ = noble::projects::parse_status(&s);
        let _ = noble::projects::parse_log(&s);
        let _ = noble::term::pane::parse_cwd_url(&s);
        let _ = noble::term::pane::msys_to_windows(&s);
        let _ = noble::term::pane::process_label(&s, &s);
        let q = rng.text(&PIECES, 2);
        let _ = noble::term::pane::find_matches(std::slice::from_ref(&s), &q);
        if let Some((token, ..)) = noble::term::link::token_at(&s, rng.below(40) as u16) {
            let _ = noble::term::link::classify(&token, &cwd);
        }
        let _ = noble::term::link::classify(&s, &cwd);
        let n = rng.below(20);
        let _ = noble::util::truncate(&s, n);
        let _ = noble::util::truncate_left(&s, n);
        let _ = noble::util::pad_left(&s, n);
        let _ = noble::util::pad_right(&s, n);
        let _ = noble::util::fuzzy_score(&q, &s);
        let _ = noble::util::base64_decode(s.as_bytes());
        let _ = noble::theme::parse_hex(&s);
        let _ = noble::update::parse_version(&s);
        let _ = noble::update::is_newer(&s, &q);
        let _ = noble::ai::json::normalize_label(&s);
        let _ = noble::keys::Chord::parse(&s);
    }
}

/// Random JSON-ish text (valid or not) through the JSONC reader and every JSON consumer.
#[test]
fn json_consumers_never_panic() {
    const KEYS: [&str; 24] = [
        "\"five_hour\"",
        "\"seven_day\"",
        "\"utilization\"",
        "\"used_percent\"",
        "\"resets_at\"",
        "\"resetsAtMs\"",
        "\"rate_limits\"",
        "\"primary\"",
        "\"window_minutes\"",
        "\"tag_name\"",
        "\"assets\"",
        "\"name\"",
        "\"browser_download_url\"",
        "\"message\"",
        "\"profiles\"",
        "\"schemes\"",
        "\"colorScheme\"",
        "\"list\"",
        "\"defaults\"",
        "\"background\"",
        "\"foreground\"",
        "\"remaining\"",
        "\"limit\"",
        "\"usage\"",
    ];
    const VALUES: [&str; 20] = [
        "0",
        "-1",
        "1e308",
        "-1e308",
        "255",
        "100.5",
        "9223372036854775807",
        "-9223372036854775808",
        "null",
        "true",
        "\"\"",
        "\"é日🙂\"",
        "\"2026-13-45\"",
        "\"2026-01-01T00:00:00Z\"",
        "\"1e300\"",
        "\"-1e300\"",
        "\"NaN\"",
        "\"#zzz\"",
        "\"#c8c8c8\"",
        "\"v1.2.3\"",
    ];
    let mut rng = Rng::seeded(0x0bad_cafe_f00d_beef);
    let dir = std::env::temp_dir().join(format!("noble-fuzz-json-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for i in 0..steps(3000) {
        let text = json_like(&mut rng, &KEYS, &VALUES, 0);
        // Comments and trailing commas as Windows Terminal writes them, sometimes cut short.
        let jsonc = format!("// c\n{text} /* x */,");
        let cut = rng.below(jsonc.len() + 1);
        let cut = (0..=cut).rev().find(|i| jsonc.is_char_boundary(*i)).unwrap_or(0);
        let _ = noble::wt::parse(&jsonc[..cut]);
        let _ = noble::wt::strip_jsonc(&jsonc);
        let _ = noble::wt::parse(&text);
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            let _ = noble::ai::providers::parse_claude(&v);
            let _ = noble::ai::providers::parse_codex(&v);
            let _ = noble::ai::providers::parse_agy(&v);
            let _ = noble::ai::providers::parse_ocgo(&v);
            let _ = noble::ai::providers::parse_kilo_pass(&v);
            let _ = noble::ai::providers::parse_cmdc(&v);
            let _ = noble::ai::json::extract_windows(&v, &["five_hour", "seven_day", "primary", "usage"], 4);
            let _ = noble::ai::json::window_from(&v, "5H");
            let _ = noble::ai::json::reset_at(&v, &["resets_at", "resetsAtMs"]);
            let _ = noble::update::parse_release(&v);
        }
        // Claude hook payload from stdin, then read back as the app does.
        noble::hooks::run_cli(
            noble::hooks::Agent::Claude,
            "notification",
            &text,
            &dir,
            Some("7"),
            Some(&i.to_string()),
        );
    }
    let _ = noble::hooks::read_records(&dir, 7);
    let _ = std::fs::remove_dir_all(&dir);
}

fn json_like(rng: &mut Rng, keys: &[&str], values: &[&str], depth: usize) -> String {
    match rng.below(if depth > 3 { 2 } else { 5 }) {
        0 => rng.pick(values).to_string(),
        1 => rng.text(&["{", "}", "[", "]", ",", ":", "\"", "é", "0"], 3),
        2 => {
            let items: Vec<String> = (0..rng.below(4)).map(|_| json_like(rng, keys, values, depth + 1)).collect();
            format!("[{}]", items.join(","))
        }
        _ => {
            let items: Vec<String> = (0..rng.below(5))
                .map(|_| format!("{}:{}", rng.pick(keys), json_like(rng, keys, values, depth + 1)))
                .collect();
            format!("{{{}}}", items.join(","))
        }
    }
}

/// Every key the terminal can deliver (any character, any modifiers) encodes without panicking.
#[test]
fn key_encoding_never_panics() {
    let mut rng = Rng::seeded(0x7777_1111_3333_5555);
    let mods = [
        KeyModifiers::NONE,
        KeyModifiers::SHIFT,
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::CONTROL | KeyModifiers::ALT,
        KeyModifiers::all(),
    ];
    for _ in 0..steps(20_000) {
        let code = match rng.below(4) {
            0 => KeyCode::F(rng.below(256) as u8),
            1 => *rng.pick(&[KeyCode::Up, KeyCode::Home, KeyCode::BackTab, KeyCode::Null, KeyCode::Esc]),
            _ => KeyCode::Char(char::from_u32(rng.next() as u32 % 0x11_0000).unwrap_or('\u{fffd}')),
        };
        let ev = KeyEvent::new(code, *rng.pick(&mods));
        let _ = noble::term::input::encode_key(&ev, rng.below(2) == 0);
        let _ = noble::keys::Chord::from_event(&ev).to_string();
    }
}

/// Hostile output through the emulator step the reader thread runs (`pane::feed`): synchronized output, focus and
/// scheme modes, DECRQM, XTVERSION, color queries and sets, cut anywhere, with resets in between. Must not panic,
/// and what it queues stays bounded by what it read.
#[test]
fn terminal_queries_never_panic() {
    use noble::term::colors::{ColorReport, OscScan, SharedColors, Terminator};
    use noble::term::pane::{Callbacks, feed};
    const BITS: [&str; 40] = [
        "\x1b[?2026h",
        "\x1b[?2026l",
        "\x1b[?2026$p",
        "\x1b[?1004h",
        "\x1b[?1004l",
        "\x1b[?2031;1049;99999h",
        "\x1b[?2031l",
        "\x1b[?996n",
        "\x1b[?1004$p",
        "\x1b[$p",
        "\x1b[65535$p",
        "\x1b[?65536$p",
        "\x1b[>q",
        "\x1b[>0q",
        "\x1b[>99999q",
        "\x1b]11;?\x07",
        "\x1b]11;?\x1b\\",
        "\x1b]10;?;?;?;?\x07",
        "\x1b]12;?\x07",
        "\x1b]4;1;?;256;?;-1;?;x;?\x07",
        "\x1b]4;\x07",
        "\x1b]4;1;rgb:ff/00/00\x07",
        "\x1b]104\x07",
        "\x1b]11;#zzzz\x07",
        "\x1b]",
        "\x1b",
        "\x1b\\",
        "\x1bc",
        "\x07",
        "\x18",
        "\u{9c}",
        ";",
        "?",
        "11",
        "4",
        "🙂",
        "\u{0}",
        "\r\n",
        "abc",
        "\x1b[",
    ];
    let mut rng = Rng::seeded(0x2026_1004_2031_0996);
    let colors = SharedColors::default();
    let mut parser = noble::vt100::Parser::new_with_callbacks(
        noble::term::vt_size(10),
        noble::term::vt_size(40),
        0,
        Callbacks::with_colors(colors.clone()),
    );
    let mut scan = OscScan::default();
    let mut read = 0usize;
    let mut queued = 0usize;
    for i in 0..steps(20_000) {
        let text = rng.text(&BITS, 8);
        let bytes = text.as_bytes();
        // Cut at a random byte, so sequences split between reads like on a PTY.
        let cut = rng.below(bytes.len() + 1);
        for part in [&bytes[..cut], &bytes[cut..]] {
            read += part.len();
            feed(&mut parser, part);
            let _ = parser.callbacks().sync_left();
            let _ = parser.callbacks().mode_on(1004);
        }
        for b in bytes {
            let _ = scan.push(*b);
        }
        parser.callbacks_mut().seal_color_replies(Terminator::St);
        queued += parser.callbacks().responses.len();
        parser.callbacks_mut().responses.clear();
        if i % 97 == 0 {
            // Another palette, a light one.
            let mut r = ColorReport::dark();
            r.bg = [rng.below(256) as u8, rng.below(256) as u8, rng.below(256) as u8];
            *colors.write().unwrap() = r;
        }
    }
    assert!(queued <= read * 64 + 1024, "{queued} bytes of answers for {read} bytes read");
    for number in [&b"4"[..], b"10", b"11", b"12", b"", b"x", b"99999999999999999999"] {
        let params: [&[u8]; 4] = [b"?", b"", b"300", b"?"];
        for n in 0..=params.len() {
            let _ = ColorReport::dark().answers(number, &params[..n]);
        }
    }
}

/// The text of the whole main screen (scrollback and rows) as logical lines: the rows of a wrapped run are joined
/// cell by cell (a blank cell is a space, the spacer left by a wide character at the edge and the continuation
/// cell are nothing) and the trailing blanks of every line and the blank lines at the end are cut.
fn logical_lines(parser: &mut noble::vt100::Parser<noble::term::pane::Callbacks>) -> Vec<String> {
    let screen = parser.screen_mut();
    let keep = screen.scrollback();
    let (rows, cols) = screen.size();
    let (rows, cols) = (usize::from(rows.get()), cols.get());
    let history = screen.scrollback_len();
    let mut lines = Vec::new();
    let mut open = String::new();
    for i in 0..history + rows {
        // Scrolled so that absolute row `i` is the first one shown (or, below the history, the live screen).
        let offset = history.saturating_sub(i);
        screen.set_scrollback(offset);
        let row = u16::try_from(i - (history - offset)).unwrap();
        for col in 0..cols {
            let cell = screen.cell(row, col).expect("a cell inside the screen");
            if cell.is_wide_continuation() || cell.is_leading_spacer() {
                continue;
            }
            open.push_str(if cell.has_contents() { cell.contents() } else { " " });
        }
        if !screen.row_wrapped(row) {
            lines.push(open.trim_end().to_string());
            open.clear();
        }
    }
    if !open.is_empty() {
        lines.push(open.trim_end().to_string());
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    screen.set_scrollback(keep);
    lines
}

/// Every cell of the shown screen exists, a wide character is followed by its continuation cell and a continuation
/// cell follows a wide character (at every scroll position of the history too).
fn assert_grid_sound(parser: &mut noble::vt100::Parser<noble::term::pane::Callbacks>, what: &str) {
    let screen = parser.screen_mut();
    let keep = screen.scrollback();
    let (rows, cols) = screen.size();
    let (rows, cols) = (rows.get(), cols.get());
    assert!(screen.scrollback_len() <= screen.scrollback_capacity(), "{what}: history over its capacity");
    let top = screen.scrollback_len();
    let mut offset = 0;
    loop {
        screen.set_scrollback(offset);
        for row in 0..rows {
            for col in 0..cols {
                let cell = screen.cell(row, col).unwrap_or_else(|| panic!("{what}: no cell at {row},{col}"));
                if cell.is_wide_continuation() {
                    let prev = col.checked_sub(1).and_then(|c| screen.cell(row, c));
                    assert!(prev.is_some_and(|p| p.is_wide()), "{what}: orphan continuation at {row},{col}");
                }
                if cell.is_wide() {
                    let next = screen.cell(row, col + 1);
                    assert!(next.is_some_and(|n| n.is_wide_continuation()), "{what}: wide cell cut at {row},{col}");
                }
            }
            assert!(screen.cell(row, cols).is_none(), "{what}: a row is wider than the screen");
        }
        if offset >= top {
            break;
        }
        offset = (offset + usize::from(rows)).min(top);
    }
    screen.set_scrollback(keep);
}

/// Random bytes mixed with random resizes (narrow, wide, tall, short; on the main and the alternate screen; wide
/// characters, SGR, cursor moves, scroll regions): never a panic, every row has the screen's width, no wide
/// character is cut, the history stays within its capacity, and a shrink followed by a grow back loses no text of a
/// line when nothing was written in between (the lossless round trip behind zoom and restore).
#[test]
fn reflow_survives_random_output_and_resizes() {
    const BITS: [&str; 46] = [
        "a",
        "bc",
        "hello world ",
        "0123456789",
        "x",
        " ",
        "\r\n",
        "\r\n",
        "\n",
        "\r",
        "\t",
        "\x08",
        "日本",
        "語",
        "🙂",
        "e\u{301}",
        "\x1b[1;31m",
        "\x1b[48;5;20m",
        "\x1b[0m",
        "\x1b[H",
        "\x1b[3;5H",
        "\x1b[9999;9999H",
        "\x1b[2J",
        "\x1b[3J",
        "\x1b[K",
        "\x1b[2K",
        "\x1b[2L",
        "\x1b[2M",
        "\x1b[3P",
        "\x1b[3@",
        "\x1b[4X",
        "\x1b[2A",
        "\x1b[2B",
        "\x1b[5D",
        "\x1b[2;5r",
        "\x1b[r",
        "\x1b[S",
        "\x1b[T",
        "\x1b7",
        "\x1b8",
        "\x1bM",
        "\x1b[?1049h",
        "\x1b[?1049l",
        "\x1b[?25l",
        "\x1b[?7l",
        "\x1b[?7h",
    ];
    let mut rng = Rng::seeded(0x7e_f10a_2026_1025);
    let total = steps(6_000);
    let mut parser = None;
    for i in 0..total {
        // A fresh emulator now and then keeps the history small; every third one has a tiny history that fills up.
        if i % 150 == 0 {
            let capacity = if (i / 150) % 3 == 2 { 12 } else { 100_000 };
            let rows = 2 + rng.below(25) as u16;
            let cols = 2 + rng.below(60) as u16;
            parser = Some((
                noble::vt100::Parser::new_with_callbacks(
                    noble::term::vt_size(rows),
                    noble::term::vt_size(cols),
                    capacity,
                    noble::term::pane::Callbacks::default(),
                ),
                capacity == 100_000,
            ));
        }
        let (parser, lossless) = parser.as_mut().unwrap();
        let text = rng.text(&BITS, 12);
        let bytes = text.as_bytes();
        let cut = rng.below(bytes.len() + 1);
        noble::term::pane::feed(parser, &bytes[..cut]);
        if rng.below(3) == 0 {
            let rows = 2 + rng.below(25) as u16;
            let cols = 2 + rng.below(60) as u16;
            parser.screen_mut().set_size(noble::term::vt_size(rows), noble::term::vt_size(cols));
            assert_grid_sound(parser, "after a resize");
        }
        noble::term::pane::feed(parser, &bytes[cut..]);
        if i % 7 == 0 {
            assert_grid_sound(parser, "after output");
        }
        if *lossless && i % 5 == 0 {
            // Round trip on the main screen: narrower or shorter, then back, with no output in between.
            noble::term::pane::feed(parser, b"\x1b[?1049l");
            let (rows0, cols0) = parser.screen().size();
            let before = logical_lines(parser);
            let rows1 = 2 + rng.below(25) as u16;
            let cols1 = 2 + rng.below(60) as u16;
            parser.screen_mut().set_size(noble::term::vt_size(rows1), noble::term::vt_size(cols1));
            assert_grid_sound(parser, "in the narrow size");
            let during = logical_lines(parser);
            parser.screen_mut().set_size(rows0, cols0);
            assert_grid_sound(parser, "back at the old size");
            let after = logical_lines(parser);
            assert_eq!(before, after, "step {i}: {cols0}x{rows0} -> {cols1}x{rows1} -> back changed the text");
            // The text is the same at the other size too, as far as the width lets a wide character stay whole.
            if cols1 >= 2 {
                assert_eq!(
                    before.len(),
                    during.len(),
                    "step {i}: {cols0}x{rows0} -> {cols1}x{rows1} changed the number of lines"
                );
            }
        }
    }
}
