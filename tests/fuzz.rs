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
                s.push_str(self.pick(pieces));
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
        noble::hooks::run_cli("Notification", &text, &dir, Some("7"), Some(&i.to_string()));
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
