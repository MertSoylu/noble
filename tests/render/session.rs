//! Session save and restore across windows, workspaces and restore failures.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use noble::app::App;
use noble::config::Config;
use noble::keys::Action;
use noble::store::{SavedTab, Workspace};
use noble::term::layout::SavedNode;
use std::path::PathBuf;
use std::time::Duration;

use super::common::*;

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

/// A crash that also left its lock behind (the window that held it is gone): the next launch restores
/// the tabs, and the lock is released after the restore.
#[test]
fn a_crash_with_its_lock_left_behind_restores() {
    let data = session_dir("crash-lock");
    let dead = "4000000000-1-0";
    let json = serde_json::json!({
        "saved_at": 0,
        "running": [dead],
        "tabs": [{ "instance": dead, "name": null, "origin": "crash-lock",
                   "layout": { "type": "leaf", "cwd": data.display().to_string() }, "focus": 0 }],
    });
    std::fs::write(data.join("session.json"), json.to_string()).unwrap();
    std::fs::write(data.join("session.json.lock"), "4000000000-1").unwrap();
    let mut app = session_app(&data, false, session_cfg());
    app.restore_last_session();
    assert_eq!(tab_origins(&app), ["crash-lock"]);
    assert!(!data.join("session.json.lock").exists(), "the lock is released after the restore");
    app.shutdown();
    assert_eq!(session_origins(&data, false), ["crash-lock"]);
    let _ = std::fs::remove_dir_all(&data);
}

/// A lock file left old and unreadable (a crash while it was being written, or an older NOBLE) must not
/// hold up the restore or the save on exit: both return at once.
#[test]
fn a_stale_lock_does_not_block_restore_or_exit() {
    let data = session_dir("stale-lock");
    write_session(&data.join("session.json"), &one_tab("kept", leaf(&data)));
    let lock = data.join("session.json.lock");
    std::fs::write(&lock, "").unwrap();
    let old = std::time::SystemTime::now() - Duration::from_secs(3600);
    std::fs::File::options().write(true).open(&lock).unwrap().set_modified(old).unwrap();
    let started = std::time::Instant::now();
    let mut app = session_app(&data, false, session_cfg());
    app.restore_last_session();
    assert_eq!(tab_origins(&app), ["kept"]);
    app.shutdown();
    let took = started.elapsed();
    assert!(took < Duration::from_secs(5), "a stale lock held the startup and exit for {took:?}");
    assert!(!lock.exists(), "the lock is released");
    assert_eq!(session_origins(&data, false), ["kept"]);
    let _ = std::fs::remove_dir_all(&data);
}

/// A session file damaged before startup (cut short, binary, or not a session at all) starts an empty window
/// without a panic. The damaged bytes are kept as `session.json.corrupt`, and the window keeps working: its tabs
/// are saved to a fresh file. An empty file is no damage: it keeps no copy.
#[test]
fn a_damaged_session_file_starts_an_empty_window() {
    let bodies: [&[u8]; 4] =
        [b"", b"{\"saved_at\": 0, \"tabs\": [{\"origin\": \"x\", \"lay", b"\xff\xfe\x00\x01", b"[1, 2]"];
    for (i, body) in bodies.into_iter().enumerate() {
        let data = session_dir(&format!("damaged-{i}"));
        let file = data.join("session.json");
        std::fs::write(&file, body).unwrap();
        let mut app = session_app(&data, false, session_cfg());
        app.restore_last_session();
        assert!(app.tabs.is_empty(), "case {i}: restored {:?}", tab_origins(&app));
        assert_eq!(app.restored_tabs, 0, "case {i}");
        let copy = data.join("session.json.corrupt");
        if body.is_empty() {
            assert!(!copy.exists(), "case {i}: an empty file is not kept");
        } else {
            assert_eq!(std::fs::read(copy).unwrap(), body, "case {i}");
        }
        app.new_tab(data.clone(), None, Some("after-damage".into()));
        app.shutdown();
        assert_eq!(session_origins(&data, false), ["after-damage"], "case {i}");
        let _ = std::fs::remove_dir_all(&data);
    }
}

/// Saved tabs that cannot start at all (here the shell is missing, so no pane spawns) are not replaced by the
/// empty set of tabs that opened: they stay in the session file and the next start restores them.
#[test]
fn tabs_that_cannot_start_are_kept_for_the_next_start() {
    let data = session_dir("unstartable");
    write_session(&data.join("session.json"), &one_tab("kept-for-later", leaf(&data)));
    let mut app = session_app(&data, false, session_cfg());
    app.shell = noble::term::pane::ShellSpec::new("/no/such/noble-test-shell", Vec::new());
    app.restore_last_session();
    assert_eq!(app.restored_tabs, 0);
    assert!(app.tabs.is_empty());
    assert_eq!(session_origins(&data, false), ["kept-for-later"], "the saved tab is still in the file");
    app.shutdown();
    assert_eq!(session_origins(&data, false), ["kept-for-later"], "and still there after a clean exit");

    let mut next = session_app(&data, false, session_cfg());
    next.restore_last_session();
    assert_eq!(next.restored_tabs, 1, "the next start restores it");
    assert_eq!(tab_origins(&next), ["kept-for-later"]);
    next.shutdown();
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
    // A launcher only runs when its first word is a program on PATH. Linux has `/bin/echo`;
    // on Windows `echo` is a shell builtin (only Git's usr\bin puts an echo.exe on PATH), so cmd runs it.
    let command = if cfg!(windows) { "cmd /c echo relaunch-marker" } else { "echo relaunch-marker" };
    cfg.launchers =
        vec![noble::config::Launcher { key: "x".into(), name: "Agent".into(), command: command.into(), show: true }];
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
    assert_eq!(text.matches(command).count(), 1, "launcher stored once, on its pane: {text}");

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

/// The session is saved a couple of seconds after the tabs change, without quitting (a crash or a
/// kill keeps it), and an unchanged session is not written again.
#[test]
fn session_is_saved_shortly_after_tabs_change() {
    let data = session_dir("autosave");
    let mut app = session_app(&data, false, session_cfg());
    app.session_autosave = true;
    assert!(app.session_save_due().is_none(), "nothing changed: no wake-up needed");
    app.new_tab(data.clone(), None, Some("autosaved".into()));
    std::thread::sleep(Duration::from_millis(1100));
    app.tick();
    assert!(session_origins(&data, false).is_empty(), "debounced: not saved at once");
    assert!(app.session_save_due().is_some(), "a save is waiting");
    std::thread::sleep(Duration::from_millis(2100));
    app.tick();
    assert_eq!(session_origins(&data, false), ["autosaved"]);
    assert!(app.session_save_due().is_none(), "saved: no wake-up needed");

    // Unchanged tabs are not written again.
    let file = data.join(session_file(false));
    let first = std::fs::metadata(&file).unwrap().modified().unwrap();
    std::thread::sleep(Duration::from_millis(1100));
    app.tick();
    assert_eq!(std::fs::metadata(&file).unwrap().modified().unwrap(), first);

    // Closing the last tab is a change too.
    app.remove_tab(0);
    std::thread::sleep(Duration::from_millis(1100));
    app.tick();
    std::thread::sleep(Duration::from_millis(2100));
    app.tick();
    assert!(session_origins(&data, false).is_empty());
    let _ = std::fs::remove_dir_all(&data);
}

/// Workspaces can be deleted from the palette (after a confirmation), and saving says when it replaced
/// a workspace of the same name or dropped the oldest one at the limit.
#[test]
fn workspaces_delete_replace_and_limit_are_announced() {
    use noble::app::{Overlay, PaletteCmd, PromptPurpose};
    let mut app = demo_app(110, 30);
    app.workspaces.list.clear();
    app.new_tab(std::env::temp_dir(), None, Some("one".into()));
    let save_as = |app: &mut App, name: &str| {
        let p = noble::app::Prompt::new(String::new(), name.into(), PromptPurpose::SaveWorkspace);
        app.submit_prompt(p);
        // A name that is taken asks first: say yes.
        if matches!(app.overlay, Some(Overlay::Confirm(_))) {
            app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        }
        toast_texts(app).last().cloned().unwrap_or_default()
    };
    assert!(save_as(&mut app, "alpha").contains("saved"));
    assert!(save_as(&mut app, "ALPHA").contains("replaced"), "same name, any case");
    assert_eq!(app.workspaces.list.len(), 1);
    for i in 0..19 {
        save_as(&mut app, &format!("w{i}"));
    }
    assert_eq!(app.workspaces.list.len(), 20);
    let msg = save_as(&mut app, "overflow");
    assert!(msg.contains("dropped 'ALPHA'"), "{msg}");
    assert_eq!(app.workspaces.list.len(), 20);

    // Palette: a delete entry per workspace, guarded by the confirm dialog.
    app.run(Action::Palette);
    let Some(Overlay::Palette(p)) = &app.overlay else { panic!("no palette") };
    let item = p.all.iter().find(|i| i.title == "Delete Workspace: overflow").expect("delete entry");
    assert_eq!(item.cmd, PaletteCmd::DeleteWorkspace("overflow".into()));
    app.overlay = None;
    app.run_palette(PaletteCmd::DeleteWorkspace("overflow".into()));
    assert!(matches!(app.overlay, Some(Overlay::Confirm(_))));
    assert!(render(&mut app, 110, 30).contains("DELETE WORKSPACE"));
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.workspaces.list.len(), 20, "esc keeps it");
    app.run_palette(PaletteCmd::DeleteWorkspace("overflow".into()));
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(app.overlay, Some(Overlay::Confirm(_))), "enter must not confirm");
    app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.workspaces.list.iter().all(|w| w.name != "overflow"));
    assert!(toast_texts(&app).iter().any(|t| t.contains("deleted")), "{:?}", toast_texts(&app));
}

/// Saving a workspace: a blank name keeps the prompt open and says why; a taken name asks before it is
/// replaced, and No leaves the saved workspace as it was.
#[test]
fn workspace_names_are_checked_before_saving() {
    use noble::app::{Overlay, PromptPurpose};
    let mut app = demo_app(110, 30);
    app.workspaces.list.clear();
    app.new_tab(std::env::temp_dir(), None, Some("one".into()));
    let prompt = |name: &str| noble::app::Prompt::new("SAVE WORKSPACE AS", name.into(), PromptPurpose::SaveWorkspace);
    app.submit_prompt(prompt("   "));
    let Some(Overlay::Prompt(p)) = &app.overlay else { panic!("the prompt closed on a blank name") };
    assert!(p.error.is_some());
    assert!(app.workspaces.list.is_empty());
    let text = render(&mut app, 110, 30);
    assert!(text.contains("type a name first"), "{text}");
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    app.submit_prompt(prompt("alpha"));
    assert_eq!(app.workspaces.list.len(), 1);
    app.submit_prompt(prompt("ALPHA"));
    let text = render(&mut app, 110, 30);
    assert!(text.contains("REPLACE WORKSPACE") && text.contains("“ALPHA” exists — replace it?"), "{text}");
    assert_eq!(app.workspaces.list[0].name, "alpha", "nothing is replaced before the confirm");
    app.on_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
    assert!(app.overlay.is_none());
    assert_eq!(app.workspaces.list[0].name, "alpha", "No keeps the old workspace");
    app.submit_prompt(prompt("ALPHA"));
    app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert_eq!(app.workspaces.list.len(), 1);
    assert!(toast_texts(&app).last().is_some_and(|t| t.contains("replaced")), "{:?}", toast_texts(&app));
}
