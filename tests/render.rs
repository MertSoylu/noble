//! Headless render tests: every screen is drawn at many sizes without panicking
//! and text dumps are written under `target/audit/` (for visual review).

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use noble::ai::{Presence, ProviderState, Status, Usage, Window};
use noble::app::{App, View};
use noble::config::Config;
use noble::event::AppEvent;
use noble::keys::Action;
use noble::projects::{GitInfo, Project};
use noble::sensors::{DiskInfo, ProcInfo, SensorSample, StaticInfo};
use noble::store::{SavedTab, Workspace};
use noble::term::layout::SavedNode;

fn audit_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("audit");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn render(app: &mut App, w: u16, h: u16) -> String {
    app.size = (w, h);
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| noble::ui::draw(f, app)).unwrap();
    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..h {
        let mut skip = 0usize;
        for x in 0..w {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let sym = buf[(x, y)].symbol();
            out.push_str(sym);
            skip = unicode_width::UnicodeWidthStr::width(sym).saturating_sub(1);
        }
        out.push('\n');
    }
    out
}

fn save(name: &str, text: &str) {
    std::fs::write(audit_dir().join(format!("{name}.txt")), text).unwrap();
}

fn demo_app(w: u16, h: u16) -> App {
    // Snapshots must not land on an in-between frame; the animations have their own tests.
    let mut cfg = Config::default();
    cfg.general.animations = false;
    let mut app = App::headless(cfg, (w, h));
    app.operator = "mert".into();
    app.handle(AppEvent::SensorStatic(Box::new(StaticInfo {
        host: "DESKTOP-SM3V".into(),
        os: "Windows 11 Home".into(),
        cpu_brand: "12th Gen Intel(R) Core(TM) i7-12700H".into(),
        cores: 12,
        total_mem: 32 * 1024 * 1024 * 1024,
    })));
    let procs: Vec<ProcInfo> = [
        ("code.exe", 12.4, 820u64),
        ("msedge.exe", 8.1, 640),
        ("noble.exe", 2.3, 24),
        ("pwsh.exe", 1.2, 96),
        ("explorer.exe", 0.8, 180),
        ("claude.exe", 0.6, 210),
        ("svchost.exe", 0.2, 40),
        ("Discord.exe", 0.1, 300),
    ]
    .iter()
    .enumerate()
    .map(|(i, (n, c, m))| ProcInfo { pid: 1000 + i as u32 * 17, name: n.to_string(), cpu: *c, mem: m * 1024 * 1024 })
    .collect();
    for i in 0..120 {
        let t = i as f32 / 8.0;
        app.handle(AppEvent::Sensors(Box::new(SensorSample {
            cpu: 22.0 + 18.0 * t.sin() + if i % 17 == 0 { 30.0 } else { 0.0 },
            cores: (0..12).map(|c| (c as f32 * 7.0 + i as f32 * 3.0) % 90.0).collect(),
            freq_mhz: 2700,
            mem_used: 11 * 1024 * 1024 * 1024 + (i as u64 * 7_000_000),
            mem_total: 32 * 1024 * 1024 * 1024,
            swap_used: 1_200_000_000,
            swap_total: 4_000_000_000,
            rx_rate: 900_000.0 * (1.0 + (t * 1.3).sin() as f64),
            tx_rate: 120_000.0 * (1.0 + (t * 0.7).cos() as f64),
            disks: vec![
                DiskInfo { mount: "C:\\".into(), total: 512 << 30, used: 311 << 30 },
                DiskInfo { mount: "D:\\".into(), total: 1024 << 30, used: 900 << 30 },
            ],
            procs: procs.clone(),
            proc_count: 312,
            uptime: 5 * 3600 + 8 * 60,
            battery: Some(noble::battery::Battery {
                percent: 76.0,
                state: noble::battery::PowerState::Discharging,
                secs_left: Some(3 * 3600 + 12 * 60),
                secs_to_full: None,
            }),
        })));
    }
    let now = SystemTime::now();
    let mk = |name: &str, branch: &str, mins: u64, git: Option<GitInfo>| Project {
        name: name.into(),
        path: PathBuf::from(format!("C:\\Users\\Mert\\Desktop\\{name}")),
        branch: Some(branch.into()),
        last_active: Some(now - Duration::from_secs(mins * 60)),
        git,
    };
    app.handle(AppEvent::Projects(vec![
        mk(
            "noble-rs",
            "main",
            3,
            Some(GitInfo {
                dirty: 3,
                untracked: 1,
                ahead: 1,
                behind: 0,
                branch: Some("main".into()),
                last_subject: Some("feat: HUD bridge".into()),
                last_commit: Some(chrono::Utc::now().timestamp() - 7200),
                commits: [
                    ("a1b2c3d", 7200, "feat: HUD bridge"),
                    ("9f8e7d6", 5 * 3600, "fix: hover buttons no longer leak text"),
                    ("4c5d6e7", 26 * 3600, "docs: README keys table"),
                    ("0badc0d", 3 * 86_400, "refactor: split app into modules"),
                ]
                .iter()
                .map(|(h, ago, s)| noble::projects::Commit {
                    hash: h.to_string(),
                    time: chrono::Utc::now().timestamp() - ago,
                    author: "Mert".into(),
                    subject: s.to_string(),
                })
                .collect(),
                changes: vec![
                    (" M".into(), "src/ui/bridge.rs".into()),
                    ("M ".into(), "src/app/menu.rs".into()),
                    ("??".into(), "tests/new.rs".into()),
                ],
            }),
        ),
        mk("api-server", "feature/auth-refresh", 90, Some(GitInfo { dirty: 0, ..Default::default() })),
        mk("dotfiles", "main", 60 * 26, Some(GitInfo { dirty: 1, behind: 2, ..Default::default() })),
        mk("blog", "master", 60 * 24 * 9, None),
        mk("Noble", "main", 60 * 5, Some(GitInfo { dirty: 54, ..Default::default() })),
        mk("scratch", "dev", 60 * 24 * 40, None),
    ]));
    let reset = chrono::Utc::now().timestamp() + 2 * 3600 + 14 * 60;
    let states = vec![
        ProviderState {
            id: "claude",
            name: "Claude Code",
            login_hint: "claude",
            presence: Presence::Ready,
            status: Status::Ok,
            usage: Some(Usage {
                windows: vec![
                    Window { label: "5H".into(), used: 71, resets_at: Some(reset) },
                    Window { label: "WEEK".into(), used: 28, resets_at: Some(reset + 4 * 86_400) },
                ],
                plan: Some("MAX".into()),
                note: None,
            }),
            fetched_at: Some(chrono::Utc::now().timestamp() - 120),
        },
        ProviderState {
            id: "codex",
            name: "Codex",
            login_hint: "codex login",
            presence: Presence::Ready,
            status: Status::Error("offline".into()),
            usage: Some(Usage {
                windows: vec![
                    Window { label: "5H".into(), used: 18, resets_at: Some(reset + 3600) },
                    Window { label: "WEEK".into(), used: 92, resets_at: Some(reset + 86_400) },
                ],
                plan: Some("PLUS".into()),
                note: None,
            }),
            fetched_at: Some(chrono::Utc::now().timestamp() - 3600),
        },
    ];
    for s in states {
        app.handle(AppEvent::Ai(Box::new(s)));
    }
    app.workspaces.upsert(Workspace {
        name: "daily".into(),
        saved_at: 0,
        tabs: vec![SavedTab {
            name: None,
            origin: "noble-rs".into(),
            layout: SavedNode::Leaf { cwd: "C:\\".into(), launch: None },
            focus: 0,
        }],
    });
    app.recent.record(&std::env::temp_dir());
    app
}

const SIZES: [(u16, u16); 7] = [(160, 45), (120, 34), (110, 30), (90, 28), (76, 24), (56, 18), (30, 8)];

#[test]
fn bridge_renders_at_all_sizes() {
    for (w, h) in SIZES {
        let mut app = demo_app(w, h);
        let text = render(&mut app, w, h);
        save(&format!("bridge-{w}x{h}"), &text);
        if w >= 56 {
            assert!(text.contains("Projects"), "{w}x{h}\n{text}");
            assert!(text.contains("noble-rs"), "{w}x{h}");
        }
        if w >= 92 && h >= 16 {
            assert!(text.contains("AI usage"), "{w}x{h}\n{text}");
            assert!(text.contains("System"));
            assert!(text.contains("Claude Code"));
            assert!(
                text.contains("PROJECT") && text.contains("STATUS"),
                "{w}x{h}
{text}"
            );
            assert!(
                text.contains("3 uncommitted changes (1 new) · 1 commit to push"),
                "{w}x{h}
{text}"
            );
            // The old process list was removed.
            assert!(!text.contains("code.exe"));
        }
    }
    let wide = render(&mut demo_app(160, 45), 160, 45);
    assert!(wide.contains("● 3 changed ↑1") && wide.contains("✓ clean") && wide.contains("● 1 changed ↓2"), "{wide}");
    // The list is short: below it the selected project's card (commits + changed files).
    assert!(wide.contains("RECENT COMMITS") && wide.contains("9f8e7d6 fix: hover buttons"), "{wide}");
    assert!(wide.contains("M src/ui/bridge.rs") && wide.contains("? tests/new.rs"), "{wide}");
    // With room to spare the System CPU graph grows (at least 4 rows of braille).
    let braille_rows = wide.lines().filter(|l| l.chars().any(|c| ('\u{2801}'..='\u{28ff}').contains(&c))).count();
    assert!(braille_rows >= 5, "{wide}");
    // No room for the card = not shown; a dirty project with no file list still does not say "clean".
    let small = render(&mut demo_app(56, 18), 56, 18);
    assert!(!small.contains("RECENT COMMITS"), "{small}");
    let mut app = demo_app(160, 45);
    app.bridge.proj_sel = app.visible_projects().iter().position(|i| app.projects[*i].name == "Noble").unwrap();
    let text = render(&mut app, 160, 45);
    assert!(text.contains("+54 more") && !text.contains("working tree clean"), "{text}");
    // Files that do not fit spread across columns, the rest become "+N more"; no overflow at any size.
    let idx = app.visible_projects()[app.bridge.proj_sel];
    if let Some(g) = app.projects[idx].git.as_mut() {
        g.changes = (0..40).map(|i| (" M".to_string(), format!("src/module_{i}.rs"))).collect();
    }
    let text = render(&mut app, 160, 45);
    save("bridge-card-many-160x45", &text);
    assert!(text.contains("src/module_0.rs") && text.contains(" more"), "{text}");
    for (w, h) in SIZES {
        render(&mut app, w, h);
    }
    app.bridge.proj_sel = 1;
    let text = render(&mut app, 160, 45);
    assert!(text.contains("✓ working tree clean"), "{text}");
    // Screen overflows: no panic even at very small sizes.
    for (w, h) in [(1, 1), (5, 3), (20, 4), (200, 3), (3, 60)] {
        let mut app = demo_app(w, h);
        render(&mut app, w, h);
    }
}

#[test]
fn system_and_overlays_render() {
    for (w, h) in SIZES {
        let mut app = demo_app(w, h);
        app.run(Action::System);
        assert_eq!(app.view, View::System);
        let text = render(&mut app, w, h);
        save(&format!("system-{w}x{h}"), &text);
        if w >= 90 {
            assert!(text.contains("PROCESSES"));
            assert!(text.contains("code.exe"));
        }
    }
    let mut app = demo_app(110, 30);
    app.run(Action::Palette);
    save("palette-110x30", &render(&mut app, 110, 30));
    for c in "theme".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    let text = render(&mut app, 110, 30);
    save("palette-filtered-110x30", &text);
    assert!(text.contains("Theme: Ice"));
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.overlay.is_none());

    app.run(Action::Help);
    let text = render(&mut app, 110, 30);
    save("help-110x30", &text);
    assert!(text.contains("KEYBOARD REFERENCE"));
    app.overlay = None;

    app.run(Action::Quit);
    save("quit-110x30", &render(&mut app, 110, 30));
}

#[test]
fn bridge_keyboard_flow() {
    let mut app = demo_app(120, 34);
    let key = |app: &mut App, c: KeyCode| app.on_key(KeyEvent::new(c, KeyModifiers::NONE));
    key(&mut app, KeyCode::Down);
    assert_eq!(app.bridge.proj_sel, 1);
    key(&mut app, KeyCode::Char('/'));
    for c in "dot".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    assert_eq!(app.visible_projects().len(), 1);
    assert_eq!(app.selected_project().unwrap().name, "dotfiles");
    save("bridge-filter-120x34", &render(&mut app, 120, 34));
    // AltGr (Ctrl+Alt on Windows) symbols are typed as text, not dropped as shortcuts.
    let altgr = KeyModifiers::CONTROL | KeyModifiers::ALT;
    for c in ['\\', '@'] {
        app.on_key(KeyEvent::new(KeyCode::Char(c), altgr));
    }
    assert!(app.bridge.filtering);
    assert_eq!(app.bridge.filter, "dot\\@");
    key(&mut app, KeyCode::Esc);
    assert!(app.bridge.filter.is_empty());
    // Prefix + unknown key: a warning, no crash.
    app.on_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    assert!(app.prefix_armed);
    let text = render(&mut app, 120, 34);
    save("bridge-prefix-120x34", &text);
    // Terminal-only hints (split, zoom) are not shown on Home.
    let bar = text.lines().last().unwrap_or_default();
    assert!(bar.contains("new tab") && bar.contains("commands"), "{bar}");
    assert!(!bar.contains("split") && !bar.contains("zoom"), "{bar}");
    key(&mut app, KeyCode::Char('y'));
    assert!(!app.prefix_armed);
    // Theme cycling.
    let before = app.theme.name;
    app.run(Action::CycleTheme);
    assert_ne!(before, app.theme.name);
    for t in noble::theme::theme_names() {
        app.set_theme(t);
        save(&format!("theme-{t}-110x30"), &render(&mut app, 110, 30));
    }
}

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
    // The split opens the new pane in the same directory.
    app.run(Action::SplitRight);
    let new = app.tabs[0].focus;
    assert_ne!(new, id);
    assert_eq!(std::fs::canonicalize(&app.panes[&new].start_cwd).unwrap(), want);
    app.run(Action::CloseTab);
    let _ = std::fs::remove_dir_all(&target);
}

#[test]
fn boot_sequence_renders() {
    for (ms, name) in [(300u64, "early"), (1200, "mid"), (1850, "late")] {
        let mut app = demo_app(110, 30);
        app.boot = Some(noble::app::Boot { started: std::time::Instant::now() - Duration::from_millis(ms) });
        let text = render(&mut app, 110, 30);
        save(&format!("boot-{name}-110x30"), &text);
        render(&mut app, 40, 10);
        render(&mut app, 3, 3);
    }
}

/// Launcher: the command runs in a new tab in the selected directory, the shell stays open.
#[test]
fn launcher_runs_command_in_directory() {
    let mut app = demo_app(110, 30);
    let probe = if cfg!(windows) { "cmd /c echo LAUNCH_PROBE_OK" } else { "echo LAUNCH_PROBE_OK" };
    app.launchers = vec![(
        noble::config::Launcher { key: "z".into(), name: "probe".into(), command: probe.into(), show: true },
        true,
    )];
    app.launch(0, Some(std::env::temp_dir()));
    assert_eq!(app.tabs.len(), 1);
    assert!(app.tab_title(0).ends_with("· probe"), "{}", app.tab_title(0));
    let id = app.tabs[0].focus;
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        app.pump();
        let text = app.panes[&id].parser().screen().contents();
        if text.lines().any(|l| l.trim() == "LAUNCH_PROBE_OK") {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "launcher output never arrived:\n{text}");
        std::thread::sleep(Duration::from_millis(100));
    }
    save("launcher-110x30", &render(&mut app, 110, 30));
    // The shell is still open after the command ends (the pane did not close).
    std::thread::sleep(Duration::from_millis(500));
    app.pump();
    assert_eq!(app.pane_count(), 1);
    app.run(Action::CloseTab);
}

fn click(app: &mut App, x: u16, y: u16) {
    use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
    for kind in [MouseEventKind::Down(MouseButton::Left), MouseEventKind::Up(MouseButton::Left)] {
        app.handle(AppEvent::Input(Event::Mouse(MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        })));
    }
}

fn find_hit(app: &App, pred: impl Fn(&noble::app::Hit) -> bool) -> Option<ratatui::layout::Rect> {
    app.hits.iter().rev().find(|(_, h)| pred(h)).map(|(r, _)| *r)
}

/// A launcher that is not installed stays hidden and does nothing on Home; an
/// installed one can be hidden from Settings and given a different shortcut.
#[test]
fn launchers_only_installed_and_configurable() {
    use noble::app::{Hit, Overlay, SettingItem, SettingKey};
    use noble::config::Launcher;
    let mut app = demo_app(110, 30);
    let present = if cfg!(windows) { "cmd" } else { "sh" };
    let mut cfg = app.cfg.clone();
    cfg.launchers = vec![
        Launcher { key: "c".into(), name: "claude".into(), command: present.into(), show: true },
        Launcher { key: "x".into(), name: "codex".into(), command: "noble-missing-tool-xyz".into(), show: true },
    ];
    app.apply_config(cfg);
    let text = render(&mut app, 110, 30);
    save("bridge-launchers-110x30", &text);
    assert!(text.contains("c Claude") && !text.contains("x Codex"), "{text}");
    assert!(find_hit(&app, |h| *h == Hit::Launcher(1)).is_none());
    // The key of a missing launcher opens nothing.
    app.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    assert_eq!(app.tabs.len(), 0);

    // Settings: a single row ("1 of 1 shown"), settings in a popup.
    let items = app.settings_items();
    let ql = items.iter().position(|i| *i == SettingItem::Setting(SettingKey::QuickLaunch)).unwrap();
    app.run(Action::Settings);
    app.settings_sel = ql;
    let text = render(&mut app, 110, 30);
    assert!(text.contains("Quick launch") && text.contains("1 of 1 shown"), "{text}");
    let row = find_hit(&app, |h| *h == Hit::Setting(ql)).expect("quick launch row");
    click(&mut app, row.x + 3, row.y);
    assert!(matches!(app.overlay, Some(Overlay::Launchers { .. })));
    let text = render(&mut app, 110, 30);
    save("settings-launchers-110x30", &text);
    // Only the installed one is listed.
    assert!(text.contains("QUICK LAUNCH") && text.contains("Claude") && !text.contains("Codex"), "{text}");
    assert!(find_hit(&app, |h| *h == Hit::LaunchShow(1)).is_none());
    // Shortcut: "x" is skipped because another launcher already has it.
    app.on_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(app.cfg.launchers[0].key, "l");
    app.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(app.cfg.launchers[0].key, "c");
    // Clicking the key chip changes it too; the window stays open.
    render(&mut app, 110, 30);
    let chip = find_hit(&app, |h| *h == Hit::LaunchKey(0)).expect("key chip");
    click(&mut app, chip.x + 1, chip.y);
    assert_eq!(app.cfg.launchers[0].key, "l");
    assert!(app.overlay.is_some());
    // Clicking the row hides it.
    render(&mut app, 110, 30);
    let row = find_hit(&app, |h| *h == Hit::LaunchShow(0)).expect("launcher row");
    click(&mut app, row.x + 3, row.y);
    assert!(!app.cfg.launchers[0].show);
    for (w, h) in SIZES {
        render(&mut app, w, h);
    }
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.overlay.is_none());
    app.run(Action::Bridge);
    let text = render(&mut app, 110, 30);
    assert!(!text.contains("l Claude") && !text.contains("c Claude"), "{text}");
    // A provider that is not installed has no setting.
    app.ai_installed = vec!["claude"];
    let items = app.settings_items();
    assert!(items.contains(&SettingItem::Setting(SettingKey::Claude)));
    assert!(!items.contains(&SettingItem::Setting(SettingKey::Codex)));
    app.run(Action::Settings);
    let text = render(&mut app, 110, 60);
    assert!(text.contains("Claude Code") && !text.contains("OpenCode Go"), "{text}");
    for (w, h) in SIZES {
        render(&mut app, w, h);
    }
}

#[test]
fn settings_screen_mouse_and_keys() {
    use noble::app::Hit;
    let mut app = demo_app(110, 30);
    // Click the ⚙ Settings tab in the top strip.
    render(&mut app, 110, 30);
    let tab = find_hit(&app, |h| *h == Hit::TabSettings).expect("settings tab");
    click(&mut app, tab.x + 1, tab.y);
    assert_eq!(app.view, View::Settings);
    for (w, h) in [(160, 45), (110, 30), (76, 24), (56, 18), (30, 8)] {
        save(&format!("settings-{w}x{h}"), &render(&mut app, w, h));
    }
    let text = render(&mut app, 110, 30);
    assert!(text.contains("Tokyo Night") && text.contains("Gruvbox"), "{text}");
    // Clicking a theme card applies the theme.
    let idx = noble::theme::THEMES.iter().position(|t| t.name == "nord").unwrap();
    let card = find_hit(&app, |h| *h == Hit::Setting(idx)).expect("nord card");
    click(&mut app, card.x + 2, card.y);
    assert_eq!(app.theme.name, "nord");
    // Keyboard: move down and flip a toggle.
    let before = app.cfg.general.clock_24h;
    let items = app.settings_items();
    let clock =
        items.iter().position(|i| *i == noble::app::SettingItem::Setting(noble::app::SettingKey::Clock24)).unwrap();
    app.settings_sel = clock;
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_ne!(before, app.cfg.general.clock_24h);
    // Turning a provider off removes it from the AI panel.
    let claude =
        items.iter().position(|i| *i == noble::app::SettingItem::Setting(noble::app::SettingKey::Claude)).unwrap();
    app.settings_sel = claude;
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!app.cfg.ai.providers.iter().any(|p| p == "claude"));
    // The Claude hooks row only exists when Claude Code is installed (or our hooks are still set).
    let hooks = noble::app::SettingItem::Setting(noble::app::SettingKey::ClaudeHooks);
    assert!(app.settings_items().contains(&hooks));
    assert!(render(&mut app, 160, 45).contains("Claude Code status hooks"));
    app.ai_installed.retain(|id| *id != "claude");
    assert!(!app.settings_items().contains(&hooks));
    let text = render(&mut app, 160, 45);
    assert!(!text.contains("status hooks"), "{text}");
    app.hooks_installed = true;
    assert!(app.settings_items().contains(&hooks));
    app.hooks_installed = false;
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.view, View::Bridge);
    // Keyboard navigation in the theme grid does not overflow.
    app.run(Action::Settings);
    app.settings_sel = 0;
    for code in [
        KeyCode::Up,
        KeyCode::Left,
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Right,
        KeyCode::End,
        KeyCode::Down,
        KeyCode::Up,
        KeyCode::Up,
    ] {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
        render(&mut app, 110, 30);
    }
}

#[test]
fn terminal_mouse_split_and_drag() {
    use noble::app::Hit;
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("mouse".into()));
    render(&mut app, 110, 30);
    let split = find_hit(&app, |h| matches!(h, Hit::PaneSplit { dir: noble::term::layout::Dir::Row, .. }))
        .expect("split button");
    click(&mut app, split.x + 1, split.y);
    assert_eq!(app.tabs[0].panes().len(), 2);
    render(&mut app, 110, 30);
    let down = find_hit(&app, |h| matches!(h, Hit::PaneSplit { dir: noble::term::layout::Dir::Col, .. }))
        .expect("split down button");
    click(&mut app, down.x + 1, down.y);
    assert_eq!(app.tabs[0].panes().len(), 3);
    let text = render(&mut app, 110, 30);
    save("terminal-mouse-split-110x30", &text);
    // Drag the vertical divider to the left.
    let div = app.hits.iter().find_map(|(r, h)| match h {
        Hit::Divider { div, .. } if div.dir == noble::term::layout::Dir::Row => Some(*r),
        _ => None,
    });
    let div = div.expect("divider");
    let before = app.tabs[0].root.layout(app.body()).0[0].1.width;
    let first = app.tabs[0].root.layout(app.body()).0[0].0;
    let size_before = app.panes[&first].size;
    use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
    for (kind, x) in [
        (MouseEventKind::Down(MouseButton::Left), div.x),
        (MouseEventKind::Drag(MouseButton::Left), div.x - 10),
        (MouseEventKind::Drag(MouseButton::Left), div.x - 20),
        (MouseEventKind::Up(MouseButton::Left), div.x - 20),
    ] {
        app.handle(AppEvent::Input(Event::Mouse(MouseEvent {
            kind,
            column: x,
            row: div.y + 3,
            modifiers: KeyModifiers::NONE,
        })));
        render(&mut app, 110, 30);
        if matches!(kind, MouseEventKind::Drag(_)) {
            // The shell keeps its size while dragging (no redraw storm), it is resized on release.
            assert_eq!(app.panes[&first].size, size_before, "PTY resized during the drag");
        }
    }
    assert!(app.panes[&first].size.1 + 15 <= size_before.1, "PTY not resized after the drag");
    let after = app.tabs[0].root.layout(app.body()).0[0].1.width;
    assert!(after + 15 <= before, "divider did not move: {before} -> {after}");
    save("terminal-mouse-drag-110x30", &render(&mut app, 110, 30));
    // The close button.
    let close = find_hit(&app, |h| matches!(h, Hit::PaneClose(_))).unwrap();
    click(&mut app, close.x + 1, close.y);
    assert_eq!(app.tabs[0].panes().len(), 2);
    app.run(Action::CloseTab);
}

/// A divider drag whose release never arrives (button let go outside the window) leaves the
/// panes drawn at a size their shells do not have yet. Selecting text past the shell's width
/// there must not crash (it did: vt100 `contents_between` underflowed).
#[test]
fn selection_beyond_the_shell_size_after_a_lost_release() {
    use crossterm::event::{MouseButton, MouseEventKind};
    use noble::app::Hit;
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("sel".into()));
    app.run(Action::SplitRight);
    render(&mut app, 110, 30);
    let div = find_hit(&app, |h| matches!(h, Hit::Divider { .. })).expect("divider");
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), div.x, div.y + 3);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), div.x - 30, div.y + 3);
    // No Up: the right pane is now drawn 30 columns wider than its shell.
    render(&mut app, 110, 30);
    let right = app.tabs[0].root.layout(app.body()).0[1].0;
    let inner = app
        .hits
        .iter()
        .find_map(|(_, h)| match h {
            Hit::Pane { pane, inner } if *pane == right => Some(*inner),
            _ => None,
        })
        .expect("right pane");
    assert!(inner.width > app.panes[&right].size.1 + 10, "{inner:?} {:?}", app.panes[&right].size);
    let (x, y) = (inner.right() - 1, inner.y + 1);
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x - 3, y);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x, y + 1);
    mouse(&mut app, MouseEventKind::Up(MouseButton::Left), x, y + 1);
    render(&mut app, 110, 30);
    app.run(Action::CloseTab);
}

fn animated_app() -> App {
    let mut app = demo_app(110, 30);
    let mut cfg = app.cfg.clone();
    cfg.general.animations = true;
    app.apply_config(cfg);
    app
}

#[test]
fn page_slide_transition() {
    let mut app = animated_app();
    render(&mut app, 110, 30);
    app.run(Action::Settings);
    // First frame: the transition starts, the new page is not fully in place yet.
    let first = render(&mut app, 110, 30);
    assert!(app.slide.is_some());
    save("slide-start-110x30", &first);
    // Move the transition to its middle.
    if let Some(s) = &mut app.slide {
        s.started = std::time::Instant::now() - Duration::from_millis(90);
    }
    let mid = render(&mut app, 110, 30);
    save("slide-mid-110x30", &mid);
    std::thread::sleep(noble::app::SLIDE_DURATION + Duration::from_millis(20));
    app.tick();
    assert!(app.slide.is_none());
    let done = render(&mut app, 110, 30);
    assert!(done.contains("Tokyo Night"));
    assert_ne!(mid, done);
    // The way back slides in the opposite direction.
    app.run(Action::Bridge);
    render(&mut app, 110, 30);
    assert_eq!(app.slide.as_ref().map(|s| s.dir), Some(-1));
    // On a size change the transition is cancelled safely.
    render(&mut app, 80, 20);
    render(&mut app, 30, 8);
}

#[test]
fn zoom_animation_grows_from_tile() {
    let mut app = animated_app();
    app.new_tab(std::env::temp_dir(), None, Some("zoom".into()));
    app.run(Action::SplitRight);
    render(&mut app, 110, 30);
    let pane = app.tabs[0].focus;
    app.run(Action::Zoom);
    let z = app.zoom_anim.as_ref().expect("zoom animation");
    assert_eq!(z.pane, pane);
    assert_eq!(z.to, app.body());
    assert!(z.from.width < z.to.width, "should grow from its tile");
    if let Some(z) = &mut app.zoom_anim {
        z.started = std::time::Instant::now() - Duration::from_millis(60);
    }
    save("zoom-in-mid-110x30", &render(&mut app, 110, 30));
    std::thread::sleep(noble::app::ZOOM_DURATION + Duration::from_millis(20));
    app.tick();
    assert!(app.zoom_anim.is_none());
    // Coming back: it shrinks from fullscreen back to its place.
    app.run(Action::Zoom);
    let z = app.zoom_anim.as_ref().unwrap();
    assert_eq!(z.from, app.body());
    assert!(z.to.width < z.from.width);
    save("zoom-out-start-110x30", &render(&mut app, 110, 30));
    app.run(Action::CloseTab);
}

#[test]
fn hover_highlights_clickable_items() {
    use crossterm::event::{Event, MouseEvent, MouseEventKind};
    use noble::app::Hit;
    let mut app = demo_app(110, 30);
    render(&mut app, 110, 30);
    let row = find_hit(&app, |h| *h == Hit::Project(2)).unwrap();
    let base = {
        let mut term = Terminal::new(TestBackend::new(110, 30)).unwrap();
        term.draw(|f| noble::ui::draw(f, &mut app)).unwrap();
        term.backend().buffer()[(row.x + 5, row.y)].bg
    };
    app.handle(AppEvent::Input(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column: row.x + 5,
        row: row.y,
        modifiers: KeyModifiers::NONE,
    })));
    let mut term = Terminal::new(TestBackend::new(110, 30)).unwrap();
    term.draw(|f| noble::ui::draw(f, &mut app)).unwrap();
    let hovered = term.backend().buffer()[(row.x + 5, row.y)].bg;
    assert_ne!(base, hovered, "row should highlight under the mouse");
    assert_eq!(hovered, app.theme.hover());
}

fn wait_for(app: &mut App, pane: noble::term::layout::PaneId, needle: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        app.pump();
        let (lines, _) = app.panes[&pane].all_lines();
        if lines.iter().any(|l| l.trim() == needle) {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "output '{needle}' never arrived:\n{}", lines.join("\n"));
        std::thread::sleep(Duration::from_millis(100));
    }
}

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
    let mut cfg = app.cfg.clone();
    if cfg!(windows) {
        cfg.terminal.shell = "cmd.exe".into();
    }
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

/// When a background tab sends a notification a ◆ appears on it and clears when opened.
#[test]
fn background_tab_notification_marks_tab() {
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("worker".into()));
    app.new_tab(std::env::temp_dir(), None, Some("front".into()));
    assert_eq!(app.view, View::Term(1));
    std::thread::sleep(Duration::from_millis(800));
    app.pump();
    let worker = app.tabs[0].focus;
    {
        let pane = &app.panes[&worker];
        pane.parser().process(b"\x1b]9;Build finished\x07");
        pane.dirty.store(true, std::sync::atomic::Ordering::Release);
    }
    app.handle(AppEvent::PtyOutput);
    assert!(app.tabs[0].alert);
    assert!(app.outer_bell);
    assert!(app.toasts.iter().any(|t| t.text.contains("Build finished")), "toast missing");
    let text = render(&mut app, 110, 30);
    save("terminal-alert-110x30", &text);
    assert!(text.lines().next().unwrap().contains('◆'), "{text}");
    app.run(Action::GoTab(1));
    app.tick();
    assert!(!app.tabs[0].alert);
    app.run(Action::CloseTab);
    app.run(Action::CloseTab);
}

/// Home: an open AI session shows on the project, the usage history is drawn as a graph.
#[test]
fn bridge_shows_agent_sessions_and_usage_graph() {
    let mut app = demo_app(160, 45);
    let now = chrono::Utc::now().timestamp();
    app.usage_history = noble::store::UsageHistory::memory();
    for i in 0..30 {
        let key = noble::store::UsageHistory::key("claude", "5H");
        app.usage_history.record(&key, now - 23 * 3600 + i * 2700, (i * 3 % 100) as u8);
    }
    let dir = std::env::temp_dir();
    let mut projects = app.projects.clone();
    projects.push(Project {
        name: "agentdemo".into(),
        path: dir.clone(),
        branch: Some("main".into()),
        last_active: Some(SystemTime::now()),
        git: None,
    });
    app.handle(AppEvent::Projects(projects));
    let probe = if cfg!(windows) { "cmd /c echo claude-probe" } else { "echo claude-probe" };
    app.launchers = vec![(
        noble::config::Launcher { key: "z".into(), name: "claude".into(), command: probe.into(), show: true },
        true,
    )];
    app.launch(0, Some(dir.clone()));
    assert_eq!(app.agent_sessions(&dir), vec![("claude", noble::app::AgentState::Running)]);
    app.run(Action::Bridge);
    app.on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    for c in "agentdemo".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    let text = render(&mut app, 160, 45);
    save("bridge-agents-160x45", &text);
    assert!(text.contains("claude running"), "{text}");
    assert!(text.contains("agentdemo ●"), "{text}");
    assert!(!text.contains("24h"), "{text}");
    for (w, h) in SIZES {
        render(&mut app, w, h);
    }
    app.tabs.clear();
    app.panes.clear();
}

/// Warns once when a quota threshold is crossed; never again for the same window.
#[test]
fn quota_warning_fires_once() {
    let mut app = demo_app(110, 30);
    app.toasts.clear();
    let state = |used: u8| ProviderState {
        id: "claude",
        name: "Claude Code",
        login_hint: "claude",
        presence: Presence::Ready,
        status: Status::Ok,
        usage: Some(Usage {
            windows: vec![Window { label: "5H".into(), used, resets_at: Some(4_000_000_000) }],
            plan: None,
            note: None,
        }),
        fetched_at: Some(chrono::Utc::now().timestamp()),
    };
    app.handle(AppEvent::Ai(Box::new(state(50))));
    assert!(app.toasts.is_empty());
    app.handle(AppEvent::Ai(Box::new(state(93))));
    assert_eq!(app.toasts.len(), 1);
    assert!(app.toasts[0].text.contains("Claude Code 5h usage at 93%"), "{}", app.toasts[0].text);
    app.toasts.clear();
    app.handle(AppEvent::Ai(Box::new(state(95))));
    assert!(app.toasts.is_empty());
}

/// Terminal color scheme: pane background and ANSI colors are independent of the UI theme.
#[test]
fn terminal_color_scheme_applies() {
    use ratatui::style::Color;
    let mut app = demo_app(110, 30);
    let mut cfg = app.cfg.clone();
    cfg.terminal.colors = "light-gray".into();
    app.apply_config(cfg);
    app.new_tab(std::env::temp_dir(), None, Some("gray".into()));
    let id = app.tabs[0].focus;
    // Red (ANSI 1) text is drawn with the scheme's red.
    app.panes[&id].parser().process(b"\x1b[2J\x1b[H\x1b[31mRED\x1b[0m plain");
    let mut term = Terminal::new(TestBackend::new(110, 30)).unwrap();
    term.draw(|f| noble::ui::draw(f, &mut app)).unwrap();
    let inner = find_hit_inner(&app).expect("pane");
    let buf = term.backend().buffer().clone();
    assert_eq!(buf[(inner.x, inner.y)].symbol(), "R");
    assert_eq!(buf[(inner.x, inner.y)].fg, Color::Rgb(0xa3, 0x26, 0x1b));
    assert_eq!(buf[(inner.x + 5, inner.y)].fg, Color::Rgb(0x1e, 0x1e, 0x1e));
    let empty = (inner.x + 20, inner.y + 10);
    assert_eq!(buf[empty].bg, Color::Rgb(0xc8, 0xc8, 0xc8));
    save("terminal-light-gray-110x30", &render(&mut app, 110, 30));
    // A custom background color overrides the scheme.
    let mut cfg = app.cfg.clone();
    cfg.terminal.background = "#d0d0d0".into();
    app.apply_config(cfg);
    term.draw(|f| noble::ui::draw(f, &mut app)).unwrap();
    assert_eq!(term.backend().buffer()[empty].bg, Color::Rgb(0xd0, 0xd0, 0xd0));
    app.run(Action::CloseTab);
}

fn find_hit_inner(app: &App) -> Option<ratatui::layout::Rect> {
    app.hits.iter().find_map(|(_, h)| match h {
        noble::app::Hit::Pane { inner, .. } => Some(*inner),
        _ => None,
    })
}

/// Scheme selector: the Windows Terminal PowerShell scheme comes first and all
/// built-ins are in the list; navigating previews, esc reverts, clicking picks.
#[test]
fn scheme_picker_previews_and_selects() {
    use noble::app::{Hit, Overlay, SettingItem, SettingKey};
    let mut app = demo_app(110, 30);
    let wt = noble::wt::parse(
        r#"{ "profiles": { "defaults": { "colorScheme": "Dark+" },
             "list": [ { "name": "Windows PowerShell", "commandline": "powershell.exe" } ] } }"#,
    )
    .unwrap();
    app.term_schemes = noble::wt::all_schemes(Some(&wt));
    assert_eq!(app.cfg.terminal.colors, "windows-terminal");
    app.run(Action::Settings);
    app.settings_sel =
        app.settings_items().iter().position(|i| *i == SettingItem::Setting(SettingKey::TermColors)).unwrap();
    let text = render(&mut app, 110, 30);
    save("settings-term-colors-110x30", &text);
    assert!(text.contains("Windows PowerShell · Dark+"), "{text}");
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(app.overlay, Some(Overlay::Schemes(_))));
    let text = render(&mut app, 110, 30);
    save("scheme-picker-110x30", &text);
    for name in ["Follow theme", "Windows PowerShell · Dark+", "Campbell Powershell", "One Half Light", "Tango Dark"] {
        assert!(
            text.contains(name),
            "{name} missing
{text}"
        );
    }
    // Down: the preview is applied, esc reverts it.
    app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.cfg.terminal.colors, "campbell");
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.cfg.terminal.colors, "windows-terminal");
    // Clicking a row picks it.
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    render(&mut app, 110, 30);
    let idx = app.scheme_options().iter().position(|n| n == "light-gray").unwrap();
    let row = find_hit(&app, |h| *h == Hit::TermScheme(idx)).expect("light gray row");
    click(&mut app, row.x + 3, row.y);
    assert_eq!(app.cfg.terminal.colors, "light-gray");
    assert!(app.overlay.is_none());
    for (w, h) in [(160, 45), (76, 24), (56, 18), (30, 8)] {
        app.open_scheme_picker();
        save(&format!("scheme-picker-{w}x{h}"), &render(&mut app, w, h));
        app.overlay = None;
    }
}

/// Battery: shown on Home and System, warns once each at 20% and 10% while on
/// battery, never appears on a machine without a battery.
#[test]
fn battery_is_shown_and_warns_when_low() {
    use noble::battery::{Battery, PowerState};
    let mut app = demo_app(160, 45);
    let home = render(&mut app, 160, 45);
    save("bridge-battery-160x45", &home);
    assert!(home.contains("BATTERY") && home.contains("76%") && home.contains("3h 12m left"), "{home}");
    // The battery sits at the very bottom of the system panel (right above its frame).
    let lines: Vec<&str> = home.lines().collect();
    let status_row = lines.iter().position(|l| l.contains("3h 12m left")).unwrap();
    assert!(lines[status_row + 1].contains('╰'), "{home}");
    app.run(Action::System);
    let sys = render(&mut app, 110, 30);
    save("system-battery-110x30", &sys);
    assert!(sys.contains("76%") && sys.contains("3h 12m left"), "{sys}");
    let sys_big = render(&mut app, 160, 45);
    save("system-battery-160x45", &sys_big);
    assert!(sys_big.contains("BATTERY"), "{sys_big}");
    // A ↯ mark while charging.
    let mut charging = app.sensors.last.clone().unwrap();
    charging.battery =
        Some(Battery { percent: 64.0, state: PowerState::Charging, secs_left: None, secs_to_full: Some(2700) });
    app.handle(AppEvent::Sensors(Box::new(charging)));
    app.run(Action::Bridge);
    let text = render(&mut app, 160, 45);
    save("bridge-battery-charging-160x45", &text);
    assert!(text.contains("64% ↯") && text.contains("full in 45m"), "{text}");
    app.run(Action::System);

    let base = app.sensors.last.clone().unwrap();
    let sample = |percent: f32, state: PowerState| {
        let mut s = base.clone();
        s.battery = Some(Battery { percent, state, secs_left: None, secs_to_full: None });
        AppEvent::Sensors(Box::new(s))
    };
    app.toasts.clear();
    let ev = sample(19.0, PowerState::Discharging);
    app.handle(ev);
    let ev = sample(18.0, PowerState::Discharging);
    app.handle(ev);
    assert_eq!(app.toasts.len(), 1, "one warning at 20%");
    assert!(app.toasts[0].text.starts_with("battery at 19%"), "{}", app.toasts[0].text);
    let ev = sample(9.0, PowerState::Discharging);
    app.handle(ev);
    assert_eq!(app.toasts.len(), 2);
    // Plugging in and draining again re-arms the warning.
    let ev = sample(9.0, PowerState::Charging);
    app.handle(ev);
    app.toasts.clear();
    let ev = sample(9.0, PowerState::Discharging);
    app.handle(ev);
    assert_eq!(app.toasts.len(), 1);

    let mut desk = demo_app(160, 45);
    let mut s = desk.sensors.last.clone().unwrap();
    s.battery = None;
    desk.handle(AppEvent::Sensors(Box::new(s)));
    let text = render(&mut desk, 160, 45);
    assert!(!text.contains("BATTERY") && !text.contains("left"), "{text}");
}

/// First launch card: drawn at every size, the prefix can be picked and applied.
#[test]
fn welcome_card_picks_prefix() {
    use noble::app::{Hit, Overlay};
    let mut app = demo_app(110, 30);
    app.show_welcome();
    let text = render(&mut app, 110, 30);
    save("welcome-110x30", &text);
    assert!(text.contains("WELCOME TO NOBLE") && text.contains("found 6 git projects"), "{text}");
    assert!(text.contains("ctrl+g"), "{text}");
    for (w, h) in SIZES {
        save(&format!("welcome-{w}x{h}"), &render(&mut app, w, h));
    }
    render(&mut app, 110, 30);
    // Clicking the background does not close the card; clicking the ctrl+g chip picks it.
    click(&mut app, 1, 29);
    assert!(matches!(app.overlay, Some(Overlay::Welcome { .. })));
    let chip = find_hit(&app, |h| *h == Hit::WelcomePrefix(3)).expect("ctrl+g chip");
    click(&mut app, chip.x + 1, chip.y);
    assert!(matches!(app.overlay, Some(Overlay::Welcome { prefix: 3 })));
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.overlay.is_none());
    assert_eq!(app.cfg.keys.prefix, "ctrl+g");
    assert!(app.ui_state.data.welcomed);
    // The new prefix actually works.
    app.on_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL));
    assert!(app.prefix_armed);
}

/// Adding a project folder: a prompt opens, an invalid path is rejected and a
/// valid one joins the list along with the default roots.
#[test]
fn add_project_folder_flow() {
    use noble::app::Overlay;
    let mut app = demo_app(110, 30);
    app.on_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    assert!(matches!(app.overlay, Some(Overlay::Prompt(_))));
    save("add-folder-110x30", &render(&mut app, 110, 30));
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.add_project_root("Z:/definitely/not/here");
    assert!(app.cfg.projects.roots.is_empty());
    let dir = std::env::temp_dir();
    app.add_project_root(&dir.display().to_string());
    assert!(app.cfg.projects.roots.iter().any(|r| std::path::Path::new(r) == dir.as_path()));
    let defaults = noble::projects::default_roots().len();
    assert_eq!(app.cfg.projects.roots.len(), defaults + 1);
    // The same folder is not added twice.
    app.add_project_root(&dir.display().to_string());
    assert_eq!(app.cfg.projects.roots.len(), defaults + 1);
}

/// The hint shows on the wide status bar and takes no space on the narrow one.
#[test]
fn status_bar_shows_tips_when_wide() {
    let mut app = demo_app(200, 40);
    let text = render(&mut app, 200, 40);
    assert!(text.lines().last().unwrap().contains("tip: "), "{text}");
    let narrow = render(&mut app, 110, 30);
    assert!(!narrow.lines().last().unwrap().contains("tip: "));
}

fn mouse(app: &mut App, kind: crossterm::event::MouseEventKind, x: u16, y: u16) {
    use crossterm::event::{Event, MouseEvent};
    app.handle(AppEvent::Input(Event::Mouse(MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE })));
}

/// Right-click menus: tab and pane title; the command picked from the menu runs.
#[test]
fn passthrough_sends_shortcuts_to_the_app() {
    use noble::app::{SettingItem, SettingKey};
    let key = |c: char, m: KeyModifiers| KeyEvent::new(KeyCode::Char(c), m);
    let prefix = || KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("keys".into()));
    let term = app.view;
    // Shell keys (alt+p, alt+. …) reach the shell in a terminal; the others are NOBLE's.
    app.on_key(key('p', KeyModifiers::ALT));
    assert!(app.overlay.is_none());
    let text = render(&mut app, 110, 30);
    assert!(text.contains("ctrl+a : commands") && !text.contains("alt+p commands"), "{text}");
    app.on_key(key('m', KeyModifiers::ALT));
    assert_eq!(app.view, View::System);
    app.on_key(key('p', KeyModifiers::ALT));
    assert!(app.overlay.is_some(), "outside a terminal alt+p stays the palette");
    app.overlay = None;
    app.view = term;

    // "lock" (default): prefix i locks the pane, alt+m / alt+z go to the app.
    app.on_key(prefix());
    app.on_key(key('i', KeyModifiers::NONE));
    assert!(app.focused_locked());
    let text = render(&mut app, 110, 30);
    save("term-keys-locked-110x30", &text);
    assert!(text.contains("🔒 bash") || text.contains("🔒 "), "{text}");
    assert!(text.contains("KEYS") && text.contains("unlock keys"), "{text}");
    app.on_key(key('m', KeyModifiers::ALT));
    app.on_key(key('z', KeyModifiers::ALT));
    assert!(app.overlay.is_none() && !app.tabs[0].zoomed);
    assert_eq!(app.view, term);
    // The lock also shows on a pane too narrow for the corner tag.
    let narrow = render(&mut app, 16, 10);
    save("term-keys-locked-16x10", &narrow);
    assert!(narrow.contains('🔒'), "{narrow}");
    // The prefix still works while locked; prefix i unlocks.
    app.on_key(prefix());
    app.on_key(key('i', KeyModifiers::NONE));
    assert!(!app.focused_locked());
    app.on_key(key('m', KeyModifiers::ALT));
    assert_eq!(app.view, View::System);

    // "once": prefix + a shortcut sends only that key; prefix i passes the next key.
    assert_eq!(noble::config::parse(noble::config::DEFAULT_CONFIG).unwrap().keys.passthrough, "lock");
    assert_eq!(app.setting_value(SettingKey::Passthrough), "lock (ctrl+a i)");
    app.activate_setting(SettingItem::Setting(SettingKey::Passthrough), 1);
    assert_eq!(app.setting_value(SettingKey::Passthrough), "once (ctrl+a + key)");
    app.view = term;
    app.toasts.clear();
    app.on_key(prefix());
    app.on_key(key('m', KeyModifiers::ALT));
    assert!(app.view == term && app.toasts.is_empty(), "sent to the app, no 'not bound' warning");
    app.on_key(prefix());
    app.on_key(key('i', KeyModifiers::NONE));
    assert!(!app.focused_locked() && app.pass_next.is_some());
    app.on_key(key('m', KeyModifiers::ALT));
    assert_eq!(app.view, term);
    app.on_key(key('m', KeyModifiers::ALT));
    assert_eq!(app.view, View::System);
    // A pending "next key" is dropped when the terminal is left.
    app.view = term;
    app.on_key(prefix());
    app.on_key(key('i', KeyModifiers::NONE));
    app.view = View::Bridge;
    app.on_key(key('m', KeyModifiers::ALT));
    assert_eq!(app.view, View::System);
    app.view = term;
    app.on_key(key('m', KeyModifiers::ALT));
    assert_eq!(app.view, View::System, "not carried back into the terminal");

    // shell_first off: NOBLE takes alt+p in terminals too.
    app.activate_setting(SettingItem::Setting(SettingKey::ShellFirst), 1);
    app.view = term;
    app.on_key(key('p', KeyModifiers::ALT));
    assert!(app.overlay.is_some());
}

/// Closing a pane or tab with something still running, and a paste with line breaks, ask first.
#[test]
fn risky_terminal_actions_ask_first() {
    use noble::app::Overlay;
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("busy".into()));
    let id = app.focused_pane().unwrap();
    // An idle shell closes right away.
    app.run(Action::SplitRight);
    let second = app.focused_pane().unwrap();
    app.run(Action::ClosePane);
    assert!(app.overlay.is_none() && !app.panes.contains_key(&second));

    // A command running at the prompt (shell integration seen): ask, then close on "yes".
    {
        let p = app.panes.get_mut(&id).unwrap();
        p.prompted = true;
        p.command_started = Some(std::time::Instant::now());
    }
    app.run(Action::ClosePane);
    assert!(matches!(app.overlay, Some(Overlay::Confirm(_))));
    let text = render(&mut app, 110, 30);
    save("confirm-close-pane-110x30", &text);
    assert!(text.contains("CLOSE PANE") && text.contains("still running"), "{text}");
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.overlay.is_none() && app.panes.contains_key(&id), "esc keeps the pane");
    app.run(Action::CloseTab);
    assert!(render(&mut app, 110, 30).contains("CLOSE TAB"));

    // Paste: a line break asks first when the app has no bracketed paste, one line does not.
    // The mode is set on the emulator directly so the test does not depend on the shell's version.
    app.overlay = None;
    app.panes[&id].parser().process(b"\x1b[?2004h");
    app.paste_into(id, "echo one\necho two".into());
    assert!(app.overlay.is_none(), "bracketed paste: the app itself decides");
    app.panes[&id].parser().process(b"\x1b[?2004l");
    app.paste_into(id, "echo one".into());
    assert!(app.overlay.is_none());
    app.paste_into(id, "echo one\necho two\n".into());
    let text = render(&mut app, 110, 30);
    save("confirm-paste-110x30", &text);
    assert!(text.contains("PASTE") && text.contains("2 lines"), "{text}");
    app.on_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
    assert!(app.overlay.is_none());

    app.run(Action::CloseTab);
    app.on_key(enter);
    assert!(app.tabs.is_empty());
}

#[test]
fn context_menus_on_tabs_and_panes() {
    use crossterm::event::{MouseButton, MouseEventKind};
    use noble::app::{Hit, Overlay};
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("menu".into()));
    render(&mut app, 110, 30);
    let title = find_hit(&app, |h| matches!(h, Hit::PaneTitle(_))).expect("pane title");
    mouse(&mut app, MouseEventKind::Down(MouseButton::Right), title.x + 3, title.y);
    assert!(matches!(app.overlay, Some(Overlay::Menu(_))));
    let text = render(&mut app, 110, 30);
    save("menu-pane-110x30", &text);
    assert!(text.contains("Search scrollback") && text.contains("Open in VS Code"), "{text}");
    let split = app
        .hits
        .iter()
        .find_map(|(r, h)| match h {
            Hit::MenuItem(i) => match &app.overlay {
                Some(Overlay::Menu(m)) if m.items[*i].label == "Split right" => Some(*r),
                _ => None,
            },
            _ => None,
        })
        .expect("split item");
    click(&mut app, split.x + 2, split.y);
    assert!(app.overlay.is_none());
    assert_eq!(app.tabs[0].panes().len(), 2);
    // Tab menu: pick "Close tab" with the keyboard.
    render(&mut app, 110, 30);
    let tab = find_hit(&app, |h| *h == Hit::Tab(0)).unwrap();
    mouse(&mut app, MouseEventKind::Down(MouseButton::Right), tab.x + 1, tab.y);
    save("menu-tab-110x30", &render(&mut app, 110, 30));
    let n = match &app.overlay {
        Some(Overlay::Menu(m)) => m.items.len(),
        _ => panic!("tab menu not open"),
    };
    for _ in 0..n - 1 {
        app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.tabs.is_empty());
    // The menu does not overflow even on a small screen.
    app.new_tab(std::env::temp_dir(), None, Some("tiny".into()));
    let id = app.tabs[0].focus;
    app.open_pane_menu(id, 28, 7);
    render(&mut app, 30, 8);
    app.overlay = None;
    app.run(Action::CloseTab);
}

/// What was mouse-only has an action (and a prefix key): moving tabs, the pane menu,
/// the update notice.
#[test]
fn mouse_only_things_have_actions() {
    use noble::app::Overlay;
    use noble::keys::Chord;
    let mut app = demo_app(110, 30);
    for name in ["one", "two", "three"] {
        app.new_tab(std::env::temp_dir(), None, Some(name.into()));
    }
    let names = |app: &App| (0..app.tabs.len()).map(|i| app.tab_title(i)).collect::<Vec<_>>();
    // The last tab is open; move it to the front with the prefix keys.
    let prefix = app.keymap.prefix;
    let press = |app: &mut App, c: char| {
        app.on_key(KeyEvent::new(prefix.code, prefix.mods));
        app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    };
    assert_eq!(app.view, View::Term(2));
    press(&mut app, '<');
    press(&mut app, '<');
    press(&mut app, '<'); // already first: nothing happens
    assert_eq!(names(&app), ["three", "one", "two"]);
    assert_eq!(app.view, View::Term(0), "the view follows the moved tab");
    press(&mut app, '>');
    assert_eq!(names(&app), ["one", "three", "two"]);
    // Pane menu under the pane title, as with a right-click.
    render(&mut app, 110, 30);
    press(&mut app, '.');
    let text = render(&mut app, 110, 30);
    save("menu-pane-key-110x30", &text);
    assert!(matches!(app.overlay, Some(Overlay::Menu(_))) && text.contains("Copy path"), "{text}");
    app.overlay = None;
    assert_eq!(app.keymap.prefix_map.get(&Chord::parse(".").unwrap()), Some(&Action::PaneMenu));
    // Update actions are in the palette only while a newer version is known.
    let titles = |app: &App| app.palette_items().into_iter().map(|i| i.title).collect::<Vec<_>>();
    assert!(!titles(&app).iter().any(|t| t == "Update NOBLE"));
    app.handle(AppEvent::Update(Ok("99.0.0".into())));
    assert!(titles(&app).iter().any(|t| t == "Update NOBLE"));
    app.run(Action::DismissUpdate);
    assert!(app.update_notice().is_none());
    assert!(!titles(&app).iter().any(|t| t == "Dismiss Update Notice"));
    for _ in 0..3 {
        app.run(Action::CloseTab);
    }
}

/// Reordering tabs by dragging and renaming them with a double click.
#[test]
fn tabs_drag_and_rename() {
    use crossterm::event::{MouseButton, MouseEventKind};
    use noble::app::{Hit, Overlay};
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("first".into()));
    app.new_tab(std::env::temp_dir(), None, Some("second".into()));
    render(&mut app, 110, 30);
    let t0 = find_hit(&app, |h| *h == Hit::Tab(0)).unwrap();
    let t1 = find_hit(&app, |h| *h == Hit::Tab(1)).unwrap();
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), t0.x + 1, t0.y);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), t1.x + 2, t1.y);
    mouse(&mut app, MouseEventKind::Up(MouseButton::Left), t1.x + 2, t1.y);
    assert_eq!(app.tabs[0].origin, "second");
    assert_eq!(app.tabs[1].origin, "first");
    assert_eq!(app.view, View::Term(1), "the dragged tab stays active");
    render(&mut app, 110, 30);
    let active = find_hit(&app, |h| *h == Hit::Tab(1)).unwrap();
    click(&mut app, active.x + 1, active.y);
    click(&mut app, active.x + 1, active.y);
    assert!(matches!(app.overlay, Some(Overlay::Prompt(_))), "double-click renames");
    app.overlay = None;
    app.run(Action::CloseTab);
    app.run(Action::CloseTab);
    // Tabs of different widths: the short one only moves once the pointer is far enough into the long
    // one to stay over it afterwards, and it does not swap back and forth (jitter).
    app.new_tab(std::env::temp_dir(), None, Some("a".into()));
    app.new_tab(std::env::temp_dir(), None, Some("a-much-longer-tab".into()));
    render(&mut app, 110, 30);
    let short = find_hit(&app, |h| *h == Hit::Tab(0)).unwrap();
    let long = find_hit(&app, |h| *h == Hit::Tab(1)).unwrap();
    let drag = |app: &mut App, x: u16| mouse(app, MouseEventKind::Drag(MouseButton::Left), x, short.y);
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), short.x + 1, short.y);
    drag(&mut app, long.x + 1);
    assert_eq!(app.tabs[0].origin, "a", "too early: it would land back on the long tab");
    drag(&mut app, long.right() - 1);
    assert_eq!(app.tabs[1].origin, "a", "moved past the long tab");
    // More drag events before the next frame (stale rects) change nothing.
    drag(&mut app, long.x + 1);
    drag(&mut app, long.right() - 1);
    assert_eq!(app.tabs[1].origin, "a");
    render(&mut app, 110, 30);
    drag(&mut app, short.x + 1);
    assert_eq!(app.tabs[0].origin, "a", "and back to the front");
    mouse(&mut app, MouseEventKind::Up(MouseButton::Left), short.x + 1, short.y);
    app.run(Action::CloseTab);
    app.run(Action::CloseTab);
}

/// Project row: quick actions on hover; a pinned project moves to the top.
#[test]
fn project_row_actions_and_pins() {
    use crossterm::event::{MouseButton, MouseEventKind};
    use noble::app::{Hit, Overlay, ProjectAct};
    let mut app = demo_app(110, 30);
    render(&mut app, 110, 30);
    let row = find_hit(&app, |h| *h == Hit::Project(3)).unwrap();
    mouse(&mut app, MouseEventKind::Moved, row.x + 5, row.y);
    let text = render(&mut app, 110, 30);
    save("project-hover-110x30", &text);
    assert!(text.contains(" ☆  ⋯ ") && !text.contains(" code ") && !text.contains(" pull "), "{text}");
    let pin = find_hit(&app, |h| *h == Hit::ProjectAct(3, ProjectAct::Pin)).expect("pin button");
    let name = app.projects[app.visible_projects()[3]].name.clone();
    click(&mut app, pin.x + 1, pin.y);
    assert_eq!(app.projects[0].name, name, "pinned project moves to the top");
    assert_eq!(app.selected_project().unwrap().name, name, "selection follows");
    let text = render(&mut app, 110, 30);
    assert!(text.contains(&format!("★ {name}")), "{text}");
    // Right-click project menu.
    let first = find_hit(&app, |h| *h == Hit::Project(0)).unwrap();
    mouse(&mut app, MouseEventKind::Down(MouseButton::Right), first.x + 4, first.y);
    let text = render(&mut app, 110, 30);
    save("menu-project-110x30", &text);
    assert!(matches!(app.overlay, Some(Overlay::Menu(_))) && text.contains("Unpin"), "{text}");
}

/// Project row from the keyboard: the selected row shows ★ ⋯ without the mouse, → focuses
/// them, ⏎ pins the project or opens its menu, any other key leaves them.
#[test]
fn project_actions_from_keyboard() {
    use noble::app::{Overlay, ProjectAct};
    let mut app = demo_app(110, 30);
    let key = |app: &mut App, c: KeyCode| app.on_key(KeyEvent::new(c, KeyModifiers::NONE));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Down);
    let text = render(&mut app, 110, 30);
    assert!(text.contains(" ☆  ⋯ "), "buttons visible on the selected row without hover: {text}");
    let name = app.selected_project().unwrap().name.clone();
    key(&mut app, KeyCode::Right);
    assert_eq!(app.bridge.proj_act, Some(ProjectAct::Pin));
    save("project-keys-110x30", &render(&mut app, 110, 30));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.projects[0].name, name, "pinned project moves to the top");
    assert_eq!(app.selected_project().unwrap().name, name, "selection follows");
    assert_eq!(app.bridge.proj_act, Some(ProjectAct::Pin), "focus stays on the star");
    assert!(app.tabs.is_empty(), "⏎ on the star does not open a terminal");
    // ⏎ again unpins.
    key(&mut app, KeyCode::Enter);
    assert!(!app.ui_state.is_pinned(&app.selected_project().unwrap().path));
    // → ⋯ ⏎ opens the project menu; ← goes back; esc leaves the buttons.
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Left);
    assert_eq!(app.bridge.proj_act, Some(ProjectAct::Pin));
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.bridge.proj_act, None);
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Right);
    render(&mut app, 110, 30);
    key(&mut app, KeyCode::Enter);
    let text = render(&mut app, 110, 30);
    assert!(matches!(app.overlay, Some(Overlay::Menu(_))) && text.contains("Pin to top"), "{text}");
    // Moving the selection leaves the buttons.
    app.overlay = None;
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Down);
    assert_eq!(app.bridge.proj_act, None);
}

/// Claude Code hook: when a background session asks for permission the tab is
/// marked, the session list on Home shows "needs you" and clicking the row opens the tab.
#[test]
fn claude_hook_states_drive_sessions() {
    use noble::app::{AgentState, Hit};
    use noble::hooks::HookRecord;
    let mut app = demo_app(160, 45);
    app.new_tab(std::env::temp_dir(), None, Some("agent".into()));
    let pane = app.tabs[0].focus;
    app.run(Action::Bridge);
    app.toasts.clear();
    let rec = |event: &str, msg: Option<&str>| HookRecord {
        event: event.into(),
        message: msg.map(str::to_string),
        ts: chrono::Utc::now().timestamp(),
    };
    app.apply_hook_records([(pane, rec("prompt", None))].into_iter().collect());
    assert_eq!(app.agent_state(pane), Some(("claude", AgentState::Working)));
    assert!(app.toasts.is_empty(), "working is not an alert");
    app.apply_hook_records([(pane, rec("notification", Some("Claude needs your permission to use Bash")))].into());
    assert_eq!(app.agent_state(pane), Some(("claude", AgentState::NeedsYou)));
    assert!(app.tabs[0].alert);
    assert!(app.toasts.iter().any(|t| t.text.contains("needs your permission")), "toast missing");
    let text = render(&mut app, 160, 45);
    save("bridge-sessions-160x45", &text);
    assert!(text.contains("◆ claude") && text.contains("needs you"), "{text}");
    let row = find_hit(&app, |h| *h == Hit::Tab(0)).expect("session row");
    // The session row (right column) must differ from the tab in the top strip.
    let session_row =
        app.hits.iter().filter(|(r, h)| *h == Hit::Tab(0) && r.y > 1).map(|(r, _)| *r).next().unwrap_or(row);
    click(&mut app, session_row.x + 2, session_row.y);
    assert_eq!(app.view, View::Term(0));
    // Reading the same event again does not re-notify; when done it says "your turn".
    app.toasts.clear();
    let same = app.agent_hooks.clone();
    app.apply_hook_records(same);
    assert!(app.toasts.is_empty());
    app.apply_hook_records([(pane, rec("stop", None))].into());
    assert_eq!(app.agent_state(pane), Some(("claude", AgentState::Idle)));
    app.run(Action::CloseTab);
    assert!(app.agent_hooks.is_empty());
}

/// Runs the real `noble hook <event>` the way Claude Code's hooks do: inside the pane's
/// environment (`NOBLE_INSTANCE`, `NOBLE_PANE`), with the event JSON on stdin.
fn noble_hook(app: &App, pane: noble::term::layout::PaneId, instance: u32, event: &str, stdin: &str) {
    use std::io::Write as _;
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_noble"))
        .args(["hook", event])
        .env("NOBLE_HOME", &app.paths.data)
        .env("NOBLE_INSTANCE", instance.to_string())
        .env("NOBLE_PANE", pane.to_string())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("noble hook");
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    assert!(child.wait().unwrap().success());
}

/// What `App::tick` does once a second while hooks are live.
fn scan_hooks(app: &mut App) {
    let records = noble::hooks::read_records(&app.paths.data, std::process::id());
    app.apply_hook_records(records);
}

fn pump_until(app: &mut App, what: &str, done: impl Fn(&App) -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        app.pump();
        if done(app) {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "{what} never happened\n{}", render(app, 110, 30));
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A stand-in for an agent's TUI in `shell`: sets the window title the way the agent
/// does, then waits for a line (Enter "exits" it).
fn fake_agent(shell: &noble::term::pane::ShellSpec, title: &str) -> String {
    match shell.label().to_lowercase().as_str() {
        "pwsh" | "powershell" => {
            format!("Write-Host -NoNewline ([char]27 + ']0;{title}' + [char]7); $null = Read-Host")
        }
        // cmd.exe has no escape sequences: `title` sets the console title, which ConPTY reports as OSC 0.
        "cmd" => format!("title {title} & pause >nul"),
        // bash, zsh and fish all understand `\e` in printf. The wait is an external program:
        // fish's own `read` re-titles the terminal.
        _ => format!("printf '\\e]0;{title}\\a'; head -n 1 >/dev/null"),
    }
}

/// Agent lifecycle in one pane, on every installed shell: Claude opened from a quick
/// launch goes through its hook events and exits; the shell prompt (OSC 7 / 9;9) ends it;
/// another agent started in the same pane is shown instead; back at the shell nothing
/// is shown; closing the pane clears the records. Orphan records, late writes and other
/// NOBLE instances are covered along the way.
#[test]
fn agent_state_follows_the_program_in_the_pane() {
    check_agent_lifecycle("");
    if cfg!(windows) {
        check_agent_lifecycle("cmd.exe");
    } else {
        for shell in ["bash", "zsh", "fish", "pwsh"] {
            if let Some(path) = noble::util::which(shell) {
                check_agent_lifecycle(&path.display().to_string());
            }
        }
    }
}

fn check_agent_lifecycle(shell: &str) {
    use noble::app::AgentState;
    let mut app = demo_app(160, 45);
    if !shell.is_empty() {
        let mut cfg = app.cfg.clone();
        cfg.terminal.shell = shell.into();
        app.apply_config(cfg);
    }
    let tag = std::path::Path::new(shell)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "default".into());
    // Own data folder: headless apps share one, and panes of parallel tests have the same ids.
    app.paths.data = std::env::temp_dir().join(format!("noble-agent-life-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&app.paths.data);
    let me = std::process::id();
    let agents = noble::hooks::agents_dir(&app.paths.data);

    // Quick launch, as `App::launch` does it: the tab is named after the launcher.
    let claude = fake_agent(&app.shell, "Claude Code");
    let project = std::env::temp_dir();
    let origin = format!("{} · claude", project.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap());
    app.new_tab(project, Some(&claude), Some(origin.clone()));
    let pane = app.tabs[0].focus;
    pump_until(&mut app, &format!("{shell}: agent title"), |a| a.panes[&pane].label().contains("Claude Code"));
    assert_eq!(app.agent_state(pane), Some(("claude", AgentState::Running)), "{shell}: no hook yet");
    assert_eq!(app.tab_title(0), origin);

    // Claude's hooks.
    let steps = [
        ("session-start", "{}", AgentState::Idle),
        ("prompt", r#"{"prompt":"fix it"}"#, AgentState::Working),
        ("notification", r#"{"message":"Claude needs your permission to use Bash"}"#, AgentState::NeedsYou),
        ("stop", "{}", AgentState::Idle),
    ];
    for (event, stdin, want) in steps {
        noble_hook(&app, pane, me, event, stdin);
        scan_hooks(&mut app);
        assert_eq!(app.agent_state(pane), Some(("claude", want)), "{shell}: after {event}");
    }
    // Another NOBLE instance's pane with the same id is not ours.
    noble_hook(&app, pane, me + 1, "notification", "{}");
    // An orphan: a pane this instance never had (or already closed).
    noble_hook(&app, 9999, me, "prompt", "{}");
    scan_hooks(&mut app);
    assert!(!agents.join(format!("{me}-9999.json")).exists(), "{shell}: orphan record kept");
    assert!(agents.join(format!("{}-{pane}.json", me + 1)).exists(), "{shell}: other instance's record removed");

    // `/exit`: SessionEnd clears the record; the program itself is still closing.
    noble_hook(&app, pane, me, "session-end", r#"{"reason":"prompt_input_exit"}"#);
    scan_hooks(&mut app);
    assert!(!app.agent_hooks.contains_key(&pane), "{shell}: session-end left the record");
    assert!(!agents.join(format!("{me}-{pane}.json")).exists());

    // A stop hook that races the exit lands after session-end.
    noble_hook(&app, pane, me, "stop", "{}");
    // The agent exits; the shell prompt comes back.
    app.panes[&pane].write(b"\r");
    pump_until(&mut app, &format!("{shell}: prompt after the agent"), |a| a.panes[&pane].prompted);
    scan_hooks(&mut app);
    assert_eq!(app.agent_state(pane), None, "{shell}: agent shown at the shell prompt");
    // The title is gone, or is the one the shell sets for its own prompt.
    let label = app.panes[&pane].label();
    assert!(!label.contains("Claude"), "{shell}: the agent's title outlived it: {label}");
    assert!(!app.tab_title(0).contains("claude"), "{shell}: tab still says {:?}", app.tab_title(0));
    assert!(!agents.join(format!("{me}-{pane}.json")).exists(), "{shell}: record left after the prompt");
    let text = render(&mut app, 160, 45);
    save(&format!("agent-back-at-shell-{tag}"), &text);
    let top = text.lines().next().unwrap_or_default();
    assert!(!top.contains("claude"), "{shell}: top bar still says claude: {top}");

    // Another harness in the same pane.
    let codex = fake_agent(&app.shell, "codex");
    app.panes[&pane].write(format!("{codex}\r").as_bytes());
    pump_until(&mut app, &format!("{shell}: second agent"), |a| a.agent_state(pane).is_some());
    assert_eq!(app.agent_state(pane), Some(("codex", AgentState::Running)), "{shell}");
    // An elevated cmd.exe prefixes its title with "Administrator: ", which the tab's 16-character
    // label cuts before the program name; the pane label still carries it. Other shells show it.
    let tab = app.tab_title(0);
    let label = app.panes[&pane].label();
    assert!(tab.contains("codex") || (shell == "cmd.exe" && label.contains("codex")), "{shell}: {tab:?} / {label:?}");
    assert!(!tab.contains("claude"), "{shell}: {tab:?}");
    app.panes[&pane].write(b"\r");
    pump_until(&mut app, &format!("{shell}: back at the shell"), |a| a.agent_state(pane).is_none());
    assert!(!app.panes[&pane].label().contains("codex"), "{shell}");

    // Claude again, typed at the prompt this time; closing the pane clears its record.
    // Records written in the same second as the last prompt count as leftovers of the
    // program before it (hook timestamps are in seconds), so wait for the next second.
    std::thread::sleep(Duration::from_millis(1100));
    noble_hook(&app, pane, me, "session-start", "{}");
    scan_hooks(&mut app);
    assert_eq!(app.agent_state(pane), Some(("claude", AgentState::Idle)), "{shell}");
    app.run(Action::CloseTab);
    assert!(app.agent_hooks.is_empty());
    assert!(!agents.join(format!("{me}-{pane}.json")).exists(), "{shell}: record left after close");
    let _ = std::fs::remove_dir_all(&app.paths.data);
}

/// A warning row appears in the AI panel when the quota would fill before it resets.
#[test]
fn quota_pace_warning_line() {
    let mut app = demo_app(160, 45);
    let now = chrono::Utc::now().timestamp();
    let key = noble::store::UsageHistory::key("claude", "5H");
    app.usage_history = noble::store::UsageHistory::memory();
    app.usage_history.record(&key, now - 40 * 60, 40);
    app.usage_history.record(&key, now, 71);
    let text = render(&mut app, 160, 45);
    save("bridge-pace-160x45", &text);
    assert!(text.contains("▲ 5h full in ~37m at this pace"), "{text}");
    // No estimate for the weekly window even when it fills fast (5 hour window only).
    let week = noble::store::UsageHistory::key("codex", "WEEK");
    app.usage_history = noble::store::UsageHistory::memory();
    app.usage_history.record(&week, now - 3600, 80);
    app.usage_history.record(&week, now, 92);
    let text = render(&mut app, 160, 45);
    assert!(!text.contains("at this pace"), "{text}");
}

/// `noble hook <event>`: the compiled binary reads the JSON on stdin and writes
/// the state file; outside NOBLE (no env var) it writes nothing and always exits 0.
#[test]
fn hook_cli_writes_state_file() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let home = std::env::temp_dir().join(format!("noble-hookcli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let run = |with_env: bool| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_noble"));
        cmd.args(["hook", "notification"]).env("NOBLE_HOME", &home).stdin(Stdio::piped());
        cmd.env_remove("NOBLE_INSTANCE").env_remove("NOBLE_PANE");
        if with_env {
            cmd.env("NOBLE_INSTANCE", "4242").env("NOBLE_PANE", "7");
        }
        let mut child = cmd.spawn().unwrap();
        child.stdin.take().unwrap().write_all(br#"{"message":"Claude is waiting for your input"}"#).unwrap();
        child.wait().unwrap()
    };
    assert!(run(false).success());
    assert!(noble::hooks::read_records(&home, 4242).is_empty());
    assert!(run(true).success());
    let recs = noble::hooks::read_records(&home, 4242);
    assert_eq!(recs[&7].message.as_deref(), Some("Claude is waiting for your input"));
    let _ = std::fs::remove_dir_all(&home);
}

/// Waits until the shell's startup output settles (content unchanged for 700 ms).
fn wait_idle(app: &mut App, pane: noble::term::layout::PaneId) {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut last = String::new();
    let mut stable = std::time::Instant::now();
    while std::time::Instant::now() < deadline {
        app.pump();
        let now = app.panes[&pane].parser().screen().contents();
        if now != last {
            last = now;
            stable = std::time::Instant::now();
        } else if !last.trim().is_empty() && stable.elapsed() > Duration::from_millis(700) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
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

/// Battery friendly drawing: invisible data never wakes the screen; an idle
/// terminal redraws once a minute, Home once a second, animation/palette more often.
#[test]
fn redraw_only_when_something_visible_changes() {
    use noble::battery::{Battery, PowerState};
    let mut app = demo_app(110, 30);
    let mut sample = app.sensors.last.clone().unwrap();
    // Plugged in: drawn within 1 s for Home's clock with seconds.
    sample.battery = Some(Battery { percent: 80.0, state: PowerState::Full, secs_left: None, secs_to_full: None });
    assert!(app.handle(AppEvent::Sensors(Box::new(sample.clone()))));
    assert!(app.live_clock());
    let home = app.redraw_after().unwrap();
    assert!(home <= Duration::from_millis(1005), "{home:?}");
    // On battery: seconds and blinking stop, the clock draws once a minute.
    sample.battery =
        Some(Battery { percent: 80.0, state: PowerState::Discharging, secs_left: None, secs_to_full: None });
    app.handle(AppEvent::Sensors(Box::new(sample.clone())));
    assert!(!app.live_clock());
    let home = app.redraw_after().unwrap();
    assert!(home > Duration::from_millis(1005) || chrono::Local::now().format("%S").to_string() == "59", "{home:?}");
    // Terminal: sensor/project events do not wake the screen, the clock draws once a minute.
    app.new_tab(std::env::temp_dir(), None, Some("quiet".into()));
    app.toasts.clear();
    assert!(!app.handle(AppEvent::Sensors(Box::new(sample))));
    assert!(!app.handle(AppEvent::Projects(app.projects.clone())));
    let term = app.redraw_after().unwrap();
    assert!(term <= Duration::from_secs(61), "{term:?}");
    // A key always redraws; the palette cursor blinks twice a second (alt+p is the shell's in a terminal).
    let key = |c, m| AppEvent::Input(crossterm::event::Event::Key(KeyEvent::new(KeyCode::Char(c), m)));
    assert!(app.handle(key('a', KeyModifiers::CONTROL)));
    assert!(app.handle(key(':', KeyModifiers::NONE)));
    assert!(app.redraw_after().unwrap() <= Duration::from_millis(500));
    app.overlay = None;
    // We wake at the moment a notification must expire and disappear.
    app.toast(noble::app::ToastLevel::Info, "hello");
    assert!(app.take_dirty());
    assert!(app.redraw_after().unwrap() <= Duration::from_millis(3100));
    app.run(Action::CloseTab);
}

/// The big Home clock draws without seconds on battery and with seconds plugged in.
#[test]
fn clock_hides_seconds_on_battery() {
    use noble::battery::{Battery, PowerState};
    let mut app = demo_app(160, 45);
    let mut sample = app.sensors.last.clone().unwrap();
    sample.battery = Some(Battery { percent: 80.0, state: PowerState::Charging, secs_left: None, secs_to_full: None });
    app.handle(AppEvent::Sensors(Box::new(sample.clone())));
    let plugged = render(&mut app, 160, 45);
    sample.battery =
        Some(Battery { percent: 80.0, state: PowerState::Discharging, secs_left: None, secs_to_full: None });
    app.handle(AppEvent::Sensors(Box::new(sample)));
    let battery = render(&mut app, 160, 45);
    save("bridge-clock-battery-160x45", &battery);
    // The small seconds area right of the big clock (the digits' last row).
    let secs_row = |t: &str| t.lines().nth(4).unwrap_or("").chars().take(24).collect::<String>();
    let digits = |s: String| s.chars().filter(|c| c.is_ascii_digit()).count();
    assert_eq!(digits(secs_row(&plugged)), 2, "seconds shown when plugged in:\n{plugged}");
    assert_eq!(digits(secs_row(&battery)), 0, "no seconds on battery:\n{battery}");
}

#[test]
fn every_provider_is_configurable() {
    use noble::app::{PROVIDER_KEYS, SettingItem};
    let mut app = demo_app(110, 30);
    let ids: Vec<&str> = noble::ai::providers::registry().iter().map(|d| d.id).collect();
    // Every provider has a setting and the default config knows them all.
    let keyed: Vec<&str> = PROVIDER_KEYS.iter().map(|k| k.provider_id()).collect();
    assert_eq!(keyed, ids);
    let parsed = noble::config::parse(noble::config::DEFAULT_CONFIG).unwrap();
    assert_eq!(parsed.ai.providers, ids);
    // The toggle really changes something.
    for key in PROVIDER_KEYS {
        let before = app.setting_on(key);
        app.activate_setting(SettingItem::Setting(key), 1);
        assert_ne!(app.setting_on(key), before, "{key:?}");
    }
}

/// New release: a notice appears bottom right, drops to the status bar in a
/// terminal and, once dismissed, is never shown again for the same version.
#[test]
fn update_notice_bottom_right() {
    let mut app = demo_app(120, 32);
    app.handle(AppEvent::Update(Ok(noble::update::current().into())));
    assert!(app.update_notice().is_none(), "same version must not be announced");
    app.handle(AppEvent::Update(Ok("99.0.0".into())));
    assert_eq!(app.update_notice(), Some("99.0.0"));
    let text = render(&mut app, 120, 32);
    save("update-notice-120x32", &text);
    let row = text.lines().nth(30).unwrap();
    assert!(row.contains("NOBLE 99.0.0 is available") && row.trim_end().ends_with("update  ×"), "{text}");
    let hit = find_hit(&app, |h| *h == noble::app::Hit::Update).expect("update hit");
    assert_eq!((hit.y, hit.x + hit.width + 4), (30, 120));
    for (w, h) in [(160, 45), (80, 24), (40, 12), (30, 8)] {
        save(&format!("update-notice-{w}x{h}"), &render(&mut app, w, h));
    }
    // Not shown when the setting is off.
    app.cfg.general.check_updates = false;
    assert!(!render(&mut app, 120, 32).contains("99.0.0"));
    app.cfg.general.check_updates = true;

    // On a terminal tab the shell's last line stays clear: the notice sits in the status bar.
    app.new_tab(std::env::temp_dir(), None, Some("shell".into()));
    let text = render(&mut app, 120, 32);
    save("update-notice-terminal-120x32", &text);
    let last = text.lines().last().unwrap();
    assert!(last.contains("↑ 99.0.0") && !text.lines().nth(30).unwrap().contains("99.0.0"), "{text}");
    app.run(Action::CloseTab);

    render(&mut app, 120, 32);
    let close = find_hit(&app, |h| *h == noble::app::Hit::UpdateDismiss).expect("dismiss hit");
    click(&mut app, close.x, close.y);
    assert!(app.update_notice().is_none());
    assert!(!render(&mut app, 120, 32).contains("99.0.0"));
    app.handle(AppEvent::Update(Ok("99.0.0".into())));
    assert!(app.update_notice().is_none(), "dismissed version stays hidden");
    app.handle(AppEvent::Update(Ok("99.0.1".into())));
    assert_eq!(app.update_notice(), Some("99.0.1"));
}

/// Shell integration edge cases for bash, zsh and fish (`term/integration.rs`): unusual
/// directory names, user startup options, the user's own prompt hooks and `ZDOTDIR`.
/// Every shell starts through a small wrapper named after it (so NOBLE treats it as that
/// shell) that points HOME, XDG_CONFIG_HOME and XDG_DATA_HOME at a temporary directory: the
/// tests write their own "user config" there and never read the real one.
/// Unix only: the wrapper is a POSIX script. On Windows the same scripts run under Git Bash,
/// whose cwd tracking `cwd_is_tracked_after_cd` covers; pwsh and cmd report with OSC 9;9.
#[cfg(unix)]
mod shells {
    use super::*;
    use noble::term::layout::PaneId;
    use std::path::Path;

    struct IsolatedShell {
        shell: &'static str,
        home: PathBuf,
        wrapper: PathBuf,
    }

    impl IsolatedShell {
        /// `None` when the shell is not installed. `env` is exported by the wrapper as well.
        fn new(shell: &'static str, case: &str, env: &[(&str, &str)]) -> Option<IsolatedShell> {
            use std::os::unix::fs::PermissionsExt;
            let real = noble::util::which(shell)?;
            let root = std::env::temp_dir().join(format!("noble-shell-{case}-{shell}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let home = root.join("home");
            std::fs::create_dir_all(home.join(".config").join("fish")).unwrap();
            std::fs::create_dir_all(root.join("bin")).unwrap();
            let q = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
            let mut script = format!(
                "#!/bin/sh\nexport HOME={h} XDG_CONFIG_HOME={h}/.config XDG_DATA_HOME={h}/.local/share\n\
                 unset NOBLE_USER_ZDOTDIR PROMPT_COMMAND\n",
                h = q(&home.display().to_string())
            );
            for (k, v) in env {
                script.push_str(&format!("export {k}={}\n", q(v)));
            }
            script.push_str(&format!("exec {} \"$@\"\n", q(&real.display().to_string())));
            let wrapper = root.join("bin").join(shell);
            std::fs::write(&wrapper, script).unwrap();
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
            Some(IsolatedShell { shell, home, wrapper })
        }

        fn write(&self, rel: &str, text: &str) {
            let path = self.home.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }

        /// Opens a tab with this shell and `args` in `cwd`.
        fn open(&self, args: &[&str], cwd: &Path) -> (App, PaneId) {
            let mut app = demo_app(110, 30);
            let mut cfg = app.cfg.clone();
            cfg.terminal.shell = self.wrapper.display().to_string();
            cfg.terminal.shell_args = args.iter().map(|a| a.to_string()).collect();
            app.apply_config(cfg);
            app.new_tab(cwd.to_path_buf(), None, Some("shell".into()));
            let id = app.tabs[0].focus;
            (app, id)
        }
    }

    impl Drop for IsolatedShell {
        fn drop(&mut self) {
            if let Some(root) = self.home.parent() {
                let _ = std::fs::remove_dir_all(root);
            }
        }
    }

    /// Pumps until the pane's reported directory (OSC 7) is `want`; panics with the screen otherwise.
    fn wait_cwd(app: &mut App, id: PaneId, want: &Path, what: &str) {
        let want = std::fs::canonicalize(want).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            app.pump();
            let reported = app.panes[&id].parser().callbacks().cwd.clone();
            if reported.as_ref().and_then(|c| std::fs::canonicalize(c).ok()) == Some(want.clone()) {
                return;
            }
            if std::time::Instant::now() > deadline {
                let screen = app.panes[&id].parser().screen().contents();
                panic!("{what}: reported cwd {reported:?}, want {want:?}\n{screen}");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Pumps until the pane's screen contains `needle`.
    fn wait_screen(app: &mut App, id: PaneId, needle: &str, what: &str) {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            app.pump();
            let screen = app.panes[&id].parser().screen().contents();
            if screen.contains(needle) {
                return;
            }
            assert!(std::time::Instant::now() < deadline, "{what}: {needle:?} never appeared\n{screen}");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// OSC 7 round trip: directories with spaces, Unicode, `%`, `#`, `;`, `?` and a trailing
    /// space are reported by every installed bash/zsh/fish and decoded back to the same path.
    #[test]
    fn cwd_with_special_characters_is_tracked() {
        for shell in ["bash", "zsh", "fish"] {
            let Some(sh) = IsolatedShell::new(shell, "chars", &[]) else { continue };
            let base = sh.home.join("dir with spaces ü 日本");
            let odd = base.join("100% #1;x? ");
            std::fs::create_dir_all(&odd).unwrap();
            let (mut app, id) = sh.open(&[], &base);
            wait_cwd(&mut app, id, &base, &format!("{shell} start"));
            app.panes[&id].write(b"cd '100% #1;x? '\r");
            wait_cwd(&mut app, id, &odd, &format!("{shell} cd"));
            assert_eq!(app.panes[&id].cwd(), odd, "{}", sh.shell);
            app.run(Action::CloseTab);
        }
    }

    /// Types `cd <dir>` and checks both the OSC 7 report and the user's own hook marker.
    fn cd_and_check(app: &mut App, id: PaneId, dir: &Path, marker: &str, what: &str) {
        app.panes[&id].write(format!("cd '{}'\r", dir.display()).as_bytes());
        wait_cwd(app, id, dir, what);
        wait_screen(app, id, marker, what);
    }

    /// The user's own prompt hooks keep running next to NOBLE's, which still reports the cwd.
    #[test]
    fn user_prompt_hooks_keep_working() {
        let target = std::env::temp_dir().join(format!("noble-hooks-{}", std::process::id()));
        std::fs::create_dir_all(&target).unwrap();
        // bash: a string PROMPT_COMMAND ending in a comment, and the bash 5.1+ array form.
        for (case, rc) in [
            ("bash-str", "PROMPT_COMMAND='__u=$((__u+1)); echo \"UHOOK$__u\" # user hook'\n"),
            ("bash-arr", "__u=0\nPROMPT_COMMAND=('__u=$((__u+1))' 'echo \"UHOOK$__u\"')\n"),
        ] {
            let Some(sh) = IsolatedShell::new("bash", case, &[]) else { break };
            sh.write(".bashrc", rc);
            let (mut app, id) = sh.open(&[], &sh.home);
            wait_cwd(&mut app, id, &sh.home, case);
            wait_screen(&mut app, id, "UHOOK1", case);
            cd_and_check(&mut app, id, &target, "UHOOK2", case);
            app.run(Action::CloseTab);
        }
        if let Some(sh) = IsolatedShell::new("zsh", "hooks", &[]) {
            sh.write(
                ".zshrc",
                "precmd() { __u=$((__u+1)); echo \"UHOOK$__u\" }\n\
                 __v() { echo VHOOK }\nprecmd_functions+=(__v)\n",
            );
            let (mut app, id) = sh.open(&[], &sh.home);
            wait_cwd(&mut app, id, &sh.home, "zsh");
            wait_screen(&mut app, id, "UHOOK1", "zsh");
            wait_screen(&mut app, id, "VHOOK", "zsh");
            cd_and_check(&mut app, id, &target, "UHOOK2", "zsh");
            app.run(Action::CloseTab);
        }
        if let Some(sh) = IsolatedShell::new("fish", "hooks", &[]) {
            sh.write(
                ".config/fish/config.fish",
                "set -g __u 0\nfunction fish_prompt\n    set -g __u (math $__u + 1)\n    echo \"UHOOK$__u> \"\nend\n",
            );
            let (mut app, id) = sh.open(&[], &sh.home);
            wait_cwd(&mut app, id, &sh.home, "fish");
            wait_screen(&mut app, id, "UHOOK1>", "fish");
            cd_and_check(&mut app, id, &target, "UHOOK2>", "fish");
            app.run(Action::CloseTab);
        }
        let _ = std::fs::remove_dir_all(&target);
    }

    /// zsh finds the user's startup files in their own `ZDOTDIR`: one set by `~/.zshenv`
    /// (the common `ZDOTDIR=~/.config/zsh` setup) and one NOBLE inherited from its environment
    /// (`extra_env` hands it over as `NOBLE_USER_ZDOTDIR`).
    #[test]
    fn zsh_user_zdotdir_is_sourced() {
        let target = std::env::temp_dir().join(format!("noble-zdotdir-{}", std::process::id()));
        std::fs::create_dir_all(&target).unwrap();
        let Some(sh) = IsolatedShell::new("zsh", "zdot-env", &[]) else { return };
        sh.write(".zshenv", "ZDOTDIR=$HOME/.config/zsh\n");
        sh.write(".config/zsh/.zshrc", "precmd() { echo ZRC_HOOK }\n");
        let (mut app, id) = sh.open(&[], &sh.home);
        wait_cwd(&mut app, id, &sh.home, "zshenv ZDOTDIR");
        wait_screen(&mut app, id, "ZRC_HOOK", "zshenv ZDOTDIR");
        app.panes[&id].write(b"echo \"ZD=$ZDOTDIR\"\r");
        wait_screen(&mut app, id, &format!("ZD={}", sh.home.join(".config/zsh").display()), "zshenv ZDOTDIR");
        cd_and_check(&mut app, id, &target, "ZRC_HOOK", "zshenv ZDOTDIR");
        app.run(Action::CloseTab);
        drop(sh);

        let root = std::env::temp_dir().join(format!("noble-shell-zdot-inherit-zsh-{}", std::process::id()));
        let user = root.join("home").join("zdot");
        let Some(sh) =
            IsolatedShell::new("zsh", "zdot-inherit", &[("NOBLE_USER_ZDOTDIR", &user.display().to_string())])
        else {
            return;
        };
        sh.write("zdot/.zshenv", "export ZENV_SEEN=1\n");
        sh.write("zdot/.zshrc", "precmd() { echo \"ZRC2_HOOK$ZENV_SEEN\" }\n");
        let (mut app, id) = sh.open(&[], &sh.home);
        wait_cwd(&mut app, id, &sh.home, "inherited ZDOTDIR");
        wait_screen(&mut app, id, "ZRC2_HOOK1", "inherited ZDOTDIR");
        cd_and_check(&mut app, id, &target, "ZRC2_HOOK1", "inherited ZDOTDIR");
        app.run(Action::CloseTab);
        let _ = std::fs::remove_dir_all(&target);
    }

    /// User shell options: login shells keep the integration (and load their login files);
    /// options that skip every startup file start the shell as asked.
    #[test]
    fn user_shell_args_and_integration() {
        let target = std::env::temp_dir().join(format!("noble-args-{}", std::process::id()));
        std::fs::create_dir_all(&target).unwrap();
        // bash login shells read ~/.bash_profile, not the rc file: NOBLE loads it itself.
        for login in ["-l", "--login"] {
            let Some(sh) = IsolatedShell::new("bash", &format!("login{login}"), &[]) else { break };
            sh.write(".bash_profile", "echo PROFILE_LOADED\n. ~/.bashrc\n");
            sh.write(".bashrc", "PROMPT_COMMAND='echo RC_HOOK'\n");
            let (mut app, id) = sh.open(&[login], &sh.home);
            wait_cwd(&mut app, id, &sh.home, login);
            wait_screen(&mut app, id, "PROFILE_LOADED", login);
            wait_screen(&mut app, id, "RC_HOOK", login);
            cd_and_check(&mut app, id, &target, "RC_HOOK", login);
            app.run(Action::CloseTab);
        }
        // The documented opt-out (README): these start the shell untouched, without the user's
        // files and without NOBLE's hook, and the shell still works.
        for (shell, arg) in
            [("bash", "--norc"), ("zsh", "-f"), ("zsh", "--no-rcs"), ("fish", "--no-config"), ("fish", "-N")]
        {
            let Some(sh) = IsolatedShell::new(shell, &format!("norc{arg}"), &[]) else { continue };
            sh.write(".bashrc", "echo RC_READ\n");
            sh.write(".zshrc", "echo RC_READ\n");
            sh.write(".config/fish/config.fish", "echo RC_READ\n");
            let (mut app, id) = sh.open(&[arg], &sh.home);
            let probe = if shell == "fish" { "echo ALIVE(math 1+1)\r" } else { "echo ALIVE$((1+1))\r" };
            app.panes[&id].write(probe.as_bytes());
            wait_screen(&mut app, id, "ALIVE2", arg);
            let screen = app.panes[&id].parser().screen().contents();
            assert!(!screen.contains("RC_READ"), "{shell} {arg}: rc file was read\n{screen}");
            app.run(Action::CloseTab);
        }
        let _ = std::fs::remove_dir_all(&target);
    }
}

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

/// Degenerate sizes: zero width or height, 1×1, one row, one column.
const TINY: [(u16, u16); 12] =
    [(0, 0), (0, 10), (10, 0), (1, 1), (2, 2), (3, 1), (1, 3), (8, 2), (30, 8), (300, 1), (1, 120), (12, 5)];

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

/// A saved workspace edited by hand or damaged (focus out of range, a huge or infinite ratio —
/// JSON `1e39` reads as an infinite f32 — a missing or invalid directory) still opens and draws.
#[test]
fn damaged_workspace_restores() {
    use noble::term::layout::Dir;
    let json = r#"{"name":"é","saved_at":-9223372036854775808,"tabs":[{"name":"日本","origin":"","focus":99,
        "layout":{"type":"split","dir":"Row","ratio":1e39,"a":{"type":"leaf","cwd":"\u0000"},
        "b":{"type":"split","dir":"Col","ratio":-5,"a":{"type":"leaf","cwd":"/no/such/dir"},
        "b":{"type":"leaf","cwd":""}}}}]}"#;
    let ws: Workspace = serde_json::from_str(json).unwrap();
    assert!(matches!(&ws.tabs[0].layout, SavedNode::Split { dir: Dir::Row, ratio, .. } if ratio.is_infinite()));
    let mut app = demo_app(100, 30);
    assert_eq!(app.open_workspace(&ws), 1);
    app.run(Action::GoTab(1));
    for (w, h) in SIZES.into_iter().chain(TINY) {
        render(&mut app, w, h);
    }
    app.run(Action::ResizeLeft);
    app.run(Action::FocusRight);
    app.run(Action::ResizeDown);
    render(&mut app, 100, 30);
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
    let safe_click = |h: &Hit| {
        !matches!(
            h,
            Hit::Launcher(_)
                | Hit::MenuItem(_)
                | Hit::PaletteItem(_)
                | Hit::OpenFiles
                | Hit::OpenSelected
                | Hit::Update
        )
    };
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
                let items = app.settings_items();
                let config_row =
                    |i: usize| matches!(items.get(i), Some(SettingItem::OpenConfig | SettingItem::ReloadConfig));
                let targets: Vec<ratatui::layout::Rect> = app
                    .hits
                    .iter()
                    .filter(|(_, hit)| safe_click(hit))
                    .filter(|(_, hit)| !matches!(hit, Hit::Setting(i) if config_row(*i)))
                    .filter(|(_, hit)| spawn_ok || !matches!(hit, Hit::NewTab | Hit::PaneSplit { .. }))
                    .map(|(r, _)| *r)
                    .collect();
                if !targets.is_empty() {
                    let r = *rng.pick(&targets);
                    let x = r.x + rng.below(r.width as usize) as u16;
                    let y = r.y + rng.below(r.height as usize) as u16;
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
                chunk.extend_from_slice(rng.pick(&pieces));
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

// ─── Session save / restore ───────────────────────────────────────────────

/// A headless app whose data directory (`session.json` / `session-dev.json`) is `data`.
fn session_app(data: &std::path::Path, dev: bool, cfg: Config) -> App {
    let mut app = App::headless(cfg, (110, 30));
    app.paths.data = data.to_path_buf();
    app.dev = dev;
    app
}

fn session_cfg() -> Config {
    let mut cfg = Config::default();
    cfg.general.animations = false;
    cfg
}

fn session_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("noble-session-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn session_file(dev: bool) -> &'static str {
    if dev { "session-dev.json" } else { "session.json" }
}

fn one_tab(origin: &str, layout: SavedNode) -> Workspace {
    Workspace {
        name: "last session".into(),
        saved_at: 0,
        tabs: vec![SavedTab { name: None, origin: origin.into(), layout, focus: 0 }],
    }
}

/// Writes a session file the way older versions did (a plain workspace, no window tags).
fn write_session(file: &std::path::Path, ws: &Workspace) {
    std::fs::write(file, serde_json::to_string(ws).unwrap()).unwrap();
}

fn leaf(dir: &std::path::Path) -> SavedNode {
    SavedNode::Leaf { cwd: dir.display().to_string(), launch: None }
}

fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
    std::fs::canonicalize(a).ok() == std::fs::canonicalize(b).ok()
}

fn start_cwds(app: &App, tab: usize) -> Vec<PathBuf> {
    app.tabs[tab].panes().iter().map(|id| app.panes[id].start_cwd.clone()).collect()
}

fn toast_texts(app: &App) -> Vec<String> {
    app.toasts.iter().map(|t| t.text.clone()).collect()
}

fn session_origins(data: &std::path::Path, dev: bool) -> Vec<String> {
    let text = std::fs::read_to_string(data.join(session_file(dev))).unwrap_or_default();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    json["tabs"].as_array().into_iter().flatten().filter_map(|t| t["origin"].as_str().map(String::from)).collect()
}

fn tab_origins(app: &App) -> Vec<String> {
    app.tabs.iter().map(|t| t.origin.clone()).collect()
}

/// Several windows share one session file: each one merges only its own tabs into it on exit,
/// so the tabs of every window come back on the next launch (and only once).
#[test]
fn every_window_keeps_its_tabs_in_the_session() {
    for dev in [false, true] {
        let data = session_dir(if dev { "multi-dev" } else { "multi" });
        let mut a = session_app(&data, dev, session_cfg());
        a.restore_last_session();
        a.new_tab(data.clone(), None, Some("window-a".into()));
        let mut b = session_app(&data, dev, session_cfg());
        b.restore_last_session();
        b.new_tab(data.clone(), None, Some("window-b".into()));
        a.shutdown();
        b.shutdown();
        let mut origins = session_origins(&data, dev);
        origins.sort();
        assert_eq!(origins, ["window-a", "window-b"], "{}: tabs of a window were lost", session_file(dev));

        // The next launch restores both windows' tabs, once.
        let mut c = session_app(&data, dev, session_cfg());
        c.restore_last_session();
        let mut restored = tab_origins(&c);
        restored.sort();
        assert_eq!(restored, ["window-a", "window-b"], "{}", session_file(dev));
        // Closing them for good: they do not come back.
        while !c.tabs.is_empty() {
            c.remove_tab(0);
        }
        c.shutdown();
        let mut d = session_app(&data, dev, session_cfg());
        d.restore_last_session();
        assert!(d.tabs.is_empty(), "{}: closed tabs came back: {:?}", session_file(dev), tab_origins(&d));
        d.shutdown();
        let _ = std::fs::remove_dir_all(&data);
    }
}

/// A second window opened while the first one runs starts empty instead of restoring the
/// same tabs again; after both close, the next launch has each tab exactly once. Also loads a
/// session file written by an older version (no instance tags).
#[test]
fn a_second_window_does_not_restore_the_session_again() {
    for dev in [false, true] {
        let data = session_dir(if dev { "second-dev" } else { "second" });
        let old = one_tab("saved", leaf(&data));
        write_session(&data.join(session_file(dev)), &old);
        let mut a = session_app(&data, dev, session_cfg());
        a.restore_last_session();
        assert_eq!(tab_origins(&a), ["saved"], "{}", session_file(dev));
        let mut b = session_app(&data, dev, session_cfg());
        b.restore_last_session();
        assert!(b.tabs.is_empty(), "{}: second window duplicated {:?}", session_file(dev), tab_origins(&b));
        assert_eq!(b.restored_tabs, 0);
        b.shutdown();
        a.shutdown();
        let mut c = session_app(&data, dev, session_cfg());
        c.restore_last_session();
        assert_eq!(tab_origins(&c), ["saved"], "{}", session_file(dev));
        c.shutdown();
        let _ = std::fs::remove_dir_all(&data);
    }
}

/// A window that crashed (still listed as running, its process gone) does not stop the next
/// launch from restoring; its tabs come back too.
#[test]
fn a_crashed_window_does_not_block_the_restore() {
    let data = session_dir("crashed");
    let json = serde_json::json!({
        "saved_at": 0,
        "running": ["4000000000-1-0"],
        "tabs": [{ "instance": "4000000000-1-0", "name": null, "origin": "crashed",
                   "layout": { "type": "leaf", "cwd": data.display().to_string() }, "focus": 0 }],
    });
    std::fs::write(data.join("session.json"), json.to_string()).unwrap();
    let mut app = session_app(&data, false, session_cfg());
    app.restore_last_session();
    assert_eq!(tab_origins(&app), ["crashed"]);
    app.shutdown();
    let _ = std::fs::remove_dir_all(&data);
}

/// A pane whose directory was deleted comes back in the home directory; the rest of the
/// layout is kept. Checked through the startup path for both session files.
#[test]
fn restore_falls_back_to_home_for_a_missing_dir() {
    for dev in [false, true] {
        let data = session_dir(if dev { "missing-dev" } else { "missing" });
        let gone = data.join("deleted-project");
        let layout = SavedNode::Split {
            dir: noble::term::layout::Dir::Row,
            ratio: 0.5,
            a: Box::new(leaf(&gone)),
            b: Box::new(leaf(&data)),
        };
        write_session(&data.join(session_file(dev)), &one_tab("restored", layout));
        // The other build's file must not be read.
        write_session(&data.join(session_file(!dev)), &one_tab("other-build", leaf(&data)));
        let mut app = session_app(&data, dev, session_cfg());
        app.restore_last_session();
        assert_eq!(app.restored_tabs, 1, "{}", session_file(dev));
        assert_eq!(app.tabs[0].origin, "restored");
        let cwds = start_cwds(&app, 0);
        assert_eq!(cwds.len(), 2, "split lost: {cwds:?}");
        assert!(same_dir(&cwds[0], &dirs::home_dir().unwrap()), "{cwds:?}");
        assert!(same_dir(&cwds[1], &data), "{cwds:?}");
        assert!(toast_texts(&app).iter().any(|t| t.contains("deleted-project")), "{:?}", toast_texts(&app));
        app.run(Action::CloseTab);
        let _ = std::fs::remove_dir_all(&data);
    }
}

/// A directory that exists but cannot be entered (no permission) must not drop the tab: the
/// pane starts in the home directory instead.
#[test]
fn restore_survives_an_inaccessible_dir() {
    // Windows: denying access needs an ACL edit (no std API for it); the fallback it would hit
    // (a failed spawn is retried in home) is shared by both platforms and checked here on Unix.
    if cfg!(windows) {
        return;
    }
    let data = session_dir("locked");
    let locked = data.join("locked");
    std::fs::create_dir_all(&locked).unwrap();
    set_mode(&locked, 0o000);
    if std::fs::read_dir(&locked).is_ok() {
        // Running as root: permissions do not apply, nothing to check.
        set_mode(&locked, 0o755);
        let _ = std::fs::remove_dir_all(&data);
        return;
    }
    let mut app = session_app(&data, false, session_cfg());
    let opened = app.open_workspace(&one_tab("locked", leaf(&locked)));
    set_mode(&locked, 0o755);
    let _ = std::fs::remove_dir_all(&data);
    assert_eq!(opened, 1, "tab dropped; toasts: {:?}", toast_texts(&app));
    assert!(same_dir(&start_cwds(&app, 0)[0], &dirs::home_dir().unwrap()));
}

#[cfg(unix)]
fn set_mode(path: &std::path::Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[cfg(not(unix))]
fn set_mode(_: &std::path::Path, _: u32) {}

/// A network share that stopped answering (a stat that blocks for tens of seconds) must not
/// hold up startup: the probe gives up after a short timeout and the pane starts in home.
#[test]
fn restore_does_not_wait_for_a_hanging_share() {
    let data = session_dir("share");
    let share = data.join("dead-share");
    std::fs::create_dir_all(&share).unwrap();
    let mut app = session_app(&data, false, session_cfg());
    app.dir_probe = |p| {
        if p.ends_with("dead-share") {
            std::thread::sleep(Duration::from_secs(60));
        }
        p.is_dir()
    };
    let started = std::time::Instant::now();
    let opened = app.open_workspace(&one_tab("share", leaf(&share)));
    let took = started.elapsed();
    let _ = std::fs::remove_dir_all(&data);
    assert!(took < Duration::from_secs(10), "restore blocked for {took:?}");
    assert_eq!(opened, 1);
    assert!(same_dir(&start_cwds(&app, 0)[0], &dirs::home_dir().unwrap()), "{:?}", start_cwds(&app, 0));
    assert!(toast_texts(&app).iter().any(|t| t.contains("dead-share")), "{:?}", toast_texts(&app));
}

/// A quick-launch tab (e.g. an AI agent) remembers its launcher command in the session: on
/// restore the pane that ran it runs it again, in the same directory and with the same
/// "<dir> · <launcher>" title. Other panes split off in that tab come back as plain shells.
#[test]
fn restored_launch_tab_reruns_its_command() {
    let data = session_dir("launch");
    let project = data.join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let mut cfg = session_cfg();
    cfg.launchers = vec![noble::config::Launcher {
        key: "x".into(),
        name: "Agent".into(),
        command: "echo relaunch-marker".into(),
        show: true,
    }];
    fn marker_in(app: &App, pane: usize) -> bool {
        let id = app.tabs[0].panes()[pane];
        app.panes[&id].all_lines().0.iter().any(|l| l.contains("relaunch-marker"))
    }
    fn wait(app: &mut App, what: &dyn Fn(&App) -> bool) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while !what(app) && std::time::Instant::now() < deadline {
            app.pump();
            std::thread::sleep(Duration::from_millis(100));
        }
        what(app)
    }

    let mut app = session_app(&data, false, cfg.clone());
    app.restore_last_session();
    app.launch(0, Some(project.clone()));
    assert_eq!(app.tabs.len(), 1, "launcher did not open a tab");
    assert!(
        wait(&mut app, &|a: &App| marker_in(a, 0)),
        "the launcher command never ran:\n{}",
        render(&mut app, 110, 30)
    );
    let origin = app.tabs[0].origin.clone();
    assert!(origin.ends_with(" · Agent"), "{origin}");
    app.run(Action::SplitRight);
    assert_eq!(app.tabs[0].panes().len(), 2);
    app.shutdown();
    let text = std::fs::read_to_string(data.join("session.json")).unwrap();
    assert_eq!(text.matches("echo relaunch-marker").count(), 1, "launcher stored once, on its pane: {text}");

    for round in 0..2 {
        let mut again = session_app(&data, false, cfg.clone());
        again.restore_last_session();
        assert_eq!(again.restored_tabs, 1);
        assert_eq!(again.tabs[0].origin, origin, "title kept");
        assert!(same_dir(&start_cwds(&again, 0)[0], &project));
        assert!(
            wait(&mut again, &|a: &App| marker_in(a, 0)),
            "round {round}: the launcher command did not run again on restore:\n{}",
            render(&mut again, 110, 30)
        );
        // The split pane is a plain shell: wait for its prompt, then make sure nothing ran there.
        let split = again.tabs[0].panes()[1];
        assert!(wait(&mut again, &|a: &App| a.panes[&split].parser().callbacks().cwd.is_some()), "no prompt");
        again.pump();
        assert!(!marker_in(&again, 1), "the launcher ran in the split pane too");
        // Restored again on the next launch too.
        again.shutdown();
    }
    let _ = std::fs::remove_dir_all(&data);
}
