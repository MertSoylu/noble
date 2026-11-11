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
    assert!(text.contains("start Claude in the project") && !text.contains("Codex"), "{text}");
    for l in &mut app.launchers {
        l.1 = false;
    }
    let text = render(&mut app, 110, 30);
    assert!(!text.contains("start ") && text.contains("command palette: every action"), "{text}");
}

/// Up to three installed CLIs are named, with their keys: the line reads `start Claude / Codex in the project`.
#[test]
fn welcome_card_names_up_to_three_launchers() {
    use noble::config::Launcher;
    let mut app = demo_app(110, 30);
    app.launchers = [("c", "claude"), ("x", "codex")]
        .into_iter()
        .map(|(key, name)| (Launcher { key: key.into(), name: name.into(), command: name.into(), show: true }, true))
        .collect();
    app.show_welcome();
    let text = render(&mut app, 110, 30);
    assert!(text.contains("start Claude / Codex in the project"), "{text}");
    let row = text.lines().find(|l| l.contains("start Claude")).expect("the launcher row");
    assert!(row.contains("c  x"), "the keys sit beside their line: {row:?}");
}

/// With more launchers than the card names, the line keeps the first three keys and "…" and says "an AI CLI"
/// instead of listing them. At every size the card is drawn at, the line is whole or cut at a word.
#[test]
fn welcome_card_launcher_line_marks_the_launchers_it_does_not_name() {
    use noble::config::Launcher;
    const LINE: &str = "start an AI CLI in the project";
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
        // The row carries the keys and the line; a narrow card may leave the row out.
        let Some(row) = text.lines().find(|l| l.contains("start ")) else { continue };
        checked += 1;
        assert!(row.contains("c  x  g  …"), "{w}x{h}: the first three keys and \"…\" are not drawn: {row:?}\n{text}");
        let line = launcher_text(&text).expect("the launcher line");
        assert_word_cut(&format!("start {line}"), LINE, &format!("{w}x{h}"));
        // The CLIs are not listed by name.
        assert!(!row.contains("Claude") && !row.contains("Codex"), "{w}x{h}: {row:?}");
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
/// first word is left out whole. Nine launchers show three keys and "…"; at 45 columns the texts are cut after a
/// whole word, and the key column never ends in a cut key.
#[test]
fn welcome_card_cuts_text_at_word_boundaries() {
    use noble::config::Launcher;
    const TERMINAL: &str = "open a terminal in the project";
    const PALETTE: &str = "command palette: every action";
    const SHORTCUTS: &str = "all shortcuts";
    const LINE: &str = "start an AI CLI in the project";
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
    // The Shell row exists only with two installed shells; the card gets a fixed list so the test does not
    // depend on this machine ("" is auto, as in `shell_options`).
    let Some(noble::app::Overlay::Welcome(w)) = &mut app.overlay else { panic!("no welcome card") };
    w.shells = vec![String::new(), "bash".into(), "zsh".into()];
    w.shell = 0;
    // Nine launchers show as "c  x  e  …" (10 columns), so the texts keep 48 columns: each one is whole.
    let text = render(&mut app, 120, 34);
    save("welcome-cut-120x34", &text);
    assert_eq!(card_text(&text, "⏎").as_deref(), Some(TERMINAL), "{text}");
    assert_eq!(card_text(&text, "alt+p").as_deref(), Some(PALETTE), "{text}");
    assert_eq!(card_text(&text, "?").as_deref(), Some(SHORTCUTS), "{text}");
    assert_eq!(launcher_text(&text).as_deref(), Some("an AI CLI in the project"), "{text}");
    assert!(text.contains("c  x  e  …"), "{text}");
    for (w, h) in [(120, 34), (90, 28), (76, 24), (56, 18), (45, 30), (45, 18), (45, 12), (30, 8)] {
        let text = render(&mut app, w, h);
        save(&format!("welcome-cut-{w}x{h}"), &text);
        for (key, full) in [("⏎", TERMINAL), ("alt+p", PALETTE), ("?", SHORTCUTS)] {
            // A drawn row always has its description (a bare key reads as a broken hint).
            let Some(shown) = card_text(&text, key) else { continue };
            assert!(!shown.is_empty(), "{w}x{h}: the {key} row has no description\n{text}");
            assert_word_cut(&shown, full, &format!("{w}x{h} {key}"));
        }
        // The launcher line is the whole phrase, or cut after a whole word; no CLI is named on it.
        if let Some(line) = launcher_text(&text) {
            assert_word_cut(&format!("start {line}"), LINE, &format!("{w}x{h} launchers"));
            assert!(!line.contains("Claude") && !line.contains("Codex"), "{w}x{h}: {line:?}");
        }
    }
    // The narrowest card cuts its setup values at a word too: "Follow…", not "Follow th…".
    let text = render(&mut app, 30, 8);
    assert!(text.contains("Follow…") && text.contains("auto…"), "{text}");
    assert!(!text.contains("Follow th") && !text.contains("auto (p"), "{text}");
}

/// The first screen a new user sees: at 120×34 and 110×30 each explanation of the card is whole, so no row cuts
/// its text with "…" (the launcher keys keep their "…" for the launchers they leave out).
#[test]
fn welcome_card_explanations_are_whole_at_120_and_110() {
    use noble::config::Launcher;
    let keys = ["c", "x", "e", "g", "b", "f", "u", "d", "l"];
    let commands =
        ["claude", "codex", "opencode", "antigravity", "oh-my-pi", "freebuff", "cursor", "command code", "cline"];
    let mut app = demo_app(110, 30);
    app.launchers = keys
        .iter()
        .zip(commands)
        .map(|(key, name)| {
            (Launcher { key: key.to_string(), name: name.into(), command: name.into(), show: true }, true)
        })
        .collect();
    app.show_welcome();
    for (w, h) in [(120, 34), (110, 30)] {
        let text = render(&mut app, w, h);
        assert_eq!(card_text(&text, "⏎").as_deref(), Some("open a terminal in the project"), "{w}x{h}\n{text}");
        assert_eq!(card_text(&text, "alt+p").as_deref(), Some("command palette: every action"), "{w}x{h}\n{text}");
        assert_eq!(card_text(&text, "?").as_deref(), Some("all shortcuts"), "{w}x{h}\n{text}");
        assert_eq!(launcher_text(&text).as_deref(), Some("an AI CLI in the project"), "{w}x{h}\n{text}");
    }
}
