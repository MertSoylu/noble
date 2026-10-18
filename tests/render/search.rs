//! Scrollback search and open links.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use noble::app::App;
use noble::event::AppEvent;
use noble::keys::Action;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::time::Duration;

use super::common::*;

/// Scrollback search: matches are found, the selected match is scrolled into
/// view, the search bar is drawn and esc restores everything.
#[test]
fn terminal_search_finds_scrollback() {
    let mut app = demo_app(110, 30);
    let script = if cfg!(windows) {
        "(for /L %i in (1,1,60) do @echo row %i) & echo NEEDLE_DONE"
    } else {
        "for i in $(seq 1 60); do echo row $i; done; echo NEEDLE_DONE"
    };
    // The script is cmd.exe syntax on Windows and POSIX sh elsewhere: the shell is chosen to match, not
    // left to the machine's default (fish would not run it).
    let mut cfg = app.cfg.clone();
    cfg.terminal.shell = if cfg!(windows) { "cmd.exe" } else { "/bin/sh" }.into();
    app.apply_config(cfg);
    app.new_tab(std::env::temp_dir(), Some(script), Some("search".into()));
    let id = app.tabs[0].focus;
    wait_for(&mut app, id, "NEEDLE_DONE");
    app.run(Action::Search);
    assert!(app.search.is_some());
    for c in "ROW 1".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    let s = app.search.as_ref().unwrap();
    // "row 1", "row 10".."row 19" = 11 matches (the command line echo may be excluded).
    assert!(s.matches.len() >= 11, "{}", s.matches.len());
    assert!(app.panes[&id].scroll_offset() > 0, "should scroll to the newest match above the screen");
    let text = render(&mut app, 110, 30);
    save("terminal-search-110x30", &text);
    assert!(text.contains("find ROW 1"), "{text}");
    // Go to the previous match: the counter decreases.
    let before = app.search.as_ref().unwrap().current;
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.search.as_ref().unwrap().current, before.map(|i| i - 1));
    for c in "zzz".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    assert!(render(&mut app, 110, 30).contains("no matches"));
    render(&mut app, 30, 8);
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.search.is_none());
    assert_eq!(app.panes[&id].scroll_offset(), 0);
    app.run(Action::CloseTab);
}

/// Output that arrives while the search bar is open must not throw the next jump off: the matches and
/// the history length follow the output, so the match lands in the middle of the screen.
#[test]
fn search_step_lands_on_match_after_new_output() {
    let mut app = demo_app(110, 30);
    // The script waits before its second batch, so that output arrives while the search bar is open.
    let (script, cfg_shell) = if cfg!(windows) {
        (
            "(for /L %i in (1,1,60) do @echo row %i) & echo NEEDLE_DONE & ping -n 3 127.0.0.1 >nul & (for /L %i in (1,1,40) do @echo extra %i) & echo MORE_DONE & ping -n 30 127.0.0.1 >nul",
            "cmd.exe",
        )
    } else {
        (
            "for i in $(seq 1 60); do echo row $i; done; echo NEEDLE_DONE; sleep 2; for i in $(seq 1 40); do echo extra $i; done; echo MORE_DONE; sleep 30",
            "/bin/sh",
        )
    };
    let mut cfg = app.cfg.clone();
    cfg.terminal.shell = cfg_shell.into();
    app.apply_config(cfg);
    app.new_tab(std::env::temp_dir(), Some(script), Some("search".into()));
    let id = app.tabs[0].focus;
    wait_for(&mut app, id, "NEEDLE_DONE");
    app.run(Action::Search);
    for c in "ROW 1".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    let typed = app.search.as_ref().unwrap().history;
    wait_for(&mut app, id, "MORE_DONE");
    // The matches of the last output follow within one refresh interval (the main loop ticks meanwhile).
    run_for(&mut app, Duration::from_millis(300));
    let s = app.search.as_ref().unwrap();
    assert!(s.history > typed, "the search should follow output that arrives while it is open");
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let s = app.search.as_ref().unwrap();
    let target = s.matches[s.current.unwrap()].line;
    let pane = &app.panes[&id];
    let (lines, _) = pane.all_lines();
    let rows = (0u16..).take_while(|&r| pane.visible_row(r).is_some()).count();
    let middle = pane.visible_row((rows / 2) as u16).unwrap();
    assert_eq!(middle.trim_end(), lines[target].trim_end(), "the selected match should be in the middle");
    app.run(Action::CloseTab);
}

/// While a pane is scrolled back it shows a position bar in its last column (with the search matches
/// on it) and a "↓ live" chip counting the lines that arrived meanwhile; a click on the bar jumps,
/// a click on the chip returns to the newest output. Nothing of it shows while live.
#[test]
fn scrollback_helpers_while_scrolled_back() {
    use noble::app::Hit;
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("scroll".into()));
    std::thread::sleep(Duration::from_millis(1500));
    app.pump();
    let id = app.tabs[0].focus;
    let feed = |app: &mut App, from: usize, to: usize| {
        let pane = &app.panes[&id];
        let text: String = (from..to).map(|i| format!("line {i}\r\n")).collect();
        pane.parser().process(text.as_bytes());
        pane.dirty.store(true, std::sync::atomic::Ordering::Release);
        app.handle(AppEvent::PtyOutput);
    };
    feed(&mut app, 0, 200);
    let live = render(&mut app, 110, 30);
    assert!(!live.contains("↓ live"), "{live}");
    assert!(find_hit(&app, |h| matches!(h, Hit::ScrollTrack { .. } | Hit::ScrollLive(_))).is_none());

    app.run(Action::ScrollUp);
    let offset = app.panes[&id].scroll_offset();
    assert!(offset > 0);
    assert_eq!(app.panes[&id].new_lines(), 0);
    let text = render(&mut app, 110, 30);
    save("pane-scrolled-back-110x30", &text);
    let track = find_hit(&app, |h| matches!(h, Hit::ScrollTrack { .. })).expect("position bar");
    let chip = find_hit(&app, |h| matches!(h, Hit::ScrollLive(_))).expect("live chip");
    assert!(text.contains(" ↓ live "), "{text}");
    assert!(!text.contains("↓ live ·"), "{text}");
    // The bar runs down the content's last column, the chip sits below it on the bottom row.
    assert_eq!(track.x, chip.right() - 1);
    assert_eq!(track.bottom(), chip.y);
    let column: String = (track.y..track.bottom())
        .map(|y| text.lines().nth(y as usize).unwrap().chars().nth(track.x as usize).unwrap())
        .collect();
    assert!(column.contains('┃') && column.contains('│'), "{column}");
    assert!(!column.ends_with('┃'), "scrolled back, the thumb is not at the bottom: {column}");
    // The title's "↑N" tag and the status bar hint agree with the offset.
    assert!(text.contains(&format!("↑{offset}")), "{text}");
    assert!(text.lines().last().unwrap().contains("back to live"), "{text}");

    // New output keeps the view in place and is counted on the chip.
    feed(&mut app, 200, 207);
    assert!(app.panes[&id].scroll_offset() >= offset + 7);
    let new = app.panes[&id].new_lines();
    assert!(new >= 7, "{new}");
    let text = render(&mut app, 110, 30);
    assert!(text.contains(&format!("↓ live · {new}")), "{text}");
    let buf_chip = find_hit(&app, |h| matches!(h, Hit::ScrollLive(_))).unwrap();
    assert!(buf_chip.width > chip.width);
    // The mouse on the chip underlines its label and keeps its colors, so it stays readable.
    app.hover = Some((buf_chip.x + 2, buf_chip.y));
    let (_, buf) = render_buf(&mut app, 110, 30);
    let label = &buf[(buf_chip.x + 1, buf_chip.y)];
    assert_eq!((label.fg, label.bg), (app.theme.on_accent, app.theme.accent2));
    assert!(label.modifier.contains(ratatui::style::Modifier::UNDERLINED));
    app.hover = None;

    // The wheel over the bar and over the chip scrolls the pane as over its content.
    use crossterm::event::MouseEventKind;
    let track = find_hit(&app, |h| matches!(h, Hit::ScrollTrack { .. })).unwrap();
    let before = app.panes[&id].scroll_offset();
    mouse(&mut app, MouseEventKind::ScrollUp, track.x, track.y + 2);
    assert_eq!(app.panes[&id].scroll_offset(), before + 3);
    render(&mut app, 110, 30);
    for _ in 0..2 {
        mouse(&mut app, MouseEventKind::ScrollUp, track.x, track.y + 2);
    }
    assert_eq!(app.panes[&id].scroll_offset(), before + 9, "every wheel step over the bar counts");
    mouse(&mut app, MouseEventKind::ScrollDown, track.x, track.y + 2);
    assert_eq!(app.panes[&id].scroll_offset(), before + 6);
    let chip = find_hit(&app, |h| matches!(h, Hit::ScrollLive(_))).unwrap();
    mouse(&mut app, MouseEventKind::ScrollDown, chip.x + 1, chip.y);
    assert_eq!(app.panes[&id].scroll_offset(), before + 3);
    assert_eq!(app.panes[&id].new_lines(), new);

    // Scrolling further keeps the count; a click at the top of the bar goes to the oldest lines.
    app.run(Action::ScrollUp);
    assert_eq!(app.panes[&id].new_lines(), new);
    let track = find_hit(&app, |h| matches!(h, Hit::ScrollTrack { .. })).unwrap();
    click(&mut app, track.x, track.y);
    let history = app.panes[&id].history_len();
    assert!(app.panes[&id].scroll_offset() + 20 >= history, "{} of {history}", app.panes[&id].scroll_offset());
    assert_eq!(app.panes[&id].new_lines(), new);

    // Search matches are marked on the bar, the selected one in the accent color.
    app.run(Action::Search);
    for c in "line 12".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    assert!(app.panes[&id].scroll_offset() > 0);
    let text = render(&mut app, 110, 30);
    save("pane-scrolled-search-110x30", &text);
    let track = find_hit(&app, |h| matches!(h, Hit::ScrollTrack { .. })).unwrap();
    let chip = find_hit(&app, |h| matches!(h, Hit::ScrollLive(_))).unwrap();
    // The chip moves up above the search bar.
    assert_eq!(track.bottom(), chip.y);
    assert!(text.lines().nth(chip.y as usize + 1).unwrap().contains("find line 12"), "{text}");
    let column: String = (track.y..track.bottom())
        .map(|y| text.lines().nth(y as usize).unwrap().chars().nth(track.x as usize).unwrap())
        .collect();
    assert!(column.contains('●') && column.contains('•'), "{column}");
    let mut term = Terminal::new(TestBackend::new(110, 30)).unwrap();
    term.draw(|f| noble::ui::draw(f, &mut app)).unwrap();
    let buf = term.backend().buffer().clone();
    let current = (track.y..track.bottom()).find(|&y| buf[(track.x, y)].symbol() == "●").unwrap();
    assert_eq!(buf[(track.x, current)].fg, app.theme.accent);
    // The other matches in a color that shows as a mark (the match background tint does not).
    let other = (track.y..track.bottom()).find(|&y| buf[(track.x, y)].symbol() == "•").unwrap();
    assert_eq!(buf[(track.x, other)].fg, app.theme.accent2);
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.panes[&id].scroll_offset(), 0);

    // Small panes: only the chip, or nothing when even that does not fit.
    for (w, h) in [(40, 12), (30, 8)] {
        app.run(Action::ScrollUp);
        assert!(app.panes[&id].scroll_offset() > 0);
        let text = render(&mut app, w, h);
        save(&format!("pane-scrolled-back-{w}x{h}"), &text);
        assert!(text.lines().all(|l| l.chars().count() == w as usize), "{text}");
        let chip = find_hit(&app, |h| matches!(h, Hit::ScrollLive(_)));
        let track = find_hit(&app, |h| matches!(h, Hit::ScrollTrack { .. }));
        if let Some(Hit::Pane { inner, .. }) =
            app.hits.iter().find(|(_, h)| matches!(h, Hit::Pane { .. })).map(|(_, h)| h.clone())
        {
            assert_eq!(track.is_some(), inner.width >= 20 && inner.height >= 4, "{w}x{h}: {inner:?}");
            if let Some(c) = chip {
                assert!(c.right() <= inner.right() && c.y < inner.bottom());
            }
        }
        // A click on the chip returns to live, and the helpers are gone.
        if let Some(c) = chip {
            click(&mut app, c.x, c.y);
        } else {
            app.panes[&id].scroll_reset();
        }
        assert_eq!(app.panes[&id].scroll_offset(), 0);
        assert_eq!(app.panes[&id].new_lines(), 0);
        render(&mut app, w, h);
        assert!(find_hit(&app, |h| matches!(h, Hit::ScrollLive(_) | Hit::ScrollTrack { .. })).is_none());
    }
    app.run(Action::CloseTab);
}

/// The keyboard way to ctrl+click (macOS terminals keep ctrl+click for their own menu): the links on the pane's
/// screen in a menu, the newest first, a file only when it exists and a bare folder name left out.
#[test]
fn open_link_lists_the_links_on_screen() {
    let dir = std::env::temp_dir().join(format!("noble-links-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("notes.txt"), "x\n").unwrap();
    let mut app = demo_app(110, 30);
    app.run(Action::OpenLink);
    assert!(app.overlay.is_none(), "no terminal, no menu");
    app.new_tab(dir.clone(), None, Some("links".into()));
    let id = app.tabs[0].focus;
    wait_idle(&mut app, id);
    let feed = |app: &mut App, bytes: &[u8]| app.panes[&id].parser().process(bytes);
    feed(&mut app, b"\x1b[2J\x1b[H");
    feed(&mut app, b"old https://example.com/a and missing.txt:4\r\n");
    feed(&mut app, b"see \x1b]8;;https://example.com/docs\x1b\\docs\x1b]8;;\x1b\\ in sub\r\n");
    feed(&mut app, b"error at notes.txt:3 (again https://example.com/a)\r\n");
    render(&mut app, 110, 30);
    app.run(Action::OpenLink);
    let Some(noble::app::Overlay::Menu(m)) = &app.overlay else { panic!("links menu expected") };
    let labels: Vec<&str> = m.items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels.len(), 3, "{labels:?}");
    assert!(labels[0].starts_with("1 ") && labels[0].ends_with("notes.txt:3"), "{labels:?}");
    assert_eq!(labels[1], "2 https://example.com/a");
    assert_eq!(labels[2], "3 https://example.com/docs");
    save("open-link-menu-110x30", &render(&mut app, 110, 30));
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.overlay.is_none());
    app.run(Action::CloseTab);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Output while the search is open refreshes the matches at most once per interval, and the output that came
/// in during the interval is computed when it has passed (the main loop ticks meanwhile). The interval is moved
/// by setting the time of the last refresh, so the test does not depend on how long the shell takes.
#[test]
fn search_refresh_is_throttled_and_follows_the_last_output() {
    let mut app = demo_app(110, 30);
    // The shell prints the third row two seconds later, after the search has started.
    let script = if cfg!(windows) {
        "(echo row 1) & (echo row 2) & ping -n 3 127.0.0.1 >nul & (echo row 3) & ping -n 30 127.0.0.1 >nul"
    } else {
        "echo row 1; echo row 2; sleep 2; echo row 3; sleep 30"
    };
    let mut cfg = app.cfg.clone();
    cfg.terminal.shell = if cfg!(windows) { "cmd.exe" } else { "/bin/sh" }.into();
    app.apply_config(cfg);
    app.new_tab(std::env::temp_dir(), Some(script), Some("throttle".into()));
    let id = app.tabs[0].focus;
    wait_for(&mut app, id, "row 2");
    app.run(Action::Search);
    for c in "row".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    let before = app.search.as_ref().unwrap().matches.len();
    assert!(before >= 2, "{before}");
    // Refreshed "just now": the interval is far from over, so output that arrives now waits for it.
    app.search.as_mut().unwrap().refreshed = Some(std::time::Instant::now() + Duration::from_secs(60));
    wait_for(&mut app, id, "row 3");
    let s = app.search.as_ref().unwrap();
    assert_eq!((s.matches.len(), s.stale), (before, true), "throttled while the output comes in");
    assert!(app.redraw_after().is_some_and(|d| d <= Duration::from_millis(100)), "a wake-up for the refresh");
    // The interval has passed: the next tick computes the stale matches.
    app.search.as_mut().unwrap().refreshed = Some(std::time::Instant::now() - Duration::from_secs(1));
    app.tick();
    let s = app.search.as_ref().unwrap();
    assert_eq!((s.matches.len(), s.stale), (before + 1, false), "the last output is never left out");
    app.run(Action::CloseTab);
}

/// A resize of the search's pane re-wraps its lines, so the matches are computed again at once, not on the next
/// output: the text the search holds is read anew at the new size, and the matches are those of that text.
#[test]
fn search_follows_a_pane_resize() {
    use noble::term::pane::find_matches;
    use std::sync::Arc;
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("resize".into()));
    let id = app.tabs[0].focus;
    wait_idle(&mut app, id);
    let text: String = (0..20).map(|i| format!("row {i} {}\r\n", "x".repeat(90))).collect();
    app.panes[&id].parser().process(text.as_bytes());
    app.run(Action::Search);
    for c in "row".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    let before = Arc::clone(&app.search.as_ref().unwrap().text.as_ref().unwrap().lines);
    // Narrower: every long line wraps again, so the lines (and the matches on them) change. The draw only marks the
    // matches stale: the throttled refresh computes them on the next tick (`redraw_after` wakes up for it).
    render(&mut app, 70, 30);
    assert!(app.search.as_ref().unwrap().stale, "a resize marks the matches stale");
    assert!(app.redraw_after().is_some_and(|d| d <= Duration::from_millis(100)), "a wake-up for the refresh");
    app.search.as_mut().unwrap().refreshed = Some(std::time::Instant::now() - Duration::from_secs(1));
    app.tick();
    let s = app.search.as_ref().unwrap();
    let text = s.text.as_ref().unwrap();
    assert!(!Arc::ptr_eq(&before, &text.lines), "the text was read anew at the new size");
    assert_eq!(s.matches, find_matches(&text.lines, "row").0, "the matches are those of the new lines");
    assert!(!s.stale);
    app.run(Action::CloseTab);
}

/// Past `MAX_MATCHES` the search keeps the newest matches and the bar says so ("10000+").
#[test]
fn search_count_says_when_it_is_capped() {
    use noble::term::pane::MAX_MATCHES;
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("cap".into()));
    let id = app.tabs[0].focus;
    wait_idle(&mut app, id);
    // 120 lines of 100 "o": 12 000 matches, more than the cap.
    let text: String = (0..120).map(|i| format!("{i:03} {}\r\n", "o".repeat(100))).collect();
    app.panes[&id].parser().process(text.as_bytes());
    app.run(Action::Search);
    app.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
    let s = app.search.as_ref().unwrap();
    assert!(s.capped, "{}", s.matches.len());
    assert_eq!(s.matches.len(), MAX_MATCHES);
    // The newest 10 000 matches are the last 100 of the 120 lines (100 matches each): the oldest kept one is line 020.
    let (lines, _) = app.panes[&id].all_lines();
    let first = lines.iter().position(|l| l.starts_with("020 ")).unwrap();
    let last = lines.iter().rposition(|l| l.starts_with("119 ")).unwrap();
    assert_eq!((s.matches.first().unwrap().line, s.matches.last().unwrap().line), (first, last));
    let screen = render(&mut app, 110, 30);
    assert!(screen.contains(&format!("{MAX_MATCHES}/{MAX_MATCHES}+")), "{screen}");
    save("search-capped-110x30", &screen);
    // Small panes: the bar keeps its counter (the hints go first). Narrower, the lines wrap: fewer matches.
    for (w, h) in [(60, 12), (40, 10), (30, 8)] {
        save(&format!("search-capped-{w}x{h}"), &render(&mut app, w, h));
    }
    app.run(Action::CloseTab);
}

/// A capped search whose selection falls out of the kept window (the newest 10 000 matches) keeps the selection on
/// its line: no other match is selected, the bar says "?", and the next step goes to the oldest kept match.
#[test]
fn a_selection_older_than_the_kept_matches_is_not_jumped_to() {
    use noble::term::pane::MAX_MATCHES;
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("detach".into()));
    let id = app.tabs[0].focus;
    wait_idle(&mut app, id);
    let line = |i: usize| format!("{i:03} {}\r\n", "o".repeat(100));
    let text: String = (0..120).map(line).collect();
    app.panes[&id].parser().process(text.as_bytes());
    app.run(Action::Search);
    app.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
    let oldest = app.search.as_ref().unwrap().matches[0];
    app.search.as_mut().unwrap().current = Some(0);
    // Twenty more lines: the window moves on, and the oldest kept matches (lines 020 to 039) drop off it.
    let more: String = (120..140).map(line).collect();
    app.panes[&id].parser().process(more.as_bytes());
    app.search.as_mut().unwrap().text = None;
    app.refresh_search();
    let s = app.search.as_ref().unwrap();
    assert!(s.capped && s.matches.len() == MAX_MATCHES, "{}", s.matches.len());
    assert_eq!(s.current, None, "the selection is not moved to the newest match");
    assert_eq!(s.detached, Some(oldest), "the selection stays on its line");
    let screen = render(&mut app, 110, 30);
    assert!(screen.contains(&format!("?/{MAX_MATCHES}+")), "{screen}");
    save("search-detached-110x30", &screen);
    // Stepping goes to the nearest kept match: the oldest one kept.
    app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    let s = app.search.as_ref().unwrap();
    assert_eq!((s.current, s.detached), (Some(0), None));
    app.run(Action::CloseTab);
}

/// Search open while 50 thousand lines stream through the pty: the time the output takes to handle (the
/// search follows it), the draws, and the cost of one refresh of the search on the full scrollback. Checks
/// that the search is final once the output stopped. Run with `--ignored --nocapture`, ideally `--release`.
#[test]
#[ignore]
fn search_during_heavy_output() {
    const LINES: usize = 50_000;
    let dir = std::env::temp_dir().join(format!("noble-search-perf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lines.txt");
    let body: String = (0..LINES).map(|i| format!("line {i:05} lorem ipsum dolor sit amet consectetur\n")).collect();
    std::fs::write(&file, body + "PERF_DONE\n").unwrap();
    let mut app = demo_app(160, 45);
    let mut cfg = app.cfg.clone();
    if cfg!(windows) {
        cfg.terminal.shell = "cmd.exe".into();
    }
    app.apply_config(cfg);
    let cmd = if cfg!(windows) { format!("type \"{}\"", file.display()) } else { format!("cat '{}'", file.display()) };
    app.new_tab(dir.clone(), Some(&cmd), Some("perf".into()));
    let id = app.tabs[0].focus;
    // "o" occurs several times on every line: the match count runs far past any cap.
    app.run(Action::Search);
    app.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
    let started = std::time::Instant::now();
    let (mut frames, mut draw, mut output) = (0u32, Duration::ZERO, Duration::ZERO);
    let mut term = Terminal::new(TestBackend::new(160, 45)).unwrap();
    loop {
        let t = std::time::Instant::now();
        app.pump();
        app.tick();
        output += t.elapsed();
        let t = std::time::Instant::now();
        term.draw(|f| noble::ui::draw(f, &mut app)).unwrap();
        draw += t.elapsed();
        frames += 1;
        let (lines, _) = app.panes[&id].all_lines();
        if lines.iter().any(|l| l.trim() == "PERF_DONE") {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(120), "output never finished");
        std::thread::sleep(Duration::from_millis(16));
    }
    let total = started.elapsed();
    // Once the output stopped the search must be final (a throttled refresh follows at most one interval later).
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        app.pump();
        app.tick();
        if app.search.as_ref().unwrap().history == app.panes[&id].history_len() || std::time::Instant::now() > deadline
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let s = app.search.as_ref().unwrap();
    assert_eq!(s.history, app.panes[&id].history_len(), "the search is final once the output stopped");
    // One refresh on the full scrollback: the copy is dropped first, so the lines are read anew.
    let started = std::time::Instant::now();
    for _ in 0..20 {
        app.search.as_mut().unwrap().text = None;
        app.refresh_search();
    }
    let per_refresh = started.elapsed() / 20;
    println!(
        "{LINES} lines in {:.2}s · {frames} frames · output handling {:.1} ms · avg draw {:.2} ms · \
         one refresh {:.2} ms · {} matches",
        total.as_secs_f64(),
        output.as_secs_f64() * 1000.0,
        draw.as_secs_f64() * 1000.0 / frames as f64,
        per_refresh.as_secs_f64() * 1000.0,
        app.search.as_ref().unwrap().matches.len()
    );
    app.run(Action::CloseTab);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The ctrl+hover highlight shows the link under the pointer while ctrl is held. A key or a mouse event
/// without ctrl ends it (a released ctrl is not always reported), and output that moves the link moves or ends
/// the highlight too: the link a ctrl+click opens is the one highlighted on the screen as it is.
#[test]
fn link_highlight_follows_ctrl_the_pointer_and_the_output() {
    use crossterm::event::{Event, KeyEventKind, MouseEvent, MouseEventKind};
    use noble::term::link::Link;
    use ratatui::style::Modifier;
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("links".into()));
    let id = app.tabs[0].focus;
    wait_idle(&mut app, id);
    let feed = |app: &mut App, bytes: &[u8]| {
        let pane = &app.panes[&id];
        pane.parser().process(bytes);
        pane.dirty.store(true, std::sync::atomic::Ordering::Release);
        app.handle(AppEvent::PtyOutput);
    };
    feed(&mut app, b"\x1b[2J\x1b[Hopen https://example.com/a now");
    render(&mut app, 110, 30);
    let inner = find_hit_inner(&app).unwrap();
    // Column 7 is inside the address (it starts at column 5).
    let (x, y) = (inner.x + 7, inner.y);
    let event = |app: &mut App, e: Event| app.handle(AppEvent::Input(e));
    let moved =
        |m: KeyModifiers| Event::Mouse(MouseEvent { kind: MouseEventKind::Moved, column: x, row: y, modifiers: m });
    let underlined = |app: &mut App| render_cells(app, 110, 30)[(x, y)].modifier.contains(Modifier::UNDERLINED);

    event(&mut app, moved(KeyModifiers::CONTROL));
    let hover = app.link_hover.clone().expect("ctrl over a link highlights it");
    assert_eq!(hover.target, Link::Url("https://example.com/a".into()));
    assert_eq!((hover.row, hover.from, hover.to), (0, 5, 26));
    assert!(underlined(&mut app));

    // A key without ctrl ends the highlight with no mouse movement at all.
    event(&mut app, Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)));
    assert!(app.link_hover.is_none() && !underlined(&mut app));
    // So does the release of ctrl, when the terminal reports it as a key event.
    event(&mut app, moved(KeyModifiers::CONTROL));
    assert!(app.link_hover.is_some());
    event(&mut app, Event::Key(KeyEvent::new_with_kind(KeyCode::Null, KeyModifiers::NONE, KeyEventKind::Release)));
    assert!(app.link_hover.is_none());
    // A mouse move without ctrl does not highlight either.
    event(&mut app, moved(KeyModifiers::NONE));
    assert!(app.link_hover.is_none());

    // Output that changes the text under the pointer changes the highlight with it, with no input at all.
    event(&mut app, moved(KeyModifiers::CONTROL));
    feed(&mut app, b"\x1b[H\x1b[2Kopen https://example.com/c");
    assert_eq!(
        app.link_hover.as_ref().map(|h| (h.target.clone(), h.from, h.to)),
        Some((Link::Url("https://example.com/c".into()), 5, 26)),
        "the highlight moved to the link now under the pointer"
    );
    feed(&mut app, b"\x1b[H\x1b[2Kplain text");
    assert!(app.link_hover.is_none(), "the link under the pointer is gone");

    // The screen changes with no event (what a layout change does): the next draw finds the link under the pointer
    // again, and asks for a frame that shows the new highlight.
    feed(&mut app, b"\x1b[H\x1b[2Kopen https://example.com/d");
    event(&mut app, moved(KeyModifiers::CONTROL));
    assert!(app.link_hover.is_some());
    app.panes[&id].parser().process(b"\x1b[H\x1b[2Kplain again");
    app.take_dirty();
    render(&mut app, 110, 30);
    assert!(app.link_hover.is_none(), "the draw found the link gone");
    assert!(app.take_dirty(), "a frame is drawn again with the new highlight");
    app.run(Action::CloseTab);
}
