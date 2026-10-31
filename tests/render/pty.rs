//! Real shells in a PTY: cwd tracking, command results, output throughput and hostile names.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use noble::app::{App, View};
use noble::event::AppEvent;
use noble::keys::Action;
use noble::term::layout::PaneId;
use noble::term::pane::ShellSpec;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::path::PathBuf;
use std::time::Duration;

use super::common::*;

/// A real PTY: the shell starts, a command runs, the output reaches the
/// emulator, splitting and closing update the tab tree correctly.
#[test]
fn real_terminal_session() {
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), Some("echo NOBLE_PTY_OK"), Some("test".into()));
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.view, View::Term(0));
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let mut seen = false;
    while std::time::Instant::now() < deadline {
        app.pump();
        let contents = {
            let id = app.tabs[0].focus;
            app.panes[&id].parser().screen().contents()
        };
        if contents.matches("NOBLE_PTY_OK").count() >= 2 || contents.lines().any(|l| l.trim() == "NOBLE_PTY_OK") {
            seen = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let text = render(&mut app, 110, 30);
    save("terminal-110x30", &text);
    assert!(seen, "shell output never arrived:\n{text}");

    app.run(Action::SplitRight);
    assert_eq!(app.tabs[0].panes().len(), 2);
    app.run(Action::SplitDown);
    assert_eq!(app.tabs[0].panes().len(), 3);
    std::thread::sleep(Duration::from_millis(800));
    app.pump();
    save("terminal-split-110x30", &render(&mut app, 110, 30));
    app.run(Action::Zoom);
    assert!(app.tabs[0].zoomed);
    save("terminal-zoom-110x30", &render(&mut app, 110, 30));
    app.run(Action::Zoom);
    app.run(Action::FocusLeft);
    app.run(Action::ClosePane);
    assert_eq!(app.tabs[0].panes().len(), 2);
    let snapshot = app.snapshot("t");
    assert_eq!(snapshot.pane_count(), 2);
    app.run(Action::CloseTab);
    // Two panes are left: closing the tab asks first.
    app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.tabs.is_empty());
    assert_eq!(app.view, View::Bridge);
    assert_eq!(app.pane_count(), 0);
}

/// Shell integration: after `cd` the pane knows the real directory (OSC 9;9 / OSC 7).
/// Every supported shell that is installed is checked (Git Bash on Windows).
#[test]
fn cwd_is_tracked_after_cd() {
    check_cwd_tracking("");
    let mut checked = vec!["default"];
    if cfg!(windows) {
        check_cwd_tracking("cmd.exe");
        checked.push("cmd.exe");
        // Windows PowerShell 5.1 is the default shell where pwsh is not installed; when pwsh took the
        // default above, 5.1 is checked on its own.
        if noble::util::which("pwsh").is_some()
            && let Some(ps) = noble::util::which("powershell")
        {
            check_cwd_tracking(&ps.display().to_string());
            checked.push("powershell");
        }
        let git_bash = std::path::Path::new(r"C:\Program Files\Git\bin\bash.exe");
        if git_bash.is_file() {
            check_cwd_tracking(&git_bash.display().to_string());
            checked.push("git bash");
        }
    } else {
        for shell in ["bash", "zsh", "fish", "pwsh"] {
            if let Some(path) = noble::util::which(shell) {
                check_cwd_tracking(&path.display().to_string());
                checked.push(shell);
            }
        }
    }
    assert_checked(&checked, "cwd_is_tracked_after_cd");
}

fn check_cwd_tracking(shell: &str) {
    let mut app = demo_app(110, 30);
    if !shell.is_empty() {
        let mut cfg = app.cfg.clone();
        cfg.terminal.shell = shell.into();
        app.apply_config(cfg);
    }
    let target = std::env::temp_dir().join(format!("noble-cwd-{}", std::process::id()));
    std::fs::create_dir_all(&target).unwrap();
    app.new_tab(dirs::home_dir().unwrap(), None, Some("cwd".into()));
    let id = app.tabs[0].focus;
    // Type only once the first prompt reported its directory: a slow cold start (pwsh
    // on Linux) would otherwise swallow the input typed before the shell was ready.
    let ready = std::time::Instant::now() + Duration::from_secs(30);
    while app.panes[&id].parser().callbacks().cwd.is_none() {
        assert!(std::time::Instant::now() < ready, "{shell}: no first prompt\n{}", render(&mut app, 110, 30));
        app.pump();
        std::thread::sleep(Duration::from_millis(100));
    }
    // pwsh prints its prompt before PSReadLine is ready (slowest on macOS): let it settle.
    wait_idle(&mut app, id);
    app.panes[&id].write(format!("cd \"{}\"\r", target.display()).as_bytes());
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let want = std::fs::canonicalize(&target).unwrap();
    loop {
        app.pump();
        let cwd = app.panes[&id].cwd();
        if std::fs::canonicalize(&cwd).ok() == Some(want.clone()) {
            break;
        }
        if std::time::Instant::now() > deadline {
            let text = render(&mut app, 110, 30);
            panic!("cwd not tracked, still {cwd:?}\n{text}");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // The tab is named after the folder the shell is in now; with `tab_follows_cwd` off
    // it keeps the name it was opened with.
    let folder = target.file_name().unwrap().to_string_lossy().into_owned();
    assert!(app.tab_title(0).starts_with(&folder), "{shell}: tab title {:?}", app.tab_title(0));
    app.cfg.terminal.tab_follows_cwd = false;
    assert!(app.tab_title(0).starts_with("cwd"), "{shell}: fixed tab title {:?}", app.tab_title(0));
    app.cfg.terminal.tab_follows_cwd = true;
    // A new tab (prefix t) opens in the same directory and is named after it.
    app.run(Action::NewTab);
    assert_eq!(app.tabs.len(), 2);
    assert!(app.tab_title(1).starts_with(&folder), "{shell}: new tab title {:?}", app.tab_title(1));
    app.run(Action::CloseTab);
    app.go_tab(1);
    // The split opens the new pane in the same directory.
    app.run(Action::SplitRight);
    let new = app.tabs[0].focus;
    assert_ne!(new, id);
    assert_eq!(std::fs::canonicalize(&app.panes[&new].start_cwd).unwrap(), want);
    app.run(Action::CloseTab);
    let _ = std::fs::remove_dir_all(&target);
}

/// Terminal compatibility: checks that common escape sequences render correctly
/// in a real pane (colors, styles, wide/combined characters, the alternate
/// screen, OSC 8 links).
#[test]
fn terminal_compatibility_basics() {
    use ratatui::style::{Color, Modifier};
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("compat".into()));
    let id = app.tabs[0].focus;
    wait_idle(&mut app, id);
    let feed = |app: &mut App, bytes: &[u8]| app.panes[&id].parser().process(bytes);
    feed(&mut app, b"\x1b[2J\x1b[H");
    feed(&mut app, b"\x1b[38;2;255;0;0mR\x1b[0m\x1b[38;5;196mI\x1b[0m\x1b[1;3;4mB\x1b[0m\r\n");
    feed(&mut app, "日本|e\u{301}|🙂|end\r\n".as_bytes());
    feed(&mut app, b"see \x1b]8;;https://example.com/docs\x1b\\docs\x1b]8;;\x1b\\ here\r\n");
    let mut term = Terminal::new(TestBackend::new(110, 30)).unwrap();
    term.draw(|f| noble::ui::draw(f, &mut app)).unwrap();
    let inner = find_hit_inner(&app).unwrap();
    let buf = term.backend().buffer().clone();
    let cell = |x: u16, y: u16| buf[(inner.x + x, inner.y + y)].clone();
    assert_eq!(cell(0, 0).fg, Color::Rgb(255, 0, 0), "truecolor");
    assert_eq!(cell(1, 0).fg, Color::Indexed(196), "256 colors");
    let m = cell(2, 0).modifier;
    assert!(m.contains(Modifier::BOLD) && m.contains(Modifier::ITALIC) && m.contains(Modifier::UNDERLINED));
    assert_eq!(cell(0, 1).symbol(), "日", "wide char");
    assert_eq!(cell(2, 1).symbol(), "本", "wide char takes two columns");
    assert_eq!(cell(4, 1).symbol(), "|");
    assert_eq!(cell(5, 1).symbol(), "e\u{301}", "combining mark stays with its base");
    assert_eq!(cell(7, 1).symbol(), "🙂");
    save("terminal-compat-110x30", &render(&mut app, 110, 30));
    // OSC 8: the link sits on top of the text.
    assert_eq!(app.panes[&id].hyperlink_at(2, 5).map(|h| h.0).as_deref(), Some("https://example.com/docs"));
    assert_eq!(app.panes[&id].hyperlink_at(2, 9), None);
    // Alternate screen: the old content returns when the fullscreen app exits.
    feed(&mut app, b"\x1b[?1049h\x1b[2J\x1b[HFULLSCREEN");
    assert!(render(&mut app, 110, 30).contains("FULLSCREEN"));
    feed(&mut app, b"\x1b[?1049l");
    let back = render(&mut app, 110, 30);
    assert!(!back.contains("FULLSCREEN") && back.contains("see docs here"), "{back}");
    app.run(Action::CloseTab);
}

/// Throughput under heavy output: while a 50 thousand line file is printed the
/// output keeps drawing. Prints duration and frame count (run with `--ignored --nocapture`).
const PERF_LINES: usize = 50_000;

#[test]
#[ignore]
fn heavy_output_throughput() {
    let dir = std::env::temp_dir().join(format!("noble-perf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lines.txt");
    let body: String =
        (0..PERF_LINES).map(|i| format!("line {i:05} lorem ipsum dolor sit amet consectetur\n")).collect();
    std::fs::write(&file, body + "PERF_DONE\n").unwrap();
    let mut app = demo_app(160, 45);
    let mut cfg = app.cfg.clone();
    if cfg!(windows) {
        cfg.terminal.shell = "cmd.exe".into();
    }
    app.apply_config(cfg);
    let cmd = if cfg!(windows) { format!("type \"{}\"", file.display()) } else { format!("cat '{}'", file.display()) };
    let started = std::time::Instant::now();
    app.new_tab(dir.clone(), Some(&cmd), Some("perf".into()));
    let id = app.tabs[0].focus;
    let mut frames = 0u32;
    let mut draw_time = Duration::ZERO;
    let mut term = Terminal::new(TestBackend::new(160, 45)).unwrap();
    loop {
        app.pump();
        let t = std::time::Instant::now();
        term.draw(|f| noble::ui::draw(f, &mut app)).unwrap();
        draw_time += t.elapsed();
        frames += 1;
        let (lines, _) = app.panes[&id].all_lines();
        if lines.iter().any(|l| l.trim() == "PERF_DONE") {
            break;
        }
        if started.elapsed() > Duration::from_secs(120) {
            let screen = app.panes[&id].parser().screen().contents();
            panic!(
                "output never finished after {} lines:
{screen}",
                lines.len()
            );
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    let total = started.elapsed();
    println!(
        "{PERF_LINES} lines in {:.2}s · {frames} frames · avg draw {:.2} ms · {:.0} lines/s",
        total.as_secs_f64(),
        draw_time.as_secs_f64() * 1000.0 / frames as f64,
        PERF_LINES as f64 / total.as_secs_f64()
    );
    app.run(Action::CloseTab);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Cost of a reflow: a full 100 thousand row scrollback at 200 columns is cut to 80 columns and back, and one column
/// narrower. Prints the times (run with `cargo test --release --test render reflow_cost -- --ignored --nocapture`).
///
/// Measured on the development machine (Windows 11, 12th gen i7-12700H, release build), 100 thousand rows of history
/// that is half 350 character lines (two rows at 200 columns) and half short ones:
///
/// | resize | first version | now |
/// | --- | --- | --- |
/// | 200 -> 80 columns | 330 ms | 103 ms |
/// | 80 -> 200 columns | 156 ms | 60 ms |
/// | 200 -> 199 columns | 145 ms | 44 ms |
///
/// "Now" moves a row that already fits over as it is, copies runs of ordinary cells in one go, leaves the padding of
/// history rows out, and does not rewrap the oldest history that the capacity cuts off anyway (`reflow::skippable`).
/// The default history is 5000 rows, which takes about a twentieth of that (a few milliseconds); only the
/// largest setting is near the 50 ms that a resize may take without being noticed, so reflow stays eager. The
/// budget below is generous so that a slow CI runner passes while a quadratic mistake does not.
#[test]
#[ignore]
fn reflow_cost_of_a_full_scrollback() {
    use noble::vt100::Parser;
    const HISTORY: usize = 100_000;
    let mut parser = Parser::new(noble::term::vt_size(50), noble::term::vt_size(200), HISTORY);
    // Half of the lines fill two rows (350 characters), the others one: a mix of wrapped and short lines.
    let long = "0123456789 ".repeat(32);
    let mut n = 0;
    while parser.screen().scrollback_len() < HISTORY {
        let line = if n % 2 == 0 { &long[..350] } else { "short line" };
        parser.process(
            format!(
                "{n:06} {line}
"
            )
            .as_bytes(),
        );
        n += 1;
    }
    let history = parser.screen().scrollback_len();
    let time = |what: &str, cols: u16, parser: &mut Parser| {
        let started = std::time::Instant::now();
        parser.screen_mut().set_size(noble::term::vt_size(50), noble::term::vt_size(cols));
        let took = started.elapsed();
        println!("{what}: {:.1} ms (history {} rows)", took.as_secs_f64() * 1000.0, parser.screen().scrollback_len());
        took
    };
    let narrow = time("200 -> 80 columns", 80, &mut parser);
    let wide = time("80 -> 200 columns", 200, &mut parser);
    let one = time("200 -> 199 columns", 199, &mut parser);
    println!("history before: {history} rows");
    let budget = Duration::from_millis(500);
    for took in [narrow, wide, one] {
        assert!(took < budget, "a reflow of a full scrollback took {took:?}");
    }
}

/// A quoted command (path with spaces) runs intact in cmd.exe: launchers are
/// invoked as `"C:\...\claude.exe"`.
#[test]
fn cmd_runs_quoted_commands() {
    // cmd.exe only exists on Windows; Unix shell quoting is covered by `pane::tests::unix_quoting`.
    if !cfg!(windows) {
        return;
    }
    let mut app = demo_app(110, 30);
    let mut cfg = app.cfg.clone();
    cfg.terminal.shell = "cmd.exe".into();
    app.apply_config(cfg);
    let dir = std::env::temp_dir().join(format!("noble quoted {}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("probe file.txt"), "QUOTED_PROBE_OK\r\n").unwrap();
    let cmd = format!("type \"{}\"", dir.join("probe file.txt").display());
    app.new_tab(dir.clone(), Some(&cmd), Some("quoted".into()));
    let id = app.tabs[0].focus;
    wait_for(&mut app, id, "QUOTED_PROBE_OK");
    app.run(Action::CloseTab);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file name full of shell syntax (ctrl+click opens it in a terminal editor) reaches the
/// program as text in every installed shell: the file is read and nothing in its name runs.
/// `command_with_arg` is what `App::edit_in_tab` builds its command with.
#[test]
fn hostile_file_name_is_not_executed() {
    let mut shells: Vec<String> = vec![String::new()];
    if cfg!(windows) {
        shells.push("cmd.exe".into());
        // Windows PowerShell 5.1 next to the default pwsh (it splits its command line differently).
        shells.extend(noble::util::which("powershell").map(|p| p.display().to_string()));
        let git_bash = std::path::Path::new(r"C:\Program Files\Git\bin\bash.exe");
        if git_bash.is_file() {
            shells.push(git_bash.display().to_string());
        }
    } else {
        for shell in ["bash", "zsh", "fish", "pwsh", "sh"] {
            shells.extend(noble::util::which(shell).map(|p| p.display().to_string()));
        }
    }
    for shell in &shells {
        check_hostile_file_name(shell);
    }
    assert_checked(&shells, "hostile_file_name_is_not_executed");
}

fn check_hostile_file_name(shell: &str) {
    let mut app = demo_app(110, 30);
    if !shell.is_empty() {
        let mut cfg = app.cfg.clone();
        cfg.terminal.shell = shell.into();
        app.apply_config(cfg);
    }
    let label = app.shell.label().to_lowercase();
    let dir = std::env::temp_dir().join(format!("noble-hostile-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Each part would create a `pw*` directory if the shell ran it: POSIX and PowerShell
    // `$(…)`, backticks, fish `(…)`, `;` after a closed quote (ASCII and typographic), cmd `&`.
    let mut name = String::from("h $(mkdir pw1) `mkdir pw2` (mkdir pw3) ’;mkdir pw4;’ &mkdir pw5& ^ ü 日本");
    if label != "cmd" {
        // cmd.exe refuses `%` and `!` (see `quote_arg`); every other shell takes them.
        name.push_str(" ';mkdir pw6;' %PATH% !x!");
    }
    if !cfg!(windows) {
        // Not allowed in Windows file names.
        name.push_str(r#" ";mkdir pw7;" \"#);
    }
    name.push_str(".txt");
    let file = dir.join(&name);
    let needle = format!("HOSTILE_OK_{}", std::process::id());
    std::fs::write(&file, format!("{needle}\n")).unwrap();
    // The editor is a native program, so it gets the argument as it is. PowerShell's own
    // cmdlets would read `\` as a path separator on Linux (even with -LiteralPath), so there
    // the native `cat` stands in for the editor; on Windows `cat` is a Get-Content alias.
    let reader = match label.as_str() {
        "pwsh" | "powershell" if cfg!(windows) => "Get-Content -LiteralPath",
        "cmd" => "type",
        _ => "cat",
    };
    let command = app.shell.command_with_arg(reader, &file.display().to_string()).expect("quotable");
    app.new_tab(dir.clone(), Some(&command), Some("hostile".into()));
    let id = app.tabs[0].focus;
    wait_for(&mut app, id, &needle);
    wait_idle(&mut app, id);
    let ran: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("pw"))
        .collect();
    assert!(ran.is_empty(), "{label}: the file name ran as a command, created {ran:?}");
    app.run(Action::CloseTab);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The pane title shows how the last command ended (`✓` / `✗ code` / cmd.exe's time alone) and a
/// live timer while a command runs; a narrow pane drops it before the title loses its folder.
#[test]
fn pane_title_shows_the_last_command_result() {
    use noble::battery::{Battery, PowerState};
    use noble::term::pane::CommandResult;
    let mut app = demo_app(110, 30);
    // Plugged in: the timer counts seconds in its first minute (on battery it shows the glyph alone).
    let mut sample = app.sensors.last.clone().unwrap();
    sample.battery = Some(Battery { percent: 80.0, state: PowerState::Full, secs_left: None, secs_to_full: None });
    app.handle(AppEvent::Sensors(Box::new(sample.clone())));
    app.new_tab(std::env::temp_dir(), None, Some("result".into()));
    let id = app.tabs[0].focus;
    let ms = Duration::from_millis;
    let title_row = |app: &mut App, w: u16, h: u16| {
        let text = render(app, w, h);
        // The pane's title row is the one with its close button.
        text.lines().find(|l| l.contains(" ✕ ")).unwrap_or_default().to_string()
    };
    let set = |app: &mut App, code: Option<i32>, took: Duration| {
        let p = app.panes.get_mut(&id).unwrap();
        p.last_result = Some(CommandResult { code, took, at: std::time::Instant::now() });
    };
    assert!(!title_row(&mut app, 110, 30).contains('✓'));
    set(&mut app, Some(0), ms(2460));
    let row = title_row(&mut app, 110, 30);
    save("pane-result-ok-110x30", &render(&mut app, 110, 30));
    assert!(row.contains(" ✓ 2.4s "), "{row}");
    set(&mut app, Some(1), ms(12_700));
    let row = title_row(&mut app, 110, 30);
    assert!(row.contains(" ✗ 1 · 12s "), "{row}");
    set(&mut app, None, ms(65_000));
    let row = title_row(&mut app, 110, 30);
    assert!(row.contains(" · 1m05s ") && !row.contains('✓') && !row.contains('✗'), "{row}");
    set(&mut app, Some(0), ms(2460));

    // A narrow pane keeps its buttons; the folder goes before the badge (`fit_title`).
    for (w, h) in [(60, 20), (40, 12), (30, 8)] {
        let text = render(&mut app, w, h);
        save(&format!("pane-result-{w}x{h}"), &text);
        let row = text.lines().find(|l| l.contains(" ✕ ├")).unwrap_or_default().to_string();
        assert!(row.contains('✓') || !row.contains(" · "), "{w}x{h}: the folder outranked the badge: {row}");
        assert_eq!(unicode_width::UnicodeWidthStr::width(row.as_str()), w as usize, "{row}");
    }

    // A command running: the old result is hidden, the live timer shows after 2 s.
    {
        let p = app.panes.get_mut(&id).unwrap();
        p.prompted = true;
        p.command_started = Some(std::time::Instant::now() - ms(500));
    }
    let row = title_row(&mut app, 110, 30);
    assert!(!row.contains('✓') && !row.contains('◷'), "{row}");
    let next = app.redraw_after().unwrap();
    assert!(next <= ms(1505), "the timer must appear on time: {next:?}");
    app.panes.get_mut(&id).unwrap().command_started = Some(std::time::Instant::now() - ms(7_300));
    let row = title_row(&mut app, 110, 30);
    save("pane-result-timer-110x30", &render(&mut app, 110, 30));
    assert!(row.contains(" ◷ 0:0"), "{row}");
    assert!(app.redraw_after().unwrap() <= ms(1005));
    // After the first minute it redraws once a minute, and not at all off screen.
    app.panes.get_mut(&id).unwrap().command_started = Some(std::time::Instant::now() - ms(125_000));
    assert!(title_row(&mut app, 110, 30).contains(" ◷ 2m "));
    // On battery the first minute shows the glyph alone and redraws only when the minute is up.
    sample.battery =
        Some(Battery { percent: 80.0, state: PowerState::Discharging, secs_left: None, secs_to_full: None });
    app.handle(AppEvent::Sensors(Box::new(sample)));
    app.panes.get_mut(&id).unwrap().command_started = Some(std::time::Instant::now() - ms(7_300));
    let row = title_row(&mut app, 110, 30);
    assert!(row.contains(" ◷ ") && !row.contains("0:0"), "{row}");
    // The timer's own wake-up is a minute away on battery. `redraw_after` is the earliest of several wake-ups, and
    // its clock part can be due within a second (at second 59), so the timer is checked on its own: a clock-based
    // check here would race the minute boundary.
    assert!(noble::term::pane::live_timer_next(ms(7_300), true) > ms(1005));
    assert!(noble::term::pane::live_timer_next(ms(7_300), false) <= ms(1005));
    app.view = View::Bridge;
    assert!(app.redraw_after().unwrap() <= ms(1005) || !app.live_clock());
    app.view = View::Term(0);
    app.run(Action::CloseTab);
}

/// A real command that fails, in every installed shell: the prompt reports its exit code (OSC
/// 133;D) and the pane title shows it; cmd.exe cannot report one, so only its time shows there.
/// A command that works shows `✓`; the first prompt of the shell records nothing.
#[test]
fn failing_command_shows_its_exit_code() {
    check_exit_code("");
    let mut checked = vec!["default"];
    if cfg!(windows) {
        check_exit_code("cmd.exe");
        checked.push("cmd.exe");
        let git_bash = std::path::Path::new(r"C:\Program Files\Git\bin\bash.exe");
        if git_bash.is_file() {
            check_exit_code(&git_bash.display().to_string());
            checked.push("git bash");
        }
    } else {
        for shell in ["bash", "zsh", "fish", "pwsh"] {
            if let Some(path) = noble::util::which(shell) {
                check_exit_code(&path.display().to_string());
                checked.push(shell);
            }
        }
    }
    assert_checked(&checked, "failing_command_shows_its_exit_code");
}

fn check_exit_code(shell: &str) {
    let mut app = demo_app(110, 30);
    if !shell.is_empty() {
        let mut cfg = app.cfg.clone();
        cfg.terminal.shell = shell.into();
        app.apply_config(cfg);
    }
    let kind = app.shell.label().to_lowercase();
    app.new_tab(std::env::temp_dir(), None, Some("exit".into()));
    let id = app.tabs[0].focus;
    pump_until(&mut app, &format!("{kind}: first prompt"), |a| a.panes[&id].prompted);
    // pwsh prints its prompt before PSReadLine is ready (slowest on macOS): let it settle.
    wait_idle(&mut app, id);
    assert_eq!(app.panes[&id].last_result, None, "{kind}: the first prompt is not a command");
    // Typed through NOBLE's own key handling, as a user does (that is what starts the clock).
    let type_line = |app: &mut App, line: &str| {
        for c in line.chars() {
            app.handle(AppEvent::Input(crossterm::event::Event::Key(KeyEvent::new(
                KeyCode::Char(c),
                KeyModifiers::NONE,
            ))));
        }
        app.handle(AppEvent::Input(crossterm::event::Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))));
    };
    let (failing, want) = match kind.as_str() {
        "cmd" => ("cmd /c exit 3", None),
        "pwsh" | "powershell" if cfg!(windows) => ("cmd /c exit 3", Some(3)),
        _ => ("sh -c 'exit 3'", Some(3)),
    };
    type_line(&mut app, failing);
    pump_until(&mut app, &format!("{kind}: result of `{failing}`"), |a| a.panes[&id].last_result.is_some());
    let result = app.panes[&id].last_result.unwrap();
    assert_eq!(result.code, want, "{kind}: exit code of `{failing}`");
    let text = render(&mut app, 110, 30);
    let tag = kind.replace(' ', "-");
    save(&format!("pane-exit-code-{tag}"), &text);
    let row = text.lines().find(|l| l.contains(" ✕ ")).unwrap_or_default();
    let badge = if want.is_some() { " ✗ 3 · " } else { " · " };
    assert!(row.contains(badge), "{kind}: {row}");
    // Shells that mark a command's start (OSC 133;C) are timed by that mark: zsh, fish, bash 4.4+
    // (`PS0`) and PowerShell with PSReadLine. cmd.exe and bash 3.2 cannot; they fall back to Enter.
    let marks = app.panes[&id].start_marks;
    let expect_marks = match kind.as_str() {
        "zsh" | "fish" => Some(true),
        "cmd" => Some(false),
        "bash" => Some(bash_has_ps0(shell)),
        // PSReadLine is always there in an interactive Windows PowerShell / pwsh on Windows; checked
        // for real on Windows only.
        "pwsh" | "powershell" if cfg!(windows) => Some(true),
        _ => None,
    };
    if let Some(expect) = expect_marks {
        assert_eq!(marks, expect, "{kind}: command start marks");
    }

    // A command that works; the next prompt replaces the result.
    wait_idle(&mut app, id);
    type_line(&mut app, "cd .");
    pump_until(&mut app, &format!("{kind}: result of `cd .`"), |a| {
        a.panes[&id].last_result.is_some_and(|r| r.code != want || r.at > result.at)
    });
    let ok = app.panes[&id].last_result.unwrap();
    assert_eq!(ok.code, want.map(|_| 0), "{kind}: exit code of `cd .`");
    // An Enter on an empty line brings the prompt back but keeps the result.
    wait_idle(&mut app, id);
    type_line(&mut app, "");
    wait_idle(&mut app, id);
    assert_eq!(app.panes[&id].last_result, Some(ok), "{kind}: an empty line replaced the result");
    if marks {
        // A line typed and cleared again runs nothing: with start marks that is known for sure.
        app.handle(AppEvent::Input(crossterm::event::Event::Key(KeyEvent::new(
            KeyCode::Char('x'),
            KeyModifiers::NONE,
        ))));
        app.handle(AppEvent::Input(crossterm::event::Event::Key(KeyEvent::new(
            KeyCode::Backspace,
            KeyModifiers::NONE,
        ))));
        wait_idle(&mut app, id);
        type_line(&mut app, "");
        wait_idle(&mut app, id);
        assert_eq!(app.panes[&id].last_result, Some(ok), "{kind}: a cleared line replaced the result");
        assert!(app.panes[&id].command_started.is_none(), "{kind}: the clock runs after a cleared line");
    }
    app.run(Action::CloseTab);
}

/// Without start marks every Enter restarts the command clock (so a continuation line's typing time
/// is not counted) and a typed line counts as a command. Once the shell marks command starts
/// (OSC 133;C) only the mark starts the clock and records a result: an Enter on an empty, cleared or
/// continuation line runs nothing and shows no timer. The marks are fed by hand into a pane whose
/// command sleeps, so the shell itself adds none.
#[test]
fn command_start_marks_decide_what_ran() {
    let mut app = demo_app(110, 30);
    let wait = match app.shell.label().to_lowercase().as_str() {
        "cmd" => "ping -n 60 127.0.0.1 >nul",
        "pwsh" | "powershell" => "Start-Sleep 60",
        _ => "sleep 60",
    };
    app.new_tab(std::env::temp_dir(), Some(wait), None);
    let id = app.tabs[0].focus;
    std::thread::sleep(Duration::from_millis(800));
    app.pump();
    app.panes.get_mut(&id).unwrap().prompted = true;
    let key = |app: &mut App, code: KeyCode| {
        app.handle(AppEvent::Input(crossterm::event::Event::Key(KeyEvent::new(code, KeyModifiers::NONE))));
    };
    let feed = |app: &mut App, bytes: &[u8]| {
        let pane = &app.panes[&id];
        pane.parser().process(bytes);
        pane.dirty.store(true, std::sync::atomic::Ordering::Release);
        app.handle(AppEvent::PtyOutput);
    };
    // No marks: the last Enter starts the clock, a typed line is a command.
    key(&mut app, KeyCode::Char('x'));
    key(&mut app, KeyCode::Enter);
    let first = app.panes[&id].command_started.expect("Enter starts the clock");
    std::thread::sleep(Duration::from_millis(200));
    key(&mut app, KeyCode::Enter);
    let restarted = app.panes[&id].command_started.expect("a later Enter restarts the clock");
    assert!(restarted > first, "a later Enter restarts the clock");
    // The result is counted from the last Enter, so it is shorter than the gap the test made between the two.
    let gap = restarted.duration_since(first);
    feed(&mut app, b"\x1b]133;D;1\x07");
    let guessed = app.panes[&id].last_result.expect("a typed line counts");
    assert_eq!(guessed.code, Some(1));
    assert!(guessed.took < gap, "counted from the first Enter: {:?} against a gap of {gap:?}", guessed.took);

    // The shell marks a start: the mark starts the clock, later Enters (input for the command) do not.
    key(&mut app, KeyCode::Char('x'));
    key(&mut app, KeyCode::Enter);
    feed(&mut app, b"\x1b]133;C\x07");
    let (marks, ran, started) = {
        let p = &app.panes[&id];
        (p.start_marks, p.ran, p.command_started)
    };
    assert!(marks && ran);
    let started = started.expect("the mark starts the clock");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.panes[&id].command_started, Some(started));
    feed(&mut app, b"\x1b]133;D;4\x07");
    let result = app.panes[&id].last_result.unwrap();
    assert_eq!(result.code, Some(4));
    assert!(!app.panes[&id].ran && app.panes[&id].command_started.is_none());

    // No mark before the prompt: a cleared line (or a continuation line) ran nothing.
    key(&mut app, KeyCode::Char('x'));
    key(&mut app, KeyCode::Backspace);
    key(&mut app, KeyCode::Enter);
    assert!(app.panes[&id].command_started.is_none() && app.panes[&id].running_for().is_none());
    feed(&mut app, b"\x1b]133;D;4\x07");
    assert_eq!(app.panes[&id].last_result, Some(result), "a line that ran nothing replaced the result");
    let row = render(&mut app, 110, 30).lines().find(|l| l.contains(" ✕ ")).unwrap_or_default().to_string();
    assert!(row.contains(" ✗ 4 · ") && !row.contains('◷'), "{row}");
    app.run(Action::CloseTab);
}

/// Input never waits for the program: a program that runs and never reads its input takes a 200 KB paste at
/// once, and the UI keeps answering keys and drawing while it sits there (the paste used to block the main
/// thread until the program exited).
#[test]
fn paste_does_not_wait_for_a_program_that_does_not_read() {
    let mut app = demo_app(110, 30);
    let mut cfg = app.cfg.clone();
    // Long enough to outlive the test on a slow runner; the pane is also checked to be alive at the end, so a program
    // that ended early cannot make the paste look refused-free by accident.
    let (program, args) = if cfg!(windows) { ("timeout", vec!["/t", "60", "/nobreak"]) } else { ("sleep", vec!["60"]) };
    cfg.terminal.shell = program.into();
    cfg.terminal.shell_args = args.into_iter().map(String::from).collect();
    app.apply_config(cfg);
    app.new_tab(std::env::temp_dir(), None, Some("stuck".into()));
    let id = app.tabs[0].focus;
    let body: String = (0..2500).map(|_| format!("{}\r", "a".repeat(79))).collect();
    assert!(body.len() > 190_000);
    let started = std::time::Instant::now();
    let queued = app.panes.get_mut(&id).unwrap().paste(&body);
    let took = started.elapsed();
    assert!(queued, "the paste was refused: the program exited, or too much was waiting for it");
    assert!(took < Duration::from_secs(1), "the paste waited {took:?} for the program");
    let started = std::time::Instant::now();
    app.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    app.pump();
    let text = render(&mut app, 110, 30);
    assert!(started.elapsed() < Duration::from_secs(1), "the UI stalled after the paste");
    assert!(text.contains(" ✕ "), "the pane still draws:\n{text}");
    // The program is still there: the paste was queued for a program that does not read, not for one that exited.
    assert!(!app.panes[&id].has_exited(), "the program exited during the test");
    app.run(Action::CloseTab);
    assert!(app.tabs.is_empty());
}

/// Terminal queries are answered and the answer reaches the program's input (the reader thread queues it to
/// the pane's writer). Needs POSIX `sh`, `stty`, `dd` and `od` to read the answer back, so it runs on Unix; the
/// Windows panes answer the same query through the same reader thread, but there is no such tool there. The queue
/// step itself is checked on every OS by `pane::tests::query_answers_are_queued_to_the_writer`.
#[test]
fn cursor_position_answer_reaches_the_program() {
    if cfg!(windows) {
        return;
    }
    let mut app = demo_app(110, 30);
    let mut cfg = app.cfg.clone();
    cfg.terminal.shell = "sh".into();
    // Raw mode, ask for the cursor position, read the 6-byte answer (`ESC [ 1 ; 1 R`) and print its hex.
    cfg.terminal.shell_args = vec![
        "-c".into(),
        r"stty raw -echo; printf '\033[6n'; dd bs=1 count=6 2>/dev/null | od -An -tx1 | tr -d ' \n'; sleep 3".into(),
    ];
    app.apply_config(cfg);
    app.new_tab(std::env::temp_dir(), None, Some("dsr".into()));
    let id = app.tabs[0].focus;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        app.pump();
        let screen = app.panes[&id].parser().screen().contents();
        if screen.contains("1b5b313b3152") {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "no answer came back:\n{screen}");
        std::thread::sleep(Duration::from_millis(50));
    }
    app.run(Action::CloseTab);
    assert!(app.tabs.is_empty());
}

/// Identity and mode queries are answered and the answers reach the program: XTVERSION (`CSI > q`) names NOBLE and
/// its version, and DECRQM (`CSI ? Ps $ p`) reports a mode the program set (1) and a mode NOBLE does not know (0).
/// The program reads each answer back by its byte count, with POSIX `sh`, `stty`, `dd` and `od`, so it runs on Unix.
/// Windows panes get the same answers through the same reader thread, but ConPTY hands a Windows program the answer
/// as keyboard input, so the bytes it reads are not the bytes NOBLE wrote: the answers are checked by the unit tests in
/// `term/osc.rs`, and the queue step on every OS by `pane::tests::query_answers_are_queued_to_the_writer`.
#[test]
fn identity_and_mode_answers_reach_the_program() {
    if cfg!(windows) {
        return;
    }
    let xtversion = format!("\x1bP>|NOBLE {}\x1b\\", env!("CARGO_PKG_VERSION"));
    // (the query, the answer it must get). The program sets 2004 and 1004 first, then asks for them.
    let queries: [(&str, &str); 4] = [
        ("\x1b[?2004h\x1b[?2004$p", "\x1b[?2004;1$y"),
        ("\x1b[>q", xtversion.as_str()),
        ("\x1b[?1004h\x1b[?1004$p", "\x1b[?1004;1$y"),
        ("\x1b[?2031h\x1b[?2031$p", "\x1b[?2031;1$y"),
    ];
    // Raw mode; each step prints the query (`\033` for ESC), reads the answer's bytes back and prints them as hex.
    let mut script = String::from("stty raw -echo;");
    for (query, answer) in queries {
        script += &format!(
            "printf '{}';dd bs=1 count={} 2>/dev/null | od -An -tx1 | tr -d ' \n';printf ' ';",
            query.replace('\x1b', "\\033"),
            answer.len()
        );
    }
    let mut app = demo_app(110, 30);
    let mut cfg = app.cfg.clone();
    cfg.terminal.shell = "sh".into();
    cfg.terminal.shell_args = vec!["-c".into(), format!("{script}sleep 3")];
    app.apply_config(cfg);
    app.new_tab(std::env::temp_dir(), None, Some("identity".into()));
    let id = app.tabs[0].focus;
    let hex = |s: &str| s.bytes().map(|b| format!("{b:02x}")).collect::<String>();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        app.pump();
        let Some(pane) = app.panes.get(&id) else { panic!("the program's pane closed before the answers came back") };
        let screen = pane.parser().screen().contents();
        if queries.iter().all(|(_, answer)| screen.contains(&hex(answer))) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "not every answer came back:\n{screen}");
        std::thread::sleep(Duration::from_millis(50));
    }
    app.run(Action::CloseTab);
    assert!(app.tabs.is_empty());
}

/// Runs `script` (POSIX `sh`) in a new tab until `want` shows on its screen. With `shift_enter`, Shift+Enter is typed
/// into the pane as soon as the program has printed `READY`, which it does after its keyboard setup, so the key
/// comes after the flags it pushed and after its raw mode.
fn key_program(script: &str, shift_enter: bool, want: &str) {
    let mut app = demo_app(110, 30);
    let mut cfg = app.cfg.clone();
    cfg.terminal.shell = "sh".into();
    cfg.terminal.shell_args = vec!["-c".into(), script.into()];
    app.apply_config(cfg);
    app.new_tab(std::env::temp_dir(), None, Some("keys".into()));
    let id = app.tabs[0].focus;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut sent = false;
    loop {
        app.pump();
        let screen = app.panes[&id].parser().screen().contents();
        if shift_enter && !sent && screen.contains("READY") {
            sent = true;
            app.handle(AppEvent::Input(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT))));
        }
        if screen.contains(want) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{want} did not come back:\n{screen}");
        std::thread::sleep(Duration::from_millis(50));
    }
    app.run(Action::CloseTab);
    assert!(app.tabs.is_empty());
}

/// Kitty keyboard protocol, flag 1, through a real PTY and the key path a user types on: a program that pushed
/// `CSI > 1 u` gets Shift+Enter as `CSI 13 ; 2 u` (7 bytes), a program that pushed nothing gets the plain carriage
/// return, and a program that asks for the flags (`CSI ? u`) reads the reply `CSI ? 0 u` (5 bytes). The programs read
/// their bytes back with POSIX `sh`, `stty`, `dd` and `od`, so this runs on Unix. On Windows ConPTY hands a program
/// that reads console input records no CSI u at all, so the Windows behavior is covered by its own tests.
#[test]
fn kitty_keyboard_flags_reach_the_program() {
    if cfg!(windows) {
        return;
    }
    let hex = |s: &str| s.bytes().map(|b| format!("{b:02x}")).collect::<String>();
    // Each program prints READY once its raw mode and its flags are set, then reads the key's bytes back.
    let pushed = "stty raw -echo; printf '\\033[>1u'; printf READY; dd bs=1 count=7 2>/dev/null | od -An -tx1 | tr -d ' \n'; sleep 3";
    key_program(pushed, true, &format!("READY{}", hex("\x1b[13;2u")));
    let plain = "stty raw -echo; printf READY; dd bs=1 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'; sleep 3";
    key_program(plain, true, &format!("READY{}", hex("\r")));
    // The query is answered with the flags in effect: none pushed, so 0.
    let asks = "stty raw -echo; printf '\\033[?u'; dd bs=1 count=5 2>/dev/null | od -An -tx1 | tr -d ' \n'; sleep 3";
    key_program(asks, false, &hex("\x1b[?0u"));
}

/// ConPTY's win32-input-mode (`CSI ? 9001 h`), through a real ConPTY: the pane has the mode after startup, and a program
/// that reads console input records (Windows PowerShell 5.1, `[Console]::ReadKey`) gets Shift+Enter, Ctrl+Enter and Esc
/// the way the console reports them (`term::input::win32_key`), from keys typed through the App. The program is
/// PowerShell, which every Windows runner has. On Unix there is no ConPTY, so the test returns early there.
#[test]
fn win32_input_keys_reach_a_console_reader() {
    if !cfg!(windows) {
        return;
    }
    let script = "Write-Output READY; 1..3 | ForEach-Object { $k = [Console]::ReadKey($true); \
                  Write-Output ('K:' + $k.Key + ':' + $k.Modifiers + ':' + [int]$k.KeyChar) }; Start-Sleep -Seconds 60";
    let mut app = demo_app(110, 30);
    let mut cfg = app.cfg.clone();
    cfg.terminal.shell = "powershell.exe".into();
    cfg.terminal.shell_args = vec!["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), script.into()];
    app.apply_config(cfg);
    app.new_tab(std::env::temp_dir(), None, Some("win32".into()));
    let id = app.tabs[0].focus;
    pump_until(&mut app, "the console reader is ready", |app| screen_text(app, id).contains("READY"));
    // ConPTY turned the mode on before the program printed anything: the pane's own accessor says so.
    assert!(app.panes[&id].parser().callbacks().win32_input(), "{}", screen_text(&app, id));
    // Each key is sent once the previous one was read, so the lines come out in the order of the keys.
    let keys = [
        (KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT), "K:Enter:Shift:13"),
        (KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL), "K:Enter:Control:10"),
        (KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), "K:Escape:0:27"),
    ];
    for (key, line) in keys {
        app.handle(AppEvent::Input(Event::Key(key)));
        pump_until(&mut app, line, |app| screen_text(app, id).contains(line));
    }
    app.run(Action::CloseTab);
    assert!(app.tabs.is_empty());
}

/// The kitty keyboard flags through ConPTY, on Windows: a program pushes `CSI > 1 u` and the pane's flags become 1
/// (ConPTY passes the push through to NOBLE), then it pops with `CSI < u` and the flags are 0 again. The program holds
/// the pushed state until the test has seen flag 1 and sends a key (`[Console]::ReadKey` returns it), so the check does
/// not depend on how fast the runner is. The program is PowerShell, which writes the sequences itself. Windows only: on
/// Unix the same flags are checked by `kitty_keyboard_flags_reach_the_program`, and there is no ConPTY to pass them
/// through.
#[test]
fn kitty_flags_pushed_through_conpty_reach_the_pane() {
    if !cfg!(windows) {
        return;
    }
    let script = "[Console]::Write([char]27 + '[>1u'); Write-Output READY; [Console]::ReadKey($true) | Out-Null; \
                  [Console]::Write([char]27 + '[<u'); Write-Output POPPED; Start-Sleep -Seconds 60";
    let mut app = demo_app(110, 30);
    let mut cfg = app.cfg.clone();
    cfg.terminal.shell = "powershell.exe".into();
    cfg.terminal.shell_args = vec!["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), script.into()];
    app.apply_config(cfg);
    app.new_tab(std::env::temp_dir(), None, Some("kitty-conpty".into()));
    let id = app.tabs[0].focus;
    let flags = |app: &App| noble::term::pane::key_mode(&app.panes[&id].parser()).kitty;
    pump_until(&mut app, "the program is ready", |app| screen_text(app, id).contains("READY"));
    pump_until(&mut app, "the pushed flags reach the pane", |app| flags(app) == 1);
    // The program is waiting in ReadKey: this key lets it pop the flags.
    app.handle(AppEvent::Input(Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))));
    pump_until(&mut app, "the popped flags leave the pane", |app| {
        flags(app) == 0 && screen_text(app, id).contains("POPPED")
    });
    app.run(Action::CloseTab);
    assert!(app.tabs.is_empty());
}

/// Focus reports (DECSET 1004): a program that set the mode gets `CSI O` when its pane loses the focus and `CSI I`
/// when it gets it back, for a focus move between panes and for NOBLE's own window. A repeated event sends nothing.
/// The programs are this test binary again: the `#[ignore]`d `focus_child_*` tests below print each focus event
/// the reporting one reads (crossterm reads `CSI I` / `CSI O` as focus events on Unix; on Windows ConPTY passes
/// them to the child as console focus events), so the pane's screen shows what arrived.
#[test]
fn focus_reports_follow_the_focused_pane() {
    let exe = std::env::current_exe().expect("the test binary").display().to_string();
    let child = |test: &str| ShellSpec::new(exe.clone(), vec!["--ignored".into(), "--nocapture".into(), test.into()]);
    let mut app = demo_app(110, 30);
    app.shell = child("focus_child_reports");
    app.run(Action::NewTab);
    let reporting = app.tabs[0].focus;
    // The mode is set before the focus moves away from this pane, so every report below is a real change.
    pump_until(&mut app, "the reporting program is ready", |app| screen_text(app, reporting).contains("READY"));
    app.shell = child("focus_child_plain");
    app.run(Action::SplitRight);
    let plain = app.tabs[0].focus;
    pump_until(&mut app, "the plain program is ready", |app| screen_text(app, plain).contains("READY"));

    // The focus moves to the new pane: the reporting pane hears that it lost it, the plain one has no mode.
    pump_until(&mut app, "the reporting pane hears the loss", |app| focus_events(app, reporting, "focus-out") == 1);
    app.run(Action::FocusLeft);
    assert_eq!(app.tabs[0].focus, reporting);
    pump_until(&mut app, "the reporting pane hears the gain", |app| focus_events(app, reporting, "focus-in") == 1);
    // NOBLE's window goes to the background and comes back; a second gain in a row is no change.
    app.handle(AppEvent::Input(Event::FocusLost));
    pump_until(&mut app, "the window loss reaches the pane", |app| focus_events(app, reporting, "focus-out") == 2);
    app.handle(AppEvent::Input(Event::FocusGained));
    app.handle(AppEvent::Input(Event::FocusGained));
    pump_until(&mut app, "the window gain reaches the pane", |app| focus_events(app, reporting, "focus-in") == 2);

    // Nothing more arrives, and the pane without the mode was never told anything.
    std::thread::sleep(Duration::from_millis(500));
    app.pump();
    assert_eq!(focus_events(&app, reporting, "focus-out"), 2, "{}", screen_text(&app, reporting));
    assert_eq!(focus_events(&app, reporting, "focus-in"), 2, "{}", screen_text(&app, reporting));
    // The pane whose program never set the mode was sent nothing. On Windows ConPTY turns the mode on in every pane by
    // itself (its host asks for focus events), so the plain pane has it there too, and the reports reach it only as
    // console focus events that a program reading no input never sees. On Unix the pane's tty echoes every byte
    // NOBLE writes to it (`^[[I` / `^[[O`), so an echo would show a report.
    #[cfg(not(windows))]
    {
        assert!(!app.panes[&plain].wants_focus_reports());
        let echoed = screen_text(&app, plain);
        assert!(!echoed.contains("^[[I") && !echoed.contains("^[[O"), "{echoed}");
    }
    app.run(Action::CloseTab);
    app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.tabs.is_empty());
}

/// The screen of a pane, as text.
fn screen_text(app: &App, id: PaneId) -> String {
    app.panes[&id].parser().screen().contents()
}

/// How many times `word` (a focus event the child printed) is on a pane's screen.
fn focus_events(app: &App, id: PaneId, word: &str) -> usize {
    screen_text(app, id).matches(word).count()
}

/// The reporting program of `focus_reports_follow_the_focused_pane`: sets focus reports, then prints each focus event.
/// It runs only as that test's child (`--ignored`).
#[test]
#[ignore = "a child program of focus_reports_follow_the_focused_pane"]
fn focus_child_reports() {
    focus_child(true);
}

/// The program without the mode in `focus_reports_follow_the_focused_pane`: it prints `READY` and reads no input, so
/// the line discipline of its tty echoes what NOBLE writes to it on Unix.
#[test]
#[ignore = "a child program of focus_reports_follow_the_focused_pane"]
fn focus_child_plain() {
    println!("READY");
    std::thread::sleep(Duration::from_secs(60));
}

/// Raw mode, `READY` once the mode is set (when `reports`), then one line per focus event until `q` or a minute.
fn focus_child(reports: bool) {
    use std::io::Write;
    crossterm::terminal::enable_raw_mode().expect("raw mode");
    let mut out = std::io::stdout();
    if reports {
        write!(out, "\x1b[?1004h").expect("the pane");
    }
    write!(out, "READY\r\n").expect("the pane");
    out.flush().expect("the pane");
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        if !crossterm::event::poll(Duration::from_millis(50)).unwrap_or(false) {
            continue;
        }
        let word = match crossterm::event::read() {
            Ok(Event::FocusGained) => "focus-in",
            Ok(Event::FocusLost) => "focus-out",
            Ok(Event::Key(k)) if k.code == KeyCode::Char('q') => break,
            Ok(_) => continue,
            Err(_) => break,
        };
        write!(out, "{word}\r\n").expect("the pane");
        out.flush().expect("the pane");
    }
    let _ = crossterm::terminal::disable_raw_mode();
}

/// Synchronized output (DECSET 2026): a program writes a frame inside `CSI ? 2026 h` … `l` in several chunks with
/// pauses between them. Every frame NOBLE draws meanwhile is the previous complete one, never a half-written one,
/// and the finished frame shows when the block ends. The parser itself does hold the half frame (it is fed as it
/// arrives), so the check is on what the pane draws. The program is this test binary again (`sync_child_*`).
#[test]
fn synchronized_frame_is_never_drawn_half_written() {
    let exe = std::env::current_exe().expect("the test binary").display().to_string();
    let mut app = demo_app(110, 30);
    app.shell = ShellSpec::new(exe, vec!["--ignored".into(), "--nocapture".into(), "sync_child_frame".into()]);
    app.run(Action::NewTab);
    let id = app.tabs[0].focus;
    // Long enough for the slowest runner: the pauses in the program add up to well under it.
    app.panes[&id].parser().callbacks_mut().set_sync_timeout(Some(Duration::from_secs(10)));
    // The first frame is drawn (and kept) before the block starts: the program waits for a second after printing it.
    pump_until(&mut app, "the first frame is drawn", |app| screen_text(app, id).contains("BASE-FRAME"));
    let mut shown = render(&mut app, 110, 30);
    assert!(shown.contains("BASE-FRAME"), "{shown}");
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let mut during_block = 0;
    while !shown.contains("FINAL-FRAME") {
        assert!(!shown.contains("PARTIAL"), "a half-written frame was drawn:\n{shown}");
        assert!(shown.contains("BASE-FRAME"), "the pane went blank or changed before the frame was complete:\n{shown}");
        // The emulator already holds the half frame: that is the case the drawing has to cover.
        if screen_text(&app, id).contains("PARTIAL") {
            during_block += 1;
        }
        assert!(std::time::Instant::now() < deadline, "the frame never completed:\n{shown}");
        app.pump();
        app.tick();
        std::thread::sleep(Duration::from_millis(5));
        shown = render(&mut app, 110, 30);
    }
    assert!(during_block > 0, "the test never saw the emulator holding a half frame");
    assert!(!shown.contains("PARTIAL"), "{shown}");
    app.run(Action::CloseTab);
    app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
}

/// A program that opens a block and never closes it is not frozen: after the timeout (about 150 ms) its frame is drawn
/// and the held-back output event is sent, with no more bytes coming.
#[test]
fn forgotten_synchronized_block_is_released_after_the_timeout() {
    let exe = std::env::current_exe().expect("the test binary").display().to_string();
    let mut app = demo_app(110, 30);
    app.shell = ShellSpec::new(exe, vec!["--ignored".into(), "--nocapture".into(), "sync_child_forgotten".into()]);
    app.run(Action::NewTab);
    let id = app.tabs[0].focus;
    // A timeout long enough to see the frame held (the default one is checked by the unit tests).
    let timeout = Duration::from_millis(1500);
    app.panes[&id].parser().callbacks_mut().set_sync_timeout(Some(timeout));
    pump_until(&mut app, "the first frame is drawn", |app| screen_text(app, id).contains("BASE-FRAME"));
    let shown = render(&mut app, 110, 30);
    assert!(shown.contains("BASE-FRAME"), "{shown}");
    // While the block is open the frame is not drawn; after the timeout it is, with no further output.
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while app.panes[&id].sync_wake().is_none() {
        app.pump();
        assert!(std::time::Instant::now() < deadline, "the block never opened");
        std::thread::sleep(Duration::from_millis(2));
    }
    let opened = std::time::Instant::now();
    loop {
        app.pump();
        app.tick();
        let shown = render(&mut app, 110, 30);
        if shown.contains("FORGOTTEN-FRAME") {
            break;
        }
        assert!(opened.elapsed() < Duration::from_secs(30), "the frame was held for good:\n{shown}");
        assert!(shown.contains("BASE-FRAME"), "the held frame is the previous one:\n{shown}");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(opened.elapsed() >= timeout - Duration::from_millis(200), "released after {:?}", opened.elapsed());
    // The event that was held back reaches the app: the tick releases it once the time is up, whichever of the two
    // came first in the loop above.
    app.tick();
    app.pump();
    assert!(app.panes[&id].sync_wake().is_none(), "the held output was released");
    app.run(Action::CloseTab);
    app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
}

/// The program of `synchronized_frame_is_never_drawn_half_written`. It runs only as that test's child (`--ignored`).
#[test]
#[ignore = "a child program of synchronized_frame_is_never_drawn_half_written"]
fn sync_child_frame() {
    use std::io::Write;
    let mut out = sync_child_start();
    // The frame in two chunks with pauses, then the end of the block together with the last part.
    for (text, pause) in [("\x1b[?2026h\x1b[2J\x1b[HPARTIAL-ONE", 300), ("\r\nPARTIAL-TWO", 300)] {
        write!(out, "{text}").expect("the pane");
        out.flush().expect("the pane");
        std::thread::sleep(Duration::from_millis(pause));
    }
    write!(out, "\x1b[2J\x1b[HFINAL-FRAME\x1b[?2026l").expect("the pane");
    out.flush().expect("the pane");
    std::thread::sleep(Duration::from_secs(60));
}

/// The program of `forgotten_synchronized_block_is_released_after_the_timeout`: opens a block and never closes it.
#[test]
#[ignore = "a child program of forgotten_synchronized_block_is_released_after_the_timeout"]
fn sync_child_forgotten() {
    use std::io::Write;
    let mut out = sync_child_start();
    write!(out, "\x1b[?2026h\x1b[2J\x1b[HFORGOTTEN-FRAME").expect("the pane");
    out.flush().expect("the pane");
    std::thread::sleep(Duration::from_secs(60));
}

/// What both synchronized-output programs begin with: raw mode (the answer to the probe must not be echoed), the
/// probe for mode 2026, a first frame, and a pause long enough for the test to draw it.
fn sync_child_start() -> std::io::Stdout {
    use std::io::Write;
    crossterm::terminal::enable_raw_mode().expect("raw mode");
    let mut out = std::io::stdout();
    write!(out, "\x1b[?2026$p\x1b[2J\x1b[HBASE-FRAME").expect("the pane");
    out.flush().expect("the pane");
    std::thread::sleep(Duration::from_millis(1500));
    out
}

/// A folder the shell reports on a network path (Git Bash's `file://localhost//server/share`) is shown in the
/// title, but it is never checked, joined or acted on: the pane keeps using its start folder.
#[test]
fn network_folder_is_shown_but_never_used() {
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("unc".into()));
    let id = app.tabs[0].focus;
    wait_idle(&mut app, id);
    let start = app.panes[&id].start_cwd.clone();
    app.panes[&id].parser().process(b"\x1b]7;file://localhost//server/share\x1b\\");
    let shown = PathBuf::from(if cfg!(windows) { r"\\server\share" } else { "//server/share" });
    assert_eq!(app.panes[&id].shown_cwd(), shown);
    assert_eq!(app.panes[&id].cwd(), start);
    // No check was started for it, so no answer for it is accepted either.
    assert!(!app.panes[&id].set_cwd_usable(&shown.display().to_string(), true));
    app.pump();
    assert_eq!(app.panes[&id].cwd(), start);
    let text = render(&mut app, 110, 30);
    assert!(text.contains("server/share") || text.contains(r"server\share"), "the title shows it:\n{text}");
    app.run(Action::CloseTab);
}

/// A folder the shell reports is checked on a background thread: `cwd` answers the start folder until the check
/// comes back as `AppEvent::PaneCwd`, which `pump` applies. A folder that does not exist is never used.
#[test]
fn reported_folder_is_checked_in_the_background() {
    let mut app = demo_app(110, 30);
    let target = std::env::temp_dir().join(format!("noble-check-{}", std::process::id()));
    std::fs::create_dir_all(&target).unwrap();
    app.new_tab(dirs::home_dir().unwrap(), None, Some("check".into()));
    let id = app.tabs[0].focus;
    wait_idle(&mut app, id);
    let start = app.panes[&id].start_cwd.clone();
    let report = |app: &mut App, dir: &std::path::Path| {
        let p = dir.display().to_string().replace('\\', "/");
        let p = p.trim_start_matches('/');
        app.panes[&id].parser().process(format!("\x1b]7;file://localhost/{p}\x1b\\").as_bytes());
    };
    report(&mut app, &target);
    // Nothing waits for the disk: until the answer is applied the start folder is used.
    assert_eq!(app.panes[&id].cwd(), start);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while app.panes[&id].cwd() != target {
        assert!(std::time::Instant::now() < deadline, "the reported folder was never applied");
        app.pump();
        std::thread::sleep(Duration::from_millis(20));
    }
    let missing = target.join("missing");
    report(&mut app, &missing);
    let until = std::time::Instant::now() + Duration::from_millis(500);
    while std::time::Instant::now() < until {
        app.pump();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(app.panes[&id].cwd(), start, "a folder that does not exist is not used");
    // An answer for a folder the shell has left is ignored.
    assert!(!app.panes[&id].set_cwd_usable(&target.display().to_string(), true));
    let _ = std::fs::remove_dir_all(&target);
    app.run(Action::CloseTab);
}

/// A config whose panes follow the UI theme (`theme`), so the theme alone decides the pane colors.
fn theme_colored(app: &App, theme: &str) -> noble::config::Config {
    let mut cfg = app.cfg.clone();
    cfg.general.theme = theme.into();
    cfg.general.transparent = false;
    cfg.terminal.colors = noble::theme::FOLLOW_THEME.into();
    cfg.terminal.background = String::new();
    cfg.terminal.foreground = String::new();
    cfg
}

/// `text` as the format string of POSIX `printf`: ESC, BEL and the backslash written as escapes.
fn printf_escaped(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\x1b' => "\\033".to_string(),
            '\x07' => "\\007".to_string(),
            '\\' => "\\\\".to_string(),
            c => c.to_string(),
        })
        .collect()
}

/// Color queries reach the program with the pane's colors: OSC 11 and OSC 4 end the way the query ended (BEL or ST),
/// `CSI ? 996 n` tells light from dark, and `COLORFGBG` is in the environment. The theme is a light one (Latte), so a
/// dark default would show. The program reads each answer back by its byte count, with POSIX `sh`, `stty`, `dd` and
/// `od`, so it runs on Unix; ConPTY hands a Windows program the answer as keyboard input, so there the answers are
/// checked by `pane::tests::color_answers_end_like_the_query` (the queue step on every OS) and the `osc.rs` tests.
#[test]
fn color_queries_reach_the_program() {
    if cfg!(windows) {
        return;
    }
    let mut app = demo_app(110, 30);
    let cfg = theme_colored(&app, "latte");
    app.apply_config(cfg);
    app.sync_color_reports();
    let report = app.colors.read().unwrap().clone();
    assert!(report.is_light(), "Latte is a light theme");
    let rgb = noble::term::colors::xterm_rgb(report.bg);
    let red = noble::term::colors::xterm_rgb(report.palette(1));
    // (the query, the answer it must get).
    let queries = [
        ("\x1b]11;?\x07".to_string(), format!("\x1b]11;{rgb}\x07")),
        ("\x1b]11;?\x1b\\".to_string(), format!("\x1b]11;{rgb}\x1b\\")),
        ("\x1b]4;1;?\x07".to_string(), format!("\x1b]4;1;{red}\x07")),
        ("\x1b[?996n".to_string(), "\x1b[?997;2n".to_string()),
    ];
    // Raw mode; each step prints the query (`\033` for ESC, `\\` for the backslash), reads the answer's bytes back
    // and prints them as hex. The environment variable comes first.
    let mut script = String::from("printf 'FGBG=%s ' \"$COLORFGBG\";stty raw -echo;");
    for (query, answer) in &queries {
        script += &format!(
            "printf '{}';dd bs=1 count={} 2>/dev/null | od -An -tx1 | tr -d ' \n';printf ' ';",
            printf_escaped(query),
            answer.len()
        );
    }
    let mut cfg = app.cfg.clone();
    cfg.terminal.shell = "sh".into();
    cfg.terminal.shell_args = vec!["-c".into(), format!("{script}sleep 3")];
    app.apply_config(cfg);
    app.new_tab(std::env::temp_dir(), None, Some("colors".into()));
    let id = app.tabs[0].focus;
    let hex = |s: &str| s.bytes().map(|b| format!("{b:02x}")).collect::<String>();
    pump_until(&mut app, "every color answer came back", |app| {
        let screen = screen_text(app, id);
        screen.contains("FGBG=0;15") && queries.iter().all(|(_, answer)| screen.contains(&hex(answer)))
    });
    app.run(Action::CloseTab);
    assert!(app.tabs.is_empty());
}

/// A change of theme sends `CSI ? 997 ; Ps n` to the panes whose program set mode 2031 and to no other, and only when
/// the background turned light or dark. `sync_color_reports` names the panes it told, which is checked on every OS;
/// on Unix the tty echoes what NOBLE wrote to a pane, so the screens show who got the notice.
#[test]
fn theme_switch_notifies_only_panes_with_scheme_notices() {
    let mut app = demo_app(110, 30);
    let cfg = theme_colored(&app, "amber");
    app.apply_config(cfg);
    app.sync_color_reports();
    if !cfg!(windows) {
        let mut cfg = app.cfg.clone();
        cfg.terminal.shell = "sh".into();
        cfg.terminal.shell_args = vec!["-c".into(), "sleep 30".into()];
        app.apply_config(cfg);
    }
    app.run(Action::NewTab);
    let with_mode = app.tabs[0].focus;
    app.run(Action::NewTab);
    let without = app.tabs[1].focus;
    // The program of the first pane asks for the notices.
    app.panes[&with_mode].parser().process(b"\x1b[?2031h");
    assert!(app.panes[&with_mode].wants_scheme_notices());
    assert!(!app.panes[&without].wants_scheme_notices());

    // Dark to light: the pane with the mode is told, the other is not.
    let cfg = theme_colored(&app, "latte");
    app.apply_config(cfg);
    assert_eq!(app.sync_color_reports(), vec![with_mode]);
    // Nothing changed since: nothing is sent.
    assert!(app.sync_color_reports().is_empty());
    // Light to another light theme: the colors change, the scheme does not turn.
    let cfg = theme_colored(&app, "paper");
    app.apply_config(cfg);
    assert!(app.sync_color_reports().is_empty());
    assert!(app.colors.read().unwrap().is_light());
    // A light terminal scheme over a dark theme counts the same.
    let mut cfg = theme_colored(&app, "amber");
    cfg.terminal.colors = "solarized-light".into();
    app.apply_config(cfg);
    assert!(app.sync_color_reports().is_empty(), "still light");
    // Back to dark: told again, with the dark notice.
    let cfg = theme_colored(&app, "amber");
    app.apply_config(cfg);
    assert_eq!(app.sync_color_reports(), vec![with_mode]);
    assert!(!app.colors.read().unwrap().is_light());
    // A program that reset the mode is no longer told.
    app.panes[&with_mode].parser().process(b"\x1b[?2031l");
    let cfg = theme_colored(&app, "latte");
    app.apply_config(cfg);
    assert!(app.sync_color_reports().is_empty());

    #[cfg(not(windows))]
    {
        // Light, dark, light were sent while the mode was on: two notices in the order they were sent. The
        // tty shows ESC as `^[`.
        pump_until(&mut app, "the notices are echoed", |app| {
            screen_text(app, with_mode).matches("^[[?997;").count() >= 2
        });
        let echoed = screen_text(&app, with_mode);
        let (first, second) = (echoed.find("^[[?997;2n"), echoed.find("^[[?997;1n"));
        assert!(first.is_some() && second.is_some() && first < second, "{echoed}");
        std::thread::sleep(Duration::from_millis(300));
        app.pump();
        assert!(!screen_text(&app, without).contains("997"), "{}", screen_text(&app, without));
    }
    app.run(Action::CloseTab);
    app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    app.run(Action::CloseTab);
    app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
}

/// The command output actions on a real pane: the shell's marks open a command block, the failed command's badge
/// is a button that scrolls to its output, the jumps stop at the edges, and a send with no agent pane says so.
#[test]
fn command_output_follows_the_shell_marks() {
    let mut app = demo_app(110, 30);
    let wait = match app.shell.label().to_lowercase().as_str() {
        "cmd" => "ping -n 60 127.0.0.1 >nul",
        "pwsh" | "powershell" => "Start-Sleep 60",
        _ => "sleep 60",
    };
    app.new_tab(std::env::temp_dir(), Some(wait), None);
    let id = app.tabs[0].focus;
    std::thread::sleep(Duration::from_millis(800));
    app.pump();
    app.panes.get_mut(&id).unwrap().prompted = true;
    let feed = |app: &mut App, bytes: &[u8]| {
        let pane = &app.panes[&id];
        pane.parser().process(bytes);
        pane.dirty.store(true, std::sync::atomic::Ordering::Release);
        app.handle(AppEvent::PtyOutput);
    };
    // A command that prints thirty lines and fails (exit 101), marked the way NOBLE's shell hooks mark it.
    let mut bytes = b"~/app $ cargo test\r\n\x1b]133;C\x07".to_vec();
    for i in 0..30 {
        bytes.extend(format!("line {i}\r\n").as_bytes());
    }
    bytes.extend(b"\x1b]133;D;101\x07");
    feed(&mut app, &bytes);
    let last = app.panes[&id].last_result.expect("the prompt ends the command");
    assert_eq!(last.code, Some(101));
    let blocks = app.panes[&id].command_blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(app.panes[&id].block_lines(&blocks[0]).map(|l| l.len()), Some(30));

    // No agent pane in the tab: nothing is sent, and the toast says so.
    app.run(Action::SendLastOutput);
    assert!(toast_texts(&app).iter().any(|t| t.contains("no agent pane")), "{:?}", toast_texts(&app));

    // The failed badge is a button: a click scrolls the view to the command's output.
    render(&mut app, 110, 30);
    let badge = find_hit(&app, |h| *h == noble::app::Hit::CommandBadge(id)).expect("the ✗ badge is clickable");
    click(&mut app, badge.x, badge.y);
    assert!(app.panes[&id].scroll_offset() > 0, "the view moved to the output");

    // It is the only command: the jumps find nothing further and say so.
    app.run(Action::PrevCommand);
    assert!(toast_texts(&app).iter().any(|t| t.contains("no earlier command")), "{:?}", toast_texts(&app));
    app.run(Action::NextCommand);
    assert_eq!(app.panes[&id].scroll_offset(), 0, "past the newest command the view is live");
}

/// A progress report (OSC 9;4) a real program prints reaches the pane while its command runs, shows in the pane title,
/// and the prompt that comes back after the command clears it. cmd.exe has no way to print an escape sequence from its
/// command line, so it is not checked here (the parser tests cover the report itself).
#[test]
fn progress_report_from_a_real_program_reaches_the_pane() {
    check_progress("");
    let mut checked = vec!["default"];
    if cfg!(windows) {
        let git_bash = std::path::Path::new(r"C:\Program Files\Git\bin\bash.exe");
        if git_bash.is_file() {
            check_progress(&git_bash.display().to_string());
            checked.push("git bash");
        }
        if noble::util::which("pwsh").is_some()
            && let Some(ps) = noble::util::which("powershell")
        {
            check_progress(&ps.display().to_string());
            checked.push("powershell");
        }
    } else {
        for shell in ["bash", "zsh", "fish", "pwsh"] {
            if let Some(path) = noble::util::which(shell) {
                check_progress(&path.display().to_string());
                checked.push(shell);
            }
        }
    }
    assert_checked(&checked, "progress_report_from_a_real_program_reaches_the_pane");
}

fn check_progress(shell: &str) {
    use noble::term::pane::{Progress, ProgressState};
    let mut app = demo_app(110, 30);
    if !shell.is_empty() {
        let mut cfg = app.cfg.clone();
        cfg.terminal.shell = shell.into();
        app.apply_config(cfg);
    }
    let kind = app.shell.label().to_lowercase();
    app.new_tab(std::env::temp_dir(), None, Some("progress".into()));
    let id = app.tabs[0].focus;
    pump_until(&mut app, &format!("{kind}: first prompt"), |a| a.panes[&id].prompted);
    // pwsh prints its prompt before PSReadLine is ready (slowest on macOS): let it settle before typing.
    wait_idle(&mut app, id);
    // The command prints the report and sleeps, so the report is on screen while it runs.
    let line = match kind.as_str() {
        "pwsh" | "powershell" => "Write-Host -NoNewline ([char]27 + ']9;4;1;42' + [char]7); Start-Sleep -Seconds 3",
        _ => "printf '\\e]9;4;1;42\\a'; sleep 3",
    };
    for c in line.chars() {
        app.handle(AppEvent::Input(Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))));
    }
    app.handle(AppEvent::Input(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))));
    pump_until(&mut app, &format!("{kind}: a progress report"), |a| a.panes[&id].progress().is_some());
    assert_eq!(
        app.panes[&id].progress(),
        Some(Progress { state: ProgressState::Normal, percent: Some(42) }),
        "{kind}: the report"
    );
    let row = render(&mut app, 110, 30).lines().find(|l| l.contains(" ✕ ")).unwrap_or_default().to_string();
    assert!(row.contains("42%"), "{kind}: {row}");
    // The command ends and the prompt comes back: the prompt mark clears the report.
    pump_until(&mut app, &format!("{kind}: the prompt back"), |a| a.panes[&id].last_result.is_some());
    assert_eq!(app.panes[&id].progress(), None, "{kind}: the prompt clears the report");
    app.run(Action::CloseTab);
}
