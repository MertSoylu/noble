//! Fullscreen (alternate screen) redraws, the way Claude Code's fullscreen renderer draws: cursor-positioned repaints,
//! erase in line and in display, synchronized output blocks, and a resize while the alternate screen is up. The
//! drawn pane must show exactly what the program drew last, with no fragments of an earlier frame left behind.
//! The programs are this test binary again (`fullscreen_child`, `fullscreen_block_child`) running in a real PTY.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use noble::app::App;
use noble::event::AppEvent;
use noble::term::layout::PaneId;
use noble::term::pane::ShellSpec;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use std::io::Write;
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;

use super::common::*;

/// The pane's drawn inner area, one string per row (a wide character once: its second cell is skipped).
fn drawn_rows(app: &mut App, w: u16, h: u16) -> Vec<String> {
    app.size = (w, h);
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| noble::ui::draw(f, app)).unwrap();
    let inner = find_hit_inner(app).expect("pane inner");
    let buf: Buffer = term.backend().buffer().clone();
    (inner.y..inner.bottom())
        .map(|y| {
            let mut s = String::new();
            let mut skip = 0usize;
            for x in inner.x..inner.right() {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                let sym = buf[(x, y)].symbol();
                s.push_str(sym);
                skip = UnicodeWidthStr::width(sym).saturating_sub(1);
            }
            s.trim_end().to_string()
        })
        .collect()
}

/// The emulator's visible rows, trailing blanks removed.
fn emulator_rows(app: &App, id: PaneId) -> Vec<String> {
    let p = app.panes[&id].parser();
    let (_, cols) = p.screen().size();
    p.screen().rows(0, cols.get()).map(|r| r.trim_end().to_string()).collect()
}

/// The text of a pane's screen (the emulator's visible rows, joined).
fn screen_of(app: &App, id: PaneId) -> String {
    app.panes[&id].parser().screen().contents()
}

/// Headless: the emulator is fed the bytes directly (no program runs), and each frame is checked on the screen. The
/// pane's shell prints nothing after `wait_idle`, and the size never changes here.
#[test]
fn fullscreen_frames_leave_no_fragments() {
    let (w, h) = (110, 30);
    let mut app = demo_app(w, h);
    app.new_tab(std::env::temp_dir(), None, Some("fullscreen".into()));
    let id = app.tabs[0].focus;
    wait_idle(&mut app, id);
    let feed = |app: &mut App, bytes: &[u8]| app.panes[&id].parser().process(bytes);
    // A full frame: clear, then ten long rows.
    feed(&mut app, b"\x1b[?1049h\x1b[?1002h\x1b[?1006h\x1b[?2026h\x1b[H\x1b[2J");
    for r in 1..=10 {
        feed(&mut app, format!("\x1b[{r};1Hrow {r:02} {}", "=".repeat(90)).as_bytes());
    }
    feed(&mut app, b"\x1b[?2026l");
    assert_eq!(drawn_rows(&mut app, w, h)[..10], emulator_rows(&app, id)[..10], "the first frame");
    // The next frame: short rows, each erased with EL. Row 12 holds wide characters.
    feed(&mut app, b"\x1b[?2026h");
    for r in 1..=10 {
        feed(&mut app, format!("\x1b[{r};1HB{r:02}\x1b[K").as_bytes());
    }
    feed(&mut app, "\x1b[12;1H日本語 end\x1b[K".as_bytes());
    feed(&mut app, b"\x1b[?2026l");
    let drawn = drawn_rows(&mut app, w, h);
    assert_eq!(drawn, emulator_rows(&app, id), "the second frame");
    assert_eq!(drawn[11], "日本語 end");
    assert!(
        !drawn.iter().any(|r| r.contains("====")),
        "a row of the first frame is still drawn:\n{}",
        drawn.join("\n")
    );
    // Wide characters overwritten by narrow ones, erased by ECH, and ED from the middle of a row.
    feed(&mut app, b"\x1b[?2026h\x1b[H\x1b[2J");
    feed(&mut app, "\x1b[6;1H日本\x1b[6;2Hx".as_bytes());
    feed(&mut app, "\x1b[7;1H🙂🙂\x1b[7;3H\x1b[X".as_bytes());
    feed(&mut app, "\x1b[8;1H日本語\x1b[8;1H\x1b[2X".as_bytes());
    feed(&mut app, "\x1b[9;1HAAAAAAAAAAAAAAAAAAAAAAAAAA\x1b[9;5H\x1b[J".as_bytes());
    feed(&mut app, b"\x1b[?2026l");
    let drawn = drawn_rows(&mut app, w, h);
    assert_eq!(drawn, emulator_rows(&app, id), "the wide character frame");
    assert_eq!(&drawn[5..9], [" x本", "🙂", "  本語", "AAAA"], "{}", drawn.join("\n"));
    assert!(drawn[9..].iter().all(|r| r.is_empty()), "ED left rows below the cursor:\n{}", drawn.join("\n"));
    app.run(noble::keys::Action::CloseTab);
}

/// One row of the fake fullscreen app: `F<n> r<row> c<cols>`. Even rows are padded to the full width, odd rows end
/// early, so a fragment left from a longer row shows.
fn frame_line(n: u32, row: u16, cols: u16) -> String {
    let mut s = format!("F{n} r{row:02} c{cols}");
    if row.is_multiple_of(2) {
        let pad = usize::from(cols).saturating_sub(s.len());
        s.push_str(&".".repeat(pad));
    }
    s
}

/// Rows a frame draws: a full frame (odd) fills the screen, a partial one (even) draws the top half and erases
/// the rest with ED (`CSI J`). Every row of a full frame is erased with EL (`CSI 2 K`) before it is written.
fn frame_rows(n: u32, rows: u16) -> u16 {
    if n % 2 == 1 { rows } else { rows / 2 }
}

/// The program of `fullscreen_redraw_after_resize_leaves_no_fragments`: enters the alternate screen with mouse
/// tracking, and draws a frame each time its size changes, reading the size like a TUI does on SIGWINCH.
#[test]
#[ignore = "a child program of fullscreen_redraw_after_resize_leaves_no_fragments"]
fn fullscreen_child() {
    let mut out = std::io::stdout();
    write!(out, "\x1b[?1049h\x1b[?1002h\x1b[?1006h").unwrap();
    out.flush().unwrap();
    let (mut n, mut last) = (0u32, None);
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(60) {
        let size = crossterm::terminal::size().ok();
        if let Some((cols, rows)) = size.filter(|s| Some(*s) != last) {
            n += 1;
            write!(out, "\x1b[?2026h").unwrap();
            if n % 2 == 1 {
                write!(out, "\x1b[H\x1b[2J").unwrap();
            }
            for r in 1..=frame_rows(n, rows) {
                write!(out, "\x1b[{r};1H\x1b[2K{}", frame_line(n, r, cols)).unwrap();
            }
            if n % 2 == 0 {
                write!(out, "\x1b[{};1H\x1b[J", frame_rows(n, rows) + 1).unwrap();
            }
            write!(out, "\x1b[?2026l").unwrap();
            out.flush().unwrap();
            last = size;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Resizes the pane through several sizes while the child is in the alternate screen: after each resize the drawn
/// pane is exactly the child's last frame at the new size (rows below a partial frame are empty).
#[test]
fn fullscreen_redraw_after_resize_leaves_no_fragments() {
    let exe = std::env::current_exe().expect("the test binary").display().to_string();
    let mut app = demo_app(110, 30);
    app.shell = ShellSpec::new(exe, vec!["--ignored".into(), "--nocapture".into(), "fullscreen_child".into()]);
    app.run(noble::keys::Action::NewTab);
    let id = app.tabs[0].focus;
    pump_until(&mut app, "the first frame", |app| screen_of(app, id).contains("F1 r01"));
    for (w, h) in [(110, 30), (100, 24), (70, 20), (120, 40), (60, 12), (110, 30)] {
        render(&mut app, w, h);
        let inner = find_hit_inner(&app).expect("pane inner");
        let (iw, ih) = (inner.width, inner.height);
        // The frame of this size is complete when the emulator shows exactly what the child drew for it (its number is
        // on the first row). Frames of the earlier sizes never match: their width is different.
        let deadline = Instant::now() + Duration::from_secs(30);
        let expected = loop {
            app.pump();
            app.tick();
            let screen = emulator_rows(&app, id);
            let n: u32 = screen[0].split(' ').next().and_then(|f| f.trim_start_matches('F').parse().ok()).unwrap_or(0);
            let content = frame_rows(n, ih);
            let expected: Vec<String> = (1..=ih)
                .map(|r| if r <= content { frame_line(n, r, iw).trim_end().to_string() } else { String::new() })
                .collect();
            if n > 0 && screen == expected {
                break expected;
            }
            assert!(Instant::now() < deadline, "no complete frame at {iw}x{ih} arrived:\n{}", screen.join("\n"));
            std::thread::sleep(Duration::from_millis(20));
        };
        // What is drawn is what the emulator holds, and no fragment of an earlier frame is left.
        assert_eq!(drawn_rows(&mut app, w, h), expected, "fragments of an earlier frame after the resize to {iw}x{ih}");
    }
    app.run(noble::keys::Action::CloseTab);
    app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
}

/// The program of `resize_inside_a_synchronized_block_draws_the_last_frame`. Each Enter key from NOBLE starts a frame
/// (its first half, at the size the program has then, inside a synchronized block) or ends one (the rest of it). The
/// block stays open until the next Enter. A long line is printed on the main screen before the alternate screen: a
/// resize rewraps it, which renumbers the main screen's lines while the alternate screen is up.
#[test]
#[ignore = "a child program of resize_inside_a_synchronized_block_draws_the_last_frame"]
fn fullscreen_block_child() {
    let mut out = std::io::stdout();
    write!(out, "{}\r\n", "m".repeat(200)).unwrap();
    write!(out, "\x1b[?1049h").unwrap();
    crossterm::terminal::enable_raw_mode().expect("raw mode");
    write!(out, "READY\r\n").unwrap();
    out.flush().unwrap();
    let mut n = 0u32;
    while wait_enter() {
        n += 1;
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        let half = rows / 2;
        write!(out, "\x1b[?2026h\x1b[H\x1b[2J").unwrap();
        for r in 1..=half {
            write!(out, "\x1b[{r};1H{}", frame_line(n, r, cols)).unwrap();
        }
        out.flush().unwrap();
        if !wait_enter() {
            break;
        }
        for r in half + 1..=rows {
            write!(out, "\x1b[{r};1H{}", frame_line(n, r, cols)).unwrap();
        }
        write!(out, "\x1b[?2026l").unwrap();
        out.flush().unwrap();
    }
    crossterm::terminal::disable_raw_mode().ok();
}

/// The block child's wait for NOBLE: an Enter key. `false` when none comes for a minute (the child then ends).
fn wait_enter() -> bool {
    use crossterm::event::{Event, KeyEventKind};
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(60) {
        if !crossterm::event::poll(Duration::from_millis(50)).unwrap_or(false) {
            continue;
        }
        if let Ok(Event::Key(k)) = crossterm::event::read()
            && k.kind == KeyEventKind::Press
            && k.code == KeyCode::Enter
        {
            return true;
        }
    }
    false
}

/// Sends an Enter key to the focused pane (`fullscreen_block_child` starts or ends a frame with it).
fn press_enter(app: &mut App) {
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
}

/// The pane's inner size as drawn last (columns, rows).
fn inner_size(app: &App) -> (u16, u16) {
    let inner = find_hit_inner(app).expect("pane inner");
    (inner.width, inner.height)
}

/// Waits until the pane's program has a synchronized block open.
fn wait_block_open(app: &mut App, id: PaneId) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.panes[&id].sync_wake().is_none() {
        app.pump();
        app.tick();
        assert!(Instant::now() < deadline, "no synchronized block opened:\n{}", screen_of(app, id));
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Waits until the pane's program has no synchronized block open any more (its last one ended).
fn wait_block_closed(app: &mut App, id: PaneId) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.panes[&id].sync_wake().is_some() {
        app.pump();
        app.tick();
        assert!(Instant::now() < deadline, "the synchronized block never ended:\n{}", screen_of(app, id));
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The rows of frame `n` as the child drew it at `cols` columns, cut to what a `width` x `height` screen shows of it
/// (from the top left). Every row of a block frame is drawn, so every row has its text.
fn frame_text(n: u32, cols: u16, width: u16, height: u16) -> Vec<String> {
    (1..=height)
        .map(|r| frame_line(n, r, cols).chars().take(usize::from(width)).collect::<String>().trim_end().to_string())
        .collect()
}

/// Waits until what is drawn at `size` is `expected` (the rows of a complete frame).
fn wait_frame(app: &mut App, size: (u16, u16), expected: Vec<String>, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        app.pump();
        app.tick();
        let drawn = drawn_rows(app, size.0, size.1);
        if drawn == expected {
            return;
        }
        assert!(Instant::now() < deadline, "{what} never drawn:\n{}", drawn.join("\n"));
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Waits until what is drawn at `size` is the pane's screen. The screen is read again after the draw: output still
/// arriving after a block ended (ConPTY repaints after a resize) changes the screen, so the check waits until it
/// settles.
fn wait_drawn_is_screen(app: &mut App, id: PaneId, size: (u16, u16), what: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        app.pump();
        app.tick();
        let before = emulator_rows(app, id);
        let drawn = drawn_rows(app, size.0, size.1);
        if emulator_rows(app, id) == before && drawn == before {
            return;
        }
        assert!(Instant::now() < deadline, "{what}: the drawn frame is not the screen:\n{}", drawn.join("\n"));
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A resize while the child holds a synchronized block open: NOBLE keeps drawing the last complete frame (cut or
/// padded to the new size) until the block ends, never the half of the new frame that has arrived, and then the
/// frame the program draws at the new size. The block is allowed to stay open past `SYNC_TIMEOUT` here, so the test
/// decides when it ends: past the timeout NOBLE draws the live screen, by design.
#[test]
fn resize_inside_a_synchronized_block_draws_the_last_frame() {
    let exe = std::env::current_exe().expect("the test binary").display().to_string();
    let mut app = demo_app(110, 30);
    app.shell = ShellSpec::new(exe, vec!["--ignored".into(), "--nocapture".into(), "fullscreen_block_child".into()]);
    app.run(noble::keys::Action::NewTab);
    let id = app.tabs[0].focus;
    app.panes[&id].parser().callbacks_mut().set_sync_timeout(Some(Duration::from_secs(60)));
    wait_on_screen(&mut app, id, "READY");
    render(&mut app, 110, 30);
    press_enter(&mut app);
    wait_block_open(&mut app, id);
    press_enter(&mut app);
    wait_block_closed(&mut app, id);
    let (iw, ih) = inner_size(&app);
    wait_frame(&mut app, (110, 30), frame_text(1, iw, iw, ih), "the first frame");
    // `last` is the newest frame drawn in full. Each round: a resize, a frame started at that size, a second resize
    // while its block is open, the end of the block, and one more frame at the second size.
    let mut last = 1u32;
    for (first, second) in [((100, 24), (70, 20)), ((120, 40), (60, 12)), ((110, 30), (90, 22))] {
        render(&mut app, first.0, first.1);
        // What the pane shows before the block: the last complete frame, at the first size.
        let before = drawn_rows(&mut app, first.0, first.1);
        press_enter(&mut app);
        wait_block_open(&mut app, id);
        let epoch = app.panes[&id].parser().screen().reflow_epoch();
        render(&mut app, second.0, second.1);
        assert_ne!(
            app.panes[&id].parser().screen().reflow_epoch(),
            epoch,
            "the resize did not rewrap the main screen, so the test does not cover that case"
        );
        let (iw2, ih2) = inner_size(&app);
        // Inside the block: that frame, cut to the new size, and no half of the frame being written.
        let cut: Vec<String> = (0..ih2)
            .map(|r| {
                let row = before.get(usize::from(r)).map_or("", String::as_str);
                row.chars().take(usize::from(iw2)).collect::<String>().trim_end().to_string()
            })
            .collect();
        let held = drawn_rows(&mut app, second.0, second.1);
        assert!(
            !held.iter().any(|r| r.contains(&format!("F{} ", last + 1))),
            "a half-written frame was drawn inside the block at {iw2}x{ih2}:\n{}",
            held.join("\n")
        );
        assert_eq!(held, cut, "inside the block the last complete frame is not drawn, cut to {iw2}x{ih2}");
        // The program ends the block; what is drawn is then the screen itself.
        press_enter(&mut app);
        wait_block_closed(&mut app, id);
        wait_drawn_is_screen(&mut app, id, second, &format!("after the block at {iw2}x{ih2}"));
        // The next frame, started at the new size, is drawn in full.
        press_enter(&mut app);
        wait_block_open(&mut app, id);
        press_enter(&mut app);
        wait_block_closed(&mut app, id);
        wait_frame(&mut app, second, frame_text(last + 2, iw2, iw2, ih2), &format!("the frame at {iw2}x{ih2}"));
        last += 2;
    }
    app.run(noble::keys::Action::CloseTab);
    app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
}

/// The programs of `mouse_reports_reach_the_program` and `wheel_is_arrow_keys_on_the_alternate_screen`: enter the
/// alternate screen (with mouse capture when `mouse`), print READY, then one line per mouse or key event it reads.
fn input_child(mouse: bool) {
    use crossterm::event::{DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind};
    let mut out = std::io::stdout();
    crossterm::terminal::enable_raw_mode().expect("raw mode");
    write!(out, "\x1b[?1049h").unwrap();
    if mouse {
        crossterm::execute!(out, EnableMouseCapture).expect("mouse capture");
    }
    write!(out, "READY\r\n").unwrap();
    out.flush().unwrap();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(40) {
        if !crossterm::event::poll(Duration::from_millis(50)).unwrap_or(false) {
            continue;
        }
        match crossterm::event::read() {
            Ok(Event::Mouse(m)) => {
                write!(out, "EV {:?} {},{} m{}\r\n", m.kind, m.column, m.row, m.modifiers.bits()).unwrap();
            }
            Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => {
                write!(out, "KEY {:?}\r\n", k.code).unwrap();
            }
            _ => {}
        }
        out.flush().unwrap();
    }
    if mouse {
        crossterm::execute!(out, DisableMouseCapture).ok();
    }
    crossterm::terminal::disable_raw_mode().ok();
}

/// The program of `mouse_reports_reach_the_program`.
#[test]
#[ignore = "a child program of mouse_reports_reach_the_program"]
fn mouse_child() {
    input_child(true);
}

/// The program of `wheel_is_arrow_keys_on_the_alternate_screen`: no mouse tracking, only the alternate screen.
#[test]
#[ignore = "a child program of wheel_is_arrow_keys_on_the_alternate_screen"]
fn alternate_screen_keys_child() {
    input_child(false);
}

/// Waits until `needle` is on the pane's screen.
fn wait_on_screen(app: &mut App, id: PaneId, needle: &str) {
    let deadline = Instant::now() + Duration::from_secs(40);
    while !screen_of(app, id).contains(needle) {
        app.pump();
        app.tick();
        assert!(Instant::now() < deadline, "'{needle}' never arrived:\n{}", screen_of(app, id));
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Starts `program` in a new tab, waits for its READY line and returns the pane with the pane's inner area.
fn start_input_child(program: &str) -> (App, PaneId, ratatui::layout::Rect) {
    let exe = std::env::current_exe().expect("the test binary").display().to_string();
    let mut app = demo_app(110, 30);
    app.shell = ShellSpec::new(exe, vec!["--ignored".into(), "--nocapture".into(), program.into()]);
    app.run(noble::keys::Action::NewTab);
    let id = app.tabs[0].focus;
    wait_on_screen(&mut app, id, "READY");
    render(&mut app, 110, 30);
    let inner = find_hit_inner(&app).expect("pane inner");
    (app, id, inner)
}

/// Sends a mouse event with modifiers to the app (a wheel notch, a press, a drag, a release).
fn send_mouse(app: &mut App, kind: crossterm::event::MouseEventKind, x: u16, y: u16, mods: KeyModifiers) {
    use crossterm::event::{Event, MouseEvent};
    app.handle(AppEvent::Input(Event::Mouse(MouseEvent { kind, column: x, row: y, modifiers: mods })));
}

/// Wheel, clicks, drags and modifiers reach a program that enabled mouse tracking, at its own cells (relative to the
/// pane). With shift the mouse is NOBLE's: the press starts a selection and the program gets nothing.
#[test]
fn mouse_reports_reach_the_program() {
    use crossterm::event::{MouseButton, MouseEventKind as K};
    let (mut app, id, inner) = start_input_child("mouse_child");
    let at = |x: u16, y: u16| (inner.x + x, inner.y + y);
    let (x, y) = at(3, 2);
    send_mouse(&mut app, K::ScrollUp, x, y, KeyModifiers::NONE);
    wait_on_screen(&mut app, id, "EV ScrollUp 3,2 m0");
    let (x, y) = at(5, 4);
    send_mouse(&mut app, K::Down(MouseButton::Left), x, y, KeyModifiers::NONE);
    let (x2, y2) = at(9, 4);
    send_mouse(&mut app, K::Drag(MouseButton::Left), x2, y2, KeyModifiers::NONE);
    send_mouse(&mut app, K::Up(MouseButton::Left), x2, y2, KeyModifiers::NONE);
    wait_on_screen(&mut app, id, "EV Down(Left) 5,4 m0");
    wait_on_screen(&mut app, id, "EV Drag(Left) 9,4 m0");
    wait_on_screen(&mut app, id, "EV Up(Left) 9,4 m0");
    // The right button: its press, its drag and its release all reach the program (a release is what many
    // programs act on, so a lost one would leave a click half done).
    let (x, y) = at(1, 6);
    send_mouse(&mut app, K::Down(MouseButton::Right), x, y, KeyModifiers::NONE);
    wait_on_screen(&mut app, id, "EV Down(Right) 1,6 m0");
    let (x2, y2) = at(4, 6);
    send_mouse(&mut app, K::Drag(MouseButton::Right), x2, y2, KeyModifiers::NONE);
    wait_on_screen(&mut app, id, "EV Drag(Right) 4,6 m0");
    send_mouse(&mut app, K::Up(MouseButton::Right), x2, y2, KeyModifiers::NONE);
    wait_on_screen(&mut app, id, "EV Up(Right) 4,6 m0");
    // Shift+press and shift+drag: NOBLE selects, the program gets nothing.
    let (x, y) = at(6, 8);
    send_mouse(&mut app, K::Down(MouseButton::Left), x, y, KeyModifiers::SHIFT);
    let (x2, y2) = at(12, 8);
    send_mouse(&mut app, K::Drag(MouseButton::Left), x2, y2, KeyModifiers::SHIFT);
    let selection = app.panes[&id].selection.expect("shift+drag selects");
    assert_eq!(selection.head, (8, 12), "the selection follows the pointer");
    send_mouse(&mut app, K::Up(MouseButton::Left), x2, y2, KeyModifiers::SHIFT);
    // A plain press with the control key on it (no link under it): the program gets it with the control bit.
    send_mouse(&mut app, K::Down(MouseButton::Left), at(2, 9).0, at(2, 9).1, KeyModifiers::CONTROL);
    wait_on_screen(&mut app, id, "EV Down(Left) 2,9 m2");
    run_for(&mut app, Duration::from_millis(300));
    let screen = screen_of(&app, id);
    assert!(!screen.contains("EV Down(Left) 6,8") && !screen.contains("EV Drag(Left) 12,8"), "{screen}");
    app.run(noble::keys::Action::CloseTab);
    app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
}

/// On the alternate screen without mouse tracking, a wheel notch becomes three arrow keys (cursor keys, as `less`
/// and `man` take them) and nothing else reaches the program.
#[test]
fn wheel_is_arrow_keys_on_the_alternate_screen() {
    use crossterm::event::MouseEventKind as K;
    let (mut app, id, inner) = start_input_child("alternate_screen_keys_child");
    let (x, y) = (inner.x + 3, inner.y + 2);
    send_mouse(&mut app, K::ScrollUp, x, y, KeyModifiers::NONE);
    send_mouse(&mut app, K::ScrollDown, x, y, KeyModifiers::NONE);
    run_for(&mut app, Duration::from_millis(600));
    let screen = screen_of(&app, id);
    assert_eq!(screen.matches("KEY Up").count(), 3, "{screen}");
    assert_eq!(screen.matches("KEY Down").count(), 3, "{screen}");
    assert!(!screen.contains("EV "), "a mouse report reached a program without mouse tracking:\n{screen}");
    app.run(noble::keys::Action::CloseTab);
    app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
}
