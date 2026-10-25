//! Focus reports (DECSET 1004): a program that set the mode gets `CSI I` when its pane gains the focus and
//! `CSI O` when it loses it. The focused pane is the one the visible terminal tab focuses, and no pane has it
//! while NOBLE's window is in the background or another screen is shown. Only a change is reported, so a
//! repeated event sends nothing, and a pane whose program did not set the mode gets nothing.
//! On Windows ConPTY turns the mode on in every pane itself (its host asks for focus events), so there every
//! pane gets the reports; a console program reads them only as focus events, and a shell ignores them.
//!
//! Color scheme notices (DECSET 2031) work the same way: the pane colors are kept in `App::colors`, which the panes'
//! reader threads answer color queries from, and a program that set the mode gets `CSI ? 997 ; Ps n` when a change of
//! theme or scheme turns the background light or dark.

use super::App;
use crate::term::colors::ColorReport;
use crate::term::layout::PaneId;
use crate::theme::{TermPalette, Theme};

/// The reports for a change of focus from `before` to `after`, as `(pane, gained)`: the pane that lost the focus
/// first (so a program sees `CSI O` before `CSI I`), then the one that gained it.
pub fn transitions(before: Option<PaneId>, after: Option<PaneId>) -> Vec<(PaneId, bool)> {
    if before == after {
        return Vec::new();
    }
    before.map(|id| (id, false)).into_iter().chain(after.map(|id| (id, true))).collect()
}

impl App {
    /// The pane that has the focus now: the focused pane of the visible terminal tab, none while NOBLE's window
    /// is known to be in the background (`window_focused`, `None` when the terminal sends no focus events).
    fn focus_target(&self) -> Option<PaneId> {
        if self.window_focused == Some(false) {
            return None;
        }
        self.focused_pane().filter(|id| self.panes.contains_key(id))
    }

    /// Sends the focus reports for every change since the last call (`focus_held`). Called after each event and
    /// action, so a focus move that any path makes is reported once, and nothing is sent when nothing moved.
    pub(super) fn sync_focus_reports(&mut self) {
        let now = self.focus_target();
        if now == self.focus_held {
            return;
        }
        let before = std::mem::replace(&mut self.focus_held, now);
        for (id, gained) in transitions(before, now) {
            // A pane that closed meanwhile has no program left to tell; one without the mode is not told.
            if let Some(pane) = self.panes.get(&id).filter(|p| p.wants_focus_reports()) {
                pane.write(if gained { b"\x1b[I" } else { b"\x1b[O" });
            }
        }
    }

    /// The colors the panes are drawn with now: the same palette `ui/terminal.rs` resolves every frame. A transparent
    /// theme has no background of its own, so the opaque theme's stands in for it.
    fn color_report(&self) -> ColorReport {
        let t = &self.cfg.terminal;
        let pal = TermPalette::resolve(&self.term_schemes, &t.colors, &t.background, &t.foreground, &self.theme);
        let solid = Theme::by_name(self.theme.name, false);
        let fallback = TermPalette::resolve(&self.term_schemes, &t.colors, &t.background, &t.foreground, &solid);
        ColorReport::new(&pal, &fallback)
    }

    /// Replaces the shared pane colors when the palette changed (a theme, a scheme or a color in Settings, a reload
    /// of the config or of Windows Terminal's schemes), and sends `CSI ? 997 ; Ps n` to the panes whose program set
    /// mode 2031 when that turned the background light or dark. Returns the panes that were told. Called after each
    /// event, action and tick, like `sync_focus_reports`; only the main thread writes the report.
    pub fn sync_color_reports(&mut self) -> Vec<PaneId> {
        let now = self.color_report();
        let before = self.colors.read().unwrap_or_else(|e| e.into_inner()).clone();
        if now == before {
            return Vec::new();
        }
        let flipped = now.is_light() != before.is_light();
        let notice = now.scheme_notice();
        *self.colors.write().unwrap_or_else(|e| e.into_inner()) = now;
        if !flipped {
            return Vec::new();
        }
        let mut told: Vec<PaneId> =
            self.panes.iter().filter(|(_, p)| p.wants_scheme_notices()).map(|(id, _)| *id).collect();
        told.sort_unstable();
        for id in &told {
            self.panes[id].write(notice);
        }
        told
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_moves_report_the_loss_before_the_gain() {
        assert!(transitions(None, None).is_empty());
        assert!(transitions(Some(3), Some(3)).is_empty(), "the same pane again is no change");
        assert_eq!(transitions(None, Some(3)), vec![(3, true)]);
        assert_eq!(transitions(Some(3), None), vec![(3, false)]);
        assert_eq!(transitions(Some(3), Some(4)), vec![(3, false), (4, true)]);
    }
}
