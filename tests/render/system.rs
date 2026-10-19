//! System screen, its panel buttons and the boot sequence.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use noble::app::{App, View};
use noble::keys::Action;
use std::time::Duration;

use super::common::*;

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
fn boot_sequence_renders() {
    let boot_at = |app: &mut App, ms: u64| {
        app.boot = Some(noble::app::Boot { started: std::time::Instant::now() - Duration::from_millis(ms), seed: 7 });
    };
    for (ms, name) in [(250u64, "open"), (1000, "converge"), (1500, "decode"), (2250, "hold"), (2850, "iris")] {
        for (w, h) in SIZES {
            let mut app = demo_app(w, h);
            boot_at(&mut app, ms);
            let text = render(&mut app, w, h);
            save(&format!("boot-{name}-{w}x{h}"), &text);
        }
        for (w, h) in [(1, 1), (3, 3), (5, 3), (20, 4), (200, 3), (3, 60)] {
            let mut app = demo_app(w, h);
            boot_at(&mut app, ms);
            render(&mut app, w, h);
        }
    }
    // Fully decoded: only the logo, inside the brackets.
    let mut app = demo_app(110, 30);
    boot_at(&mut app, 2250);
    let text = render(&mut app, 110, 30);
    for line in noble::ui::hud::LOGO {
        assert!(text.contains(line.trim_end()), "{text}");
    }
    assert!(text.contains('┌') && text.contains('┘'), "{text}");
    let letters = text.chars().filter(|c| c.is_ascii_alphanumeric()).count();
    assert_eq!(letters, 0, "no text besides the logo\n{text}");
    // The iris has opened over Home; once the boot ends the app draws on its own.
    boot_at(&mut app, 2850);
    let text = render(&mut app, 110, 30);
    assert!(text.contains("AI usage") && !text.contains(noble::ui::hud::LOGO[0]), "{text}");
    boot_at(&mut app, 3000);
    let text = render(&mut app, 110, 30);
    assert!(text.contains("Projects"), "{text}");
    app.tick();
    assert!(app.boot.is_none());
    // Narrow windows get a one-line logo.
    let mut app = demo_app(20, 8);
    boot_at(&mut app, 2250);
    assert!(render(&mut app, 20, 8).contains("NOBLE"));
}

/// System's Processes panel: "/ Filter" and "K Terminate" are buttons that do what the keys do; the
/// status bar lists only the keys the panel does not show.
#[test]
fn system_panel_buttons_run_the_keys() {
    use noble::app::{ConfirmAction, Hit, Overlay};
    let mut app = demo_app(120, 34);
    app.run(Action::System);
    let (text, buf) = render_buf(&mut app, 120, 34);
    save("system-buttons-120x34", &text);
    let bar = text.lines().last().unwrap();
    assert!(bar.contains("esc home") && !bar.contains("terminate") && !bar.contains("filter"), "{bar}");
    let filter = find_hit(&app, |h| *h == Hit::SystemFilter).expect("filter button");
    let kill = find_hit(&app, |h| *h == Hit::SystemTerminate).expect("terminate button");
    assert_eq!(filter.y, kill.y);
    assert_eq!(cells_text(&buf, filter).trim(), "/ Filter");
    assert_eq!(cells_text(&buf, kill).trim(), "K Terminate");
    // Without a selected process the button only says so, as the key does.
    app.system.selected_pid = None;
    click(&mut app, kill.x + 1, kill.y);
    assert!(app.overlay.is_none());
    // With one selected it asks first (the same confirmation as K); "No" keeps the process.
    let row = find_hit(&app, |h| matches!(h, Hit::Proc(_))).expect("a process row");
    click(&mut app, row.x + 5, row.y);
    render(&mut app, 120, 34);
    let kill = find_hit(&app, |h| *h == Hit::SystemTerminate).expect("terminate button");
    click(&mut app, kill.x + 1, kill.y);
    let Some(Overlay::Confirm(c)) = &app.overlay else { panic!("no confirmation") };
    assert!(matches!(c.action, ConfirmAction::Kill { .. }));
    render(&mut app, 120, 34);
    let no = find_hit(&app, |h| *h == Hit::ConfirmNo).expect("no button");
    click(&mut app, no.x, no.y);
    assert!(app.overlay.is_none());
    // The filter button starts the filter as "/" does; Esc leaves it.
    render(&mut app, 120, 34);
    let filter = find_hit(&app, |h| *h == Hit::SystemFilter).expect("filter button");
    click(&mut app, filter.x + 1, filter.y);
    assert!(app.system.filtering && app.system.filter.is_empty());
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.system.filtering);

    // Small sizes: a button that does not fit is left out, never cut in half.
    for (w, h) in [(90, 28), (60, 18), (40, 12), (30, 8)] {
        let mut app = demo_app(w, h);
        app.run(Action::System);
        let (text, buf) = render_buf(&mut app, w, h);
        save(&format!("system-buttons-{w}x{h}"), &text);
        match find_hit(&app, |h| *h == Hit::SystemFilter) {
            Some(r) => assert_eq!(cells_text(&buf, r).trim(), "/ Filter", "{w}x{h}"),
            None => assert!(!text.contains("Filt"), "{w}x{h}: {text}"),
        }
        match find_hit(&app, |h| *h == Hit::SystemTerminate) {
            Some(r) => assert_eq!(cells_text(&buf, r).trim(), "K Terminate", "{w}x{h}"),
            None => assert!(!text.contains("Termi"), "{w}x{h}: {text}"),
        }
    }
}

/// System with a kept filter: Esc clears the filter, so the hint bar says "esc clear" until it is empty.
#[test]
fn system_filter_hint_says_esc_clears() {
    let mut app = demo_app(120, 34);
    let key = |app: &mut App, c: KeyCode| app.on_key(KeyEvent::new(c, KeyModifiers::NONE));
    app.run(Action::System);
    let text = render(&mut app, 120, 34);
    assert!(text.lines().last().unwrap().contains("esc home"), "{text}");
    key(&mut app, KeyCode::Char('/'));
    for c in "zzqx".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Enter);
    assert!(!app.system.filtering);
    assert_eq!(app.system.filter, "zzqx");
    let text = render(&mut app, 120, 34);
    assert!(text.lines().last().unwrap().contains("esc clear"), "{text}");
    assert!(!text.lines().last().unwrap().contains("esc home"), "{text}");
    key(&mut app, KeyCode::Esc);
    assert!(app.system.filter.is_empty());
    assert_eq!(app.view, View::System, "the first Esc only clears the filter");
    let text = render(&mut app, 120, 34);
    assert!(text.lines().last().unwrap().contains("esc home"), "{text}");
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.view, View::Bridge);
}
