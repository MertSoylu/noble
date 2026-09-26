//! A single terminal panel: a real PTY (ConPTY/Unix pty) + the vt100 emulator.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Context, Result};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::config::TerminalCfg;
use crate::event::AppEvent;
use crate::term::integration;
use crate::term::layout::PaneId;
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
    /// Desktop notification sent by the app (OSC 9 text, OSC 777;notify).
    pub notice: Option<String>,
    /// OSC 8 links (newest last, capped in number).
    pub hyperlinks: std::collections::VecDeque<Hyperlink>,
    /// An OSC 8 link opened but not yet closed: (absolute line, column, address).
    open_link: Option<(usize, u16, String)>,
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

/// Text marked with OSC 8: absolute line (0 = oldest scrollback line) and column span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hyperlink {
    pub line: usize,
    pub from: u16,
    pub to: u16,
    pub url: String,
}

/// Maximum number of OSC 8 links kept.
const MAX_HYPERLINKS: usize = 1000;

/// Scrollback length (without disturbing the visible scroll position).
fn history_len(screen: &mut vt100::Screen) -> usize {
    let current = screen.scrollback();
    screen.set_scrollback(usize::MAX);
    let len = screen.scrollback();
    screen.set_scrollback(current);
    len
}

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
                let path = parse_cwd_url(&join(rest));
                self.cwd = Some(if cfg!(windows) { msys_to_windows(&path) } else { path });
                self.shell_prompt();
            }
            // OSC 9;9: Windows Terminal's cwd report.
            [b"9", b"9", rest @ ..] if !rest.is_empty() => {
                self.cwd = Some(join(rest).trim_matches('"').to_string());
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
                    let cols = screen.size().1;
                    if line == l0 {
                        self.push_link(Hyperlink { line, from: c0, to: col, url: open });
                    } else {
                        // Line-spanning link: end of the first line and start of the last.
                        self.push_link(Hyperlink { line: l0, from: c0, to: cols, url: open.clone() });
                        self.push_link(Hyperlink { line, from: 0, to: col, url: open });
                    }
                }
                if !url.is_empty() {
                    self.open_link = Some((line, col, url));
                }
            }
            // OSC 133: FinalTerm/VS Code prompt marks. A = prompt started, D = command finished.
            [b"133", kind, ..] if matches!(kind.first(), Some(b'A' | b'D')) => self.shell_prompt(),
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

/// Pane title: the process name from the shell's OSC title. `C:\WINDOWS\system32\cmd.exe`
/// → `cmd`, `vim - notes.md` → `vim`. `fallback` when there is no title.
pub fn process_label(title: &str, fallback: &str) -> String {
    let source = if title.trim().is_empty() { fallback.trim() } else { title.trim() };
    if source.is_empty() {
        return "shell".into();
    }
    let head = source.split(" - ").next().unwrap_or(source).split(" | ").next().unwrap_or(source).trim();
    // If it looks like a path, take the last segment.
    let leaf = if head.contains('\\') || head.contains('/') {
        head.replace('\\', "/").trim_end_matches('/').rsplit('/').next().unwrap_or(head).to_string()
    } else {
        head.to_string()
    };
    // Compared by bytes: `to_lowercase` can change the length ("İ", "K") and the cut would miss.
    let bare = [".exe", ".cmd", ".bat", ".com"]
        .iter()
        .find_map(|ext| {
            let cut = leaf.len().checked_sub(ext.len())?;
            leaf.get(cut..).filter(|tail| tail.eq_ignore_ascii_case(ext)).map(|_| leaf[..cut].to_string())
        })
        .unwrap_or(leaf);
    let bare = bare.trim();
    let label = if bare.is_empty() { "shell" } else { bare };
    util::truncate(label, 28)
}

#[derive(Clone, Debug)]
pub struct ShellSpec {
    pub program: String,
    pub args: Vec<String>,
    /// Directory of the bash/zsh/fish integration scripts (`term::integration`);
    /// `None` starts the shell exactly as configured.
    pub integration: Option<PathBuf>,
}

impl ShellSpec {
    pub fn new(program: impl Into<String>, args: Vec<String>) -> ShellSpec {
        ShellSpec { program: program.into(), args, integration: None }
    }

    pub fn label(&self) -> String {
        process_label("", &self.program)
    }

    fn kind(&self) -> ShellKind {
        match self.label().to_lowercase().as_str() {
            "pwsh" | "powershell" => ShellKind::PowerShell,
            "cmd" => ShellKind::Cmd,
            "bash" => ShellKind::Bash,
            "zsh" => ShellKind::Zsh,
            "fish" => ShellKind::Fish,
            _ => ShellKind::Posix,
        }
    }

    /// The integration directory, unless this shell has none or the user's
    /// arguments skip the startup files (or run a command instead).
    fn integration_dir(&self) -> Option<&Path> {
        let dir = self.integration.as_deref()?;
        let skip: &[&str] = match self.kind() {
            ShellKind::Bash => &["--norc", "--rcfile", "--init-file", "-c"],
            ShellKind::Zsh => &["-f", "--no-rcs", "-c"],
            ShellKind::Fish => &["-N", "--no-config", "-c", "--command"],
            _ => return None,
        };
        (!self.args.iter().any(|a| skip.contains(&a.as_str()))).then_some(dir)
    }

    /// Writes the integration scripts. If that fails the shell starts without them
    /// (a missing `--rcfile` would also skip the user's `~/.bashrc`).
    pub fn prepared(&self) -> ShellSpec {
        let mut spec = self.clone();
        let failed = spec.integration_dir().is_some_and(|dir| integration::install(dir).is_err());
        if failed {
            spec.integration = None;
        }
        spec
    }

    /// Arguments of an interactive shell: the user's own plus the integration.
    fn interactive_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        match (self.kind(), self.integration_dir()) {
            (ShellKind::Bash, Some(dir)) => {
                // bash wants long options before single-letter ones.
                args.extend(["--rcfile".into(), integration::bash_rc(dir).display().to_string()]);
                args.extend(self.args.iter().cloned());
            }
            (ShellKind::Fish, Some(dir)) => {
                args.extend(self.args.iter().cloned());
                let script = integration::fish_script(dir).display().to_string();
                args.extend(["--init-command".into(), format!("source {}", self.quote(&script))]);
            }
            _ => args.extend(self.args.iter().cloned()),
        }
        args
    }

    /// Quotes one word for this Unix shell (fish escapes inside single quotes differently).
    fn quote(&self, word: &str) -> String {
        match self.kind() {
            ShellKind::Fish => format!("'{}'", word.replace('\\', r"\\").replace('\'', r"\'")),
            _ => format!("'{}'", word.replace('\'', r"'\''")),
        }
    }

    /// Arguments that start the shell so it runs `command` first and then stays
    /// interactive. A prompt hook reporting the working directory is added as well
    /// (so the split and the session record know the real directory).
    pub fn args_with_command(&self, command: Option<&str>) -> Vec<String> {
        let mut args = self.args.clone();
        let cmd = command.filter(|c| !c.trim().is_empty());
        match self.kind() {
            ShellKind::PowerShell => {
                let script = match cmd {
                    Some(c) => format!("{PWSH_CWD_HOOK}; {c}"),
                    None => PWSH_CWD_HOOK.to_string(),
                };
                args.extend(["-NoExit".into(), "-Command".into(), script]);
            }
            ShellKind::Cmd => {
                // The command is launched via the `NOBLE_LAUNCH` variable (see `extra_env`):
                // cmd.exe does not recognize the `\"` escape, so quoted paths would break.
                if cmd.is_some() {
                    args.extend(["/K".into(), "%NOBLE_LAUNCH%".into()]);
                }
            }
            ShellKind::Bash | ShellKind::Zsh | ShellKind::Fish | ShellKind::Posix => match cmd {
                None => args = self.interactive_args(),
                Some(c) => {
                    // After the command the same shell takes over, with the integration.
                    let again: Vec<String> = std::iter::once(self.program.as_str())
                        .chain(self.interactive_args().iter().map(String::as_str))
                        .map(|w| self.quote(w))
                        .collect();
                    args.extend(["-c".into(), format!("{c}; exec {}", again.join(" "))]);
                }
            },
        }
        args
    }

    /// Prepares a launcher command for this shell: on Windows the program is
    /// resolved on PATH as `.exe`/`.cmd` and invoked with its full path, so the
    /// `.ps1` shims that trip PowerShell's script policy never come into play.
    pub fn invocation(&self, command: &str) -> String {
        let command = command.trim();
        if !cfg!(windows) {
            return command.to_string();
        }
        let (program, rest) = match command.split_once(char::is_whitespace) {
            Some((p, r)) => (p, r.trim_start()),
            None => (command, ""),
        };
        let resolved = util::which(program).filter(|p| {
            let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            matches!(ext.as_str(), "exe" | "cmd" | "bat" | "com")
        });
        let Some(path) = resolved else { return command.to_string() };
        let tail = if rest.is_empty() { String::new() } else { format!(" {rest}") };
        match self.kind() {
            ShellKind::PowerShell => format!("& '{}'{tail}", path.display().to_string().replace('\'', "''")),
            ShellKind::Cmd => format!("\"{}\"{tail}", path.display()),
            ShellKind::Bash | ShellKind::Zsh | ShellKind::Fish | ShellKind::Posix => command.to_string(),
        }
    }

    /// A command line that runs a program (given by full path) with its arguments
    /// (paths with spaces are quoted per the shell).
    pub fn invocation_of(&self, program: &Path, args: &str) -> String {
        let path = program.display().to_string();
        match self.kind() {
            ShellKind::PowerShell => format!("& '{}' {args}", path.replace('\'', "''")),
            ShellKind::Cmd => format!("\"{path}\" {args}"),
            ShellKind::Bash | ShellKind::Zsh | ShellKind::Fish | ShellKind::Posix => {
                format!("{} {args}", self.quote(&path))
            }
        }
    }

    /// Shell-specific environment variables: for cmd a directory-reporting PROMPT
    /// and the command to run (`NOBLE_LAUNCH`, so the quotes stay untouched); for
    /// zsh the integration's `ZDOTDIR` and the user's own one.
    pub fn extra_env(&self, command: Option<&str>) -> Vec<(String, String)> {
        let mut env = Vec::new();
        match self.kind() {
            ShellKind::Cmd => {
                if std::env::var("PROMPT").is_err() {
                    env.push(("PROMPT".into(), r"$E]9;9;$P$E\$P$G".into()));
                }
                if let Some(c) = command.filter(|c| !c.trim().is_empty()) {
                    env.push(("NOBLE_LAUNCH".into(), c.to_string()));
                }
            }
            ShellKind::Zsh => {
                if let Some(dir) = self.integration_dir() {
                    let ours = integration::zsh_dir(dir);
                    // Inside a NOBLE pane ZDOTDIR may already be ours; the user's is kept aside then.
                    let user = std::env::var_os("ZDOTDIR")
                        .filter(|d| Path::new(d) != ours)
                        .or_else(|| std::env::var_os("NOBLE_USER_ZDOTDIR"))
                        .filter(|d| !d.is_empty());
                    if let Some(user) = user {
                        env.push(("NOBLE_USER_ZDOTDIR".into(), user.to_string_lossy().into_owned()));
                    }
                    env.push(("ZDOTDIR".into(), ours.display().to_string()));
                }
            }
            _ => {}
        }
        env
    }
}

/// Reports the working directory with OSC 9;9 on every prompt, wrapping the current
/// prompt (oh-my-posh included). It contains no double quotes so command line
/// quoting never breaks.
pub const PWSH_CWD_HOOK: &str = r"$global:__nobleP=$function:prompt; function global:prompt { [Console]::Write([char]27+']9;9;'+$executionContext.SessionState.Path.CurrentLocation.ProviderPath+[char]27+'\'); & $global:__nobleP }";

enum ShellKind {
    PowerShell,
    Cmd,
    Bash,
    Zsh,
    Fish,
    /// Another Unix shell (sh, dash, nu …): started as configured, without integration.
    Posix,
}

/// Decides which shell to use; bash/zsh/fish get the integration scripts from `<data>/shell`.
pub fn resolve_shell(cfg: &TerminalCfg, data: &Path) -> ShellSpec {
    let program = if !cfg.shell.trim().is_empty() {
        cfg.shell.trim().to_string()
    } else if cfg!(windows) {
        util::which("pwsh")
            .or_else(|| util::which("powershell"))
            .map(|p| p.display().to_string())
            .or_else(|| std::env::var("COMSPEC").ok())
            .unwrap_or_else(|| "cmd.exe".into())
    } else {
        std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/sh".into())
    };
    let mut spec = ShellSpec::new(program, cfg.shell_args.clone());
    spec.integration = Some(data.join("shell"));
    if spec.args.is_empty() && matches!(spec.kind(), ShellKind::PowerShell) {
        spec.args.push("-NoLogo".into());
    }
    spec
}

/// Mouse selection; (row, col) coordinates inside the pane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Selection {
    pub anchor: (u16, u16),
    pub head: (u16, u16),
}

impl Selection {
    pub fn ordered(&self) -> ((u16, u16), (u16, u16)) {
        if self.anchor <= self.head { (self.anchor, self.head) } else { (self.head, self.anchor) }
    }

    pub fn contains(&self, row: u16, col: u16) -> bool {
        let (s, e) = self.ordered();
        (row, col) >= s && (row, col) <= e
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

pub struct Pane {
    pub id: PaneId,
    pub parser: Arc<Mutex<vt100::Parser<Callbacks>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    pub dirty: Arc<AtomicBool>,
    /// (row, col)
    pub size: (u16, u16),
    pub start_cwd: PathBuf,
    pub shell_label: String,
    pub command: Option<String>,
    pub selection: Option<Selection>,
    pub pid: Option<u32>,
    /// When the user last pressed Enter: to time the command
    /// (reset when the prompt comes back).
    pub command_started: Option<std::time::Instant>,
    /// Key lock: NOBLE's direct shortcuts go to the app in this pane instead (prefix still works).
    pub passthrough: bool,
    /// A prompt signal arrived at least once: the shell integration works, so `command_started` is reliable.
    pub prompted: bool,
}

pub struct SpawnSpec<'a> {
    pub cwd: &'a Path,
    pub command: Option<&'a str>,
    pub shell: &'a ShellSpec,
    pub rows: u16,
    pub cols: u16,
    pub scrollback: usize,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Pane {
    pub fn spawn(id: PaneId, spec: SpawnSpec<'_>, tx: Sender<AppEvent>) -> Result<Pane> {
        let rows = spec.rows.max(2);
        let cols = spec.cols.max(4);
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .context("could not open a pseudo terminal")?;
        let shell = spec.shell.prepared();
        let mut cmd = CommandBuilder::new(&shell.program);
        cmd.args(shell.args_with_command(spec.command));
        let cwd = if spec.cwd.is_dir() {
            spec.cwd.to_path_buf()
        } else {
            dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
        };
        cmd.cwd(&cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "NOBLE");
        cmd.env("NOBLE_PANE", id.to_string());
        // So the Claude Code hooks write the state to the right NOBLE instance.
        cmd.env("NOBLE_INSTANCE", std::process::id().to_string());
        for (k, v) in shell.extra_env(spec.command) {
            cmd.env(k, v);
        }
        let mut child =
            pair.slave.spawn_command(cmd).with_context(|| format!("could not start {}", spec.shell.program))?;
        drop(pair.slave);
        let pid = child.process_id();
        let killer = child.clone_killer();
        let mut reader = pair.master.try_clone_reader().context("pty reader")?;
        let writer: Arc<Mutex<Box<dyn Write + Send>>> =
            Arc::new(Mutex::new(pair.master.take_writer().context("pty writer")?));
        let parser =
            Arc::new(Mutex::new(vt100::Parser::new_with_callbacks(rows, cols, spec.scrollback, Callbacks::default())));
        let dirty = Arc::new(AtomicBool::new(false));

        {
            let parser = parser.clone();
            let writer = writer.clone();
            let dirty = dirty.clone();
            let tx = tx.clone();
            std::thread::Builder::new().name(format!("pty-read-{id}")).spawn(move || {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let responses = {
                                let mut p = lock(&parser);
                                // An emulator error must not kill the whole pane: skip this chunk.
                                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.process(&buf[..n])));
                                std::mem::take(&mut p.callbacks_mut().responses)
                            };
                            if !responses.is_empty() {
                                let mut w = lock(&writer);
                                let _ = w.write_all(&responses);
                                let _ = w.flush();
                            }
                            if !dirty.swap(true, Ordering::AcqRel) && tx.send(AppEvent::PtyOutput).is_err() {
                                break;
                            }
                        }
                    }
                }
            })?;
        }
        {
            let tx = tx.clone();
            std::thread::Builder::new().name(format!("pty-wait-{id}")).spawn(move || {
                let _ = child.wait();
                let _ = tx.send(AppEvent::PtyExit(id));
            })?;
        }

        Ok(Pane {
            id,
            parser,
            writer,
            master: pair.master,
            killer,
            dirty,
            size: (rows, cols),
            start_cwd: cwd,
            shell_label: spec.shell.label(),
            command: spec.command.map(str::to_string),
            selection: None,
            pid,
            command_started: None,
            passthrough: false,
            prompted: false,
        })
    }

    pub fn write(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let mut w = lock(&self.writer);
        let _ = w.write_all(bytes);
        let _ = w.flush();
    }

    /// Pastes text; wraps it in bracketed paste when the app wants that.
    pub fn paste(&self, text: &str) {
        let bracketed = lock(&self.parser).screen().bracketed_paste();
        let body = text.replace("\r\n", "\r").replace('\n', "\r");
        if bracketed {
            self.write(format!("\x1b[200~{body}\x1b[201~").as_bytes());
        } else {
            self.write(body.as_bytes());
        }
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let (rows, cols) = (rows.max(2), cols.max(4));
        if self.size == (rows, cols) {
            return;
        }
        self.size = (rows, cols);
        let _ = self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
        lock(&self.parser).screen_mut().set_size(rows, cols);
        self.selection = None;
    }

    pub fn parser(&self) -> MutexGuard<'_, vt100::Parser<Callbacks>> {
        lock(&self.parser)
    }

    /// Scrolls the scrollback; positive = up (older).
    pub fn scroll(&self, delta: i32) {
        let mut p = lock(&self.parser);
        let cur = p.screen().scrollback() as i32;
        p.screen_mut().set_scrollback((cur + delta).max(0) as usize);
    }

    pub fn scroll_reset(&self) {
        let mut p = lock(&self.parser);
        if p.screen().scrollback() != 0 {
            p.screen_mut().set_scrollback(0);
        }
    }

    /// The launcher command the pane was opened with is still running: no prompt has come
    /// back since. Without shell integration (no prompt signal) this stays true.
    pub fn launch_running(&self) -> bool {
        self.command.is_some() && !self.prompted
    }

    pub fn scroll_offset(&self) -> usize {
        lock(&self.parser).screen().scrollback()
    }

    pub fn label(&self) -> String {
        let p = lock(&self.parser);
        process_label(&p.callbacks().title, &self.shell_label)
    }

    pub fn cwd(&self) -> PathBuf {
        let p = lock(&self.parser);
        p.callbacks().cwd.as_ref().map(PathBuf::from).filter(|p| p.is_dir()).unwrap_or_else(|| self.start_cwd.clone())
    }

    /// Clipboard content the app requested via OSC 52 (once).
    pub fn take_clipboard(&self) -> Option<String> {
        lock(&self.parser).callbacks_mut().clipboard.take()
    }

    pub fn take_bell(&self) -> bool {
        std::mem::take(&mut lock(&self.parser).callbacks_mut().bell)
    }

    pub fn take_prompt(&self) -> bool {
        std::mem::take(&mut lock(&self.parser).callbacks_mut().prompt)
    }

    pub fn take_notice(&self) -> Option<String> {
        lock(&self.parser).callbacks_mut().notice.take()
    }

    pub fn selected_text(&self) -> Option<String> {
        let sel = self.selection?;
        if sel.is_empty() {
            return None;
        }
        let ((r0, c0), (r1, c1)) = sel.ordered();
        let p = lock(&self.parser);
        // The pane can be drawn larger than its screen for a moment (the PTY is resized only when a
        // divider drag ends); vt100 panics on columns past the screen, so the selection is kept inside.
        let (rows, cols) = p.screen().size();
        let (r0, r1) = (r0.min(rows.saturating_sub(1)), r1.min(rows.saturating_sub(1)));
        let (c0, c1) = (c0.min(cols), c1.saturating_add(1).min(cols));
        let text = p.screen().contents_between(r0, c0, r1, c1);
        let text: Vec<&str> = text.lines().map(|l| l.trim_end()).collect();
        let joined = text.join("\n");
        (!joined.trim().is_empty()).then_some(joined)
    }

    pub fn kill(&mut self) {
        let _ = self.killer.kill();
    }

    /// All lines including the scrollback (0 = oldest) and the scrollback length.
    /// The visible scroll position is preserved.
    pub fn all_lines(&self) -> (Vec<String>, usize) {
        let mut p = lock(&self.parser);
        let screen = p.screen_mut();
        let original = screen.scrollback();
        screen.set_scrollback(usize::MAX);
        let history = screen.scrollback();
        let (rows, cols) = screen.size();
        let total = history + rows as usize;
        let mut lines: Vec<String> = Vec::with_capacity(total);
        let mut top = 0usize;
        while top < total {
            let offset = history.saturating_sub(top);
            screen.set_scrollback(offset);
            let start = history - offset;
            for (i, row) in screen.rows(0, cols).enumerate() {
                if start + i == lines.len() {
                    lines.push(row);
                }
            }
            top = start + rows as usize;
            if offset == 0 {
                break;
            }
        }
        screen.set_scrollback(original);
        (lines, history)
    }

    /// Scrolls so the absolute line (0 = oldest) lands in the middle of the screen.
    pub fn scroll_to_line(&self, line: usize, history: usize) {
        let rows = self.size.0 as usize;
        let top = line.saturating_sub(rows / 2);
        let offset = history.saturating_sub(top);
        lock(&self.parser).screen_mut().set_scrollback(offset);
    }

    /// The OSC 8 link at on-screen (row, col): address and column span.
    pub fn hyperlink_at(&self, row: u16, col: u16) -> Option<(String, u16, u16)> {
        let mut p = lock(&self.parser);
        let offset = p.screen().scrollback();
        let line = history_len(p.screen_mut()).saturating_sub(offset) + row as usize;
        p.callbacks()
            .hyperlinks
            .iter()
            .rev()
            .find(|h| h.line == line && col >= h.from && col < h.to)
            .map(|h| (h.url.clone(), h.from, h.to))
    }

    /// Visible line text (for link detection).
    /// What is still running in the pane (its label), if closing it would stop something:
    /// a full-screen app, a command started at the prompt, or a launcher command before its first prompt.
    pub fn busy(&self) -> Option<String> {
        let alt = lock(&self.parser).screen().alternate_screen();
        let command = if self.prompted { self.command_started.is_some() } else { self.command.is_some() };
        // The shell rarely names the command it runs (no title): say "a command" rather than the shell.
        let label = self.label();
        (alt || command).then(|| if label == self.shell_label { "a command".into() } else { label })
    }

    pub fn visible_row(&self, row: u16) -> Option<String> {
        let p = lock(&self.parser);
        let cols = p.screen().size().1;
        p.screen().rows(0, cols).nth(row as usize)
    }
}

/// A search match: absolute line (0 = oldest scrollback line) and column span.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub line: usize,
    pub col: u16,
    pub width: u16,
}

/// Case-insensitive plain text search; columns are computed with display width.
pub fn find_matches(lines: &[String], query: &str) -> Vec<Match> {
    let fold = |c: char| c.to_lowercase().next().unwrap_or(c);
    let q: Vec<char> = query.chars().map(fold).collect();
    if q.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (line_no, line) in lines.iter().enumerate() {
        let chars: Vec<(char, u16)> =
            line.chars().map(|c| (fold(c), unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) as u16)).collect();
        if chars.len() < q.len() {
            continue;
        }
        let mut col = 0u16;
        let mut i = 0;
        while i + q.len() <= chars.len() {
            if chars[i..i + q.len()].iter().zip(&q).all(|((a, _), b)| a == b) {
                let width = chars[i..i + q.len()].iter().map(|(_, w)| *w).sum();
                out.push(Match { line: line_no, col, width });
                col += width;
                i += q.len();
            } else {
                col += chars[i].1;
                i += 1;
            }
        }
    }
    out
}

impl Drop for Pane {
    fn drop(&mut self) {
        let _ = self.killer.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(process_label("C:\\WINDOWS\\system32\\cmd.exe", "x"), "cmd");
        assert_eq!(process_label("", "C:\\Program Files\\PowerShell\\7\\pwsh.exe"), "pwsh");
        assert_eq!(process_label("vim - notes.md", "bash"), "vim");
        assert_eq!(process_label("", ""), "shell");
        assert_eq!(process_label("/bin/zsh", "x"), "zsh");
        assert_eq!(process_label("✳ Claude Code", "pwsh"), "✳ Claude Code");
        assert_eq!(process_label("C:\\Tools\\NODE.EXE", "x"), "NODE");
        // Titles whose lowercase form has a different byte length must not panic.
        assert_eq!(process_label("\u{212A}x.exe", "x"), "\u{212A}x");
        assert_eq!(process_label("İİİİİ.exe", "x"), "İİİİİ");
    }

    #[test]
    fn cwd_urls() {
        assert_eq!(parse_cwd_url("file://host/home/me/my%20dir"), "/home/me/my dir");
        assert_eq!(parse_cwd_url("file://DESKTOP/C:/Users/Mert"), "C:/Users/Mert");
        assert_eq!(parse_cwd_url("C:\\x"), "C:\\x");
        // A '%' before a multi-byte character is kept as it is (used to panic).
        assert_eq!(parse_cwd_url("file://h/tmp/%aé"), "/tmp/%aé");
        assert_eq!(parse_cwd_url("file://h/tmp/%+a"), "/tmp/%+a");
        assert_eq!(msys_to_windows("/c/Users/me"), "C:/Users/me");
        assert_eq!(msys_to_windows("/d"), "D:/");
        assert_eq!(msys_to_windows("/home/me"), "/home/me");
        assert_eq!(msys_to_windows("/tmp"), "/tmp");
    }

    #[test]
    fn shell_command_args() {
        let pwsh = ShellSpec::new("C:\\pwsh.exe", vec!["-NoLogo".into()]);
        let with = pwsh.args_with_command(Some("claude"));
        assert_eq!(&with[..3], ["-NoLogo", "-NoExit", "-Command"]);
        assert!(with[3].starts_with(PWSH_CWD_HOOK) && with[3].ends_with("; claude"));
        assert!(!PWSH_CWD_HOOK.contains('"'));
        assert_eq!(pwsh.args_with_command(None)[3], PWSH_CWD_HOOK);
        assert!(pwsh.extra_env(Some("claude")).is_empty());
        let cmd = ShellSpec::new("cmd.exe", vec![]);
        assert_eq!(cmd.args_with_command(Some("codex")), vec!["/K", "%NOBLE_LAUNCH%"]);
        let quoted = r#""C:\Program Files\x\claude.exe" --resume"#;
        assert!(cmd.extra_env(Some(quoted)).contains(&("NOBLE_LAUNCH".into(), quoted.into())));
        assert!(cmd.args_with_command(None).is_empty());
        let bash = ShellSpec::new("/bin/bash", vec![]);
        assert_eq!(bash.args_with_command(Some("gemini")), vec!["-c", "gemini; exec '/bin/bash'"]);
        assert!(bash.args_with_command(None).is_empty());
    }

    #[test]
    fn unix_shell_integration() {
        let dir = PathBuf::from("/data/shell");
        let with = |program: &str, args: Vec<String>| {
            let mut spec = ShellSpec::new(program, args);
            spec.integration = Some(dir.clone());
            spec
        };
        let rc = integration::bash_rc(&dir).display().to_string();
        let bash = with("/usr/bin/bash", vec!["-i".into()]);
        assert_eq!(bash.args_with_command(None), vec!["--rcfile".to_string(), rc.clone(), "-i".into()]);
        // After a launcher command the shell comes back with the integration.
        let launched = bash.args_with_command(Some("claude"));
        assert_eq!(launched[..2], ["-i", "-c"]);
        assert_eq!(launched[2], format!("claude; exec '/usr/bin/bash' '--rcfile' '{rc}' '-i'"));
        // Arguments that skip the startup files turn it off.
        assert_eq!(with("bash", vec!["--norc".into()]).args_with_command(None), vec!["--norc"]);

        let zsh = with("/bin/zsh", vec![]);
        assert!(zsh.args_with_command(None).is_empty());
        let env = zsh.extra_env(None);
        let zdotdir = integration::zsh_dir(&dir).display().to_string();
        assert!(env.contains(&("ZDOTDIR".into(), zdotdir)), "{env:?}");
        assert!(with("zsh", vec!["-f".into()]).extra_env(None).is_empty());

        let fish = with("fish", vec![]);
        let args = fish.args_with_command(None);
        assert_eq!(args[0], "--init-command");
        assert!(args[1].starts_with("source '") && args[1].contains("noble.fish"), "{args:?}");

        // Other shells start as configured.
        assert!(with("/bin/dash", vec![]).args_with_command(None).is_empty());
        assert!(with("/bin/dash", vec![]).extra_env(None).is_empty());
    }

    #[test]
    fn unix_quoting() {
        let sh = ShellSpec::new("/bin/sh", vec![]);
        assert_eq!(sh.quote("it's"), r"'it'\''s'");
        let fish = ShellSpec::new("fish", vec![]);
        assert_eq!(fish.quote(r"a\b'c"), r"'a\\b\'c'");
        assert_eq!(sh.invocation_of(Path::new("/opt/my tool/x"), "--y"), "'/opt/my tool/x' --y");
    }

    #[test]
    fn launcher_invocation() {
        let pwsh = ShellSpec::new("powershell.exe", vec![]);
        // A command that is not on PATH stays as it is.
        assert_eq!(pwsh.invocation("definitely-not-a-real-tool --x"), "definitely-not-a-real-tool --x");
        if cfg!(windows) {
            let inv = pwsh.invocation("cmd /c echo hi");
            assert!(
                inv.starts_with("& '") && inv.to_lowercase().contains("cmd.exe'") && inv.ends_with(" /c echo hi"),
                "{inv}"
            );
            let cmd = ShellSpec::new("cmd.exe", vec![]);
            assert!(cmd.invocation("cmd").starts_with('"'));
        } else {
            // On Unix the command runs as written: the shell finds it on PATH itself.
            let bash = ShellSpec::new("/bin/bash", vec![]);
            assert_eq!(bash.invocation("sh -c 'echo hi'"), "sh -c 'echo hi'");
        }
    }

    /// A title set by a program that has since exited (the shell prompt came back) is
    /// dropped; one the shell sets for its prompt, after the mark, is kept.
    #[test]
    fn prompt_signal_drops_the_finished_programs_title() {
        for mark in [&b"\x1b]7;file://h/tmp\x07"[..], b"\x1b]9;9;C:\\x\x1b\\", b"\x1b]133;A\x07", b"\x1b]133;D;0\x07"] {
            let mut parser = vt100::Parser::new_with_callbacks(10, 40, 0, Callbacks::default());
            parser.process("\x1b]0;\u{2733} Claude Code\x07".as_bytes());
            assert_eq!(parser.callbacks().title, "\u{2733} Claude Code");
            // Agent progress (OSC 9;4) and notifications are not prompt marks.
            parser.process(b"\x1b]9;4;3;0\x07\x1b]9;done\x07");
            assert!(!parser.callbacks().prompt);
            assert_eq!(parser.callbacks().title, "\u{2733} Claude Code");
            parser.process(mark);
            assert!(parser.callbacks().prompt, "{mark:?}");
            assert_eq!(parser.callbacks().title, "", "{mark:?}");
            parser.process(b"\x1b]0;user@host: ~\x07");
            assert_eq!(parser.callbacks().title, "user@host: ~");
        }
    }

    #[test]
    fn callbacks_answer_queries() {
        let mut parser = vt100::Parser::new_with_callbacks(10, 40, 0, Callbacks::default());
        parser.process(b"abc\x1b[6n\x1b]7;file://h/tmp/x\x07\x1b]0;C:\\bin\\cmd.exe\x07");
        let cb = parser.callbacks();
        assert_eq!(cb.responses, b"\x1b[1;4R");
        assert_eq!(cb.title, "C:\\bin\\cmd.exe");
        assert_eq!(cb.cwd.as_deref(), Some("/tmp/x"));
        assert!(cb.prompt);
    }

    #[test]
    fn osc8_hyperlinks_are_tracked() {
        let mut parser = vt100::Parser::new_with_callbacks(5, 40, 100, Callbacks::default());
        parser.process(b"see \x1b]8;id=1;https://example.com/a;b\x1b\\docs\x1b]8;;\x1b\\ now");
        let links = &parser.callbacks().hyperlinks;
        assert_eq!(links.len(), 1);
        assert_eq!(links[0], Hyperlink { line: 0, from: 4, to: 8, url: "https://example.com/a;b".into() });
        // The absolute line number stays fixed while the screen scrolls.
        parser.process(b"\r\n\n\n\n\n\n\x1b]8;;file:///tmp/x.rs\x07x.rs\x1b]8;;\x07");
        let last = parser.callbacks().hyperlinks.back().unwrap().clone();
        assert_eq!((last.from, last.to), (0, 4));
        assert!(last.line >= 6, "{last:?}");
    }

    #[test]
    fn notifications_and_prompt_marks() {
        let mut parser = vt100::Parser::new_with_callbacks(10, 40, 0, Callbacks::default());
        // A progress bar (9;4) is not a notification.
        parser.process(b"]9;4;1;50");
        assert_eq!(parser.callbacks().notice, None);
        assert!(!parser.callbacks().prompt);
        parser.process(
            b"]9;

Claude needs your permission",
        );
        assert_eq!(parser.callbacks().notice.as_deref(), Some("Claude needs your permission"));
        parser.process(b"]777;notify;Build;done in 3s");
        assert_eq!(parser.callbacks().notice.as_deref(), Some("Build · done in 3s"));
        parser.process(b"]133;D;0");
        assert!(parser.callbacks().prompt);
    }

    #[test]
    fn search_is_case_insensitive_and_width_aware() {
        let lines = vec!["Error: x".to_string(), "日本 error error".to_string(), "none".to_string()];
        let m = find_matches(&lines, "ERROR");
        assert_eq!(
            m,
            vec![
                Match { line: 0, col: 0, width: 5 },
                Match { line: 1, col: 5, width: 5 },
                Match { line: 1, col: 11, width: 5 },
            ]
        );
        assert!(find_matches(&lines, "").is_empty());
    }

    #[test]
    fn selection_order() {
        let s = Selection { anchor: (3, 5), head: (1, 2) };
        assert_eq!(s.ordered(), ((1, 2), (3, 5)));
        assert!(s.contains(2, 0));
        assert!(!s.contains(3, 6));
    }
}
