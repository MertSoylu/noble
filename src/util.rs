//! Small, pure helpers: formatting, fuzzy matching, PATH lookup.

use std::path::{Path, PathBuf};
use std::time::Duration;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Is this a dev install: `install.cmd` installs the binary as `noble-dev` so it
/// runs side by side with the stable `noble`.
pub fn is_dev_build() -> bool {
    static DEV: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DEV.get_or_init(|| {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().eq_ignore_ascii_case("noble-dev")))
            .unwrap_or(false)
    })
}

/// Turns a byte count into a short human readable form: 1536 → "1.5K".
pub fn fmt_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes}B")
    } else if value >= 100.0 {
        format!("{value:.0}{}", UNITS[unit])
    } else {
        format!("{value:.1}{}", UNITS[unit])
    }
}

/// Bytes per second: "1.2M/s".
pub fn fmt_rate(bytes_per_sec: f64) -> String {
    format!("{}/s", fmt_bytes(bytes_per_sec.max(0.0) as u64))
}

/// Writes a duration with its two largest units: "2h 14m", "4d 3h", "45s".
pub fn fmt_duration(d: Duration) -> String {
    let s = d.as_secs();
    let (days, hours, mins) = (s / 86_400, (s % 86_400) / 3600, (s % 3600) / 60);
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins:02}m")
    } else if mins > 0 {
        format!("{mins}m")
    } else {
        format!("{s}s")
    }
}

/// Writes a past time in one compact unit: "now", "5m", "2h", "3d", "6w".
pub fn fmt_ago(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        "now".into()
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        format!("{}h", s / 3600)
    } else if s < 86_400 * 14 {
        format!("{}d", s / 86_400)
    } else if s < 86_400 * 365 {
        format!("{}w", s / (86_400 * 7))
    } else {
        format!("{}y", s / (86_400 * 365))
    }
}

/// Display width (column) calculation.
pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Fits text into `max` columns; appends "…" when it overflows.
pub fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if w + cw + 1 > max {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

/// Cuts the start of text (for paths): "…\Desktop\noble".
pub fn truncate_left(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if width(s) <= max {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out: Vec<char> = Vec::new();
    let mut w = 0;
    for ch in chars.iter().rev() {
        let cw = ch.width().unwrap_or(0);
        if w + cw + 1 > max {
            break;
        }
        out.push(*ch);
        w += cw;
    }
    out.push('…');
    out.iter().rev().collect()
}

/// Fills text right-aligned into `w` columns.
pub fn pad_left(s: &str, w: usize) -> String {
    let cur = width(s);
    if cur >= w { s.to_string() } else { format!("{}{s}", " ".repeat(w - cur)) }
}

/// Fills text left-aligned into `w` columns (truncates when it overflows).
pub fn pad_right(s: &str, w: usize) -> String {
    let t = truncate(s, w);
    let cur = width(&t);
    format!("{t}{}", " ".repeat(w.saturating_sub(cur)))
}

/// Drops a UTF-8 byte order mark: JSON written by Windows tools (PowerShell 5
/// `Set-Content -Encoding utf8`, Notepad) may start with one and serde rejects it.
pub fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

/// Shortens the home directory to "~".
pub fn tilde(path: &Path) -> String {
    // Compared by path components: "C:\Users\Mert2" is not under "C:\Users\Mert".
    if let Some(home) = dirs::home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        if rest.as_os_str().is_empty() {
            return "~".into();
        }
        return format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display());
    }
    path.display().to_string()
}

/// Fuzzy substring match. `None` when there is no match; otherwise a score
/// (higher is better). Consecutive letters, word starts and an exact prefix are rewarded.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    let q: Vec<char> = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let orig: Vec<char> = text.chars().collect();
    let mut score = 0i32;
    let mut qi = 0;
    let mut prev_match: Option<usize> = None;
    for (i, ch) in t.iter().enumerate() {
        if qi < q.len() && *ch == q[qi] {
            let mut s = 1;
            if let Some(p) = prev_match
                && p + 1 == i
            {
                s += 5;
            }
            let boundary = i == 0
                || matches!(t[i - 1], ' ' | '-' | '_' | ':' | '/' | '\\' | '.')
                || (orig.get(i).is_some_and(|c| c.is_uppercase()) && orig.get(i - 1).is_some_and(|c| c.is_lowercase()));
            if boundary {
                s += 8;
            }
            if i == 0 {
                s += 6;
            }
            score += s;
            prev_match = Some(i);
            qi += 1;
        }
    }
    if qi < q.len() {
        return None;
    }
    // Short texts get a slight edge.
    score -= (t.len() as i32) / 8;
    Some(score)
}

/// Finds an executable on PATH (checks PATHEXT on Windows).
pub fn which(name: &str) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.components().count() > 1 {
        return candidate.exists().then(|| candidate.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        let mut v: Vec<String> = pathext.split(';').filter(|e| !e.is_empty()).map(|e| e.to_lowercase()).collect();
        if Path::new(name).extension().is_some() {
            v.insert(0, String::new());
        }
        v
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let full = dir.join(format!("{name}{ext}"));
            if full.is_file() {
                return Some(full);
            }
        }
    }
    None
}

/// Prepares a `Command` to run a program in the background: on Windows the
/// `.cmd/.bat` shims run through `cmd /C` and no console window is opened.
pub fn command_for(program: &Path) -> std::process::Command {
    #[allow(unused_mut)]
    let mut cmd = {
        let ext = program.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if cfg!(windows) && (ext == "cmd" || ext == "bat") {
            let mut c = std::process::Command::new("cmd");
            c.arg("/C").arg(program);
            c
        } else {
            std::process::Command::new(program)
        }
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.stdin(std::process::Stdio::null());
    cmd
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Simple base64 encoder (to send OSC 52 to the outer terminal).
pub fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let acc = chunk.iter().enumerate().fold(0u32, |acc, (i, b)| acc | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(BASE64[(acc >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Simple base64 decoder (for OSC 52 clipboard requests).
pub fn base64_decode(input: &[u8]) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    }
    let clean: Vec<u8> = input.iter().copied().filter(|c| *c != b'=' && !c.is_ascii_whitespace()).collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut acc = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            acc |= val(*c)? << (18 - 6 * i);
        }
        out.push((acc >> 16) as u8);
        if chunk.len() > 2 {
            out.push((acc >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(acc as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip() {
        for text in ["", "f", "fo", "foo", "foob", "fooba", "foobar", "çğ ✓"] {
            let encoded = base64_encode(text.as_bytes());
            assert_eq!(base64_decode(encoded.as_bytes()).unwrap(), text.as_bytes(), "{encoded}");
        }
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
    }

    #[test]
    fn tilde_only_shortens_paths_under_home() {
        let home = dirs::home_dir().unwrap();
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(tilde(&home), "~");
        assert_eq!(tilde(&home.join("src")), format!("~{sep}src"));
        // A sibling that only shares the name as a text prefix stays as it is.
        let sibling = PathBuf::from(format!("{}2", home.display())).join("x");
        assert_eq!(tilde(&sibling), sibling.display().to_string());
    }

    #[test]
    fn bytes_format() {
        assert_eq!(fmt_bytes(512), "512B");
        assert_eq!(fmt_bytes(1536), "1.5K");
        assert_eq!(fmt_bytes(32 * 1024 * 1024 * 1024), "32.0G");
        assert_eq!(fmt_bytes(300 * 1024 * 1024), "300M");
    }

    #[test]
    fn durations() {
        assert_eq!(fmt_duration(Duration::from_secs(45)), "45s");
        assert_eq!(fmt_duration(Duration::from_secs(2 * 3600 + 14 * 60)), "2h 14m");
        assert_eq!(fmt_duration(Duration::from_secs(4 * 86_400 + 3 * 3600)), "4d 3h");
        assert_eq!(fmt_ago(Duration::from_secs(30)), "now");
        assert_eq!(fmt_ago(Duration::from_secs(7200)), "2h");
    }

    #[test]
    fn truncation() {
        assert_eq!(truncate("hello world", 8), "hello w…");
        assert_eq!(truncate("hi", 8), "hi");
        assert_eq!(truncate_left("C:\\Users\\Mert\\Desktop", 10), "…t\\Desktop");
        assert_eq!(pad_left("7", 3), "  7");
        assert_eq!(pad_right("abc", 5), "abc  ");
    }

    #[test]
    fn fuzzy() {
        assert!(fuzzy_score("spr", "Split Pane Right").is_some());
        assert!(fuzzy_score("xyz", "Split Pane Right").is_none());
        let a = fuzzy_score("new", "New Tab").unwrap();
        let b = fuzzy_score("new", "Go to Next Window").unwrap_or(-100);
        assert!(a > b);
    }

    #[test]
    fn base64() {
        assert_eq!(base64_decode(b"aGVsbG8=").unwrap(), b"hello");
        assert_eq!(base64_decode(b"aGk=").unwrap(), b"hi");
    }
}
