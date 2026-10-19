//! Redraw scheduling: the animations that need frames now, the dirty flag and the next wake-up.

use super::*;

impl App {
    /// Does an animation need frequent redraws?
    pub fn animating(&self) -> bool {
        self.moving() || self.fx.busy()
    }

    /// The animations drawn on every frame (the effects in `fx` pace themselves: `Effects::next_frame`).
    pub(super) fn moving(&self) -> bool {
        self.boot.is_some() || self.slide.is_some() || self.zoom_anim.is_some() || self.toast_moving()
    }

    /// Whether animations play now: the setting, and on battery the "animations on battery" one.
    pub fn anim_on(&self) -> bool {
        let g = &self.cfg.general;
        g.animations && (g.animations_on_battery || !self.on_battery())
    }

    /// Whether a non-event change happened since the last draw (read once).
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// State shown by every screen: notifications and tab markers.
    pub(super) fn ui_fingerprint(&self) -> (u64, usize, Vec<(bool, Option<TabAlert>)>) {
        (self.toast_serial, self.toasts.len(), self.tabs.iter().map(|t| (t.activity, t.alert)).collect())
    }

    /// When the screen changes even with no events: clock, animation,
    /// loading indicator, notification timeout… `None`: never changes.
    /// The main loop draws only when that moment arrives (or an event fires).
    pub fn redraw_after(&self) -> Option<Duration> {
        use chrono::Timelike;
        if self.moving() {
            return Some(Duration::from_millis(16));
        }
        let mut best: Option<Duration> = None;
        let mut want = |d: Duration| best = Some(best.map_or(d, |b| b.min(d)));
        // Effects: every frame while something fades, only at visible steps for sliding numbers.
        if let Some(d) = self.fx.next_frame() {
            want(d);
        }
        let now = chrono::Local::now();
        let into_sec = now.timestamp_subsec_millis().min(999) as u64;
        let to_second = Duration::from_millis(1000 - into_sec + 5);
        match self.view {
            // Home: seconds of the big clock and its two blinking dots (absent on battery).
            View::Bridge if self.live_clock() => want(to_second),
            // On other screens only HH:MM in the top strip.
            _ => want(to_second + Duration::from_secs(59 - now.second().min(59) as u64)),
        }
        let loading = match self.view {
            View::Bridge => {
                // Only providers on the quota panel spin: a configured but missing CLI stays
                // Pending forever and would otherwise redraw Home ~8 times a second.
                !self.projects_loaded
                    || self
                        .quota_providers()
                        .any(|p| matches!(p.status, crate::ai::Status::Loading | crate::ai::Status::Pending))
                    || self.sensors.last.is_none()
            }
            View::System => self.sensors.last.is_none(),
            _ => false,
        };
        if loading {
            want(Duration::from_millis(120));
        }
        if let Some(t) = self.toasts.iter().filter_map(|t| t.expires_at()).min() {
            want(t.saturating_duration_since(Instant::now()) + Duration::from_millis(5));
        }
        // Search matches that output made stale: recomputed when the refresh interval has passed.
        if let Some(s) = self.search.as_ref().filter(|s| s.stale) {
            want(s.refreshed.map_or(Duration::ZERO, |t| search::SEARCH_REFRESH.saturating_sub(t.elapsed())));
        }
        // A notification's countdown darkens one cell at a time: wake for the next cell only.
        if self.anim_on() {
            let now = Instant::now();
            for t in &self.toasts {
                let (Some(life), None) = (t.life, t.hover_from) else { continue };
                let cells = crate::ui::toast_cells(&t.text, self.size.0).max(1) as f64;
                let step = life.as_secs_f64() / cells;
                if step > 0.0 {
                    // The drawing rounds: the next cell turns half a step past a whole one.
                    let elapsed = t.active(now).as_secs_f64();
                    let k = (elapsed / step + 0.5).floor() + 0.5;
                    let at = Duration::from_secs_f64(k * step);
                    want(at.saturating_sub(Duration::from_secs_f64(elapsed)).max(Duration::from_millis(16)));
                }
            }
        }
        // The blinking cursor of the palette and of prompts (steady on battery, see `cursor_lit`).
        if self.live_clock() && matches!(self.overlay, Some(Overlay::Palette(_) | Overlay::Prompt(_))) {
            want(Duration::from_millis(500));
        }
        // The live timer of a command running in a visible pane (see `pane::live_timer`). A pane
        // running an agent shows the agent's state instead of the timer.
        let battery = self.on_battery();
        for id in self.visible_panes().into_iter().filter(|id| self.agent_state(*id).is_none()) {
            if let Some(elapsed) = self.panes.get(&id).and_then(|p| p.running_for()) {
                want(crate::term::pane::live_timer_next(elapsed, battery));
            }
        }
        // A working agent in a visible pane: its spinner (see `agent_spinning`), and the minutes
        // of its "working 2m", which change once a minute (on battery that is its only redraw).
        if self.agent_spinning() {
            want(Duration::from_millis(crate::ui::hud::AGENT_SPIN_MS));
        }
        let now_ms = chrono::Utc::now().timestamp_millis();
        for since in self.visible_panes().into_iter().filter_map(|p| self.agent_badge(p)?.working_since) {
            let into_minute = (now_ms - since * 1000).rem_euclid(60_000) as u64;
            want(Duration::from_millis(60_000 - into_minute + 5));
        }
        let charging = self.sensors.battery().is_some_and(|(b, _)| b.state == crate::battery::PowerState::Charging);
        if charging && matches!(self.view, View::Bridge | View::System) {
            want(Duration::from_millis(450));
        }
        best
    }
}
