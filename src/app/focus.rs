//! Focus mode: a terminal tab hides NOBLE's top and bottom bars, so its panes get the whole screen.
//! The bars come back over the content, without resizing the panes, while the prefix is armed or the
//! pointer rests on the top or bottom row. Other tabs' markers stay visible as a chip on the bottom row
//! (drawn by `ui`). The setting is `terminal.focus_mode`; prefix f toggles the running app's flag.

use std::time::{Duration, Instant};

use ratatui::layout::Position;

use super::{App, Hit, ToastLevel, View};
use crate::keys::Action;

/// How long the pointer must rest on an edge row before the bar on it appears.
pub const EDGE_DWELL: Duration = Duration::from_millis(400);

/// The screen row whose bar the pointer rests on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusEdge {
    Top,
    Bottom,
}

impl App {
    /// Focus mode is on and a terminal tab is shown: the bars are hidden.
    pub fn focus_mode_active(&self) -> bool {
        self.focus_mode && matches!(self.view, View::Term(_))
    }

    /// Whether the top and the bottom bar are drawn now, as `(top, bottom)`. Outside focus mode both are
    /// always drawn (the bars are where they have always been). In focus mode both show while the prefix is
    /// armed, and otherwise the bar of an edge the pointer has rested on for `EDGE_DWELL`.
    pub fn focus_reveal(&self) -> (bool, bool) {
        if !self.focus_mode_active() || self.prefix_armed {
            return (true, true);
        }
        let dwelt =
            |edge: FocusEdge| self.focus_edge.is_some_and(|(e, since)| e == edge && since.elapsed() >= EDGE_DWELL);
        (dwelt(FocusEdge::Top), dwelt(FocusEdge::Bottom))
    }

    /// Follows the pointer's row: a dwell starts when it moves onto the top or bottom row and ends when it
    /// leaves that row. Only focus mode on a terminal tab has edges to track.
    pub(super) fn track_focus_edge(&mut self, x: u16, y: u16) {
        let last = self.size.1.saturating_sub(1);
        let edge = if y == 0 {
            Some(FocusEdge::Top)
        } else if y == last {
            Some(FocusEdge::Bottom)
        } else {
            None
        };
        // A pane's own buttons (split, zoom, close) sit on that row; resting on one must not cover it with a bar.
        // The hits of the last frame still list them while a bar is drawn over them.
        let on_pane_button = self.hits.iter().any(|(r, h)| {
            matches!(h, Hit::PaneSplit { .. } | Hit::PaneZoom(_) | Hit::PaneClose(_)) && r.contains(Position::new(x, y))
        });
        let edge = edge.filter(|_| self.focus_mode_active() && !on_pane_button);
        if self.focus_edge.map(|(e, _)| e) != edge {
            self.focus_edge = edge.map(|e| (e, Instant::now()));
        }
    }

    /// Time until a pending edge dwell ends (its bar then appears): `redraw_after` wakes up for it.
    pub(super) fn focus_dwell_due(&self) -> Option<Duration> {
        if !self.focus_mode_active() {
            return None;
        }
        let (_, since) = self.focus_edge?;
        let left = EDGE_DWELL.checked_sub(since.elapsed())?;
        Some(left + Duration::from_millis(5))
    }

    /// Prefix f and the palette: flips focus mode for this run (the config keeps its own value).
    pub(super) fn toggle_focus_mode(&mut self) {
        self.focus_mode = !self.focus_mode;
        self.focus_edge = None;
        let state = if self.focus_mode { "on" } else { "off" };
        let how = match self.keymap.prefix_key(Action::FocusMode) {
            Some(key) => format!(" — {} {key} toggles it", self.keymap.prefix),
            None => String::new(),
        };
        let scope = if matches!(self.view, View::Term(_)) { "" } else { " (applies to terminal tabs)" };
        let level = if self.focus_mode { ToastLevel::Ok } else { ToastLevel::Info };
        self.toast(level, format!("Focus mode {state}{how}{scope}"));
    }
}
