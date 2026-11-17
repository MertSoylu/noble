//! The Agent Inbox: the list of agents that need you, its preview, the keys, the mouse and the quick answers.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use noble::app::{App, Hit, Overlay, QuickAnswer, View};
use noble::hooks::{Agent, HookRecord, NOTIFICATION, PERMISSION_REQUEST, PROMPT};
use noble::keys::{Action, Chord};
use noble::term::layout::PaneId;

use super::common::*;

/// The permission dialog of Claude Code with three options, as the pane's screen shows it.
const DIALOG3: &[u8] = b"\x1b[1m Bash command\x1b[m\r\n   npm test\r\n Do you want to proceed?\r\n \
    \x1b[36m\xe2\x9d\xaf 1. Yes\x1b[m\r\n   2. Yes, and don't ask again for npm test commands\r\n   \
    3. No, and tell Claude what to do differently (esc)\r\n";
/// The same without the "don't ask again" option: the No option is the second.
const DIALOG2: &[u8] =
    b" Do you want to make this edit to src/lib.rs?\r\n \x1b[36m\xe2\x9d\xaf 1. Yes\x1b[m\r\n   2. No, and tell Claude what to do differently (esc)\r\n";

fn key(app: &mut App, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn record(event: &str, message: Option<&str>, ago: i64, agent: Option<Agent>) -> HookRecord {
    HookRecord {
        event: event.into(),
        message: message.map(Into::into),
        ts: noble::util::unix_now() - ago,
        agent,
        session: None,
        subagents: 0,
    }
}

/// A tab whose pane runs a stand-in for an agent's TUI: no shell prompt comes back while the test runs (the prompt
/// would end the agent's records), and on Unix the terminal echoes what is typed to it.
fn agent_tab(app: &mut App, name: &str) -> PaneId {
    let command = fake_agent(&app.shell, "Claude Code");
    assert!(app.new_tab(std::env::temp_dir(), Some(&command), Some(name.into())));
    app.tabs.last().expect("tab").focus
}

/// Waits until the terminal echoed `digit` at the end of a line of the pane's screen (Unix only: a Windows console
/// program does not echo what it does not read, so the app-side effects are all that is checked there).
fn echoed(app: &mut App, pane: PaneId, digit: char) {
    if cfg!(unix) {
        pump_until(app, "the digit reached the program", |app| {
            app.panes[&pane].live_rows().iter().any(|r| r.trim_end().ends_with(digit))
        });
    }
}

/// Three tabs: a Claude permission prompt that has waited 7 minutes (its dialog on the screen), a Codex permission
/// request that has waited 90 seconds, and a Claude that works.
fn app_with_three(w: u16, h: u16) -> (App, [PaneId; 3]) {
    let mut app = demo_app(w, h);
    let panes = ["noble", "api", "docs"].map(|name| agent_tab(&mut app, name));
    app.panes[&panes[0]].parser().process(DIALOG3);
    app.panes[&panes[1]].parser().process(b"codex wants to run `cargo test`\r\nAllow? (y/n)\r\n");
    let records = [
        (panes[0], record(PERMISSION_REQUEST, Some("Claude asks to run Bash"), 7 * 60, None)),
        (panes[1], record(PERMISSION_REQUEST, Some("Codex asks to run a command"), 90, Some(Agent::Codex))),
        (panes[2], record(PROMPT, None, 3, None)),
    ];
    app.apply_hook_records(records.into_iter().collect());
    (app, panes)
}

#[test]
fn the_inbox_is_an_action_with_a_prefix_key_and_a_palette_entry() {
    let mut app = demo_app(120, 34);
    let key_i = Chord::parse("I").unwrap();
    assert_eq!(app.keymap.prefix_map.get(&key_i), Some(&Action::AgentInbox));
    assert_eq!(Action::from_id("agent_inbox"), Some(Action::AgentInbox));
    assert!(app.palette_items().iter().any(|i| i.title == "Agent Inbox"), "on every screen");
    // Through the prefix key.
    agent_tab(&mut app, "work");
    let prefix = app.keymap.prefix;
    app.on_key(KeyEvent::new(prefix.code, prefix.mods));
    app.on_key(KeyEvent::new(KeyCode::Char('I'), KeyModifiers::SHIFT));
    assert!(matches!(app.overlay, Some(Overlay::Inbox(_))), "prefix I opens the inbox");
}

#[test]
fn an_empty_inbox_says_so() {
    let mut app = demo_app(120, 34);
    app.run(Action::AgentInbox);
    for (w, h) in SIZES {
        let text = render(&mut app, w, h);
        save(&format!("inbox-empty-{w}x{h}"), &text);
        // A narrow box wraps the sentence over two lines.
        let said = if w >= 40 { "No agent is waiting for you" } else { "No agent is waiting" };
        assert!(text.contains(said), "{w}x{h}:\n{text}");
        assert!(text.contains("esc close"), "{w}x{h}:\n{text}");
    }
    key(&mut app, KeyCode::Enter);
    assert!(matches!(app.overlay, Some(Overlay::Inbox(_))), "⏎ has nowhere to go");
    key(&mut app, KeyCode::Esc);
    assert!(app.overlay.is_none());
}

#[test]
fn the_list_shows_who_waits_where_and_for_how_long() {
    let (mut app, panes) = app_with_three(120, 34);
    app.run(Action::AgentInbox);
    let text = render(&mut app, 120, 34);
    save("inbox-120x34", &text);
    assert!(text.contains("AGENT INBOX · 1/2"), "{text}");
    // Longest wait first: the Claude prompt (7 minutes), then Codex (90 seconds). The working agent is not listed.
    let claude = text.lines().position(|l| l.contains("claude") && l.contains("tab 1")).expect("claude row");
    let codex = text.lines().position(|l| l.contains("codex") && l.contains("tab 2")).expect("codex row");
    assert!(claude < codex, "{text}");
    assert!(!text.contains("tab 3"), "a working agent is not waiting:\n{text}");
    let row = text.lines().nth(claude).unwrap();
    assert!(row.contains("waiting 7m"), "{row}");
    assert!(text.lines().nth(codex).unwrap().contains("waiting 1m"), "{text}");
    assert!(row.contains("Claude asks to run Bash"), "the request is on the row: {row}");
    // The selected row's preview: what the pane shows, with the dialog's question and options.
    for line in ["Do you want to proceed?", "1. Yes", "3. No, and tell Claude"] {
        assert!(text.contains(line), "preview shows {line:?}:\n{text}");
    }
    // A quick answer is offered for the Claude permission prompt, with its chips and hints.
    assert!(text.contains("y yes") && text.contains("n no"), "{text}");
    assert!(find_hit(&app, |h| *h == Hit::InboxAnswer(QuickAnswer::Yes)).is_some());
    assert!(find_hit(&app, |h| *h == Hit::InboxAnswer(QuickAnswer::No)).is_some());
    assert!(find_hit(&app, |h| *h == Hit::InboxRow(panes[0])).is_some());
    assert!(find_hit(&app, |h| *h == Hit::InboxRow(panes[1])).is_some());
    assert!(find_hit(&app, |h| *h == Hit::InboxRow(panes[2])).is_none());

    // ↓ selects Codex: its screen is the preview, and there is no quick answer for another agent.
    key(&mut app, KeyCode::Down);
    let text = render(&mut app, 120, 34);
    assert!(text.contains("AGENT INBOX · 2/2"), "{text}");
    assert!(text.contains("Allow? (y/n)"), "{text}");
    assert!(!text.contains("Do you want to proceed?"), "{text}");
    assert!(!text.contains("y yes"), "jump only:\n{text}");
    assert!(find_hit(&app, |h| matches!(h, Hit::InboxAnswer(_))).is_none());
    // A quick key on a jump-only row sends nothing and says why.
    key(&mut app, KeyCode::Char('y'));
    assert_eq!(app.agents_waiting(), 2);
    assert!(toast_texts(&app).iter().any(|t| t.contains("no quick answer")), "{:?}", toast_texts(&app));
}

/// The inbox at every size, from a roomy window down to 30x8: nothing overflows, the way out stays.
#[test]
fn the_inbox_fits_every_size() {
    let (mut app, _) = app_with_three(160, 45);
    app.run(Action::AgentInbox);
    for (w, h) in SIZES {
        let text = render(&mut app, w, h);
        save(&format!("inbox-{w}x{h}"), &text);
        assert!(text.contains("AGENT INBOX"), "{w}x{h}:\n{text}");
        assert!(text.contains("esc close"), "{w}x{h}:\n{text}");
        assert!(text.contains("claude"), "{w}x{h}: the selected agent is named:\n{text}");
        assert_no_doubled_frame(&text);
    }
    // A narrow list keeps the agent and how long it waits.
    let text = render(&mut app, 30, 8);
    assert!(text.contains("claude") && text.contains("7m"), "{text}");
    // A window with room for the preview shows it; one without drops it and keeps the rows.
    assert!(render(&mut app, 76, 24).contains("Do you want to proceed?"));
    assert!(!render(&mut app, 30, 8).contains("Do you want to proceed?"));
    // Clicks inside the box never reach what is under it.
    assert!(find_hit(&app, |h| *h == Hit::Inert).is_some());
}

#[test]
fn enter_jumps_to_the_selected_pane_and_a_zoomed_tab_is_restored() {
    let (mut app, panes) = app_with_three(120, 34);
    app.view = View::Term(2);
    app.run(Action::AgentInbox);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert!(app.overlay.is_none());
    assert_eq!((app.view, app.tabs[1].focus), (View::Term(1), panes[1]));
    // The pane's tab shows another pane, zoomed: the pane must be visible.
    app.run(Action::SplitRight);
    let other = app.tabs[1].focus;
    assert_ne!(other, panes[1]);
    app.run(Action::Zoom);
    assert!(app.tabs[1].zoomed);
    app.view = View::Term(2);
    app.run(Action::AgentInbox);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert_eq!((app.view, app.tabs[1].focus), (View::Term(1), panes[1]));
    assert!(!app.tabs[1].zoomed);
}

#[test]
fn rows_select_on_the_first_click_and_jump_on_the_second() {
    let (mut app, panes) = app_with_three(120, 34);
    app.run(Action::AgentInbox);
    render(&mut app, 120, 34);
    let second = find_hit(&app, |h| *h == Hit::InboxRow(panes[1])).expect("codex row");
    click(&mut app, second.x + 3, second.y);
    let Some(Overlay::Inbox(st)) = &app.overlay else { panic!("a click on a row keeps the inbox open") };
    assert_eq!(st.pane, Some(panes[1]));
    let text = render(&mut app, 120, 34);
    assert!(text.contains("Allow? (y/n)"), "the preview follows the click:\n{text}");
    let second = find_hit(&app, |h| *h == Hit::InboxRow(panes[1])).expect("codex row");
    click(&mut app, second.x + 3, second.y);
    assert!(app.overlay.is_none());
    assert_eq!((app.view, app.tabs[1].focus), (View::Term(1), panes[1]));
}

/// Home's "needs you" chips open the inbox: the AI panel's top line on a wide window, the quota strip on a narrow one.
#[test]
fn home_chips_open_the_inbox() {
    let (mut app, _) = app_with_three(160, 45);
    for (w, h) in [(160, 45), (80, 24)] {
        app.overlay = None;
        app.view = View::Bridge;
        let text = render(&mut app, w, h);
        save(&format!("inbox-home-{w}x{h}"), &text);
        let chip = find_hit(&app, |h| *h == Hit::Inbox).unwrap_or_else(|| panic!("{w}x{h} has the chip:\n{text}"));
        let line = text.lines().nth(chip.y as usize).unwrap();
        assert!(line.contains("◆ 2 need you"), "{line}");
        click(&mut app, chip.x + 2, chip.y);
        assert!(matches!(app.overlay, Some(Overlay::Inbox(_))), "{w}x{h}");
    }
}

/// A quick answer types the number the dialog shows, into the pane, and the pane reads as working at once.
#[test]
fn a_quick_answer_types_the_number_the_screen_shows() {
    for (dialog, no_key, label) in [(DIALOG3, '3', "three options"), (DIALOG2, '2', "two options")] {
        let mut app = demo_app(120, 34);
        let pane = agent_tab(&mut app, "work");
        app.panes[&pane].parser().process(dialog);
        app.apply_hook_records(
            [(pane, record(PERMISSION_REQUEST, Some("Claude asks to run Bash"), 5, None))].into_iter().collect(),
        );
        assert_eq!(app.agents_waiting(), 1, "{label}");
        app.view = View::Bridge;
        app.run(Action::AgentInbox);
        let text = render(&mut app, 120, 34);
        assert!(text.contains("y yes") && text.contains("n no"), "{label}:\n{text}");

        // `n` is no: the number the screen shows for it.
        key(&mut app, KeyCode::Char('n'));
        assert_eq!(app.agents_waiting(), 0, "{label}: the pane reads as answered");
        echoed(&mut app, pane, no_key);
        // The list is empty now and the overlay stays.
        let text = render(&mut app, 120, 34);
        assert!(text.contains("No agent is waiting for you"), "{label}:\n{text}");
        assert!(toast_texts(&app).iter().any(|t| t.starts_with("answered no")), "{label}: {:?}", toast_texts(&app));
        app.overlay = None;
        app.view = View::Term(0);
        app.run(Action::CloseTab);
    }
}

/// Yes is the first option, and the chip answers the selected row with the mouse.
#[test]
fn the_yes_chip_answers_with_the_mouse() {
    let mut app = demo_app(120, 34);
    let pane = agent_tab(&mut app, "work");
    app.panes[&pane].parser().process(DIALOG3);
    app.apply_hook_records(
        [(pane, record(NOTIFICATION, Some("Claude needs your permission to use Bash"), 5, None))].into_iter().collect(),
    );
    app.run(Action::AgentInbox);
    render(&mut app, 120, 34);
    let chip = find_hit(&app, |h| *h == Hit::InboxAnswer(QuickAnswer::Yes)).expect("yes chip");
    click(&mut app, chip.x + 1, chip.y);
    assert_eq!(app.agents_waiting(), 0);
    echoed(&mut app, pane, '1');
    app.overlay = None;
    app.run(Action::CloseTab);
}

/// No quick answer unless the pane's screen still shows the dialog, or when the agent asks a question, whatever the
/// record says; the keys then send nothing to the pane.
#[test]
fn nothing_is_sent_unless_the_screen_and_the_record_agree() {
    let mut app = demo_app(120, 34);
    let panes = ["gone", "question", "codex"].map(|name| agent_tab(&mut app, name));
    // The dialog is off the screen (the agent's own output scrolled it away).
    app.panes[&panes[0]].parser().process(b"just some output\r\n");
    // Claude's question: its options are the agent's own, a "1." means nothing here.
    app.panes[&panes[1]].parser().process(DIALOG3);
    app.panes[&panes[2]].parser().process(DIALOG3);
    let records = [
        (panes[0], record(PERMISSION_REQUEST, Some("Claude asks to run Bash"), 30, None)),
        (panes[1], record(NOTIFICATION, Some("Claude asks a question"), 20, None)),
        (panes[2], record(PERMISSION_REQUEST, Some("Codex asks"), 10, Some(Agent::Codex))),
    ];
    app.apply_hook_records(records.into_iter().collect());
    app.run(Action::AgentInbox);
    for step in 0..3 {
        let text = render(&mut app, 120, 34);
        assert!(!text.contains("y yes"), "row {step} is jump only:\n{text}");
        assert!(find_hit(&app, |h| matches!(h, Hit::InboxAnswer(_))).is_none());
        for c in ['y', 'n', '1'] {
            key(&mut app, KeyCode::Char(c));
        }
        key(&mut app, KeyCode::Down);
    }
    assert_eq!(app.agents_waiting(), 3);
}

/// The list follows the records while it is open: a new agent that waits appears, one that stops leaves, and the
/// selection stays on its agent, or at its place when that agent leaves.
#[test]
fn the_open_inbox_follows_the_records() {
    let (mut app, panes) = app_with_three(120, 34);
    app.run(Action::AgentInbox);
    key(&mut app, KeyCode::Down);
    assert!(render(&mut app, 120, 34).contains("AGENT INBOX · 2/2"));
    // The working agent now asks too (the newest wait, so it is last): the selection stays on Codex.
    let mut records: std::collections::HashMap<PaneId, HookRecord> = [
        (panes[0], record(PERMISSION_REQUEST, Some("Claude asks to run Bash"), 7 * 60, None)),
        (panes[1], record(PERMISSION_REQUEST, Some("Codex asks to run a command"), 90, Some(Agent::Codex))),
        (panes[2], record(PERMISSION_REQUEST, Some("Claude asks to run Edit"), 2, None)),
    ]
    .into();
    app.apply_hook_records(records.clone());
    let text = render(&mut app, 120, 34);
    assert!(text.contains("AGENT INBOX · 2/3") && text.contains("tab 3"), "{text}");
    // Codex stops waiting: the same place (now the newest agent) is selected.
    records.insert(panes[1], record("stop", None, 0, Some(Agent::Codex)));
    app.apply_hook_records(records);
    let text = render(&mut app, 120, 34);
    assert!(text.contains("AGENT INBOX · 2/2"), "{text}");
    assert!(text.contains("Claude asks to run Edit"), "{text}");
}
