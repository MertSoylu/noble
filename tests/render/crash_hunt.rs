//! Degenerate sizes, random input and escape sequences must never panic.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use noble::ai::{Presence, ProviderState, Status, Usage, Window};
use noble::app::{App, View};
use noble::event::AppEvent;
use noble::keys::Action;
use noble::sensors::{DiskInfo, ProcInfo, SensorSample};

use super::common::*;

// ─── Crash hunt: tiny screens, random input, random escape sequences ─────────────

/// Deterministic xorshift generator: a failing run is reproduced by its seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    /// The fixed seed, or `NOBLE_FUZZ_SEED` to explore further (a failure prints the seed to reuse).
    fn seeded(default: u64) -> Rng {
        let seed = std::env::var("NOBLE_FUZZ_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(default);
        eprintln!("fuzz seed {seed}");
        Rng(seed.max(1))
    }
}

/// Iterations of a fuzz loop: `NOBLE_FUZZ_SCALE` multiplies them for a longer run.
fn fuzz_steps(base: usize) -> usize {
    base * std::env::var("NOBLE_FUZZ_SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or(1)
}

/// Every screen and overlay is drawn at degenerate sizes without panicking.
#[test]
fn every_screen_survives_tiny_sizes() {
    let mut app = demo_app(80, 24);
    app.new_tab(std::env::temp_dir(), None, Some("tiny".into()));
    app.run(Action::SplitRight);
    app.run(Action::SplitDown);
    let screens: [&dyn Fn(&mut App); 17] = [
        &|a| a.show_welcome(),
        &|a| a.open_scheme_picker(),
        &|a| a.open_launcher_picker(),
        &|a| a.open_tab_menu(0, 500, 500),
        &|a| {
            let path = a.projects[0].path.clone();
            a.open_project_menu(path, 0, 0)
        },
        &|a| a.run(Action::Bridge),
        &|a| a.run(Action::System),
        &|a| a.run(Action::Settings),
        &|a| a.run(Action::GoTab(1)),
        &|a| a.run(Action::Palette),
        &|a| a.run(Action::Help),
        &|a| a.run(Action::RenameTab),
        &|a| a.run(Action::PaneMenu),
        &|a| a.run(Action::Search),
        &|a| a.run(Action::Quit),
        &|a| a.run(Action::Zoom),
        &|a| {
            a.run(Action::Bridge);
            a.on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
            a.on_key(KeyEvent::new(KeyCode::Char('é'), KeyModifiers::NONE));
        },
    ];
    for open in screens {
        app.overlay = None;
        app.search = None;
        open(&mut app);
        for (w, h) in TINY {
            render(&mut app, w, h);
            // A resize event at that size, then the frame after it.
            app.handle(AppEvent::Input(crossterm::event::Event::Resize(w, h)));
            render(&mut app, w, h);
        }
    }
    app.overlay = None;
    app.run(Action::CloseTab);
}

/// Random keys, mouse events, paste and resizes (tiny sizes included) never panic.
/// Only input that stays inside the app is generated: no Enter (it would run a shell command or
/// a palette/menu item), no clicks that open external programs, no clipboard writes.
#[test]
fn random_input_never_panics() {
    use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
    use noble::app::{Hit, Overlay, SettingItem};
    let mut app = demo_app(100, 30);
    app.cfg.terminal.copy_on_select = false;
    let mut rng = Rng::seeded(0x00c0_ffee_d00d_f00d);
    let texts = ["é", "日本語", "🙂", "e\u{301}", "İ", "\u{200b}", "a b", "ß", "\t", "ǅ", "x", "/", "..", "\u{202e}"];
    let actions: Vec<Action> = Action::ALL
        .into_iter()
        .filter(|a| !matches!(a, Action::Quit | Action::OpenConfig | Action::ReloadConfig | Action::Update))
        .collect();
    let keys = [
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Tab,
        KeyCode::BackTab,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::Esc,
        KeyCode::Backspace,
        KeyCode::Delete,
        KeyCode::F(1),
    ];
    let mods = [KeyModifiers::NONE, KeyModifiers::SHIFT, KeyModifiers::ALT, KeyModifiers::CONTROL];
    // A press that starts an external program (an editor, the file manager, a browser) or writes the
    // headless (relative) config path is never sent: neither a left click nor a right click (which steps a
    // Settings row and runs the buttons too).
    fn allowed(app: &App, hit: &Hit, spawn_ok: bool) -> bool {
        let config_row =
            |i: usize| matches!(app.settings_items().get(i), Some(SettingItem::OpenConfig | SettingItem::ReloadConfig));
        !matches!(
            hit,
            Hit::Launcher(_)
                | Hit::MenuItem(_)
                | Hit::PaletteItem(_)
                | Hit::OpenFiles
                | Hit::OpenSelected
                | Hit::Update
                | Hit::ProjectPr
                | Hit::Port(_)
        ) && !matches!(hit, Hit::Setting(i) if config_row(*i))
            && (spawn_ok || !matches!(hit, Hit::NewTab | Hit::PaneSplit { .. }))
    }
    // The press goes to the topmost target under the point (`hit_at`), which can be another one than the
    // target picked (a Settings row under an overlay's backdrop).
    fn press_is_safe(app: &App, x: u16, y: u16, spawn_ok: bool) -> bool {
        let at = ratatui::layout::Position { x, y };
        app.hits.iter().rev().find(|(r, _)| r.contains(at)).is_none_or(|(_, hit)| allowed(app, hit, spawn_ok))
    }
    let (mut w, mut h) = (100u16, 30u16);
    for step in 0..fuzz_steps(4000) {
        // A random click on the Settings row could turn it back on (and write the real clipboard).
        app.cfg.terminal.copy_on_select = false;
        let spawn_ok = app.pane_count() < 3;
        // Where typed text lands in a text field (elsewhere letters and space are shortcuts, which
        // could run "Open config" or a launcher). A filter flag only counts on its own screen.
        let text_field = match (&app.overlay, app.view) {
            (Some(o), _) => matches!(o, Overlay::Palette(_) | Overlay::Prompt(_)),
            (None, View::Bridge) => app.bridge.filtering,
            (None, View::System) => app.system.filtering,
            (None, View::Term(_)) => true,
            (None, View::Settings) => false,
        };
        match rng.below(10) {
            0 => {
                (w, h) = if rng.below(3) == 0 {
                    *rng.pick(&TINY)
                } else {
                    (20 + rng.below(160) as u16, 5 + rng.below(50) as u16)
                };
                app.handle(AppEvent::Input(Event::Resize(w, h)));
            }
            1 => {
                let a = *rng.pick(&actions);
                let spawns = matches!(a, Action::NewTab | Action::SplitRight | Action::SplitDown);
                if spawn_ok || !spawns {
                    app.run(a);
                }
            }
            2 | 3 => app.on_key(KeyEvent::new(*rng.pick(&keys), *rng.pick(&mods))),
            4 if text_field => {
                let t = *rng.pick(&texts);
                // A multi-line paste into a shell would run it: pasted only into NOBLE's own fields.
                if rng.below(4) == 0 && !matches!((&app.overlay, app.view), (None, View::Term(_))) {
                    app.handle(AppEvent::Input(Event::Paste(format!("{t}\n{t}"))));
                } else {
                    for c in t.chars() {
                        app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
                    }
                }
            }
            // Characters open the filter and reach the shell (never followed by Enter); elsewhere
            // letters are shortcuts that may start launchers or external programs.
            4 => {
                let c = *rng.pick(&['/', 'é', '日', 'x']);
                if matches!(app.view, View::Term(_)) || c == '/' {
                    app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
                }
            }
            5 | 6 => {
                let kind = *rng.pick(&[
                    MouseEventKind::Moved,
                    MouseEventKind::ScrollUp,
                    MouseEventKind::ScrollDown,
                    MouseEventKind::Down(MouseButton::Right),
                    MouseEventKind::Up(MouseButton::Right),
                    MouseEventKind::Up(MouseButton::Left),
                    MouseEventKind::Drag(MouseButton::Left),
                ]);
                let (x, y) = (rng.below(w as usize + 4) as u16, rng.below(h as usize + 4) as u16);
                // An unsafe press becomes a plain move (the step still draws a frame).
                let kind = if matches!(kind, MouseEventKind::Down(_)) && !press_is_safe(&app, x, y, spawn_ok) {
                    MouseEventKind::Moved
                } else {
                    kind
                };
                app.handle(AppEvent::Input(Event::Mouse(MouseEvent {
                    kind,
                    column: x,
                    row: y,
                    modifiers: *rng.pick(&[KeyModifiers::NONE, KeyModifiers::SHIFT]),
                })));
            }
            7 | 8 => {
                // Left press on a random safe target (at a random point inside it), sometimes dragged.
                // "Open config" starts an editor and "Reload config" writes the headless (relative) path.
                let targets: Vec<ratatui::layout::Rect> =
                    app.hits.iter().filter(|(_, hit)| allowed(&app, hit, spawn_ok)).map(|(r, _)| *r).collect();
                let point = (!targets.is_empty()).then(|| {
                    let r = *rng.pick(&targets);
                    (r.x + rng.below(r.width as usize) as u16, r.y + rng.below(r.height as usize) as u16)
                });
                if let Some((x, y)) = point.filter(|&(x, y)| press_is_safe(&app, x, y, spawn_ok)) {
                    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
                    if rng.below(2) == 0 {
                        let (dx, dy) = (rng.below(w as usize + 2) as u16, rng.below(h as usize + 2) as u16);
                        mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), dx, dy);
                        mouse(&mut app, MouseEventKind::Up(MouseButton::Left), dx, dy);
                    } else {
                        mouse(&mut app, MouseEventKind::Up(MouseButton::Left), x, y);
                    }
                }
            }
            _ => app.pump(),
        }
        // Clicks resolve against the hits of the last frame, like in the real loop.
        render(&mut app, w, h);
        if step % 500 == 0 {
            app.tick();
        }
    }
    while !app.tabs.is_empty() {
        app.remove_tab(0);
    }
}

/// Hostile values from outside (git output, provider APIs, sensors, the battery): non-ASCII and
/// control characters, extreme timestamps, percentages over 100, NaN. Home, System and the
/// notifications must still draw at every size.
#[test]
fn hostile_external_data_renders() {
    let mut app = demo_app(160, 45);
    let weird = "é日🙂\u{301}\u{202e}\t\x1b[31m\u{0}İ";
    let status = format!(
        "## {weird}...origin/{weird} [ahead 99999999999, behind x]\n M {weird}\n?? \"{weird} -> ü\"\nR  a -> {weird}\nXé\n\u{fffd}\n"
    );
    let mut git = noble::projects::parse_status(&status);
    git.last_commit = Some(i64::MIN);
    git.last_subject = Some(weird.repeat(20));
    git.commits = [i64::MIN, i64::MAX, 0, -1]
        .into_iter()
        .map(|time| noble::projects::Commit {
            hash: weird.into(),
            time,
            author: weird.into(),
            subject: weird.repeat(30),
        })
        .collect();
    git.commits
        .extend(noble::projects::parse_log(&format!("{weird}\u{1f}-9223372036854775808\u{1f}{weird}\u{1f}{weird}")));
    for i in 0..app.projects.len() {
        let path = app.projects[i].path.clone();
        app.handle(AppEvent::Git(path, git.clone()));
    }
    for (used, resets_at, fetched_at) in
        [(255, Some(i64::MIN), Some(i64::MIN)), (101, Some(i64::MAX), Some(i64::MAX)), (100, Some(0), None)]
    {
        // Ok: recorded in the usage history and checked for the quota warning; Error: shown as is.
        for (id, status) in [("claude", Status::Ok), ("codex", Status::Error(weird.into()))] {
            app.handle(AppEvent::Ai(Box::new(ProviderState {
                id,
                name: "Claude Code",
                login_hint: "claude",
                presence: Presence::Ready,
                status,
                usage: Some(Usage {
                    windows: ["5H", "WEEK", weird, ""]
                        .iter()
                        .map(|l| Window { label: l.to_string(), used, resets_at })
                        .collect(),
                    plan: Some(weird.into()),
                    note: Some(weird.into()),
                }),
                fetched_at,
            })));
        }
        app.handle(AppEvent::Sensors(Box::new(SensorSample {
            cpu: f32::NAN,
            cores: vec![f32::INFINITY, -5.0, 250.0],
            freq_mhz: u64::MAX,
            mem_used: u64::MAX,
            mem_total: 0,
            swap_used: 5,
            swap_total: 0,
            rx_rate: f64::NAN,
            tx_rate: f64::INFINITY,
            disks: vec![DiskInfo { mount: weird.into(), total: 0, used: u64::MAX }],
            procs: vec![ProcInfo { pid: 1, name: weird.into(), cpu: f32::NAN, mem: u64::MAX }],
            proc_count: usize::MAX,
            uptime: u64::MAX,
            battery: Some(noble::battery::Battery {
                percent: [f32::NAN, 250.0, -3.0][used as usize % 3],
                state: noble::battery::PowerState::Discharging,
                secs_left: Some(u64::MAX),
                secs_to_full: Some(u64::MAX),
            }),
        })));
        for view in [Action::Bridge, Action::System] {
            app.run(view);
            for (w, h) in SIZES.into_iter().chain(TINY) {
                render(&mut app, w, h);
            }
        }
    }
}

/// Random and malformed escape sequences (and invalid UTF-8) fed to a real pane's emulator,
/// with the pane drawn and resized in between, never panic.
#[test]
fn random_escape_sequences_never_panic() {
    let mut app = demo_app(80, 24);
    app.new_tab(std::env::temp_dir(), None, Some("escapes".into()));
    let id = app.tabs[0].focus;
    wait_idle(&mut app, id);
    let pieces: [&[u8]; 58] = [
        b"\x1b[",
        b"\x1b]",
        b"\x1bP",
        b"\x1b",
        b"\x1b[?",
        b"\x07",
        b"\x1b\\",
        b";",
        b":",
        b"0",
        b"1",
        b"9",
        b"65535",
        b"99999999999",
        b"-1",
        b"m",
        b"H",
        b"J",
        b"K",
        b"r",
        b"h",
        b"l",
        b"@",
        b"L",
        b"M",
        b"P",
        b"X",
        b"S",
        b"T",
        b"G",
        b"d",
        b"b",
        b"t",
        b"7;file://h\xc3\xa9/\xe6\x97\xa5/%zz%e9%",
        b"7;file://",
        b"9;9;\"C:\\x\xff\"",
        b"9;4;3;",
        b"8;;https://\xe6\x97\xa5",
        b"8;id=\xff;",
        b"133;A",
        b"133;D;\xff",
        b"777;notify;\xf0\x9f;",
        b"0;title \xf0\x9f\x99\x82",
        b"2;",
        b"\xff",
        b"\xc3",
        b"\xe6\x97",
        "日本".as_bytes(),
        "e\u{301}\u{200d}".as_bytes(),
        b"\r\n",
        b"\x08\x08\x08",
        b"\t",
        b"\x1b[?1049h",
        b"\x1b[?1049l",
        b"\x1b[?1000h\x1b[?1006h",
        b"\x1b[6n",
        b"\x1bc",
        b"\x1b#8",
    ];
    let mut rng = Rng::seeded(0x1234_5678_9abc_def1);
    for round in 0..fuzz_steps(600) {
        let mut chunk = Vec::new();
        for _ in 0..rng.below(48) {
            if rng.below(6) == 0 {
                chunk.push(rng.next() as u8);
            } else {
                chunk.extend_from_slice(rng.pick::<&[u8]>(&pieces));
            }
        }
        {
            let pane = &app.panes[&id];
            let mut p = pane.parser();
            // As in the PTY reader thread: an emulator panic skips the chunk (vt100 has a few on
            // hostile input); what matters here is that the app keeps working with that state.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.process(&chunk)));
            // Replies go nowhere, and no system clipboard write can come from here.
            p.callbacks_mut().responses.clear();
            p.callbacks_mut().clipboard = None;
            pane.dirty.store(true, std::sync::atomic::Ordering::Release);
        }
        app.handle(AppEvent::PtyOutput);
        let (w, h) =
            if round % 7 == 0 { *rng.pick(&TINY) } else { (20 + rng.below(140) as u16, 4 + rng.below(40) as u16) };
        render(&mut app, w, h);
        if round % 50 == 0 {
            // Search through whatever landed in the scrollback.
            app.run(Action::Search);
            for c in ["é", "日", "\u{301}", "x"][rng.below(4)].chars() {
                app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
            }
            render(&mut app, w, h);
            app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        }
    }
    app.run(Action::CloseTab);
}
