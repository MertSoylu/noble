//! Terminal çıktısındaki bağlantılar: URL'ler ve `dosya.rs:42:7` biçimindeki yollar.
//! ctrl+tık ile açılır; ctrl basılıyken fare altındaki bağlantının altı çizilir.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub enum Link {
    Url(String),
    File { path: PathBuf, line: Option<u32>, col: Option<u32> },
}

/// Satırda `col` sütunundaki "kelime": sınırlayıcılar arasında kalan metin ve
/// kapladığı sütun aralığı (bitiş hariç).
pub fn token_at(line: &str, col: u16) -> Option<(String, u16, u16)> {
    // Her karakterin başladığı sütun (geniş karakterler iki sütun kaplar).
    let mut cells: Vec<(char, u16, u16)> = Vec::new();
    let mut x = 0u16;
    for ch in line.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
        if w == 0 {
            continue;
        }
        cells.push((ch, x, w));
        x += w;
    }
    let idx = cells.iter().position(|(_, x, w)| col >= *x && col < x + w)?;
    let is_delim = |c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '<' | '>' | '|' | '│' | '{' | '}');
    if is_delim(cells[idx].0) {
        return None;
    }
    let mut start = idx;
    while start > 0 && !is_delim(cells[start - 1].0) {
        start -= 1;
    }
    let mut end = idx + 1;
    while end < cells.len() && !is_delim(cells[end].0) {
        end += 1;
    }
    // Cümle sonu noktalaması ve eşlenmemiş kapanış parantezleri bağlantıya dahil değil.
    loop {
        let text: String = cells[start..end].iter().map(|c| c.0).collect();
        let last = cells[end - 1].0;
        let unbalanced =
            |open: char, close: char| last == close && text.matches(open).count() < text.matches(close).count();
        if end - start > 1
            && (matches!(last, '.' | ',' | ';' | ':' | '!' | '?') || unbalanced('(', ')') || unbalanced('[', ']'))
        {
            end -= 1;
        } else {
            break;
        }
    }
    while start < end && matches!(cells[start].0, '(' | '[') {
        start += 1;
    }
    if start >= end {
        return None;
    }
    let text: String = cells[start..end].iter().map(|c| c.0).collect();
    let (c0, c1) = (cells[start].1, cells[end - 1].1 + cells[end - 1].2);
    (col >= c0 && col < c1).then_some((text, c0, c1))
}

/// Metnin bağlantı olup olmadığını belirler; yollar `cwd`'ye göre çözülür ve
/// yalnızca gerçekten varsa kabul edilir.
pub fn classify(token: &str, cwd: &Path) -> Option<Link> {
    let lower = token.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return (token.len() > 8).then(|| Link::Url(token.to_string()));
    }
    if let Some(rest) = lower.strip_prefix("file://") {
        let path = crate::term::pane::parse_cwd_url(&format!("file://{}", &token[token.len() - rest.len()..]));
        return classify(&path, cwd);
    }
    // Sondaki ":satır" ve ":satır:sütun" ya da "(satır,sütun)" eklerini ayır.
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
    // Tek kelimelik metinler ("make", "done") ancak gerçekten dosyaysa bağlantıdır.
    let looks_like_path = path_part.contains(['/', '\\', '.']) || !nums.is_empty();
    if !looks_like_path || !candidate.exists() {
        return None;
    }
    Some(Link::File { path: candidate, line: nums.first().copied(), col: nums.get(1).copied() })
}

/// Bağlantıyı dış uygulamada açar: URL tarayıcıda, dosya `code -g` (varsa) ya da
/// sistemin varsayılan uygulamasında.
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
            // `start` bir URL'deki & işaretlerini yorumlar; rundll32 metni olduğu gibi alır.
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
        // Geniş karakterler sütun hesabını kaydırmaz.
        let wide = "日本 x.rs";
        assert_eq!(token_at(wide, 5).map(|t| (t.0, t.1)), Some(("x.rs".into(), 5)));
        assert_eq!(token_at("(see foo.rs)", 6).map(|t| t.0).as_deref(), Some("foo.rs"));
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
