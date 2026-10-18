//! Tab strip, tab menus and tab actions.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use noble::app::{App, View};
use noble::event::AppEvent;
use noble::keys::Action;
use noble::store::{SavedTab, Workspace};
use noble::term::layout::SavedNode;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::common::*;

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
    assert!(app.tabs[0].alert.is_some());
    assert!(app.outer_bell);
    // The same message is queued for the outer terminal's desktop notification (OSC 9 / 777).
    assert!(app.outer_notice.as_deref().is_some_and(|t| t.contains("Build finished")), "{:?}", app.outer_notice);
    assert!(app.toasts.iter().any(|t| t.text.contains("Build finished")), "toast missing");
    let text = render(&mut app, 110, 30);
    save("terminal-alert-110x30", &text);
    assert!(text.lines().next().unwrap().contains('◆'), "{text}");
    app.run(Action::GoTab(1));
    app.tick();
    assert!(app.tabs[0].alert.is_none());
    app.run(Action::CloseTab);
    app.run(Action::CloseTab);
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
    // The tab has two panes: the menu's close asks first.
    assert!(matches!(app.overlay, Some(noble::app::Overlay::Confirm(_))));
    app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
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
    // Every tab opens in the same folder: keep the names they were opened with to tell them apart.
    app.cfg.terminal.tab_follows_cwd = false;
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

/// Closing a split tab keeps it: `reopen_tab` brings its panes back in their directories, at the old
/// index and focused, and the palette lists the action only while there is something to reopen. With
/// nothing to reopen it says so; a shell that exits on its own is not kept.
#[test]
fn closed_tab_reopens_with_its_panes() {
    use noble::term::layout::Dir;
    let has_reopen = |app: &App| app.palette_items().iter().any(|i| i.title == "Reopen Closed Tab");
    let base = std::env::temp_dir().join(format!("noble-reopen-{}", std::process::id()));
    let (alpha, beta) = (base.join("alpha"), base.join("beta"));
    std::fs::create_dir_all(&alpha).unwrap();
    std::fs::create_dir_all(&beta).unwrap();

    let mut app = demo_app(110, 30);
    assert_eq!(app.keymap.prefix_key(Action::ReopenTab).as_deref(), Some("T"), "prefix T is free");
    app.new_tab(std::env::temp_dir(), None, Some("first".into()));
    let layout = SavedNode::Split {
        dir: Dir::Row,
        ratio: 0.5,
        a: Box::new(SavedNode::Leaf { cwd: alpha.display().to_string(), launch: None }),
        b: Box::new(SavedNode::Leaf { cwd: beta.display().to_string(), launch: None }),
    };
    let ws = Workspace {
        name: "split".into(),
        saved_at: 0,
        tabs: vec![SavedTab { name: Some("grid".into()), origin: "grid".into(), layout, focus: 1 }],
    };
    assert_eq!(app.open_workspace(&ws), 1);
    app.view = View::Term(1);
    let dirs = |app: &App| -> Vec<PathBuf> { app.tabs[1].panes().iter().map(|id| app.panes[id].cwd()).collect() };
    let before = dirs(&app);
    assert_eq!(before, vec![alpha.clone(), beta.clone()]);
    assert!(!has_reopen(&app), "nothing closed yet");

    app.run(Action::CloseTab);
    app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert_eq!(app.tabs.len(), 1, "the split tab closed");
    let key = app.keymap.hint(Action::ReopenTab).expect("T is bound");
    let hint = format!("tab closed · {key} reopens it (launch commands run again)");
    assert!(
        app.toasts.iter().any(|t| t.text == hint),
        "toast {hint:?}: {:?}",
        app.toasts.iter().map(|t| &t.text).collect::<Vec<_>>()
    );
    assert!(has_reopen(&app), "listed once a tab can come back");

    app.run(Action::ReopenTab);
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.view, View::Term(1), "back at its old index and focused");
    assert_eq!(app.tabs[1].panes().len(), 2, "same number of panes");
    assert_eq!(dirs(&app), before, "same directories in the same order");
    assert_eq!(app.tabs[1].focus, app.tabs[1].panes()[1], "the saved focus comes back");
    assert_eq!(app.tabs[1].name.as_deref(), Some("grid"));
    assert!(!has_reopen(&app), "history is used up");
    let text = render(&mut app, 110, 30);
    save("reopened-tab-110x30", &text);

    app.run(Action::ReopenTab);
    assert!(app.toasts.iter().any(|t| t.text == "no closed tab to reopen"), "empty history says so");

    // A shell that exits on its own closes its tab without keeping it.
    app.new_tab(std::env::temp_dir(), None, Some("gone".into()));
    let id = app.tabs.last().unwrap().focus;
    let (tabs, kept) = (app.tabs.len(), app.closed_tabs.len());
    app.handle(AppEvent::PtyExit(id));
    assert_eq!(app.tabs.len(), tabs - 1, "the exited shell's tab closed");
    assert_eq!(app.closed_tabs.len(), kept, "an exit is not a close the user made");
    std::fs::remove_dir_all(&base).ok();
}

/// A tab menu or rename prompt names its tab by its stable id, not by index: when another tab closes
/// (its shell exits) while the menu is open, the command still hits the tab it was opened for,
/// and once every tab is gone nothing panics.
#[test]
fn tab_menu_and_prompt_survive_tabs_closing() {
    use noble::app::{MenuCmd, Overlay};
    let mut app = demo_app(110, 30);
    for name in ["first", "second", "third"] {
        app.new_tab(std::env::temp_dir(), None, Some(name.into()));
    }
    let first = app.tabs[0].focus;
    let menu_cmd = |app: &App, label: &str| match &app.overlay {
        Some(Overlay::Menu(m)) => m.items.iter().find(|i| i.label.starts_with(label)).map(|i| i.cmd.clone()),
        _ => None,
    };

    // Rename: the menu for "third" (index 2) is open when "first" exits; "third" is index 1 now.
    app.open_tab_menu(2, 5, 5);
    let rename = menu_cmd(&app, "Rename").expect("rename item");
    app.handle(AppEvent::PtyExit(first));
    assert_eq!(app.tabs.len(), 2);
    app.run_menu(rename);
    let Some(Overlay::Prompt(mut prompt)) = app.overlay.take() else { panic!("no rename prompt") };
    prompt.value = "renamed".into();
    app.submit_prompt(prompt);
    assert_eq!(app.tabs[1].name.as_deref(), Some("renamed"));
    assert_eq!(app.tabs[0].name, None, "the wrong tab was renamed");

    // A rename prompt whose tab closes meanwhile is dropped, not applied to a neighbour.
    app.run(Action::GoTab(1));
    app.run(Action::RenameTab);
    let Some(Overlay::Prompt(mut prompt)) = app.overlay.take() else { panic!("no rename prompt") };
    let gone = app.tabs[0].focus;
    app.handle(AppEvent::PtyExit(gone));
    prompt.value = "late".into();
    app.submit_prompt(prompt);
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.tabs[0].name.as_deref(), Some("renamed"), "the prompt renamed another tab");

    // Close tab and move tab: the menu was opened for a tab that is gone by the time it runs.
    app.new_tab(std::env::temp_dir(), None, Some("extra".into()));
    let (keep, extra, extra_tab) = (app.tabs[0].focus, app.tabs[1].focus, app.tabs[1].id);
    app.open_tab_menu(0, 5, 5);
    let (mv, close) = (menu_cmd(&app, "Move right").expect("move item"), menu_cmd(&app, "Close").expect("close"));
    app.handle(AppEvent::PtyExit(keep));
    assert_eq!(app.tabs.len(), 1);
    app.run_menu(mv);
    assert_eq!(app.tabs[0].focus, extra, "moving a vanished tab changed the rest");
    app.run_menu(close);
    assert_eq!(app.tabs.len(), 1, "a vanished tab's close command closed another tab");

    // Every tab gone: the queued commands do nothing (a clamp into an empty list used to panic).
    app.handle(AppEvent::PtyExit(extra));
    assert!(app.tabs.is_empty());
    app.run_menu(MenuCmd::MoveTab(extra_tab, 1));
    app.run_menu(MenuCmd::CloseTab(extra_tab));
    app.run_menu(MenuCmd::RenameTab(extra_tab));
    assert!(app.overlay.is_none());
}

/// A folder whose name contains " · " keeps its whole name as the tab title.
#[test]
fn tab_title_keeps_folder_names_with_separator() {
    let mut app = demo_app(110, 30);
    app.cfg.terminal.tab_follows_cwd = false;
    app.new_tab(std::env::temp_dir(), None, Some("proj · notes".into()));
    assert_eq!(app.tab_title(0), "proj · notes");
    // A launcher tab keeps naming what runs while it runs.
    app.new_tab(std::env::temp_dir(), Some("echo hi"), Some("proj · notes · claude".into()));
    assert_eq!(app.tab_title(1), "proj · notes · claude");
}

/// The tab strip leads each tab's title with the most urgent agent state among its panes:
/// ◆ needs you, ● your turn, a spinner (or "…") while working, ○ unknown.
#[test]
fn tab_strip_shows_agent_dots() {
    let mut app = demo_app(120, 30);
    set_power(&mut app, true);
    app.new_tab(std::env::temp_dir(), None, None);
    app.run(Action::SplitRight);
    let (a, b) = (app.tabs[0].panes()[0], app.tabs[0].panes()[1]);
    app.new_tab(std::env::temp_dir(), None, None);
    let c = app.tabs[1].focus;
    app.new_tab(std::env::temp_dir(), None, None);
    for (tab, name) in app.tabs.iter_mut().zip(["first", "second", "third"]) {
        tab.name = Some(name.into());
    }
    let now = chrono::Utc::now().timestamp();
    // Tab 1: one pane finished and one needs you, so ◆. Tab 2: working.
    app.view = View::Term(2);
    app.take_dirty();
    app.apply_hook_records(
        [(a, hook_rec("stop", now, 0)), (b, hook_rec("notification", now, 0)), (c, hook_rec("prompt", now, 0))].into(),
    );
    assert!(app.take_dirty(), "the dots of other tabs changed");
    let (text, buf) = render_buf(&mut app, 120, 30);
    save("tabs-agent-dots-120x30", &text);
    let top = text.lines().next().unwrap().to_string();
    assert!(top.contains(" ◆ first"), "{top}");
    // No working pane is visible (tab 3 is shown): the working dot is static.
    assert!(top.contains(" … second"), "{top}");
    assert!(top.contains("▌third ×▐") && !top.contains('○'), "{top}");
    // The agent dot already says "needs you": the notice does not repeat as a marker.
    assert_eq!(top.matches('◆').count(), 1, "{top}");
    assert!(idle_redraw(&app), "a working agent in a hidden tab costs nothing: {:?}", app.redraw_after());
    let x = (0..120).find(|x| buf[(*x, 0)].symbol() == "◆").unwrap();
    assert_eq!(buf[(x, 0)].fg, app.theme.warn);
    // Showing the working tab: its dot spins with the pane title while the pane shows output.
    app.view = View::Term(1);
    let top = render(&mut app, 120, 30).lines().next().unwrap().to_string();
    assert!(top.contains("▌… second"), "a silent working pane does not spin: {top}");
    assert!(idle_redraw(&app), "{:?}", app.redraw_after());
    app.panes.get_mut(&c).unwrap().last_output = Some(Instant::now());
    let top = render(&mut app, 120, 30).lines().next().unwrap().to_string();
    assert!(top.chars().any(|ch| SPIN.contains(ch)), "{top}");
    assert!(app.redraw_after().unwrap() <= Duration::from_millis(150));
    // The finished pane alone: ●.
    app.apply_hook_records([(a, hook_rec("stop", now, 0)), (c, hook_rec("prompt", now, 0))].into());
    let top = render(&mut app, 120, 30).lines().next().unwrap().to_string();
    assert!(top.contains(" ● first"), "{top}");
    // Small sizes keep the strip within bounds.
    for (w, h) in [(80, 24), (60, 20), (30, 8)] {
        let text = render(&mut app, w, h);
        save(&format!("tabs-agent-dots-{w}x{h}"), &text);
        assert!(text.lines().next().unwrap().chars().count() <= w as usize);
    }
    for _ in 0..3 {
        app.view = View::Term(0);
        app.run(Action::CloseTab);
    }
}

/// Opens a tab with a fixed name (the automatic name is the folder).
fn named_tab(app: &mut App, name: &str) {
    app.new_tab(std::env::temp_dir(), None, None);
    app.tabs.last_mut().unwrap().name = Some(name.into());
}

/// The top bar's tab markers say what happened in a background tab: ◆ notice, ✗ failed command,
/// ✓ long command done, • output. The marker has its own cell after the title.
#[test]
fn tab_markers_are_typed() {
    use noble::term::TabAlert;
    let mut app = demo_app(120, 30);
    for name in ["notice", "failed", "done", "quiet", "active"] {
        named_tab(&mut app, name);
    }
    app.tabs[0].raise(TabAlert::Notice);
    app.tabs[1].raise(TabAlert::Failed);
    app.tabs[2].raise(TabAlert::Done);
    app.tabs[3].activity = true;
    assert_eq!(app.view, View::Term(4));
    let (text, buf) = render_buf(&mut app, 120, 30);
    save("tabs-markers-120x30", &text);
    let top = text.lines().next().unwrap().to_string();
    for row in ["notice ◆ ×", "failed ✗ ×", "done ✓ ×", "quiet • ×"] {
        assert!(top.contains(row), "{row}: {top}");
    }
    let th = &app.theme;
    let color = |glyph: &str| {
        let x = (0..120u16).find(|x| buf[(*x, 0)].symbol() == glyph).unwrap();
        buf[(x, 0)].fg
    };
    assert_eq!(color("◆"), th.warn);
    assert_eq!(color("✗"), th.crit);
    assert_eq!(color("✓"), th.ok);
    assert_eq!(color("•"), th.accent2);
    // Nothing overwrites the title's last letter, and the active tab has no marker.
    assert!(top.contains("notice") && top.contains("▌active ×▐"), "{top}");
    // Priority: a notice or failure beats a finished command, which beats plain output; less urgent
    // events never replace a more urgent one.
    let t = &mut app.tabs[3];
    t.raise(TabAlert::Done);
    t.raise(TabAlert::Failed);
    t.raise(TabAlert::Done);
    assert_eq!(t.alert, Some(TabAlert::Failed));
    t.raise(TabAlert::Notice);
    t.raise(TabAlert::Failed);
    assert_eq!(t.alert, Some(TabAlert::Notice));
    app.tabs[3].alert = Some(TabAlert::Done);
    let top = render(&mut app, 120, 30).lines().next().unwrap().to_string();
    assert!(top.contains("quiet ✓ ×") && !top.contains("quiet • ×"), "{top}");
    // The whole row stays inside the screen at small sizes.
    for (w, h) in [(80, 24), (60, 20), (40, 12), (30, 8)] {
        let text = render(&mut app, w, h);
        save(&format!("tabs-markers-{w}x{h}"), &text);
        let top = text.lines().next().unwrap().to_string();
        assert!(top.chars().count() <= w as usize);
        // The active tab is never summarized into a chip: the window slides to include it.
        if (w, h) == (80, 24) || (w, h) == (60, 20) {
            assert!(top.contains("▌active ×▐"), "{w}x{h}: {top}");
        }
        for (r, _) in app.hits.iter().filter(|(r, h)| r.y == 0 && matches!(h, noble::app::Hit::Tab(_))) {
            assert!(r.right() <= w, "{w}x{h}: {r:?}");
        }
    }
    // Visiting a tab clears its marker.
    app.run(Action::GoTab(1));
    app.tick();
    assert!(app.tabs[0].alert.is_none() && !app.tabs[0].activity);
    let top = render(&mut app, 120, 30).lines().next().unwrap().to_string();
    assert!(!top.contains("notice ◆"), "{top}");
    for _ in 0..5 {
        app.run(Action::CloseTab);
    }
}

/// Whether a top strip row shows the small clock ("HH:MM").
fn has_small_clock(row: &str) -> bool {
    row.chars().collect::<Vec<_>>().windows(5).any(|w| {
        w[0].is_ascii_digit() && w[1].is_ascii_digit() && w[2] == ':' && w[3].is_ascii_digit() && w[4].is_ascii_digit()
    })
}

/// The window of terminal tabs always contains the active one: tabs before it are summarized as "‹N"
/// (a palette button like "+N"), tabs after it as "+N", and the hit rects stay on the screen.
#[test]
fn active_tab_stays_in_the_strip() {
    use noble::app::Hit;
    let mut app = demo_app(80, 24);
    for i in 0..9 {
        named_tab(&mut app, &format!("t{i}"));
    }
    for (w, h) in [(80, 24), (60, 20)] {
        for active in [8, 4, 0] {
            app.view = View::Term(active);
            let text = render(&mut app, w, h);
            let top = text.lines().next().unwrap().to_string();
            let name = format!("▌t{active} ×▐");
            assert!(top.contains(&name), "{w}x{h} tab {active}: {top}");
            for (r, _) in app.hits.iter().filter(|(r, _)| r.y == 0) {
                assert!(r.right() <= w, "{w}x{h}: {r:?}");
            }
            let tab = find_hit(&app, |h| *h == Hit::Tab(active)).expect("active tab hit");
            assert!(tab.right() <= w);
            if active == 8 {
                // Tabs before the window: a left chip that opens the palette.
                // (The " + " new-tab button is not a count: no "+N" chip after the window.)
                assert!(top.contains("‹5") && !top.contains("+1"), "{top}");
                let (_, buf) = render_buf(&mut app, w, h);
                let chip = find_hit(&app, |h| *h == Hit::HiddenTabs).expect("‹N chip");
                assert!(cells_text(&buf, chip).trim_start().starts_with('‹'), "{top}");
                click(&mut app, chip.x, chip.y);
                assert!(matches!(app.overlay, Some(noble::app::Overlay::Palette(_))));
                app.overlay = None;
            }
        }
    }
    // A tab in the middle shows both chips.
    app.view = View::Term(4);
    let top = render(&mut app, 80, 24).lines().next().unwrap().to_string();
    assert!(top.contains("‹") && top.contains("+") && top.contains("▌t4 ×▐"), "{top}");
    for _ in 0..9 {
        app.run(Action::CloseTab);
    }
}

/// Home has the big clock, so the top strip shows no small one there (the tabs get the room); the
/// other screens keep it. Compact widths draw the fixed Home and System tabs as one glyph each.
#[test]
fn top_strip_clock_and_compact_fixed_tabs() {
    use noble::app::Hit;
    let mut app = demo_app(120, 30);
    named_tab(&mut app, "work");
    app.view = View::Bridge;
    let home = render(&mut app, 120, 30).lines().next().unwrap().to_string();
    assert!(!has_small_clock(&home) && home.contains(" Home ") && home.contains("Settings"), "{home}");
    app.view = View::Term(0);
    let term = render(&mut app, 120, 30).lines().next().unwrap().to_string();
    assert!(has_small_clock(&term), "{term}");
    // Wide: the words. Compact (under 80 columns): one glyph each, hit rects over the glyph.
    app.view = View::Bridge;
    let wide = render(&mut app, 80, 24).lines().next().unwrap().to_string();
    assert!(wide.contains(" Home ") && wide.contains(" System "), "{wide}");
    let (text, buf) = render_buf(&mut app, 60, 20);
    let compact = text.lines().next().unwrap().to_string();
    assert!(compact.contains(" ⌂ ") && compact.contains(" ▤ "), "{compact}");
    assert!(!compact.contains("Home") && !compact.contains("System"), "{compact}");
    for (hit, glyph) in [(Hit::TabBridge, "⌂"), (Hit::TabSystem, "▤")] {
        let r = find_hit(&app, |h| *h == hit).expect("fixed tab hit");
        assert_eq!(r.width, 3, "{r:?}");
        assert_eq!(cells_text(&buf, r).trim(), glyph, "{compact}");
    }
}

/// A command that ends in a background tab leaves ✗ (non-zero exit code) or ✓ (a long one that worked);
/// a quick success leaves no marker, and only a failure that took a while raises a toast.
#[test]
fn background_command_result_marks_tab() {
    use noble::term::TabAlert;
    let mut app = demo_app(110, 30);
    named_tab(&mut app, "worker");
    named_tab(&mut app, "front");
    std::thread::sleep(Duration::from_millis(800));
    app.pump();
    let worker = app.tabs[0].focus;
    let ms = Duration::from_millis;
    // Finishes a command typed at the prompt `ago` ago with the exit code `code`.
    let finish = |app: &mut App, ago: Duration, code: i32, typed: bool| {
        {
            let pane = app.panes.get_mut(&worker).unwrap();
            pane.prompted = true;
            pane.edited = typed;
            pane.command_started = Some(std::time::Instant::now() - ago);
            pane.parser().process(format!("\x1b]133;D;{code}\x07").as_bytes());
            pane.dirty.store(true, std::sync::atomic::Ordering::Release);
        }
        app.toasts.clear();
        app.handle(AppEvent::PtyOutput);
    };
    let reset = |app: &mut App| {
        app.tabs[0].alert = None;
        app.tabs[0].activity = false;
    };
    // A quick success: output only.
    finish(&mut app, ms(200), 0, true);
    assert_eq!(app.tabs[0].alert, None);
    assert!(app.tabs[0].activity && app.toasts.is_empty());
    // A quick failure: the marker, but no toast.
    reset(&mut app);
    finish(&mut app, ms(200), 1, true);
    assert_eq!(app.tabs[0].alert, Some(TabAlert::Failed));
    assert!(app.toasts.is_empty(), "{:?}", app.toasts.iter().map(|t| &t.text).collect::<Vec<_>>());
    let top = render(&mut app, 110, 30).lines().next().unwrap().to_string();
    assert!(top.contains("worker ✗ ×"), "{top}");
    // A long success: ✓ and a toast.
    reset(&mut app);
    finish(&mut app, Duration::from_secs(3600), 0, true);
    assert_eq!(app.tabs[0].alert, Some(TabAlert::Done));
    assert!(app.toasts.iter().any(|t| t.text.contains("done in")), "toast missing");
    let top = render(&mut app, 110, 30).lines().next().unwrap().to_string();
    assert!(top.contains("worker ✓ ×"), "{top}");
    // A long failure: ✗ with its code in the toast.
    reset(&mut app);
    finish(&mut app, Duration::from_secs(3600), 2, true);
    assert_eq!(app.tabs[0].alert, Some(TabAlert::Failed));
    assert!(app.toasts.iter().any(|t| t.text.contains("failed (exit 2)")), "toast missing");
    // A command that was not typed (a launcher command, the first prompt) never counts as failed.
    reset(&mut app);
    finish(&mut app, ms(200), 1, false);
    assert_eq!(app.tabs[0].alert, None);
    // A notice beats a failure on the same tab.
    finish(&mut app, ms(200), 1, true);
    app.tabs[0].raise(TabAlert::Notice);
    assert_eq!(app.tabs[0].alert, Some(TabAlert::Notice));
    app.run(Action::CloseTab);
    app.run(Action::CloseTab);
}

/// A split tab shows its pane count ("⊞2") when the strip has room; compact mode keeps it for the
/// active tab only. The active tab has edge bars and hovering a tab still highlights it.
#[test]
fn tab_strip_split_count_and_active_edges() {
    use noble::app::Hit;
    assert_eq!(unicode_width::UnicodeWidthStr::width("⊞"), 1);
    let mut app = demo_app(120, 30);
    named_tab(&mut app, "one");
    app.run(Action::SplitRight);
    app.run(Action::SplitDown);
    named_tab(&mut app, "two");
    app.run(Action::SplitRight);
    named_tab(&mut app, "solo");
    let (text, buf) = render_buf(&mut app, 120, 30);
    save("tabs-split-120x30", &text);
    let top = text.lines().next().unwrap().to_string();
    assert!(top.contains("one ⊞3 ×") && top.contains("two ⊞2 ×"), "{top}");
    assert!(top.contains("▌solo ×▐") && !top.contains("solo ⊞"), "{top}");
    // The active tab is drawn with accent edges on the base background.
    let bar = (0..120u16).find(|x| buf[(*x, 0)].symbol() == "▌").unwrap();
    assert_eq!((buf[(bar, 0)].fg, buf[(bar, 0)].bg), (app.theme.accent, app.theme.bg));
    let end = (0..120u16).find(|x| buf[(*x, 0)].symbol() == "▐").unwrap();
    assert_eq!(buf[(end, 0)].fg, app.theme.accent);
    assert!(end > bar);
    // Hover still highlights a tab, and clicks land on the right tab.
    let r = find_hit(&app, |h| *h == Hit::Tab(0)).unwrap();
    app.hover = Some((r.x + 1, 0));
    let (_, buf) = render_buf(&mut app, 120, 30);
    assert_eq!(buf[(r.x + 1, 0)].bg, app.theme.hover());
    click(&mut app, r.x + 1, 0);
    assert_eq!(app.view, View::Term(0));
    // Compact mode: only the active tab shows its count.
    let top = render(&mut app, 70, 20).lines().next().unwrap().to_string();
    assert!(top.contains("▌one ⊞3 ×▐") && !top.contains("two ⊞"), "{top}");
    app.view = View::Term(2);
    let top = render(&mut app, 70, 20).lines().next().unwrap().to_string();
    assert!(!top.contains('⊞'), "{top}");
    // Small sizes keep the strip within bounds.
    app.view = View::Term(0);
    for (w, h) in [(160, 45), (80, 24), (60, 20), (44, 12), (40, 12), (30, 8)] {
        let text = render(&mut app, w, h);
        save(&format!("tabs-split-{w}x{h}"), &text);
        assert!(text.lines().next().unwrap().chars().count() <= w as usize);
    }
    for _ in 0..3 {
        app.run(Action::CloseTab);
    }
}

/// Tabs that do not fit collapse into "+N"; the most urgent marker among them stays visible.
#[test]
fn hidden_tabs_summary_keeps_the_marker() {
    use noble::term::TabAlert;
    let mut app = demo_app(80, 24);
    for i in 0..12 {
        named_tab(&mut app, &format!("tab{i}"));
    }
    app.view = View::Term(0);
    app.tabs[11].raise(TabAlert::Failed);
    app.tabs[10].raise(TabAlert::Done);
    let (text, buf) = render_buf(&mut app, 80, 24);
    save("tabs-hidden-80x24", &text);
    let top = text.lines().next().unwrap().to_string();
    let plus = top.find('+').unwrap_or_else(|| panic!("no +N: {top}"));
    let tail = &top[plus..];
    assert!(tail.contains('✗') && !tail.contains('✓'), "{top}");
    let x = (0..80u16).find(|x| buf[(*x, 0)].symbol() == "✗").unwrap();
    assert_eq!(buf[(x, 0)].fg, app.theme.crit);
    // The count matches the tabs that were left out.
    let shown = app.hits.iter().filter(|(r, h)| r.y == 0 && matches!(h, noble::app::Hit::Tab(_))).count();
    assert!(top.contains(&format!("+{}", 12 - shown)), "{top}");
    for _ in 0..12 {
        app.run(Action::CloseTab);
    }
}

/// The "+N" chip of the tab bar is a button: it opens the palette listing only the tabs, the hidden ones too.
#[test]
fn hidden_tabs_chip_opens_the_tab_list() {
    use noble::app::{Hit, Overlay, PaletteCmd};
    let mut app = demo_app(80, 24);
    for i in 0..12 {
        named_tab(&mut app, &format!("tab{i}"));
    }
    app.view = View::Term(0);
    let (text, buf) = render_buf(&mut app, 80, 24);
    let chip = find_hit(&app, |h| *h == Hit::HiddenTabs).expect("+N chip");
    assert_eq!(chip.y, 0);
    assert!(cells_text(&buf, chip).trim_start().starts_with('+'), "{text}");
    click(&mut app, chip.x, chip.y);
    let Some(Overlay::Palette(st)) = &app.overlay else { panic!("no palette: {text}") };
    assert_eq!(st.query, "");
    let listed: Vec<PaletteCmd> = st.matches.iter().map(|i| st.all[*i].cmd.clone()).collect();
    // The tab bar hides the last tabs: the palette lists all of them, and nothing else.
    assert_eq!(listed, (1..=12).map(PaletteCmd::GoTab).collect::<Vec<_>>());
    let text = render(&mut app, 80, 24);
    save("palette-hidden-tabs-80x24", &text);
    assert!(text.contains("Tab 12: tab11"), "{text}");
}
