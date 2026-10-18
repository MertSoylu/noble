//! Status bar tips, the prefix hint and the one-time shell key notice.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use noble::app::View;
use noble::keys::Action;

use super::common::*;

/// The hint shows on the wide status bar and takes no space on the narrow one.
#[test]
fn status_bar_shows_tips_when_wide() {
    let mut app = demo_app(200, 40);
    let text = render(&mut app, 200, 40);
    assert!(text.lines().last().unwrap().contains("tip: "), "{text}");
    let narrow = render(&mut app, 110, 30);
    assert!(!narrow.lines().last().unwrap().contains("tip: "));
}

/// Until the prefix key is used once, the tip points it out (with the configured key).
#[test]
fn status_bar_points_out_the_prefix_until_used() {
    let mut app = demo_app(200, 40);
    let want = format!("tip: press {}, then ? for all keys", app.keymap.prefix);
    let text = render(&mut app, 200, 40);
    save("status-prefix-tip-200x40", &text);
    assert!(text.lines().last().unwrap().contains(&want), "{text}");
    for (w, h) in SIZES {
        save(&format!("status-prefix-tip-{w}x{h}"), &render(&mut app, w, h));
    }
    app.on_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    assert!(app.ui_state.data.prefix_used);
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let text = render(&mut app, 200, 40);
    let last = text.lines().last().unwrap();
    assert!(last.contains("tip: ") && !last.contains(&want), "{text}");
}

/// The tip follows the screen: terminal tips in a terminal, Home's on Home, none on Settings. It
/// moves with screen changes only: renders of the same screen show the same tip.
#[test]
fn tips_follow_the_screen() {
    let mut app = demo_app(200, 40);
    app.ui_state.data.prefix_used = true;
    app.new_tab(std::env::temp_dir(), None, Some("tips".into()));
    let term = app.view;
    let tip_of = |text: &str| text.lines().last().unwrap().split_once("tip: ").map(|(_, t)| t.trim().to_string());
    let (mut home, mut in_term) = (Vec::new(), Vec::new());
    for _ in 0..10 {
        app.view = View::Bridge;
        home.push(tip_of(&render(&mut app, 200, 40)).expect("a tip on Home"));
        app.view = term;
        in_term.push(tip_of(&render(&mut app, 200, 40)).expect("a tip in a terminal"));
    }
    let term_only = ["ctrl+click", "zooms", "searches", "pane title"];
    let home_only = ["on Home", "Settings →", "project"];
    assert!(home.iter().all(|t| !term_only.iter().any(|w| t.contains(w))), "{home:?}");
    assert!(in_term.iter().all(|t| !home_only.iter().any(|w| t.contains(w))), "{in_term:?}");
    assert!(std::collections::HashSet::<&String>::from_iter(&home).len() > 1, "{home:?}");
    app.view = View::System;
    assert!(tip_of(&render(&mut app, 200, 40)).is_some(), "System has tips too");
    app.view = View::Settings;
    let text = render(&mut app, 200, 40);
    assert!(!text.lines().last().unwrap().contains("tip: "), "no tip on Settings\n{text}");
    // Same screen, later renders: the same tip.
    app.view = View::Bridge;
    let first = tip_of(&render(&mut app, 200, 40));
    for _ in 0..3 {
        assert_eq!(tip_of(&render(&mut app, 200, 40)), first);
    }
}

/// `general.show_tips` off (Settings, "Show tips"): no tip on any screen, the prefix tip included.
#[test]
fn show_tips_off_hides_every_tip() {
    use noble::app::{SettingItem, SettingKey};
    let mut app = demo_app(200, 40);
    app.new_tab(std::env::temp_dir(), None, Some("tips".into()));
    let term = app.view;
    assert!(render(&mut app, 200, 40).lines().last().unwrap().contains("tip: "));
    app.activate_setting(SettingItem::Setting(SettingKey::Tips), 1);
    assert!(!app.cfg.general.show_tips);
    for view in [View::Bridge, View::System, term] {
        app.view = view;
        let text = render(&mut app, 200, 40);
        assert!(!text.lines().last().unwrap().contains("tip: "), "{view:?}\n{text}");
    }
    app.activate_setting(SettingItem::Setting(SettingKey::Tips), 1);
    app.view = View::Bridge;
    assert!(render(&mut app, 200, 40).lines().last().unwrap().contains("tip: "));
}

/// A shell key with a NOBLE shortcut goes to the shell; the first time each one does, a notice
/// names the prefix chord (or the setting when the action has none), and it is never shown again.
#[test]
fn shell_key_notice_shows_once_per_key() {
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("shell".into()));
    let term = app.view;
    let alt = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT);
    app.on_key(alt('t'));
    assert_eq!(app.view, term, "still in the terminal");
    assert!(
        app.toasts.iter().any(|t| t.text.contains("alt+t went to the shell · NOBLE: ctrl+a t")),
        "{:?}",
        app.toasts.iter().map(|t| &t.text).collect::<Vec<_>>()
    );
    let shown = app.toasts.len();
    app.on_key(alt('t'));
    assert_eq!(app.toasts.len(), shown, "once per key");
    assert!(app.ui_state.data.shell_keys_told.contains(&"alt+t".to_string()));
    // Another shell key gets its own notice.
    app.on_key(alt('p'));
    assert!(app.toasts.iter().any(|t| t.text.contains("alt+p went to the shell · NOBLE: ctrl+a")));
    // An action without a prefix binding points at the setting.
    app.keymap.prefix_map.retain(|_, a| *a != Action::Settings);
    app.on_key(alt('s'));
    assert!(
        app.toasts.iter().any(|t| t.text == "alt+s went to the shell · Settings → Leave shell keys to the shell"),
        "{:?}",
        app.toasts.iter().map(|t| &t.text).collect::<Vec<_>>()
    );
}

/// "? keys" is the first hint on Home, so it is the last to go when the bar is narrow.
#[test]
fn status_bar_keeps_help_longest() {
    let mut app = demo_app(120, 30);
    let wide = render(&mut app, 120, 30);
    assert!(wide.lines().last().unwrap().contains("? keys"), "{wide}");
    for w in [70u16, 56, 50] {
        let text = render(&mut app, w, 24);
        let last = text.lines().last().unwrap();
        assert!(last.contains("? keys"), "{w}: {last}");
    }
    let text = render(&mut app, 50, 24);
    assert!(!text.lines().last().unwrap().contains("pin / more"), "{text}");
}

/// The "commands" chip on the status bar opens the palette; System's kill button is labelled "Terminate".
#[test]
fn status_bar_palette_chip_is_clickable() {
    use noble::app::{Hit, Overlay};
    let mut app = demo_app(110, 30);
    render(&mut app, 110, 30);
    let chip = find_hit(&app, |h| *h == Hit::Palette).expect("palette chip");
    assert_eq!(chip.y, 29);
    click(&mut app, chip.x + 1, chip.y);
    assert!(matches!(app.overlay, Some(Overlay::Palette(_))));
    app.overlay = None;
    app.view = View::System;
    let text = render(&mut app, 110, 30);
    let last = text.lines().last().unwrap();
    assert!(!last.contains("terminate") && !last.contains("end task"), "{last}");
    assert!(text.contains("K Terminate") && !text.contains("end task"), "{text}");
}

/// Below 50 columns the status bar's palette chip is just the key; it is dropped when even that does not fit.
#[test]
fn narrow_status_bar_palette_chip_shows_the_key() {
    use noble::app::Hit;
    let mut app = demo_app(49, 20);
    let (text, buf) = render_buf(&mut app, 49, 20);
    let chip = find_hit(&app, |h| *h == Hit::Palette).expect("key-only palette chip");
    assert_eq!(chip.y, 19);
    let shown = cells_text(&buf, chip);
    assert!(!shown.contains("commands") && !shown.trim().is_empty(), "{shown:?}: {text}");
    save("bridge-narrow-49x20", &text);
    // Wide enough again: the full chip with its word.
    let mut app = demo_app(50, 20);
    let (_, buf) = render_buf(&mut app, 50, 20);
    let chip = find_hit(&app, |h| *h == Hit::Palette).expect("palette chip");
    assert!(cells_text(&buf, chip).contains("commands"));
    // Too narrow for the key: no chip, and nothing is cut.
    let mut app = demo_app(6, 20);
    render(&mut app, 6, 20);
    assert!(find_hit(&app, |h| *h == Hit::Palette).is_none());
}
