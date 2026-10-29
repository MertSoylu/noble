//! vt100 callbacks: the OSC sequences a shell or program sends (window title, working directory,
//! clipboard, notifications, prompt marks, exit codes, OSC 8 links) and the OSC 7 path helpers.

use super::blocks::{CommandBlock, MAX_BLOCKS, retain_live};
use super::colors::{OscScan, SCHEME_NOTICES, SharedColors, Terminator};
use super::pane::{screen_base, span_text};
use crate::util;

/// vt100 callbacks: title, working directory, terminal query replies, clipboard.
#[derive(Default)]
pub struct Callbacks {
    /// The pane's colors for the OSC 4 / 10 / 11 / 12 and `CSI ? 996 n` queries (`None`: they are not answered).
    pub colors: Option<SharedColors>,
    /// Answers to color queries still waiting for the terminator of their OSC string (`seal_color_replies`).
    color_replies: Vec<String>,
    /// Finds how the OSC strings in the output end (`feed`).
    pub(super) osc_scan: OscScan,
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
    /// The commands the shell ran in this pane, oldest first: opened by `133;C`, closed by `133;D` (see
    /// `open_block`, `close_block`). Pruned where the numbering is no longer valid (`retain_live`).
    pub blocks: std::collections::VecDeque<CommandBlock>,
    /// Desktop notification sent by the app (OSC 9 text, OSC 777;notify).
    pub notice: Option<String>,
    /// The progress report the program set with OSC 9;4 (see `Progress`). Cleared by state 0 and by the next
    /// prompt mark (`shell_prompt`), like a shell's own title.
    pub progress: Option<Progress>,
    /// `progress` changed since the app last took the flag (`Pane::take_progress_changed`): the pane's title and
    /// tab need a redraw.
    pub progress_changed: bool,
    /// OSC 8 links (newest last, capped in number).
    pub hyperlinks: std::collections::VecDeque<Hyperlink>,
    /// An OSC 8 link opened but not yet closed: (absolute line, column, address).
    open_link: Option<(usize, u16, String, u64)>,
    /// The host named by the pane's first OSC 7 report (its own shell, before any `ssh`).
    local_host: Option<String>,
    /// The `TRACKED_MODES` a program has set (`CSI ? Ps h`) and not reset since.
    tracked_modes: std::collections::BTreeSet<u16>,
    /// When the program opened the synchronized-output block it is in (`CSI ? 2026 h`); `None` outside one.
    /// A block that outlives `sync_timeout` no longer counts as open (see `sync_left`).
    sync_since: Option<std::time::Instant>,
    /// Replaces `SYNC_TIMEOUT` (tests make the block long or short).
    sync_timeout: Option<std::time::Duration>,
    /// The last chunk of output ended in ESC, so a `c` at the start of the next one is a full reset (RIS).
    pub(super) ended_in_esc: bool,
}

/// Synchronized output (DECSET 2026): the program brackets a frame in `CSI ? 2026 h` … `l`, and the terminal shows
/// it only when it is complete. The reader thread holds the redraw back meanwhile (`Pane::spawn`).
pub const SYNC_OUTPUT: u16 = 2026;

/// How long a synchronized block may stay open: a program that never sends the `l` must not freeze its pane.
pub const SYNC_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(150);

/// Focus reports (DECSET 1004): the program gets `CSI I` / `CSI O` when its pane gains or loses the focus
/// (`App::sync_focus_reports`).
pub const FOCUS_REPORTS: u16 = 1004;

/// Private DEC modes that vt100 passes on to `unhandled_csi` (it does not act on them), so NOBLE records their
/// set/reset itself for DECRQM. A mode vt100 handles is read from its screen instead (`Callbacks::private_mode`).
const TRACKED_MODES: &[u16] = &[FOCUS_REPORTS, 1047, SYNC_OUTPUT, SCHEME_NOTICES];

impl Callbacks {
    /// Callbacks that answer color queries from `colors`.
    pub fn with_colors(colors: SharedColors) -> Callbacks {
        Callbacks { colors: Some(colors), ..Callbacks::default() }
    }

    /// The shell is back at its prompt, so the program that ran before it has exited:
    /// the window title it set no longer describes the pane (a shell that titles its
    /// prompt sets a new one right after). Only shells emit these marks; agents such as
    /// Claude Code set a title (OSC 0) and progress (OSC 9;4) but never OSC 7 / 9;9 / 133.
    fn shell_prompt(&mut self) {
        self.prompt = true;
        self.title.clear();
        // The program that reported its progress has exited with the prompt (a bar left behind would read as busy).
        self.store_progress(None);
    }
}

/// An exit code as a shell prints it. PowerShell may report a Windows status such as `3221225786`
/// (above `i32::MAX`): that is kept as the same 32 bits, the way `$LASTEXITCODE` shows it.
fn parse_exit_code(raw: &[u8]) -> Option<i32> {
    let text = std::str::from_utf8(raw).ok()?.trim();
    text.parse::<i32>().ok().or_else(|| text.parse::<u32>().ok().map(|c| c as i32))
}

/// Text marked with OSC 8: stable line number (`pane::screen_base`: lines scrolled out plus scrollback index) and
/// column span.
///
/// The number stays right while output scrolls, a full scrollback included. A reflow renumbers the lines: the
/// links of an older `epoch` (`Screen::reflow_epoch`) are dropped. `text` (the marked cells when the link closed) is
/// a last guard that lets a lookup discard a link whose line no longer holds that text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hyperlink {
    pub line: usize,
    pub from: u16,
    pub to: u16,
    pub url: String,
    pub text: String,
    pub epoch: u64,
}

/// Maximum number of OSC 8 links kept.
const MAX_HYPERLINKS: usize = 1000;

/// A progress report from OSC 9;4 (`9;4;<state>[;<progress>]`, ConEmu's form that Windows Terminal, Ghostty and
/// Claude Code send). A pane shows it in its title and its tab shows it as busy; an agent without hooks counts as
/// working while it reports busy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub state: ProgressState,
    /// Percent done, 0 to 100 (values past 100 are clamped); `None` when the report has none, and always for the
    /// busy-without-percent state.
    pub percent: Option<u8>,
}

/// The states of a progress report (the number after `9;4;`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressState {
    /// 1: work in progress, with a percentage when the report has one.
    Normal,
    /// 2: the work failed.
    Error,
    /// 3: busy, no percentage.
    Indeterminate,
    /// 4: paused, or a warning.
    Paused,
}

impl Progress {
    /// Work is going on (states 1 and 3).
    pub fn busy(&self) -> bool {
        matches!(self.state, ProgressState::Normal | ProgressState::Indeterminate)
    }

    /// A busy report with no percentage to show: a spinner stands for it.
    pub fn spins(&self) -> bool {
        self.busy() && self.percent.is_none()
    }
}

impl crate::vt100::Callbacks for Callbacks {
    fn set_window_title(&mut self, _: &mut crate::vt100::Screen, title: &[u8]) {
        self.title = String::from_utf8_lossy(title).trim().to_string();
    }

    fn audible_bell(&mut self, _: &mut crate::vt100::Screen) {
        self.bell = true;
    }

    fn copy_to_clipboard(&mut self, _: &mut crate::vt100::Screen, _ty: &[u8], data: &[u8]) {
        if let Some(bytes) = util::base64_decode(data) {
            self.clipboard = Some(String::from_utf8_lossy(&bytes).into_owned());
        }
    }

    fn unhandled_osc(&mut self, screen: &mut crate::vt100::Screen, params: &[&[u8]]) {
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
            // OSC 9;4;<state>[;<progress>]: ConEmu's progress report (`Progress`). It is no notification.
            [b"9", b"4", state, rest @ ..] => self.set_progress(state, rest.first().copied()),
            // OSC 9;<text>: iTerm2-style notification. Numeric subcodes (other ConEmu
            // extensions) are not notifications.
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
                let line = screen_base(screen) + row as usize;
                let epoch = screen.reflow_epoch();
                if let Some((l0, c0, open, opened)) = self.open_link.take() {
                    let cols = crate::term::screen_size(screen).1;
                    // A span whose line already left the scrollback cannot be verified later: dropped.
                    let mut span = |line: usize, from: u16, to: u16| {
                        if let Some(text) = span_text(screen, line, from, to) {
                            self.push_link(Hyperlink { line, from, to, url: open.clone(), text, epoch });
                        }
                    };
                    if opened != epoch {
                        // A reflow between the start and the end moved the start: no span to take.
                    } else if line == l0 {
                        span(line, c0, col);
                    } else {
                        // Line-spanning link: end of the first line and start of the last.
                        span(l0, c0, cols);
                        span(line, 0, col);
                    }
                }
                if !url.is_empty() {
                    self.open_link = Some((line, col, url, epoch));
                }
            }
            // OSC 133: FinalTerm/VS Code prompt marks. A = prompt started, D = command finished,
            // optionally with its exit code (`133;D;1`). A bare D keeps a code already reported, and
            // so does a second code: the first one of a prompt is NOBLE's own (see `exit`).
            [b"133", kind, rest @ ..] if matches!(kind.first(), Some(b'A' | b'D')) => {
                let code = rest.first().and_then(|c| parse_exit_code(c));
                if kind.first() == Some(&b'D') {
                    if let Some(code) = code {
                        self.exit.get_or_insert(code);
                    }
                    self.close_block(screen, code);
                }
                self.shell_prompt();
            }
            // OSC 133;C: the command line typed at the prompt starts running.
            [b"133", kind, ..] if kind.first() == Some(&b'C') => {
                self.started = Some(std::time::Instant::now());
                self.open_block(screen);
            }
            // Color queries (`?`): OSC 4 palette entries, 10 text, 11 background, 12 cursor. The answer waits for the
            // string's terminator (`seal_color_replies`). A color being set, or reset (104, 110 …), is ignored.
            [number @ (b"4" | b"10" | b"11" | b"12"), rest @ ..] => {
                if let Some(colors) = &self.colors {
                    let report = colors.read().unwrap_or_else(|e| e.into_inner());
                    self.color_replies.extend(report.answers(number, rest));
                }
            }
            _ => {}
        }
    }

    fn visual_bell(&mut self, _: &mut crate::vt100::Screen) {
        self.bell = true;
    }

    fn unhandled_csi(
        &mut self,
        screen: &mut crate::vt100::Screen,
        i1: Option<u8>,
        i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let first = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        match (i1, i2, c) {
            // DSR: cursor position. ConPTY asks for this at startup and waits for the answer.
            (None, None, 'n') if first == 6 => {
                let (row, col) = screen.cursor_position();
                self.responses.extend_from_slice(format!("\x1b[{};{}R", row + 1, col + 1).as_bytes());
            }
            (None, None, 'n') if first == 5 => self.responses.extend_from_slice(b"\x1b[0n"),
            // Color scheme query: `CSI ? 996 n` → `CSI ? 997 ; 1 n` (dark) or `; 2 n` (light).
            (Some(b'?'), None, 'n') if first == 996 => {
                if let Some(colors) = &self.colors {
                    let report = colors.read().unwrap_or_else(|e| e.into_inner());
                    self.responses.extend_from_slice(report.scheme_notice());
                }
            }
            // Primary device attributes.
            (None, None, 'c') if first == 0 => self.responses.extend_from_slice(b"\x1b[?1;2c"),
            // Secondary device attributes.
            (Some(b'>'), None, 'c') => self.responses.extend_from_slice(b"\x1b[>0;10;1c"),
            // XTVERSION: `CSI > q` (or `CSI > 0 q`) asks for the name and version of the terminal.
            (Some(b'>'), None, 'q') if first == 0 => {
                self.responses
                    .extend_from_slice(format!("\x1bP>|NOBLE {}\x1b\\", env!("CARGO_PKG_VERSION")).as_bytes());
            }
            // DECRQM for private modes: `CSI ? Ps $ p` → `CSI ? Ps ; Pm $ y`, Pm 1 set, 2 reset, 0 not recognized.
            (Some(b'?'), Some(b'$'), 'p') => {
                let pm = match self.private_mode(screen, first) {
                    Some(true) => 1,
                    Some(false) => 2,
                    None => 0,
                };
                self.responses.extend_from_slice(format!("\x1b[?{first};{pm}$y").as_bytes());
            }
            // DECRQM for ANSI modes: `CSI Ps $ p` → `CSI Ps ; Pm $ y`. NOBLE tracks no ANSI mode yet, so none is
            // recognized.
            (Some(b'$'), None, 'p') => self.responses.extend_from_slice(format!("\x1b[{first};0$y").as_bytes()),
            // Set and reset of a tracked private mode. vt100 hands over each unhandled mode of a sequence with the
            // whole parameter list, so the modes that are not tracked are skipped here.
            (Some(b'?'), None, 'h' | 'l') => {
                for mode in params.iter().filter_map(|p| p.first()).filter(|m| TRACKED_MODES.contains(*m)) {
                    if c == 'h' {
                        self.tracked_modes.insert(*mode);
                    } else {
                        self.tracked_modes.remove(mode);
                    }
                    if *mode == SYNC_OUTPUT {
                        // A second `h` inside an open block does not extend it; one after the timeout starts a new one.
                        if c == 'l' {
                            self.sync_since = None;
                        } else if !self.sync_open() {
                            self.sync_since = Some(std::time::Instant::now());
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

impl Callbacks {
    /// Time left of the open synchronized block at `now`; `None` when no block is open (none was started, it ended,
    /// or it ran past the timeout).
    pub fn sync_left_at(&self, now: std::time::Instant) -> Option<std::time::Duration> {
        let limit = self.sync_timeout.unwrap_or(SYNC_TIMEOUT);
        let used = now.saturating_duration_since(self.sync_since?);
        limit.checked_sub(used).filter(|left| !left.is_zero())
    }

    /// Time left of the open synchronized block (see `sync_left_at`).
    pub fn sync_left(&self) -> Option<std::time::Duration> {
        self.sync_left_at(std::time::Instant::now())
    }

    /// Whether a synchronized block is open now: the frame being drawn is not complete.
    pub fn sync_open(&self) -> bool {
        self.sync_left().is_some()
    }

    /// Sets the longest a block may stay open; `None` restores `SYNC_TIMEOUT`. For tests.
    pub fn set_sync_timeout(&mut self, timeout: Option<std::time::Duration>) {
        self.sync_timeout = timeout;
    }

    /// Ends the color answers waiting for a terminator: they go out the way the query ended (BEL or ST).
    pub fn seal_color_replies(&mut self, end: Terminator) {
        for body in self.color_replies.drain(..) {
            self.responses.extend_from_slice(body.as_bytes());
            self.responses.extend_from_slice(end.bytes());
        }
    }

    /// A full reset (RIS) or the end of the program: the modes it set are gone, an open block is over.
    pub fn reset_modes(&mut self) {
        self.tracked_modes.clear();
        self.sync_since = None;
    }

    /// Whether a `TRACKED_MODES` mode is set (`CSI ? Ps h` seen and not reset since).
    pub fn mode_on(&self, mode: u16) -> bool {
        self.tracked_modes.contains(&mode)
    }

    /// Whether a private DEC mode is set, for DECRQM. `None` when NOBLE does not recognize the mode.
    /// The modes vt100 handles are read from the screen; the `TRACKED_MODES` are read from `tracked_modes`, so adding
    /// a mode there is enough for DECRQM. A mode vt100 handles needs its own arm here.
    fn private_mode(&self, screen: &crate::vt100::Screen, mode: u16) -> Option<bool> {
        use crate::vt100::{MouseProtocolEncoding, MouseProtocolMode};
        let mouse = screen.mouse_protocol_mode();
        let encoding = screen.mouse_protocol_encoding();
        Some(match mode {
            1 => screen.application_cursor(),
            // DECTCEM: set (1) while the cursor is shown.
            25 => !screen.hide_cursor(),
            9 => mouse == MouseProtocolMode::Press,
            1000 => mouse == MouseProtocolMode::PressRelease,
            1002 => mouse == MouseProtocolMode::ButtonMotion,
            1003 => mouse == MouseProtocolMode::AnyMotion,
            1005 => encoding == MouseProtocolEncoding::Utf8,
            1006 => encoding == MouseProtocolEncoding::Sgr,
            47 | 1049 => screen.alternate_screen(),
            2004 => screen.bracketed_paste(),
            _ if TRACKED_MODES.contains(&mode) => self.mode_on(mode),
            _ => return None,
        })
    }

    /// A command starts running (`133;C`): its output begins on the cursor's line. The typed command is on the line
    /// above (Enter moved the cursor down before the shell ran it), so that line is kept as the command's text.
    fn open_block(&mut self, screen: &mut crate::vt100::Screen) {
        let epoch = screen.reflow_epoch();
        retain_live(&mut self.blocks, screen.history_dropped(), epoch);
        let (row, _) = screen.cursor_position();
        let start = screen_base(screen) + row as usize;
        let cols = crate::term::screen_size(screen).1;
        let line = start.checked_sub(1).and_then(|l| span_text(screen, l, 0, cols)).map(|t| t.trim_end().to_string());
        if self.blocks.len() >= MAX_BLOCKS {
            self.blocks.pop_front();
        }
        self.blocks.push_back(CommandBlock { start, end: None, epoch, exit: None, line });
    }

    /// A command finished (`133;D`): its output ends where the cursor is, and the line the cursor is on counts when
    /// it holds output (the command printed without a final line break). A block that a reflow renumbered in the
    /// meantime is dropped instead: its lines are no longer where it says.
    fn close_block(&mut self, screen: &crate::vt100::Screen, code: Option<i32>) {
        let epoch = screen.reflow_epoch();
        let Some(block) = self.blocks.back_mut().filter(|b| b.end.is_none()) else { return };
        if block.epoch != epoch {
            self.blocks.pop_back();
            return;
        }
        let (row, col) = screen.cursor_position();
        let end = screen_base(screen) + row as usize + usize::from(col > 0);
        block.end = Some(end.max(block.start));
        block.exit = code;
    }

    fn push_link(&mut self, link: Hyperlink) {
        if link.to > link.from && !link.url.is_empty() {
            // Links of an older numbering (a reflow) can no longer be found.
            self.hyperlinks.retain(|h| h.epoch == link.epoch);
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

    /// An OSC 9;4 report: state 0 clears the bar, 1 to 4 set it. A state that is not a number from 0 to 4 changes
    /// nothing, so the bar is only ever set or cleared by a state the report names.
    fn set_progress(&mut self, state: &[u8], percent: Option<&[u8]>) {
        let Some(state) = parse_number(state) else { return };
        let percent = percent.and_then(parse_number).map(|p| p.min(100) as u8);
        let next = match state {
            0 => None,
            1 => Some(Progress { state: ProgressState::Normal, percent }),
            2 => Some(Progress { state: ProgressState::Error, percent }),
            // Busy: the report's percentage, if it sends one, is not shown.
            3 => Some(Progress { state: ProgressState::Indeterminate, percent: None }),
            4 => Some(Progress { state: ProgressState::Paused, percent }),
            _ => return,
        };
        self.store_progress(next);
    }

    /// Stores a progress report and flags the change when it differs from the stored one.
    fn store_progress(&mut self, next: Option<Progress>) {
        if next != self.progress {
            self.progress = next;
            self.progress_changed = true;
        }
    }
}

/// A decimal number in an OSC parameter; anything else is `None`. A number past `u32::MAX` saturates, so an
/// absurd percentage reads as 100 rather than as no percentage.
fn parse_number(raw: &[u8]) -> Option<u32> {
    if raw.is_empty() || !raw.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(raw.iter().fold(0u32, |n, &d| n.saturating_mul(10).saturating_add(u32::from(d - b'0'))))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parser(rows: u16, cols: u16) -> crate::vt100::Parser<Callbacks> {
        crate::vt100::Parser::new_with_callbacks(
            crate::term::vt_size(rows),
            crate::term::vt_size(cols),
            0,
            Callbacks::default(),
        )
    }

    /// The answer a query gets, taken out of the emulator the way the reader thread does.
    fn answer(p: &mut crate::vt100::Parser<Callbacks>, query: &[u8]) -> String {
        p.process(query);
        String::from_utf8(std::mem::take(&mut p.callbacks_mut().responses)).unwrap()
    }

    #[test]
    fn xtversion_names_noble_and_its_version() {
        let mut p = parser(10, 40);
        let want = format!("\x1bP>|NOBLE {}\x1b\\", env!("CARGO_PKG_VERSION"));
        assert_eq!(answer(&mut p, b"\x1b[>q"), want);
        assert_eq!(answer(&mut p, b"\x1b[>0q"), want);
        // Another parameter is not XTVERSION.
        assert_eq!(answer(&mut p, b"\x1b[>1q"), "");
    }

    #[test]
    fn decrqm_reads_the_modes_vt100_handles_from_the_screen() {
        let mut p = parser(10, 40);
        // Defaults: the cursor is shown, the other modes are reset.
        assert_eq!(answer(&mut p, b"\x1b[?25$p"), "\x1b[?25;1$y");
        assert_eq!(answer(&mut p, b"\x1b[?1$p"), "\x1b[?1;2$y");
        assert_eq!(answer(&mut p, b"\x1b[?1000$p"), "\x1b[?1000;2$y");
        assert_eq!(answer(&mut p, b"\x1b[?2004$p"), "\x1b[?2004;2$y");
        assert_eq!(answer(&mut p, b"\x1b[?1049$p"), "\x1b[?1049;2$y");
        // Each mode set (`h`) and then reset (`l`), with the state DECRQM answers in between.
        for (mode, set, reset) in [
            (1, "\x1b[?1h", "\x1b[?1l"),
            (25, "\x1b[?25h", "\x1b[?25l"),
            (1002, "\x1b[?1002h", "\x1b[?1002l"),
            (1003, "\x1b[?1003h", "\x1b[?1003l"),
            (1005, "\x1b[?1005h", "\x1b[?1005l"),
            (1006, "\x1b[?1006h", "\x1b[?1006l"),
            (2004, "\x1b[?2004h", "\x1b[?2004l"),
            (1049, "\x1b[?1049h", "\x1b[?1049l"),
            (47, "\x1b[?47h", "\x1b[?47l"),
        ] {
            let query = format!("\x1b[?{mode}$p");
            p.process(set.as_bytes());
            assert_eq!(answer(&mut p, query.as_bytes()), format!("\x1b[?{mode};1$y"), "set {mode}");
            p.process(reset.as_bytes());
            assert_eq!(answer(&mut p, query.as_bytes()), format!("\x1b[?{mode};2$y"), "reset {mode}");
        }
    }

    #[test]
    fn decrqm_mouse_modes_follow_the_one_mode_vt100_keeps() {
        let mut p = parser(10, 40);
        p.process(b"\x1b[?1000h");
        assert_eq!(answer(&mut p, b"\x1b[?1000$p"), "\x1b[?1000;1$y");
        assert_eq!(answer(&mut p, b"\x1b[?1002$p"), "\x1b[?1002;2$y");
        p.process(b"\x1b[?1002h");
        assert_eq!(answer(&mut p, b"\x1b[?1002$p"), "\x1b[?1002;1$y");
        assert_eq!(answer(&mut p, b"\x1b[?1000$p"), "\x1b[?1000;2$y", "vt100 keeps one mouse mode");
        p.process(b"\x1b[?1002l");
        assert_eq!(answer(&mut p, b"\x1b[?1002$p"), "\x1b[?1002;2$y");
    }

    #[test]
    fn decrqm_tracks_the_modes_vt100_passes_on() {
        let mut p = parser(10, 40);
        // 1047 is not handled by vt100: NOBLE records it itself, in a sequence with a mode vt100 handles too.
        assert_eq!(answer(&mut p, b"\x1b[?1047$p"), "\x1b[?1047;2$y");
        p.process(b"\x1b[?1049;1047h");
        assert_eq!(answer(&mut p, b"\x1b[?1047$p"), "\x1b[?1047;1$y");
        assert_eq!(answer(&mut p, b"\x1b[?1049$p"), "\x1b[?1049;1$y");
        p.process(b"\x1b[?1047l");
        assert_eq!(answer(&mut p, b"\x1b[?1047$p"), "\x1b[?1047;2$y");
        // A mode that is not tracked is not recorded, and an unknown mode is not recognized.
        p.process(b"\x1b[?1047;9998h");
        assert_eq!(answer(&mut p, b"\x1b[?1047$p"), "\x1b[?1047;1$y");
        assert_eq!(answer(&mut p, b"\x1b[?9998$p"), "\x1b[?9998;0$y");
        assert_eq!(answer(&mut p, b"\x1b[?9999$p"), "\x1b[?9999;0$y");
    }

    #[test]
    fn scheme_notices_mode_is_tracked_until_reset() {
        let mut p = parser(10, 40);
        assert_eq!(answer(&mut p, b"\x1b[?2031$p"), "\x1b[?2031;2$y", "supported, off");
        p.process(b"\x1b[?2031h");
        assert!(p.callbacks().mode_on(SCHEME_NOTICES));
        assert_eq!(answer(&mut p, b"\x1b[?2031$p"), "\x1b[?2031;1$y");
        p.process(b"\x1b[?2031l");
        assert!(!p.callbacks().mode_on(SCHEME_NOTICES));
        assert_eq!(answer(&mut p, b"\x1b[?2031$p"), "\x1b[?2031;2$y");
    }

    #[test]
    fn scheme_query_is_answered_from_the_shared_colors() {
        let colors = SharedColors::default();
        let mut p = crate::vt100::Parser::new_with_callbacks(
            crate::term::vt_size(10),
            crate::term::vt_size(40),
            0,
            Callbacks::with_colors(colors.clone()),
        );
        assert_eq!(answer(&mut p, b"\x1b[?996n"), "\x1b[?997;1n", "dark");
        colors.write().unwrap().bg = [0xfa, 0xfa, 0xfa];
        assert_eq!(answer(&mut p, b"\x1b[?996n"), "\x1b[?997;2n", "light");
        // A pane without colors answers nothing, and the ordinary status report is untouched.
        let mut bare = parser(10, 40);
        assert_eq!(answer(&mut bare, b"\x1b[?996n\x1b[5n"), "\x1b[0n");
    }

    #[test]
    fn focus_reports_mode_is_tracked_until_reset() {
        let mut p = parser(10, 40);
        assert_eq!(answer(&mut p, b"\x1b[?1004$p"), "\x1b[?1004;2$y");
        p.process(b"\x1b[?1004h");
        assert!(p.callbacks().mode_on(FOCUS_REPORTS));
        assert_eq!(answer(&mut p, b"\x1b[?1004$p"), "\x1b[?1004;1$y");
        p.process(b"\x1b[?1004l");
        assert!(!p.callbacks().mode_on(FOCUS_REPORTS));
        assert_eq!(answer(&mut p, b"\x1b[?1004$p"), "\x1b[?1004;2$y");
        // Set in a sequence with a mode vt100 handles.
        p.process(b"\x1b[?1049;1004h");
        assert_eq!(answer(&mut p, b"\x1b[?1004$p"), "\x1b[?1004;1$y");
        // Like xterm, the mode survives the prompt marks (a program that leaves without resetting it keeps it),
        // and the other sequences that do not touch it.
        p.process(b"\x1b]7;file:///tmp\x07\x1b]9;9;C:\\\x07\x1b]133;A\x07\x1b]0;title\x07\x1b]133;C\x07");
        assert!(p.callbacks().mode_on(FOCUS_REPORTS));
        // The alternate screen exit does not reset it either (xterm does not).
        p.process(b"\x1b[?1049l");
        assert!(p.callbacks().mode_on(FOCUS_REPORTS));
    }

    #[test]
    fn synchronized_output_is_reported_and_blocks_follow_h_and_l() {
        let mut p = parser(10, 40);
        assert_eq!(answer(&mut p, b"\x1b[?2026$p"), "\x1b[?2026;2$y", "supported, off");
        assert!(!p.callbacks().sync_open());
        p.process(b"\x1b[?2026h");
        assert!(p.callbacks().sync_open());
        assert_eq!(answer(&mut p, b"\x1b[?2026$p"), "\x1b[?2026;1$y");
        p.process(b"\x1b[?2026l");
        assert!(!p.callbacks().sync_open());
        assert_eq!(answer(&mut p, b"\x1b[?2026$p"), "\x1b[?2026;2$y");
        // Several blocks one after the other, and one set in a sequence with another mode.
        p.process(b"\x1b[?2026h a \x1b[?2026l\x1b[?2026h b \x1b[?2026l");
        assert!(!p.callbacks().sync_open());
        p.process(b"\x1b[?25;2026h");
        assert!(p.callbacks().sync_open());
        p.process(b"\x1b[?2026l");
        // A sequence split across reads still opens the block.
        p.process(b"\x1b[?20");
        assert!(!p.callbacks().sync_open());
        p.process(b"26h");
        assert!(p.callbacks().sync_open());
        p.process(b"\x1b[?2026l");
        assert!(!p.callbacks().sync_open());
    }

    #[test]
    fn synchronized_block_ends_at_the_timeout_and_a_repeated_h_does_not_extend_it() {
        use std::time::{Duration, Instant};
        let mut p = parser(10, 40);
        p.callbacks_mut().set_sync_timeout(Some(Duration::from_millis(100)));
        let start = Instant::now();
        p.process(b"\x1b[?2026h");
        let cb = p.callbacks();
        assert!(cb.sync_left_at(start).is_some(), "open right away");
        assert!(cb.sync_left_at(start + Duration::from_millis(99)).is_some());
        assert!(cb.sync_left_at(start + Duration::from_millis(100) + Duration::from_secs(1)).is_none());
        assert!(cb.sync_left_at(start + Duration::from_secs(5)).is_none());
        // A program that forgot the `l`: the block is over by itself, the mode is still set for DECRQM.
        std::thread::sleep(Duration::from_millis(130));
        assert!(!p.callbacks().sync_open());
        assert_eq!(answer(&mut p, b"\x1b[?2026$p"), "\x1b[?2026;1$y");
        // Its next `h` opens a new block; an `h` inside an open block keeps the first one's time. The limit is long
        // here so a sleep that overshoots on a loaded machine can never end the block before the second `h`.
        p.callbacks_mut().set_sync_timeout(Some(Duration::from_secs(10)));
        p.process(b"[?2026h");
        assert!(p.callbacks().sync_open());
        let opened = Instant::now();
        std::thread::sleep(Duration::from_millis(60));
        p.process(b"[?2026h");
        // The block began before `opened`, so at least 60 ms are used up; a restart would leave about 10 s.
        let left = p.callbacks().sync_left_at(opened + Duration::from_millis(60)).expect("still open");
        assert!(left <= Duration::from_millis(9_940), "the second h did not start the time over: {left:?}");
        // Without an override the limit is `SYNC_TIMEOUT`.
        p.callbacks_mut().set_sync_timeout(None);
        p.process(b"\x1b[?2026l\x1b[?2026h");
        let left = p.callbacks().sync_left().expect("open");
        assert!(left > Duration::from_millis(100) && left <= SYNC_TIMEOUT, "{left:?}");
    }

    #[test]
    fn full_reset_clears_the_tracked_modes_and_the_block() {
        let mut p = parser(10, 40);
        p.process(b"\x1b[?2026h\x1b[?1004h");
        p.callbacks_mut().reset_modes();
        assert!(!p.callbacks().sync_open());
        assert!(!p.callbacks().mode_on(FOCUS_REPORTS));
        assert_eq!(answer(&mut p, b"\x1b[?2026$p"), "\x1b[?2026;2$y");
    }

    #[test]
    fn decrqm_ansi_modes_are_not_recognized() {
        let mut p = parser(10, 40);
        assert_eq!(answer(&mut p, b"\x1b[4$p"), "\x1b[4;0$y");
        assert_eq!(answer(&mut p, b"\x1b[20$p"), "\x1b[20;0$y");
    }

    #[test]
    fn existing_device_attribute_answers_are_unchanged() {
        let mut p = parser(10, 40);
        assert_eq!(answer(&mut p, b"\x1b[5n\x1b[c\x1b[>c"), "\x1b[0n\x1b[?1;2c\x1b[>0;10;1c");
    }

    fn parser_with_history(rows: u16, cols: u16, history: usize) -> crate::vt100::Parser<Callbacks> {
        crate::vt100::Parser::new_with_callbacks(
            crate::term::vt_size(rows),
            crate::term::vt_size(cols),
            history,
            Callbacks::default(),
        )
    }

    #[test]
    fn command_marks_open_and_close_a_block_with_its_lines_code_and_typed_line() {
        let mut p = parser_with_history(10, 40, 100);
        p.process(b"~/app $ cargo test\r\n\x1b]133;C\x07");
        p.process(b"running 1 test\r\nFAILED\r\n\x1b]133;D;101\x07~/app $ ");
        let blocks: Vec<CommandBlock> = p.callbacks().blocks.iter().cloned().collect();
        assert_eq!(
            blocks,
            vec![CommandBlock {
                start: 1,
                end: Some(3),
                epoch: p.screen().reflow_epoch(),
                exit: Some(101),
                line: Some("~/app $ cargo test".into()),
            }]
        );
    }

    #[test]
    fn output_without_a_final_line_break_counts_its_line_and_a_bare_d_keeps_no_code() {
        let mut p = parser_with_history(10, 40, 100);
        p.process(b"$ printf x\r\n\x1b]133;C\x07x\x1b]133;D\x07");
        let b = p.callbacks().blocks.back().unwrap().clone();
        assert_eq!((b.start, b.end, b.exit), (1, Some(2), None));
        // A second D after the block closed changes nothing, and a D without a block opens none.
        p.process(b"\x1b]133;D;7\x07");
        assert_eq!(p.callbacks().blocks.len(), 1);
        assert_eq!(p.callbacks().blocks.back().unwrap().exit, None);
    }

    #[test]
    fn a_command_still_running_has_no_end_yet() {
        let mut p = parser_with_history(10, 40, 100);
        p.process(b"$ sleep 9\r\n\x1b]133;C\x07waiting\r\n");
        let b = p.callbacks().blocks.back().unwrap().clone();
        assert_eq!((b.start, b.end), (1, None));
    }

    #[test]
    fn blocks_are_pruned_when_their_lines_scroll_out_of_history() {
        let mut p = parser_with_history(4, 40, 3);
        p.process(b"$ a\r\n\x1b]133;C\x07one\r\n\x1b]133;D;0\x07");
        assert_eq!(p.callbacks().blocks.len(), 1);
        // Enough output to push the block's lines out of the 3-line history.
        p.process(&b"x\r\n".repeat(8));
        assert!(p.screen().history_dropped() > 1);
        p.process(b"$ b\r\n\x1b]133;C\x07");
        let starts: Vec<usize> = p.callbacks().blocks.iter().map(|b| b.start).collect();
        assert_eq!(starts.len(), 1, "the first block went with its lines: {starts:?}");
        assert!(starts[0] >= p.screen().history_dropped());
    }

    #[test]
    fn blocks_do_not_survive_a_reflow() {
        let mut p = parser_with_history(4, 40, 100);
        p.process(b"$ a\r\n\x1b]133;C\x07one\r\n\x1b]133;D;0\x07");
        let before = p.screen().reflow_epoch();
        p.screen_mut().set_size(crate::term::vt_size(4), crate::term::vt_size(20));
        assert_ne!(p.screen().reflow_epoch(), before, "a column change renumbers the lines");
        // The next command prunes the old block; the numbers of the new one are the new epoch's.
        p.process(b"$ b\r\n\x1b]133;C\x07");
        let blocks: Vec<CommandBlock> = p.callbacks().blocks.iter().cloned().collect();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].epoch, p.screen().reflow_epoch());
    }

    #[test]
    fn a_reflow_while_a_command_runs_drops_its_block_at_the_end() {
        let mut p = parser_with_history(4, 40, 100);
        p.process(b"$ long\r\n\x1b]133;C\x07out\r\n");
        p.screen_mut().set_size(crate::term::vt_size(4), crate::term::vt_size(20));
        p.process(b"\x1b]133;D;0\x07");
        assert!(p.callbacks().blocks.is_empty(), "its start is in the old numbering");
    }

    fn report(state: ProgressState, percent: Option<u8>) -> Option<Progress> {
        Some(Progress { state, percent })
    }

    /// OSC 9;4: each state sets the bar, state 0 clears it. A percentage is clamped to 100 and one that is not a
    /// number is dropped; an unknown state, a state that is not a number and a missing state change nothing.
    #[test]
    fn progress_reports_set_and_clear_the_bar() {
        let mut p = parser(10, 40);
        assert_eq!(p.callbacks().progress, None);
        p.process(b"\x1b]9;4;1;42\x07");
        assert_eq!(p.callbacks().progress, report(ProgressState::Normal, Some(42)));
        // Busy without a percentage, even when the report sends one.
        p.process(b"\x1b]9;4;3;0\x1b\\");
        assert_eq!(p.callbacks().progress, report(ProgressState::Indeterminate, None));
        p.process(b"\x1b]9;4;2;7\x07");
        assert_eq!(p.callbacks().progress, report(ProgressState::Error, Some(7)));
        p.process(b"\x1b]9;4;4\x07");
        assert_eq!(p.callbacks().progress, report(ProgressState::Paused, None));
        p.process(b"\x1b]9;4;1\x07");
        assert_eq!(p.callbacks().progress, report(ProgressState::Normal, None), "no percentage");
        p.process(b"\x1b]9;4;1;150\x07");
        assert_eq!(p.callbacks().progress, report(ProgressState::Normal, Some(100)), "clamped");
        p.process(b"\x1b]9;4;1;99999999999999999999999\x07");
        assert_eq!(p.callbacks().progress, report(ProgressState::Normal, Some(100)), "saturates");
        p.process(b"\x1b]9;4;1;x\x07");
        assert_eq!(p.callbacks().progress, report(ProgressState::Normal, None), "not a number");
        p.process(b"\x1b]9;4;1;30\x07");
        for ignored in [&b"\x1b]9;4;5;90\x07"[..], b"\x1b]9;4;x;90\x07", b"\x1b]9;4;\x07", b"\x1b]9;4\x07"] {
            p.process(ignored);
            assert_eq!(p.callbacks().progress, report(ProgressState::Normal, Some(30)), "{ignored:?}");
        }
        p.process(b"\x1b]9;4;0\x07");
        assert_eq!(p.callbacks().progress, None);
        // A clear with a percentage clears too.
        p.process(b"\x1b]9;4;1;5\x07\x1b]9;4;0;50\x07");
        assert_eq!(p.callbacks().progress, None);
    }

    /// A progress report is no notification and no prompt mark, and the busy states are the ones an agent or a tab
    /// shows as working.
    #[test]
    fn progress_is_neither_a_notice_nor_a_prompt_and_busy_states_spin() {
        let mut p = parser(10, 40);
        p.process(b"\x1b]9;4;1;50\x07\x1b]9;4;3;0\x07");
        assert_eq!(p.callbacks().notice, None);
        assert!(!p.callbacks().prompt);
        let busy = |state, percent| Progress { state, percent }.busy();
        assert!(busy(ProgressState::Normal, Some(5)) && busy(ProgressState::Normal, None));
        assert!(busy(ProgressState::Indeterminate, None));
        assert!(!busy(ProgressState::Error, None) && !busy(ProgressState::Paused, Some(3)));
        // A spinner stands for a busy report with no percentage only.
        let spins = |state, percent| Progress { state, percent }.spins();
        assert!(!spins(ProgressState::Normal, Some(5)));
        assert!(spins(ProgressState::Normal, None) && spins(ProgressState::Indeterminate, None));
        assert!(!spins(ProgressState::Paused, None));
    }

    /// Every prompt mark (the shell is back) clears the bar, the way it ends a title the program set.
    #[test]
    fn a_prompt_mark_clears_the_progress_report() {
        for mark in [&b"\x1b]7;file://h/tmp\x07"[..], b"\x1b]9;9;C:\\x\x1b\\", b"\x1b]133;A\x07", b"\x1b]133;D;0\x07"] {
            let mut p = parser(10, 40);
            p.process(b"\x1b]9;4;3;0\x07");
            assert!(p.callbacks().progress.is_some());
            p.process(mark);
            assert_eq!(p.callbacks().progress, None, "{mark:?}");
        }
    }

    /// `progress_changed` is set when the stored report changes and only then; the app takes it once.
    #[test]
    fn a_progress_change_is_flagged_and_a_repeat_is_not() {
        let mut p = parser(10, 40);
        p.process(b"\x1b]9;4;3;0\x07");
        assert!(std::mem::take(&mut p.callbacks_mut().progress_changed));
        assert!(!p.callbacks().progress_changed, "taken");
        p.process(b"\x1b]9;4;3;0\x07");
        assert!(!p.callbacks().progress_changed, "the same report again");
        p.process(b"\x1b]9;4;1;10\x07");
        assert!(p.callbacks().progress_changed);
        p.callbacks_mut().progress_changed = false;
        p.process(b"\x1b]9;4;0\x07");
        assert!(p.callbacks().progress_changed, "cleared");
        p.callbacks_mut().progress_changed = false;
        p.process(b"\x1b]9;4;0\x07");
        assert!(!p.callbacks().progress_changed, "clearing nothing changes nothing");
    }
}
