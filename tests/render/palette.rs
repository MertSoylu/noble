//! Command palette, prefix chord hints and the no-op and empty-menu rules.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use noble::app::App;
use noble::config::Config;
use noble::event::AppEvent;
use noble::keys::Action;

use super::common::*;

/// Keys in a menu with no items do nothing (the selection arithmetic used to underflow).
#[test]
fn empty_menu_keys_do_not_panic() {
    use noble::app::{Menu, Overlay};
    let mut app = demo_app(110, 30);
    app.overlay = Some(Overlay::Menu(Menu { title: "empty".into(), x: 0, y: 0, items: Vec::new(), selected: 0 }));
    for code in [KeyCode::Up, KeyCode::Char('k'), KeyCode::Down, KeyCode::Tab, KeyCode::Enter] {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }
    assert!(matches!(app.overlay, Some(Overlay::Menu(_))));
}

/// The palette lists the key a tab is really bound to, not a fixed alt+N.
#[test]
fn palette_tab_hints_follow_rebinds() {
    use noble::app::Overlay;
    let hint_of = |app: &mut App| {
        app.overlay = None;
        app.run(Action::Palette);
        let Some(Overlay::Palette(p)) = &app.overlay else { panic!("no palette") };
        p.all.iter().find(|i| i.title.starts_with("Tab 1:")).expect("tab item").hint.clone()
    };
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("one".into()));
    assert_eq!(hint_of(&mut app), "alt+1");

    let mut cfg = Config::default();
    cfg.general.animations = false;
    cfg.keys.direct_bindings.insert("alt+1".into(), "none".into());
    cfg.keys.direct_bindings.insert("f2".into(), "tab_1".into());
    let mut app = App::headless(cfg, (110, 30));
    app.new_tab(std::env::temp_dir(), None, Some("one".into()));
    assert_eq!(hint_of(&mut app), "f2");

    let mut cfg = Config::default();
    cfg.keys.direct_bindings.insert("alt+1".into(), "none".into());
    cfg.keys.prefix_bindings.insert("1".into(), "none".into());
    let mut app = App::headless(cfg, (110, 30));
    app.new_tab(std::env::temp_dir(), None, Some("one".into()));
    assert_eq!(hint_of(&mut app), "", "an unbound tab shows no key");
}

/// The armed-prefix hints come from the keymap: a rebound key is shown, an unbound action is not listed.
#[test]
fn prefix_hints_follow_the_keymap() {
    use noble::keys::Keymap;
    let ctrl_a = || KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
    let mut app = demo_app(120, 30);
    app.new_tab(std::env::temp_dir(), None, Some("keys".into()));
    let mut cfg = app.cfg.keys.clone();
    // Rebind split-right, drop zoom's prefix keys and close-pane's key.
    cfg.prefix_bindings.insert("v".into(), "none".into());
    cfg.prefix_bindings.insert("|".into(), "none".into());
    cfg.prefix_bindings.insert("%".into(), "split_right".into());
    cfg.prefix_bindings.insert("z".into(), "none".into());
    cfg.prefix_bindings.insert("x".into(), "none".into());
    app.keymap = Keymap::from_config(&cfg);
    app.on_key(ctrl_a());
    let text = render(&mut app, 120, 30);
    save("status-prefix-rebound-120x30", &text);
    let bar = text.lines().last().unwrap();
    assert!(bar.contains("PREFIX") && bar.contains("% split right") && bar.contains("esc cancel"), "{bar}");
    assert!(!bar.contains("zoom") && !bar.contains("close") && !bar.contains("v split"), "{bar}");
    for (w, h) in SIZES {
        let t = render(&mut app, w, h);
        assert!(t.lines().last().unwrap().contains("PREFIX") || w < 20, "{w}x{h}");
    }
}

/// A pending prefix is dropped by a click, a paste and an overlay, and Esc; prefix twice sends the key.
#[test]
fn prefix_is_cancelled_by_other_input() {
    use crossterm::event::{Event, MouseButton, MouseEventKind};
    let ctrl_a = || KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
    let mut app = demo_app(120, 30);
    app.new_tab(std::env::temp_dir(), None, Some("keys".into()));
    app.on_key(ctrl_a());
    assert!(app.prefix_armed);
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), 60, 15);
    assert!(!app.prefix_armed, "a click cancels");
    app.on_key(ctrl_a());
    app.handle(AppEvent::Input(Event::Paste("x".into())));
    assert!(!app.prefix_armed, "a paste cancels");
    app.on_key(ctrl_a());
    app.overlay = Some(noble::app::Overlay::Launchers { selected: 0 });
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.prefix_armed, "an overlay key cancels");
    app.on_key(ctrl_a());
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.prefix_armed && app.overlay.is_none());
    // Prefix twice: not armed afterwards and no "not bound" toast.
    app.on_key(ctrl_a());
    app.on_key(ctrl_a());
    assert!(!app.prefix_armed);
}

/// Commands with nothing to act on say why instead of doing nothing.
#[test]
fn no_op_commands_explain_themselves() {
    use noble::keys::Action;
    let mut app = demo_app(110, 30);
    for (action, want) in [
        (Action::CloseTab, "no tab here"),
        (Action::RenameTab, "no tab here"),
        (Action::MoveTabLeft, "no tab here"),
        (Action::Zoom, "no pane here"),
    ] {
        app.toasts.clear();
        app.run(action);
        assert!(toast_texts(&app).iter().any(|t| t.contains(want)), "{action:?}: {:?}", toast_texts(&app));
    }
    app.new_tab(std::env::temp_dir(), None, Some("one".into()));
    app.toasts.clear();
    app.run(Action::Zoom);
    assert!(toast_texts(&app).iter().any(|t| t.contains("nothing to zoom")), "{:?}", toast_texts(&app));
    app.toasts.clear();
    app.run(Action::MoveTabRight);
    assert!(toast_texts(&app).iter().any(|t| t.contains("last tab")), "{:?}", toast_texts(&app));
}

/// Palette hints follow the view: in a terminal a hint is a key that reaches NOBLE there (never a shell key such
/// as alt+t, and no direct key while the pane's keys are locked). Search and Pass keys are offered only in a
/// terminal, Home shows the single keys it handles itself, and the hooks offer actions only while the offer shows.
#[test]
fn palette_hints_follow_the_view() {
    let titles = |app: &App| app.palette_items().into_iter().map(|i| i.title).collect::<Vec<_>>();
    let hint_of = |app: &App, title: &str| {
        app.palette_items().into_iter().find(|i| i.title == title).unwrap_or_else(|| panic!("no {title}")).hint
    };
    let mut app = demo_app(120, 30);
    let home = titles(&app);
    assert!(!home.iter().any(|t| t == "Search Scrollback" || t == "Pass Keys to the App"), "{home:?}");
    assert_eq!(hint_of(&app, "Rescan Projects"), "r");
    assert_eq!(hint_of(&app, "Refresh AI Quotas"), "R");
    assert_eq!(hint_of(&app, "Add Project or Folder…"), "a");
    // `A` still works (project mode) but is not shown anywhere.
    assert_eq!(hint_of(&app, "Add Project…"), "");

    app.new_tab(std::env::temp_dir(), None, Some("keys".into()));
    let term = titles(&app);
    assert!(term.iter().any(|t| t == "Search Scrollback") && term.iter().any(|t| t == "Pass Keys to the App"));
    let shell = hint_of(&app, "New Terminal Tab");
    assert_ne!(shell, "alt+t", "alt+t goes to the shell in a terminal");
    assert_eq!(shell, app.keymap.term_hint(Action::NewTab, false).unwrap_or_default());
    app.run(Action::Passthrough);
    assert!(app.focused_locked());
    let locked = hint_of(&app, "New Terminal Tab");
    assert_eq!(locked, app.keymap.term_hint(Action::NewTab, true).unwrap_or_default());
    assert!(!locked.contains("alt+t"), "{locked}");
    app.run(Action::Passthrough);

    app.run(Action::Bridge);
    let hooks = |app: &App| {
        let t = titles(app);
        (t.iter().any(|x| x == "Enable Claude Status Hooks"), t.iter().any(|x| x == "Dismiss Claude Status Offer"))
    };
    assert_eq!(hooks(&app), (false, false));
    app.hooks_offer = true;
    assert_eq!(hooks(&app), (true, true));
    app.run(Action::DismissHooksOffer);
    assert!(!app.hooks_offer);
    assert_eq!(hooks(&app), (false, false));
    app.hooks_offer = true;
    app.run(Action::EnableHooks);
    assert!(!app.hooks_offer, "the palette answers the offer");
    assert!(!app.hooks_installed, "headless never touches ~/.claude");
    assert_eq!(hooks(&app), (false, false));
}

/// The palette's matched letters are bold accent; the rest of the title keeps the text color.
#[test]
fn palette_marks_matched_title_letters() {
    use noble::app::Overlay;
    let mut app = demo_app(110, 30);
    app.open_palette();
    for c in "settings".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    let text = render(&mut app, 110, 30);
    save("palette-highlight-110x30", &text);
    let buf = render_cells(&mut app, 110, 30);
    // Find the "Open Settings" row and its cells.
    let row = (0..30u16)
        .find(|y| {
            let line: String = (0..110u16).map(|x| buf[(x, *y)].symbol().to_string()).collect();
            line.contains("Open Settings")
        })
        .expect("Open Settings row");
    let line: String = (0..110u16).map(|x| buf[(x, row)].symbol().to_string()).collect();
    // A byte offset in the line; the cell column counts chars (the palette draws multi-byte glyphs).
    let at = line[..line.find("Open Settings").unwrap()].chars().count() as u16;
    // "settings" matches the whole word "Settings" (the O before it is not matched).
    let s = &buf[(at + 5, row)];
    assert_eq!(s.symbol(), "S");
    assert!(s.modifier.contains(ratatui::style::Modifier::BOLD), "matched letter is bold");
    assert_eq!(s.fg, app.theme.accent, "matched letter is accent");
    let o = &buf[(at, row)];
    assert_ne!(o.fg, app.theme.accent, "unmatched letter keeps the text color");
    assert!(matches!(app.overlay, Some(Overlay::Palette(_))));
}

/// Recently run commands come first, most recent first, once each, and the list is kept to ten.
#[test]
fn palette_empty_query_lists_recent_commands_first() {
    use noble::app::{Overlay, PaletteCmd};
    let mut app = demo_app(110, 30);
    let (a, b) = (noble::theme::THEMES[3].name, noble::theme::THEMES[5].name);
    app.run_palette(PaletteCmd::Theme(a));
    app.run_palette(PaletteCmd::Theme(b));
    app.run_palette(PaletteCmd::Theme(a));
    assert_eq!(app.ui_state.data.palette_recent, vec![format!("theme:{a}"), format!("theme:{b}")]);
    app.open_palette();
    let Some(Overlay::Palette(p)) = &app.overlay else { panic!("no palette") };
    let cmds: Vec<PaletteCmd> = p.matches.iter().take(3).map(|i| p.all[*i].cmd.clone()).collect();
    assert_eq!(cmds[..2], [PaletteCmd::Theme(a), PaletteCmd::Theme(b)]);
    assert!(!cmds[2..].contains(&PaletteCmd::Theme(a)) && !cmds[2..].contains(&PaletteCmd::Theme(b)), "no duplicates");
    // The memory is bounded.
    for t in noble::theme::THEMES.iter().take(12) {
        app.run_palette(PaletteCmd::Theme(t.name));
    }
    assert_eq!(app.ui_state.data.palette_recent.len(), 10);
}

/// A "Setting:" palette item toggles its setting, like ⏎ on that row; typing its name and ⏎ does it.
#[test]
fn palette_setting_item_toggles_a_switch() {
    use noble::app::{Overlay, PaletteCmd, SettingItem, SettingKey};
    let mut app = demo_app(110, 30);
    let before = app.cfg.general.animations;
    app.open_palette();
    for c in "setting animations".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    let Some(Overlay::Palette(p)) = &app.overlay else { panic!("no palette") };
    let first = p.current().expect("a match").clone();
    let state = if before { "On" } else { "Off" };
    assert_eq!(first.title, format!("Setting: Animations ({state})"));
    assert_eq!(first.group, "SET");
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.cfg.general.animations, !before, "the switch flipped");
    assert_eq!(app.ui_state.data.palette_recent, vec!["setting:Animations".to_string()]);
    // A choice opens its popup, as ⏎ does on its row.
    app.run_palette(PaletteCmd::Setting(SettingItem::Setting(SettingKey::Theme)));
    assert!(matches!(app.overlay, Some(Overlay::Themes(_))), "the theme row opens the theme picker");
}
