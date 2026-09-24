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
            layout: SavedNode::Leaf { cwd: "C:\\".into() },
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
    std::thread::sleep(Duration::from_millis(1500));
    app.pump();
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
    }
    let after = app.tabs[0].root.layout(app.body()).0[0].1.width;
    assert!(after + 15 <= before, "divider did not move: {before} -> {after}");
    save("terminal-mouse-drag-110x30", &render(&mut app, 110, 30));
    // The close button.
    let close = find_hit(&app, |h| matches!(h, Hit::PaneClose(_))).unwrap();
    click(&mut app, close.x + 1, close.y);
    assert_eq!(app.tabs[0].panes().len(), 2);
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
    // A key always redraws; the palette cursor blinks twice a second.
    assert!(
        app.handle(AppEvent::Input(crossterm::event::Event::Key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::ALT))))
    );
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
