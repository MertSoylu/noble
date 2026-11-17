//! Agent duel: the start form, the compare overlay and the end dialog at many sizes (animations off), the menus and
//! palette entries, and the whole flow against a real repository with a stand-in agent.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use noble::app::{App, DuelView, EndState, EndStep, Overlay, View};
use noble::config::{Config, Launcher};
use noble::duel::{CompareReport, FileChange, SideReport};
use noble::event::AppEvent;
use noble::keys::Action;
use noble::projects::Project;
use noble::store::{DuelRecord, DuelSide};

use super::common::*;

fn key(app: &mut App, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        key(app, KeyCode::Char(c));
    }
}

fn launcher(name: &str, command: &str) -> (Launcher, bool) {
    (Launcher { key: name[..1].into(), name: name.into(), command: command.into(), show: true }, true)
}

/// A headless app with one project on Home; `repo_marker` gives it a `.git` folder (enough for the form; the git
/// work of the flow test needs a real repository).
fn app_with_project(dir: &Path, size: (u16, u16), repo_marker: bool) -> App {
    let mut cfg = Config::default();
    cfg.general.animations = false;
    let mut app = App::headless(cfg, size);
    if repo_marker {
        std::fs::create_dir_all(dir.join(".git")).unwrap();
    }
    app.handle(AppEvent::Projects(vec![Project {
        name: "duelrepo".into(),
        path: dir.to_path_buf(),
        branch: Some("main".into()),
        last_active: None,
        git: None,
        repo: repo_marker,
        tasks: Vec::new(),
        claude: None,
    }]));
    app.launchers = vec![
        launcher("claude", "claude"),
        launcher("codex", "codex"),
        launcher("antigravity", "agy"),
        launcher("pi", "pi"),
        (Launcher { key: "e".into(), name: "opencode".into(), command: "opencode".into(), show: true }, false),
        launcher("copilot", "copilot --allow-all"),
    ];
    app
}

fn form_of(app: &App) -> &noble::app::DuelForm {
    match &app.overlay {
        Some(Overlay::Duel(DuelView::Form(f))) => f,
        _ => panic!("the duel form is not open"),
    }
}

#[test]
fn the_form_offers_only_installed_agents_with_a_known_prompt_form() {
    let dir = TempDir::new("duel-form-agents");
    let mut app = app_with_project(dir.path(), (110, 30), true);
    app.run(Action::StartDuel);
    let f = form_of(&app);
    let names: Vec<&str> = f.agents.iter().map(|a| a.name.as_str()).collect();
    // antigravity and pi have no known form, opencode is not installed: all three stay out.
    assert_eq!(names, ["claude", "codex", "copilot"]);
    assert_eq!((f.a, f.b), (0, 1), "two different agents by default");
    let text = render(&mut app, 110, 30);
    save("duel-form-110x30", &text);
    for needle in
        ["AGENT DUEL · duelrepo", "both start from main", "claude", "codex", "copilot", "task", "start", "esc"]
    {
        assert!(text.contains(needle), "{needle}: {text}");
    }
    assert!(!text.contains("antigravity") && !text.contains(" pi "), "{text}");
}

#[test]
fn the_form_needs_a_repository_and_an_agent_that_can_start_with_a_task() {
    let dir = TempDir::new("duel-form-refuse");
    // A plain folder.
    let mut app = app_with_project(dir.path(), (110, 30), false);
    app.run(Action::StartDuel);
    assert!(app.overlay.is_none());
    assert!(render(&mut app, 110, 30).contains("not a git repository"));
    // A repository, but only agents with no known way to start with a task.
    let mut app = app_with_project(dir.path(), (110, 30), true);
    app.launchers = vec![launcher("antigravity", "agy"), launcher("cursor", "cursor-agent")];
    app.run(Action::StartDuel);
    assert!(app.overlay.is_none());
    assert!(render(&mut app, 110, 30).contains("no installed agent can start with a task"));
    // No project selected.
    let mut cfg = Config::default();
    cfg.general.animations = false;
    let mut app = App::headless(cfg, (110, 30));
    app.run(Action::StartDuel);
    assert!(app.overlay.is_none());
}

#[test]
fn the_form_edits_the_task_and_checks_it_before_starting() {
    let dir = TempDir::new("duel-form-edit");
    let mut app = app_with_project(dir.path(), (110, 30), true);
    app.run(Action::StartDuel);
    // An empty task, and one that an agent would read as an option, are refused in the form.
    key(&mut app, KeyCode::Enter);
    assert!(form_of(&app).error.as_deref().is_some_and(|e| e.contains("task")), "{:?}", form_of(&app).error);
    assert!(!form_of(&app).busy);
    typed(&mut app, "--help");
    key(&mut app, KeyCode::Enter);
    assert!(form_of(&app).error.as_deref().is_some_and(|e| e.contains("option")));
    assert!(render(&mut app, 110, 30).contains("option"));
    // Typing clears the error; Ctrl+J and Shift+Enter add lines; a paste keeps its line breaks.
    key(&mut app, KeyCode::Backspace);
    assert!(form_of(&app).error.is_none());
    for _ in 0..5 {
        key(&mut app, KeyCode::Backspace);
    }
    typed(&mut app, "fix the bug");
    app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    typed(&mut app, "then test it");
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
    app.handle(AppEvent::Input(Event::Paste("third\nfourth".into())));
    assert_eq!(form_of(&app).task, "fix the bug\nthen test it\nthird\nfourth");
    let text = render(&mut app, 110, 30);
    save("duel-form-text-110x30", &text);
    for line in ["fix the bug", "then test it", "third", "fourth"] {
        assert!(text.contains(line), "{line}: {text}");
    }
    // Up and down move between the lines; Tab goes to the agent rows, where ←→ pick the agent.
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Up);
    typed(&mut app, "X");
    assert_eq!(form_of(&app).task, "fix the bug\nthen Xtest it\nthird\nfourth");
    key(&mut app, KeyCode::BackTab);
    assert_eq!(form_of(&app).field, noble::app::DuelField::B);
    key(&mut app, KeyCode::Right);
    assert_eq!(form_of(&app).b, 2);
    key(&mut app, KeyCode::Right);
    assert_eq!(form_of(&app).b, 0, "the list wraps");
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Left);
    assert_eq!(form_of(&app).a, 2);
    // Esc closes it without starting anything.
    key(&mut app, KeyCode::Esc);
    assert!(app.overlay.is_none() && app.duels().is_empty());
}

#[test]
fn the_form_fits_every_size() {
    let dir = TempDir::new("duel-form-sizes");
    let mut app = app_with_project(dir.path(), (110, 30), true);
    app.run(Action::StartDuel);
    typed(&mut app, "a task that is long enough to wrap around the narrow windows more than once, and then some");
    app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    typed(&mut app, "second line");
    for (w, h) in SIZES.into_iter().chain([(40, 12), (30, 7), (24, 6), (20, 5)]) {
        let text = render(&mut app, w, h);
        save(&format!("duel-form-{w}x{h}"), &text);
        assert_eq!(text.lines().count(), usize::from(h));
        assert!(text.lines().all(|l| l.chars().count() <= usize::from(w)), "{w}x{h}");
        if w >= 56 && h >= 16 {
            assert!(text.contains("AGENT DUEL") && text.contains("second line"), "{w}x{h}: {text}");
        }
    }
    // An error and the busy note replace the footer in a short box instead of overflowing it.
    key(&mut app, KeyCode::End);
    for _ in 0..200 {
        key(&mut app, KeyCode::Backspace);
    }
    key(&mut app, KeyCode::Enter);
    let text = render(&mut app, 30, 7);
    assert!(text.contains("type the task"), "{text}");
}

fn report_side(agent: &str, branch: &str, wt: &Path, (root, files): (&str, usize), commits: u32) -> SideReport {
    let files: Vec<FileChange> = (0..files)
        .map(|i| FileChange {
            path: format!("{root}/module{}/file_number_{i}.rs", i % 3),
            added: Some(10 + i as u32),
            removed: Some(i as u32 % 4),
            committed: i % 2 == 0,
            uncommitted: i % 5 == 0,
            untracked: i % 7 == 6,
        })
        .collect();
    SideReport {
        agent: agent.into(),
        branch: branch.into(),
        worktree: wt.to_path_buf(),
        worktree_exists: true,
        commits,
        file_count: files.len(),
        added: files.iter().filter_map(|f| f.added).sum(),
        removed: files.iter().filter_map(|f| f.removed).sum(),
        uncommitted: files.iter().filter(|f| f.uncommitted).count(),
        files,
        error: None,
    }
}

fn record(project: &Path, data: &Path) -> DuelRecord {
    let side = |agent: &str, branch: &str, wt: &str| DuelSide {
        agent: agent.into(),
        launcher: agent.into(),
        branch: branch.into(),
        worktree: data.join("duels").join("duelrepo-k3x9qa").join(wt).display().to_string(),
        extra: Default::default(),
    };
    DuelRecord {
        id: "k3x9qa".into(),
        project: project.display().to_string(),
        name: "duelrepo".into(),
        base: "1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b".into(),
        base_branch: Some("main".into()),
        prompt: "Make the parser faster\nand keep the tests green".into(),
        created: 1_700_000_000,
        a: side("claude", "noble/duel-k3x9qa-a", "a"),
        b: side("codex", "noble/duel-k3x9qa-b", "b"),
        extra: Default::default(),
    }
}

#[test]
fn the_compare_overlay_shows_both_sides_and_one_in_a_narrow_window() {
    let dir = TempDir::new("duel-compare");
    let mut app = app_with_project(dir.path(), (120, 40), true);
    let rec = record(dir.path(), &app.paths.data.clone());
    let report = CompareReport {
        id: rec.id.clone(),
        a: report_side("claude", &rec.a.branch, Path::new(&rec.a.worktree), ("src", 40), 3),
        b: report_side("codex", &rec.b.branch, Path::new(&rec.b.worktree), ("lib", 2), 0),
    };
    app.ui_state.modify(|d| d.duels.push(rec.clone()));
    app.run(Action::CompareDuel);
    // While git is asked the overlay says so; the answer is what the background thread would send.
    let Some(Overlay::Duel(DuelView::Compare(_))) = &app.overlay else { panic!("compare overlay") };
    assert!(render(&mut app, 120, 40).contains("reading the worktree"));
    app.handle(AppEvent::Duel(noble::app::DuelEvent::Compared(rec.id.clone(), Box::new(report.clone()))));
    let wide = render(&mut app, 120, 40);
    save("duel-compare-120x40", &wide);
    for needle in [
        "COMPARE DUEL · duelrepo · k3x9qa",
        "base 1a2b3c4 on main",
        "task  Make the parser faster",
        "A · claude",
        "B · codex",
        "noble/duel-k3x9qa-a",
        "40 files",
        "3 commits",
        "✓ committed",
        "file_number_0.rs",
        "more (↑↓ scrolls)",
        "git -C",
        "merge noble/duel-k3x9qa-a",
        "copy A commands",
        "NOBLE never runs these",
    ] {
        assert!(wide.contains(needle), "{needle}\n{wide}");
    }
    assert!(wide.contains("2 files") && wide.contains("0 commits"), "the other side too\n{wide}");
    // → focuses side B: its commands are the ones shown; scrolling moves side A's list only when A is focused.
    key(&mut app, KeyCode::Right);
    assert!(render(&mut app, 120, 40).contains("merge noble/duel-k3x9qa-b"));
    key(&mut app, KeyCode::Left);
    for _ in 0..12 {
        key(&mut app, KeyCode::Down);
    }
    let scrolled = render(&mut app, 120, 40);
    assert!(
        scrolled.contains("src/module0/file_number_12.rs") && !scrolled.contains("src/module0/file_number_0.rs"),
        "{scrolled}"
    );
    // A narrow window shows the focused side alone.
    for (w, h) in [(80, 28), (56, 18), (40, 14), (30, 8), (20, 5)] {
        let text = render(&mut app, w, h);
        save(&format!("duel-compare-{w}x{h}"), &text);
        assert!(text.lines().all(|l| l.chars().count() <= usize::from(w)), "{w}x{h}");
        if (40..62).contains(&w) {
            assert!(text.contains("A · claude") && !text.contains("B · codex"), "{w}x{h}: one side\n{text}");
        }
    }
    key(&mut app, KeyCode::Tab);
    assert!(render(&mut app, 56, 18).contains("B · codex"));
    // A side whose worktree folder is gone, and one git could not read, say so.
    let mut broken = report.clone();
    broken.a.worktree_exists = false;
    broken.b.error = Some("not a valid object name".into());
    app.handle(AppEvent::Duel(noble::app::DuelEvent::Compared(rec.id.clone(), Box::new(broken))));
    key(&mut app, KeyCode::Left);
    let text = render(&mut app, 120, 40);
    assert!(text.contains("worktree folder is gone") && text.contains("not a valid object name"), "{text}");
    // An answer for another duel is ignored; esc closes.
    app.handle(AppEvent::Duel(noble::app::DuelEvent::Compared("zzzzzz".into(), Box::new(report))));
    key(&mut app, KeyCode::Esc);
    assert!(app.overlay.is_none());
}

#[test]
fn the_end_dialog_names_what_happens_and_keeps_the_branches_by_default() {
    let dir = TempDir::new("duel-end");
    let mut app = app_with_project(dir.path(), (110, 34), true);
    let rec = record(dir.path(), &app.paths.data.clone());
    app.ui_state.modify(|d| d.duels.push(rec.clone()));
    app.run(Action::EndDuel);
    let text = render(&mut app, 110, 34);
    save("duel-end-110x34", &text);
    for needle in [
        "END DUEL · k3x9qa",
        "Closes this duel's panes (both agents stop)",
        "worktrees of duelrepo",
        "branches noble/duel-k3x9qa-a and noble/duel-k3x9qa-b are kept,",
        "so no work is lost",
        "● keep both branches",
        "○ also delete noble/duel-k3x9qa-a",
        "○ also delete both branches",
        "remove worktrees",
    ] {
        assert!(text.contains(needle), "{needle}\n{text}");
    }
    // Picking a branch to delete is its own step: nothing is chosen until a key or a click says so.
    key(&mut app, KeyCode::Char('2'));
    let text = render(&mut app, 110, 34);
    assert!(text.contains("● also delete noble/duel-k3x9qa-a") && text.contains("○ keep both branches"), "{text}");
    key(&mut app, KeyCode::Down);
    assert!(render(&mut app, 110, 34).contains("● also delete noble/duel-k3x9qa-b"));
    for (w, h) in [(80, 24), (56, 18), (40, 14), (30, 8), (20, 5)] {
        let text = render(&mut app, w, h);
        save(&format!("duel-end-{w}x{h}"), &text);
        assert!(text.lines().all(|l| l.chars().count() <= usize::from(w)), "{w}x{h}");
    }
    // An answer that arrives while nothing waits for it changes nothing.
    app.handle(AppEvent::Duel(noble::app::DuelEvent::Inspected(rec.id.clone(), Ok((3, 1)))));
    assert!(render(&mut app, 110, 34).contains("remove worktrees"), "still asking");
    key(&mut app, KeyCode::Esc);
    assert!(app.overlay.is_none() && app.duels().len() == 1, "esc cancels, nothing was removed");
    // Uncommitted changes: the text says what would be lost, and only the explicit force key goes on.
    let dirty = EndState {
        rec: rec.clone(),
        delete: noble::duel::DeleteBranches::None,
        step: EndStep::Dirty(3, 1),
        error: None,
    };
    app.overlay = Some(Overlay::Duel(DuelView::End(Box::new(dirty))));
    let text = render(&mut app, 110, 34);
    save("duel-end-dirty-fake-110x34", &text);
    for needle in
        ["Uncommitted changes: A has 3 files, B has 1 file", "discards them", "Nothing was removed yet", "force remove"]
    {
        assert!(text.contains(needle), "{needle}\n{text}");
    }
    assert!(!text.contains("keep both branches"), "no branch choice at this step");
    for code in [KeyCode::Enter, KeyCode::Char('y')] {
        key(&mut app, code);
        assert!(matches!(&app.overlay, Some(Overlay::Duel(DuelView::End(_)))), "{code:?} does not force");
    }
    for (w, h) in [(56, 18), (30, 8), (20, 5)] {
        let text = render(&mut app, w, h);
        assert!(text.lines().all(|l| l.chars().count() <= usize::from(w)), "{w}x{h}");
    }
    key(&mut app, KeyCode::Esc);
    assert!(app.overlay.is_none() && app.duels().len() == 1);
}

#[test]
fn menus_and_the_palette_offer_the_duel_where_it_applies() {
    let dir = TempDir::new("duel-menus");
    let mut app = app_with_project(dir.path(), (110, 30), true);
    app.open_project_menu(dir.path().to_path_buf(), 10, 5);
    let text = render(&mut app, 110, 30);
    assert!(text.contains("Start agent duel…"), "{text}");
    app.overlay = None;
    // The palette lists Start (a repository is selected), but not Compare or End (no duel yet).
    app.run(Action::Palette);
    let Some(Overlay::Palette(st)) = &app.overlay else { panic!("palette") };
    let titles: Vec<&str> = st.all.iter().map(|i| i.title.as_str()).collect();
    assert!(titles.contains(&"Start Agent Duel…"), "{titles:?}");
    assert!(!titles.contains(&"Compare Agent Duel") && !titles.contains(&"End Agent Duel…"));
    app.overlay = None;
    // With a duel recorded for the project, Home offers both.
    let rec = record(dir.path(), &app.paths.data.clone());
    app.ui_state.modify(|d| d.duels.push(rec));
    app.run(Action::Palette);
    let Some(Overlay::Palette(st)) = &app.overlay else { panic!("palette") };
    let titles: Vec<&str> = st.all.iter().map(|i| i.title.as_str()).collect();
    assert!(titles.contains(&"Compare Agent Duel") && titles.contains(&"End Agent Duel…"), "{titles:?}");
    app.overlay = None;
    // A folder that is not a repository gets no menu row.
    let plain = TempDir::new("duel-menus-plain");
    let mut app = app_with_project(plain.path(), (110, 30), false);
    app.open_project_menu(plain.path().to_path_buf(), 10, 5);
    assert!(!render(&mut app, 110, 30).contains("Start agent duel"));
}

// ─── The whole flow ─────────────────────────────────────────────────────────────────────────────

/// Runs the real git (without the user's global configuration); `None` and a note when git is not installed.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A stand-in `claude` or `codex` that writes its first argument (the task) to `<dir>/<name>.args` instead of
/// starting an agent, so the test never runs a real one. The launcher command is its path. Unix: a shell script.
/// Windows: a `.cmd` file, only when the path has no space (a launcher command is split at spaces).
fn stand_in(dir: &Path, name: &str) -> Option<String> {
    let out = dir.join(format!("{name}.args"));
    if cfg!(windows) {
        let path = dir.join(format!("{name}.cmd"));
        let text = format!("@echo off\r\n>\"{}\" echo %~1\r\n", out.display());
        std::fs::write(&path, text).ok()?;
        let command = path.display().to_string();
        (!command.contains(' ')).then_some(command)
    } else {
        // Unix: any POSIX shell runs it; `printf '%s'` writes the argument exactly.
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\nprintf '%s' \"$1\" > '{}'\n", out.display())).ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).ok()?;
        }
        Some(path.display().to_string())
    }
}

#[test]
fn a_duel_starts_two_agents_in_two_worktrees_and_ends_cleanly() {
    if noble::util::which("git").is_none() {
        eprintln!("skipped: git is not installed");
        return;
    }
    let tmp = TempDir::new("duel-flow");
    let (project, bin) = (tmp.path().join("project"), tmp.path().join("bin"));
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    git(&project, &["init", "-q"]);
    git(&project, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    git(&project, &["config", "user.email", "duel@example.com"]);
    git(&project, &["config", "user.name", "Duel Test"]);
    // Same line endings for NOBLE's git (system config: autocrlf on Windows) and this test's (no system config).
    git(&project, &["config", "core.autocrlf", "false"]);
    std::fs::write(project.join("a.txt"), "one\n").unwrap();
    git(&project, &["add", "-A"]);
    git(&project, &["commit", "-q", "-m", "first"]);

    let mut app = app_with_project(&project, (150, 40), false);
    // The project is a real repository now; the stand-ins take the place of the agents (never a real one).
    app.projects[0].repo = true;
    let (Some(claude), Some(codex)) = (stand_in(&bin, "claude"), stand_in(&bin, "codex")) else {
        // Windows with a space in the temp path: a launcher command is split at spaces, so no stand-in can be named.
        eprintln!("skipped: no usable stand-in agent path");
        return;
    };
    app.launchers = vec![launcher("claude", &claude), launcher("codex", &codex)];

    // Start: the task has quotes, a dollar sign and a line break (the Unix shells and PowerShell pass them as text).
    app.run(Action::StartDuel);
    // (cmd.exe cannot quote any of that, and says so in the form: its task is plain.)
    let task = if cfg!(windows) { "add a test for the parser" } else { "fix \"it\"; echo $HOME 'x' `id`\nthen stop" };
    let (first, second) = task.split_once('\n').unwrap_or((task, ""));
    typed(&mut app, first);
    if !second.is_empty() {
        app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
        typed(&mut app, second);
    }
    key(&mut app, KeyCode::Enter);
    assert!(form_of(&app).busy, "git is asked in the background");
    pump_until(&mut app, "the duel's tab", |app| !app.tabs.is_empty());
    assert!(app.overlay.is_none(), "the form closed");
    assert_eq!(app.duels().len(), 1);
    let rec = app.duels()[0].clone();
    assert_eq!((rec.a.agent.as_str(), rec.b.agent.as_str()), ("claude", "codex"));
    assert_eq!(rec.prompt, task);
    let task = task.to_string();

    // One tab, two panes side by side, each in its own worktree, named after the duel.
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.view, View::Term(0));
    assert_eq!(app.tab_title(0), format!("⚔ duel {}", rec.id));
    let panes = app.tabs[0].panes();
    assert_eq!(panes.len(), 2);
    let cwds: Vec<PathBuf> = panes.iter().map(|id| app.panes[id].cwd()).collect();
    for (cwd, side) in cwds.iter().zip([&rec.a, &rec.b]) {
        assert!(noble::duel::is_inside(cwd, Path::new(&side.worktree)), "{cwd:?} vs {}", side.worktree);
        assert!(Path::new(&side.worktree).join("a.txt").is_file());
    }
    assert_eq!(
        app.panes[&panes[0]].launcher.as_deref(),
        Some(claude.as_str()),
        "a restored pane runs the plain launcher"
    );
    let wide = render(&mut app, 150, 40);
    save("duel-tab-150x40", &wide);
    assert!(wide.contains("⚔ duel"), "{wide}");
    // The tab is a duel tab: its menu offers Compare and End, and Compare finds it by the pane's folder.
    assert_eq!(app.duel_of_tab(0).map(|d| d.id.clone()), Some(rec.id.clone()));
    // The state survives a restart: the record is in state.json's data.
    assert_eq!(app.ui_state.data.duels, vec![rec.clone()]);

    // Unix: the stand-ins received the task exactly as typed, whatever shell NOBLE runs.
    if cfg!(unix) {
        let wait = |name: &str| {
            let file = bin.join(format!("{name}.args"));
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            while !file.exists() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            std::fs::read_to_string(&file).unwrap_or_default()
        };
        assert_eq!(wait("claude"), task, "claude got the task as one argument");
        assert_eq!(wait("codex"), task, "codex got the task as one argument");
    }

    // Compare: nothing changed yet; then side A gets work and a refresh shows it.
    app.run(Action::CompareDuel);
    pump_until(
        &mut app,
        "the compare answer",
        |app| matches!(&app.overlay, Some(Overlay::Duel(DuelView::Compare(st))) if st.report.is_some()),
    );
    let text = render(&mut app, 150, 40);
    assert!(text.contains("0 files") && text.contains("no changes yet"), "{text}");
    std::fs::write(Path::new(&rec.a.worktree).join("made_by_a.txt"), "x\ny\n").unwrap();
    key(&mut app, KeyCode::Char('r'));
    pump_until(
        &mut app,
        "the refreshed answer",
        |app| matches!(&app.overlay, Some(Overlay::Duel(DuelView::Compare(st))) if st.report.as_ref().is_some_and(|r| r.a.file_count == 1)),
    );
    let text = render(&mut app, 150, 40);
    save("duel-compare-flow-150x40", &text);
    assert!(text.contains("made_by_a.txt") && text.contains("1 file"), "{text}");
    key(&mut app, KeyCode::Esc);

    // End: uncommitted work in A asks first; nothing is removed until the force key.
    app.run(Action::EndDuel);
    key(&mut app, KeyCode::Char('y'));
    pump_until(
        &mut app,
        "the dirty answer",
        |app| matches!(&app.overlay, Some(Overlay::Duel(DuelView::End(st))) if matches!(st.step, noble::app::EndStep::Dirty(..))),
    );
    let text = render(&mut app, 110, 34);
    save("duel-end-dirty-110x34", &text);
    assert!(text.contains("Uncommitted changes: A has 1 file") && text.contains("force remove"), "{text}");
    assert_eq!(app.tabs.len(), 1, "the agents still run");
    assert!(Path::new(&rec.a.worktree).join("made_by_a.txt").is_file());
    key(&mut app, KeyCode::Enter);
    assert!(matches!(&app.overlay, Some(Overlay::Duel(DuelView::End(_)))), "Enter does not force");
    key(&mut app, KeyCode::Char('f'));
    pump_until(&mut app, "the duel to end", |app| app.duels().is_empty());
    assert!(app.overlay.is_none() && app.tabs.is_empty(), "the panes closed");
    assert!(!Path::new(&rec.a.worktree).exists() && !Path::new(&rec.b.worktree).exists());
    let left = git(&project, &["branch", "--list", "--format=%(refname:short)"]);
    assert!(left.contains(&rec.a.branch) && left.contains(&rec.b.branch), "the branches are kept: {left}");
    assert_eq!(git(&project, &["status", "--porcelain"]), "", "the project itself was never touched");
}

#[test]
fn the_overlays_answer_clicks() {
    use noble::app::{DuelHit, Hit};
    use noble::duel::{DeleteBranches, Side};
    let dir = TempDir::new("duel-clicks");
    let mut app = app_with_project(dir.path(), (110, 34), true);
    let at = |app: &App, what: DuelHit| find_hit(app, |h| *h == Hit::Duel(what)).unwrap_or_else(|| panic!("{what:?}"));
    // The form: a chip picks the agent of its side, the start chip checks the form (no task: it says so).
    app.run(Action::StartDuel);
    render(&mut app, 110, 34);
    let chip = at(&app, DuelHit::Pick(Side::B, 2));
    click(&mut app, chip.x + 1, chip.y);
    assert_eq!(form_of(&app).b, 2);
    assert_eq!(form_of(&app).field, noble::app::DuelField::B, "the row of the chip is focused");
    render(&mut app, 110, 34);
    let start = at(&app, DuelHit::Start);
    click(&mut app, start.x + 1, start.y);
    assert!(form_of(&app).error.as_deref().is_some_and(|e| e.contains("task")));
    render(&mut app, 110, 34);
    let task = at(&app, DuelHit::Field(noble::app::DuelField::Task));
    click(&mut app, task.x + 4, task.y);
    assert_eq!(form_of(&app).field, noble::app::DuelField::Task);
    // A click outside the box cancels, as it does for every overlay.
    click(&mut app, 0, 33);
    assert!(app.overlay.is_none());

    // The compare overlay: a side header focuses that side.
    let rec = record(dir.path(), &app.paths.data.clone());
    app.ui_state.modify(|d| d.duels.push(rec.clone()));
    app.run(Action::CompareDuel);
    let report = CompareReport {
        id: rec.id.clone(),
        a: report_side("claude", &rec.a.branch, Path::new(&rec.a.worktree), ("src", 3), 1),
        b: report_side("codex", &rec.b.branch, Path::new(&rec.b.worktree), ("lib", 2), 0),
    };
    app.handle(AppEvent::Duel(noble::app::DuelEvent::Compared(rec.id.clone(), Box::new(report))));
    render(&mut app, 110, 34);
    let head = at(&app, DuelHit::Side(Side::B));
    click(&mut app, head.x + 1, head.y);
    assert!(render(&mut app, 110, 34).contains("merge noble/duel-k3x9qa-b"), "side B's commands");
    app.overlay = None;

    // The end dialog: a row picks the branch choice; the chip starts the check (which needs real worktrees, so it
    // is not clicked here).
    app.run(Action::EndDuel);
    render(&mut app, 110, 34);
    let row = at(&app, DuelHit::Delete(DeleteBranches::Both));
    click(&mut app, row.x + 3, row.y);
    assert!(render(&mut app, 110, 34).contains("● also delete both branches"));
    assert!(find_hit(&app, |h| *h == Hit::Duel(DuelHit::Confirm)).is_some());
}
