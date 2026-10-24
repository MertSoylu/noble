//! vt100 callbacks: the OSC sequences a shell or program sends (window title, working directory,
//! clipboard, notifications, prompt marks, exit codes, OSC 8 links) and the OSC 7 path helpers.

use super::pane::{history_len, span_text};
use crate::util;

/// vt100 callbacks: title, working directory, terminal query replies, clipboard.
#[derive(Default)]
pub struct Callbacks {
    pub title: String,
    pub cwd: Option<String>,
    /// Replies the emulator must write back to the PTY (DSR, DA).
    pub responses: Vec<u8>,
    pub clipboard: Option<String>,
    pub bell: bool,
    /// The shell drew a new prompt (OSC 7 / 9;9 / 133): the previous command finished.
    /// See `Callbacks::shell_prompt` for what else that resets.
    pub prompt: bool,
    /// Exit code of the command that just finished, from `OSC 133;D;<code>` (NOBLE's own hooks send it
    /// before the directory report; cmd.exe cannot, so it stays `None` there). Taken with `prompt`.
    /// The first code since the last take wins: NOBLE's hook reads the status before anything else
    /// runs, and a user's prompt may send its own `133;D;<code>` right after with a different one.
    pub exit: Option<i32>,
    /// When the shell last said a command line starts running (`OSC 133;C`: bash's `PS0`, zsh
    /// `preexec`, fish `fish_preexec`, PowerShell's read-line wrapper). It is not sent for an empty
    /// line, so it tells a command that ran from an Enter that only brought the prompt back.
    pub started: Option<std::time::Instant>,
    /// Desktop notification sent by the app (OSC 9 text, OSC 777;notify).
    pub notice: Option<String>,
    /// OSC 8 links (newest last, capped in number).
    pub hyperlinks: std::collections::VecDeque<Hyperlink>,
    /// An OSC 8 link opened but not yet closed: (absolute line, column, address).
    open_link: Option<(usize, u16, String)>,
    /// The host named by the pane's first OSC 7 report (its own shell, before any `ssh`).
    local_host: Option<String>,
}

impl Callbacks {
    /// The shell is back at its prompt, so the program that ran before it has exited:
    /// the window title it set no longer describes the pane (a shell that titles its
    /// prompt sets a new one right after). Only shells emit these marks; agents such as
    /// Claude Code set a title (OSC 0) and progress (OSC 9;4) but never OSC 7 / 9;9 / 133.
    fn shell_prompt(&mut self) {
        self.prompt = true;
        self.title.clear();
    }
}

/// An exit code as a shell prints it. PowerShell may report a Windows status such as `3221225786`
/// (above `i32::MAX`): that is kept as the same 32 bits, the way `$LASTEXITCODE` shows it.
fn parse_exit_code(raw: &[u8]) -> Option<i32> {
    let text = std::str::from_utf8(raw).ok()?.trim();
    text.parse::<i32>().ok().or_else(|| text.parse::<u32>().ok().map(|c| c as i32))
}

/// Text marked with OSC 8: absolute line (0 = oldest scrollback line) and column span.
///
/// vt100 does not report how many lines scrolled out of a full scrollback, so `line` drifts once the
/// scrollback is full. `text` (the marked cells when the link closed) lets a lookup discard a link
/// whose line no longer holds that text instead of returning the wrong address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hyperlink {
    pub line: usize,
    pub from: u16,
    pub to: u16,
    pub url: String,
    pub text: String,
}

/// Maximum number of OSC 8 links kept.
const MAX_HYPERLINKS: usize = 1000;

impl vt100::Callbacks for Callbacks {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = String::from_utf8_lossy(title).trim().to_string();
    }

    fn audible_bell(&mut self, _: &mut vt100::Screen) {
        self.bell = true;
    }

    fn copy_to_clipboard(&mut self, _: &mut vt100::Screen, _ty: &[u8], data: &[u8]) {
        if let Some(bytes) = util::base64_decode(data) {
            self.clipboard = Some(String::from_utf8_lossy(&bytes).into_owned());
        }
    }

    fn unhandled_osc(&mut self, screen: &mut vt100::Screen, params: &[&[u8]]) {
        let join = |parts: &[&[u8]]| {
            parts.iter().map(|p| String::from_utf8_lossy(p).into_owned()).collect::<Vec<_>>().join(";")
        };
        match params {
            // OSC 7: file://host/path
            [b"7", rest @ ..] if !rest.is_empty() => {
                let url = join(rest);
                // A remote shell (`ssh` from the pane) reports its own host's path: still a prompt, but
                // not a directory here (a split would open a missing or unrelated local folder).
                let host = cwd_url_host(&url);
                let local = host.as_deref().is_none_or(|h| {
                    is_local_host(h) || self.local_host.as_deref().is_none_or(|first| same_host(first, h))
                });
                if let Some(h) = &host
                    && self.local_host.is_none()
                    && local
                {
                    self.local_host = Some(h.clone());
                }
                if local {
                    let path = parse_cwd_url(&url);
                    self.cwd = Some(if cfg!(windows) { windows_cwd(&path) } else { path });
                }
                self.shell_prompt();
            }
            // OSC 9;9: Windows Terminal's cwd report. A network path is not taken (see `is_network_path`): the
            // pane falls back to the folder it started in, and nothing ever probes the share.
            [b"9", b"9", rest @ ..] if !rest.is_empty() => {
                let reported = join(rest);
                let dir = reported.trim_matches(|c: char| c == '"' || c.is_whitespace());
                self.cwd = (!is_network_path(dir)).then(|| dir.to_string());
                self.shell_prompt();
            }
            // OSC 9;<text>: iTerm2-style notification. Numeric subcodes (ConEmu
            // extensions such as 9;4 progress) are not notifications.
            [b"9", rest @ ..] if !rest.is_empty() && !rest[0].iter().all(u8::is_ascii_digit) => {
                self.set_notice(join(rest));
            }
            // OSC 777;notify;title;body (rxvt / Ghostty / WezTerm).
            [b"777", b"notify", rest @ ..] => {
                let parts: Vec<String> = rest.iter().map(|p| String::from_utf8_lossy(p).trim().to_string()).collect();
                self.set_notice(parts.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" · "));
            }
            // OSC 8;params;address … OSC 8;; — an empty address closes the link.
            // The address may contain ";", so the remaining parts are joined.
            [b"8", _params, uri @ ..] => {
                let url = join(uri);
                let (row, col) = screen.cursor_position();
                let line = history_len(screen) + row as usize;
                if let Some((l0, c0, open)) = self.open_link.take() {
                    let cols = crate::term::screen_size(screen).1;
                    // A span whose line already left the scrollback cannot be verified later: dropped.
                    let mut span = |line: usize, from: u16, to: u16| {
                        if let Some(text) = span_text(screen, line, from, to) {
                            self.push_link(Hyperlink { line, from, to, url: open.clone(), text });
                        }
                    };
                    if line == l0 {
                        span(line, c0, col);
                    } else {
                        // Line-spanning link: end of the first line and start of the last.
                        span(l0, c0, cols);
                        span(line, 0, col);
                    }
                }
                if !url.is_empty() {
                    self.open_link = Some((line, col, url));
                }
            }
            // OSC 133: FinalTerm/VS Code prompt marks. A = prompt started, D = command finished,
            // optionally with its exit code (`133;D;1`). A bare D keeps a code already reported, and
            // so does a second code: the first one of a prompt is NOBLE's own (see `exit`).
            [b"133", kind, rest @ ..] if matches!(kind.first(), Some(b'A' | b'D')) => {
                if kind.first() == Some(&b'D')
                    && let Some(code) = rest.first().and_then(|c| parse_exit_code(c))
                {
                    self.exit.get_or_insert(code);
                }
                self.shell_prompt();
            }
            // OSC 133;C: the command line typed at the prompt starts running.
            [b"133", kind, ..] if kind.first() == Some(&b'C') => {
                self.started = Some(std::time::Instant::now());
            }
            _ => {}
        }
    }

    fn visual_bell(&mut self, _: &mut vt100::Screen) {
        self.bell = true;
    }

    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let first = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        match (i1, c) {
            // DSR: cursor position. ConPTY asks for this at startup and waits for the answer.
            (None, 'n') if first == 6 => {
                let (row, col) = screen.cursor_position();
                self.responses.extend_from_slice(format!("\x1b[{};{}R", row + 1, col + 1).as_bytes());
            }
            (None, 'n') if first == 5 => self.responses.extend_from_slice(b"\x1b[0n"),
            // Primary device attributes.
            (None, 'c') if first == 0 => self.responses.extend_from_slice(b"\x1b[?1;2c"),
            // Secondary device attributes.
            (Some(b'>'), 'c') => self.responses.extend_from_slice(b"\x1b[>0;10;1c"),
            _ => {}
        }
    }
}

impl Callbacks {
    fn push_link(&mut self, link: Hyperlink) {
        if link.to > link.from && !link.url.is_empty() {
            self.hyperlinks.push_back(link);
            if self.hyperlinks.len() > MAX_HYPERLINKS {
                self.hyperlinks.pop_front();
            }
        }
    }

    fn set_notice(&mut self, text: String) {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if !text.is_empty() {
            self.notice = Some(util::truncate(&text, 120));
        }
    }
}

/// Turns an OSC 7 URL into a local path: `file://host/C:/x` → `C:/x`.
pub fn parse_cwd_url(raw: &str) -> String {
    let mut p = raw.trim().to_string();
    if let Some(after) = p.strip_prefix("file://") {
        p = match after.find('/') {
            Some(i) => after[i..].to_string(),
            None => after.to_string(),
        };
        p = percent_decode(&p);
        let b = p.as_bytes();
        if b.len() >= 3 && b[0] == b'/' && b[2] == b':' && b[1].is_ascii_alphabetic() {
            p.remove(0);
        }
    }
    p
}

/// The host of an OSC 7 `file://host/path` URL; `None` when there is none (`file:///x`, a bare path).
pub fn cwd_url_host(raw: &str) -> Option<String> {
    let after = raw.trim().strip_prefix("file://")?;
    let host = &after[..after.find('/').unwrap_or(after.len())];
    (!host.is_empty()).then(|| percent_decode(host))
}

/// Host names compare without case and by their first label (`box` and `box.lan` are one machine).
pub(super) fn same_host(a: &str, b: &str) -> bool {
    let short = |h: &str| h.split('.').next().unwrap_or(h).to_ascii_lowercase();
    short(a) == short(b)
}

/// Is `host` this machine? `localhost` always is; otherwise it is compared with the system host name.
fn is_local_host(host: &str) -> bool {
    static NAME: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    NAME.get_or_init(sysinfo::System::host_name).as_deref().is_some_and(|name| same_host(name, host))
}

/// Does `path` start with two separators: `\\server\share`, `//server/share`, `\\?\UNC\…`, a `\\.\` device?
/// On Windows such a path names another machine (or a device), and touching it connects there and sends the
/// user's credentials (NTLM) to that machine. Text from the terminal is checked with this before any file
/// system call. Unix: `//x` means `/x`, so refusing it costs nothing there.
pub fn is_network_path(path: &str) -> bool {
    let b = path.as_bytes();
    if b.len() < 2 || !matches!(b[0], b'\\' | b'/') || !matches!(b[1], b'\\' | b'/') {
        return false;
    }
    // A verbatim local path (`\\?\C:\x`, `//?/C:/x`: what `std::fs::canonicalize` returns on Windows) names a folder
    // on this machine. `\\?\UNC\server\share` and every other `\\` path are still network paths.
    let verbatim_drive =
        b.len() >= 6 && b[2] == b'?' && matches!(b[3], b'\\' | b'/') && b[4].is_ascii_alphabetic() && b[5] == b':';
    !verbatim_drive
}

/// An OSC 7 path as a Windows directory: Git Bash's `/c/x` → `C:\x`, its `//server/share` → `\\server\share`,
/// with backslashes throughout (explorer.exe and "copy path" want them).
pub fn windows_cwd(path: &str) -> String {
    let p = msys_to_windows(path);
    let unc = p.starts_with("//") && !p.starts_with("///");
    if !unc && !(p.len() >= 2 && p.as_bytes()[1] == b':') {
        // Not a Windows path (a remote or unconverted MSYS path): left as reported.
        return p;
    }
    p.replace('/', "\\")
}

/// Git Bash / MSYS2 report Windows directories as `/c/Users/me`; turns that into `C:/Users/me`.
pub fn msys_to_windows(path: &str) -> String {
    let b = path.as_bytes();
    if b.len() >= 2 && b[0] == b'/' && b[1].is_ascii_alphabetic() && (b.len() == 2 || b[2] == b'/') {
        return format!("{}:/{}", (b[1] as char).to_ascii_uppercase(), path[2..].trim_start_matches('/'));
    }
    path.to_string()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let hex = |b: u8| (b as char).to_digit(16);
    while i < bytes.len() {
        // Works on bytes: slicing the `str` could split a multi-byte character ("%aé").
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
