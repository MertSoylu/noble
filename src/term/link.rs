//! Links in terminal output: URLs and paths like `file.rs:42:7`.
//! Opened with ctrl+click; while ctrl is held the link under the mouse is underlined.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub enum Link {
    Url(String),
    File { path: PathBuf, line: Option<u32>, col: Option<u32> },
}

/// The "word" at column `col` of a line: the text between delimiters and the
/// column span it covers (exclusive end).
pub fn token_at(line: &str, col: u16) -> Option<(String, u16, u16)> {
    // One entry per terminal cell: first character, start column, width and byte range in `line`.
    // Wide characters take two columns; zero-width characters (combining marks) belong to the
    // previous cell, as in the terminal grid.
    struct Cell {
        ch: char,
        x: u16,
        w: u16,
        bytes: std::ops::Range<usize>,
    }
    let mut cells: Vec<Cell> = Vec::new();
    let mut x = 0u16;
    for (i, ch) in line.char_indices() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
        if w == 0 {
            if let Some(last) = cells.last_mut() {
                last.bytes.end = i + ch.len_utf8();
            }
            continue;
        }
        cells.push(Cell { ch, x, w, bytes: i..i + ch.len_utf8() });
        x += w;
    }
    let idx = cells.iter().position(|c| col >= c.x && col < c.x + c.w)?;
    let is_delim = |c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '<' | '>' | '|' | '│' | '{' | '}');
    if is_delim(cells[idx].ch) {
        return None;
    }
    let mut start = idx;
    while start > 0 && !is_delim(cells[start - 1].ch) {
        start -= 1;
    }
    let mut end = idx + 1;
    while end < cells.len() && !is_delim(cells[end].ch) {
        end += 1;
    }
    let text = |start: usize, end: usize| &line[cells[start].bytes.start..cells[end - 1].bytes.end];
    // A URL glued to other text (`[docs](https://…)`, `url=https://…`) starts at its scheme.
    let lower = text(start, end).to_ascii_lowercase();
    if let Some(off) = ["https://", "http://", "file://"].iter().filter_map(|s| lower.find(s)).min() {
        let at =
            start + cells[start..end].iter().take_while(|c| c.bytes.start - cells[start].bytes.start < off).count();
        let lead = cells[start..at].iter().all(|c| matches!(c.ch, '(' | '['));
        if !lead {
            if idx >= at { start = at } else { end = at }
        }
    }
    // Opening brackets around the link, then end-of-sentence punctuation and unmatched
    // closing brackets, are not part of it; balanced ones inside (`…/Foo_(bar)`) are.
    while start < end && matches!(cells[start].ch, '(' | '[') {
        start += 1;
    }
    while end > start {
        let t = text(start, end);
        let last = cells[end - 1].ch;
        let unbalanced = |open: char, close: char| last == close && t.matches(open).count() < t.matches(close).count();
        if matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | '(' | '[') || unbalanced('(', ')') || unbalanced('[', ']')
        {
            end -= 1;
        } else {
            break;
        }
    }
    if start >= end {
        return None;
    }
    let (c0, c1) = (cells[start].x, cells[end - 1].x + cells[end - 1].w);
    (col >= c0 && col < c1).then(|| (text(start, end).to_string(), c0, c1))
}

/// Decides whether text is a link; paths are resolved against `cwd` and are
/// only accepted when they actually exist.
pub fn classify(token: &str, cwd: &Path) -> Option<Link> {
    let lower = token.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        let rest = &token[lower.find("://").unwrap_or(0) + 3..];
        return (!rest.is_empty()).then(|| Link::Url(token.to_string()));
    }
    if let Some(rest) = lower.strip_prefix("file://") {
        let path = crate::term::pane::parse_cwd_url(&format!("file://{}", &token[token.len() - rest.len()..]));
        return classify(&path, cwd);
    }
    // Strip the trailing ":line", ":line:col" or "(line,col)" suffixes.
    let (mut path_part, mut nums) = (token, Vec::new());
    for _ in 0..2 {
        match path_part.rsplit_once(':') {
            Some((head, n)) if !n.is_empty() && n.len() <= 7 && n.bytes().all(|b| b.is_ascii_digit()) => {
                nums.insert(0, n.parse::<u32>().ok()?);
                path_part = head;
            }
            _ => break,
        }
    }
    if nums.is_empty()
        && let Some(open) = path_part.rfind('(')
        && path_part.ends_with(')')
    {
        let inner = &path_part[open + 1..path_part.len() - 1];
        let parsed: Vec<u32> = inner.split(',').filter_map(|n| n.trim().parse().ok()).collect();
        if !parsed.is_empty() && parsed.len() <= 2 {
            nums = parsed;
            path_part = &path_part[..open];
        }
    }
    if path_part.is_empty() || path_part.len() > 400 {
        return None;
    }
    let candidate = if let Some(rest) = path_part.strip_prefix("~/").or_else(|| path_part.strip_prefix("~\\")) {
        dirs::home_dir()?.join(rest)
    } else {
        let p = PathBuf::from(path_part);
        if p.is_absolute() { p } else { cwd.join(path_part.trim_start_matches("./").trim_start_matches(".\\")) }
    };
    // Single-word texts ("make", "done") are links only if they are real files.
    let looks_like_path = path_part.contains(['/', '\\', '.']) || !nums.is_empty();
    if !looks_like_path || !candidate.exists() {
        return None;
    }
    Some(Link::File { path: candidate, line: nums.first().copied(), col: nums.get(1).copied() })
}

/// Opens a link in an external app: URLs in the browser, files with `code -g`
/// (if present) or with the system's default application.
pub fn open(link: &Link) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let spawn = |mut c: Command| {
        c.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    };
    let system = |target: &std::ffi::OsStr| {
        let mut c = if cfg!(windows) {
            // `start` interprets '&' inside a URL; rundll32 takes the text as it is.
            let mut c = Command::new("rundll32");
            c.arg("url.dll,FileProtocolHandler");
            c
        } else if cfg!(target_os = "macos") {
            Command::new("open")
        } else {
            Command::new("xdg-open")
        };
        c.arg(target);
        c
    };
    match link {
        Link::Url(url) => spawn(system(std::ffi::OsStr::new(url))),
        Link::File { path, line, col } => {
            if path.is_file()
                && line.is_some()
                && let Some(code) = crate::util::which("code")
            {
                let mut c = crate::util::command_for(&code);
                let target = format!("{}:{}:{}", path.display(), line.unwrap_or(1), col.unwrap_or(1));
                c.arg("-g").arg(target);
                return spawn(c);
            }
            spawn(system(path.as_os_str()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_cut_at_delimiters() {
        let line = "see https://example.com/a_(b). and \"src/main.rs:12:4\", ok";
        let at = |col| token_at(line, col).map(|t| t.0);
        assert_eq!(at(6).as_deref(), Some("https://example.com/a_(b)"));
        assert_eq!(at(40).as_deref(), Some("src/main.rs:12:4"));
        assert_eq!(at(3), None);
        let (_, c0, c1) = token_at(line, 6).unwrap();
        assert_eq!((c0, c1), (4, 29));
        // Wide characters do not shift the column count.
        let wide = "日本 x.rs";
        assert_eq!(token_at(wide, 5).map(|t| (t.0, t.1)), Some(("x.rs".into(), 5)));
        assert_eq!(token_at("(see foo.rs)", 6).map(|t| t.0).as_deref(), Some("foo.rs"));
    }

    /// Every row: line, clicked column, expected token and its column span (None = no token).
    #[test]
    fn token_table() {
        /// Token text, first column, end column.
        type Span<'a> = (&'a str, u16, u16);
        #[rustfmt::skip]
        let rows: &[(&str, u16, Option<Span>)] = &[
            // Plain paths with a location suffix.
            ("src/main.rs:12:3", 2, Some(("src/main.rs:12:3", 0, 16))),
            ("--> src/main.rs:12:3", 6, Some(("src/main.rs:12:3", 4, 20))),
            ("src/main.rs:12:3: error", 2, Some(("src/main.rs:12:3", 0, 16))),
            ("/home/x/y.rs:12", 3, Some(("/home/x/y.rs:12", 0, 15))),
            (r"C:\x\y.rs:12:3", 4, Some((r"C:\x\y.rs:12:3", 0, 14))),
            ("file:12:3 x", 0, Some(("file:12:3", 0, 9))),
            ("file(12,5)", 1, Some(("file(12,5)", 0, 10))),
            ("Foo.cs(12,5): error CS1002", 2, Some(("Foo.cs(12,5)", 0, 12))),
            ("(see foo.rs:12)", 6, Some(("foo.rs:12", 5, 14))),
            // Trailing punctuation.
            ("open src/main.rs.", 6, Some(("src/main.rs", 5, 16))),
            ("a src/main.rs, b", 3, Some(("src/main.rs", 2, 13))),
            ("at src/main.rs:", 4, Some(("src/main.rs", 3, 14))),
            ("see https://x.dev/a).", 5, Some(("https://x.dev/a", 4, 19))),
            ("go https://x.dev/a!?", 5, Some(("https://x.dev/a", 3, 18))),
            ("see https://x.dev/a.", 19, None),
            // URLs in brackets; balanced parentheses inside the URL stay.
            ("(https://x.dev/a)", 3, Some(("https://x.dev/a", 1, 16))),
            ("[https://x.dev/a]", 3, Some(("https://x.dev/a", 1, 16))),
            ("see https://en.wikipedia.org/wiki/Foo_(bar) ok", 6, Some(("https://en.wikipedia.org/wiki/Foo_(bar)", 4, 43))),
            ("(https://en.wikipedia.org/wiki/Foo_(bar))", 3, Some(("https://en.wikipedia.org/wiki/Foo_(bar)", 1, 40))),
            ("(see https://x.dev/a_(b)), next", 7, Some(("https://x.dev/a_(b)", 5, 24))),
            ("[https://x.dev/a_(b)].", 3, Some(("https://x.dev/a_(b)", 1, 20))),
            ("(", 0, None),
            // Markdown links and URLs glued to other text.
            ("[docs](https://x.dev/a) now", 9, Some(("https://x.dev/a", 7, 22))),
            ("see [docs](https://x.dev/a_(b)).", 12, Some(("https://x.dev/a_(b)", 11, 30))),
            ("url=https://x.dev/a", 6, Some(("https://x.dev/a", 4, 19))),
            ("https://a.dev/?u=https://b.dev", 20, Some(("https://a.dev/?u=https://b.dev", 0, 30))),
            // file:// URIs.
            ("open file:///tmp/a%20b.rs now", 8, Some(("file:///tmp/a%20b.rs", 5, 25))),
            ("<file:///C:/x/y.rs>", 3, Some(("file:///C:/x/y.rs", 1, 18))),
            // Unicode: wide characters take two cells, combining marks none.
            ("日本 x.rs", 5, Some(("x.rs", 5, 9))),
            ("エラー: 日本/ファイル.rs:3:1", 9, Some(("日本/ファイル.rs:3:1", 8, 28))),
            ("エラー: 日本/ファイル.rs:3:1", 27, Some(("日本/ファイル.rs:3:1", 8, 28))),
            ("日本/x.rs", 0, Some(("日本/x.rs", 0, 9))),
            ("日本/x.rs", 1, Some(("日本/x.rs", 0, 9))),
            ("dosyalar/çalışma.rs:3 ok", 10, Some(("dosyalar/çalışma.rs:3", 0, 21))),
            ("cafe\u{301}/x.rs:2", 1, Some(("cafe\u{301}/x.rs:2", 0, 11))),
            ("a cafe\u{301}.rs", 5, Some(("cafe\u{301}.rs", 2, 9))),
            ("→ src/ü.rs:1", 3, Some(("src/ü.rs:1", 2, 12))),
            ("x \u{1F600} y.rs", 5, Some(("y.rs", 5, 9))),
            ("x \u{1F600} y.rs", 4, None),
        ];
        let mut failed = Vec::new();
        for &(line, col, want) in rows {
            let got = token_at(line, col);
            let want = want.map(|(t, a, b)| (t.to_string(), a, b));
            if got != want {
                failed.push(format!("{line:?} @{col}: got {got:?}, want {want:?}"));
            }
        }
        assert!(failed.is_empty(), "misdetected tokens:\n{}", failed.join("\n"));
    }

    /// The column span of a token covers exactly the terminal cells that hold its text, as
    /// `vt100` lays the line out (the row text links are detected in comes from the same screen).
    #[test]
    fn token_spans_match_terminal_cells() {
        let lines = [
            "エラー: 日本/ファイル.rs:3:1 x",
            "a cafe\u{301}/ü.rs:2 (https://x.dev/Foo_(b)).",
            "\u{1F600} [d](https://x.dev) y",
        ];
        for line in lines {
            let mut parser = vt100::Parser::new(2, 80, 0);
            parser.process(line.as_bytes());
            let screen = parser.screen();
            let row = screen.rows(0, 80).next().unwrap();
            for col in 0..40u16 {
                let Some((token, c0, c1)) = token_at(&row, col) else { continue };
                let cells: String = (c0..c1).filter_map(|c| screen.cell(0, c)).map(|c| c.contents()).collect();
                assert_eq!(cells, token, "{line:?} @{col}");
            }
        }
    }

    /// Every row: token, expected link (paths relative to a temporary directory).
    #[test]
    fn classify_table() {
        let dir = std::env::temp_dir().join(format!("noble-link-table-{}", std::process::id()));
        for f in ["src/main.rs", "ünï/日本.rs", "a b.rs", "cafe\u{301}.rs", "Makefile"] {
            let p = dir.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        let file = |f: &str, line: Option<u32>, col: Option<u32>| Some(Link::File { path: dir.join(f), line, col });
        let url = |u: &str| Some(Link::Url(u.to_string()));
        let abs = dir.join("src/main.rs").display().to_string();
        let file_uri = format!("file://{}", dir.join("a%20b.rs").display().to_string().replace('\\', "/"));
        let file_uri = if cfg!(windows) { file_uri.replacen("file://", "file:///", 1) } else { file_uri };
        let rows: Vec<(String, Option<Link>)> = vec![
            ("https://x.dev/a?b=1&c".into(), url("https://x.dev/a?b=1&c")),
            ("HTTPS://X.dev".into(), url("HTTPS://X.dev")),
            ("http://a".into(), url("http://a")),
            ("https://".into(), None),
            ("http://".into(), None),
            ("src/main.rs".into(), file("src/main.rs", None, None)),
            ("src/main.rs:12".into(), file("src/main.rs", Some(12), None)),
            ("src/main.rs:12:3".into(), file("src/main.rs", Some(12), Some(3))),
            ("./src/main.rs(7,2)".into(), file("src/main.rs", Some(7), Some(2))),
            ("src/main.rs(7)".into(), file("src/main.rs", Some(7), None)),
            ("src/main.rs(7, 2)".into(), file("src/main.rs", Some(7), Some(2))),
            (format!("{abs}:12"), file("src/main.rs", Some(12), None)),
            (format!("{abs}:12:3"), file("src/main.rs", Some(12), Some(3))),
            ("Makefile:4".into(), file("Makefile", Some(4), None)),
            ("ünï/日本.rs:3:1".into(), file("ünï/日本.rs", Some(3), Some(1))),
            ("cafe\u{301}.rs:2".into(), file("cafe\u{301}.rs", Some(2), None)),
            (file_uri.clone(), file("a b.rs", None, None)),
            (format!("{file_uri}:9"), file("a b.rs", Some(9), None)),
            ("src/missing.rs:3".into(), None),
            ("src/main.rs:".into(), None),
            ("done".into(), None),
            ("Makefile".into(), None),
        ];
        let mut failed = Vec::new();
        for (token, want) in rows {
            let got = classify(&token, &dir);
            if got != want {
                failed.push(format!("{token:?}: got {got:?}, want {want:?}"));
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(failed.is_empty(), "misclassified tokens:\n{}", failed.join("\n"));
    }

    /// Windows paths with a drive letter; they cannot exist on other systems.
    #[test]
    fn classify_windows_drive_paths() {
        if !cfg!(windows) {
            // Linux/macOS: `C:\...` is not an absolute path there; covered by the Windows CI job.
            return;
        }
        let dir = std::env::temp_dir().join(format!("noble-link-win-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("x")).unwrap();
        std::fs::write(dir.join("x").join("y.rs"), "").unwrap();
        let p = dir.join("x").join("y.rs");
        let s = p.display().to_string();
        let want = |line, col| Some(Link::File { path: p.clone(), line, col });
        assert_eq!(classify(&format!("{s}:12:3"), &dir), want(Some(12), Some(3)));
        assert_eq!(classify(&format!("{s}(12,5)"), &dir), want(Some(12), Some(5)));
        let uri = format!("file:///{}", s.replace('\\', "/"));
        assert_eq!(classify(&uri, &dir), want(None, None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn classify_urls_and_paths() {
        let dir = std::env::temp_dir().join(format!("noble-link-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {}").unwrap();
        assert_eq!(classify("https://x.dev/a?b=1&c", &dir), Some(Link::Url("https://x.dev/a?b=1&c".into())));
        assert_eq!(
            classify("src/main.rs:12:4", &dir),
            Some(Link::File { path: dir.join("src/main.rs"), line: Some(12), col: Some(4) })
        );
        assert_eq!(
            classify("./src/main.rs(7,2)", &dir),
            Some(Link::File { path: dir.join("src/main.rs"), line: Some(7), col: Some(2) })
        );
        assert!(matches!(classify("src", &dir), None | Some(Link::File { .. })));
        assert_eq!(classify("src/missing.rs:3", &dir), None);
        assert_eq!(classify("done", &dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
