//! Confirm dialogs, the help overlay, prompts and toasts.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use noble::app::{App, ToastLevel};
use noble::keys::Action;
use std::time::Duration;

use super::common::*;

/// Add-project prompt: an invalid path keeps the prompt open with the text and the error inside it;
/// the cursor edits in the middle of the text.
#[test]
fn prompt_validates_on_enter_and_edits_at_the_cursor() {
    use noble::app::Overlay;
    let k = |app: &mut App, code: KeyCode| app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    let mut app = demo_app(110, 30);
    app.open_add_prompt(noble::app::AddMode::Project);
    let empty = render(&mut app, 110, 30);
    assert!(empty.contains("path, e.g."), "placeholder: {empty}");
    for c in "Z:/nope/here".chars() {
        k(&mut app, KeyCode::Char(c));
    }
    k(&mut app, KeyCode::Enter);
    let Some(Overlay::Prompt(p)) = &app.overlay else { panic!("prompt closed on an invalid path") };
    assert_eq!(p.value, "Z:/nope/here");
    assert!(p.error.as_deref().is_some_and(|e| e.contains("not a folder")));
    let text = render(&mut app, 110, 30);
    assert!(text.contains("not a folder: Z:/nope/here"), "{text}");
    for size in [(60, 12), (30, 8)] {
        render(&mut app, size.0, size.1);
    }
    // Editing clears the error; Home / Right / Delete / typing work at the cursor.
    k(&mut app, KeyCode::Home);
    k(&mut app, KeyCode::Right);
    k(&mut app, KeyCode::Delete);
    for c in "ab".chars() {
        k(&mut app, KeyCode::Char(c));
    }
    k(&mut app, KeyCode::End);
    k(&mut app, KeyCode::Left);
    k(&mut app, KeyCode::Backspace);
    let Some(Overlay::Prompt(p)) = &app.overlay else { panic!("prompt gone") };
    assert_eq!(p.value, "Zab/nope/hee");
    assert!(p.error.is_none());
    // A valid folder closes the prompt.
    let dir = std::env::temp_dir();
    if let Some(Overlay::Prompt(p)) = &mut app.overlay {
        p.clear();
        p.insert(&dir.display().to_string());
    }
    k(&mut app, KeyCode::Enter);
    assert!(app.overlay.is_none(), "valid folder submits");
}

/// The text inside the box whose title row holds `title`: its rows (up to the "confirm" button row)
/// joined with single spaces.
fn box_text(text: &str, title: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let top = lines.iter().position(|l| l.contains(&format!("╭ {title} "))).expect("box title row");
    let start = lines[top].find(&format!("╭ {title} ")).unwrap();
    let left = lines[top][..start].chars().count();
    let right = left + lines[top][start..].chars().position(|c| c == '╮').expect("box corner");
    let mut parts = Vec::new();
    for line in &lines[top + 1..] {
        if line.contains(" confirm") {
            break;
        }
        let inner: String = line.chars().skip(left + 1).take(right - left - 1).collect();
        if !inner.trim().is_empty() {
            parts.push(inner.trim().to_string());
        }
    }
    parts.join(" ")
}

/// Help at a size too small for the key list says so; nothing panics down to a 1×1 terminal.
#[test]
fn help_at_a_tiny_size_says_so() {
    let mut app = demo_app(110, 30);
    app.run(Action::Help);
    let text = render(&mut app, 30, 8);
    assert!(text.contains("enlarge window"), "{text}");
    for (w, h) in [(1, 1), (2, 3), (12, 5), (29, 40), (40, 5)] {
        render(&mut app, w, h);
    }
    // Big enough again: the key list is back.
    assert!(render(&mut app, 110, 30).contains("KEYBOARD REFERENCE"));
}

/// The keyboard reference: two columns on a wide screen with no scrolling; one column that scrolls
/// on a narrow one. Typed keys filter the rows, esc clears the filter before it closes, and ⏎ or a
/// click runs the selected row's action.
#[test]
fn help_has_columns_a_filter_and_runs_rows() {
    use noble::app::{Hit, Overlay};
    let key = |app: &mut App, c: KeyCode| app.on_key(KeyEvent::new(c, KeyModifiers::NONE));

    let mut app = demo_app(160, 45);
    app.run(Action::Help);
    let text = render(&mut app, 160, 45);
    save("help-160x45", &text);
    let geo = app.help_geometry("").expect("the reference fits at 160x45");
    assert_eq!(geo.columns.len(), 2, "two columns from 120 columns");
    assert!(matches!(geo.columns[1].first(), Some(noble::app::help::Line::Header(_))), "a section starts column 2");
    // The sections are ~88 lines: two columns of 40 rows still scroll a little.
    assert!(geo.max_scroll() > 0 && geo.max_scroll() < 20, "{geo:?}");
    for (w, h) in [(110, 30), (60, 20), (30, 8)] {
        save(&format!("help-{w}x{h}"), &render(&mut app, w, h));
    }
    // Three columns (from 170) hold the whole list at 170x45.
    let mut wide = demo_app(170, 45);
    wide.run(Action::Help);
    render(&mut wide, 170, 45);
    let geo = wide.help_geometry("").expect("fits");
    assert_eq!(geo.columns.len(), 3);
    assert_eq!(geo.max_scroll(), 0, "three columns need no scrolling at 170x45");
    for name in ["PREFIX", "GLOBAL", "HOME", "SYSTEM", "MOUSE"] {
        assert!(text.contains(name), "{name} is drawn:\n{text}");
    }
    assert!(text.contains("type to filter · ⏎ run · esc close"), "{text}");

    // A narrow screen keeps one column, and it scrolls with the selection.
    let mut narrow = demo_app(110, 30);
    narrow.run(Action::Help);
    render(&mut narrow, 110, 30);
    assert_eq!(narrow.help_geometry("").expect("fits").columns.len(), 1);
    for _ in 0..6 {
        key(&mut narrow, KeyCode::PageDown);
    }
    let text = render(&mut narrow, 110, 30);
    let Some(Overlay::Help(st)) = &narrow.overlay else { panic!("help is open") };
    assert!(st.scroll > 0 && st.selected > 40, "{st:?}");
    assert!(text.contains("▌"), "the selected row is marked and on screen:\n{text}");

    // Typing filters: only the sections with a matching row stay, and the title shows the query.
    let mut app = demo_app(160, 45);
    app.run(Action::Help);
    render(&mut app, 160, 45);
    for c in "split".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    let text = render(&mut app, 160, 45);
    save("help-filtered-160x45", &text);
    assert!(text.contains("KEYBOARD REFERENCE · /split"), "{text}");
    assert!(text.contains("split right"), "{text}");
    assert!(!text.contains("HOME"), "a section without a match is not drawn:\n{text}");
    let all = app.help_actions("").len();
    let split = app.help_actions("split").len();
    assert!(split > 0 && split < all);
    // Backspace widens the filter again.
    key(&mut app, KeyCode::Backspace);
    for _ in 0..4 {
        key(&mut app, KeyCode::Backspace);
    }
    let text = render(&mut app, 160, 45);
    assert!(text.contains("HOME") && text.contains("KEYBOARD REFERENCE") && !text.contains("/split"), "{text}");
    // Esc clears the filter first; the next esc closes.
    for c in "split".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Esc);
    assert!(matches!(&app.overlay, Some(Overlay::Help(st)) if st.query.is_empty()));
    key(&mut app, KeyCode::Esc);
    assert!(app.overlay.is_none());

    // ⏎ runs the selected row's action and closes the overlay.
    app.run(Action::Help);
    render(&mut app, 160, 45);
    for c in "palette".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    assert_eq!(app.help_actions("palette").first(), Some(&Action::Palette));
    render(&mut app, 160, 45);
    key(&mut app, KeyCode::Enter);
    assert!(matches!(app.overlay, Some(Overlay::Palette(_))), "⏎ opens the command palette");

    // A click on a row runs it too.
    app.overlay = None;
    app.run(Action::Help);
    render(&mut app, 160, 45);
    let idx = app.help_actions("").iter().position(|a| *a == Action::Palette).expect("palette row");
    let row = find_hit(&app, |h| *h == Hit::HelpRow(idx)).expect("the row is clickable");
    click(&mut app, row.x + 1, row.y);
    assert!(matches!(app.overlay, Some(Overlay::Palette(_))), "a click opens the command palette");
}

/// A long question wraps over several rows: the whole text and the buttons stay visible, also at 30x8.
#[test]
fn confirm_wraps_a_long_question() {
    use noble::app::{Confirm, ConfirmAction, Overlay};
    let body = "cargo watch --exec 'test --workspace --all-features' is still running in this tab — close it anyway?";
    let mut app = demo_app(110, 30);
    app.overlay =
        Some(Overlay::Confirm(Confirm { title: "CLOSE PANE".into(), body: body.into(), action: ConfirmAction::Quit }));
    let text = render(&mut app, 110, 30);
    save("confirm-wrap-110x30", &text);
    assert_eq!(box_text(&text, "CLOSE PANE"), body, "{text}");
    assert!(text.contains("confirm") && text.contains("cancel"), "{text}");
    let small = render(&mut app, 30, 8);
    save("confirm-wrap-30x8", &small);
    // Too narrow for the button words: the keys and their buttons stay.
    assert!(small.contains(" y ") && small.contains("n/esc"), "{small}");
    assert_no_doubled_frame(&small);
    for (w, h) in [(1, 1), (3, 2), (10, 4)] {
        render(&mut app, w, h);
    }
}

/// A narrow confirmation: the chips stay inside the frame (the n/esc chip is left out when it does not
/// fit) and a question that does not fit ends with "…" on its last visible line.
#[test]
fn confirm_narrow_keeps_chips_inside_and_marks_the_cut() {
    use noble::app::{Confirm, ConfirmAction, Overlay};
    let body = "cargo watch --exec 'test --workspace --all-features' is still running in this tab — close it anyway?";
    let mut app = demo_app(110, 30);
    for (w, h) in [(16u16, 8u16), (30, 8)] {
        app.overlay = None;
        let (_, plain) = render_buf(&mut app, w, h);
        app.overlay = Some(Overlay::Confirm(Confirm {
            title: "CLOSE PANE".into(),
            body: body.into(),
            action: ConfirmAction::Quit,
        }));
        let (text, buf) = render_buf(&mut app, w, h);
        save(&format!("confirm-narrow-{w}x{h}"), &text);
        // The box: centered, as wide as the window allows (at most 52), the frame's right column last.
        let box_w = (w - 2).min(52);
        let left = (w - box_w) / 2;
        let right = left + box_w - 1;
        // Nothing is drawn right of the frame: the cells past its ring match the screen without the box.
        for y in 0..h {
            for x in right + 2..w {
                assert_eq!(
                    buf[(x, y)].symbol(),
                    plain[(x, y)].symbol(),
                    "drawn past the frame at ({x},{y}) {w}x{h}:\n{text}"
                );
            }
        }
        let top = (0..h).find(|&y| buf[(left, y)].symbol() == "╭").expect("box top");
        let bottom = (top..h).find(|&y| buf[(left, y)].symbol() == "╰").expect("box bottom");
        let row = |y: u16| -> String { (left + 1..right).map(|x| buf[(x, y)].symbol()).collect() };
        let buttons = row(bottom - 1);
        assert!(buttons.contains(" y "), "yes chip at {w}x{h}: {text}");
        // The question is cut: its last visible line ends with "…", and what is shown is the start of it.
        let shown: Vec<String> = (top + 1..bottom - 1).map(row).filter(|l| !l.trim().is_empty()).collect();
        let shown = shown.iter().map(|l| l.trim()).collect::<Vec<_>>().join(" ");
        assert!(shown.ends_with('…'), "the cut question has no ellipsis at {w}x{h}: {shown:?}\n{text}");
        let words: String = shown.trim_end_matches('…').chars().filter(|c| *c != ' ').collect();
        assert!(body.replace(' ', "").starts_with(&words), "{shown:?}");
        assert!(words.len() > 10, "too little of the question is shown at {w}x{h}: {shown:?}");
    }
    // At 16 columns the n/esc chip does not fit inside the frame: it is left out, and so is its click target.
    app.overlay =
        Some(Overlay::Confirm(Confirm { title: "CLOSE PANE".into(), body: body.into(), action: ConfirmAction::Quit }));
    let text = render(&mut app, 16, 8);
    assert!(!text.contains("n/esc"), "{text}");
    assert!(find_hit(&app, |h| *h == noble::app::Hit::ConfirmNo).is_none(), "no cancel target when it is not drawn");
    let text = render(&mut app, 30, 8);
    assert!(text.contains("n/esc"), "{text}");
}

/// The wrapping keeps words whole and cuts a word that is wider than the box.
#[test]
fn wrapped_confirm_text_keeps_whole_words() {
    let mut app = demo_app(110, 30);
    app.overlay = Some(noble::app::Overlay::Confirm(noble::app::Confirm {
        title: "WIDE".into(),
        body: "x".repeat(120),
        action: noble::app::ConfirmAction::Quit,
    }));
    let text = render(&mut app, 110, 30);
    // Cut pieces are joined with a space by `box_text`: the letters themselves must all be there.
    assert_eq!(box_text(&text, "WIDE").replace(' ', ""), "x".repeat(120), "{text}");
}

/// Moves the toasts' start back, as if they had been on screen for `secs` seconds.
fn age_toasts(app: &mut App, secs: u64) {
    for t in &mut app.toasts {
        t.born = t.born.checked_sub(Duration::from_secs(secs)).expect("clock far enough along");
    }
}

/// A config error keeps its line and column, and stays on screen until it is clicked away.
#[test]
fn config_error_toast_stays_until_dismissed() {
    use noble::app::ToastTarget;
    let mut app = demo_app(110, 30);
    let dir = std::env::temp_dir().join(format!("noble-config-toast-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::write(&path, "[general]\ntheme = \"ice\"\n\n[ai]\nenabled = \"yes\"\n").unwrap();
    app.paths.config = path;
    app.toasts.clear();
    app.reload_config(false);
    let toast = app.toasts.last().expect("config error toast");
    assert!(toast.text.starts_with("config line 5, column 11: "), "{}", toast.text);
    assert!(!toast.text.contains('\n'));
    assert_eq!(toast.target, Some(ToastTarget::Config { line: Some(5) }));
    assert!(toast.expires_at().is_none(), "an error does not expire");
    age_toasts(&mut app, 600);
    app.tick();
    assert_eq!(toast_texts(&app).len(), 1, "still on screen after ten minutes");
    let text = render(&mut app, 110, 30);
    assert!(text.contains("config line 5, column 11"), "{text}");
    // The history keeps the whole message.
    assert!(app.notices.back().is_some_and(|n| n.text.starts_with("config line 5")));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Errors stay until dismissed; the other notifications leave once their time is up.
#[test]
fn error_toast_does_not_expire() {
    let mut app = demo_app(110, 30);
    app.toasts.clear();
    app.toast(ToastLevel::Error, "an error that must stay");
    app.toast(ToastLevel::Info, "a note that leaves");
    age_toasts(&mut app, 60);
    app.tick();
    assert_eq!(toast_texts(&app), vec!["an error that must stay"]);
}

/// A click on a notification dismisses it and leaves the others alone.
#[test]
fn clicking_a_toast_removes_it() {
    let mut app = demo_app(110, 30);
    app.toasts.clear();
    app.toast(ToastLevel::Error, "first, an error");
    app.toast(ToastLevel::Info, "second note");
    render(&mut app, 110, 30);
    let id = app.toasts[0].id;
    let rect = find_hit(&app, |h| matches!(h, noble::app::Hit::Toast(x) if *x == id)).expect("toast hit");
    click(&mut app, rect.x + 1, rect.y);
    assert_eq!(toast_texts(&app), vec!["second note"]);
    // Adding a toast does not change the ids of the ones before it.
    render(&mut app, 110, 30);
    let id = app.toasts[0].id;
    app.toast(ToastLevel::Info, "third note");
    render(&mut app, 110, 30);
    let rect = find_hit(&app, |h| matches!(h, noble::app::Hit::Toast(x) if *x == id)).expect("toast hit");
    click(&mut app, rect.x + 1, rect.y);
    assert_eq!(toast_texts(&app), vec!["third note"]);
}

/// The pointer on a notification pauses its countdown; once it leaves, the countdown goes on.
#[test]
fn hovered_toast_waits_for_the_pointer_to_leave() {
    use crossterm::event::MouseEventKind;
    let mut app = demo_app(110, 30);
    app.toasts.clear();
    app.toast(ToastLevel::Info, "hover me");
    render(&mut app, 110, 30);
    let id = app.toasts[0].id;
    let rect = find_hit(&app, |h| matches!(h, noble::app::Hit::Toast(x) if *x == id)).expect("toast hit");
    age_toasts(&mut app, 60);
    mouse(&mut app, MouseEventKind::Moved, rect.x + 1, rect.y);
    app.tick();
    assert_eq!(toast_texts(&app).len(), 1, "a toast under the pointer stays");
    mouse(&mut app, MouseEventKind::Moved, 0, 0);
    app.tick();
    assert!(app.toasts.is_empty(), "its time is up once the pointer has left");
}

/// The `notifications` action lists the past notifications newest first; running one copies its text.
#[test]
fn notifications_action_lists_past_toasts() {
    use noble::app::{Overlay, PaletteCmd};
    let mut app = demo_app(110, 30);
    app.toast(ToastLevel::Info, "first note");
    app.toast(ToastLevel::Error, "second, an error");
    app.toast(ToastLevel::Ok, "third note");
    app.run(Action::Notifications);
    let Some(Overlay::Palette(st)) = &app.overlay else { panic!("no palette") };
    let titles: Vec<&str> = st.all.iter().map(|i| i.title.as_str()).collect();
    assert!(titles.len() >= 3, "{titles:?}");
    assert!(titles[0].ends_with("third note"), "{titles:?}");
    assert!(titles[1].ends_with("second, an error"), "{titles:?}");
    assert!(titles[2].ends_with("first note"), "{titles:?}");
    assert_eq!(titles[0].as_bytes()[2], b':', "title starts with the time: {}", titles[0]);
    assert_eq!(st.all[0].cmd, PaletteCmd::CopyNotice("third note".into()));
    let cmd = st.all[0].cmd.clone();
    app.overlay = None;
    app.run_palette(cmd);
    let texts = toast_texts(&app);
    assert!(texts.iter().any(|t| t == "copied" || t == "clipboard unavailable"), "{texts:?}");

    // The history keeps the last 50 notifications only.
    for i in 0..60 {
        app.toast(ToastLevel::Info, format!("note {i}"));
    }
    assert_eq!(app.notices.len(), noble::app::NOTICE_HISTORY);
    assert_eq!(app.notices.back().map(|n| n.text.as_str()), Some("note 59"));
    assert_eq!(app.notices.front().map(|n| n.text.as_str()), Some("note 10"));
}
