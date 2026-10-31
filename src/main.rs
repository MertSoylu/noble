//! NOBLE entry point: terminal setup, input thread and the frame loop.

use std::io::{Write, stdout};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, PopKeyboardEnhancementFlags};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use noble::app::App;
use noble::config::Paths;
use noble::event::AppEvent;

const FRAME: Duration = Duration::from_millis(16);
/// Longest sleep without events: config and hook checks run on this interval.
const IDLE_TICK: Duration = Duration::from_secs(2);
/// `noble.log` larger than this is moved to `noble.log.1` at startup (`util::rotate_log`).
const LOG_ROTATE_SIZE: u64 = 256 * 1024;
/// The panic hook stops appending to `noble.log` at this size, so a run that panics in a loop cannot fill the disk.
const LOG_HARD_CAP: u64 = 1024 * 1024;

fn usage() {
    println!(
        "NOBLE {} — retro-futurist HUD terminal workspace\n\n\
         USAGE: noble [--config <path>] [--no-boot] [<dir>]\n       \
                noble update [--check]\n       \
                noble doctor [--config <path>]\n\n\
         ARGUMENTS:\n  \
           <dir>             open a shell tab in this folder (`noble .` for the current one)\n\n\
         COMMANDS:\n  \
           update            download and install the latest release\n  \
           doctor            print a report for bug reports (no terminal is opened)\n\n\
         OPTIONS:\n  \
           --config <path>   use an alternate config file\n  \
           --no-boot         skip the boot sequence\n  \
           --paths           print config and data locations\n  \
           -V, --version     print version\n  \
           -h, --help        print this help",
        env!("CARGO_PKG_VERSION")
    );
}

/// Set when `enter_terminal` pushed the keyboard flags (Unix only; Windows never sets it). `restore_terminal` swaps it
/// back to false, so a second restore (the panic hook, then `main`) pops nothing more.
static KEYBOARD_PUSHED: AtomicBool = AtomicBool::new(false);

fn restore_terminal() {
    let _ = disable_raw_mode();
    let mut out = stdout();
    #[cfg(not(windows))]
    let _ = execute!(out, crossterm::event::DisableBracketedPaste);
    let _ = execute!(out, crossterm::event::DisableFocusChange);
    // Popped before the alternate screen is left: the kitty stack is kept per screen and NOBLE pushed on the
    // alternate one. Windows pushes nothing (`enter_terminal`), so the flag stays false there and nothing is popped.
    if KEYBOARD_PUSHED.swap(false, Ordering::SeqCst) {
        let _ = execute!(out, PopKeyboardEnhancementFlags);
    }
    let _ = execute!(out, DisableMouseCapture, LeaveAlternateScreen, crossterm::cursor::Show);
    // The outer terminal: the progress report is cleared and the title it had before NOBLE comes back.
    let _ = out.write_all(noble::outer::exit_sequences(outer_progress()).as_bytes());
    let _ = out.flush();
}

/// Whether the outer terminal shows the progress report (`outer::progress_supported`), read from the environment.
fn outer_progress() -> bool {
    noble::outer::progress_supported(
        cfg!(windows),
        std::env::var_os("TERM_PROGRAM").is_some(),
        std::env::var_os("WT_SESSION").is_some(),
    )
}

/// Reported before the terminal is touched: without a home folder NOBLE has nowhere for its files.
fn no_folder_error() -> anyhow::Error {
    anyhow::anyhow!("no home folder for NOBLE's files: set NOBLE_HOME to the folder to use")
}

fn main() -> anyhow::Result<()> {
    // `noble hook <event>`: called from a Claude Code hook; it never touches the
    // terminal and always exits silently with success.
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.first().map(String::as_str) == Some("hook") {
        use std::io::{Read, Write};
        // `noble hook <event> [--agent <id>]`: an unknown agent id records nothing (and reads nothing).
        let Some(agent) = noble::hooks::agent_arg(argv.get(2..).unwrap_or(&[])) else { return Ok(()) };
        let mut input = String::new();
        let _ = std::io::stdin().take(1 << 20).read_to_string(&mut input);
        let event = argv.get(1).map(String::as_str).unwrap_or("");
        // An agent that reads a reply (Antigravity, Cursor) gets it first, outside NOBLE too. A failed write is
        // ignored.
        if let Some(reply) = noble::hooks::hook_reply(agent, event) {
            let _ = writeln!(std::io::stdout(), "{reply}");
        }
        // No folder for the files: nothing to record, and a hook never reports an error.
        let Some(paths) = Paths::resolve(None) else { return Ok(()) };
        let (instance, pane) = (std::env::var("NOBLE_INSTANCE").ok(), std::env::var("NOBLE_PANE").ok());
        noble::hooks::run_cli(agent, event, &input, &paths.data, instance.as_deref(), pane.as_deref());
        return Ok(());
    }
    if argv.first().map(String::as_str) == Some("update") {
        std::process::exit(noble::update::run_cli(&argv[1..]));
    }
    // `noble doctor`: a report for bug reports. Like `update`, it never opens the terminal.
    if argv.first().map(String::as_str) == Some("doctor") {
        std::process::exit(noble::doctor::run_cli(&argv[1..]));
    }
    let mut config_override: Option<PathBuf> = None;
    let mut no_boot = false;
    let mut start_arg: Option<String> = None;
    let mut args = argv.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                usage();
                return Ok(());
            }
            "-V" | "--version" => {
                let dev = if noble::util::is_dev_build() { " (dev build)" } else { "" };
                println!("noble {}{dev}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--config" => match args.next() {
                Some(p) => config_override = Some(PathBuf::from(p)),
                None => anyhow::bail!("--config needs a path"),
            },
            "--no-boot" => no_boot = true,
            "--paths" => {
                let p = Paths::resolve(config_override.clone()).ok_or_else(no_folder_error)?;
                println!("config: {}\ndata:   {}", p.config.display(), p.data.display());
                return Ok(());
            }
            other if other.starts_with('-') && other != "-" => anyhow::bail!("unknown argument '{other}' (try --help)"),
            // One folder to open a shell tab in (`noble .`).
            other if start_arg.is_none() => start_arg = Some(other.to_string()),
            _ => anyhow::bail!("only one folder can be given (try --help)"),
        }
    }
    // Checked before the terminal is touched, so the error is a plain message and a non-zero exit code.
    let start_dir = match start_arg {
        Some(arg) => {
            let cwd = std::env::current_dir().unwrap_or_default();
            match noble::util::resolve_start_dir(&arg, &cwd, dirs::home_dir().as_deref()) {
                Ok(dir) => Some(dir),
                Err(e) => {
                    eprintln!("noble: {e}");
                    std::process::exit(2);
                }
            }
        }
        None => None,
    };
    let paths = Paths::resolve(config_override).ok_or_else(no_folder_error)?;

    // Restore the terminal if the main thread panics; background panics are
    // written only to the log file so they never corrupt the screen.
    let default_hook = std::panic::take_hook();
    let log_file = paths.data_file("noble.log");
    // Still single-threaded here, before the hook is replaced: an oversized log from an earlier run moves aside.
    noble::util::rotate_log(&log_file, LOG_ROTATE_SIZE);
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().name() == Some("main") {
            restore_terminal();
            // Background commands end now if the lock is free; otherwise the unwinding reaches `run`.
            noble::ai::kill_background_children_if_free();
            default_hook(info);
        } else {
            // At the hard cap nothing more is appended: a run that panics in a loop cannot fill the disk.
            let thread = std::thread::current().name().unwrap_or("?").to_string();
            let line = format!("[{}] panic in thread '{thread}': {info}", chrono::Local::now().format("%F %T"));
            noble::util::append_log_line(&log_file, LOG_HARD_CAP, &line);
        }
    }));

    // After the hook, so a panic here restores too. A failed setup still gives the terminal back (raw mode and the
    // alternate screen may already be on).
    if let Err(e) = enter_terminal() {
        restore_terminal();
        return Err(e.into());
    }
    let result = run(paths, no_boot, start_dir);
    restore_terminal();
    result
}

/// Puts the terminal in NOBLE's mode: raw input, the alternate screen, mouse and focus reports, the title stack and
/// bracketed paste. An error leaves the caller to restore the terminal.
fn enter_terminal() -> std::io::Result<()> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(
        out,
        EnterAlternateScreen,
        EnableMouseCapture,
        // Focus reports (FocusGained / FocusLost) let notifications skip a window that is in front.
        crossterm::event::EnableFocusChange,
    )?;
    // Unix: the keyboard flags go on after the alternate screen is up and before the input thread reads the terminal
    // (the query below reads the terminal's answer itself). Windows: nothing is pushed, since the console API already
    // reports Shift and Ctrl with Enter and crossterm's enhancement flags are Unsupported there.
    #[cfg(not(windows))]
    {
        use crossterm::event::{KeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
        // `TERM=dumb` answers neither query, and crossterm would wait out its two-second timeout for an answer that
        // never comes, so it is not asked there.
        let dumb = std::env::var_os("TERM").is_some_and(|term| term == "dumb");
        // The query gets `CSI ? u` or the device attributes answer; a terminal that gives neither, or an error,
        // pushes nothing and startup goes on.
        let supported = !dumb && matches!(crossterm::terminal::supports_keyboard_enhancement(), Ok(true));
        // Disambiguated escape codes: Shift+Enter, Esc and the Ctrl and Alt keys arrive as CSI u sequences. Report
        // alternate keys: a shifted key also carries the character it types, so Alt+! is not read as Alt+1 with Shift.
        let flags =
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS;
        if supported && execute!(out, PushKeyboardEnhancementFlags(flags)).is_ok() {
            KEYBOARD_PUSHED.store(true, Ordering::SeqCst);
        }
    }
    // The title the terminal has now goes on its title stack; `restore_terminal` pops it. From the first frame on,
    // NOBLE sets the title itself (`App::outer_sequences`: "NOBLE", or "NOBLE · ◆ N" while agents wait).
    let _ = out.write_all(noble::outer::SAVE_TITLE.as_bytes());
    let _ = out.flush();
    // Unix: pastes arrive as one `Event::Paste`. crossterm's Windows console input has no
    // bracketed paste; there a paste arrives as a burst of key events, which the input thread turns back
    // into a paste (`event::coalesce_paste`).
    #[cfg(not(windows))]
    execute!(out, crossterm::event::EnableBracketedPaste)?;
    Ok(())
}

fn run(paths: Paths, no_boot: bool, start_dir: Option<PathBuf>) -> anyhow::Result<()> {
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    terminal.clear()?;
    let (tx, rx) = mpsc::channel::<AppEvent>();
    {
        let tx = tx.clone();
        std::thread::Builder::new().name("input".into()).spawn(move || {
            while let Ok(ev) = event::read() {
                // Windows: a paste arrives as a burst of key presses (no bracketed paste in the console
                // API); everything already queued is read with it and turned back into a paste
                // (`event::coalesce_paste`). Unix terminals send `Event::Paste` themselves.
                let batch = if cfg!(windows) {
                    let mut batch = vec![ev];
                    while batch.len() < 100_000 && event::poll(std::time::Duration::ZERO).unwrap_or(false) {
                        match event::read() {
                            Ok(ev) => batch.push(ev),
                            Err(_) => break,
                        }
                    }
                    noble::event::coalesce_paste(batch)
                } else {
                    vec![ev]
                };
                if batch.into_iter().any(|ev| tx.send(AppEvent::Input(ev)).is_err()) {
                    break;
                }
            }
        })?;
    }
    // SIGHUP/SIGTERM (Unix) and console close/logoff/shutdown (Windows) end the loop like a quit.
    noble::termination::install(tx.clone());
    // Terminal.app before macOS 26 has no truecolor: every color is sent from the 256-color palette there.
    let colors_256 = noble::util::lacks_truecolor();
    let size = terminal.size()?;
    let mut app = App::start(paths, tx, (size.width, size.height), !no_boot);
    if let Some(dir) = start_dir {
        // After the session restore, so the requested folder ends up focused.
        app.open_project_shell(dir);
    }

    // A panic on the main thread unwinds through the loop. The panic hook has restored the terminal already; here
    // the session is saved and the commands are ended, then the panic goes on, so the exit stays the same.
    let draw_error = match catch_unwind(AssertUnwindSafe(|| event_loop(&mut terminal, &mut app, &rx, colors_256))) {
        Ok(draw_error) => draw_error,
        Err(payload) => {
            let _ = catch_unwind(AssertUnwindSafe(|| app.shutdown_after_panic()));
            resume_unwind(payload);
        }
    };
    app.shutdown();
    draw_error.map_or(Ok(()), |e| Err(e.into()))
}

/// The frame loop. Returns the error of a failed draw (the terminal is gone), `None` on a normal quit.
///
/// Drawing happens in only two cases: when an event arrives that changed
/// something visible (`pending`, at most at FRAME rate) or when the moment
/// comes for something that changes over time (clock, animation, notification
/// timeout). Otherwise the process sleeps; an idle terminal draws once a minute.
fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
    rx: &mpsc::Receiver<AppEvent>,
    colors_256: bool,
) -> Option<std::io::Error> {
    let progress = outer_progress();
    let mut last_draw = Instant::now() - FRAME;
    let mut pending = true;
    let mut draw_error = None;
    loop {
        let timed = app.redraw_after().map(|d| Instant::now() + d);
        let wake = if pending { Some(last_draw + FRAME) } else { timed };
        let wait = wake.map(|w| w.saturating_duration_since(Instant::now())).unwrap_or(IDLE_TICK).min(IDLE_TICK);
        // Unsaved tab changes: wake up in time for the session save (nothing changed: no extra wake-up).
        let wait = app.session_save_due().map_or(wait, |d| wait.min(d));
        match rx.recv_timeout(wait) {
            Ok(ev) => pending |= app.handle(ev),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        // Drain the accumulated events but keep drawing disabled.
        let drain_start = Instant::now();
        while drain_start.elapsed() < Duration::from_millis(8) {
            match rx.try_recv() {
                Ok(ev) => pending |= app.handle(ev),
                Err(_) => break,
            }
        }
        app.tick();
        // The outer terminal's progress pause and window title follow the agents waiting for you; only a change is
        // written (`App::outer_sequences`).
        if let Some(seq) = app.outer_sequences(progress) {
            let mut out = stdout();
            let _ = out.write_all(seq.as_bytes());
            let _ = out.flush();
        }
        pending |= app.take_dirty();
        if app.quit {
            break;
        }
        let due = pending || timed.is_some_and(|t| Instant::now() >= t);
        if due && last_draw.elapsed() < FRAME {
            // Frame limit: draw on the next round.
            pending = true;
        } else if due {
            // A failed draw (the terminal is gone) still ends through `shutdown`, which saves the session.
            if let Err(e) = terminal.draw(|f| {
                noble::ui::draw(f, app);
                if colors_256 {
                    noble::ui::to_256_colors(f.buffer_mut());
                }
            }) {
                draw_error = Some(e);
                break;
            }
            // Desktop notification through the outer terminal; skipped while the window is known to be in front.
            if let Some(text) = app.outer_notice.take()
                && app.window_focused != Some(true)
                && noble::outer::supported(cfg!(windows), std::env::var_os("TERM_PROGRAM").is_some())
            {
                let mut out = stdout();
                let _ = out.write_all(noble::outer::sequences(&text).as_bytes());
                let _ = out.flush();
            }
            if std::mem::take(&mut app.outer_bell) {
                // Bell to the outer terminal: Windows Terminal marks the tab, the taskbar flashes.
                let mut out = stdout();
                let _ = out.write_all(b"\x07");
                let _ = out.flush();
            }
            last_draw = Instant::now();
            // A draw can ask for one more frame (`relink_after_layout` re-finds the link under the pointer after a
            // layout change): it is drawn at the frame limit, not only at the next event or idle tick.
            pending = app.take_dirty();
        }
    }
    draw_error
}
