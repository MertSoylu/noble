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
    // The note sits on a row of its own: the dimmed Home screen behind the overlay shows nothing on it.
    assert!(text.lines().any(|l| l.trim() == "enlarge window"), "{text}");
    for (w, h) in [(1, 1), (2, 3), (12, 5), (29, 40), (40, 5)] {
        render(&mut app, w, h);
    }
    // Big enough again: the key list is back.
    assert!(render(&mut app, 110, 30).contains("KEYBOARD REFERENCE"));
}

/// No key of the keyboard reference runs into its description: at every audited size the key column
/// is wider than the key it shows, so at least two blanks come before the text. A key too long for
/// its column is cut with "…" instead.
#[test]
fn help_keys_keep_a_gap_from_their_text() {
    use noble::app::Overlay;
    for (w, h) in [(160, 45), (120, 34), (110, 30), (90, 28), (76, 24), (56, 18), (30, 8)] {
        let mut app = demo_app(w, h);
        app.run(Action::Help);
        let text = render(&mut app, w, h);
        let Some(geo) = app.help_geometry("") else {
            assert!(text.contains("enlarge window"), "{w}x{h}:\n{text}");
            continue;
        };
        // Every scroll position shows other rows: each one is checked on the screen it is drawn on.
        for scroll in 0..=geo.max_scroll() {
            if let Some(Overlay::Help(st)) = &mut app.overlay {
                st.scroll = scroll;
            }
            let text = render(&mut app, w, h);
            for line in geo.columns.iter().flat_map(|col| col.iter().skip(scroll).take(geo.rows)) {
                let noble::app::help::Line::Row { key, desc, .. } = line else { continue };
                let key_shown = noble::util::truncate(key, usize::from(geo.key_w) - noble::app::help::KEY_GAP);
                // The key as drawn, then at least two blanks, then the start of the description.
                let head: String = desc.chars().take(3).collect();
                let gap_ok = text.lines().any(|l| {
                    l.match_indices(key_shown.as_str()).any(|(i, _)| {
                        let rest = &l[i + key_shown.len()..];
                        let after = rest.trim_start_matches(' ');
                        rest.len() - after.len() >= 2 && after.starts_with(&head)
                    })
                });
                assert!(gap_ok, "{w}x{h}: `{key}` runs into its text `{desc}`:\n{text}");
            }
        }
        // The first render is at scroll 0: the MOUSE section is in view at 160x45.
        if (w, h) == (160, 45) {
            assert!(text.contains("double-click title  zoom / restore the pane"), "{text}");
        }
    }
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
    let wide_text = render(&mut wide, 170, 45);
    let geo = wide.help_geometry("").expect("fits");
    assert_eq!(geo.columns.len(), 3);
    assert_eq!(geo.max_scroll(), 0, "three columns need no scrolling at 170x45");
    for name in ["PREFIX", "GLOBAL", "HOME", "SYSTEM", "MOUSE"] {
        assert!(wide_text.contains(name), "{name} is drawn:\n{wide_text}");
    }
    assert!(wide_text.contains("type to filter · ⏎ run · esc close"), "{wide_text}");

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
    assert_eq!(st.all[0].cmd, PaletteCmd::Notice { text: "third note".into(), target: None });
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

/// Two terminal tabs at 120 columns: the first with one pane, the second split in two. Returns the second tab's
/// first pane and its split (the one with focus).
fn two_tabs_second_split(app: &mut App) -> (noble::term::layout::PaneId, noble::term::layout::PaneId) {
    app.new_tab(std::env::temp_dir(), None, Some("first".into()));
    app.new_tab(std::env::temp_dir(), None, Some("second".into()));
    let first = app.tabs[1].focus;
    app.run(Action::SplitRight);
    (first, app.tabs[1].focus)
}

/// The agent in `pane` needs you while its tab is in the background (a hook record, as Claude writes it).
fn needs_you(app: &mut App, pane: noble::term::layout::PaneId) {
    let now = chrono::Utc::now().timestamp();
    app.apply_hook_records(std::collections::HashMap::from([(pane, hook_rec("notification", now, 0))]));
}

/// Clicks the notification whose text is `text` (it must be on screen).
fn click_toast(app: &mut App, text: &str) {
    render(app, 120, 30);
    let id = app.toasts.iter().find(|t| t.text == text).expect("toast on screen").id;
    let rect = find_hit(app, |h| matches!(h, noble::app::Hit::Toast(x) if *x == id)).expect("toast hit");
    click(app, rect.x + 1, rect.y);
    assert!(!app.toasts.iter().any(|t| t.id == id), "a click dismisses the notification");
}

/// "Claude needs you" from a background pane: its toast leads to that tab and pane, and the click dismisses it.
#[test]
fn clicking_a_pane_toast_goes_to_its_pane() {
    use noble::app::{ToastTarget, View};
    let mut app = demo_app(120, 30);
    let (first, other) = two_tabs_second_split(&mut app);
    app.view = View::Term(0);
    app.toasts.clear();
    needs_you(&mut app, other);
    let text = app.toasts[0].text.clone();
    assert_eq!(app.toasts[0].target, Some(ToastTarget::Pane { tab: app.tabs[1].id, pane: other }));
    click_toast(&mut app, &text);
    assert_eq!(app.view, View::Term(1), "the pane's tab is shown");
    assert_eq!(app.tabs[1].focus, other, "and its pane focused");
    assert_ne!(app.tabs[1].focus, first);
}

/// A click on a pane's notification restores a zoomed tab that shows another pane; a pane that is already the
/// zoomed one keeps its zoom.
#[test]
fn pane_toast_click_keeps_the_zoom_consistent() {
    use noble::app::View;
    let mut app = demo_app(120, 30);
    let (first, other) = two_tabs_second_split(&mut app);
    app.view = View::Term(1);
    app.run(Action::Zoom);
    assert!(app.tabs[1].zoomed && app.tabs[1].focus == other);
    // The hidden pane needs you: the zoom goes, the pane is shown.
    app.view = View::Term(0);
    app.toasts.clear();
    needs_you(&mut app, first);
    let text = app.toasts[0].text.clone();
    click_toast(&mut app, &text);
    assert_eq!((app.view, app.tabs[1].focus), (View::Term(1), first));
    assert!(!app.tabs[1].zoomed, "the pane must be visible");
    // The zoomed pane itself needs you: it stays zoomed.
    app.tabs[1].focus = other;
    app.run(Action::Zoom);
    assert!(app.tabs[1].zoomed);
    app.view = View::Term(0);
    app.toasts.clear();
    needs_you(&mut app, other);
    let text = app.toasts[0].text.clone();
    click_toast(&mut app, &text);
    assert_eq!((app.view, app.tabs[1].focus), (View::Term(1), other));
    assert!(app.tabs[1].zoomed);
}

/// A notification whose pane or tab has closed only dismisses: nothing else moves.
#[test]
fn closed_pane_or_tab_toast_only_dismisses() {
    use noble::app::View;
    let mut app = demo_app(120, 30);
    let (first, other) = two_tabs_second_split(&mut app);
    // The split pane closes; its notification is still on screen.
    app.view = View::Term(0);
    app.toasts.clear();
    needs_you(&mut app, other);
    let text = app.toasts[0].text.clone();
    app.view = View::Term(1);
    app.run(Action::ClosePane);
    assert_eq!(app.tabs[1].panes(), vec![first]);
    app.view = View::Bridge;
    click_toast(&mut app, &text);
    assert_eq!(app.view, View::Bridge, "the pane is gone: no view change");
    // The whole tab closes: the same goes for its pane's notification.
    app.view = View::Term(0);
    app.toasts.clear();
    needs_you(&mut app, first);
    let text = app.toasts[0].text.clone();
    app.view = View::Term(1);
    app.run(Action::CloseTab);
    assert_eq!(app.tabs.len(), 1);
    app.view = View::Bridge;
    click_toast(&mut app, &text);
    assert_eq!(app.view, View::Bridge, "the tab is gone: no view change");
}

/// The notification list: Enter on a pane's notification goes to its pane; ctrl+y copies it instead, and a
/// notification without a pane is copied by Enter. The footer shows both choices.
#[test]
fn notification_list_goes_to_the_pane_and_copies() {
    use noble::app::{Overlay, PaletteCmd, ToastTarget, View};
    let mut app = demo_app(120, 30);
    let (_, other) = two_tabs_second_split(&mut app);
    app.view = View::Term(0);
    app.toasts.clear();
    needs_you(&mut app, other);
    app.toast(ToastLevel::Info, "plain note");
    let key = |app: &mut App, code: KeyCode, mods: KeyModifiers| app.on_key(KeyEvent::new(code, mods));
    // Opens the list and selects the row whose command matches `want`.
    let select = |app: &mut App, want: fn(&PaletteCmd) -> bool| {
        app.run(Action::Notifications);
        let Some(Overlay::Palette(st)) = &mut app.overlay else { panic!("no list") };
        st.selected = st.all.iter().position(|i| want(&i.cmd)).expect("row");
    };
    let to_pane = |c: &PaletteCmd| matches!(c, PaletteCmd::Notice { target: Some(ToastTarget::Pane { .. }), .. });
    let plain = |c: &PaletteCmd| matches!(c, PaletteCmd::Notice { target: None, .. });

    select(&mut app, to_pane);
    let text = render(&mut app, 120, 30);
    assert!(text.contains("go to") && text.contains("ctrl+y copy"), "{text}");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(app.overlay.is_none(), "Enter closes the list");
    assert_eq!((app.view, app.tabs[1].focus), (View::Term(1), other), "Enter shows the pane");

    // Ctrl+Y copies the same notification and leaves the view alone.
    app.view = View::Term(0);
    select(&mut app, to_pane);
    key(&mut app, KeyCode::Char('y'), KeyModifiers::CONTROL);
    assert!(app.overlay.is_none(), "copying closes the list too");
    assert_eq!(app.view, View::Term(0));
    let texts = toast_texts(&app);
    assert!(texts.iter().any(|t| t == "copied" || t == "clipboard unavailable"), "{texts:?}");

    // A notification without a pane: Enter copies, and the footer offers no "go to".
    select(&mut app, plain);
    let text = render(&mut app, 120, 30);
    assert!(text.contains("⏎ copy") && !text.contains("go to") && !text.contains("ctrl+y"), "{text}");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.view, View::Term(0));
    let texts = toast_texts(&app);
    assert!(texts.iter().any(|t| t == "copied" || t == "clipboard unavailable"), "{texts:?}");

    // The pane has closed since: the footer offers "copy" and Enter copies instead of doing nothing.
    app.view = View::Term(1);
    app.run(Action::ClosePane);
    app.view = View::Term(0);
    app.toasts.clear();
    select(&mut app, to_pane);
    let text = render(&mut app, 120, 30);
    assert!(text.contains("⏎ copy") && !text.contains("go to"), "{text}");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.view, View::Term(0));
    let texts = toast_texts(&app);
    assert!(texts.iter().any(|t| t == "copied" || t == "clipboard unavailable"), "{texts:?}");

    // The list stays readable at every size; the audit dumps show the footers.
    for (w, h) in SIZES {
        app.run(Action::Notifications);
        let Some(Overlay::Palette(st)) = &mut app.overlay else { panic!("no list") };
        st.selected = st.all.iter().position(|i| to_pane(&i.cmd)).expect("row");
        let text = render(&mut app, w, h);
        save(&format!("notifications-list-{w}x{h}"), &text);
        app.overlay = None;
    }
}
