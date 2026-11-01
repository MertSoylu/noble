//! Focus mode: a terminal tab hides the top and bottom bars. The top bar comes back over the content on a pointer
//! rest on the top row, the bottom bar only while the prefix is armed, and the alert chip on the top row keeps the
//! other tabs' markers in sight.

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
fn focus_mode_hides_the_bars_and_gives_the_panes_the_whole_screen() {
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
    assert_eq!(pane_rows(&app), rows + 3, "the PTY grows by the two bar rows and the bottom frame line");
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
    assert_eq!(pane_rows(&app), rows + 3, "the bars are drawn over the panes, not around them");

    // The next key ends the prefix, and the bars go with it.
    app.on_key(plain('y'));
    assert!(!app.prefix_armed);
    let text = render(&mut app, 110, 30);
    assert!(!has_top_bar(&text) && !has_status_bar(&text), "{text}");
}

#[test]
fn pointer_dwell_on_the_top_row_reveals_the_top_bar_only() {
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
    assert!(!has_status_bar(&text), "a rest on the top row reveals the top bar only");
    assert_eq!(app.focus_reveal(), (true, false));

    // Leaving the row hides it again at once.
    mouse(&mut app, Moved, 40, 10);
    let text = render(&mut app, 110, 30);
    assert!(!has_top_bar(&text), "{text}");

    // The last row reveals nothing, however long the pointer rests there: the terminal reaches that row.
    mouse(&mut app, Moved, 40, 29);
    std::thread::sleep(Duration::from_millis(450));
    let text = render(&mut app, 110, 30);
    save("focus-mode-dwell-bottom-110x30", &text);
    assert!(!has_status_bar(&text) && !has_top_bar(&text), "{text}");
    assert_eq!(app.focus_reveal(), (false, false), "no dwell starts on the last row");

    // Moving from the top row straight onto the last row hides the top bar at once.
    mouse(&mut app, Moved, 40, 0);
    std::thread::sleep(Duration::from_millis(450));
    assert!(has_top_bar(&render(&mut app, 110, 30)));
    mouse(&mut app, Moved, 40, 29);
    let text = render(&mut app, 110, 30);
    assert!(!has_top_bar(&text) && !has_status_bar(&text), "{text}");
    std::thread::sleep(Duration::from_millis(450));
    let text = render(&mut app, 110, 30);
    assert!(!has_top_bar(&text) && !has_status_bar(&text), "{text}");
}

/// The bottom bar is the prefix's: it comes back only while the prefix is armed, even after a rest on the last row.
/// Both bars are drawn over the content, so the pane keeps its rows, and the next key ends the prefix and the bars with it.
#[test]
fn the_bottom_bar_comes_back_only_while_the_prefix_is_armed() {
    use crossterm::event::MouseEventKind::Moved;
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("armed-bottom".into()));
    app.run(Action::FocusMode);
    render(&mut app, 110, 30);
    let rows = pane_rows(&app);

    // A rest on the last row shows nothing.
    mouse(&mut app, Moved, 40, 29);
    std::thread::sleep(Duration::from_millis(450));
    let text = render(&mut app, 110, 30);
    assert!(!has_status_bar(&text) && !has_top_bar(&text), "{text}");

    // Prefix armed: both bars, no resize.
    app.on_key(ctrl_a());
    let text = render(&mut app, 110, 30);
    assert!(has_top_bar(&text) && last_line(&text).contains("PREFIX"), "{text}");
    assert_eq!(app.focus_reveal(), (true, true));
    assert_eq!(pane_rows(&app), rows, "the bars are drawn over the panes, not around them");

    // The next key ends the prefix. The pointer still rests on the last row, and that brings nothing back.
    app.on_key(plain('y'));
    let text = render(&mut app, 110, 30);
    assert!(!has_status_bar(&text) && !has_top_bar(&text), "{text}");
    assert_eq!(app.focus_reveal(), (false, false));
}

#[test]
fn resting_on_a_pane_button_does_not_reveal_the_top_bar() {
    use crossterm::event::MouseEventKind::Moved;
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("buttons".into()));
    app.run(Action::FocusMode);
    render(&mut app, 110, 30);
    let button = find_hit(&app, |h| matches!(h, Hit::PaneSplit { .. } | Hit::PaneZoom(_) | Hit::PaneClose(_)))
        .expect("a pane button on the top row");
    assert_eq!(button.y, 0, "the pane title row is the top row in focus mode");

    // Away from the buttons the dwell still works; moving onto a button cancels it and the buttons stay usable.
    mouse(&mut app, Moved, 2, 0);
    std::thread::sleep(Duration::from_millis(450));
    assert!(has_top_bar(&render(&mut app, 110, 30)));
    mouse(&mut app, Moved, button.x, 0);
    let text = render(&mut app, 110, 30);
    assert!(!has_top_bar(&text), "{text}");
    std::thread::sleep(Duration::from_millis(450));
    let text = render(&mut app, 110, 30);
    assert!(!has_top_bar(&text), "the bar stays away while the pointer rests on a button\n{text}");
    assert!(find_hit(&app, |h| matches!(h, Hit::PaneClose(_))).is_some());
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
    // The top bar comes by the pointer's rest on the top row, the bottom bar by the prefix.
    for row in [0, 29] {
        if row == 0 {
            mouse(&mut app, MouseEventKind::Moved, 40, 0);
            std::thread::sleep(Duration::from_millis(450));
        } else {
            app.on_key(ctrl_a());
        }
        render(&mut app, 110, 30);
        let empty = (0..110).find(|&x| hit_on(&app, x, row) == Some(Hit::Bar));
        let empty = empty.unwrap_or_else(|| panic!("row {row}: a cell of the revealed bar with no button"));
        // A right click on an empty cell: no pane menu opens under the bar. Any click ends a pending prefix, so the
        // bottom bar is armed again for the left click.
        mouse(&mut app, MouseEventKind::Down(crossterm::event::MouseButton::Right), empty, row);
        mouse(&mut app, MouseEventKind::Up(crossterm::event::MouseButton::Right), empty, row);
        assert!(app.overlay.is_none(), "row {row}: no menu opens from the bar");
        if row == 29 {
            app.on_key(ctrl_a());
        }
        click(&mut app, empty, row);
        assert_eq!(app.view, view, "row {row}: a click on the bar changes nothing");
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
    assert!(first_line(&text).contains(" ◆ 2 "), "{text}");
    let chip = find_hit(&app, |h| matches!(h, Hit::FocusAlert(_))).expect("alert chip");
    assert_eq!(chip.y, 0, "on the top row, the pane title row");
    // The focused pane shows its buttons on that row, at the right end: the chip stays left of them.
    let split = find_hit(&app, |h| matches!(h, Hit::PaneSplit { .. })).expect("the focused pane's buttons");
    assert_eq!(split.y, 0);
    assert!(chip.right() < split.x, "the chip is left of the buttons: chip {chip:?}, split {split:?}\n{text}");
    assert_eq!(app.hits.iter().filter(|(_, h)| matches!(h, Hit::FocusAlert(_))).count(), 1);
    assert_eq!(find_hit(&app, |h| *h == Hit::FocusAlert(1)), Some(chip), "the notice wins over the failure");

    click(&mut app, chip.x + 1, chip.y);
    assert_eq!(app.view, View::Term(1), "the click goes to the tab with the marker");
    let text = render(&mut app, 110, 30);
    assert!(!first_line(&text).contains(" ◆ "), "the viewed tab has no chip: {text}");
}

/// The chip covers no click target of the top row: each button of the focused pane and of the pane under the mouse
/// stays the target of every cell it spans, so a click on it still works. Checked at every size, with one pane and with
/// two panes side by side (the pointer on the left pane's title shows its buttons too).
#[test]
fn alert_chip_on_the_top_row_stays_clear_of_the_pane_buttons_at_every_size() {
    for (w, h) in SIZES {
        for split in [false, true] {
            let mut app = demo_app(w, h);
            app.new_tab(std::env::temp_dir(), None, Some("a".into()));
            app.new_tab(std::env::temp_dir(), None, Some("b".into()));
            app.tabs[0].raise(TabAlert::Failed);
            app.run(Action::FocusMode);
            if split {
                app.run(Action::SplitRight);
                mouse(&mut app, MouseEventKind::Moved, 2, 0);
            }
            let text = render(&mut app, w, h);
            let case = format!("{w}x{h} split {split}");
            save(&format!("focus-mode-chip-{w}x{h}{}", if split { "-split" } else { "" }), &text);
            let chip = find_hit(&app, |h| matches!(h, Hit::FocusAlert(_)));
            let chip = chip.unwrap_or_else(|| panic!("{case}: no alert chip:\n{text}"));
            assert_eq!(chip.y, 0, "{case}: the chip is on the top row");
            let buttons: Vec<(Rect, Hit)> = app
                .hits
                .iter()
                .filter(|(_, h)| matches!(h, Hit::PaneSplit { .. } | Hit::PaneZoom(_) | Hit::PaneClose(_)))
                .cloned()
                .collect();
            assert!(!buttons.is_empty(), "{case}: no pane button is drawn");
            for (r, hit) in &buttons {
                assert_eq!(r.y, 0, "{case}: a button is on the top row");
                assert!(chip.right() <= r.x || chip.x >= r.right(), "{case}: the chip covers a button {r:?}:\n{text}");
                for x in r.x..r.right() {
                    assert_eq!(hit_on(&app, x, 0).as_ref(), Some(hit), "{case}: column {x} is not the button's");
                }
            }
        }
    }
}

/// A click on a pane button beside the chip still does what the button does.
#[test]
fn a_click_on_a_pane_button_beside_the_chip_still_splits() {
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("a".into()));
    app.new_tab(std::env::temp_dir(), None, Some("b".into()));
    app.tabs[0].raise(TabAlert::Notice);
    app.run(Action::FocusMode);
    render(&mut app, 110, 30);
    assert!(find_hit(&app, |h| matches!(h, Hit::FocusAlert(_))).is_some(), "the chip is shown");
    let split = find_hit(&app, |h| matches!(h, Hit::PaneSplit { .. })).expect("split button");
    assert_eq!(split.y, 0);
    assert_eq!(app.tabs[1].panes().len(), 1);
    click(&mut app, split.x, split.y);
    assert_eq!(app.tabs[1].panes().len(), 2, "the split button split the focused pane");
}

/// In focus mode a zoom in grows the pane to the whole screen without drawing a bottom line on the way: a line drawn at
/// the bottom edge of the growing frame would pop away when the pane reaches the last row. A zoom out starts at the full
/// screen, where the last row is the terminal's, and keeps that row open from its first frame.
#[test]
fn zoom_animation_in_focus_mode_draws_no_bottom_line_while_the_pane_grows() {
    let mut app = demo_app(110, 30);
    // demo_app turns the animations off; the demo laptop is on battery, so the setting for battery is needed too.
    app.cfg.general.animations = true;
    app.cfg.general.animations_on_battery = true;
    app.new_tab(std::env::temp_dir(), None, Some("zoom".into()));
    app.run(Action::SplitDown);
    app.run(Action::FocusMode);
    render(&mut app, 110, 30);
    let top = app
        .hits
        .iter()
        .find_map(|(_, h)| match h {
            Hit::Pane { pane, inner } if inner.y == 1 => Some(*pane),
            _ => None,
        })
        .expect("the top pane");
    app.toggle_zoom(0, top);
    let (from, to) = {
        let z = app.zoom_anim.as_ref().expect("the zoom animates");
        (z.from, z.to)
    };
    // The bottom line of the animated frame sits on the row of its bottom edge, which is `lerp_rect`'s rounding of the
    // animation's progress. Progress is read just before and after the draw, so the rows of a line are within these two.
    let bottom_at = |t: f64| {
        let lerp = |a: u16, b: u16| (f64::from(a) + (f64::from(b) - f64::from(a)) * t).round() as u16;
        lerp(from.y, to.y) + lerp(from.height, to.height)
    };
    for frac in [0.1, 0.4, 0.7, 0.95] {
        let started = std::time::Instant::now() - noble::app::ZOOM_DURATION.mul_f64(frac);
        app.zoom_anim.as_mut().expect("the zoom animates").started = started;
        let t_before = noble::app::ease(started, noble::app::ZOOM_DURATION);
        let buf = render_cells(&mut app, 110, 30);
        let t_after = noble::app::ease(started, noble::app::ZOOM_DURATION);
        for y in bottom_at(t_before).saturating_sub(1)..=bottom_at(t_after).saturating_sub(1).min(29) {
            for x in 0..110 {
                let s = buf[(x, y)].symbol();
                assert!(!matches!(s, "─" | "╰" | "╯"), "frac {frac}: a bottom line at row {y}, column {x}");
            }
        }
    }

    // Zoom out: the animation starts at the full screen, where the last row is the terminal's. A start time in the
    // future reads as no progress yet (`Instant::elapsed` is zero), so the first frame is the full screen whatever the
    // timing of the draw.
    app.zoom_anim = None;
    app.toggle_zoom(0, top);
    app.zoom_anim.as_mut().expect("the zoom out animates").started = std::time::Instant::now() + Duration::from_secs(1);
    let buf = render_cells(&mut app, 110, 30);
    for x in 0..110 {
        let s = buf[(x, 29)].symbol();
        assert!(!matches!(s, "─" | "╰" | "╯"), "the zoom out drew a bottom line on the last row at column {x}");
    }
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

/// Focus mode gives the terminal the whole screen: the pane's content reaches the screen's last row and no frame line is
/// drawn there. The title row on top stays, it carries the pane's title and buttons. With two panes stacked, the upper
/// one keeps its bottom line and only the lower one reaches the last row.
#[test]
fn focus_mode_gives_the_terminal_the_whole_screen() {
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("whole".into()));
    app.run(Action::FocusMode);
    let text = render(&mut app, 110, 30);
    save("focus-mode-whole-110x30", &text);
    let inner = find_hit_inner(&app).expect("pane");
    assert_eq!(inner.bottom(), 30, "the content ends on the screen's last row:\n{text}");
    assert!(!last_line(&text).contains(['─', '╰', '╯']), "no frame line on the last row:\n{text}");

    app.run(Action::SplitDown);
    let text = render(&mut app, 110, 30);
    save("focus-mode-whole-split-110x30", &text);
    let inners: Vec<Rect> = app
        .hits
        .iter()
        .filter_map(|(_, h)| match h {
            Hit::Pane { inner, .. } => Some(*inner),
            _ => None,
        })
        .collect();
    assert_eq!(inners.len(), 2, "{text}");
    assert_eq!(inners.iter().filter(|r| r.bottom() == 30).count(), 1, "one pane reaches the last row:\n{text}");
    assert!(!last_line(&text).contains('─'), "the lower pane has no bottom line:\n{text}");
}

/// The program of `full_screen_keeps_the_last_row_on_the_last_usable_row`: it enters the alternate screen and paints every
/// row of the screen with its number, the last one with the size it reads (`LAST<rows>`), again at each size change, as
/// a full-screen program does on a resize.
#[test]
#[ignore = "a child program of full_screen_keeps_the_last_row_on_the_last_usable_row"]
fn full_screen_child() {
    use std::io::Write as _;
    let mut out = std::io::stdout();
    write!(out, "\x1b[?1049h").unwrap();
    out.flush().unwrap();
    let mut last = None;
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_secs(120) {
        let size = crossterm::terminal::size().ok();
        if let Some((cols, rows)) = size.filter(|s| Some(*s) != last) {
            write!(out, "\x1b[?2026h\x1b[H\x1b[2J").unwrap();
            for r in 1..rows {
                write!(out, "\x1b[{r};1HR{r:03}").unwrap();
            }
            write!(out, "\x1b[{rows};1HLAST{rows:03} c{cols}\x1b[?2026l").unwrap();
            out.flush().unwrap();
            last = size;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The inner area of pane `id` as it was last drawn.
fn pane_inner(app: &App, id: noble::term::layout::PaneId) -> Option<Rect> {
    app.hits.iter().find_map(|(_, h)| match h {
        Hit::Pane { pane, inner } if *pane == id => Some(*inner),
        _ => None,
    })
}

/// The rows drawn inside pane `id` at `w` x `h`, one string per row, trailing blanks removed.
fn drawn_pane_rows(app: &mut App, id: noble::term::layout::PaneId, w: u16, h: u16) -> Vec<String> {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use unicode_width::UnicodeWidthStr;
    app.size = (w, h);
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| noble::ui::draw(f, app)).unwrap();
    let inner = pane_inner(app, id).expect("pane inner");
    let buf = term.backend().buffer().clone();
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

/// A full-screen program's last row is the pane's last usable row, in the normal layout and in focus mode, at every size
/// and after each toggle and resize. The pane's emulator has as many rows as are drawn. In the normal layout the
/// program's last row is drawn right above the pane's bottom border; in focus mode the pane has no bottom border, so
/// the program's last row is the screen's last row.
#[test]
fn full_screen_keeps_the_last_row_on_the_last_usable_row() {
    use noble::term::pane::ShellSpec;
    let exe = std::env::current_exe().expect("the test binary").display().to_string();
    let mut app = demo_app(110, 30);
    app.shell = ShellSpec::new(exe, vec!["--ignored".into(), "--nocapture".into(), "full_screen_child".into()]);
    app.run(Action::NewTab);
    let id = app.tabs[0].focus;
    let mut focus = false;
    for (w, h) in SIZES {
        for want in [false, true] {
            if want != focus {
                app.run(Action::FocusMode);
                // The toast of the toggle sits on the row above the bottom bar, where the normal layout's content ends.
                app.toasts.clear();
                focus = want;
            }
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            let drawn = loop {
                app.pump();
                app.tick();
                let drawn = drawn_pane_rows(&mut app, id, w, h);
                let inner = pane_inner(&app, id).expect("pane inner");
                let emu = app.panes[&id].size.0;
                let shown = drawn.last().map(String::as_str).unwrap_or_default();
                if usize::from(inner.height) == drawn.len()
                    && drawn.len() == usize::from(emu)
                    && shown.starts_with(&format!("LAST{emu:03} "))
                {
                    break drawn;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "{w}x{h} focus {want}: the program's frame never drawn:\n{}",
                    drawn.join("\n")
                );
                std::thread::sleep(Duration::from_millis(10));
            };
            let inner = pane_inner(&app, id).expect("pane inner");
            // Focus mode: the content reaches the screen's last row. Normal layout: the content ends above the bottom border.
            let last = if focus { app.body().bottom() } else { app.body().bottom() - 1 };
            assert_eq!(
                inner.bottom(),
                last,
                "{w}x{h} focus {focus}: the content's last row, drawn rows {}",
                drawn.len()
            );
        }
    }
    app.run(Action::CloseTab);
    app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
}

/// The shell case of the same question, and the known Windows limit behind it. ConPTY repaints its viewport from the top
/// row on every resize and keeps no history of its own, so a grown pane keeps its prompt where it was and the rows added
/// at the bottom stay blank. Pulling the history down on Windows was measured to lose lines (`set_pull_history`, see
/// CLAUDE.md), so the emulator does not do it there. Linux and macOS pull the history down, which keeps the prompt on the
/// last row. Run it with `cargo test --test render a_shell_prompt_stays_on_the_last_row_after_the_pane_grows -- --ignored`.
#[test]
#[ignore = "known Windows limit: a grown pane keeps its shell prompt in place (ConPTY, see CLAUDE.md)"]
fn a_shell_prompt_stays_on_the_last_row_after_the_pane_grows() {
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("grow".into()));
    let id = app.tabs[0].focus;
    wait_idle(&mut app, id);
    let (command, last_line) =
        if cfg!(windows) { ("1..120 | ForEach-Object { \"line $_\" }\r", "line 120") } else { ("seq 1 120\r", "120") };
    app.panes[&id].write(command.as_bytes());
    wait_for(&mut app, id, last_line);
    wait_idle(&mut app, id);
    render(&mut app, 110, 30);
    render(&mut app, 160, 45);
    run_for(&mut app, Duration::from_millis(600));
    render(&mut app, 160, 45);
    let emu = app.panes[&id].size.0;
    let (cursor_row, _) = app.panes[&id].parser().screen().cursor_position();
    assert_eq!(cursor_row, emu - 1, "the prompt is not on the pane's last row after the pane grew");
    app.run(Action::CloseTab);
    app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
}
