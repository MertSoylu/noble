//! Claude hook states, agent sessions and their pane and tab markers.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use noble::app::View;
use noble::event::AppEvent;
use noble::keys::Action;
use std::time::{Duration, Instant};

use super::common::*;

/// Claude Code hook: when a background session asks for permission the tab is
/// marked, the session list on Home shows "needs you" and clicking the row opens the tab.
#[test]
fn claude_hook_states_drive_sessions() {
    use noble::app::{AgentState, Hit};
    use noble::hooks::HookRecord;
    let mut app = demo_app(160, 45);
    app.new_tab(std::env::temp_dir(), None, Some("agent".into()));
    let pane = app.tabs[0].focus;
    app.run(Action::Bridge);
    app.toasts.clear();
    let rec = |event: &str, msg: Option<&str>| HookRecord {
        event: event.into(),
        message: msg.map(str::to_string),
        ts: chrono::Utc::now().timestamp(),
        agent: None,
        session: None,
        subagents: 0,
    };
    app.apply_hook_records([(pane, rec("prompt", None))].into_iter().collect());
    assert_eq!(app.agent_state(pane), Some(("claude", AgentState::Working)));
    assert!(app.toasts.is_empty(), "working is not an alert");
    app.apply_hook_records([(pane, rec("notification", Some("Claude needs your permission to use Bash")))].into());
    assert_eq!(app.agent_state(pane), Some(("claude", AgentState::NeedsYou)));
    assert!(app.tabs[0].alert.is_some());
    assert!(app.toasts.iter().any(|t| t.text.contains("needs your permission")), "toast missing");
    let text = render(&mut app, 160, 45);
    save("bridge-sessions-160x45", &text);
    assert!(text.contains("◆ claude") && text.contains("needs you"), "{text}");
    let row = find_hit(&app, |h| *h == Hit::Tab(0)).expect("session row");
    // The session row (right column) must differ from the tab in the top strip.
    let session_row =
        app.hits.iter().filter(|(r, h)| *h == Hit::Tab(0) && r.y > 1).map(|(r, _)| *r).next().unwrap_or(row);
    click(&mut app, session_row.x + 2, session_row.y);
    assert_eq!(app.view, View::Term(0));
    // Reading the same event again does not re-notify; when done it says "your turn".
    app.toasts.clear();
    let same = app.agent_hooks.clone();
    app.apply_hook_records(same);
    assert!(app.toasts.is_empty());
    app.apply_hook_records([(pane, rec("stop", None))].into());
    assert_eq!(app.agent_state(pane), Some(("claude", AgentState::Idle)));
    app.run(Action::CloseTab);
    assert!(app.agent_hooks.is_empty());
}

/// Claude's `Stop` fires when the main answer ends even while background subagents still
/// run: the session stays "working" without a notice until the last one is done, then it
/// says Claude finished. A permission request still needs you.
#[test]
fn claude_subagents_keep_the_session_working() {
    use noble::app::AgentState;
    use noble::hooks::HookRecord;
    let mut app = demo_app(160, 45);
    app.new_tab(std::env::temp_dir(), None, Some("agent".into()));
    let pane = app.tabs[0].focus;
    app.run(Action::Bridge);
    app.toasts.clear();
    let now = chrono::Utc::now().timestamp();
    let rec = |event: &str, ts: i64, subagents: usize| HookRecord {
        event: event.into(),
        message: (event == "notification").then(|| "Claude needs your permission to use Bash".into()),
        ts,
        agent: None,
        session: None,
        subagents,
    };
    // Each step returns the state and the notices that step raised.
    let mut apply = |event: &str, ts: i64, subagents: usize| {
        app.toasts.clear();
        app.apply_hook_records([(pane, rec(event, ts, subagents))].into());
        let notices: Vec<String> = app.toasts.iter().map(|t| t.text.clone()).collect();
        (app.agent_state(pane).map(|(_, s)| s), notices)
    };
    let finished = |n: &[String]| n.len() == 1 && n[0].contains("Claude finished");
    let (state, n) = apply("prompt", now, 0);
    assert_eq!((state, n.len()), (Some(AgentState::Working), 0));
    let (state, n) = apply("prompt", now, 2);
    assert_eq!((state, n.len()), (Some(AgentState::Working), 0), "two subagents started");
    let (state, n) = apply("stop", now + 1, 2);
    assert_eq!((state, n.len()), (Some(AgentState::Working), 0), "main answer ended, subagents run");
    let (state, n) = apply("stop", now + 1, 1);
    assert_eq!((state, n.len()), (Some(AgentState::Working), 0), "working to working never notifies");
    // The record's event stays the same when the last subagent ends; the state change notifies.
    let (state, n) = apply("stop", now + 1, 0);
    assert_eq!(state, Some(AgentState::Idle));
    assert!(finished(&n), "the last subagent finished: {n:?}");
    let (state, n) = apply("stop", now + 1, 0);
    assert_eq!((state, n.len()), (Some(AgentState::Idle), 0), "reading it again does not re-notify");
    let (state, n) = apply("stop", now + 2, 0);
    assert_eq!(state, Some(AgentState::Idle));
    assert!(finished(&n), "Claude answered the results: {n:?}");
    let (state, n) = apply("notification", now + 3, 1);
    assert_eq!(state, Some(AgentState::NeedsYou), "a subagent asks for permission");
    assert!(n.iter().any(|t| t.contains("needs your permission")), "{n:?}");
    assert!(app.tabs[0].alert.is_some());
}

/// Agent lifecycle in one pane, on every installed shell: Claude opened from a quick
/// launch goes through its hook events and exits; the shell prompt (OSC 7 / 9;9) ends it;
/// another agent started in the same pane is shown instead; back at the shell nothing
/// is shown; closing the pane clears the records. Orphan records, late writes and other
/// NOBLE instances are covered along the way.
#[test]
fn agent_state_follows_the_program_in_the_pane() {
    check_agent_lifecycle("");
    let mut checked = vec!["default"];
    if cfg!(windows) {
        check_agent_lifecycle("cmd.exe");
        checked.push("cmd.exe");
        // Windows PowerShell 5.1 when pwsh took the default above, and Git Bash.
        if noble::util::which("pwsh").is_some()
            && let Some(ps) = noble::util::which("powershell")
        {
            check_agent_lifecycle(&ps.display().to_string());
            checked.push("powershell");
        }
        let git_bash = std::path::Path::new(r"C:\Program Files\Git\bin\bash.exe");
        if git_bash.is_file() {
            check_agent_lifecycle(&git_bash.display().to_string());
            checked.push("git bash");
        }
    } else {
        for shell in ["bash", "zsh", "fish", "pwsh"] {
            if let Some(path) = noble::util::which(shell) {
                check_agent_lifecycle(&path.display().to_string());
                checked.push(shell);
            }
        }
    }
    assert_checked(&checked, "agent_state_follows_the_program_in_the_pane");
}

fn check_agent_lifecycle(shell: &str) {
    use noble::app::AgentState;
    let tag = std::path::Path::new(shell)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "default".into());
    // Own data folder: headless apps share one, and panes of parallel tests have the same ids. Declared before
    // the app, so the app (and its panes) ends before the folder is removed.
    let data = TempDir::new(&format!("noble-agent-life-{tag}"));
    let mut app = demo_app(160, 45);
    if !shell.is_empty() {
        let mut cfg = app.cfg.clone();
        cfg.terminal.shell = shell.into();
        app.apply_config(cfg);
    }
    app.paths.data = data.path().to_path_buf();
    let me = std::process::id();
    let agents = noble::hooks::agents_dir(&app.paths.data);

    // Quick launch, as `App::launch` does it: the tab is named after the launcher.
    let claude = fake_agent(&app.shell, "Claude Code");
    let project = std::env::temp_dir();
    let origin = format!("{} · claude", project.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap());
    app.new_tab(project, Some(&claude), Some(origin.clone()));
    let pane = app.tabs[0].focus;
    pump_until(&mut app, &format!("{shell}: agent title"), |a| a.panes[&pane].label().contains("Claude Code"));
    assert_eq!(app.agent_state(pane), Some(("claude", AgentState::Running)), "{shell}: no hook yet");
    assert_eq!(app.tab_title(0), origin);

    // Claude's hooks.
    let steps = [
        ("session-start", "{}", AgentState::Idle),
        ("prompt", r#"{"prompt":"fix it"}"#, AgentState::Working),
        ("notification", r#"{"message":"Claude needs your permission to use Bash"}"#, AgentState::NeedsYou),
        ("stop", "{}", AgentState::Idle),
    ];
    for (event, stdin, want) in steps {
        noble_hook(&app, pane, me, event, stdin);
        scan_hooks(&mut app);
        assert_eq!(app.agent_state(pane), Some(("claude", want)), "{shell}: after {event}");
    }
    // Another NOBLE instance's pane with the same id is not ours.
    noble_hook(&app, pane, me + 1, "notification", "{}");
    // An orphan: a pane this instance never had (or already closed).
    noble_hook(&app, 9999, me, "prompt", "{}");
    scan_hooks(&mut app);
    assert!(!agents.join(format!("{me}-9999.json")).exists(), "{shell}: orphan record kept");
    assert!(agents.join(format!("{}-{pane}.json", me + 1)).exists(), "{shell}: other instance's record removed");

    // `/exit`: SessionEnd clears the record; the program itself is still closing.
    noble_hook(&app, pane, me, "session-end", r#"{"reason":"prompt_input_exit"}"#);
    scan_hooks(&mut app);
    assert!(!app.agent_hooks.contains_key(&pane), "{shell}: session-end left the record");
    assert!(!agents.join(format!("{me}-{pane}.json")).exists());

    // A stop hook that races the exit lands after session-end.
    noble_hook(&app, pane, me, "stop", "{}");
    // The agent exits; the shell prompt comes back.
    app.panes[&pane].write(b"\r");
    pump_until(&mut app, &format!("{shell}: prompt after the agent"), |a| a.panes[&pane].prompted);
    scan_hooks(&mut app);
    assert_eq!(app.agent_state(pane), None, "{shell}: agent shown at the shell prompt");
    // The title is gone, or is the one the shell sets for its own prompt.
    let label = app.panes[&pane].label();
    assert!(!label.contains("Claude"), "{shell}: the agent's title outlived it: {label}");
    assert!(!app.tab_title(0).contains("claude"), "{shell}: tab still says {:?}", app.tab_title(0));
    assert!(!agents.join(format!("{me}-{pane}.json")).exists(), "{shell}: record left after the prompt");
    let text = render(&mut app, 160, 45);
    save(&format!("agent-back-at-shell-{tag}"), &text);
    let top = text.lines().next().unwrap_or_default();
    assert!(!top.contains("claude"), "{shell}: top bar still says claude: {top}");

    // Another harness in the same pane. pwsh prints its prompt before PSReadLine is ready
    // (slowest on macOS) and drops keys typed in between: let it settle first.
    wait_idle(&mut app, pane);
    let codex = fake_agent(&app.shell, "codex");
    app.panes[&pane].write(format!("{codex}\r").as_bytes());
    // Wait for this agent: cmd.exe's console title can lag behind for a moment (ConPTY reports it on its own
    // schedule), so the first state seen may still come from the previous program.
    pump_until(&mut app, &format!("{shell}: second agent"), |a| a.agent_state(pane).is_some_and(|(k, _)| k == "codex"));
    assert_eq!(app.agent_state(pane), Some(("codex", AgentState::Running)), "{shell}");
    // An elevated cmd.exe prefixes its title with "Administrator: ", which the tab's 16-character
    // label cuts before the program name; the pane label still carries it. Other shells show it.
    let tab = app.tab_title(0);
    let label = app.panes[&pane].label();
    assert!(tab.contains("codex") || (shell == "cmd.exe" && label.contains("codex")), "{shell}: {tab:?} / {label:?}");
    assert!(!tab.contains("claude"), "{shell}: {tab:?}");
    // Enter is repeated until the agent takes one: see `press_enter_until`.
    press_enter_until(&mut app, pane, &format!("{shell}: back at the shell"), |a| a.agent_state(pane).is_none());
    assert!(!app.panes[&pane].label().contains("codex"), "{shell}");
    // The title can reset before the shell's next prompt mark arrives; until then the pane still counts as
    // running a command, and closing its tab below would ask first instead of closing.
    pump_until(&mut app, &format!("{shell}: prompt after the second agent"), |a| a.panes[&pane].busy().is_none());

    // Claude again, typed at the prompt this time; closing the pane clears its record.
    // Records written in the same second as the last prompt count as leftovers of the
    // program before it (hook timestamps are in seconds), so wait for the next second.
    std::thread::sleep(Duration::from_millis(1100));
    noble_hook(&app, pane, me, "session-start", "{}");
    scan_hooks(&mut app);
    assert_eq!(app.agent_state(pane), Some(("claude", AgentState::Idle)), "{shell}");
    app.run(Action::CloseTab);
    assert!(app.overlay.is_none(), "{shell}: an idle pane's tab closes without asking");
    assert!(app.agent_hooks.is_empty(), "{shell}: closing the tab left its agent");
    assert!(!agents.join(format!("{me}-{pane}.json")).exists(), "{shell}: record left after close");
}

/// `noble hook <event>`: the compiled binary reads the JSON on stdin and writes
/// the state file; outside NOBLE (no env var) it writes nothing and always exits 0.
#[test]
fn hook_cli_writes_state_file() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let home = std::env::temp_dir().join(format!("noble-hookcli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let run = |with_env: bool| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_noble"));
        cmd.args(["hook", "notification"]).env("NOBLE_HOME", &home).stdin(Stdio::piped());
        cmd.env_remove("NOBLE_INSTANCE").env_remove("NOBLE_PANE");
        if with_env {
            cmd.env("NOBLE_INSTANCE", "4242").env("NOBLE_PANE", "7");
        }
        let mut child = cmd.spawn().unwrap();
        child.stdin.take().unwrap().write_all(br#"{"message":"Claude needs your permission to use Bash"}"#).unwrap();
        child.wait().unwrap()
    };
    assert!(run(false).success());
    assert!(noble::hooks::read_records(&home, 4242).is_empty());
    assert!(run(true).success());
    let recs = noble::hooks::read_records(&home, 4242);
    assert_eq!(recs[&7].message.as_deref(), Some("Claude needs your permission to use Bash"));
    let _ = std::fs::remove_dir_all(&home);
}

/// A hook state change asks for a redraw itself (on battery the screens only redraw by the minute):
/// it shows on Home, in the pane title and as the tab strip's dot on every screen. A repeated state,
/// or a subagent count change that only a hidden pane's title would show, does not.
#[test]
fn agent_state_change_marks_the_frame_dirty() {
    use noble::hooks::HookRecord;
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("agent".into()));
    let pane = app.tabs[0].focus;
    let rec = |event: &str, ts: i64| HookRecord {
        event: event.into(),
        message: None,
        ts,
        agent: None,
        session: None,
        subagents: 0,
    };
    let now = chrono::Utc::now().timestamp();

    // In the terminal the state shows in the pane title and the tab's dot.
    app.take_dirty();
    app.apply_hook_records([(pane, rec("prompt", now))].into());
    assert!(app.take_dirty(), "the visible pane's state change did not ask for a frame");
    // On Home a subagent starting keeps the state (Working) and shows nowhere.
    app.run(Action::Bridge);
    app.take_dirty();
    app.apply_hook_records([(pane, HookRecord { subagents: 1, ..rec("prompt", now) })].into());
    assert!(!app.take_dirty(), "an invisible subagent count change woke the screen");

    app.run(Action::Bridge);
    app.take_dirty();
    app.apply_hook_records([(pane, rec("prompt", now + 1))].into());
    assert!(!app.take_dirty(), "the same state again redrew");
    // Working -> Idle without a toast.
    app.apply_hook_records([(pane, rec("session-start", now + 2))].into());
    assert!(app.toasts.is_empty());
    assert!(app.take_dirty(), "the state change on Home did not ask for a frame");
    // The record going away (session ended) changes Home too.
    app.apply_hook_records(Default::default());
    assert!(app.take_dirty());
}

/// The pane title leads with the agent's state: a spinner (a static "…" on battery) with
/// "claude · working 2m +2", "◆ … needs you" in the warning color, "● … your turn"; narrow panes
/// keep the glyph and kind, then the glyph alone. Only a visible working agent asks for fast frames.
#[test]
fn pane_title_shows_the_agent_state() {
    use noble::app::AgentState;
    let mut app = demo_app(110, 30);
    set_power(&mut app, true);
    app.new_tab(std::env::temp_dir(), None, Some("agent".into()));
    let pane = app.tabs[0].focus;
    app.toasts.clear();
    let title_row = |text: &str| text.lines().find(|l| l.contains(" ✕ ")).unwrap_or_default().to_string();
    // No agent: no extra frames (the idle cost stays one frame a minute).
    render(&mut app, 110, 30);
    assert!(idle_redraw(&app), "{:?}", app.redraw_after());

    let now = chrono::Utc::now().timestamp();
    app.take_dirty();
    app.apply_hook_records([(pane, hook_rec("prompt", now - 125, 2))].into());
    assert!(app.take_dirty(), "a changed state in the visible tab asks for a frame");
    assert_eq!(app.agent_state(pane), Some(("claude", AgentState::Working)));
    // Working but its pane is silent (an interrupt sends no hook event): a static glyph and no
    // fast frames, however long the record stays.
    let row = title_row(&render(&mut app, 110, 30));
    assert!(row.contains(" … claude · working 2m +2 · "), "{row}");
    assert!(idle_redraw(&app), "a stale working state costs nothing: {:?}", app.redraw_after());
    app.panes.get_mut(&pane).unwrap().last_output = Some(Instant::now() - noble::app::AGENT_LIVE);
    render(&mut app, 110, 30);
    assert!(idle_redraw(&app), "{:?}", app.redraw_after());
    // Its pane shows output: the spinner turns.
    app.panes.get_mut(&pane).unwrap().last_output = Some(Instant::now());
    let text = render(&mut app, 110, 30);
    save("pane-agent-working-110x30", &text);
    let row = title_row(&text);
    assert!(row.contains(" claude · working 2m +2 · "), "{row}");
    let at = row.find(" claude · working").unwrap();
    let glyph = row[..at].chars().last().unwrap();
    assert!(SPIN.contains(glyph), "spinner frame expected: {row}");
    assert!(app.redraw_after().unwrap() <= Duration::from_millis(150), "the spinner turns");
    // The main answer ends while a subagent still runs: still working, still since the prompt.
    app.apply_hook_records([(pane, hook_rec("stop", now - 5, 1))].into());
    assert!(app.take_dirty(), "the subagent count shows in the visible title");
    assert!(title_row(&render(&mut app, 110, 30)).contains(" claude · working 2m +1 · "));

    // On battery the glyph is static and nothing spins; the minute count still moves on.
    set_power(&mut app, false);
    let row = title_row(&render(&mut app, 110, 30));
    assert!(row.contains(" … claude · working 2m +1 "), "{row}");
    assert!(idle_redraw(&app), "{:?}", app.redraw_after());
    assert!(app.redraw_after().unwrap() <= Duration::from_millis(60_005));
    // On another screen a working agent costs nothing.
    set_power(&mut app, true);
    app.view = View::System;
    assert!(idle_redraw(&app), "{:?}", app.redraw_after());
    app.view = View::Term(0);

    // Narrow panes: the folder goes first, then the state, and the glyph and kind stay; the
    // buttons stay too (`fit_title`).
    app.panes.get_mut(&pane).unwrap().last_output = Some(Instant::now());
    for (w, h, want, gone) in [(60, 20, "working 2m", ""), (40, 12, "working 2m", ""), (30, 8, " claude", "working")] {
        let text = render(&mut app, w, h);
        save(&format!("pane-agent-working-{w}x{h}"), &text);
        let row = text.lines().find(|l| l.contains('✕')).unwrap_or_default().to_string();
        assert!(row.chars().any(|c| SPIN.contains(c)), "{w}x{h}: {row}");
        assert!(row.contains(want), "{w}x{h}: {row}");
        assert!(gone.is_empty() || !row.contains(gone), "{w}x{h}: {row}");
    }

    // Needs you: the whole part in the warning color, bold.
    app.apply_hook_records([(pane, hook_rec("notification", now, 0))].into());
    assert!(app.take_dirty());
    let (text, buf) = render_buf(&mut app, 110, 30);
    save("pane-agent-needs-you-110x30", &text);
    let row = title_row(&text);
    assert!(row.contains("◆ claude · needs you · "), "{row}");
    let y = text.lines().position(|l| l.contains(" ✕ ")).unwrap() as u16;
    let x = (0..110).find(|x| buf[(*x, y)].symbol() == "◆").unwrap();
    assert_eq!(buf[(x, y)].fg, app.theme.warn);
    assert!(buf[(x + 2, y)].modifier.contains(ratatui::style::Modifier::BOLD));
    assert!(idle_redraw(&app), "nothing spins while it waits: {:?}", app.redraw_after());

    // Your turn.
    app.apply_hook_records([(pane, hook_rec("stop", now + 1, 0))].into());
    let (text, buf) = render_buf(&mut app, 110, 30);
    save("pane-agent-your-turn-110x30", &text);
    let row = title_row(&text);
    assert!(row.contains("● claude · your turn · "), "{row}");
    let y = text.lines().position(|l| l.contains(" ✕ ")).unwrap() as u16;
    let x = (0..110).find(|x| buf[(*x, y)].symbol() == "●").unwrap();
    assert_eq!(buf[(x, y)].fg, app.theme.ok);
    assert!(idle_redraw(&app), "{:?}", app.redraw_after());
    app.run(Action::CloseTab);
}

/// "Jump to waiting agent" (prefix a): the next pane that needs you, across tabs, restoring a
/// zoomed tab that shows another pane; else the next finished one; else a toast.
#[test]
fn jump_to_waiting_agent() {
    use noble::keys::Chord;
    let mut app = demo_app(120, 30);
    let a_key = Chord::parse("a").unwrap();
    assert_eq!(app.keymap.prefix_map.get(&a_key), Some(&Action::JumpToAgent));
    assert!(app.palette_items().iter().any(|i| i.title == "Jump to Waiting Agent"), "on every screen");
    app.new_tab(std::env::temp_dir(), None, Some("first".into()));
    let a = app.tabs[0].focus;
    app.new_tab(std::env::temp_dir(), None, Some("second".into()));
    let b = app.tabs[1].focus;
    app.run(Action::SplitRight);
    let other = app.tabs[1].focus;
    assert_ne!(other, b);
    app.run(Action::Zoom);
    assert!(app.tabs[1].zoomed);
    let now = chrono::Utc::now().timestamp();
    let recs = |x: &str, y: &str| -> std::collections::HashMap<_, _> {
        [(a, hook_rec(x, now, 0)), (b, hook_rec(y, now, 0))].into()
    };
    // Nothing waiting.
    app.apply_hook_records(recs("prompt", "prompt"));
    app.toasts.clear();
    app.view = View::Bridge;
    app.run(Action::JumpToAgent);
    assert_eq!(app.view, View::Bridge);
    assert!(app.toasts.iter().any(|t| t.text == "no agent is waiting"), "toast missing");
    // Tab 1 finished, tab 2's hidden pane needs you: that one wins, and its tab is unzoomed.
    app.apply_hook_records(recs("stop", "notification"));
    app.run(Action::JumpToAgent);
    assert_eq!(app.view, View::Term(1));
    assert_eq!(app.tabs[1].focus, b);
    assert!(!app.tabs[1].zoomed, "the pane must be visible");
    // It is the only one needing you, so it stays. Through the prefix key as well.
    let prefix = app.keymap.prefix;
    app.on_key(KeyEvent::new(prefix.code, prefix.mods));
    app.on_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    assert_eq!((app.view, app.tabs[1].focus), (View::Term(1), b));
    // Once answered, the finished agent in tab 1 is next.
    app.apply_hook_records(recs("stop", "prompt"));
    app.run(Action::JumpToAgent);
    assert_eq!((app.view, app.tabs[0].focus), (View::Term(0), a));
    for _ in 0..2 {
        app.view = View::Term(0);
        app.run(Action::CloseTab);
    }
}

/// The one-time "Show agent status in NOBLE?" notice: bottom right, click enables (same path as Settings;
/// headless only toasts), the x dismisses. An available update takes the slot first.
#[test]
fn hooks_offer_notice() {
    use noble::app::Hit;
    let mut app = demo_app(120, 32);
    assert!(!render(&mut app, 120, 32).contains("Show agent status"));
    app.hooks_offer = true;
    app.agents_found = vec![noble::hooks::Agent::Claude];
    let text = render(&mut app, 120, 32);
    save("hooks-offer-120x32", &text);
    assert!(text.lines().nth(30).unwrap().contains("Show agent status in NOBLE?"), "{text}");
    for (w, h) in [(80, 24), (40, 12), (30, 8)] {
        save(&format!("hooks-offer-{w}x{h}"), &render(&mut app, w, h));
    }
    assert_no_doubled_frame(&render(&mut app, 30, 8));
    render(&mut app, 120, 32);
    let enable = find_hit(&app, |h| *h == Hit::HooksOffer).expect("enable hit");
    click(&mut app, enable.x + 1, enable.y);
    assert!(!app.hooks_offer, "clicking answers the offer");
    assert!(app.hooks_on.is_empty(), "headless never touches ~/.claude");
    assert!(!render(&mut app, 120, 32).contains("Show agent status in NOBLE?"));

    app.hooks_offer = true;
    app.agents_found = vec![noble::hooks::Agent::Claude];
    app.handle(AppEvent::Update(Ok("99.0.0".into())));
    let text = render(&mut app, 120, 32);
    assert!(text.contains("99.0.0") && !text.contains("Show agent status"), "{text}");
    app.dismiss_update();
    render(&mut app, 120, 32);
    let close = find_hit(&app, |h| *h == Hit::HooksOfferDismiss).expect("dismiss hit");
    click(&mut app, close.x, close.y);
    assert!(!app.hooks_offer);
}

/// A permission request needs you at once and is announced once: the notification that Claude sends for the same
/// prompt is not a second question. A failed turn is an error, shown in the pane title and announced from a
/// background tab; the next prompt clears it.
#[test]
fn permission_request_and_failed_turn() {
    use noble::app::AgentState;
    use noble::hooks::HookRecord;
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("agent".into()));
    let pane = app.tabs[0].focus;
    // The agent's tab is in the background while its hook events arrive.
    app.new_tab(std::env::temp_dir(), None, Some("other".into()));
    app.toasts.clear();
    let rec = |event: &str, message: Option<&str>| HookRecord {
        event: event.into(),
        message: message.map(str::to_string),
        ts: chrono::Utc::now().timestamp(),
        agent: None,
        session: None,
        subagents: 0,
    };
    let texts = |app: &noble::app::App| app.toasts.iter().map(|t| t.text.clone()).collect::<Vec<_>>();

    app.apply_hook_records([(pane, rec("permission-request", Some("Claude asks to run Bash")))].into());
    assert_eq!(app.agent_state(pane), Some(("claude", AgentState::NeedsYou)), "at once");
    let asked = texts(&app);
    assert!(asked.len() == 1 && asked[0].contains("asks to run Bash"), "{asked:?}");

    app.toasts.clear();
    app.apply_hook_records([(pane, rec("notification", Some("Claude asks to run Bash")))].into());
    assert!(texts(&app).is_empty(), "the notification of the same prompt is not announced again: {:?}", texts(&app));
    assert_eq!(app.agent_state(pane), Some(("claude", AgentState::NeedsYou)));

    app.apply_hook_records([(pane, rec("stop-failure", Some("Claude stopped with an error: rate_limit")))].into());
    assert_eq!(app.agent_state(pane), Some(("claude", AgentState::Error)));
    let failed = texts(&app);
    assert!(failed.iter().any(|t| t.contains("stopped with an error")), "{failed:?}");
    assert!(app.tabs[0].alert.is_some(), "the tab is marked");

    app.view = View::Term(0);
    let text = render(&mut app, 110, 30);
    save("pane-agent-error-110x30", &text);
    let row = text.lines().find(|l| l.contains('✕')).unwrap_or_default().to_string();
    assert!(row.contains("✗ claude · error"), "{row}");

    app.apply_hook_records([(pane, rec("prompt", None))].into());
    assert_eq!(app.agent_state(pane), Some(("claude", AgentState::Working)), "a new prompt clears the error");
}

/// A record written by another agent's hook shows under that agent's name in the pane title.
#[test]
fn another_agents_record_shows_under_its_name() {
    use noble::app::AgentState;
    use noble::hooks::{Agent, HookRecord};
    let mut app = demo_app(110, 30);
    app.new_tab(std::env::temp_dir(), None, Some("agent".into()));
    let pane = app.tabs[0].focus;
    app.view = View::Term(0);
    let record = HookRecord {
        event: "notification".into(),
        message: Some("Codex needs your approval".into()),
        ts: chrono::Utc::now().timestamp(),
        agent: Some(Agent::Codex),
        session: None,
        subagents: 0,
    };
    app.apply_hook_records([(pane, record)].into());
    assert_eq!(app.agent_state(pane), Some(("codex", AgentState::NeedsYou)));
    let text = render(&mut app, 110, 30);
    let row = text.lines().find(|l| l.contains('✕')).unwrap_or_default().to_string();
    assert!(row.contains("◆ codex · needs you"), "{row}");
}

/// `noble hook <event> --agent <id>`: the record names that agent; without the flag it is Claude's; an unknown
/// agent records nothing. The real binary, inside a pane's environment.
#[test]
fn hook_cli_names_the_agent_it_runs_for() {
    use noble::hooks::Agent;
    use std::io::Write as _;
    let home = std::env::temp_dir().join(format!("noble-cli-agent-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let cases: [(&[&str], Option<Agent>); 3] = [
        (&["hook", "prompt", "--agent", "codex"], Some(Agent::Codex)),
        (&["hook", "prompt", "--agent", "cursor"], None),
        (&["hook", "stop"], Some(Agent::Claude)),
    ];
    for (args, want) in cases {
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_noble"))
            .args(args)
            .env("NOBLE_HOME", &home)
            .env("NOBLE_INSTANCE", "42")
            .env("NOBLE_PANE", "3")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("noble hook");
        // An unknown agent exits before reading stdin, so the write may fail: that is fine.
        let _ = child.stdin.take().unwrap().write_all(b"{}");
        assert!(child.wait().unwrap().success(), "{args:?}");
        let got = noble::hooks::read_records(&home, 42).get(&3).map(|r| r.agent());
        assert_eq!(got, want, "{args:?}");
        noble::hooks::remove_record(&home, 42, 3);
    }
    let _ = std::fs::remove_dir_all(&home);
}

/// The "Agent status hooks" popup from its Settings row: one line per agent with its state word (on, off, or not
/// found here), drawn at every size down to 30x8. A click on a row selects it and toggles it; headless apps only
/// toast and never write an agent's settings file.
#[test]
fn agent_hooks_popup_lists_every_agent() {
    use noble::app::{Hit, Overlay, SettingItem, SettingKey};
    use noble::hooks::Agent;
    let mut app = demo_app(160, 45);
    app.agents_found = vec![Agent::Claude, Agent::Codex];
    app.hooks_on = vec![Agent::Claude];
    app.open_settings();
    let page = render(&mut app, 160, 45);
    save("agent-hooks-settings-160x45", &page);
    assert!(page.contains("Agent status hooks") && page.contains("1 of 2 on"), "{page}");
    app.close_settings();
    app.activate_setting(SettingItem::Setting(SettingKey::AgentHooks), 1);
    assert!(matches!(app.overlay, Some(Overlay::AgentHooks { selected: 0 })));
    let text = render(&mut app, 160, 45);
    save("agent-hooks-160x45", &text);
    for name in ["Claude Code", "Codex", "GitHub Copilot", "OpenCode"] {
        assert!(text.contains(name), "{text}");
    }
    assert!(text.contains("not found") && text.contains(" on") && text.contains(" off"), "{text}");
    for (w, h) in [(80, 24), (40, 12), (30, 8)] {
        let small = render(&mut app, w, h);
        save(&format!("agent-hooks-{w}x{h}"), &small);
        assert_no_doubled_frame(&small);
    }
    render(&mut app, 160, 45);
    let codex = find_hit(&app, |h| *h == Hit::AgentHook(1)).expect("codex row");
    click(&mut app, codex.x + 1, codex.y);
    assert!(!app.hooks_on(Agent::Codex), "headless never touches the agent files");
    assert!(matches!(app.overlay, Some(Overlay::AgentHooks { selected: 1 })), "the clicked row is selected");
}

/// A progress report (OSC 9;4) shows in the pane title and on the tab. A percentage reads in the title, busy work
/// with no percentage spins there (a static "…" on battery) and marks its tab, and a paused or failed report shows
/// its mark. An agent without hooks reads as working while its report is busy. A changed report asks for a frame,
/// also from a background tab; only a spinner that turns asks for fast frames; the clear (state 0) removes the report.
#[test]
fn progress_report_shows_in_the_title_and_on_the_tab() {
    use noble::app::App;
    use noble::term::layout::PaneId;
    let mut app = demo_app(110, 30);
    set_power(&mut app, true);
    app.new_tab(std::env::temp_dir(), None, Some("build".into()));
    app.new_tab(std::env::temp_dir(), None, Some("other".into()));
    app.view = View::Term(0);
    let pane = app.tabs[0].focus;
    // The output as the reader thread hands it over: the answer says whether the app asks for a frame.
    let feed = |app: &mut App, pane: PaneId, bytes: &[u8]| {
        app.panes[&pane].parser().process(bytes);
        app.panes[&pane].dirty.store(true, std::sync::atomic::Ordering::Release);
        app.handle(AppEvent::PtyOutput)
    };
    let title_row = |text: &str| text.lines().find(|l| l.contains(" ✕ ")).unwrap_or_default().to_string();
    let top_row = |text: &str| text.lines().next().unwrap_or_default().to_string();
    // The tab is named after the folder its pane started in: the temp dir's last part (`Temp` on Windows, `T` on
    // macOS, `tmp` on Linux).
    let folder = std::env::temp_dir().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // The shell's prompt marks clear a report, so the reports are fed only once the first prompt is shown and the
    // shell's startup output has settled.
    pump_until(&mut app, "the shell's first prompt", |a| a.panes[&pane].prompted);
    wait_idle(&mut app, pane);

    assert!(feed(&mut app, pane, b"\x1b]9;4;1;42\x07"), "a visible report asks for a frame");
    let text = render(&mut app, 110, 30);
    save("pane-progress-110x30", &text);
    assert!(title_row(&text).contains("42%"), "{}", title_row(&text));
    // Narrow panes keep their buttons and their width; the report goes before the folder is cut (`fit_title`).
    for (w, h) in [(60, 20), (40, 12), (30, 8)] {
        let text = render(&mut app, w, h);
        save(&format!("pane-progress-{w}x{h}"), &text);
        let row = text.lines().find(|l| l.contains(" ✕ ├")).unwrap_or_default().to_string();
        assert_eq!(unicode_width::UnicodeWidthStr::width(row.as_str()), w as usize, "{w}x{h}: {row}");
    }
    // Busy with no percentage: a spinner that turns (a static "…" on battery, and then no fast frames).
    feed(&mut app, pane, b"\x1b]9;4;3;0\x07");
    let row = title_row(&render(&mut app, 110, 30));
    assert!(row.chars().any(|c| SPIN.contains(c)), "{row}");
    assert!(!row.contains('%'), "{row}");
    assert!(app.redraw_after().unwrap() <= Duration::from_millis(150), "the spinner turns");
    set_power(&mut app, false);
    let row = title_row(&render(&mut app, 110, 30));
    assert!(row.contains('…') && !row.chars().any(|c| SPIN.contains(c)), "{row}");
    assert!(idle_redraw(&app), "{:?}", app.redraw_after());
    set_power(&mut app, true);
    // The paused and failed marks, each with its percentage.
    feed(&mut app, pane, b"\x1b]9;4;4;30\x07");
    assert!(title_row(&render(&mut app, 110, 30)).contains("‖ 30%"));
    feed(&mut app, pane, b"\x1b]9;4;2;60\x07");
    assert!(title_row(&render(&mut app, 110, 30)).contains("✗ 60%"));
    feed(&mut app, pane, b"\x1b]9;4;0\x07");
    let row = title_row(&render(&mut app, 110, 30));
    assert!(!row.contains('%') && !row.contains('✗') && !row.contains('‖'), "cleared: {row}");

    // A busy report in a background tab marks its tab; the change redraws the tab strip, and nothing spins.
    app.view = View::Term(1);
    assert!(feed(&mut app, pane, b"\x1b]9;4;1;7\x07"), "a background tab's report redraws the tab strip");
    let top = top_row(&render(&mut app, 110, 30));
    assert!(top.contains(&format!("… {folder}")), "the busy tab's marker: {top}");
    assert!(idle_redraw(&app), "{:?}", app.redraw_after());
    assert!(feed(&mut app, pane, b"\x1b]9;4;0\x07"));
    assert!(!top_row(&render(&mut app, 110, 30)).contains(&format!("… {folder}")), "cleared");

    // An agent without hooks, typed at the prompt: the shell marks the command started (`133;C`) and the agent names
    // itself in its title. Busy reads as working, its percentage in the title.
    app.view = View::Term(0);
    feed(&mut app, pane, b"\x1b]133;C\x07\x1b]0;codex\x07\x1b]9;4;3;0\x07");
    let row = title_row(&render(&mut app, 110, 30));
    assert!(row.contains("codex · working"), "{row}");
    assert!(row.chars().any(|c| SPIN.contains(c)), "a working agent spins: {row}");
    feed(&mut app, pane, b"\x1b]9;4;1;55\x07");
    let row = title_row(&render(&mut app, 110, 30));
    assert!(row.contains("codex · working") && row.contains("55%"), "{row}");
}

/// A busy report in one pane marks its tab over the tab's idle agent (its open dot `○`), and the agent itself stays
/// running: the report belongs to the pane that sent it.
#[test]
fn a_busy_pane_marks_its_tab_over_an_idle_agent() {
    use noble::app::{AgentState, App};
    let mut app = demo_app(110, 30);
    set_power(&mut app, false);
    app.new_tab(std::env::temp_dir(), None, Some("split".into()));
    let tab = app.tabs.len() - 1;
    let agent = app.tabs[tab].focus;
    app.panes.get_mut(&agent).unwrap().command = Some("codex".into());
    app.run(Action::SplitRight);
    let shell = app.tabs[tab].panes().into_iter().find(|p| *p != agent).expect("the split pane");
    let top = |app: &mut App| render(app, 110, 30).lines().next().unwrap_or_default().to_string();
    assert!(top(&mut app).contains('○'), "the idle agent's dot: {}", top(&mut app));
    app.panes[&shell].parser().process(b"\x1b]9;4;3;0\x07");
    let row = top(&mut app);
    assert!(!row.contains('○') && row.contains('…'), "the busy pane marks the tab: {row}");
    assert_eq!(app.agent_state(agent), Some(("codex", AgentState::Running)));
}

/// The progress report before a failed command's result is no part of its click target: a click on the percentage
/// does not jump to the output, the result (`✗ 1`) still does.
#[test]
fn a_progress_report_is_no_part_of_the_failed_command_button() {
    use noble::app::Hit;
    use noble::term::pane::CommandResult;
    let mut app = demo_app(110, 30);
    set_power(&mut app, true);
    app.new_tab(std::env::temp_dir(), None, Some("build".into()));
    let pane = app.tabs[0].focus;
    app.panes[&pane].parser().process(b"\x1b]9;4;1;42\x07");
    let took = Duration::from_secs(2);
    app.panes.get_mut(&pane).unwrap().last_result = Some(CommandResult { code: Some(1), took, at: Instant::now() });
    let text = render(&mut app, 110, 30);
    let row = text.lines().find(|l| l.contains("✗ 1")).unwrap_or_default().to_string();
    let col = |needle: &str| {
        let at = row.find(needle).unwrap_or_else(|| panic!("{needle} in {row}"));
        unicode_width::UnicodeWidthStr::width(&row[..at]) as u16
    };
    let button = find_hit(&app, |h| *h == Hit::CommandBadge(pane)).expect("the ✗ badge is clickable");
    assert_eq!(button.x, col("✗ 1"), "the button starts at the result: {row}");
    assert!(col("42%") < button.x, "the percentage is not part of it: {row}");
}
