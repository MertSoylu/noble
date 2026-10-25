//! Settings screen and the key bindings editor.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use noble::app::{App, View};
use noble::keys::Action;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::common::*;

#[test]
fn settings_screen_mouse_and_keys() {
    use noble::app::Hit;
    let mut app = demo_app(110, 30);
    // Click the ⚙ Settings tab in the top strip.
    render(&mut app, 110, 30);
    let tab = find_hit(&app, |h| *h == Hit::TabSettings).expect("settings tab");
    click(&mut app, tab.x + 1, tab.y);
    assert_eq!(app.view, View::Settings);
    for (w, h) in [(160, 45), (110, 30), (76, 24), (56, 18), (30, 8)] {
        save(&format!("settings-{w}x{h}"), &render(&mut app, w, h));
    }
    // The theme row opens the selector; the whole UI previews the theme under the cursor.
    let theme_item = noble::app::SettingItem::Setting(noble::app::SettingKey::Theme);
    let theme_row = app.settings_items().iter().position(|i| *i == theme_item).unwrap();
    render(&mut app, 110, 30);
    let row = find_hit(&app, |h| *h == Hit::Setting(theme_row)).expect("theme row");
    click(&mut app, row.x + 3, row.y);
    assert!(matches!(app.overlay, Some(noble::app::Overlay::Themes(_))));
    let text = render(&mut app, 110, 30);
    save("theme-picker-110x30", &text);
    assert!(text.contains("THEME") && text.contains("Tokyo Night") && text.contains("Gruvbox"), "{text}");
    for (w, h) in SIZES {
        save(&format!("theme-picker-{w}x{h}"), &render(&mut app, w, h));
    }
    app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.theme.name, noble::theme::THEMES[1].name, "preview");
    assert_eq!(app.cfg.general.theme, "amber", "a preview is not saved");
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.theme.name, "amber", "esc reverts");
    // Clicking outside cancels too.
    render(&mut app, 110, 30);
    click(&mut app, row.x + 3, row.y);
    app.on_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    render(&mut app, 110, 30);
    click(&mut app, 0, 29);
    assert!(app.overlay.is_none());
    assert_eq!(app.theme.name, "amber", "backdrop click reverts");
    // Clicking a row applies it.
    render(&mut app, 110, 30);
    click(&mut app, row.x + 3, row.y);
    render(&mut app, 110, 30);
    let idx = noble::theme::THEMES.iter().position(|t| t.name == "nord").unwrap();
    let nord = find_hit(&app, |h| *h == Hit::ThemeOption(idx)).expect("nord row");
    click(&mut app, nord.x + 2, nord.y);
    assert!(app.overlay.is_none());
    assert_eq!((app.theme.name, app.cfg.general.theme.as_str()), ("nord", "nord"));
    // ←→ on the row step through the themes.
    app.settings_sel = theme_row;
    app.on_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    let next = noble::theme::THEMES[idx + 1].name;
    assert_eq!((app.theme.name, app.cfg.general.theme.as_str()), (next, next));
    app.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(app.theme.name, "nord");
    // Keyboard: move down and flip a toggle.
    let before = app.cfg.general.clock_24h;
    let items = app.settings_items();
    let clock =
        items.iter().position(|i| *i == noble::app::SettingItem::Setting(noble::app::SettingKey::Clock24)).unwrap();
    app.settings_sel = clock;
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_ne!(before, app.cfg.general.clock_24h);
    // Turning a provider off removes it from the AI panel.
    let claude =
        items.iter().position(|i| *i == noble::app::SettingItem::Setting(noble::app::SettingKey::Claude)).unwrap();
    app.settings_sel = claude;
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!app.cfg.ai.providers.iter().any(|p| p == "claude"));
    // The agent hooks row only exists when an agent is installed here (or has hooks on).
    let hooks = noble::app::SettingItem::Setting(noble::app::SettingKey::AgentHooks);
    app.agents_found = vec![noble::hooks::Agent::Claude];
    assert!(app.settings_items().contains(&hooks));
    assert!(render(&mut app, 160, 45).contains("Agent status hooks"));
    app.ai_installed.retain(|id| *id != "claude");
    app.agents_found.clear();
    assert!(!app.settings_items().contains(&hooks));
    let text = render(&mut app, 160, 45);
    assert!(!text.contains("status hooks"), "{text}");
    app.hooks_on = vec![noble::hooks::Agent::Claude];
    app.agents_found = vec![noble::hooks::Agent::Claude];
    assert!(app.settings_items().contains(&hooks));
    app.hooks_on.clear();
    app.agents_found.clear();
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.view, View::Bridge);
    // Keyboard navigation does not overflow at either end.
    app.run(Action::Settings);
    app.settings_sel = 0;
    for code in [
        KeyCode::Up,
        KeyCode::Left,
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Right,
        KeyCode::End,
        KeyCode::Down,
        KeyCode::Up,
        KeyCode::Up,
    ] {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
        render(&mut app, 110, 30);
    }
}

/// Settings: esc returns to the page it was opened from, ← / → turn a switch off / on, the wheel
/// scrolls the page (with "more" markers) and a key brings the selection back; rows that only
/// matter under a switched-off setting are dimmed.
#[test]
fn settings_back_arrows_scroll_and_dim() {
    use crossterm::event::MouseEventKind;
    use noble::app::{SettingItem, SettingKey};
    let key = |app: &mut App, code: KeyCode| app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("back".into()));
    app.run(Action::NewTab);
    app.go_tab(1);
    app.run(Action::Settings);
    // The tab it came from moved meanwhile: esc still finds it.
    app.move_tab(0, 1);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.view, View::Term(1));
    app.run(Action::System);
    app.run(Action::Settings);
    key(&mut app, KeyCode::Char('q'));
    assert_eq!(app.view, View::System);
    app.run(Action::CloseTab);
    app.run(Action::CloseTab);

    app.run(Action::Settings);
    let items = app.settings_items();
    let pos = |k: SettingKey| items.iter().position(|i| *i == SettingItem::Setting(k)).unwrap();
    // ← off, → on; pressing again changes nothing.
    app.settings_sel = pos(SettingKey::Clock24);
    assert!(app.cfg.general.clock_24h);
    key(&mut app, KeyCode::Right);
    assert!(app.cfg.general.clock_24h);
    key(&mut app, KeyCode::Left);
    key(&mut app, KeyCode::Left);
    assert!(!app.cfg.general.clock_24h);
    key(&mut app, KeyCode::Right);
    assert!(app.cfg.general.clock_24h);
    // A choice steps; a popup row ignores ←→.
    app.settings_sel = pos(SettingKey::AiRefresh);
    key(&mut app, KeyCode::Right);
    assert_eq!(app.cfg.ai.refresh_minutes, 15);
    key(&mut app, KeyCode::Left);
    assert_eq!(app.cfg.ai.refresh_minutes, 5);
    app.settings_sel = pos(SettingKey::QuickLaunch);
    key(&mut app, KeyCode::Right);
    assert!(app.overlay.is_none());

    // The wheel scrolls the page, the selection stays; a key scrolls back to it.
    app.settings_sel = 0;
    key(&mut app, KeyCode::Home);
    // A toast from an earlier key (e.g. "no tab here") would cover the marker.
    app.toasts.clear();
    let text = render(&mut app, 110, 30);
    assert!(text.contains("Theme") && text.contains("↓ more") && !text.contains("↑ more"), "{text}");
    for _ in 0..20 {
        mouse(&mut app, MouseEventKind::ScrollDown, 50, 15);
    }
    let text = render(&mut app, 110, 30);
    save("settings-scrolled-110x30", &text);
    assert!(text.contains("↑ more") && !text.contains("↓ more"), "{text}");
    assert!(text.contains("Edit config file") && !text.contains("Transparent background"), "{text}");
    assert_eq!(app.settings_sel, 0);
    key(&mut app, KeyCode::Right);
    assert!(render(&mut app, 110, 30).contains("Transparent background"));

    // Show AI usage off: the provider rows are dimmed.
    let mut cfg = app.cfg.clone();
    cfg.ai.enabled = false;
    app.apply_config(cfg);
    app.settings_sel = pos(SettingKey::AiEnabled);
    let mut term = Terminal::new(TestBackend::new(110, 60)).unwrap();
    app.size = (110, 60);
    term.draw(|f| noble::ui::draw(f, &mut app)).unwrap();
    let buf = term.backend().buffer().clone();
    let row = find_hit(&app, |h| *h == noble::app::Hit::Setting(pos(SettingKey::AiRefresh))).expect("refresh row");
    let label = (row.x..row.right()).find(|x| buf[(*x, row.y)].symbol() == "R").unwrap();
    assert_eq!(buf[(label, row.y)].fg, app.theme.dim, "dependent row not dimmed");
    let row = find_hit(&app, |h| *h == noble::app::Hit::Setting(pos(SettingKey::Shell))).expect("shell row");
    let label = (row.x..row.right()).find(|x| buf[(*x, row.y)].symbol() == "S").unwrap();
    assert_ne!(buf[(label, row.y)].fg, app.theme.dim);
    for (w, h) in SIZES {
        render(&mut app, w, h);
    }
}

#[test]
fn every_provider_is_configurable() {
    use noble::app::{PROVIDER_KEYS, SettingItem};
    let mut app = demo_app(110, 30);
    let ids: Vec<&str> = noble::ai::providers::registry().iter().map(|d| d.id).collect();
    // Every provider has a setting and the default config knows them all.
    let keyed: Vec<&str> = PROVIDER_KEYS.iter().map(|k| k.provider_id()).collect();
    assert_eq!(keyed, ids);
    let parsed = noble::config::parse(noble::config::DEFAULT_CONFIG).unwrap();
    assert_eq!(parsed.ai.providers, ids);
    // The toggle really changes something.
    for key in PROVIDER_KEYS {
        let before = app.setting_on(key);
        app.activate_setting(SettingItem::Setting(key), 1);
        assert_ne!(app.setting_on(key), before, "{key:?}");
    }
}

/// A click on the left half of a "‹ value ›" chip steps backward, the right half (and the label) forward;
/// a right click steps backward.
#[test]
fn settings_click_direction() {
    use crossterm::event::{MouseButton, MouseEventKind};
    use noble::app::{Hit, SettingItem, SettingKey};
    let mut app = demo_app(110, 40);
    app.open_settings();
    let pos =
        |app: &App, k: SettingKey| app.settings_items().iter().position(|i| *i == SettingItem::Setting(k)).unwrap();
    let i = pos(&app, SettingKey::AiRefresh);
    app.settings_sel = i;
    let text = render(&mut app, 110, 40);
    assert!(text.contains("How often usage is fetched"), "description of the selection: {text}");
    let row = find_hit(&app, |h| *h == Hit::Setting(i)).expect("row");
    let before = app.setting_value(SettingKey::AiRefresh);
    let chip_w = noble::util::width(&format!("‹ {before} ›")) as u16;
    let start = row.right() - 2 - chip_w;
    click(&mut app, start, row.y);
    let back = app.setting_value(SettingKey::AiRefresh);
    assert_ne!(back, before);
    render(&mut app, 110, 40);
    click(&mut app, start + chip_w - 1, row.y);
    assert_eq!(app.setting_value(SettingKey::AiRefresh), before, "right half steps forward again");
    render(&mut app, 110, 40);
    mouse(&mut app, MouseEventKind::Down(MouseButton::Right), row.x + 3, row.y);
    assert_eq!(app.setting_value(SettingKey::AiRefresh), back, "right click steps backward");
    for (w, h) in [(60, 12), (30, 8)] {
        render(&mut app, w, h);
    }
}

/// Settings: "/" filters the rows by name, esc clears the filter, "no settings match" shows when none fit.
#[test]
fn settings_filter_narrows_rows() {
    use noble::app::{SettingItem, SettingKey};
    let mut app = demo_app(110, 30);
    app.open_settings();
    let all = app.settings_items().len();
    app.on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    for c in "anim".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    let items = app.settings_items();
    assert!(items.contains(&SettingItem::Setting(SettingKey::Animations)));
    assert!(items.len() < all && items.iter().all(|i| i.label().to_lowercase().contains("anim")));
    let text = render(&mut app, 110, 30);
    assert!(text.contains("Settings / anim▏"), "{text}");
    assert!(!text.contains("Transparent background"), "filtered rows are hidden");
    assert!(text.contains("type") && text.contains("filter"), "status bar shows the filter keys");
    // Letters typed into the filter are text, not commands: q does not leave Settings.
    app.on_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
    assert_eq!(app.view, View::Settings);
    for _ in 0..5 {
        app.on_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    }
    assert!(app.settings_filter.is_empty());
    // A query nothing matches: the page says so, and the keys that move in the list do not panic.
    for c in "zzz".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    assert!(app.settings_items().is_empty());
    let text = render(&mut app, 110, 30);
    assert!(text.contains("no settings match"), "{text}");
    for code in [KeyCode::Down, KeyCode::End, KeyCode::Enter, KeyCode::Left, KeyCode::Up] {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
        render(&mut app, 110, 30);
    }
    // ⏎ ends typing and keeps the filter; esc clears it, then a second esc leaves the page.
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!app.settings_filtering && app.settings_filter == "zzz");
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.settings_filter.is_empty());
    assert_eq!(app.settings_items().len(), all);
    assert_eq!(app.view, View::Settings);
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_ne!(app.view, View::Settings);
}

/// A symbol typed with AltGr (Ctrl+Alt on Windows) is text in the Settings and key bindings filters, like
/// in the other text fields; a real alt+letter is not.
#[test]
fn altgr_symbols_type_into_the_filters() {
    let altgr = KeyModifiers::CONTROL | KeyModifiers::ALT;
    let mut app = demo_app(110, 30);
    app.open_settings();
    app.on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    app.on_key(KeyEvent::new(KeyCode::Char('@'), altgr));
    app.on_key(KeyEvent::new(KeyCode::Char('{'), altgr));
    app.on_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::ALT));
    assert_eq!(app.settings_filter, "@{");
}

/// Settings on a wide terminal (140+ columns): the page is 130 columns, its list keeps the first 80
/// and a description panel on the right shows the selected label in bold with its description wrapped.
/// The bottom row no longer carries the description.
#[test]
fn settings_description_panel_at_160x45() {
    use noble::app::{SettingItem, SettingKey};
    use ratatui::style::Modifier;
    let mut app = demo_app(160, 45);
    app.run(Action::Settings);
    let items = app.settings_items();
    app.settings_sel = items.iter().position(|i| *i == SettingItem::Setting(SettingKey::Restore)).unwrap();
    app.settings_follow = true;
    let text = render(&mut app, 160, 45);
    save("settings-160x45", &text);
    let buf = render_cells(&mut app, 160, 45);
    let label = SettingKey::Restore.label();
    let desc = SettingKey::Restore.description();

    // The list row keeps the label within its 80 columns; the panel repeats it in bold at column 82+.
    // The selected row's marker "▌" comes right before the list label.
    let (lx, _) = find_text(&buf, &format!("▌ {label}"), 0).expect("the list row");
    assert!(lx < 80, "list column: {lx}");
    let (px, py) = find_text(&buf, label, 82).expect("the panel label");
    assert!(buf[(px, py)].modifier.contains(Modifier::BOLD), "panel label is bold");
    // The description wraps inside the panel.
    let first_words: String = desc.split_whitespace().take(4).collect::<Vec<_>>().join(" ");
    assert!(find_text(&buf, &first_words, 82).is_some(), "panel description:\n{text}");
    assert!(desc.split_whitespace().count() > 4, "the description wraps in the panel");
    // A rule separates the list from the panel.
    assert!((0..45).any(|y| buf[(96, y)].symbol() == "│"), "separator:\n{text}");
    // The description is not on the bottom row any more.
    assert!(!text.lines().last().unwrap().contains("Reopen"), "{text}");
    assert!(find_text(&buf, "Reopen", 0).is_some_and(|(x, _)| x >= 82), "{text}");

    // Other sizes for the overflow review (dumps under target/audit).
    for (w, h) in [(160, 45), (140, 30), (200, 60), (150, 12), (96, 20), (60, 20), (30, 8)] {
        save(&format!("settings-{w}x{h}"), &render(&mut app, w, h));
    }
}

/// Settings on a narrow terminal: the selected setting's description sits under its row, dimmed and
/// indented by two, and the bottom row keeps only the "more" marker. The selection scrolls with it.
#[test]
fn settings_description_under_selected_row_at_110x30() {
    use noble::app::{SettingItem, SettingKey};
    let mut app = demo_app(110, 30);
    app.run(Action::Settings);
    let items = app.settings_items();
    let label = SettingKey::Restore.label();
    let desc = SettingKey::Restore.description();
    app.settings_sel = items.iter().position(|i| *i == SettingItem::Setting(SettingKey::Restore)).unwrap();
    app.settings_follow = true;
    let text = render(&mut app, 110, 30);
    save("settings-110x30", &text);
    let lines: Vec<&str> = text.lines().collect();
    let row = lines.iter().position(|l| l.contains(label)).expect("the selected row");
    assert!(lines[row + 1].contains(desc), "description under its row:\n{text}");
    assert!(!lines.last().unwrap().contains(desc), "no description on the bottom row:\n{text}");

    let buf = render_cells(&mut app, 110, 30);
    let (lx, ly) = find_text(&buf, label, 0).unwrap();
    let (dx, dy) = find_text(&buf, desc, 0).unwrap();
    assert_eq!((dx, dy), (lx + 2, ly + 1), "indented under the label");
    assert_eq!(buf[(dx, dy)].fg, app.theme.dim().fg.unwrap(), "dimmed");

    // The last setting's description stays on screen with its row when the page scrolls to it.
    let last = items.iter().rev().find_map(|i| match i {
        SettingItem::Setting(k) => Some(*k),
        _ => None,
    });
    let last = last.unwrap();
    app.settings_sel = items.iter().position(|i| *i == SettingItem::Setting(last)).unwrap();
    app.settings_follow = true;
    let text = render(&mut app, 110, 30);
    assert!(text.contains(last.description()), "the selected setting's description is visible:\n{text}");
    assert!(text.contains(last.label()), "{text}");
}

/// Settings → Key bindings: the row opens the editor, which lists every action with its keys. A double
/// click starts a rebinding, a key another action has is asked about first, a key NOBLE cannot bind says
/// why, and the filter (`/`) takes letters as text. The `?` reference names the editor.
#[test]
fn key_bindings_editor_renders_and_asks_before_taking_keys() {
    use noble::app::{Hit, Overlay, SettingItem, SettingKey};
    let mut app = demo_app(110, 30);
    app.open_settings();
    let row = app
        .settings_items()
        .iter()
        .position(|i| *i == SettingItem::Setting(SettingKey::KeyBindings))
        .expect("the row is in the Keys section");
    app.settings_sel = row;
    render(&mut app, 110, 30);
    let hit = find_hit(&app, |h| *h == Hit::Setting(row)).expect("the row is drawn");
    click(&mut app, hit.x + 3, hit.y);
    assert!(matches!(app.overlay, Some(Overlay::Bindings(_))), "the row opens the editor");
    let text = render(&mut app, 110, 30);
    save("bindings-110x30", &text);
    assert!(text.contains("KEY BINDINGS") && text.contains("Split Pane Right") && text.contains("PREFIX"), "{text}");
    assert!(text.contains("⏎ prefix key"), "{text}");
    for (w, h) in SIZES {
        save(&format!("bindings-{w}x{h}"), &render(&mut app, w, h));
    }
    assert_no_doubled_frame(&render(&mut app, 110, 30));

    // A double click on the row starts its prefix key; the editor says what it waits for.
    let split = app.binding_rows("").iter().position(|a| *a == Action::SplitRight).unwrap();
    render(&mut app, 110, 30);
    let target = find_hit(&app, |h| *h == Hit::BindingsRow(split)).expect("the row is drawn");
    click(&mut app, target.x + 2, target.y);
    click(&mut app, target.x + 2, target.y);
    let text = render(&mut app, 110, 30);
    assert!(text.contains("press a key for Split Pane Right after ctrl+a · esc cancel"), "{text}");

    // ctrl+alt with a symbol is AltGr on Windows: refused, and the editor keeps waiting.
    app.on_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::CONTROL | KeyModifiers::ALT));
    let text = render(&mut app, 110, 30);
    assert!(text.contains("AltGr"), "{text}");
    assert!(matches!(&app.overlay, Some(Overlay::Bindings(st)) if st.capture.is_some()), "still waiting");

    // "n" is Next Tab's prefix key: the question comes first, and "n" keeps its owner until yes.
    app.on_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
    let text = render(&mut app, 110, 30);
    save("bindings-replace-110x30", &text);
    assert!(text.contains("ctrl+a n is Next Tab — replace? y/n"), "{text}");
    app.on_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
    assert!(matches!(&app.overlay, Some(Overlay::Bindings(st)) if st.replace.is_none()), "no keeps the keys");

    // A yes saves at once: the status line says so, and the Settings row shows the keys are custom.
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.on_key(KeyEvent::new(KeyCode::Char('%'), KeyModifiers::NONE));
    let text = render(&mut app, 110, 30);
    assert!(text.contains("saved · ctrl+a % is Split Pane Right"), "{text}");
    assert_eq!(app.setting_value(SettingKey::KeyBindings), "custom");

    // The filter takes letters as text: "/" then "split" lists only the matching actions.
    app.on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    for c in "split".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    let text = render(&mut app, 110, 30);
    assert!(text.contains("/ split") && text.contains("Split Pane Down") && !text.contains("Next Tab"), "{text}");
    // esc clears the filter, then the second esc closes the editor.
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.overlay.is_none(), "esc closes the editor");
    assert!(matches!(app.view, View::Settings), "back on the Settings page");

    // The keyboard reference names the editor, so it is found from the `?` screen.
    app.run(Action::Help);
    for c in "key bindings".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    assert!(render(&mut app, 110, 30).contains("Settings → Key bindings"));
}
