//! Focus mode: a terminal tab hides NOBLE's top and bottom bars, so its panes get the whole screen.
//! The top bar comes back over the content, without resizing the panes, once the pointer has rested on the top
//! row; the bottom bar comes back only while the prefix is armed. The screen's last row belongs to the terminal,
//! so a program's clicks there reach it. Other tabs' markers stay visible as a chip on the top row (drawn by `ui`).
//! The setting is `terminal.focus_mode`; prefix f toggles the running app's flag.

use std::time::{Duration, Instant};

use ratatui::layout::Position;

use super::{App, Hit, ToastLevel, View};
use crate::keys::Action;

/// How long the pointer must rest on the top row before the top bar appears.
pub const EDGE_DWELL: Duration = Duration::from_millis(400);

impl App {
    /// Focus mode is on and a terminal tab is shown: the bars are hidden.
    pub fn focus_mode_active(&self) -> bool {
        self.focus_mode && matches!(self.view, View::Term(_))
    }

    /// Whether the top and the bottom bar are drawn now, as `(top, bottom)`. Outside focus mode both are
    /// always drawn (the bars are where they have always been). In focus mode both show while the prefix is
    /// armed; otherwise only the top bar shows, once the pointer has rested on the top row for `EDGE_DWELL`.
    pub fn focus_reveal(&self) -> (bool, bool) {
        if !self.focus_mode_active() || self.prefix_armed {
            return (true, true);
        }
        let top = self.focus_top_since.is_some_and(|since| since.elapsed() >= EDGE_DWELL);
        (top, false)
    }

    /// Follows the pointer: a dwell starts when it moves onto the top row and ends when it leaves that row. Only
    /// focus mode on a terminal tab has a dwell to track.
    pub(super) fn track_focus_edge(&mut self, x: u16, y: u16) {
        // A pane's own buttons (split, zoom, close) sit on the top row; resting on one must not cover it with a bar.
        // The hits of the last frame still list them while a bar is drawn over them.
        let on_pane_button = self.hits.iter().any(|(r, h)| {
            matches!(h, Hit::PaneSplit { .. } | Hit::PaneZoom(_) | Hit::PaneClose(_)) && r.contains(Position::new(x, y))
        });
        let on_top = y == 0 && self.focus_mode_active() && !on_pane_button;
        if !on_top {
            self.focus_top_since = None;
        } else if self.focus_top_since.is_none() {
            self.focus_top_since = Some(Instant::now());
        }
    }

    /// Time until a pending top-row dwell ends (its bar then appears): `redraw_after` wakes up for it.
    pub(super) fn focus_dwell_due(&self) -> Option<Duration> {
        if !self.focus_mode_active() {
            return None;
        }
        let since = self.focus_top_since?;
        let left = EDGE_DWELL.checked_sub(since.elapsed())?;
        Some(left + Duration::from_millis(5))
    }

    /// Prefix f and the palette: flips focus mode for this run (the config keeps its own value).
    pub(super) fn toggle_focus_mode(&mut self) {
        self.focus_mode = !self.focus_mode;
        self.focus_top_since = None;
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
