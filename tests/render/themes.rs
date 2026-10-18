//! Theme and terminal color scheme pickers.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use noble::app::App;
use noble::keys::Action;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::common::*;

/// Terminal color scheme: pane background and ANSI colors are independent of the UI theme.
#[test]
fn terminal_color_scheme_applies() {
    use ratatui::style::Color;
    let mut app = demo_app(110, 30);
    let mut cfg = app.cfg.clone();
    cfg.terminal.colors = "light-gray".into();
    app.apply_config(cfg);
    app.new_tab(std::env::temp_dir(), None, Some("gray".into()));
    let id = app.tabs[0].focus;
    // Red (ANSI 1) text is drawn with the scheme's red.
    app.panes[&id].parser().process(b"\x1b[2J\x1b[H\x1b[31mRED\x1b[0m plain");
    let mut term = Terminal::new(TestBackend::new(110, 30)).unwrap();
    term.draw(|f| noble::ui::draw(f, &mut app)).unwrap();
    let inner = find_hit_inner(&app).expect("pane");
    let buf = term.backend().buffer().clone();
    assert_eq!(buf[(inner.x, inner.y)].symbol(), "R");
    assert_eq!(buf[(inner.x, inner.y)].fg, Color::Rgb(0xa3, 0x26, 0x1b));
    assert_eq!(buf[(inner.x + 5, inner.y)].fg, Color::Rgb(0x1e, 0x1e, 0x1e));
    let empty = (inner.x + 20, inner.y + 10);
    assert_eq!(buf[empty].bg, Color::Rgb(0xc8, 0xc8, 0xc8));
    save("terminal-light-gray-110x30", &render(&mut app, 110, 30));
    // A custom background color overrides the scheme.
    let mut cfg = app.cfg.clone();
    cfg.terminal.background = "#d0d0d0".into();
    app.apply_config(cfg);
    term.draw(|f| noble::ui::draw(f, &mut app)).unwrap();
    assert_eq!(term.backend().buffer()[empty].bg, Color::Rgb(0xd0, 0xd0, 0xd0));
    app.run(Action::CloseTab);
}

/// Scheme selector: the Windows Terminal PowerShell scheme comes first and all
/// built-ins are in the list; navigating previews, esc reverts, clicking picks.
#[test]
fn scheme_picker_previews_and_selects() {
    use noble::app::{Hit, Overlay, SettingItem, SettingKey};
    let mut app = demo_app(110, 30);
    let wt = noble::wt::parse(
        r#"{ "profiles": { "defaults": { "colorScheme": "Dark+" },
             "list": [ { "name": "Windows PowerShell", "commandline": "powershell.exe" } ] } }"#,
    )
    .unwrap();
    app.term_schemes = noble::wt::all_schemes(Some(&wt));
    assert_eq!(app.cfg.terminal.colors, "windows-terminal");
    app.run(Action::Settings);
    app.settings_sel =
        app.settings_items().iter().position(|i| *i == SettingItem::Setting(SettingKey::TermColors)).unwrap();
    let text = render(&mut app, 110, 30);
    save("settings-term-colors-110x30", &text);
    assert!(text.contains("Windows PowerShell · Dark+"), "{text}");
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(app.overlay, Some(Overlay::Schemes(_))));
    let text = render(&mut app, 110, 30);
    save("scheme-picker-110x30", &text);
    for name in ["Follow theme", "Windows PowerShell · Dark+", "Campbell Powershell", "One Half Light", "Tango Dark"] {
        assert!(
            text.contains(name),
            "{name} missing
{text}"
        );
    }
    // Down: the preview is applied, esc reverts it.
    app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.cfg.terminal.colors, "campbell");
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.cfg.terminal.colors, "windows-terminal");
    // Clicking a row picks it.
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    render(&mut app, 110, 30);
    let idx = app.scheme_options().iter().position(|n| n == "light-gray").unwrap();
    let row = find_hit(&app, |h| *h == Hit::TermScheme(idx)).expect("light gray row");
    click(&mut app, row.x + 3, row.y);
    assert_eq!(app.cfg.terminal.colors, "light-gray");
    assert!(app.overlay.is_none());
    for (w, h) in [(160, 45), (76, 24), (56, 18), (30, 8)] {
        app.open_scheme_picker();
        save(&format!("scheme-picker-{w}x{h}"), &render(&mut app, w, h));
        app.overlay = None;
    }
}

/// The theme and scheme selectors start over from the file when the config reloads while
/// they preview: esc afterwards keeps the file's values instead of the stale ones.
#[test]
fn pickers_follow_a_config_reload() {
    let key = |app: &mut App, code| app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    let dir = std::env::temp_dir().join(format!("noble-picker-reload-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("config.toml");
    std::fs::write(&file, "[general]\ntheme = \"synth\"\n").unwrap();

    let mut app = demo_app(110, 30);
    assert_ne!(app.theme.name, "synth");
    app.paths.config = file.clone();
    app.open_theme_picker();
    key(&mut app, KeyCode::Down);
    app.reload_config(false);
    assert!(matches!(app.overlay, Some(noble::app::Overlay::Themes(_))), "the selector stays open");
    render(&mut app, 110, 30);
    key(&mut app, KeyCode::Esc);
    assert!(app.overlay.is_none());
    assert_eq!(app.theme.name, "synth", "esc keeps the file's theme");

    let mut app = demo_app(110, 30);
    let colors = app.cfg.terminal.colors.clone();
    app.paths.config = file;
    app.open_scheme_picker();
    key(&mut app, KeyCode::Down);
    assert_ne!(app.cfg.terminal.colors, colors, "↓ previews a scheme");
    app.reload_config(false);
    render(&mut app, 110, 30);
    key(&mut app, KeyCode::Esc);
    assert!(app.overlay.is_none());
    assert_eq!(app.cfg.terminal.colors, colors, "esc keeps the file's colors");
    let _ = std::fs::remove_dir_all(&dir);
}
