//! Real shells in a PTY: cwd tracking, command results, output throughput and hostile names.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use noble::app::{App, View};
use noble::event::AppEvent;
use noble::keys::Action;
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
    if cfg!(windows) {
        check_cwd_tracking("cmd.exe");
        // Windows PowerShell 5.1 is the default shell where pwsh is not installed; when pwsh took the
        // default above, 5.1 is checked on its own.
        if noble::util::which("pwsh").is_some()
            && let Some(ps) = noble::util::which("powershell")
        {
            check_cwd_tracking(&ps.display().to_string());
        }
        let git_bash = std::path::Path::new(r"C:\Program Files\Git\bin\bash.exe");
        if git_bash.is_file() {
            check_cwd_tracking(&git_bash.display().to_string());
        }
    } else {
        for shell in ["bash", "zsh", "fish", "pwsh"] {
            if let Some(path) = noble::util::which(shell) {
                check_cwd_tracking(&path.display().to_string());
            }
        }
    }
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
    for shell in shells {
        check_hostile_file_name(&shell);
    }
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
    assert!(app.redraw_after().unwrap() > ms(1005) || chrono::Local::now().format("%S").to_string() == "59");
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
    if cfg!(windows) {
        check_exit_code("cmd.exe");
        let git_bash = std::path::Path::new(r"C:\Program Files\Git\bin\bash.exe");
        if git_bash.is_file() {
            check_exit_code(&git_bash.display().to_string());
        }
    } else {
        for shell in ["bash", "zsh", "fish", "pwsh"] {
            if let Some(path) = noble::util::which(shell) {
                check_exit_code(&path.display().to_string());
            }
        }
    }
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
    std::thread::sleep(Duration::from_millis(30));
    key(&mut app, KeyCode::Enter);
    assert!(app.panes[&id].command_started.unwrap() > first, "a later Enter restarts the clock");
    feed(&mut app, b"\x1b]133;D;1\x07");
    let guessed = app.panes[&id].last_result.expect("a typed line counts");
    assert_eq!(guessed.code, Some(1));
    assert!(guessed.took < Duration::from_millis(30), "{:?}", guessed.took);

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
    let (program, args) = if cfg!(windows) { ("timeout", vec!["/t", "5", "/nobreak"]) } else { ("sleep", vec!["5"]) };
    cfg.terminal.shell = program.into();
    cfg.terminal.shell_args = args.into_iter().map(String::from).collect();
    app.apply_config(cfg);
    app.new_tab(std::env::temp_dir(), None, Some("stuck".into()));
    let id = app.tabs[0].focus;
    let body: String = (0..2500).map(|_| format!("{}\r", "a".repeat(79))).collect();
    assert!(body.len() > 190_000);
    let started = std::time::Instant::now();
    app.panes.get_mut(&id).unwrap().paste(&body);
    let took = started.elapsed();
    assert!(took < Duration::from_secs(1), "the paste waited {took:?} for the program");
    let started = std::time::Instant::now();
    app.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    app.pump();
    let text = render(&mut app, 110, 30);
    assert!(started.elapsed() < Duration::from_secs(1), "the UI stalled after the paste");
    assert!(text.contains(" ✕ "), "the pane still draws:\n{text}");
    app.run(Action::CloseTab);
    assert!(app.tabs.is_empty());
}

/// Terminal queries are answered and the answer reaches the program's input (the reader thread queues it to
/// the pane's writer). Needs POSIX `sh`, `stty`, `dd` and `od` to read the answer back, so it runs on Unix; the
/// Windows panes answer the same query through the same reader thread, but there is no such tool there.
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
