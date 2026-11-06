//! First-launch welcome card.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use noble::app::App;

use super::common::*;

/// First launch card: drawn at every size, the prefix can be picked and applied.
#[test]
fn welcome_card_picks_prefix() {
    use noble::app::{Hit, Overlay};
    let mut app = demo_app(110, 30);
    app.show_welcome();
    let text = render(&mut app, 110, 30);
    save("welcome-110x30", &text);
    assert!(text.contains("WELCOME TO NOBLE") && text.contains("found 6 git projects"), "{text}");
    assert!(text.contains("ctrl+g"), "{text}");
    for (w, h) in SIZES {
        save(&format!("welcome-{w}x{h}"), &render(&mut app, w, h));
    }
    assert_no_doubled_frame(&render(&mut app, 30, 8));
    render(&mut app, 110, 30);
    // Clicking the background does not close the card; clicking the ctrl+g chip picks it.
    click(&mut app, 1, 29);
    assert!(matches!(app.overlay, Some(Overlay::Welcome(_))));
    let chip = find_hit(&app, |h| *h == Hit::WelcomePrefix(3)).expect("ctrl+g chip");
    click(&mut app, chip.x + 1, chip.y);
    assert!(matches!(&app.overlay, Some(Overlay::Welcome(w)) if w.prefix == 3));
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.overlay.is_none());
    assert_eq!(app.cfg.keys.prefix, "ctrl+g");
    assert!(app.ui_state.data.welcomed);
    // The new prefix actually works.
    app.on_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL));
    assert!(app.prefix_armed);
}

/// Welcome setup: the theme previews while cycling and is kept by ⏎; esc undoes the preview.
#[test]
fn welcome_card_sets_theme() {
    use noble::app::{Hit, Overlay};
    let key = |app: &mut App, code| app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    // ⏎ keeps the theme picked with →.
    let mut app = demo_app(110, 30);
    let before = app.theme.name;
    app.show_welcome();
    let text = render(&mut app, 110, 30);
    assert!(text.contains("SET UP") && text.contains("Theme") && text.contains("Prefix key"), "{text}");
    key(&mut app, KeyCode::Right);
    let picked = app.theme.name;
    assert_ne!(picked, before, "→ previews the next theme");
    assert!(render(&mut app, 110, 30).contains(app.theme.label));
    key(&mut app, KeyCode::Enter);
    assert!(app.overlay.is_none());
    assert_eq!(app.theme.name, picked);
    assert_eq!(app.cfg.general.theme, picked);

    // esc undoes the preview and leaves the config alone.
    let mut app = demo_app(110, 30);
    let cfg_theme = app.cfg.general.theme.clone();
    app.show_welcome();
    render(&mut app, 110, 30);
    // A click on the right arrow of the theme row steps it too.
    let arrow = find_hit(&app, |h| *h == Hit::WelcomeStep(0, 1)).expect("theme › arrow");
    click(&mut app, arrow.x, arrow.y);
    assert_ne!(app.theme.name, before);
    key(&mut app, KeyCode::Down);
    assert!(matches!(&app.overlay, Some(Overlay::Welcome(w)) if w.row == 1));
    key(&mut app, KeyCode::Esc);
    assert!(app.overlay.is_none());
    assert_eq!(app.theme.name, before);
    assert_eq!(app.cfg.general.theme, cfg_theme);
    assert!(app.ui_state.data.welcomed);
}

/// Welcome setup: esc undoes the terminal colors preview too, a config reload while the card
/// is open drops the previews (the file wins), and ⏎ keeps a custom shell and prefix the card
/// did not touch.
#[test]
fn welcome_card_undoes_previews() {
    use noble::app::{Overlay, WelcomeRow};
    let key = |app: &mut App, code| app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    let row_of = |app: &App, r: WelcomeRow| match &app.overlay {
        Some(Overlay::Welcome(w)) => w.rows().iter().position(|x| *x == r),
        _ => None,
    };
    // esc: the colors go back to the config value.
    let mut app = demo_app(110, 30);
    let colors = app.cfg.terminal.colors.clone();
    app.show_welcome();
    let row = row_of(&app, WelcomeRow::Colors).expect("colors row (the built-in schemes)");
    for _ in 0..row {
        key(&mut app, KeyCode::Down);
    }
    key(&mut app, KeyCode::Right);
    assert_ne!(app.cfg.terminal.colors, colors, "→ previews a scheme");
    key(&mut app, KeyCode::Esc);
    assert!(app.overlay.is_none());
    assert_eq!(app.cfg.terminal.colors, colors);

    // The config file changes while the card previews a theme and a scheme.
    let dir = std::env::temp_dir().join(format!("noble-welcome-reload-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("config.toml");
    std::fs::write(&file, "[general]\ntheme = \"synth\"\n").unwrap();
    let mut app = demo_app(110, 30);
    assert_ne!(app.theme.name, "synth");
    app.paths.config = file;
    app.show_welcome();
    key(&mut app, KeyCode::Right);
    for _ in 0..row {
        key(&mut app, KeyCode::Down);
    }
    key(&mut app, KeyCode::Right);
    app.reload_config(false);
    assert_eq!(app.theme.name, "synth", "the file's theme, not the preview");
    assert_eq!(app.cfg.terminal.colors, colors, "the file's colors, not the preview");
    assert!(
        matches!(&app.overlay, Some(Overlay::Welcome(w))
            if noble::theme::THEMES[w.theme].name == "synth" && w.row == row),
        "the card starts over from the file"
    );
    render(&mut app, 110, 30);
    key(&mut app, KeyCode::Esc);
    assert_eq!((app.theme.name, app.cfg.terminal.colors.as_str()), ("synth", colors.as_str()));
    let _ = std::fs::remove_dir_all(&dir);

    // A custom shell and prefix survive ⏎ when only the theme changed.
    let mut app = demo_app(110, 30);
    let shell = if cfg!(windows) { r"C:\Tools\myshell.exe" } else { "/opt/tools/myshell" };
    app.cfg.terminal.shell = shell.into();
    app.cfg.keys.prefix = "ctrl+q".into();
    app.show_welcome();
    if let Some(r) = row_of(&app, WelcomeRow::Shell) {
        let text = render(&mut app, 110, 30);
        assert!(text.contains("myshell"), "the custom shell is listed by name\n{text}");
        assert!(r > 0);
    }
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Enter);
    assert!(app.overlay.is_none());
    assert_eq!(app.cfg.terminal.shell, shell);
    assert_eq!(app.cfg.keys.prefix, "ctrl+q");
}

/// The welcome card lists launcher keys only for CLIs that are installed.
#[test]
fn welcome_card_lists_only_installed_launchers() {
    use noble::config::Launcher;
    let mut app = demo_app(110, 30);
    app.launchers = vec![
        (Launcher { key: "c".into(), name: "claude".into(), command: "claude".into(), show: true }, true),
        (Launcher { key: "x".into(), name: "codex".into(), command: "codex".into(), show: true }, false),
    ];
    app.show_welcome();
    let text = render(&mut app, 110, 30);
    assert!(text.contains("start Claude right there") && !text.contains("Codex"), "{text}");
    for l in &mut app.launchers {
        l.1 = false;
    }
    let text = render(&mut app, 110, 30);
    assert!(!text.contains("right there") && text.contains("command palette"), "{text}");
}

/// The launcher line drops whole launcher names that do not fit and counts them as "+N": at every size
/// the card is drawn at, the line never ends in a cut name (it used to stop at "… / Antigravit").
#[test]
fn welcome_card_launcher_line_drops_whole_names() {
    use noble::config::Launcher;
    const NAMES: [&str; 6] = ["Claude", "Codex", "Gemini", "Antigravity", "Opencode", "Cursor"];
    let keys = ["c", "x", "g", "a", "o", "u"];
    let commands = ["claude", "codex", "gemini", "antigravity", "opencode", "cursor"];
    let mut app = demo_app(110, 30);
    app.launchers = keys
        .iter()
        .zip(commands)
        .map(|(key, name)| {
            (Launcher { key: key.to_string(), name: name.into(), command: name.into(), show: true }, true)
        })
        .collect();
    app.show_welcome();
    let mut checked = 0;
    for (w, h) in SIZES {
        let text = render(&mut app, w, h);
        save(&format!("welcome-launchers-{w}x{h}"), &text);
        // The line runs to the card's right border; a cut line never reaches "right there".
        let Some(line) = text.lines().find(|l| l.contains("start ")) else { continue };
        checked += 1;
        let from = &line[line.find("start ").expect("start")..];
        let segment = from.split('│').next().unwrap_or_default().trim_end();
        assert!(segment.ends_with(" right there"), "{w}x{h}: the launcher line is cut: {segment:?}\n{text}");
        let list = segment.strip_prefix("start ").and_then(|s| s.strip_suffix(" right there")).expect("launcher line");
        // `Claude / Codex +4`: the names shown, then the count of the names left out.
        let (shown, hidden) = match list.rsplit_once('+') {
            Some((names, n)) => (names.trim_end(), n.parse::<usize>().expect("+N count")),
            None => (list, 0),
        };
        let names: Vec<&str> = if shown.is_empty() { Vec::new() } else { shown.split(" / ").collect() };
        assert!(names.iter().all(|n| NAMES.contains(n)), "{w}x{h}: a cut or unknown name in {segment:?}\n{text}");
        assert_eq!(names.len() + hidden, NAMES.len(), "{w}x{h}: the names left out are not counted in {segment:?}");
    }
    assert!(checked >= 3, "the launcher line is drawn at 110, 120 and 160 columns");
}

/// The text a card row draws after its key, up to the card's right border (`None` when no row shows the key).
fn card_text(text: &str, key: &str) -> Option<String> {
    let marker = format!("│  {key}");
    text.lines().find_map(|line| {
        let (_, after) = line.split_once(&marker)?;
        Some(after.split('│').next().unwrap_or_default().trim().to_string())
    })
}

/// The launcher line's text after "start " up to the card's right border (`None` when the card has no such line).
fn launcher_text(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (_, after) = line.split_once("start ")?;
        Some(after.split('│').next().unwrap_or_default().trim().to_string())
    })
}

/// `shown` is `full` as the card drew it: either whole, or cut after a whole word and ended with "…".
fn assert_word_cut(shown: &str, full: &str, ctx: &str) {
    if shown == full {
        return;
    }
    let head = shown.strip_suffix('…').unwrap_or_else(|| panic!("{ctx}: {shown:?} is cut without \"…\""));
    assert!(
        !head.is_empty() && full.starts_with(head) && full[head.len()..].starts_with(' '),
        "{ctx}: {shown:?} cuts a word of {full:?}"
    );
}

/// The card's texts are cut at a word boundary with "…", never inside a word, and a row with no room for its
/// first word is left out whole. Nine launchers at 120 columns used to cut the palette text after "is i"; at 45
/// columns the key column ended in a cut key and the descriptions in fragments.
#[test]
fn welcome_card_cuts_text_at_word_boundaries() {
    use noble::config::Launcher;
    const TERMINAL: &str = "open a terminal in the selected project";
    const PALETTE: &str = "command palette — everything is in there";
    const SHORTCUTS: &str = "every shortcut · ctrl+click opens links";
    const NAMES: [&str; 9] =
        ["Claude", "Codex", "Opencode", "Antigravity", "Oh-my-pi", "Freebuff", "Cursor", "Command code", "Cline"];
    let keys = ["c", "x", "e", "g", "b", "f", "u", "d", "l"];
    let commands =
        ["claude", "codex", "opencode", "antigravity", "oh-my-pi", "freebuff", "cursor", "command code", "cline"];
    let mut app = demo_app(120, 34);
    app.launchers = keys
        .iter()
        .zip(commands)
        .map(|(key, name)| {
            (Launcher { key: key.to_string(), name: name.into(), command: name.into(), show: true }, true)
        })
        .collect();
    app.show_welcome();
    // Nine keys take 27 columns of the key column, so the texts get 33: each is cut after the last whole word.
    let text = render(&mut app, 120, 34);
    save("welcome-cut-120x34", &text);
    assert_eq!(card_text(&text, "⏎").as_deref(), Some("open a terminal in the selected…"), "{text}");
    assert_eq!(card_text(&text, "alt+p").as_deref(), Some("command palette — everything is…"), "{text}");
    assert_eq!(card_text(&text, "?").as_deref(), Some("every shortcut · ctrl+click…"), "{text}");
    for (w, h) in [(120, 34), (90, 28), (76, 24), (56, 18), (45, 30), (45, 18), (45, 12), (30, 8)] {
        let text = render(&mut app, w, h);
        save(&format!("welcome-cut-{w}x{h}"), &text);
        for (key, full) in [("⏎", TERMINAL), ("alt+p", PALETTE), ("?", SHORTCUTS)] {
            // A drawn row always has its description (a bare key reads as a broken hint).
            let Some(shown) = card_text(&text, key) else { continue };
            assert!(!shown.is_empty(), "{w}x{h}: the {key} row has no description\n{text}");
            assert_word_cut(&shown, full, &format!("{w}x{h} {key}"));
        }
        // The launcher line names whole CLIs or counts the ones left out as "+N": no name is cut.
        let Some(line) = launcher_text(&text) else { continue };
        let body = line.strip_suffix(" right there").unwrap_or(&line);
        let (list, hidden) = match body.rsplit_once('+') {
            Some((list, n)) => (list.trim_end(), n.parse::<usize>().expect("+N count")),
            None => (body, 0),
        };
        let names: Vec<&str> = if list.is_empty() { Vec::new() } else { list.split(" / ").collect() };
        assert!(names.iter().all(|n| NAMES.contains(n)), "{w}x{h}: a cut or unknown name in {line:?}\n{text}");
        assert_eq!(names.len() + hidden, NAMES.len(), "{w}x{h}: names left out not counted in {line:?}");
    }
    // The narrowest card cuts its setup values at a word too: "Follow…", not "Follow th…".
    let text = render(&mut app, 30, 8);
    assert!(text.contains("Follow…") && text.contains("auto…"), "{text}");
    assert!(!text.contains("Follow th") && !text.contains("auto (p"), "{text}");
}
