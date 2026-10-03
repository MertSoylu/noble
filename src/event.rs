//! Events arriving from the background threads into the main loop.

use std::path::PathBuf;

use crate::ai::ProviderState;
use crate::projects::{GitInfo, Project};
use crate::sensors::{SensorSample, StaticInfo};
use crate::term::layout::PaneId;

pub enum AppEvent {
    Input(crossterm::event::Event),
    /// Output arrived in one or more panes (a per-pane `dirty` flag is kept).
    PtyOutput,
    PtyExit(PaneId),
    SensorStatic(Box<StaticInfo>),
    Sensors(Box<SensorSample>),
    KillResult {
        pid: u32,
        name: String,
        ok: bool,
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
}

pub type Tx = std::sync::mpsc::Sender<AppEvent>;

/// Windows: crossterm's console input has no bracketed paste, so the terminal's own paste (Ctrl+V in
/// Windows Terminal) arrives as a burst of key presses, each line break as an Enter. Key events that
/// were all waiting at once (`batch`: one read plus everything already queued) and hold text with a line
/// break cannot be typing, so they become one `Event::Paste`: it then gets the multi-line confirmation
/// and the bracketed-paste wrapping a Unix paste gets. A burst without a line break stays as keys (it
/// runs nothing on its own, and IMEs commit several characters at once). Unix terminals send bracketed
/// paste and never need this.
pub fn coalesce_paste(batch: Vec<crossterm::event::Event>) -> Vec<crossterm::event::Event> {
    use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
    let text_key = |ev: &Event| -> Option<Option<char>> {
        let Event::Key(k) = ev else { return None };
        if k.kind == KeyEventKind::Release {
            return Some(None);
        }
        if !(k.modifiers - KeyModifiers::SHIFT).is_empty() {
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
        if text.chars().count() >= 2 && text.contains('\n') && text.contains(|c: char| c != '\n') {
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
        let ime = vec![press(KeyCode::Char('日')), press(KeyCode::Char('本'))];
        assert_eq!(coalesce_paste(ime.clone()), ime);
        // A shortcut in between ends the run.
        let ctrl_c = Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        let keys = vec![press(KeyCode::Char('a')), ctrl_c.clone(), press(KeyCode::Enter)];
        assert_eq!(coalesce_paste(keys.clone()), keys);
    }
}
