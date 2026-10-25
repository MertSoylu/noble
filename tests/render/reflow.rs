//! A long line stays whole when its pane changes size: zoom in and back, the focus mode toggle (two rows more or
//! fewer), a window resize narrow then wide, in every installed shell. Every platform reflows the lines in the
//! emulator (`Pane::spawn`); on Windows ConPTY also repaints the viewport from its own buffer after a resize, so these
//! tests run against both at once.

use noble::app::App;
use noble::keys::Action;
use noble::term::layout::PaneId;
use std::time::{Duration, Instant};

use super::common::*;

/// A line much longer than a split pane is wide, with an end marker: a cut or a doubled line shows in the count.
fn long_token(name: &str) -> String {
    format!("{name}{}END", "0123456789".repeat(11))
}

/// How many times `token` appears in the text of the screen, a soft-wrapped line counted as one line.
fn occurrences(app: &App, id: PaneId, token: &str) -> usize {
    app.panes[&id].parser().screen().contents().matches(token).count()
}

/// Pumps the app until `ok` holds and keeps holding for a moment. A resize is followed by the shell's redraw and, on
/// Windows, by ConPTY repainting its viewport; both arrive some time later and in pieces, so the text is looked at
/// once they have settled. A loss that stays fails after the wait, with the screen in the message.
fn settle(app: &mut App, what: &str, ok: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        run_for(app, Duration::from_millis(50));
        if ok(app) {
            run_for(app, Duration::from_millis(300));
            if ok(app) {
                return;
            }
        }
        if Instant::now() > deadline {
            let (w, h) = app.size;
            panic!("{what}\n{}", render(app, w, h));
        }
    }
}

/// Runs `command` in a pane of a plain shell (a tab with a launcher command on `cmd`/`sh`).
fn launch(app: &mut App, command: &str) -> PaneId {
    let mut cfg = app.cfg.clone();
    cfg.terminal.shell = if cfg!(windows) { "cmd.exe" } else { "/bin/sh" }.into();
    app.apply_config(cfg);
    app.new_tab(std::env::temp_dir(), Some(command), Some("reflow".into()));
    app.tabs[0].focus
}

fn print_command(token: &str, done: &str) -> String {
    // cmd.exe syntax on Windows and POSIX sh elsewhere (the shell is chosen to match in `launch`).
    if cfg!(windows) { format!("echo {token} & echo {done}") } else { format!("echo {token}; echo {done}") }
}

/// Zoom in and restore: the pane gets wider, then narrow again, and the line is whole every time.
#[test]
fn zoom_and_restore_keep_a_long_line_whole() {
    let mut app = demo_app(110, 30);
    let token = long_token("ZOOM");
    let id = launch(&mut app, &print_command(&token, "ZOOM_DONE"));
    wait_for(&mut app, id, "ZOOM_DONE");
    app.run(Action::SplitRight);
    render(&mut app, 110, 30);
    // Back to the first pane, whose output is on its screen.
    app.run(Action::FocusLeft);
    let id = app.tabs[0].focus;
    assert_eq!(id, app.tabs[0].panes()[0]);
    let half = render(&mut app, 110, 30);
    save("reflow-split-110x30", &half);
    let baseline = occurrences(&app, id, &token);
    assert!(baseline >= 1, "the line is on the screen:\n{half}");
    let narrow_cols = app.panes[&id].size.1;

    app.run(Action::Zoom);
    let zoomed = render(&mut app, 110, 30);
    save("reflow-zoom-110x30", &zoomed);
    assert!(app.panes[&id].size.1 > narrow_cols, "zoom makes the pane wider");
    settle(&mut app, "zoomed: the line is not whole", |a| occurrences(a, id, &token) == baseline);

    app.run(Action::Zoom);
    let restored = render(&mut app, 110, 30);
    save("reflow-restore-110x30", &restored);
    assert_eq!(app.panes[&id].size.1, narrow_cols);
    settle(&mut app, "restored: the line is not whole", |a| occurrences(a, id, &token) == baseline);
    // Round trips do not wear the text out.
    for _ in 0..3 {
        app.run(Action::Zoom);
        render(&mut app, 110, 30);
        app.run(Action::Zoom);
        render(&mut app, 110, 30);
    }
    settle(&mut app, "after three more round trips: the line is not whole", |a| occurrences(a, id, &token) == baseline);
    app.run(Action::CloseTab);
}

/// Focus mode takes the bars away: the pane gets two rows more, and two fewer when it ends. The lines that were in
/// the pane stay in order, and the newest one stays on the screen.
#[test]
fn focus_mode_toggle_keeps_the_lines() {
    let mut app = demo_app(110, 30);
    let count = 60;
    let script = if cfg!(windows) {
        format!("(for /L %i in (1,1,{count}) do @echo focusrow %i) & echo FOCUS_DONE")
    } else {
        format!("for i in $(seq 1 {count}); do echo focusrow $i; done; echo FOCUS_DONE")
    };
    let id = launch(&mut app, &script);
    wait_for(&mut app, id, "FOCUS_DONE");
    render(&mut app, 110, 30);
    let rows = app.panes[&id].size.0;
    let check = |app: &mut App, what: &str| {
        settle(app, &format!("{what}: every row once and in order, the newest on the screen"), |a| {
            let (lines, _) = a.panes[&id].all_lines();
            let seen: Vec<usize> = lines
                .iter()
                .filter_map(|l| l.trim().strip_prefix("focusrow ").and_then(|n| n.trim().parse().ok()))
                .collect();
            let expected: Vec<usize> = (1..=count).collect();
            seen == expected && a.panes[&id].parser().screen().contents().contains("FOCUS_DONE")
        });
    };
    check(&mut app, "before");
    app.run(Action::FocusMode);
    save("reflow-focus-on-110x30", &render(&mut app, 110, 30));
    assert_eq!(app.panes[&id].size.0, rows + 2);
    check(&mut app, "focus mode on");
    app.run(Action::FocusMode);
    save("reflow-focus-off-110x30", &render(&mut app, 110, 30));
    assert_eq!(app.panes[&id].size.0, rows);
    check(&mut app, "focus mode off");
    for _ in 0..3 {
        app.run(Action::FocusMode);
        render(&mut app, 110, 30);
        app.run(Action::FocusMode);
        render(&mut app, 110, 30);
    }
    check(&mut app, "after three more toggles");
    app.run(Action::CloseTab);
}

/// A window resize is one resize per frame, and a repeated frame at the same size resizes nothing: the numbering of
/// the lines (`reflow_epoch`) moves at most once per draw and not at all while a divider is dragged (the shells keep
/// their size until the release).
#[test]
fn a_resize_reflows_once_per_frame_and_not_during_a_divider_drag() {
    use crossterm::event::{MouseButton, MouseEventKind};
    use noble::app::Hit;
    let mut app = demo_app(110, 30);
    let id = launch(&mut app, &print_command(&long_token("ONCE"), "ONCE_DONE"));
    wait_for(&mut app, id, "ONCE_DONE");
    render(&mut app, 110, 30);
    let epoch = |app: &App| app.panes[&id].parser().screen().reflow_epoch();

    let before = epoch(&app);
    render(&mut app, 70, 30);
    assert_eq!(epoch(&app), before + 1, "one draw, one reflow");
    let after = epoch(&app);
    render(&mut app, 70, 30);
    render(&mut app, 70, 30);
    assert_eq!(epoch(&app), after, "a frame at the same size changes nothing");

    // A divider drag: the pane keeps its size (and its lines) until the release.
    render(&mut app, 110, 30);
    app.run(Action::SplitRight);
    render(&mut app, 110, 30);
    app.run(Action::FocusLeft);
    let first = app.tabs[0].panes()[0];
    render(&mut app, 110, 30);
    let size = app.panes[&first].size;
    let epoch_first = |app: &App| app.panes[&first].parser().screen().reflow_epoch();
    let before = epoch_first(&app);
    let div = find_hit(&app, |h| matches!(h, Hit::Divider { .. })).expect("divider");
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), div.x, div.y + 3);
    for step in 1..=12 {
        mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), div.x - step, div.y + 3);
        render(&mut app, 110, 30);
        assert_eq!(app.panes[&first].size, size, "step {step}: the pane keeps its size while the divider moves");
        assert_eq!(epoch_first(&app), before, "step {step}: no reflow during the drag");
    }
    mouse(&mut app, MouseEventKind::Up(MouseButton::Left), div.x - 12, div.y + 3);
    render(&mut app, 110, 30);
    assert_ne!(app.panes[&first].size, size, "the release resizes the pane once");
    assert_eq!(epoch_first(&app), before + 1, "and reflows it once");
    app.run(Action::CloseTab);
}

/// A long line printed in a real shell survives a window made narrow and wide again: the printed line is whole at
/// every width, in every installed shell. The shell's own prompt redraw after the resize is not looked at; a line that
/// was already printed is.
#[test]
fn a_long_line_is_whole_after_a_narrow_and_a_wide_window() {
    let mut checked = vec!["default"];
    check_resize_roundtrip("");
    if cfg!(windows) {
        check_resize_roundtrip("cmd.exe");
        checked.push("cmd.exe");
        if noble::util::which("pwsh").is_some()
            && let Some(ps) = noble::util::which("powershell")
        {
            check_resize_roundtrip(&ps.display().to_string());
            checked.push("powershell");
        }
        let git_bash = std::path::Path::new(r"C:\Program Files\Git\bin\bash.exe");
        if git_bash.is_file() {
            check_resize_roundtrip(&git_bash.display().to_string());
            checked.push("git bash");
        }
    } else {
        for shell in ["bash", "zsh", "fish", "pwsh"] {
            if let Some(path) = noble::util::which(shell) {
                check_resize_roundtrip(&path.display().to_string());
                checked.push(shell);
            }
        }
    }
    assert_checked(&checked, "a_long_line_is_whole_after_a_narrow_and_a_wide_window");
}

fn check_resize_roundtrip(shell: &str) {
    let mut app = demo_app(110, 30);
    if !shell.is_empty() {
        let mut cfg = app.cfg.clone();
        cfg.terminal.shell = shell.into();
        app.apply_config(cfg);
    }
    let kind = app.shell.label().to_lowercase();
    app.new_tab(std::env::temp_dir(), None, Some("resize".into()));
    let id = app.tabs[0].focus;
    pump_until(&mut app, &format!("{kind}: first prompt"), |a| a.panes[&id].prompted);
    // pwsh prints its prompt before PSReadLine is ready (slowest on macOS): let it settle before typing.
    wait_idle(&mut app, id);
    let token = long_token("SHELLLINE");
    // The shell joins the token from two pieces, so only its output holds the whole token (the typed line is
    // redrawn by the shell after a resize, which is the shell's business).
    let (head, tail) = token.split_at(40);
    let command = if kind.contains("powershell") || kind.contains("pwsh") {
        format!("echo ('{head}'+'{tail}')")
    } else if kind.contains("cmd") {
        format!("echo {head}^{tail}")
    } else {
        format!("echo \"{head}\"\"{tail}\"")
    };
    app.panes[&id].write(format!("{command}\r").as_bytes());
    let deadline = Instant::now() + Duration::from_secs(30);
    while occurrences(&app, id, &token) < 1 {
        assert!(Instant::now() < deadline, "{kind}: the line never printed\n{}", render(&mut app, 110, 30));
        app.pump();
        std::thread::sleep(Duration::from_millis(50));
    }
    wait_idle(&mut app, id);
    let baseline = occurrences(&app, id, &token);
    for (w, h) in [(50, 30), (30, 24), (150, 30), (110, 30)] {
        render(&mut app, w, h);
        settle(&mut app, &format!("{kind}: the line is not whole at {w}x{h}"), |a| {
            occurrences(a, id, &token) == baseline
        });
        save(&format!("reflow-{kind}-{w}x{h}").replace(' ', "-"), &render(&mut app, w, h));
    }
    app.run(Action::CloseTab);
}
