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
