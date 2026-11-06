//! Events arriving from the background threads into the main loop.

use std::path::PathBuf;

use crate::ai::ProviderState;
use crate::pr::PrInfo;
use crate::projects::{GitInfo, Project};
use crate::sensors::{SensorSample, StaticInfo};
use crate::term::input::is_altgr_char;
use crate::term::layout::PaneId;

pub enum AppEvent {
    Input(crossterm::event::Event),
    /// Output arrived in one or more panes (a per-pane `dirty` flag is kept).
    PtyOutput,
    PtyExit(PaneId),
    /// Whether the folder a pane's shell reported is usable: the answer to `Pane::cwd`'s background check.
    PaneCwd {
        pane: PaneId,
        reported: String,
        usable: bool,
    },
    SensorStatic(Box<StaticInfo>),
    Sensors(Box<SensorSample>),
    KillResult {
        pid: u32,
        name: String,
        outcome: crate::sensors::KillOutcome,
    },
    /// A program started with `util::spawn_reporting` exited with an error (an `xdg-open` with no
    /// handler for the file): the text to show.
    LaunchFailed(String),
    Projects(Vec<Project>),
    Git(PathBuf, GitInfo),
    Ai(Box<ProviderState>),
    /// Update check: the latest published release or an error.
    Update(Result<String, String>),
    /// The OS asks NOBLE to end (SIGHUP/SIGTERM, or the console closing / logoff / shutdown on
    /// Windows): the normal shutdown path runs, which saves the session.
    Quit,
    /// Listening TCP ports per project folder (`ports`): the answer to a scan request.
    Ports(Vec<(PathBuf, Vec<u16>)>),
    /// Pull request of a project's branch (`pr`): the folder and the pull request, when there is one.
    Pr(PathBuf, Option<PrInfo>),
    /// Claude Code's background sessions (`bg`), newest first; empty when there are none or `claude` could not tell.
    BgSessions(Vec<crate::bg::BgSession>),
    /// The add prompt's auto mode found out whether a folder holds repositories: `scan` folders are
    /// scanned for them, the others are added as projects (`settings::add_by_mode`).
    AddDecided {
        raw: String,
        scan: bool,
    },
}

pub type Tx = std::sync::mpsc::Sender<AppEvent>;

/// Windows: crossterm's console input has no bracketed paste, so the terminal's own paste (Ctrl+V in
/// Windows Terminal) arrives as a burst of key presses, each line break as an Enter. Key events that
/// were all waiting at once (`batch`: one read plus everything already queued) and hold more than one
/// line cannot be typing, so they become one `Event::Paste`: it then gets the multi-line confirmation
/// and the bracketed-paste wrapping a Unix paste gets. A burst of one line (with or without its Enter)
/// stays as keys: IMEs commit several characters at once, and a single command runs alike either way. Unix terminals send bracketed
/// paste and never need this.
pub fn coalesce_paste(batch: Vec<crossterm::event::Event>) -> Vec<crossterm::event::Event> {
    use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
    let text_key = |ev: &Event| -> Option<Option<char>> {
        let Event::Key(k) = ev else { return None };
        if k.kind == KeyEventKind::Release {
            return Some(None);
        }
        // Windows (the only platform whose bursts reach here, see `main.rs`): a character the layout types with
        // AltGr arrives as Ctrl+Alt (`is_altgr_char`), and it is text like any other key.
        let altgr_char = matches!(k.code, KeyCode::Char(c) if is_altgr_char(k.modifiers, c));
        if !altgr_char && !(k.modifiers - KeyModifiers::SHIFT).is_empty() {
            return None;
        }
        match k.code {
            KeyCode::Char(c) => Some(Some(c)),
            KeyCode::Enter => Some(Some('\n')),
            KeyCode::Tab => Some(Some('\t')),
            _ => None,
        }
    };
    let mut out = Vec::with_capacity(batch.len());
    let mut run: Vec<Event> = Vec::new();
    let flush = |run: &mut Vec<Event>, out: &mut Vec<Event>| {
        let text: String = run.iter().filter_map(|e| text_key(e).flatten()).collect();
        // Two lines or more: one line with its Enter is what a fast typist, an automation tool or a
        // terminal's "send text" action produces too, and it runs the same way as a typed command.
        if text.trim_end_matches('\n').contains('\n') {
            out.push(Event::Paste(text));
        } else {
            out.append(run);
        }
        run.clear();
    };
    for ev in batch {
        if text_key(&ev).is_some() {
            run.push(ev);
        } else {
            flush(&mut run, &mut out);
            out.push(ev);
        }
    }
    flush(&mut run, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    fn press(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn release(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Release))
    }

    /// A character the Windows console reports for AltGr: Ctrl+Alt with the character it types.
    fn altgr(c: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL | KeyModifiers::ALT))
    }

    #[test]
    fn altgr_symbols_are_text_in_a_paste() {
        // Windows only: the console reports AltGr as Ctrl+Alt, and `coalesce_paste` runs there alone (`main.rs`).
        // Elsewhere Ctrl+Alt symbols are not text, so there is nothing to check.
        if !cfg!(windows) {
            return;
        }
        // '\\' and '@' are typed with AltGr on common layouts (Turkish Q and German, for example): they stay text,
        // so a multi-line paste of them is still one paste.
        let burst =
            vec![altgr('\\'), altgr('@'), press(KeyCode::Enter), press(KeyCode::Char('x')), press(KeyCode::Enter)];
        assert_eq!(coalesce_paste(burst), vec![Event::Paste("\\@\nx\n".into())]);
    }

    #[test]
    fn ctrl_alt_key_without_a_character_ends_a_paste_run() {
        // Ctrl+Alt+Enter types no character, so it is a shortcut and ends the run, as Ctrl+C does.
        let alt_enter = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL | KeyModifiers::ALT));
        let keys = vec![
            press(KeyCode::Char('a')),
            press(KeyCode::Enter),
            press(KeyCode::Char('b')),
            alt_enter.clone(),
            press(KeyCode::Char('c')),
        ];
        assert_eq!(coalesce_paste(keys), vec![Event::Paste("a\nb".into()), alt_enter, press(KeyCode::Char('c'))]);
    }

    #[test]
    fn altgr_letters_follow_the_layout() {
        // Which letters a layout types with AltGr is up to the layout (Windows: Turkish Q has AltGr+I = "i"), so
        // the expectation is `is_altgr_char`'s own verdict for 'i'. The rest of the burst is plain keys.
        let i = altgr('i');
        let burst = vec![
            i.clone(),
            press(KeyCode::Char('x')),
            press(KeyCode::Enter),
            press(KeyCode::Char('y')),
            press(KeyCode::Enter),
        ];
        if is_altgr_char(KeyModifiers::CONTROL | KeyModifiers::ALT, 'i') {
            assert_eq!(coalesce_paste(burst), vec![Event::Paste("ix\ny\n".into())]);
        } else {
            // The letter stays a key, and the rest of the burst is a paste of its own.
            assert_eq!(coalesce_paste(burst), vec![i, Event::Paste("x\ny\n".into())]);
        }
    }

    #[test]
    fn a_burst_of_keys_with_a_line_break_is_a_paste() {
        let burst = vec![
            press(KeyCode::Char('l')),
            release(KeyCode::Char('l')),
            press(KeyCode::Char('s')),
            press(KeyCode::Enter),
            press(KeyCode::Char('x')),
        ];
        assert_eq!(coalesce_paste(burst), vec![Event::Paste("ls\nx".into())]);
        // Typing: one key at a time, or a burst without a line break (an IME commit).
        assert_eq!(coalesce_paste(vec![press(KeyCode::Enter)]), vec![press(KeyCode::Enter)]);
        // One line with its Enter (automation, "send text") stays keys.
        let line = vec![press(KeyCode::Char('l')), press(KeyCode::Char('s')), press(KeyCode::Enter)];
        assert_eq!(coalesce_paste(line.clone()), line);
        let ime = vec![press(KeyCode::Char('日')), press(KeyCode::Char('本'))];
        assert_eq!(coalesce_paste(ime.clone()), ime);
        // A shortcut in between ends the run.
        let ctrl_c = Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        let keys = vec![press(KeyCode::Char('a')), ctrl_c.clone(), press(KeyCode::Enter)];
        assert_eq!(coalesce_paste(keys.clone()), keys);
    }
}
