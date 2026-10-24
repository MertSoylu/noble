//! Focus mode: a terminal tab hides the top and bottom bars, the bars come back over the content
//! (prefix armed, pointer dwell) and the alert chip keeps the other tabs' markers in sight.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEventKind};
use noble::app::{App, Hit, SettingItem, SettingKey, View};
use noble::keys::Action;
use noble::term::TabAlert;
use ratatui::layout::Rect;
use std::time::Duration;

use super::common::*;

fn ctrl_a() -> KeyEvent {
    KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)
}

fn plain(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

fn last_line(text: &str) -> &str {
    text.lines().last().unwrap_or_default()
}

/// The brand of the top bar, and the palette hint of the status bar, are what the bars are known by.
fn has_top_bar(text: &str) -> bool {
    first_line(text).contains("NOBLE")
}

fn has_status_bar(text: &str) -> bool {
    last_line(text).contains("commands")
}

/// The inner height of the first pane (its PTY rows follow it).
fn pane_rows(app: &App) -> u16 {
    app.panes[&app.tabs[0].focus].size.0
}

#[test]
fn focus_mode_hides_the_bars_and_gives_the_panes_two_rows() {
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("focus".into()));
    let text = render(&mut app, 110, 30);
    assert!(has_top_bar(&text) && has_status_bar(&text), "{text}");
    let rows = pane_rows(&app);
    assert_eq!(app.body(), Rect::new(0, 1, 110, 28));

    app.run(Action::FocusMode);
    assert!(app.focus_mode);
    assert_eq!(app.body(), Rect::new(0, 0, 110, 30), "the terminal body is the whole screen");
    let text = render(&mut app, 110, 30);
    save("focus-mode-110x30", &text);
    assert!(!has_top_bar(&text), "{text}");
    assert!(!has_status_bar(&text), "{text}");
    assert!(!last_line(&text).contains("tip:"), "{text}");
    assert_eq!(pane_rows(&app), rows + 2, "the PTY grows by the two bar rows");
    let inner = find_hit_inner(&app).expect("pane");
    assert_eq!(inner.y, 1, "the pane frame starts on row 0");

    app.run(Action::FocusMode);
    assert!(!app.focus_mode);
    let text = render(&mut app, 110, 30);
    assert!(has_top_bar(&text) && has_status_bar(&text));
    assert_eq!(pane_rows(&app), rows, "toggled back: the PTY shrinks again");
}

#[test]
fn focus_mode_prefix_key_toggles_it_and_search_moved_to_slash() {
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("keys".into()));
    assert_eq!(app.keymap.prefix_key(Action::FocusMode).as_deref(), Some("f"));
    assert_eq!(app.keymap.prefix_key(Action::Search).as_deref(), Some("/"));

    app.on_key(ctrl_a());
    app.on_key(plain('f'));
    assert!(app.focus_mode, "prefix f turns focus mode on");
    assert!(app.search.is_none(), "prefix f no longer opens the search");
    app.on_key(ctrl_a());
    app.on_key(plain('/'));
    assert!(app.search.is_some(), "prefix / opens the scrollback search");
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.on_key(ctrl_a());
    app.on_key(plain('f'));
    assert!(!app.focus_mode);

    // Listed in the palette on terminal tabs only.
    assert!(app.palette_items().iter().any(|i| i.title == "Toggle Focus Mode"));
    app.run(Action::Bridge);
    assert!(!app.palette_items().iter().any(|i| i.title == "Toggle Focus Mode"));
}

#[test]
fn focus_mode_on_home_and_settings_keeps_the_bars() {
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("home".into()));
    app.run(Action::FocusMode);
    app.run(Action::Bridge);
    assert_eq!(app.view, View::Bridge);
    assert_eq!(app.body(), Rect::new(0, 1, 110, 28), "Home keeps its bars in focus mode");
    let text = render(&mut app, 110, 30);
    assert!(has_top_bar(&text) && has_status_bar(&text), "{text}");

    app.run(Action::Settings);
    let text = render(&mut app, 110, 30);
    assert!(has_top_bar(&text), "{text}");
}

#[test]
fn prefix_armed_reveals_both_bars_without_resizing() {
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("armed".into()));
    app.run(Action::FocusMode);
    let rows = pane_rows(&app);
    let text = render(&mut app, 110, 30);
    assert!(!has_top_bar(&text) && !has_status_bar(&text));

    app.on_key(ctrl_a());
    assert!(app.prefix_armed);
    let text = render(&mut app, 110, 30);
    save("focus-mode-prefix-110x30", &text);
    assert!(has_top_bar(&text), "{text}");
    assert!(last_line(&text).contains("PREFIX"), "{text}");
    assert_eq!(pane_rows(&app), rows + 2, "the bars are drawn over the panes, not around them");

    // The next key ends the prefix, and the bars go with it.
    app.on_key(plain('y'));
    assert!(!app.prefix_armed);
    let text = render(&mut app, 110, 30);
    assert!(!has_top_bar(&text) && !has_status_bar(&text), "{text}");
}

#[test]
fn pointer_dwell_on_an_edge_reveals_its_bar() {
    use crossterm::event::MouseEventKind::Moved;
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("dwell".into()));
    app.run(Action::FocusMode);
    render(&mut app, 110, 30);

    // The top row: nothing until the pointer has rested there for the dwell.
    mouse(&mut app, Moved, 40, 0);
    let text = render(&mut app, 110, 30);
    assert!(!has_top_bar(&text), "no reveal before the dwell");
    assert!(app.redraw_after().is_some(), "a wake-up is scheduled for the dwell");
    std::thread::sleep(Duration::from_millis(450));
    let text = render(&mut app, 110, 30);
    save("focus-mode-dwell-top-110x30", &text);
    assert!(has_top_bar(&text), "{text}");
    assert!(!has_status_bar(&text), "only the edge the pointer rests on");

    // Leaving the row hides it again at once.
    mouse(&mut app, Moved, 40, 10);
    let text = render(&mut app, 110, 30);
    assert!(!has_top_bar(&text), "{text}");

    // The bottom row works the same way.
    mouse(&mut app, Moved, 40, 29);
    std::thread::sleep(Duration::from_millis(450));
    let text = render(&mut app, 110, 30);
    save("focus-mode-dwell-bottom-110x30", &text);
    assert!(has_status_bar(&text), "{text}");
    assert!(!has_top_bar(&text), "{text}");
    mouse(&mut app, Moved, 40, 10);
    let text = render(&mut app, 110, 30);
    assert!(!has_status_bar(&text), "{text}");
}

#[test]
fn revealed_top_bar_takes_the_clicks_on_its_row() {
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("clicks".into()));
    app.new_tab(std::env::temp_dir(), None, Some("second".into()));
    app.run(Action::FocusMode);
    assert_eq!(app.view, View::Term(1));
    mouse(&mut app, MouseEventKind::Moved, 40, 0);
    std::thread::sleep(Duration::from_millis(450));
    render(&mut app, 110, 30);
    let home = find_hit(&app, |h| *h == Hit::TabBridge).expect("Home button in the revealed bar");
    click(&mut app, home.x + 1, home.y);
    assert_eq!(app.view, View::Bridge);
}

/// The hit the pointer would get on a cell, topmost first (as `App::hit_at`).
fn hit_on(app: &App, x: u16, y: u16) -> Option<Hit> {
    let pos = ratatui::layout::Position { x, y };
    app.hits.iter().rev().find(|(r, _)| r.contains(pos)).map(|(_, h)| h.clone())
}

#[test]
fn revealed_bars_block_clicks_on_their_empty_cells() {
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("blocker".into()));
    app.run(Action::FocusMode);
    let view = app.view;
    // Both edges: the pointer rests on the top row, then the bottom row.
    for row in [0, 29] {
        mouse(&mut app, MouseEventKind::Moved, 40, row);
        std::thread::sleep(Duration::from_millis(450));
        render(&mut app, 110, 30);
        let empty = (0..110).find(|&x| hit_on(&app, x, row) == Some(Hit::Bar));
        let empty = empty.unwrap_or_else(|| panic!("row {row}: a cell of the revealed bar with no button"));
        // A left and a right click on an empty cell: nothing happens, and no pane menu opens under the bar.
        click(&mut app, empty, row);
        mouse(&mut app, MouseEventKind::Down(crossterm::event::MouseButton::Right), empty, row);
        mouse(&mut app, MouseEventKind::Up(crossterm::event::MouseButton::Right), empty, row);
        assert_eq!(app.view, view, "row {row}: a click on the bar changes nothing");
        assert!(app.overlay.is_none(), "row {row}: no menu opens from the bar");
        mouse(&mut app, MouseEventKind::Moved, 40, 10);
    }
}

#[test]
fn alert_chip_shows_a_background_marker_and_switches_to_it() {
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("a".into()));
    app.new_tab(std::env::temp_dir(), None, Some("b".into()));
    app.new_tab(std::env::temp_dir(), None, Some("c".into()));
    app.tabs[0].raise(TabAlert::Failed);
    app.tabs[1].raise(TabAlert::Notice);
    app.run(Action::FocusMode);
    assert_eq!(app.view, View::Term(2));
    let text = render(&mut app, 110, 30);
    save("focus-mode-alert-110x30", &text);
    // Two background tabs have a marker; the most urgent one (◆ notice on tab 2) is the glyph.
    assert!(last_line(&text).contains(" ◆ 2 "), "{text}");
    let chip = find_hit(&app, |h| matches!(h, Hit::FocusAlert(_))).expect("alert chip");
    assert_eq!(chip.y, 29, "on the bottom row");
    assert_eq!(chip.right(), 109, "one cell from the corner");
    assert_eq!(app.hits.iter().filter(|(_, h)| matches!(h, Hit::FocusAlert(_))).count(), 1);
    assert_eq!(find_hit(&app, |h| *h == Hit::FocusAlert(1)), Some(chip), "the notice wins over the failure");

    click(&mut app, chip.x + 1, chip.y);
    assert_eq!(app.view, View::Term(1), "the click goes to the tab with the marker");
    let text = render(&mut app, 110, 30);
    assert!(!last_line(&text).contains(" ◆ "), "the viewed tab has no chip: {text}");
}

#[test]
fn alert_chip_is_hidden_when_a_bar_is_revealed_and_when_too_narrow() {
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("a".into()));
    app.new_tab(std::env::temp_dir(), None, Some("b".into()));
    app.tabs[0].raise(TabAlert::Notice);
    app.run(Action::FocusMode);
    render(&mut app, 110, 30);
    assert!(find_hit(&app, |h| matches!(h, Hit::FocusAlert(_))).is_some());
    app.on_key(ctrl_a());
    render(&mut app, 110, 30);
    assert!(find_hit(&app, |h| matches!(h, Hit::FocusAlert(_))).is_none(), "the status bar is revealed");
    app.on_key(plain('y'));

    // Not focus mode: the markers stay in the top bar.
    app.run(Action::FocusMode);
    render(&mut app, 110, 30);
    assert!(find_hit(&app, |h| matches!(h, Hit::FocusAlert(_))).is_none());
    app.run(Action::FocusMode);

    // Too narrow for the chip.
    app.run(Action::FocusMode);
    render(&mut app, 18, 8);
    assert!(find_hit(&app, |h| matches!(h, Hit::FocusAlert(_))).is_none());
}

#[test]
fn settings_row_and_config_reload_drive_the_flag() {
    let mut app = demo_app(110, 30);
    app.open_settings();
    let item = SettingItem::Setting(SettingKey::FocusMode);
    assert!(app.settings_items().contains(&item), "the row is in Settings");
    assert_eq!(SettingKey::FocusMode.label(), "Focus mode");
    assert!(!app.setting_on(SettingKey::FocusMode));
    app.activate_setting(item, 1);
    assert!(app.cfg.terminal.focus_mode, "the config value is written");
    assert!(app.focus_mode, "and the running flag follows");
    assert!(app.setting_on(SettingKey::FocusMode));
    app.activate_setting(item, 1);
    assert!(!app.cfg.terminal.focus_mode && !app.focus_mode);

    // A changed value in the file sets the running flag; an unchanged file keeps a toggle made since.
    let mut c = app.cfg.clone();
    c.terminal.focus_mode = true;
    app.apply_config(c);
    assert!(app.focus_mode, "the file's new value applies");
    app.new_tab(std::env::temp_dir(), None, Some("reload".into()));
    app.run(Action::FocusMode);
    assert!(!app.focus_mode);
    let same = app.cfg.clone();
    app.apply_config(same);
    assert!(!app.focus_mode, "an unchanged file does not undo the toggle");
}

#[test]
fn focus_mode_draws_at_every_size() {
    for (w, h) in SIZES {
        let mut app = demo_app(w, h);
        app.new_tab(std::env::temp_dir(), None, Some("sizes".into()));
        app.new_tab(std::env::temp_dir(), None, Some("other".into()));
        app.tabs[0].raise(TabAlert::Notice);
        app.run(Action::FocusMode);
        let text = render(&mut app, w, h);
        save(&format!("focus-mode-{w}x{h}"), &text);
        app.on_key(ctrl_a());
        let text = render(&mut app, w, h);
        save(&format!("focus-mode-prefix-{w}x{h}"), &text);
        assert!(has_top_bar(&text) || h < 3, "{w}x{h}: {text}");
    }
}
