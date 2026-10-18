//! A single terminal panel: a real PTY (ConPTY/Unix pty) + the vt100 emulator.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Context, Result};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::config::TerminalCfg;
use crate::event::AppEvent;
use crate::term::integration;
use crate::term::layout::PaneId;
use crate::util;

#[cfg(test)]
use super::osc::same_host;
pub use super::osc::{Callbacks, Hyperlink, cwd_url_host, msys_to_windows, parse_cwd_url, windows_cwd};

/// Scrollback length (without disturbing the visible scroll position).
pub(super) fn history_len(screen: &mut vt100::Screen) -> usize {
    let current = screen.scrollback();
    screen.set_scrollback(usize::MAX);
    let len = screen.scrollback();
    screen.set_scrollback(current);
    len
}

/// Text of columns `from..to` on an absolute line (0 = oldest scrollback line), or `None` if the line
/// is gone. The visible scroll position is preserved.
pub(super) fn span_text(screen: &mut vt100::Screen, line: usize, from: u16, to: u16) -> Option<String> {
    let current = screen.scrollback();
    screen.set_scrollback(usize::MAX);
    let history = screen.scrollback();
    let (row, offset) = if line >= history { (line - history, 0) } else { (0, history - line) };
    screen.set_scrollback(offset);
    let text = screen.rows(from, to.saturating_sub(from)).nth(row);
    screen.set_scrollback(current);
    text
}

/// The bytes a paste sends. Inside bracketed paste every ESC is dropped: a pasted `ESC[201~` would
/// otherwise end the paste early and run the rest as typed input (other terminals sanitize the same way).
fn paste_payload(text: &str, bracketed: bool) -> String {
    let body = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed { format!("\x1b[200~{}\x1b[201~", body.replace('\x1b', "")) } else { body }
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
            "wsl" if cfg!(windows) => ShellKind::Wsl,
            // `bash` on a Windows PATH is often `System32\bash.exe`, WSL's launcher: a Linux bash that
            // cannot read a Windows `--rcfile` (it would skip `~/.bashrc` too). Git Bash's is not there.
            "bash" if cfg!(windows) && is_windows_system_program(&self.program) => ShellKind::Wsl,
            "bash" => ShellKind::Bash,
            "zsh" => ShellKind::Zsh,
            "fish" => ShellKind::Fish,
            _ => ShellKind::Posix,
        }
    }

    /// The system `zshenv` that keeps NOBLE's zsh integration from loading, when this is a zsh pane
    /// that should have it (see `integration::system_zshenv_forcing_zdotdir`).
    pub fn zsh_integration_blocked(&self) -> Option<PathBuf> {
        (matches!(self.kind(), ShellKind::Zsh) && self.integration_dir().is_some())
            .then(integration::system_zshenv_forcing_zdotdir)
            .flatten()
    }

    /// cmd.exe (it cannot start in a UNC directory, see `Pane::spawn`).
    pub fn is_cmd(&self) -> bool {
        matches!(self.kind(), ShellKind::Cmd)
    }

    /// Do the user's PowerShell arguments already run something (`-Command`, `-File`, `-EncodedCommand`,
    /// any abbreviation PowerShell accepts)? Then NOBLE adds no `-Command` of its own: PowerShell would
    /// take everything after the first one as a single command and the user's would break.
    fn pwsh_runs_own_command(&self) -> bool {
        self.args.iter().any(|a| {
            let Some(name) = a.strip_prefix('-').or_else(|| a.strip_prefix('/')) else { return false };
            let name = name.split(':').next().unwrap_or(name).to_ascii_lowercase();
            matches!(name.as_str(), "c" | "f" | "e" | "ec")
                || (name.len() >= 3 && ["command", "file", "encodedcommand"].iter().any(|full| full.starts_with(&name)))
        })
    }

    /// The integration directory, unless this shell has none or the user's
    /// arguments skip the startup files (or run a command instead).
    fn integration_dir(&self) -> Option<&Path> {
        let dir = self.integration.as_deref()?;
        let skip: &[&str] = match self.kind() {
            ShellKind::Bash => &["--norc", "--rcfile", "--init-file", "-c"],
            ShellKind::Zsh => &["-f", "--no-rcs", "-c"],
            // The README documents these as "start the shell untouched" (fish would still run
            // `--init-command` with `-N`, but the opt-out is kept for every shell alike).
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
                // A login shell never reads `--rcfile`: the login option is dropped and the
                // login script loads the profile files instead.
                let login = self.args.iter().any(|a| without_bash_login(a).as_deref() != Some(a.as_str()));
                let rc = if login { integration::bash_login_rc(dir) } else { integration::bash_rc(dir) };
                // bash wants long options before single-letter ones.
                args.extend(["--rcfile".into(), rc.display().to_string()]);
                args.extend(self.args.iter().filter_map(|a| without_bash_login(a)));
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

    /// Quotes one argument (a file name, say) so this shell passes it to the program
    /// literally, with no expansion or command substitution. `None` when the shell
    /// cannot take the text safely: cmd.exe has no escape for `"` inside quotes and
    /// expands `%…%` / `!…!` even there, so such words are refused (as is any control
    /// character there, and a NUL everywhere). PowerShell and the Unix shells take any
    /// other text.
    pub fn quote_arg(&self, word: &str) -> Option<String> {
        if word.contains('\0') {
            return None;
        }
        match self.kind() {
            ShellKind::PowerShell => Some(pwsh_quote(word)),
            ShellKind::Cmd => {
                if word.chars().any(|c| matches!(c, '"' | '%' | '!') || c.is_control()) {
                    return None;
                }
                // Inside quotes `&|<>^()` are plain text for cmd.exe. The program then splits its
                // command line with the C runtime rules, where backslashes before the closing
                // quote would escape it, so those are doubled.
                let trailing = word.len() - word.trim_end_matches('\\').len();
                Some(format!("\"{word}{}\"", "\\".repeat(trailing)))
            }
            ShellKind::Bash | ShellKind::Zsh | ShellKind::Fish | ShellKind::Posix | ShellKind::Wsl => {
                Some(self.quote(word))
            }
        }
    }

    /// `command` followed by `arg` quoted for this shell (see `quote_arg`); `None` when the
    /// shell cannot take `arg` safely. `command` itself is used as written.
    pub fn command_with_arg(&self, command: &str, arg: &str) -> Option<String> {
        Some(format!("{command} {}", self.quote_arg(arg)?))
    }

    /// Arguments that start the shell so it runs `command` first and then stays
    /// interactive. A prompt hook reporting the working directory is added as well
    /// (so the split and the session record know the real directory).
    pub fn args_with_command(&self, command: Option<&str>) -> Vec<String> {
        let mut args = self.args.clone();
        let cmd = command.filter(|c| !c.trim().is_empty());
        match self.kind() {
            ShellKind::PowerShell if self.pwsh_runs_own_command() => {}
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
            // WSL: the command runs in Linux, then the user's login shell there takes over. `bash.exe`
            // takes `-c` like bash; `wsl.exe` runs a program with `-e`.
            ShellKind::Wsl => {
                if let Some(c) = cmd {
                    if self.label().eq_ignore_ascii_case("bash") {
                        args.extend(["-c".into(), format!("{c}; exec bash")]);
                    } else {
                        args.extend([
                            "-e".into(),
                            "sh".into(),
                            "-c".into(),
                            format!("{c}; exec \"${{SHELL:-/bin/sh}}\""),
                        ]);
                    }
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
            ShellKind::PowerShell => format!("& {}{tail}", pwsh_quote(&path.display().to_string())),
            ShellKind::Cmd => format!("\"{}\"{tail}", path.display()),
            ShellKind::Bash | ShellKind::Zsh | ShellKind::Fish | ShellKind::Posix | ShellKind::Wsl => {
                command.to_string()
            }
        }
    }

    /// A command line that runs a program (given by full path) with its arguments
    /// (paths with spaces are quoted per the shell).
    pub fn invocation_of(&self, program: &Path, args: &str) -> String {
        let path = program.display().to_string();
        match self.kind() {
            ShellKind::PowerShell => format!("& {} {args}", pwsh_quote(&path)),
            ShellKind::Cmd => format!("\"{path}\" {args}"),
            ShellKind::Bash | ShellKind::Zsh | ShellKind::Fish | ShellKind::Posix => {
                format!("{} {args}", self.quote(&path))
            }
            // A Windows program from inside WSL: through its `/mnt/c/...` path (WSL interop runs it).
            ShellKind::Wsl => format!("\"$(wslpath {})\" {args}", self.quote(&path)),
        }
    }

    /// Shell-specific environment variables: for cmd a directory-reporting PROMPT
    /// and the command to run (`NOBLE_LAUNCH`, so the quotes stay untouched); for
    /// zsh the integration's `ZDOTDIR` and the user's own one.
    pub fn extra_env(&self, command: Option<&str>) -> Vec<(String, String)> {
        let mut env = Vec::new();
        match self.kind() {
            ShellKind::Cmd => {
                // cmd.exe's PROMPT cannot show `%ERRORLEVEL%` (it is not expanded there), so cmd
                // reports only the directory: its panes show how long a command took, without an
                // exit code. PowerShell and the Unix shells also send OSC 133;D;<code>.
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

/// Reports the last command's exit code (OSC 133;D) and the working directory (OSC 9;9) on
/// every prompt, wrapping the current prompt (oh-my-posh included). `$?` is read first, before
/// anything resets it: success is 0, a failure is `$LASTEXITCODE` when a program set one, else 1
/// (a failed cmdlet). A failure is handed on to the wrapped prompt (`Write-Error … Ignore` sets `$?`
/// to false without printing or recording anything), so a prompt that reports its own status (the
/// Windows Terminal snippet, oh-my-posh) still sees it. Once PSReadLine is loaded, its
/// `PSConsoleHostReadLine` is wrapped too: a line that is not blank or only a comment marks the
/// command's start (OSC 133;C) before it runs. `Get-Variable … -ErrorAction Ignore` and the
/// `$global:__nobleR` set up front keep a profile's `Set-StrictMode` from breaking the prompt.
/// It contains no double quotes so command line quoting never breaks.
pub const PWSH_CWD_HOOK: &str = r"$global:__nobleR=$null; $global:__nobleP=$function:prompt; function global:prompt { $s=$?; if (-not $global:__nobleR -and (Test-Path Function:\PSConsoleHostReadLine)) { $global:__nobleR=$function:PSConsoleHostReadLine; function global:PSConsoleHostReadLine { $l=& $global:__nobleR; if ($l -notmatch '^\s*(#.*)?$') { [Console]::Write([char]27+']133;C'+[char]7) }; $l } }; $c=Get-Variable LASTEXITCODE -Scope Global -ValueOnly -ErrorAction Ignore; $e=1; if ($s) { $e=0 } elseif ($c) { $e=$c }; [Console]::Write([char]27+']133;D;'+$e+[char]7+[char]27+']9;9;'+$executionContext.SessionState.Path.CurrentLocation.ProviderPath+[char]27+'\'); if (-not $s) { Write-Error '' -ErrorAction Ignore }; & $global:__nobleP }";

/// A PowerShell single-quoted string: nothing inside is expanded. PowerShell also takes the
/// typographic quotes `‘ ’ ‚ ‛` as single quotes, so each of them is doubled too (a file
/// name may well contain `’`).
fn pwsh_quote(word: &str) -> String {
    let mut out = String::with_capacity(word.len() + 2);
    out.push('\'');
    for c in word.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// A bash argument without its login option: `-l` and `--login` disappear, a short option
/// cluster such as `-il` becomes `-i`; anything else is returned as it is.
fn without_bash_login(arg: &str) -> Option<String> {
    if arg == "--login" {
        return None;
    }
    let short = arg.strip_prefix('-').filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphabetic()));
    match short {
        Some(flags) if flags.contains('l') => {
            let rest = flags.replace('l', "");
            (!rest.is_empty()).then(|| format!("-{rest}"))
        }
        _ => Some(arg.to_string()),
    }
}

enum ShellKind {
    PowerShell,
    Cmd,
    Bash,
    Zsh,
    Fish,
    /// Another Unix shell (sh, dash, nu …): started as configured, without integration.
    Posix,
    /// Windows: `wsl.exe` or WSL's `System32\bash.exe`. A Linux shell behind a Windows launcher: no
    /// integration (its scripts would be Windows paths), and launcher commands go through `-c` / `-e`.
    Wsl,
}

/// Windows: is `program` (a name looked up on PATH, or a path) in the Windows system folder? That is
/// where WSL's `bash.exe` lives; Git Bash, MSYS2 and Cygwin install elsewhere.
fn is_windows_system_program(program: &str) -> bool {
    let path = if Path::new(program).components().count() > 1 {
        PathBuf::from(program)
    } else {
        match util::which(program) {
            Some(p) => p,
            None => return false,
        }
    };
    let Some(root) = std::env::var_os("SystemRoot").or_else(|| std::env::var_os("windir")) else { return false };
    let lower = |p: &Path| p.to_string_lossy().to_lowercase().replace('/', "\\");
    let root = lower(Path::new(&root));
    let path = lower(&path);
    ["system32", "sysnative"].iter().any(|d| path.starts_with(&format!("{root}\\{d}\\")))
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
        // macOS: a login PowerShell, like the other shells below (`-Login` must come first).
        // Windows and Linux start it as it is.
        if cfg!(target_os = "macos") {
            spec.args.push("-Login".into());
        }
        spec.args.push("-NoLogo".into());
    }
    // macOS terminals start login shells: `/etc/zprofile` (path_helper), `~/.zprofile`
    // (Homebrew's `brew shellenv`) and `~/.bash_profile` hold the PATH and prompt there.
    // Linux terminals start interactive non-login shells, so there the shell runs as it is.
    if cfg!(target_os = "macos")
        && cfg.shell_args.is_empty()
        && matches!(spec.kind(), ShellKind::Bash | ShellKind::Zsh | ShellKind::Fish)
    {
        spec.args.push("-l".into());
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

/// The word around `col` in a row of characters (blank = ' ', the second half of a wide character =
/// '\0'): the inclusive column range, or `None` on a blank. Quotes and brackets end a word; path and
/// URL characters (`/ . : - _ ~ \\ ...`) stay in it so a double click picks a whole path or address.
pub fn word_range(row: &[char], col: usize) -> Option<(usize, usize)> {
    let word = |c: char| c == '\0' || !(c.is_whitespace() || "\"'`()[]{}<>|;,".contains(c));
    if !row.get(col).copied().is_some_and(word) {
        return None;
    }
    let mut start = col;
    while start > 0 && word(row[start - 1]) {
        start -= 1;
    }
    let mut end = col;
    while end + 1 < row.len() && word(row[end + 1]) {
        end += 1;
    }
    Some((start, end))
}

pub struct Pane {
    pub id: PaneId,
    pub parser: Arc<Mutex<vt100::Parser<Callbacks>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    pub dirty: Arc<AtomicBool>,
    /// Counts the output chunks the reader thread has fed the parser (`scroll_text` keys its copy on it).
    output_gen: Arc<AtomicU64>,
    /// (row, col)
    pub size: (u16, u16),
    pub start_cwd: PathBuf,
    /// The last directory the shell reported, checked once (`cwd`): (reported text, usable path).
    cwd_checked: Mutex<Option<(String, PathBuf)>>,
    pub shell_label: String,
    pub command: Option<String>,
    /// The quick-launch command (as configured, e.g. `claude`) this pane was opened with;
    /// saved in the session and run again on restore.
    pub launcher: Option<String>,
    pub selection: Option<Selection>,
    pub pid: Option<u32>,
    /// When the command at the prompt started: the shell's `OSC 133;C` mark, or, for a shell that
    /// sends none (cmd.exe, bash 3.2, PowerShell without PSReadLine), the last Enter pressed.
    /// Reset when the prompt comes back.
    pub command_started: Option<std::time::Instant>,
    /// Key lock: NOBLE's direct shortcuts go to the app in this pane instead (prefix still works).
    pub passthrough: bool,
    /// A prompt signal arrived at least once: the shell integration works, so `command_started` is reliable.
    pub prompted: bool,
    /// Keys other than Enter (or a paste) reached the shell since its last prompt: the next Enter runs
    /// a command. An Enter on an empty line brings the prompt back too, but records no result.
    /// Only a guess, used by shells that do not mark command starts (`start_marks`).
    pub edited: bool,
    /// The shell marks when a command starts (`OSC 133;C` seen at least once). Then only that mark
    /// starts the clock and counts a command as run; Enter keys are not looked at.
    pub start_marks: bool,
    /// A command start mark arrived since the last prompt: the shell ran what was typed.
    pub ran: bool,
    /// When output last arrived (set on the main thread): a working agent's spinner only turns
    /// while its pane shows it is busy.
    pub last_output: Option<std::time::Instant>,
    /// How the last command at the prompt ended; shown in the pane title until the next one starts.
    pub last_result: Option<CommandResult>,
    /// While scrolled back: the offset last chosen here and the lines that arrived before that
    /// choice. vt100 raises the offset by one for every line that arrives meanwhile (the view stays
    /// put), so the growth past the chosen offset is the new output below (`new_lines`).
    /// Only the main thread scrolls, so a `Cell` is enough.
    scroll_mark: std::cell::Cell<(usize, usize)>,
}

/// A finished command: exit code (`None` when the shell does not report one, cmd.exe) and run time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandResult {
    pub code: Option<i32>,
    pub took: std::time::Duration,
    /// When it finished.
    pub at: std::time::Instant,
}

/// A running command shows its live timer in the pane title only after this long.
pub const TIMER_AFTER: std::time::Duration = std::time::Duration::from_secs(2);

/// Live timer text for a command running for `elapsed`: none before `TIMER_AFTER`, then `0:07` with
/// seconds for the first minute and `3m` / `1h02m` after it (it is redrawn once a minute then). On
/// battery it is redrawn once a minute from the start, so the first minute shows the glyph alone.
pub fn live_timer(elapsed: std::time::Duration, on_battery: bool) -> Option<String> {
    let s = elapsed.as_secs();
    Some(match s {
        _ if elapsed < TIMER_AFTER => return None,
        0..60 if on_battery => "◷".to_string(),
        0..60 => format!("◷ 0:{s:02}"),
        60..3600 => format!("◷ {}m", s / 60),
        _ => format!("◷ {}h{:02}m", s / 3600, s % 3600 / 60),
    })
}

/// When the text of `live_timer` changes next: at `TIMER_AFTER`, then every second of the first
/// minute (every minute on battery), then at each full minute.
pub fn live_timer_next(elapsed: std::time::Duration, on_battery: bool) -> std::time::Duration {
    use std::time::Duration;
    if elapsed < TIMER_AFTER {
        return TIMER_AFTER - elapsed;
    }
    let step = if elapsed.as_secs() < 60 && !on_battery { 1 } else { 60 };
    let into = Duration::new(elapsed.as_secs() % step, elapsed.subsec_nanos());
    // A little past the change, so the redraw never lands just before it.
    Duration::from_secs(step) - into + Duration::from_millis(5)
}

pub struct SpawnSpec<'a> {
    pub cwd: &'a Path,
    pub command: Option<&'a str>,
    pub shell: &'a ShellSpec,
    pub rows: u16,
    pub cols: u16,
    pub scrollback: usize,
}

/// How long a newly reported working directory may take to answer before it counts as unusable.
const CWD_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

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
        let mut cwd = if spec.cwd.is_dir() {
            spec.cwd.to_path_buf()
        } else {
            dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
        };
        // cmd.exe cannot start in a UNC directory (`\\server\share`, `\\wsl$\…`): it falls back to
        // C:\Windows. It starts in the home folder and `pushd` maps the share to a drive letter instead.
        // PowerShell and the Unix shells start there directly.
        let unc_launch;
        let mut command = spec.command;
        if shell.is_cmd() && cwd.to_string_lossy().starts_with(r"\\") {
            let pushd = format!("pushd \"{}\"", cwd.display());
            unc_launch = match command.filter(|c| !c.trim().is_empty()) {
                Some(c) => format!("{pushd} && {c}"),
                None => pushd,
            };
            command = Some(&unc_launch);
            cwd = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        }
        let mut cmd = CommandBuilder::new(&shell.program);
        cmd.args(shell.args_with_command(command));
        cmd.cwd(&cwd);
        // bash and zsh take `PWD` as the logical directory when it matches the real one, so a
        // folder reached through a symlink keeps its name (macOS `/var` is `/private/var`).
        // Windows shells report their directory with OSC 9;9 and ignore it.
        #[cfg(unix)]
        cmd.env("PWD", &cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "NOBLE");
        // The outer terminal's identity (kitty/Ghostty terminfo, iTerm2) does not describe the
        // pane: apps would look up the wrong terminfo or send that terminal's own sequences.
        // Windows Terminal's `WT_SESSION` / `WT_PROFILE_ID` would make apps think they draw in it directly.
        for var in [
            "TERMINFO",
            "TERM_PROGRAM_VERSION",
            "LC_TERMINAL",
            "LC_TERMINAL_VERSION",
            "ITERM_SESSION_ID",
            "WT_SESSION",
            "WT_PROFILE_ID",
        ] {
            cmd.env_remove(var);
        }
        cmd.env("NOBLE_PANE", id.to_string());
        // So the Claude Code hooks write the state to the right NOBLE instance.
        cmd.env("NOBLE_INSTANCE", std::process::id().to_string());
        for (k, v) in shell.extra_env(command) {
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
        let output_gen = Arc::new(AtomicU64::new(0));

        {
            let parser = parser.clone();
            let writer = writer.clone();
            let dirty = dirty.clone();
            let output_gen = output_gen.clone();
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
                                // Bumped under the same lock `scroll_text` reads it with: a new value means new output.
                                output_gen.fetch_add(1, Ordering::Release);
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
            output_gen,
            size: (rows, cols),
            start_cwd: cwd,
            cwd_checked: Mutex::new(None),
            shell_label: spec.shell.label(),
            command: spec.command.map(str::to_string),
            launcher: None,
            selection: None,
            pid,
            command_started: None,
            passthrough: false,
            prompted: false,
            edited: false,
            start_marks: false,
            ran: false,
            last_output: None,
            last_result: None,
            scroll_mark: std::cell::Cell::new((0, 0)),
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
    pub fn paste(&mut self, text: &str) {
        self.edited = true;
        let bracketed = lock(&self.parser).screen().bracketed_paste();
        self.write(paste_payload(text, bracketed).as_bytes());
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
        let cur = p.screen().scrollback() as i64;
        self.set_offset(&mut p, (cur + delta as i64).max(0) as usize);
    }

    pub fn scroll_reset(&self) {
        let mut p = lock(&self.parser);
        if p.screen().scrollback() != 0 {
            self.set_offset(&mut p, 0);
        }
    }

    /// Sets the scroll offset and keeps `scroll_mark` (the count of new lines below) in step.
    fn set_offset(&self, p: &mut vt100::Parser<Callbacks>, offset: usize) {
        let arrived = self.arrived(p.screen().scrollback());
        p.screen_mut().set_scrollback(offset);
        let now = p.screen().scrollback();
        self.scroll_mark.set(if now == 0 { (0, 0) } else { (now, arrived) });
    }

    /// Lines that arrived since scrolling back, at the current offset `cur`.
    fn arrived(&self, cur: usize) -> usize {
        if cur == 0 {
            return 0;
        }
        let (chosen, before) = self.scroll_mark.get();
        before + cur.saturating_sub(chosen)
    }

    /// New output lines that arrived below the view while it was scrolled back (0 when live).
    /// Once the scrollback is full the offset stops growing, so the count stops there too.
    pub fn new_lines(&self) -> usize {
        self.arrived(self.scroll_offset())
    }

    /// The scrollback length (lines above the screen), without moving the view.
    pub fn history_len(&self) -> usize {
        history_len(lock(&self.parser).screen_mut())
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

    /// The directory the shell reported, or the start directory when it is not usable. Each newly
    /// reported directory is checked once (time-limited, `util::usable_dirs`) and the answer is
    /// kept, so drawing never touches the disk and a dead network share cannot freeze the UI.
    pub fn cwd(&self) -> PathBuf {
        let reported = lock(&self.parser).callbacks().cwd.clone();
        let Some(reported) = reported else { return self.start_cwd.clone() };
        let mut checked = lock(&self.cwd_checked);
        if let Some((raw, path)) = checked.as_ref()
            && *raw == reported
        {
            return path.clone();
        }
        let dir = PathBuf::from(&reported);
        let usable = util::usable_dirs(std::slice::from_ref(&dir), CWD_PROBE_TIMEOUT, |p| p.is_dir()).contains(&dir);
        let path = if usable { dir } else { self.start_cwd.clone() };
        *checked = Some((reported, path.clone()));
        path
    }

    /// Name of the folder the shell last reported (or started in). No file system access, so
    /// it is cheap enough for every frame (the tab strip); a root has no name and shows whole.
    pub fn dir_name(&self) -> String {
        let p = lock(&self.parser);
        let path = p.callbacks().cwd.as_ref().map(PathBuf::from).unwrap_or_else(|| self.start_cwd.clone());
        path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
    }

    /// Clipboard content the app requested via OSC 52 (once).
    pub fn take_clipboard(&self) -> Option<String> {
        lock(&self.parser).callbacks_mut().clipboard.take()
    }

    pub fn take_bell(&self) -> bool {
        std::mem::take(&mut lock(&self.parser).callbacks_mut().bell)
    }

    /// The prompt signal since the last call: `Some` with the exit code the shell reported with it
    /// (`Some(None)` when it reported none).
    pub fn take_prompt(&self) -> Option<Option<i32>> {
        let mut p = lock(&self.parser);
        let cb = p.callbacks_mut();
        let exit = cb.exit.take();
        std::mem::take(&mut cb.prompt).then_some(exit)
    }

    /// When the shell last marked a command start (`OSC 133;C`), once.
    pub fn take_started(&self) -> Option<std::time::Instant> {
        lock(&self.parser).callbacks_mut().started.take()
    }

    /// Whether the prompt that just came back ends a command that ran: one the shell marked as
    /// started (`ran`), or, without such marks, a line that was typed (`edited`). Never for the
    /// shell's first prompt (`prompted`), which also follows a launcher command.
    pub fn command_ran(&self) -> bool {
        self.prompted && if self.start_marks { self.ran } else { self.edited }
    }

    /// How long the command started at the prompt has been running. Only with a working shell
    /// integration (`prompted`): without prompt signals `command_started` would never reset.
    pub fn running_for(&self) -> Option<std::time::Duration> {
        self.command_started.filter(|_| self.prompted).map(|t| t.elapsed())
    }

    pub fn take_notice(&self) -> Option<String> {
        lock(&self.parser).callbacks_mut().notice.take()
    }

    /// The visible characters of a row (blank = ' ', second half of a wide character = '\0').
    fn row_chars(&self, row: u16) -> Vec<char> {
        let p = lock(&self.parser);
        let screen = p.screen();
        let (_, cols) = screen.size();
        (0..cols)
            .map(|c| match screen.cell(row, c) {
                Some(cell) if cell.is_wide_continuation() => '\0',
                Some(cell) => cell.contents().chars().next().unwrap_or(' '),
                None => ' ',
            })
            .collect()
    }

    /// Selection of the word under (row, col), for a double click.
    pub fn word_selection(&self, row: u16, col: u16) -> Option<Selection> {
        let (s, e) = word_range(&self.row_chars(row), col as usize)?;
        Some(Selection { anchor: (row, s as u16), head: (row, e as u16) })
    }

    /// Selection of the whole row up to its last character, for a triple click.
    pub fn line_selection(&self, row: u16) -> Option<Selection> {
        let chars = self.row_chars(row);
        let end = chars.iter().rposition(|c| *c != ' ')?;
        Some(Selection { anchor: (row, 0), head: (row, end as u16) })
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
        scrollback_lines(&mut lock(&self.parser))
    }

    /// The text search works on: `have` (the copy the caller holds) is returned as it is while the pane has
    /// produced no output since it was read and kept its size, so a query typed without new output does not
    /// read the whole scrollback again. Output or a resize reads it anew.
    pub fn scroll_text(&self, have: Option<&ScrollText>) -> ScrollText {
        let mut p = lock(&self.parser);
        let generation = self.output_gen.load(Ordering::Acquire);
        read_scroll_text(&mut p, generation, have)
    }

    /// Scrolls so the absolute line (0 = oldest) lands in the middle of the screen.
    pub fn scroll_to_line(&self, line: usize, history: usize) {
        let rows = self.size.0 as usize;
        let top = line.saturating_sub(rows / 2);
        let offset = history.saturating_sub(top);
        self.set_offset(&mut lock(&self.parser), offset);
    }

    /// The OSC 8 link at on-screen (row, col): address and column span.
    pub fn hyperlink_at(&self, row: u16, col: u16) -> Option<(String, u16, u16)> {
        hyperlink_in(&mut lock(&self.parser), row, col)
    }

    /// What is still running in the pane (its label), if closing it would stop something:
    /// a full-screen app, a command started at the prompt, or a launcher command before its first prompt.
    pub fn busy(&self) -> Option<String> {
        let alt = lock(&self.parser).screen().alternate_screen();
        let command = if self.prompted { self.command_started.is_some() } else { self.command.is_some() };
        // The shell rarely names the command it runs (no title): say "a command" rather than the shell.
        let label = self.label();
        (alt || command).then(|| if label == self.shell_label { "a command".into() } else { label })
    }

    /// Visible line text (for link detection).
    pub fn visible_row(&self, row: u16) -> Option<String> {
        let p = lock(&self.parser);
        let cols = p.screen().size().1;
        p.screen().rows(0, cols).nth(row as usize)
    }
}

/// All lines including the scrollback (0 = oldest) and the scrollback length, read from `p`.
/// The visible scroll position is preserved.
fn scrollback_lines(p: &mut vt100::Parser<Callbacks>) -> (Vec<String>, usize) {
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

/// The scrollback text search works on (`Pane::scroll_text`), with the output generation and screen size it
/// was read at.
#[derive(Clone, Debug)]
pub struct ScrollText {
    /// Every line, 0 = oldest. Shared, so a reused copy costs a pointer, not the lines.
    pub lines: Arc<Vec<String>>,
    /// Scrollback length (see `all_lines`).
    pub history: usize,
    generation: u64,
    size: (u16, u16),
}

/// The text of `p` at output generation `generation`: `have` itself when it was read at the same generation
/// and size, otherwise read anew.
fn read_scroll_text(p: &mut vt100::Parser<Callbacks>, generation: u64, have: Option<&ScrollText>) -> ScrollText {
    let size = p.screen().size();
    if let Some(text) = have.filter(|t| t.generation == generation && t.size == size) {
        return text.clone();
    }
    let (lines, history) = scrollback_lines(p);
    ScrollText { lines: Arc::new(lines), history, generation, size }
}

/// The OSC 8 link at on-screen (row, col): address and column span.
fn hyperlink_in(p: &mut vt100::Parser<Callbacks>, row: u16, col: u16) -> Option<(String, u16, u16)> {
    let offset = p.screen().scrollback();
    let line = history_len(p.screen_mut()).saturating_sub(offset) + row as usize;
    let screen = p.screen();
    p.callbacks()
        .hyperlinks
        .iter()
        .rev()
        // The text check rejects links whose absolute line drifted (full scrollback).
        .find(|h| {
            h.line == line
                && col >= h.from
                && col < h.to
                && screen.rows(h.from, h.to - h.from).nth(row as usize).is_some_and(|t| t == h.text)
        })
        .map(|h| (h.url.clone(), h.from, h.to))
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
    #[test]
    fn word_range_picks_paths_and_stops_at_delimiters() {
        let row: Vec<char> = "cd ~/code/app; echo \"hi there\"  ".chars().collect();
        assert_eq!(word_range(&row, 4), Some((3, 12)), "a path is one word");
        assert_eq!(word_range(&row, 3), Some((3, 12)));
        assert_eq!(word_range(&row, 2), None, "blank");
        assert_eq!(word_range(&row, 100), None, "past the row");
        let s = row.iter().collect::<String>();
        let hi = s.find("hi").unwrap();
        assert_eq!(word_range(&row, hi), Some((hi, hi + 1)), "quotes end the word");
    }

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
        assert_eq!(windows_cwd("/c/Users/me"), r"C:\Users\me");
        assert_eq!(windows_cwd("C:/Program Files/Git/usr"), r"C:\Program Files\Git\usr");
        assert_eq!(windows_cwd("//server/share/x"), r"\\server\share\x");
        assert_eq!(windows_cwd("/home/me"), "/home/me");
        assert_eq!(cwd_url_host("file://box/home"), Some("box".into()));
        assert_eq!(cwd_url_host("file:///home"), None);
        assert!(same_host("Box", "box.lan") && !same_host("box", "other"));
    }

    #[test]
    fn shell_command_args() {
        let pwsh = ShellSpec::new("C:\\pwsh.exe", vec!["-NoLogo".into()]);
        let with = pwsh.args_with_command(Some("claude"));
        assert_eq!(&with[..3], ["-NoLogo", "-NoExit", "-Command"]);
        assert!(with[3].starts_with(PWSH_CWD_HOOK) && with[3].ends_with("; claude"));
        assert!(!PWSH_CWD_HOOK.contains('"'));
        // The exit code is read first thing in the prompt and reported before the directory.
        assert!(PWSH_CWD_HOOK.contains("function global:prompt { $s=$?;"));
        assert!(PWSH_CWD_HOOK.find("]133;D;").unwrap() < PWSH_CWD_HOOK.find("]9;9;").unwrap());
        // The wrapped prompt still sees a failure in `$?` (a prompt may report its own status).
        assert!(PWSH_CWD_HOOK.ends_with("if (-not $s) { Write-Error '' -ErrorAction Ignore }; & $global:__nobleP }"));
        // PSReadLine's read-line is wrapped to mark a command's start, not for a blank line.
        assert!(PWSH_CWD_HOOK.contains("function global:PSConsoleHostReadLine {"));
        assert!(
            PWSH_CWD_HOOK.contains("if ($l -notmatch '^\\s*(#.*)?$') { [Console]::Write([char]27+']133;C'+[char]7) }")
        );
        // Set up front, so `Set-StrictMode` does not trip over it in the prompt.
        assert!(PWSH_CWD_HOOK.starts_with("$global:__nobleR=$null; "));
        assert_eq!(pwsh.args_with_command(None)[3], PWSH_CWD_HOOK);
        assert!(pwsh.extra_env(Some("claude")).is_empty());
        // The user's own `-Command` / `-File` (or an abbreviation) is left alone: no second `-Command`.
        for own in [["-Command", "Import-Module x"], ["-c", "x"], ["-File", "a.ps1"], ["-EncodedCommand", "AA"]] {
            let user: Vec<String> = own.iter().map(|a| a.to_string()).collect();
            let spec = ShellSpec::new("pwsh", user.clone());
            assert_eq!(spec.args_with_command(None), user, "{own:?}");
        }
        assert!(!ShellSpec::new("pwsh", vec!["-NoProfile".into()]).pwsh_runs_own_command());
        assert!(!ShellSpec::new("pwsh", vec!["-ConfigurationName".into(), "x".into()]).pwsh_runs_own_command());
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
        // A login shell never reads `--rcfile`: the login script replaces the option.
        let login_rc = integration::bash_login_rc(&dir).display().to_string();
        for (args, rest) in [(vec!["-l"], vec![]), (vec!["--login", "-i"], vec!["-i"]), (vec!["-il"], vec!["-i"])] {
            let spec = with("bash", args.iter().map(|a| a.to_string()).collect());
            let want: Vec<String> = ["--rcfile", login_rc.as_str()].into_iter().chain(rest).map(Into::into).collect();
            assert_eq!(spec.args_with_command(None), want, "{args:?}");
        }
        // The launcher command itself still runs in a login shell.
        assert_eq!(with("bash", vec!["-l".into()]).args_with_command(Some("x"))[..2], ["-l", "-c"]);
        assert_eq!(without_bash_login("-O"), Some("-O".into()));
        assert_eq!(without_bash_login("--rcfile"), Some("--rcfile".into()));

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
        assert_eq!(with("fish", vec!["-N".into()]).args_with_command(None), vec!["-N"]);

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

    /// Hostile file names are quoted so each shell takes them literally; what cmd.exe cannot
    /// take safely is refused. (The PTY test `hostile_file_name_is_not_executed` runs them.)
    #[test]
    fn argument_quoting_per_shell() {
        let hostile = "a $(touch x) `id` ' \" ; & | %PATH% !x! ^ ü 日本";
        let pwsh = ShellSpec::new(r"C:\Program Files\PowerShell\7\pwsh.exe", vec![]);
        assert_eq!(pwsh.quote_arg("C:\\a b\\x.rs").as_deref(), Some(r"'C:\a b\x.rs'"));
        assert_eq!(pwsh.quote_arg(hostile), Some(format!("'{}'", hostile.replace('\'', "''"))));
        // Typographic single quotes end a PowerShell string as well.
        assert_eq!(pwsh.quote_arg("it’s‘x‚y‛").as_deref(), Some("'it’’s‘‘x‚‚y‛‛'"));
        assert_eq!(ShellSpec::new("powershell", vec![]).quote_arg("$env:X").as_deref(), Some("'$env:X'"));

        let cmd = ShellSpec::new(r"C:\WINDOWS\system32\cmd.exe", vec![]);
        assert_eq!(
            cmd.quote_arg(r"C:\a&b^c|d<e>(f) ü 日本;'`.rs").as_deref(),
            Some(r#""C:\a&b^c|d<e>(f) ü 日本;'`.rs""#)
        );
        assert_eq!(cmd.quote_arg(r"C:\dir\").as_deref(), Some(r#""C:\dir\\""#));
        for refused in ["a\"b", "100%", "%PATH%", "hi!", "a\nb", "a\rb", hostile] {
            assert_eq!(cmd.quote_arg(refused), None, "{refused:?}");
        }
        assert_eq!(cmd.command_with_arg("notepad", "50%.txt"), None);

        for program in ["/bin/bash", "zsh", "/bin/sh", "dash"] {
            let sh = ShellSpec::new(program, vec![]);
            assert_eq!(sh.quote_arg(hostile), Some(format!("'{}'", hostile.replace('\'', r"'\''"))), "{program}");
            assert_eq!(sh.quote_arg("it's").as_deref(), Some(r"'it'\''s'"), "{program}");
            assert_eq!(sh.quote_arg(r"a\b").as_deref(), Some(r"'a\b'"), "{program}");
        }
        let fish = ShellSpec::new("/usr/bin/fish", vec![]);
        assert_eq!(fish.quote_arg(r"a\b'c $(x) ü").as_deref(), Some(r"'a\\b\'c $(x) ü'"));

        let bash = ShellSpec::new("bash", vec![]);
        assert_eq!(bash.command_with_arg("vim +3", "/tmp/a b;c").as_deref(), Some("vim +3 '/tmp/a b;c'"));
        // A NUL cannot be part of any argument.
        for spec in [&pwsh, &cmd, &bash, &fish] {
            assert_eq!(spec.quote_arg("a\0b"), None);
        }
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
        assert_eq!(
            links[0],
            Hyperlink { line: 0, from: 4, to: 8, url: "https://example.com/a;b".into(), text: "docs".into() }
        );
        // The absolute line number stays fixed while the screen scrolls.
        parser.process(b"\r\n\n\n\n\n\n\x1b]8;;file:///tmp/x.rs\x07x.rs\x1b]8;;\x07");
        let last = parser.callbacks().hyperlinks.back().unwrap().clone();
        assert_eq!((last.from, last.to), (0, 4));
        assert!(last.line >= 6, "{last:?}");
    }

    /// Once the scrollback is full its length stops growing while lines still scroll out, so a stored
    /// absolute line drifts: the link must not answer for text that is no longer there.
    #[test]
    fn osc8_hyperlink_is_not_returned_after_the_scrollback_drifts() {
        let mut parser = vt100::Parser::new_with_callbacks(4, 40, 3, Callbacks::default());
        // Fill the scrollback (3 lines) first, then write a link on the bottom row.
        parser.process(b"1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n");
        parser.process(b"\x1b]8;;https://a.example/\x1b\\alpha\x1b]8;;\x1b\\");
        assert_eq!(hyperlink_in(&mut parser, 3, 2).map(|l| l.0).as_deref(), Some("https://a.example/"));
        // Two more lines scroll out: the link moved up to row 1 while the scrollback length is unchanged.
        parser.process(b"\r\nx\r\ny");
        assert_eq!(hyperlink_in(&mut parser, 3, 2), None);
        assert_eq!(hyperlink_in(&mut parser, 1, 2), None);
        // A link written after the drift still works.
        parser.process(b"\r\n\x1b]8;;https://b.example/\x1b\\beta\x1b]8;;\x1b\\");
        assert_eq!(hyperlink_in(&mut parser, 3, 1).map(|l| l.0).as_deref(), Some("https://b.example/"));
    }

    #[test]
    fn bracketed_paste_drops_escape_characters() {
        // A pasted end marker must not close the paste early.
        let evil = "ls\x1b[201~\nrm -rf x";
        assert_eq!(paste_payload(evil, true), "\x1b[200~ls[201~\rrm -rf x\x1b[201~");
        assert_eq!(paste_payload("a\r\nb\nc", true), "\x1b[200~a\rb\rc\x1b[201~");
        assert_eq!(paste_payload("a\r\nb\nc", false), "a\rb\rc");
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

    /// `133;D;<code>` carries the exit code with the prompt signal; a bare `133;D` or `133;A` is a
    /// prompt without one and does not wipe a code that came just before it.
    #[test]
    fn exit_codes_from_prompt_marks() {
        let mut parser = vt100::Parser::new_with_callbacks(10, 40, 0, Callbacks::default());
        parser.process(b"\x1b]133;D;2\x07\x1b]7;file://h/tmp\x07");
        assert!(parser.callbacks().prompt);
        assert_eq!(parser.callbacks().exit, Some(2));
        parser.process(b"\x1b]133;D\x07\x1b]133;A\x07");
        assert_eq!(parser.callbacks().exit, Some(2));
        let cb = parser.callbacks_mut();
        cb.exit = None;
        cb.prompt = false;
        parser.process(b"\x1b]133;D\x07");
        assert!(parser.callbacks().prompt);
        assert_eq!(parser.callbacks().exit, None);
        // PowerShell's $LASTEXITCODE for a crashed Windows program, as signed or unsigned text.
        for (text, want) in [("0", 0), ("130", 130), ("-1073741510", -1073741510), ("3221225786", -1073741510)] {
            parser.callbacks_mut().exit = None;
            parser.process(format!("\x1b]133;D;{text}\x1b\\").as_bytes());
            assert_eq!(parser.callbacks().exit, Some(want), "{text}");
        }
        // Not a number: the prompt still counts, the code is unknown.
        parser.callbacks_mut().exit = None;
        parser.process(b"\x1b]133;D;x\x07");
        assert_eq!(parser.callbacks().exit, None);
    }

    /// A user's prompt that sends its own `133;D;<code>` right after NOBLE's hook (the Windows
    /// Terminal snippet, oh-my-posh) cannot hide the real code: the first one of a prompt wins.
    #[test]
    fn first_exit_code_of_a_prompt_wins() {
        let mut parser = vt100::Parser::new_with_callbacks(10, 40, 0, Callbacks::default());
        parser.process(b"\x1b]133;D;3\x07\x1b]9;9;C:/x\x1b\\\x1b]133;D;0\x07\x1b]133;A\x07PS> ");
        assert_eq!(parser.callbacks().exit, Some(3));
        let cb = parser.callbacks_mut();
        let exit = cb.exit.take();
        cb.prompt = false;
        assert_eq!(exit, Some(3));
        // The next prompt reports its own.
        parser.process(b"\x1b]133;D;0\x07\x1b]133;D;1\x07");
        assert_eq!(parser.callbacks().exit, Some(0));
    }

    /// `133;C` marks a command's start without being a prompt signal.
    #[test]
    fn command_start_mark() {
        let mut parser = vt100::Parser::new_with_callbacks(10, 40, 0, Callbacks::default());
        parser.process(b"ls\r\n\x1b]133;C\x07out\r\n");
        assert!(parser.callbacks().started.is_some());
        assert!(!parser.callbacks().prompt);
        parser.callbacks_mut().started = None;
        parser.process(b"\x1b]133;D;0\x07\x1b]133;A\x07");
        assert!(parser.callbacks().started.is_none() && parser.callbacks().prompt);
    }

    #[test]
    fn live_timer_text_and_schedule() {
        use std::time::Duration;
        let ms = Duration::from_millis;
        assert_eq!(live_timer(ms(1500), false), None);
        assert_eq!(live_timer(ms(7200), false).as_deref(), Some("◷ 0:07"));
        assert_eq!(live_timer(ms(7200), true).as_deref(), Some("◷"));
        assert_eq!(live_timer(ms(185_000), false).as_deref(), Some("◷ 3m"));
        assert_eq!(live_timer(ms(185_000), true).as_deref(), Some("◷ 3m"));
        assert_eq!(live_timer(ms(3_720_000), false).as_deref(), Some("◷ 1h02m"));
        // Redraws: when the timer appears, every second in the first minute, then each minute.
        assert_eq!(live_timer_next(ms(500), false), ms(1500));
        assert_eq!(live_timer_next(ms(7250), false), ms(755));
        assert_eq!(live_timer_next(ms(7250), true), ms(52_755));
        assert_eq!(live_timer_next(ms(185_000), false), ms(55_005));
        for t in [2000, 7250, 59_900, 60_000, 185_000, 4_000_000] {
            for battery in [false, true] {
                let next = live_timer_next(ms(t), battery);
                // The text never changes before the next redraw.
                assert_eq!(live_timer(ms(t), battery), live_timer(ms(t) + next - ms(10), battery), "{t} {battery}");
                assert!(next <= ms(60_005), "{t}");
            }
        }
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

    /// The copy of the scrollback is reused while neither output nor the size changed; new output or a
    /// resize reads the lines anew, and the history follows.
    #[test]
    fn scroll_text_is_reused_until_output_or_resize() {
        use std::sync::Arc;
        let mut p = vt100::Parser::new_with_callbacks(4, 20, 100, super::Callbacks::default());
        for i in 0..10 {
            p.process(format!("line {i}\r\n").as_bytes());
        }
        let a = super::read_scroll_text(&mut p, 1, None);
        assert!(a.history > 0 && a.lines.iter().any(|l| l.trim_end() == "line 0"));
        let b = super::read_scroll_text(&mut p, 1, Some(&a));
        assert!(Arc::ptr_eq(&a.lines, &b.lines), "same generation and size: the copy is reused");
        // Scrolling the view back does not change the text, so the copy is still reused.
        p.screen_mut().set_scrollback(3);
        let c = super::read_scroll_text(&mut p, 1, Some(&b));
        assert!(Arc::ptr_eq(&a.lines, &c.lines));
        p.screen_mut().set_scrollback(0);
        p.process(b"line 10\r\n");
        let d = super::read_scroll_text(&mut p, 2, Some(&c));
        assert!(!Arc::ptr_eq(&a.lines, &d.lines), "new output: fresh lines");
        assert!(d.lines.iter().any(|l| l.trim_end() == "line 10"));
        assert_eq!(d.history, a.history + 1);
        p.screen_mut().set_size(4, 10);
        let e = super::read_scroll_text(&mut p, 2, Some(&d));
        assert!(!Arc::ptr_eq(&d.lines, &e.lines), "resize: fresh lines");
        assert!(e.lines.iter().all(|l| l.chars().count() <= 10 || l.is_empty()));
    }
}
