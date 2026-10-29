//! Home: project cards, launchers, AI quota, the project list and adding folders.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use noble::ai::{Presence, ProviderState, Status, Usage, Window};
use noble::app::App;
use noble::event::AppEvent;
use noble::keys::Action;
use noble::pr::{Checks, PrInfo, PrState};
use noble::projects::Project;
use std::time::{Duration, Instant, SystemTime};

use super::common::*;

/// The selected project's card shows its quick tasks as TASKS chips at 160x45, the hint bar gets `g run
/// task`, and a project without tasks shows neither.
#[test]
fn bridge_card_shows_task_chips() {
    let tasks = ["npm run dev", "npm run build", "cargo test"];
    let mut app = demo_app(160, 45);
    let mut projects = app.projects.clone();
    for p in &mut projects {
        p.tasks = tasks.iter().map(|t| t.to_string()).collect();
    }
    app.handle(AppEvent::Projects(projects));
    let text = render(&mut app, 160, 45);
    save("bridge-tasks-160x45", &text);
    assert!(text.contains("TASKS") && text.contains("▸ npm run dev"), "{text}");
    // Narrower and shorter screens: the chips wrap or are left out, never drawn past the card.
    for (w, h) in SIZES {
        let text = render(&mut app, w, h);
        save(&format!("bridge-tasks-{w}x{h}"), &text);
    }
    assert!(text.contains("▸ cargo test") && text.contains("T run task"), "{text}");
    // Home's `T` lists the same tasks in the palette, in the TASK group.
    app.on_key(KeyEvent::new(KeyCode::Char('T'), KeyModifiers::SHIFT));
    let Some(noble::app::Overlay::Palette(st)) = &app.overlay else { panic!("g opens the task list") };
    let titles: Vec<&str> = st.matches.iter().map(|i| st.all[*i].title.as_str()).collect();
    assert_eq!(titles, ["Run: npm run dev", "Run: npm run build", "Run: cargo test"]);
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    // Without tasks: no section, no hint.
    let mut app = demo_app(160, 45);
    let text = render(&mut app, 160, 45);
    assert!(!text.contains("TASKS") && !text.contains("T run task"), "{text}");
    app.on_key(KeyEvent::new(KeyCode::Char('T'), KeyModifiers::SHIFT));
    assert!(app.overlay.is_none(), "T does nothing without tasks");
}

/// A launcher on `T` keeps the key: it runs, the task list does not open and the hint is left out.
#[test]
fn launcher_on_t_keeps_the_key() {
    let mut app = demo_app(160, 45);
    let mut projects = app.projects.clone();
    for p in &mut projects {
        p.tasks = vec!["cargo test".to_string()];
    }
    app.handle(AppEvent::Projects(projects));
    app.launchers = vec![(
        noble::config::Launcher { key: "T".into(), name: "tool".into(), command: "echo tool".into(), show: true },
        true,
    )];
    let tabs = app.tabs.len();
    let text = render(&mut app, 160, 45);
    assert!(!text.contains("T run task"), "{text}");
    app.on_key(KeyEvent::new(KeyCode::Char('T'), KeyModifiers::SHIFT));
    assert!(app.overlay.is_none(), "the launcher runs instead of the task list");
    assert_eq!(app.tabs.len(), tabs + 1, "the launcher opened its tab");
}

#[test]
fn bridge_shows_the_version() {
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    // Big hero (160x45) and compact hero (90x28) both show it; small sizes must not overflow.
    for (w, h) in [(160, 45), (110, 30), (90, 28)] {
        let mut app = demo_app(w, h);
        let text = render(&mut app, w, h);
        assert!(
            text.contains(&version),
            "{w}x{h}
{text}"
        );
    }
    for (w, h) in SIZES {
        let mut app = demo_app(w, h);
        let text = render(&mut app, w, h);
        assert!(text.lines().count() <= h as usize, "{w}x{h}");
        assert!(text.lines().all(|l| l.chars().count() <= w as usize), "{w}x{h}");
    }
}

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

/// Home's project card: the selected project's pull request, its CI and its listening ports sit on the
/// card's spacer row and are clickable (hit targets only: a click would open a browser). Changing the
/// checks changes the CI part; a small terminal moves the same parts to the footer summary, and a
/// tiny one drops them without overflow.
#[test]
fn project_card_shows_pull_request_and_ports() {
    use noble::app::{Hit, PrEntry};
    let mut app = demo_app(160, 45);
    let project = app.selected_project().expect("a selected project").clone();
    let pr = |checks: Checks| PrInfo {
        number: 42,
        title: "Show pull requests and ports on the project card".into(),
        state: PrState::Open,
        url: "https://github.com/MertSoylu/noble/pull/42".into(),
        branch: project.branch.clone().unwrap_or_default(),
        checks,
    };
    let answer = |app: &mut App, info: Option<PrInfo>| {
        app.prs.insert(project.path.clone(), PrEntry { branch: project.branch.clone(), info, fetched: Instant::now() });
    };
    answer(&mut app, Some(pr(Checks { passed: 12, failed: 0, pending: 0 })));
    app.ports.insert(project.path.clone(), vec![3000, 5173]);

    let text = render(&mut app, 160, 45);
    save("home-project-live-160x45", &text);
    assert!(text.contains("PR #42 open · ✓ CI 12/12"), "{text}");
    assert!(text.contains("⇄ :3000 :5173"), "{text}");
    assert!(find_hit(&app, |h| *h == Hit::ProjectPr).is_some(), "PR hit");
    assert!(find_hit(&app, |h| *h == Hit::Port(3000)).is_some(), "port hit");
    assert!(find_hit(&app, |h| *h == Hit::Port(5173)).is_some(), "second port hit");

    answer(&mut app, Some(pr(Checks { passed: 10, failed: 2, pending: 0 })));
    assert!(render(&mut app, 160, 45).contains("✗ CI 2 failing"));
    answer(&mut app, Some(pr(Checks { passed: 9, failed: 0, pending: 3 })));
    assert!(render(&mut app, 160, 45).contains("● CI 3 running"));
    // A pull request answer for another branch is not shown.
    app.prs.get_mut(&project.path).unwrap().branch = Some("old-branch".into());
    assert!(!render(&mut app, 160, 45).contains("PR #42"));
    answer(&mut app, None);
    let text = render(&mut app, 160, 45);
    assert!(text.contains("⇄ :3000 :5173") && !text.contains("PR #42"), "{text}");

    // 80x24: the card may be shown or hidden; either way the PR and the port are on screen and clickable.
    answer(&mut app, Some(pr(Checks { passed: 12, failed: 0, pending: 0 })));
    for size in [(80, 24), (40, 12)] {
        let text = render(&mut app, size.0, size.1);
        save(&format!("home-project-live-{}x{}", size.0, size.1), &text);
        assert!(text.lines().all(|l| l.chars().count() <= size.0 as usize), "overflow at {size:?}");
    }
    let text = render(&mut app, 80, 24);
    assert!(text.contains("PR #42") && text.contains(":3000"), "{text}");
    assert!(find_hit(&app, |h| *h == Hit::ProjectPr).is_some());
    assert!(find_hit(&app, |h| *h == Hit::Port(3000)).is_some());
}

/// Without the card (a narrow window) the Home footer carries the pull request and the ports: each is
/// drawn whole or left out, never cut to a clickable stub such as `PR #4…`, at any width.
#[test]
fn footer_pull_request_is_drawn_whole_or_not_at_all() {
    use noble::app::{Hit, PrEntry};
    let mut app = demo_app(110, 30);
    let project = app.selected_project().expect("a selected project").clone();
    let info = PrInfo {
        number: 42,
        title: "Show pull requests and ports on the project card".into(),
        state: PrState::Open,
        url: "https://github.com/MertSoylu/noble/pull/42".into(),
        branch: project.branch.clone().unwrap_or_default(),
        checks: Checks { passed: 12, failed: 0, pending: 0 },
    };
    app.prs.insert(
        project.path.clone(),
        PrEntry { branch: project.branch.clone(), info: Some(info), fetched: Instant::now() },
    );
    app.ports.insert(project.path.clone(), vec![3000, 5173]);
    let mut footer_pr = false;
    for w in 24..=110u16 {
        let text = render(&mut app, w, 24);
        assert!(text.lines().all(|l| l.chars().count() <= w as usize), "overflow at {w}:\n{text}");
        for (at, _) in text.match_indices("PR #") {
            // The card's row says `PR #42 open`; the footer `PR #42 ✓`. Anything else is a cut part.
            let whole = text[at..].starts_with("PR #42 open") || text[at..].starts_with("PR #42 ✓");
            assert!(whole, "a cut pull request at {w} columns:\n{text}");
        }
        footer_pr |= text.contains("PR #42 ✓");
        if text.contains("PR #42 ✓") {
            assert!(find_hit(&app, |h| *h == Hit::ProjectPr).is_some(), "footer PR is clickable at {w}");
        }
    }
    assert!(footer_pr, "the footer never showed the pull request");
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
        repo: true,
        tasks: Vec::new(),
        claude: None,
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

/// A full quota marks its launcher on Home with ⚠, and launching it says when it resets. A window past `warn_at`
/// shows in the agent pane's title ("⚠ 5h 92%"); when such a window resets, NOBLE says so at its reset time.
#[test]
fn quota_warnings_on_launcher_pane_and_reset() {
    use noble::app::View;
    use noble::hooks::HookRecord;
    let mut app = demo_app(160, 45);
    let claude =
        noble::config::Launcher { key: "c".into(), name: "claude".into(), command: "echo claude".into(), show: true };
    app.launchers = vec![(claude, true)];
    let now = chrono::Utc::now().timestamp();
    let state = |id: &'static str, name: &'static str, used: u8, resets_at: i64| ProviderState {
        id,
        name,
        login_hint: "login",
        presence: Presence::Ready,
        status: Status::Ok,
        usage: Some(Usage {
            windows: vec![Window { label: "5H".into(), used, resets_at: Some(resets_at) }],
            plan: None,
            note: None,
        }),
        fetched_at: Some(now),
    };
    // Full: the chip is marked. Launching it says when the quota resets (1h12m, a few seconds of slack).
    app.handle(AppEvent::Ai(Box::new(state("claude", "Claude Code", 100, now + 4350))));
    let text = render(&mut app, 160, 45);
    save("bridge-quota-full-160x45", &text);
    assert!(text.contains("Claude ⚠"), "{text}");
    // Narrow sizes: the chip is reviewed for overflow in these dumps.
    for (w, h) in [(80, 24), (40, 14)] {
        save(&format!("bridge-quota-full-{w}x{h}"), &render(&mut app, w, h));
    }
    app.toasts.clear();
    app.launch(0, Some(std::env::temp_dir()));
    let texts: Vec<&str> = app.toasts.iter().map(|t| t.text.as_str()).collect();
    assert!(texts.contains(&"Claude Code 5h quota is full · resets in 1h12m"), "{texts:?}");

    // 92%: the chip is not full any more, and the agent's pane title names the window.
    app.handle(AppEvent::Ai(Box::new(state("claude", "Claude Code", 92, now + 3600))));
    assert!(!render(&mut app, 160, 45).contains("Claude ⚠"));
    app.new_tab(std::env::temp_dir(), None, Some("agent".into()));
    let pane = app.tabs.last().expect("agent tab").focus;
    let record =
        HookRecord { event: "prompt".into(), message: None, ts: now, agent: None, session: None, subagents: 0 };
    app.apply_hook_records([(pane, record)].into());
    app.view = View::Term(app.tabs.len() - 1);
    let text = render(&mut app, 160, 45);
    save("pane-quota-warning-160x45", &text);
    assert!(text.contains("claude") && text.contains("⚠ 5h 92%"), "{text}");
    // A narrow pane: the full title drops the window when it does not fit, and the dump shows it.
    save("pane-quota-warning-60x20", &render(&mut app, 60, 20));

    // A window past `warn_at` that resets: the notice comes at its reset time (the tick checks it).
    app.toasts.clear();
    let soon = chrono::Utc::now().timestamp() + 2;
    app.handle(AppEvent::Ai(Box::new(state("codex", "Codex", 93, soon))));
    assert!(app.toasts.iter().any(|t| t.text.starts_with("Codex 5h usage at 93%")), "{:?}", app.toasts.len());
    app.toasts.clear();
    std::thread::sleep(Duration::from_millis(2100));
    app.tick();
    let texts: Vec<&str> = app.toasts.iter().map(|t| t.text.as_str()).collect();
    assert!(texts.contains(&"Codex 5h quota reset"), "{texts:?}");
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

/// With no project yet, Home says how to add one (`a`) and the welcome-free overlay line says the same.
#[test]
fn empty_home_says_how_to_add_a_project() {
    let mut app = demo_app(110, 30);
    app.projects.clear();
    app.projects_loaded = true;
    let text = render(&mut app, 110, 30);
    save("bridge-empty-110x30", &text);
    assert!(
        text.contains("No git repositories found.") && text.contains("press a to add a project or folder"),
        "{text}"
    );
    assert!(!text.contains(" A "), "no `A` hint left: {text}");
    for (w, h) in [(60, 12), (30, 8)] {
        save(&format!("bridge-empty-{w}x{h}"), &render(&mut app, w, h));
    }
}

/// Auto mode (`a`) decides on Enter: a git repository or a folder with no repository inside is
/// added as a project, a folder with a repository two levels down is scanned.
#[test]
fn auto_add_decides_between_project_and_scan() {
    use noble::app::AddMode;
    let base = std::env::temp_dir().join(format!("noble-auto-add-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (repo, group, notes) = (base.join("repo"), base.join("group"), base.join("notes"));
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir_all(group.join("inner").join(".git")).unwrap();
    std::fs::create_dir_all(&notes).unwrap();
    let mut app = demo_app(110, 30);
    let defaults = noble::projects::default_roots().len();
    app.add_by_mode(AddMode::Auto, &repo.display().to_string());
    // Even the git check runs on the helper thread now (a stalled share must not freeze the UI).
    pump_until(&mut app, "repo answer", |a| a.toasts.iter().any(|t| t.text == "added project repo"));
    assert!(
        app.toasts.iter().any(|t| t.text == "added project repo"),
        "{:?}",
        app.toasts.iter().map(|t| &t.text).collect::<Vec<_>>()
    );
    assert!(app.cfg.projects.roots.is_empty(), "a repository is not a scan root");
    // The folder walk answers on a helper thread: the toast comes once it is pumped.
    app.add_by_mode(AddMode::Auto, &group.display().to_string());
    pump_until(&mut app, "scan answer", |a| a.toasts.iter().any(|t| t.text.starts_with("scanning ")));
    assert!(app.toasts.iter().any(|t| t.text.starts_with("scanning ") && t.text.ends_with("group for projects")));
    assert_eq!(app.cfg.projects.roots.len(), defaults + 1, "the folder is a scan root");
    app.add_by_mode(AddMode::Auto, &notes.display().to_string());
    pump_until(&mut app, "notes answer", |a| a.toasts.iter().any(|t| t.text == "added project notes"));
    assert!(app.toasts.iter().any(|t| t.text == "added project notes"));
    assert!(app.projects.iter().any(|p| p.path == notes && !p.repo), "a folder with no repository is a plain project");
    // Folder mode scans even a folder that holds no repository yet.
    app.add_by_mode(AddMode::Folder, &notes.display().to_string());
    assert_eq!(app.cfg.projects.roots.len(), defaults + 2);
    // Project mode (the prompt's forced mode) adds a folder as a project even when it only holds repositories.
    app.add_by_mode(AddMode::Project, &group.display().to_string());
    assert!(app.projects.iter().any(|p| p.path == group && !p.repo));
    let _ = std::fs::remove_dir_all(&base);
}

/// A folder whose repository check does not answer in time is added as a project, not scanned.
#[test]
fn auto_add_treats_a_silent_folder_as_a_project() {
    use noble::app::AddMode;
    let base = std::env::temp_dir().join(format!("noble-auto-add-slow-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let folder = base.join("share");
    std::fs::create_dir_all(&folder).unwrap();
    let mut app = demo_app(110, 30);
    app.repo_probe = |_, _| {
        std::thread::sleep(std::time::Duration::from_secs(4));
        true
    };
    let started = std::time::Instant::now();
    app.add_by_mode(AddMode::Auto, &folder.display().to_string());
    assert!(started.elapsed() < std::time::Duration::from_secs(1), "the prompt does not wait for the check");
    pump_until(&mut app, "share answer", |a| a.toasts.iter().any(|t| t.text == "added project share"));
    assert!(
        app.toasts.iter().any(|t| t.text == "added project share"),
        "{:?}",
        app.toasts.iter().map(|t| &t.text).collect::<Vec<_>>()
    );
    assert!(app.cfg.projects.roots.is_empty(), "not scanned");
    let _ = std::fs::remove_dir_all(&base);
}

/// Cached quota numbers ("~") carry their age on the provider's name row; fresh ones do not.
#[test]
fn stale_quota_shows_its_age() {
    let mut app = demo_app(160, 45);
    let text = render(&mut app, 160, 45);
    assert!(text.contains("Codex ~1h"), "{text}");
    assert!(text.contains("~18%") && !text.contains("from 1h ago"), "{text}");
    assert!(!text.contains("Claude Code ~"), "{text}");
    for (w, h) in SIZES {
        render(&mut app, w, h);
    }
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

/// Removing a project from the ⋯ menu hides it for good (a rescan does not bring it back, a
/// pin goes with it); `A` adds any folder as a project — a plain folder too — and brings a
/// removed one back. The state stays in memory (`UiState::memory` in headless mode).
#[test]
fn remove_and_add_projects() {
    use noble::app::{Hit, Overlay};
    let base = std::env::temp_dir().join(format!("noble-add-remove-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (demo, plain) = (base.join("demo"), base.join("plain notes"));
    std::fs::create_dir_all(&demo).unwrap();
    std::fs::create_dir_all(&plain).unwrap();
    let mut app = demo_app(110, 30);
    let key = |app: &mut App, c: KeyCode| app.on_key(KeyEvent::new(c, KeyModifiers::NONE));
    let mut scanned = app.projects.clone();
    scanned.push(Project {
        name: "demo".into(),
        path: demo.clone(),
        branch: Some("main".into()),
        last_active: Some(SystemTime::now()),
        git: None,
        repo: true,
        tasks: Vec::new(),
        claude: None,
    });
    app.handle(AppEvent::Projects(scanned.clone()));
    // Pin it, then remove it with → → ⏎ and the last menu item.
    app.toggle_pin(&demo);
    app.bridge.proj_sel = 0;
    assert_eq!(app.selected_project().unwrap().path, demo, "pinned projects come first");
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Right);
    render(&mut app, 110, 30);
    key(&mut app, KeyCode::Enter);
    let text = render(&mut app, 110, 30);
    save("menu-project-remove-110x30", &text);
    let Some(Overlay::Menu(menu)) = &app.overlay else { panic!("menu: {text}") };
    let remove = menu.items.iter().position(|i| i.label == "Remove from list").expect("remove item");
    assert!(text.contains("Remove from list"), "{text}");
    let row = find_hit(&app, |h| *h == Hit::MenuItem(remove)).unwrap();
    click(&mut app, row.x + 1, row.y);
    // Removing asks first and names the project; nothing changes until Yes.
    let text = render(&mut app, 110, 30);
    assert!(text.contains("REMOVE PROJECT") && text.contains("“demo”") && text.contains("A on Home adds it back"));
    assert!(app.projects.iter().any(|p| p.path == demo), "still listed before the confirm");
    key(&mut app, KeyCode::Char('y'));
    assert!(app.overlay.is_none());
    assert!(!app.projects.iter().any(|p| p.path == demo), "gone from the list");
    assert!(!app.ui_state.is_pinned(&demo), "removing unpins");
    assert!(app.toasts.iter().any(|t| t.text.contains("removed demo from Projects")));
    // A rescan finds it again: it stays hidden.
    app.handle(AppEvent::Projects(scanned.clone()));
    assert!(!app.projects.iter().any(|p| p.path == demo), "hidden after a rescan");
    // `A` opens the add prompt in project mode; tab cycles auto, folder to scan and one project.
    key(&mut app, KeyCode::Char('A'));
    let text = render(&mut app, 110, 30);
    save("add-project-110x30", &text);
    assert!(text.contains("ADD PROJECT") && text.contains("● one project"), "{text}");
    key(&mut app, KeyCode::Tab);
    let text = render(&mut app, 110, 30);
    assert!(text.contains("ADD PROJECT OR FOLDER") && text.contains("● auto"), "{text}");
    key(&mut app, KeyCode::Tab);
    let text = render(&mut app, 110, 30);
    assert!(text.contains("ADD FOLDER TO SCAN") && text.contains("● folder to scan"), "{text}");
    let project_mode = find_hit(&app, |h| *h == Hit::PromptMode(noble::app::AddMode::Project)).expect("mode choice");
    click(&mut app, project_mode.x + 1, project_mode.y);
    let Some(Overlay::Prompt(p)) = &app.overlay else { panic!("prompt closed by the click") };
    assert_eq!(p.title, "ADD PROJECT");
    for size in [(60, 12), (30, 8)] {
        save(&format!("add-project-{}x{}", size.0, size.1), &render(&mut app, size.0, size.1));
    }
    assert_no_doubled_frame(&render(&mut app, 30, 8));
    // A plain folder (no git) is added and selected; a missing one is refused.
    for c in plain.display().to_string().chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    key(&mut app, KeyCode::Enter);
    let sel = app.selected_project().expect("added project selected");
    assert_eq!(sel.path, plain);
    assert!(!sel.repo && sel.git.is_none());
    let text = render(&mut app, 160, 45);
    save("project-plain-folder-160x45", &text);
    assert!(text.contains("plain notes") && text.contains("not a git repository"), "{text}");
    let before = app.projects.len();
    app.add_project(&base.join("gone").display().to_string());
    assert_eq!(app.projects.len(), before);
    assert!(app.toasts.iter().any(|t| t.text.starts_with("not a folder")));
    app.add_project(&plain.display().to_string());
    assert_eq!(app.projects.len(), before, "not listed twice");
    // A scan that started before the add returns without the folder: it stays listed.
    app.handle(AppEvent::Projects(scanned.clone()));
    assert!(app.projects.iter().any(|p| p.path == plain), "a stale scan keeps the hand-added folder");
    assert!(!app.projects.iter().any(|p| p.path == demo), "hidden ones stay hidden");
    // The scan thread merges added folders into every scan (simulated here).
    app.handle(AppEvent::Projects(app.ui_state.manual_projects().apply(scanned.clone())));
    assert!(app.projects.iter().any(|p| p.path == plain), "added folder survives a rescan");
    // Adding the removed project brings it back for good.
    app.add_project(&demo.display().to_string());
    app.handle(AppEvent::Projects(app.ui_state.manual_projects().apply(scanned)));
    assert!(app.projects.iter().any(|p| p.path == demo), "shown again");
    // The palette's remove acts on the selected project.
    let sel = app.selected_project().unwrap().path.clone();
    app.run(Action::RemoveProject);
    assert!(matches!(app.overlay, Some(Overlay::Confirm(_))), "the palette's remove asks first");
    key(&mut app, KeyCode::Char('y'));
    assert!(!app.projects.iter().any(|p| p.path == sel));
    let _ = std::fs::remove_dir_all(&base);
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

/// Test provider state that reports a session (login) that has expired, with cached numbers.
fn expired_state(id: &'static str, name: &'static str, hint: &'static str, used: u8) -> AppEvent {
    AppEvent::Ai(Box::new(ProviderState {
        id,
        name,
        login_hint: hint,
        presence: Presence::Ready,
        status: Status::Expired,
        usage: Some(Usage {
            windows: vec![Window { label: "5H".into(), used, resets_at: None }],
            plan: None,
            note: None,
        }),
        fetched_at: Some(chrono::Utc::now().timestamp() - 3600),
    }))
}

/// An expired login stays on Home: cached bars dimmed with "~" and a one-line hint (not hidden like "no plan").
#[test]
fn expired_login_stays_on_home_with_a_hint() {
    let mut app = demo_app(160, 45);
    app.handle(expired_state("codex", "Codex", "codex login", 18));
    let text = render(&mut app, 160, 45);
    save("bridge-expired-160x45", &text);
    assert!(text.contains("Codex ~1h"), "{text}");
    assert!(text.contains("~18%"), "{text}");
    assert!(text.contains("expired · run codex login to refresh"), "{text}");
    for (w, h) in SIZES {
        save(&format!("bridge-expired-{w}x{h}"), &render(&mut app, w, h));
    }
    // Without cached numbers the hint replaces the bars.
    app.handle(AppEvent::Ai(Box::new(ProviderState {
        id: "codex",
        name: "Codex",
        login_hint: "codex login",
        presence: Presence::Ready,
        status: Status::Expired,
        usage: None,
        fetched_at: None,
    })));
    let text = render(&mut app, 160, 45);
    assert!(text.contains("Codex") && text.contains("run codex login to refresh"), "{text}");
}

/// One-column Home (80-91 columns) has no AI panel: a compact strip carries the quota instead.
#[test]
fn narrow_home_shows_a_quota_strip() {
    let mut app = demo_app(80, 24);
    let text = render(&mut app, 80, 24);
    save("bridge-strip-80x24", &text);
    assert!(text.contains("claude 71%") && text.contains("codex ~92%"), "{text}");
    app.handle(expired_state("kilo", "Kilo", "kilo auth login", 40));
    let text = render(&mut app, 80, 24);
    assert!(text.contains("kilo expired"), "{text}");
    for (w, h) in [(91, 24), (80, 24), (60, 20), (56, 18), (40, 14), (30, 8)] {
        save(&format!("bridge-strip-{w}x{h}"), &render(&mut app, w, h));
    }
    // Wide Home has the panel, so no strip (the summary would repeat it).
    let wide = render(&mut app, 160, 45);
    assert!(!wide.contains("claude 71%"), "{wide}");
}

/// A filter kept after Enter with no match still shows how to clear it.
#[test]
fn home_filter_keeps_esc_clear_when_nothing_matches() {
    let mut app = demo_app(120, 34);
    let key = |app: &mut App, c: KeyCode| app.on_key(KeyEvent::new(c, KeyModifiers::NONE));
    key(&mut app, KeyCode::Char('/'));
    for c in "zzqx".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Enter);
    assert!(!app.bridge.filtering);
    assert_eq!(app.bridge.filter, "zzqx");
    let text = render(&mut app, 120, 34);
    save("bridge-nomatch-120x34", &text);
    assert!(text.contains("Nothing matches"), "{text}");
    assert!(text.lines().last().unwrap().contains("esc clear"), "{text}");
    key(&mut app, KeyCode::Esc);
    assert!(app.bridge.filter.is_empty());
    let text = render(&mut app, 120, 34);
    assert!(!text.lines().last().unwrap().contains("esc clear"), "{text}");
}

/// Home's "/ search" label sits on the Projects panel's top border, left of the count, and is a
/// button: a click starts the project filter, as "/" does.
#[test]
fn home_search_label_starts_the_filter() {
    use noble::app::Hit;
    let mut app = demo_app(120, 34);
    let (text, buf) = render_buf(&mut app, 120, 34);
    let lines: Vec<&str> = text.lines().collect();
    let top = lines.iter().position(|l| l.contains("╭") && l.contains("Projects")).expect("projects title");
    assert!(lines[top].contains("/ search ·"), "label on the top border: {text}");
    assert_eq!(lines.iter().filter(|l| l.contains("/ search")).count(), 1, "one label: {text}");
    let label = find_hit(&app, |h| *h == Hit::HomeSearch).expect("search label");
    assert_eq!(label.y as usize, top, "hit on the top border");
    assert_eq!(cells_text(&buf, label), "/ search");
    assert!(!app.bridge.filtering);
    click(&mut app, label.x + 2, label.y);
    assert!(app.bridge.filtering && app.bridge.filter.is_empty());
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.bridge.filtering);
}

/// `o` and the "Folder" button with no project selected say why, instead of doing nothing.
#[test]
fn open_folder_without_a_project_says_so() {
    use noble::app::Hit;
    let mut app = demo_app(110, 30);
    // Without the launchers detected on this machine: their buttons come first and, with enough of them
    // installed, leave no room on the row for "Folder".
    app.launchers.clear();
    app.handle(AppEvent::Projects(Vec::new()));
    assert!(app.selected_project().is_none());
    app.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
    assert!(toast_texts(&app).last().is_some_and(|t| t == "select a project first"), "{:?}", toast_texts(&app));
    let text = render(&mut app, 110, 30);
    let folder = find_hit(&app, |h| *h == Hit::OpenFiles).expect("folder button");
    assert!(text.contains("Folder"), "{text}");
    click(&mut app, folder.x + 1, folder.y);
    assert!(toast_texts(&app).last().is_some_and(|t| t == "select a project first"), "{:?}", toast_texts(&app));
}

/// The selected project's card shows `↻ claude · 2h ago` when the project has a Claude conversation folder and a
/// `claude` launcher is installed; a click on it is a `ClaudeContinue` hit. Without either, the line is left out.
#[test]
fn project_card_shows_the_claude_line() {
    use noble::app::Hit;
    let mut app = demo_app(160, 45);
    let path = app.selected_project().expect("a selected project").path.clone();
    let two_hours = SystemTime::now() - Duration::from_secs(2 * 3600 + 30);
    for p in &mut app.projects {
        p.claude = Some(two_hours);
    }
    let claude =
        noble::config::Launcher { key: "c".into(), name: "claude".into(), command: "claude".into(), show: true };
    app.launchers = vec![(claude.clone(), true)];

    let text = render(&mut app, 160, 45);
    save("home-claude-line-160x45", &text);
    assert!(text.contains("↻ claude · 2h ago"), "{text}");
    assert!(find_hit(&app, |h| *h == Hit::ClaudeContinue).is_some(), "the line is clickable");
    // Every size: the line is drawn whole or left out, never past the card.
    for (w, h) in SIZES {
        let text = render(&mut app, w, h);
        save(&format!("home-claude-line-{w}x{h}"), &text);
        assert!(text.lines().all(|l| l.chars().count() <= w as usize), "overflow at {w}x{h}");
    }

    // No `claude` launcher installed: no line and no hit.
    app.launchers = vec![(claude.clone(), false)];
    assert!(!render(&mut app, 160, 45).contains("↻ claude"));
    assert!(find_hit(&app, |h| *h == Hit::ClaudeContinue).is_none());
    app.launchers = Vec::new();
    assert!(!render(&mut app, 160, 45).contains("↻ claude"));
    // The launcher is there, but the project has no conversation folder.
    app.launchers = vec![(claude, true)];
    for p in &mut app.projects {
        p.claude = None;
    }
    assert!(!render(&mut app, 160, 45).contains("↻ claude"));
    // Other projects keep their own times: only the selected one is on the card.
    for p in &mut app.projects {
        p.claude = (p.path == path).then_some(two_hours);
    }
    assert!(render(&mut app, 160, 45).contains("↻ claude · 2h ago"));
}

/// Without a `claude` launcher the continue action says so instead of opening a tab.
#[test]
fn continue_claude_without_a_launcher_says_so() {
    let mut app = demo_app(160, 45);
    app.launchers = Vec::new();
    let tabs = app.tabs.len();
    app.continue_claude();
    assert_eq!(app.tabs.len(), tabs, "no tab without the launcher");
    assert!(app.toasts.iter().any(|t| t.text.contains("claude is not installed")), "a toast says why");
}

/// A tall card shows more of the history: the first 8 commits come first, more only fill rows the changed files
/// leave free, and the files keep their rows.
#[test]
fn project_card_commits_follow_the_height() {
    let commits = |n: usize| -> Vec<noble::projects::Commit> {
        (0..n)
            .map(|i| noble::projects::Commit {
                hash: format!("c{i:06}"),
                time: chrono::Utc::now().timestamp() - 3600 * (i as i64 + 1),
                author: "Mert".into(),
                subject: format!("commit number {i}"),
            })
            .collect()
    };
    let shown = |text: &str| text.lines().filter(|l| l.contains("commit number")).count();
    let mut app = demo_app(160, 45);
    // Set on the app directly: a scan result keeps the git state it already has.
    let git = app.projects[0].git.as_mut().expect("demo repo");
    git.commits = commits(noble::projects::MAX_COMMITS);
    let tall = render(&mut app, 160, 60);
    save("bridge-commits-160x60", &tall);
    let normal = render(&mut app, 160, 45);
    save("bridge-commits-160x45", &normal);
    assert!(shown(&tall) > 8 && shown(&tall) > shown(&normal), "tall {} vs {}\n{tall}", shown(&tall), shown(&normal));
    assert!(shown(&normal) >= 8, "the first 8 always fit at 160x45: {normal}");
    // The demo's changed files are still listed on the tall card.
    assert!(tall.contains("src/ui/bridge.rs") && tall.contains("tests/new.rs"), "{tall}");
    // Many changed files: the extra commits give their rows back to the files.
    let git = app.projects[0].git.as_mut().unwrap();
    git.changes = (0..30).map(|i| (" M".to_string(), format!("src/file{i:02}.rs"))).collect();
    git.dirty = 30;
    let busy = render(&mut app, 160, 60);
    save("bridge-commits-busy-160x60", &busy);
    assert!(shown(&busy) <= shown(&tall), "{busy}");
    for (w, h) in SIZES {
        render(&mut app, w, h);
    }
}

/// Claude Code's background sessions (`claude --bg`) are listed in the AI panel under the agent sessions: running
/// and failed ones show, a finished one from two days ago does not. Each shown row has a hit that attaches to it
/// (`bg_attach` gives the folder, the command and the tab title). The click itself is not sent here: it would run
/// `claude attach` on this machine.
#[test]
fn bridge_lists_background_claude_sessions() {
    use noble::app::Hit;
    use noble::bg::BgSession;
    let mut app = demo_app(160, 45);
    let now = chrono::Utc::now().timestamp();
    let dir = std::env::temp_dir();
    let session = |id: &str, name: &str, state: &str, started: i64| BgSession {
        id: id.into(),
        cwd: Some(dir.clone()),
        name: name.into(),
        state: state.into(),
        started,
    };
    app.bg_sessions = vec![
        session("run1a2b", "Fix the build", "running", now - 600),
        session("old3c4d", "Stale task", "done", now - 2 * 86_400),
        session("bad5e6f", "Broken migration", "failed", now - 3600),
    ];
    let text = render(&mut app, 160, 45);
    save("bridge-bg-160x45", &text);
    assert!(text.contains("claude bg · Fix the build") && text.contains("running"), "{text}");
    assert!(text.contains("claude bg · Broken migration") && text.contains("failed"), "{text}");
    assert!(!text.contains("Stale task"), "a finished session from two days ago is not listed: {text}");
    assert!(find_hit(&app, |h| *h == Hit::BgSession(0)).is_some(), "running row hit");
    assert!(find_hit(&app, |h| *h == Hit::BgSession(2)).is_some(), "failed row hit");
    assert!(find_hit(&app, |h| *h == Hit::BgSession(1)).is_none(), "the old one has no row");

    let (cwd, line, origin) = app.bg_attach(0).expect("attach info for the running session");
    assert_eq!(cwd, dir);
    assert_eq!(line, "claude attach run1a2b");
    assert!(origin.ends_with("· claude bg"), "{origin}");

    // The same list is no change; a changed state is (then the first list goes back for the dumps).
    let same_as_first = app.bg_sessions.clone();
    assert!(!app.set_bg_sessions(same_as_first.clone()), "the same list changes nothing");
    let mut changed = app.bg_sessions.clone();
    changed[0].state = "done".into();
    assert!(app.set_bg_sessions(changed), "a new state is a change");
    app.set_bg_sessions(same_as_first);

    for (w, h) in SIZES {
        let text = render(&mut app, w, h);
        save(&format!("bridge-bg-{w}x{h}"), &text);
    }
}
