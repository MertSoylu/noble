//! Split panes, pane title bars and buttons, mouse selection and pane actions.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use noble::app::{App, View};
use noble::event::AppEvent;
use noble::keys::Action;
use std::time::Duration;

use super::common::*;

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

/// Split panes share one border line joined with `┬ ├ ┤ ┴ ┼`; the outer left and right sides
/// have no line. The horizontal divider runs along the lower pane's title row: the line drags,
/// the title text and buttons stay clickable.
#[test]
fn split_panes_share_joined_borders() {
    use crossterm::event::{MouseButton, MouseEventKind};
    use noble::app::Hit;
    use noble::term::layout::Dir;
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("grid".into()));
    for action in [Action::SplitRight, Action::SplitDown, Action::FocusLeft, Action::SplitDown] {
        app.run(action);
    }
    assert_eq!(app.tabs[0].panes().len(), 4);
    let text = render(&mut app, 110, 30);
    save("terminal-grid-110x30", &text);
    let rows: Vec<Vec<char>> = text.lines().map(|l| l.chars().collect()).collect();
    for junction in ['┬', '┴', '┼'] {
        assert!(text.contains(junction), "no {junction}:\n{text}");
    }
    assert!(!text.contains("││") && !text.contains("╮╭"), "double border:\n{text}");
    // The window's outer left and right sides have no line: lines only run between panes. (The panes'
    // own text may reach the edge: a shell prompt starts in column 0.)
    let body = app.body();
    for y in body.top()..body.bottom() {
        let row = &rows[y as usize];
        for edge in [row[0], row[row.len() - 1]] {
            assert!(!"│┃├┤┌┐└┘╭╮╰╯┬┴┼".contains(edge), "outer side line at row {y}:\n{text}");
        }
    }
    let top = |x: u16, y: u16| {
        app.hits.iter().rev().find(|(r, _)| r.contains(ratatui::layout::Position { x, y })).map(|(_, h)| h.clone())
    };
    // The left column's horizontal divider and the lower-left pane on it.
    let div = app
        .hits
        .iter()
        .find_map(|(r, h)| match h {
            Hit::Divider { div, .. } if div.dir == Dir::Col && r.x == 0 => Some(*r),
            _ => None,
        })
        .expect("horizontal divider");
    let y = div.y;
    assert!(matches!(top(3, y), Some(Hit::PaneTitle(_))), "title text is not on top");
    let line_x = (0..div.right()).rev().find(|x| rows[y as usize][*x as usize] == '─' && x + 1 < div.right()).unwrap();
    assert!(matches!(top(line_x, y), Some(Hit::Divider { .. })), "the line does not drag");
    let upper = |app: &App| app.tabs[0].root.layout(app.body()).0[0].1.height;
    let before = upper(&app);
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), line_x, y);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), line_x, y + 4);
    mouse(&mut app, MouseEventKind::Up(MouseButton::Left), line_x, y + 4);
    render(&mut app, 110, 30);
    assert_eq!(upper(&app), before + 4, "divider did not move");
    // The lower-left pane's close button on the divider row still closes it.
    let close = app
        .hits
        .iter()
        .rev()
        .find_map(|(r, h)| matches!(h, Hit::PaneClose(_)).then_some(*r).filter(|r| r.y == y + 4 && r.x < 55))
        .expect("close button on the divider row");
    click(&mut app, close.x + 1, close.y);
    assert_eq!(app.tabs[0].panes().len(), 3);
    app.run(Action::CloseTab);
}

/// In a terminal, "drag border resize" is offered only for a tab with more than one pane; right-click paste is
/// not a hint (the pane menu and the keyboard reference name it).
#[test]
fn terminal_hints_offer_drag_only_with_a_split() {
    let mut app = demo_app(200, 40);
    app.new_tab(std::env::temp_dir(), None, Some("split".into()));
    let text = render(&mut app, 200, 40);
    let bar = text.lines().last().unwrap();
    assert!(bar.contains("menu") && !bar.contains("drag border") && !bar.contains("right-click"), "{text}");
    app.run(Action::SplitRight);
    let text = render(&mut app, 200, 40);
    assert!(text.lines().last().unwrap().contains("drag border resize"), "{text}");
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
    // The lock also shows on a pane too narrow for the corner tag and all its buttons (only close
    // comes first).
    let narrow = render(&mut app, 16, 10);
    save("term-keys-locked-16x10", &narrow);
    assert!(narrow.contains('🔒') && narrow.contains('✕') && !narrow.contains("KEYS"), "{narrow}");
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
    app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.tabs.is_empty());
}

/// What was mouse-only has an action (and a prefix key): moving tabs, the pane menu,
/// the update notice.
#[test]
fn mouse_only_things_have_actions() {
    use noble::app::Overlay;
    use noble::keys::Chord;
    let mut app = demo_app(110, 30);
    // Every tab opens in the same folder: keep the names they were opened with to tell them apart.
    app.cfg.terminal.tab_follows_cwd = false;
    for name in ["one", "two", "three"] {
        app.new_tab(std::env::temp_dir(), None, Some(name.into()));
    }
    // The title's " · <label>" part follows what the shell is running, which a real shell may still be
    // setting up; the order is what this test checks.
    let names = |app: &App| {
        (0..app.tabs.len())
            .map(|i| app.tab_title(i).split(" · ").next().unwrap_or_default().to_string())
            .collect::<Vec<_>>()
    };
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

/// Closing the last pane of a tab (prefix `x`) is a close too: the tab comes back with `reopen_tab`.
#[test]
fn closing_the_last_pane_keeps_the_tab() {
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("solo".into()));
    app.new_tab(std::env::temp_dir(), None, Some("other".into()));
    app.view = View::Term(0);
    app.run(Action::ClosePane);
    assert_eq!(app.tabs.len(), 1, "the one pane closed its tab");
    app.run(Action::ReopenTab);
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.view, View::Term(0), "reopened at index 0");
    assert_eq!(app.tabs[0].origin, "solo");
    assert_eq!(app.tabs[0].panes().len(), 1);
}

/// In a split tab the pane a tab menu, rename prompt or close confirmation was opened from can
/// close while the tab lives on: the command still reaches that tab (it used to do nothing).
#[test]
fn tab_commands_survive_their_pane_closing_in_a_split_tab() {
    use noble::app::{ConfirmAction, MenuCmd, Overlay};
    let mut app = demo_app(110, 30);
    for name in ["left", "split"] {
        app.new_tab(std::env::temp_dir(), None, Some(name.into()));
    }
    app.run(Action::SplitRight);
    assert_eq!(app.tabs[1].panes().len(), 2);
    let menu_cmd = |app: &App, label: &str| match &app.overlay {
        Some(Overlay::Menu(m)) => m.items.iter().find(|i| i.label.starts_with(label)).map(|i| i.cmd.clone()),
        _ => None,
    };

    // Menu: opened from the focused (new) pane, which closes before the command runs.
    app.open_tab_menu(1, 5, 5);
    let (rename, mv) = (menu_cmd(&app, "Rename").expect("rename"), menu_cmd(&app, "Move left").expect("move"));
    let focused = app.tabs[1].focus;
    app.close_pane(focused);
    assert_eq!(app.tabs[1].panes().len(), 1, "the tab should survive");
    app.run_menu(rename);
    let Some(Overlay::Prompt(mut prompt)) = app.overlay.take() else { panic!("no rename prompt") };
    // The prompt, too, outlives another pane of its tab closing.
    app.run(Action::SplitDown);
    let focused = app.tabs[1].focus;
    app.close_pane(focused);
    prompt.value = "renamed".into();
    app.submit_prompt(prompt);
    assert_eq!(app.tabs[1].name.as_deref(), Some("renamed"), "the rename was dropped");
    assert_eq!(app.tabs[0].name, None);
    app.run_menu(mv);
    assert_eq!(app.tabs[0].name.as_deref(), Some("renamed"), "the move was dropped");

    // Close confirmation: the pane that was focused when it was asked has closed since.
    app.run(Action::GoTab(1));
    app.run(Action::SplitRight);
    let (id, focused) = (app.tabs[0].id, app.tabs[0].focus);
    app.close_pane(focused);
    app.confirm(ConfirmAction::CloseTab(id));
    assert_eq!(app.tabs.len(), 1, "the close was dropped");
    assert_eq!(app.tabs[0].origin, "left", "the wrong tab was closed");
    // A closed tab's id resolves to nothing.
    app.confirm(ConfirmAction::CloseTab(id));
    app.run_menu(MenuCmd::RenameTab(id));
    assert_eq!(app.tabs.len(), 1);
    assert!(app.overlay.is_none());
}

/// A launcher tab split in two drops the launcher from its title once the launcher's pane is
/// closed (the other pane never ran a command), and is saved under the project name alone.
#[test]
fn launcher_suffix_goes_with_its_pane_in_a_split_tab() {
    let mut app = demo_app(110, 30);
    app.cfg.terminal.tab_follows_cwd = false;
    app.new_tab(std::env::temp_dir(), Some("echo hi"), Some("proj · claude".into()));
    let launcher = app.tabs[0].focus;
    app.run(Action::SplitRight);
    app.close_pane(launcher);
    assert_eq!(app.tabs[0].panes().len(), 1);
    let title = app.tab_title(0);
    assert!(title.starts_with("proj") && !title.contains("claude"), "stale launcher title: {title:?}");
    let saved = app.snapshot("s");
    assert_eq!(saved.tabs[0].origin, "proj", "saved with the stale launcher name");

    // A plain tab in a folder named with the separator is not a launcher tab.
    app.new_tab(std::env::temp_dir(), None, Some("proj · notes".into()));
    app.run(Action::SplitRight);
    assert_eq!(app.snapshot("s").tabs[1].origin, "proj · notes");
    assert!(app.tab_title(1).starts_with("proj · notes"), "{:?}", app.tab_title(1));
}

/// A wide character (CJK) in the last two content columns is blanked where the position bar covers
/// its second cell; otherwise the terminal would never be sent the bar's cell on that row.
#[test]
fn scroll_bar_clears_wide_characters_under_it() {
    use noble::app::Hit;
    let mut app = demo_app(80, 24);
    app.new_tab(std::env::temp_dir(), None, Some("wide".into()));
    let id = app.tabs[0].focus;
    wait_idle(&mut app, id);
    render(&mut app, 80, 24);
    let cols = app.panes[&id].size.1 as usize;
    {
        let pane = &app.panes[&id];
        let text: String = (0..80).map(|i| format!("{i:<w$}中\r\n", w = cols - 2)).collect();
        pane.parser().process(text.as_bytes());
        pane.scroll(2);
    }
    let (text, buf) = render_buf(&mut app, 80, 24);
    save("pane-scrolled-wide-80x24", &text);
    let track = find_hit(&app, |h| matches!(h, Hit::ScrollTrack { .. })).expect("position bar");
    for y in track.y..track.bottom() {
        assert_eq!(buf[(track.x - 1, y)].symbol(), " ", "row {y}");
        assert!(matches!(buf[(track.x, y)].symbol(), "│" | "┃"), "row {y}");
    }
    app.run(Action::CloseTab);
}

/// Panes share their border lines: with the mouse on the line between two panes (the lower one's
/// title row), only the lower pane shows its buttons.
#[test]
fn pane_buttons_hover_one_pane_on_a_shared_line() {
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("shared".into()));
    app.run(Action::SplitDown);
    let (top, bottom) = (app.tabs[0].panes()[0], app.tabs[0].panes()[1]);
    app.tabs[0].focus = top;
    app.hover = None;
    render(&mut app, 110, 30);
    let title = app.hits.iter().find(|(_, h)| *h == noble::app::Hit::PaneTitle(bottom)).map(|(r, _)| *r).unwrap();
    app.hover = Some((title.x + 20, title.y));
    let text = render(&mut app, 110, 30);
    save("pane-buttons-shared-line-110x30", &text);
    assert_eq!(pane_buttons(&app, bottom).len(), 4, "{text}");
    assert_eq!(text.matches('✕').count(), 2, "the focused pane and the one under the mouse: {text}");
    // One row higher the mouse is inside the top pane: the bottom one hides its buttons again.
    app.hover = Some((title.x + 20, title.y - 1));
    let text = render(&mut app, 110, 30);
    assert!(pane_buttons(&app, bottom).is_empty(), "{text}");
    app.run(Action::CloseTab);
}

/// The buttons of the pane under `pane` as drawn: (button, click target).
fn pane_buttons(app: &App, pane: u64) -> Vec<(noble::ui::PaneButton, ratatui::layout::Rect)> {
    app.hits
        .iter()
        .filter_map(|(r, h)| noble::ui::PaneButton::of_hit(h).filter(|(_, p)| *p == pane).map(|(b, _)| (b, *r)))
        .collect()
}

/// The title row's buttons form one bracketed cluster (`┤ ◫ ⊟ ⤢ ✕ ├`) on the focused pane and on
/// the pane under the mouse; other panes show a plain line there and have no click targets. A
/// hovered button lights up (close in the error color) and the status bar says what it does with
/// the shortcut for it.
#[test]
fn pane_buttons_show_on_focus_and_hover() {
    use noble::ui::PaneButton;
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("buttons".into()));
    app.run(Action::SplitRight);
    let (left, right) = (app.tabs[0].panes()[0], app.tabs[0].panes()[1]);
    assert_eq!(app.tabs[0].focus, right);
    app.hover = None;
    let (text, buf) = render_buf(&mut app, 110, 30);
    save("pane-buttons-110x30", &text);
    let th = app.theme.clone();
    let shown = pane_buttons(&app, right);
    let kinds: Vec<PaneButton> = shown.iter().map(|(b, _)| *b).collect();
    assert_eq!(kinds, [PaneButton::SplitRight, PaneButton::SplitDown, PaneButton::Zoom, PaneButton::Close]);
    assert!(pane_buttons(&app, left).is_empty(), "click targets on a pane that shows no buttons");
    let y = shown[0].1.y;
    let row: String = text.lines().nth(y as usize).unwrap().to_string();
    assert!(row.contains("─┤ ◫ ⊟ ⤢ ✕ ├─"), "{row}");
    assert_eq!(row.matches('✕').count(), 1, "the unfocused pane shows buttons: {row}");
    // Each target is " glyph" (close " ✕ "), drawn right where it is clicked, next to each other.
    for (i, (b, r)) in shown.iter().enumerate() {
        assert_eq!(buf[(r.x + 1, r.y)].symbol(), b.glyph(false));
        assert_eq!(buf[(r.x + 1, r.y)].fg, th.accent);
        assert_eq!(r.width, if *b == PaneButton::Close { 3 } else { 2 });
        if i > 0 {
            assert_eq!(shown[i - 1].1.right(), r.x);
        }
    }
    assert_eq!(buf[(shown[0].1.x - 1, y)].symbol(), "┤");
    assert_eq!(buf[(shown[3].1.right(), y)].symbol(), "├");

    // The mouse over the other pane shows its buttons, dim.
    app.hover = Some((5, 10));
    let (text, buf) = render_buf(&mut app, 110, 30);
    save("pane-buttons-hover-110x30", &text);
    let other = pane_buttons(&app, left);
    assert_eq!(other.len(), 4, "{text}");
    assert!(other.iter().all(|(_, r)| buf[(r.x + 1, r.y)].fg == th.dim));
    assert_eq!(text.lines().nth(y as usize).unwrap().matches('✕').count(), 2);

    // On a button: it lights up and the status bar says what it does and how to do it by key.
    let status = |text: &str| text.lines().last().unwrap().to_string();
    // It turns into a small inverted chip, readable in every theme (`theme_contrast_meets_wcag`
    // checks crit against bg).
    for (b, colors) in [(PaneButton::Close, (th.bg, th.crit)), (PaneButton::SplitRight, (th.on_accent, th.accent))] {
        let r = other.iter().find(|(k, _)| *k == b).unwrap().1;
        app.hover = Some((r.x, r.y));
        let (text, buf) = render_buf(&mut app, 110, 30);
        let cell = &buf[(r.x + 1, r.y)];
        assert_eq!((cell.fg, cell.bg), colors, "{b:?}");
        // The chip also covers the space after the glyph (fonts draw the glyph wide, spilling
        // right), but not the space before it.
        let side = &buf[(r.x + 2, r.y)];
        assert_eq!((side.fg, side.bg), colors, "{b:?} chip cell after");
        assert_ne!(buf[(r.x, r.y)].bg, colors.1, "{b:?} chip cell before");
        let key = app.keymap.term_hint(b.action(), false).unwrap();
        let want = format!("{} {} · {key}", b.glyph(false), b.what(false));
        assert!(status(&text).contains(&want), "{want:?} not in {:?}", status(&text));
        assert!(!status(&text).contains("tip:"));
    }
    save("pane-buttons-hover-close-110x30", &render(&mut app, 110, 30));
    // Not over a button: the usual hints.
    app.hover = Some((5, 10));
    assert!(!status(&render(&mut app, 110, 30)).contains("split right"));

    // A revealed button works: close the other pane.
    let close = pane_buttons(&app, left).into_iter().find(|(b, _)| *b == PaneButton::Close).unwrap().1;
    click(&mut app, close.x + 1, close.y);
    assert_eq!(app.tabs[0].panes(), vec![right]);
    // A single pane has no zoom button.
    app.hover = None;
    render(&mut app, 110, 30);
    assert!(pane_buttons(&app, right).iter().all(|(b, _)| *b != PaneButton::Zoom));
    app.run(Action::CloseTab);
}

/// A double click on a pane's title zooms it, another one puts it back; a single click only
/// focuses. A middle click on a tab closes it.
#[test]
fn double_click_title_zooms_and_middle_click_closes_tab() {
    use crossterm::event::{MouseButton, MouseEventKind};
    use noble::app::Hit;
    let mut app = demo_app(110, 30);
    app.cfg.terminal.tab_follows_cwd = false;
    app.new_tab(std::env::temp_dir(), None, Some("zoom".into()));
    app.run(Action::SplitRight);
    let left = app.tabs[0].panes()[0];
    render(&mut app, 110, 30);
    // The title text of the left pane (the narrow target on top of the row).
    let title = |app: &App| find_hit(app, |h| *h == Hit::PaneTitle(left)).unwrap();
    let t = title(&app);
    click(&mut app, t.x + 3, t.y);
    assert_eq!(app.tabs[0].focus, left);
    assert!(!app.tabs[0].zoomed, "a single click zoomed");
    click(&mut app, t.x + 3, t.y);
    assert!(app.tabs[0].zoomed, "a double click did not zoom");
    let text = render(&mut app, 110, 30);
    save("pane-title-double-click-zoom-110x30", &text);
    assert!(text.contains("ZOOM") && text.contains('◱'), "{text}");
    // A third click right after starts over; a new double click restores.
    click(&mut app, t.x + 3, t.y);
    assert!(app.tabs[0].zoomed, "a triple click undid the zoom");
    std::thread::sleep(Duration::from_millis(500));
    let t = title(&app);
    click(&mut app, t.x + 3, t.y);
    click(&mut app, t.x + 3, t.y);
    assert!(!app.tabs[0].zoomed, "a double click did not restore");

    // Middle click on a tab closes that tab (and does not start a drag).
    app.new_tab(std::env::temp_dir(), None, Some("second".into()));
    render(&mut app, 110, 30);
    assert_eq!(app.tabs.len(), 2);
    let tab = find_hit(&app, |h| *h == Hit::Tab(0)).unwrap();
    mouse(&mut app, MouseEventKind::Down(MouseButton::Middle), tab.x + 2, tab.y);
    mouse(&mut app, MouseEventKind::Up(MouseButton::Middle), tab.x + 2, tab.y);
    // The tab has split panes: the middle click asks first.
    assert!(matches!(app.overlay, Some(noble::app::Overlay::Confirm(_))));
    app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert_eq!(app.tabs.len(), 1);
    assert!(app.tab_title(0).starts_with("second"), "{}", app.tab_title(0));
    assert_eq!(app.view, View::Term(0));
    app.run(Action::CloseTab);
}

/// Title rows in split and grid layouts at every size: rows keep their width, the buttons stay
/// together inside their pane with a line cell before the cluster, the focused pane keeps close.
#[test]
fn pane_title_rows_fit_every_layout() {
    use noble::ui::PaneButton;
    for (layout, actions) in [
        ("split", vec![Action::SplitRight]),
        ("grid", vec![Action::SplitRight, Action::SplitDown, Action::FocusLeft, Action::SplitDown]),
    ] {
        let mut app = demo_app(160, 45);
        app.new_tab(std::env::temp_dir(), None, Some(layout.into()));
        for a in actions {
            app.run(a);
        }
        let focus = app.tabs[0].focus;
        app.panes.get_mut(&focus).unwrap().last_result = Some(noble::term::pane::CommandResult {
            code: Some(1),
            took: Duration::from_millis(12_300),
            at: std::time::Instant::now(),
        });
        app.hover = None;
        for (w, h) in [(160, 45), (110, 30), (60, 20), (30, 8)] {
            let (text, buf) = render_buf(&mut app, w, h);
            save(&format!("pane-title-{layout}-{w}x{h}"), &text);
            for line in text.lines() {
                assert_eq!(unicode_width::UnicodeWidthStr::width(line), w as usize, "{w}x{h}:\n{text}");
            }
            let body = app.body();
            for (id, tile) in app.tabs[0].root.layout(body).0 {
                let frame = noble::ui::pane_outline(tile, body).rect;
                let buttons = pane_buttons(&app, id);
                if id != focus {
                    assert!(buttons.is_empty(), "{layout} {w}x{h}: buttons on an unfocused pane");
                    continue;
                }
                if frame.width >= 11 {
                    let close = buttons.iter().any(|(b, _)| *b == PaneButton::Close);
                    assert!(close, "{layout} {w}x{h}: no close\n{text}");
                }
                for (i, (_, r)) in buttons.iter().enumerate() {
                    assert!(r.x > frame.x && r.right() < frame.right() && r.y == frame.y, "{layout} {w}x{h}");
                    assert!(i == 0 || buttons[i - 1].1.right() == r.x, "{layout} {w}x{h}: gap");
                }
                if let Some((_, first)) = buttons.first() {
                    let before = buf[(first.x - 2, first.y)].symbol().to_string();
                    assert!("─┬┴┼".contains(before.as_str()), "{layout} {w}x{h}: {before:?} at the cluster\n{text}");
                }
            }
        }
        app.run(Action::CloseTab);
    }
}

/// Double click selects a word, triple click the line; the pane menu offers Copy while a selection exists.
#[test]
fn double_and_triple_click_select_word_and_line() {
    use noble::app::{Hit, Overlay};
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("one".into()));
    let id = app.focused_pane().unwrap();
    // The shell's first prompt must be out before the line is written over the screen: a fast shell
    // (zsh without a config) would otherwise print it after, onto the same line.
    wait_idle(&mut app, id);
    app.panes[&id].parser().process(b"\x1b[2J\x1b[Hfoo ~/code/app bar");
    render(&mut app, 110, 30);
    let inner = find_hit_inner(&app).expect("pane");
    // Path characters stay in the word: "~/code/app" starts at column 4.
    click(&mut app, inner.x + 6, inner.y);
    click(&mut app, inner.x + 6, inner.y);
    assert_eq!(app.panes[&id].selected_text().as_deref(), Some("~/code/app"));
    click(&mut app, inner.x + 6, inner.y);
    assert_eq!(app.panes[&id].selected_text().as_deref(), Some("foo ~/code/app bar"));
    // The pane menu (title right click) starts with Copy.
    render(&mut app, 110, 30);
    let title = find_hit(&app, |h| matches!(h, Hit::PaneTitle(_))).expect("title");
    mouse(&mut app, crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Right), title.x + 2, title.y);
    let Some(Overlay::Menu(m)) = &app.overlay else { panic!("no menu") };
    assert_eq!(m.items[0].label, "Copy");
}

/// Launchers next to a pane: the pane menu and the palette offer "in split" (the focused pane's directory);
/// in a terminal the palette's launch entries target the focused pane ("here"), on Home the selected project.
#[test]
fn launcher_in_split_and_palette_targets() {
    use noble::app::{MenuCmd, PaletteCmd};
    let mut app = demo_app(120, 32);
    let probe = if cfg!(windows) { "cmd /c echo SPLIT_PROBE_OK" } else { "echo SPLIT_PROBE_OK" };
    app.launchers = vec![(
        noble::config::Launcher { key: "z".into(), name: "probe".into(), command: probe.into(), show: true },
        true,
    )];
    // Home: the entry names the selected project.
    let project = app.selected_project().map(|p| p.name.clone()).expect("demo project");
    let titles: Vec<String> = app.palette_items().into_iter().map(|i| i.title).collect();
    assert!(titles.contains(&format!("Launch probe in {project}")), "{titles:?}");
    assert!(!titles.iter().any(|t| t.contains("in split") || t.ends_with("here")), "{titles:?}");

    let dir = std::env::temp_dir();
    app.new_tab(dir.clone(), None, Some("host".into()));
    let pane = app.tabs[0].focus;
    // Terminal: here / in split, both aimed at the focused pane.
    let items = app.palette_items();
    let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
    assert!(titles.contains(&"Launch probe here") && titles.contains(&"Launch probe in split"), "{titles:?}");
    assert!(!titles.iter().any(|t| t.starts_with("Launch probe in ") && *t != "Launch probe in split"), "{titles:?}");
    let here = items.iter().find(|i| i.title == "Launch probe here").unwrap();
    assert_eq!(here.cmd, PaletteCmd::Launch { launcher: 0, path: app.focused_cwd().unwrap() });
    assert!(items.iter().any(|i| i.cmd == PaletteCmd::LaunchSplit(0)));

    // The pane menu offers it too, in one sub-list row that opens the launchers at the same place.
    app.open_pane_menu(pane, 10, 5);
    let rows = match &app.overlay {
        Some(noble::app::Overlay::Menu(m)) => {
            m.items.iter().filter(|it| it.label.starts_with("Launch")).map(|it| it.cmd.clone()).collect::<Vec<_>>()
        }
        _ => panic!("pane menu not open"),
    };
    assert_eq!(rows, vec![MenuCmd::LaunchMenu(pane)], "one launch row in the pane menu");
    let Some(noble::app::Overlay::Menu(m)) = &app.overlay else { panic!("pane menu not open") };
    assert!(m.items.iter().any(|it| it.label == "Launch in split ›"), "row label");
    app.run_menu(MenuCmd::LaunchMenu(pane));
    let Some(noble::app::Overlay::Menu(sub)) = &app.overlay else { panic!("launch sub-list not open") };
    assert_eq!((sub.x, sub.y), (10, 5), "the sub-list opens where the menu was");
    assert_eq!(sub.items.len(), 1);
    assert_eq!((sub.items[0].label.as_str(), &sub.items[0].cmd), ("probe", &MenuCmd::LaunchSplit(pane, 0)));
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.tabs.len(), 1);
    let panes = app.tabs[0].panes();
    assert_eq!(panes.len(), 2, "split expected");
    let new = app.tabs[0].focus;
    assert_ne!(new, pane);
    assert_eq!(app.panes[&new].launcher.as_deref(), Some(probe));
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        app.pump();
        if app.panes[&new].parser().screen().contents().lines().any(|l| l.trim() == "SPLIT_PROBE_OK") {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "launcher output never appeared");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Equalize, swap and move to a new tab: the pane menu offers them once a tab has two panes, and each
/// keeps the shells running where they went.
#[test]
fn pane_arranging_actions() {
    use noble::app::{MenuCmd, Overlay};
    let mut app = demo_app(120, 32);
    app.new_tab(std::env::temp_dir(), None, Some("arrange".into()));
    let first = app.tabs[0].focus;
    let labels = |app: &noble::app::App| match &app.overlay {
        Some(Overlay::Menu(m)) => m.items.iter().map(|i| i.label.clone()).collect::<Vec<_>>(),
        _ => panic!("pane menu not open"),
    };
    // One pane: nothing to arrange, so the menu leaves the rows out and the actions only say why.
    app.open_pane_menu(first, 10, 5);
    assert!(!labels(&app).iter().any(|l| l == "Equalize splits" || l == "Move to new tab"), "{:?}", labels(&app));
    app.overlay = None;
    for a in [Action::EqualizePanes, Action::SwapPane, Action::MovePaneToTab] {
        app.run(a);
    }
    assert_eq!((app.tabs.len(), app.tabs[0].panes(), app.tabs[0].focus), (1, vec![first], first));

    app.run(Action::SplitRight);
    let second = app.tabs[0].focus;
    app.run(Action::SplitDown);
    let third = app.tabs[0].focus;
    assert_eq!(app.tabs[0].panes(), vec![first, second, third]);
    app.open_pane_menu(third, 10, 5);
    let l = labels(&app);
    for want in ["Equalize splits", "Swap with next pane", "Move to new tab"] {
        assert!(l.iter().any(|x| x == want), "{want} missing: {l:?}");
    }
    app.overlay = None;

    // Equalize: a lopsided divider goes back to even halves, and a zoomed tab shows its layout.
    app.tabs[0].root.set_ratio(&[], 0.8);
    app.tabs[0].zoomed = true;
    app.run(Action::EqualizePanes);
    assert!(!app.tabs[0].zoomed);
    let body = app.body();
    let width = |app: &noble::app::App, id| {
        app.tabs[0].root.layout(body).0.into_iter().find(|(p, _)| *p == id).map(|(_, r)| r.width).unwrap()
    };
    assert!(width(&app, first).abs_diff(width(&app, second)) <= 1, "{} vs {}", width(&app, first), width(&app, second));

    // Swap: the last pane trades places with the first, and the focus goes along with it.
    app.run(Action::SwapPane);
    assert_eq!(app.tabs[0].panes(), vec![third, second, first]);
    assert_eq!(app.tabs[0].focus, third);

    // Move to a new tab, from the pane menu: the pane keeps its shell and gets a tab right after this one.
    app.run_menu(MenuCmd::PaneAction(second, Action::MovePaneToTab));
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.tabs[0].panes(), vec![third, first]);
    assert_eq!(app.tabs[1].panes(), vec![second]);
    assert_eq!(app.view, View::Term(1));
    assert!(app.panes.contains_key(&second), "the moved pane keeps running");
    render(&mut app, 120, 32);
    app.run(Action::CloseTab);
    app.run(Action::CloseTab);
    app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.tabs.is_empty());
}

/// Closing a tab with several panes asks first, idle or not; a single idle pane still closes at once.
#[test]
fn closing_a_tab_with_several_panes_asks_first() {
    use noble::app::Overlay;
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("three".into()));
    app.run(Action::SplitRight);
    app.run(Action::SplitDown);
    assert_eq!(app.tabs[0].panes().len(), 3);
    app.run(Action::CloseTab);
    let text = render(&mut app, 110, 30);
    assert!(text.contains("CLOSE TAB") && text.contains("close the tab and its 3 panes?"), "{text}");
    // No keeps the tab and all its panes.
    app.on_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
    assert!(app.overlay.is_none());
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.tabs[0].panes().len(), 3);
    // Yes closes the tab.
    app.run(Action::CloseTab);
    assert!(matches!(app.overlay, Some(Overlay::Confirm(_))));
    app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.tabs.is_empty());
    // A single idle pane closes at once.
    app.new_tab(std::env::temp_dir(), None, Some("one".into()));
    app.run(Action::CloseTab);
    assert!(app.overlay.is_none());
    assert!(app.tabs.is_empty());
}

/// Split tabs: the panes without the focus fade their text toward the pane background
/// (`terminal.dim_inactive`). A single pane, a zoomed pane and the setting turned off keep their
/// colors; an ANSI color the palette cannot blend (theme mode) gets the DIM attribute instead.
#[test]
fn inactive_panes_dim_only_in_a_split() {
    use ratatui::style::{Color, Modifier};
    // The (foreground, DIM) of every cell showing `ch`.
    let cells_of = |buf: &ratatui::buffer::Buffer, ch: &str| -> Vec<(Color, bool)> {
        let mut v = Vec::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let c = &buf[(x, y)];
                if c.symbol() == ch {
                    v.push((c.fg, c.modifier.contains(Modifier::DIM)));
                }
            }
        }
        v
    };
    let white = Color::Rgb(255, 255, 255);

    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("dim".into()));
    app.run(Action::SplitRight);
    assert_eq!(app.tabs[0].panes().len(), 2);
    settle_layout(&mut app, 110, 30);
    write_panes(&mut app, "\x1b[38;2;255;255;255m", "QQQQ");
    let buf = render_cells(&mut app, 110, 30);
    let qs = cells_of(&buf, "Q");
    assert_eq!(qs.len(), 8, "four glyphs per pane");
    let kept = qs.iter().filter(|(fg, _)| *fg == white).count();
    assert_eq!(kept, 4, "only the focused pane keeps its colors: {qs:?}");
    let faded: Vec<Color> = qs.iter().filter(|(fg, _)| *fg != white).map(|(fg, _)| *fg).collect();
    assert_eq!(faded.len(), 4);
    assert!(faded.iter().all(|c| *c == faded[0] && matches!(c, Color::Rgb(..))), "{faded:?}");

    // Off: both panes keep the text color.
    app.cfg.terminal.dim_inactive = false;
    let buf = render_cells(&mut app, 110, 30);
    let qs = cells_of(&buf, "Q");
    assert!(qs.iter().all(|(fg, _)| *fg == white), "{qs:?}");
    app.cfg.terminal.dim_inactive = true;

    // Theme mode: ANSI red is left to the outer terminal, so the unfocused pane gets DIM.
    app.cfg.terminal.colors = "theme".into();
    write_panes(&mut app, "\x1b[31m", "¤¤¤¤");
    let buf = render_cells(&mut app, 110, 30);
    let rs = cells_of(&buf, "¤");
    assert_eq!(rs.len(), 8);
    assert_eq!(rs.iter().filter(|(_, dim)| *dim).count(), 4, "{rs:?}");
    app.cfg.terminal.colors = "windows-terminal".into();

    // Zoomed: one pane is on screen, and it has the focus.
    app.run(Action::Zoom);
    assert!(app.tabs[0].zoomed);
    settle_layout(&mut app, 110, 30);
    write_panes(&mut app, "\x1b[38;2;255;255;255m", "QQQQ");
    let buf = render_cells(&mut app, 110, 30);
    let qs = cells_of(&buf, "Q");
    assert_eq!(qs.len(), 4);
    assert!(qs.iter().all(|(fg, _)| *fg == white), "{qs:?}");

    // A single pane is never dimmed.
    let mut single = demo_app(110, 30);
    single.new_tab(std::env::temp_dir(), None, Some("one".into()));
    settle_layout(&mut single, 110, 30);
    write_panes(&mut single, "\x1b[38;2;255;255;255m", "QQQQ");
    let buf = render_cells(&mut single, 110, 30);
    let qs = cells_of(&buf, "Q");
    assert!(qs.len() == 4 && qs.iter().all(|(fg, _)| *fg == white), "{qs:?}");
}

/// Draws one frame (which resizes the panes of the first tab if the layout changed), then waits until each
/// pane's shell has gone quiet. A resize makes the shell redraw its prompt, and the redraw erases whatever
/// `write_panes` has put on the first line by then. Writing only after the shell settled keeps the glyphs.
fn settle_layout(app: &mut App, w: u16, h: u16) {
    render_cells(app, w, h);
    for id in app.tabs[0].panes() {
        wait_idle(app, id);
    }
}

/// Clears every pane of the first tab and writes `glyphs` (in the `sgr` style) at its top left.
fn write_panes(app: &mut App, sgr: &str, glyphs: &str) {
    let bytes = format!("\x1b[2J\x1b[H{sgr}{glyphs}");
    for id in app.tabs[0].panes() {
        app.panes[&id].parser().process(bytes.as_bytes());
    }
}
