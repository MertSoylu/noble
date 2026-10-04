//! Short UI effects: fades, flashes, glides and value tweens.
//!
//! The drawing code only gets `&App`, so the effect state sits behind `RefCell`s and the drawing
//! functions both read it and report what they saw (`observe_*`): a change from the previous frame
//! (a new tab, a new focus, an agent turning idle…) starts an effect. Everything runs on the main
//! thread. While effects are off (`set_enabled(false)`: the setting, or on battery when the user
//! chose so) nothing starts and every query answers with the final state, so a frame looks exactly
//! as it would without this module.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::term::TabAlert;
use crate::term::TabId;
use crate::term::layout::PaneId;
use crate::theme::Theme;

use super::AgentState;

/// A one-shot effect, keyed by what it belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fx {
    /// An overlay opened: the dimmed backdrop darkens in.
    Backdrop,
    /// Home came into view: its rows and panels fade in one after another.
    HomeIn,
    /// The update or hooks notice appeared: its button pulses a few times.
    Notice,
    /// A terminal tab appeared in the strip: it grows open.
    TabOpen(TabId),
    /// A background tab got a new marker (✓ ✗ ◆): it flashes.
    TabFlash(TabId),
    /// A pane took the focus: its frame lights up and fades back.
    Focus(PaneId),
    /// A pane was split off: it unfolds from the divider.
    Split(PaneId),
    /// An agent session changed state: its row on Home lights up.
    Agent(PaneId),
    /// The item under the mouse changed: its highlight fades in.
    Hover,
}

impl Fx {
    pub fn duration(&self) -> Duration {
        Duration::from_millis(match self {
            Fx::Backdrop => 140,
            Fx::HomeIn => HOME_STAGGER_MS * HOME_STEPS + HOME_FADE_MS,
            Fx::Notice => 1800,
            Fx::TabOpen(_) => 160,
            Fx::TabFlash(_) => 700,
            Fx::Focus(_) => 320,
            Fx::Split(_) => 180,
            Fx::Agent(_) => 900,
            Fx::Hover => 110,
        })
    }
}

/// Home's fade-in: each step (a project row, a panel) starts this much after the previous one…
const HOME_STAGGER_MS: u64 = 22;
/// …and takes this long; steps past the last one start together with it.
const HOME_FADE_MS: u64 = 160;
const HOME_STEPS: u64 = 10;
/// How long a value tween (bars, percentages) takes.
pub const TWEEN: Duration = Duration::from_millis(320);
/// How long a list's selection takes to move from row to row.
pub const GLIDE: Duration = Duration::from_millis(120);
/// How long a tab takes to close up the space it leaves.
pub const TAB_CLOSE: Duration = Duration::from_millis(160);
/// How long the frame loop keeps drawing after an effect ends (its final frame must be drawn).
pub const GRACE: Duration = Duration::from_millis(50);
/// How long a notification takes to slide in and out.
pub const TOAST_IN: Duration = Duration::from_millis(160);
pub const TOAST_OUT: Duration = Duration::from_millis(160);

/// A moment `d` ago (now, should the clock not reach back that far).
fn long_ago(d: Duration) -> Instant {
    Instant::now().checked_sub(d).unwrap_or_else(Instant::now)
}

/// Ease-out cubic of a linear 0..1 progress.
pub fn ease_out(t: f64) -> f64 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// Linear progress of something that started at `started` and lasts `dur` (1 once it is over).
pub fn linear(started: Instant, dur: Duration) -> f64 {
    (started.elapsed().as_secs_f64() / dur.as_secs_f64().max(f64::EPSILON)).clamp(0.0, 1.0)
}

struct Tween {
    from: f64,
    to: f64,
    started: Instant,
}

impl Tween {
    fn value(&self) -> f64 {
        self.from + (self.to - self.from) * ease_out(linear(self.started, TWEEN))
    }

    /// How long until the shown value crosses the next half percent (what a bar or a rounded number
    /// can show), or the tween ends; `None` once it has ended.
    fn next_change(&self) -> Option<Duration> {
        let elapsed = self.started.elapsed();
        if elapsed >= TWEEN {
            return None;
        }
        let (span, now) = (self.to - self.from, self.value());
        let next = if span > 0.0 { (now / STEP).floor() * STEP + STEP } else { (now / STEP).ceil() * STEP - STEP };
        // Inverse of `ease_out`: when the eased progress reaches that value.
        let e = (next - self.from) / span;
        let at = if (0.0..1.0).contains(&e) { TWEEN.mul_f64(1.0 - (1.0 - e).cbrt()) } else { TWEEN };
        Some(at.saturating_sub(elapsed).max(Duration::from_millis(4)))
    }
}

/// The finest change a tweened value shows (half a percent: a half-cell bar step, a rounded number).
const STEP: f64 = 0.5;
/// Frame interval while something fades or moves.
const FRAME: Duration = Duration::from_millis(16);

/// A list selection moving from one row to another.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glide {
    pub from: usize,
    pub to: usize,
    /// Eased progress, 1 when the move is over.
    pub t: f64,
}

impl Glide {
    /// Background of row `i`: the new row fades to `selected`, the old one back to `normal`.
    pub fn bg(&self, i: usize, normal: Color, selected: Color) -> Color {
        if i == self.to {
            Theme::mix(normal, selected, self.t)
        } else if i == self.from {
            Theme::mix(selected, normal, self.t)
        } else {
            normal
        }
    }
}

/// What the previous frames showed, to notice changes.
#[derive(Default)]
struct Seen {
    /// Tabs in strip order with their marker and drawn width; `None` before the first strip.
    tabs: Option<Vec<(TabId, Option<TabAlert>, u16)>>,
    /// A closed tab's place in the strip and width, closing up.
    closing: Option<(usize, u16, Instant)>,
    /// Per tab: its focused pane and its panes.
    focus: HashMap<TabId, (PaneId, Vec<PaneId>)>,
    /// Agent sessions on Home; `None` before Home showed them.
    agents: Option<HashMap<PaneId, AgentState>>,
    overlay: bool,
    notice: bool,
    hover: Option<Rect>,
    home: bool,
}

#[derive(Default)]
pub struct Effects {
    on: Cell<bool>,
    started: RefCell<HashMap<Fx, Instant>>,
    tweens: RefCell<HashMap<String, Tween>>,
    glides: RefCell<HashMap<&'static str, (usize, usize, Instant)>>,
    seen: RefCell<Seen>,
}

impl Effects {
    /// Turns effects on or off for the coming frame; off drops whatever is running.
    pub fn set_enabled(&self, on: bool) {
        if !on && self.on.get() {
            self.started.borrow_mut().clear();
            self.tweens.borrow_mut().clear();
            self.glides.borrow_mut().clear();
            self.seen.borrow_mut().closing = None;
        }
        self.on.set(on);
    }

    pub fn enabled(&self) -> bool {
        self.on.get()
    }

    /// Starts (or restarts) an effect.
    pub fn start(&self, fx: Fx) {
        if self.on.get() {
            self.started.borrow_mut().insert(fx, Instant::now());
        }
    }

    /// Linear progress of a running effect; `None` when it is not running.
    pub fn raw(&self, fx: Fx) -> Option<f64> {
        let started = *self.started.borrow().get(&fx)?;
        (started.elapsed() < fx.duration()).then(|| linear(started, fx.duration()))
    }

    /// Eased progress of a running effect; `None` when it is not running.
    pub fn progress(&self, fx: Fx) -> Option<f64> {
        self.raw(fx).map(ease_out)
    }

    /// Whether anything is still moving: the frame loop then draws at full rate.
    pub fn busy(&self) -> bool {
        if !self.on.get() {
            return false;
        }
        // Each effect stays busy a little past its end, so the frame loop draws its final state.
        self.started.borrow_mut().retain(|fx, s| s.elapsed() < fx.duration() + GRACE);
        let mut seen = self.seen.borrow_mut();
        if seen.closing.is_some_and(|c| c.2.elapsed() >= TAB_CLOSE + GRACE) {
            seen.closing = None;
        }
        // Finished tweens and glides stay: the next change starts from what they show.
        !self.started.borrow().is_empty()
            || self.tweens.borrow().values().any(|t| t.started.elapsed() < TWEEN + GRACE)
            || self.glides.borrow().values().any(|g| g.2.elapsed() < GLIDE + GRACE)
            || seen.closing.is_some()
    }

    /// When the running effects need the next frame: every frame while something fades or moves,
    /// only when a tweened value shows a new step otherwise. `None`: nothing runs.
    pub fn next_frame(&self) -> Option<Duration> {
        if !self.busy() {
            return None;
        }
        let smooth = !self.started.borrow().is_empty()
            || self.glides.borrow().values().any(|g| g.2.elapsed() < GLIDE + GRACE)
            || self.seen.borrow().closing.is_some();
        if smooth {
            return Some(FRAME);
        }
        self.tweens.borrow().values().filter_map(Tween::next_change).min()
    }

    /// The value to show for a number that moves to `target`: it slides there from what was shown.
    /// Changes smaller than `min_step` jump (a bar that only twitches is not worth the frames).
    pub fn tween(&self, key: &str, target: f64, min_step: f64) -> f64 {
        if !self.on.get() {
            return target;
        }
        let mut tweens = self.tweens.borrow_mut();
        // The last value shown, kept after the tween ends so the next change starts from it.
        let last = tweens.get(key).map(|t| (t.value(), t.to));
        match last {
            Some((_, to)) if to == target => tweens.get(key).map_or(target, Tween::value),
            Some((shown, _)) if (target - shown).abs() >= min_step => {
                tweens.insert(key.to_string(), Tween { from: shown, to: target, started: Instant::now() });
                shown
            }
            _ => {
                // First sight or a small change: show it as is.
                let done = long_ago(TWEEN + GRACE);
                tweens.insert(key.to_string(), Tween { from: target, to: target, started: done });
                target
            }
        }
    }

    /// The selection of the list `key` moving to row `sel`.
    pub fn glide(&self, key: &'static str, sel: usize) -> Glide {
        let still = Glide { from: sel, to: sel, t: 1.0 };
        if !self.on.get() {
            return still;
        }
        let mut glides = self.glides.borrow_mut();
        match glides.get(key).copied() {
            Some((_, to, _)) if to == sel => {}
            Some((_, to, _)) => {
                glides.insert(key, (to, sel, Instant::now()));
            }
            None => {
                glides.insert(key, (sel, sel, long_ago(GLIDE + GRACE)));
            }
        }
        let (from, to, started) = glides[key];
        if started.elapsed() >= GLIDE {
            return Glide { from: to, to, t: 1.0 };
        }
        Glide { from, to, t: ease_out(linear(started, GLIDE)) }
    }

    /// Forgets where lists were, so a list that opens again does not glide from its old row.
    fn forget_glides(&self) {
        self.glides.borrow_mut().clear();
    }

    /// Whether an overlay is open: opening one fades its backdrop in.
    pub fn observe_overlay(&self, open: bool) {
        let mut seen = self.seen.borrow_mut();
        if open != seen.overlay {
            seen.overlay = open;
            drop(seen);
            self.forget_glides();
            if open {
                self.start(Fx::Backdrop);
            }
        }
    }

    /// Whether Home is in view: coming back to it fades its rows in.
    pub fn observe_home(&self, shown: bool) {
        let mut seen = self.seen.borrow_mut();
        if shown != seen.home {
            seen.home = shown;
            drop(seen);
            if shown {
                self.start(Fx::HomeIn);
            }
        }
    }

    /// How far the Home fade-in is for step `n` (0 = first row): 0 hidden … 1 fully drawn.
    pub fn home_step(&self, n: u64) -> f64 {
        let Some(started) = self.started.borrow().get(&Fx::HomeIn).copied() else { return 1.0 };
        let ms = started.elapsed().as_secs_f64() * 1000.0 - (n.min(HOME_STEPS) * HOME_STAGGER_MS) as f64;
        ease_out(ms / HOME_FADE_MS as f64)
    }

    /// Whether the update/hooks notice is shown: when it appears its button pulses.
    pub fn observe_notice(&self, shown: bool) {
        let mut seen = self.seen.borrow_mut();
        if shown != seen.notice {
            seen.notice = shown;
            drop(seen);
            if shown {
                self.start(Fx::Notice);
            }
        }
    }

    /// The notice button's pulse: 0 (still) … 1 (brightest), three soft beats that fade out.
    pub fn notice_pulse(&self) -> f64 {
        self.raw(Fx::Notice).map_or(0.0, |t| (std::f64::consts::PI * 3.0 * t).sin().powi(2) * (1.0 - t))
    }

    /// The thing under the mouse: a new one fades its highlight in. Returns the fade's progress.
    pub fn observe_hover(&self, rect: Option<Rect>) -> f64 {
        let mut seen = self.seen.borrow_mut();
        if rect != seen.hover {
            seen.hover = rect;
            drop(seen);
            if rect.is_some() {
                self.start(Fx::Hover);
            }
        }
        self.progress(Fx::Hover).unwrap_or(1.0)
    }

    /// The terminal tabs in strip order (id, marker, drawn width, active). A new tab grows open, a
    /// closed one leaves a gap that closes up, a background tab whose marker changed flashes.
    pub fn observe_tabs(&self, tabs: &[(TabId, Option<TabAlert>, u16, bool)]) {
        let mut seen = self.seen.borrow_mut();
        let now: Vec<(TabId, Option<TabAlert>, u16)> = tabs.iter().map(|(id, a, w, _)| (*id, *a, *w)).collect();
        let Some(before) = seen.tabs.replace(now) else { return };
        let mut start = Vec::new();
        for (id, alert, _, active) in tabs {
            match before.iter().find(|b| b.0 == *id) {
                None => start.push(Fx::TabOpen(*id)),
                Some((_, old, _)) if alert.is_some() && alert != old && !active => start.push(Fx::TabFlash(*id)),
                _ => {}
            }
        }
        // One closed tab at a time closes up (several at once just jump).
        let gone: Vec<(usize, u16)> = before
            .iter()
            .enumerate()
            .filter(|(_, b)| !tabs.iter().any(|t| t.0 == b.0))
            .map(|(i, b)| (i, b.2))
            .collect();
        if let [(i, w)] = gone[..]
            && self.on.get()
        {
            seen.closing = Some((i, w, Instant::now()));
        }
        drop(seen);
        for fx in start {
            self.start(fx);
        }
    }

    /// The width a tab of full width `w` takes now (it grows from nothing when new).
    pub fn tab_width(&self, id: TabId, w: u16) -> u16 {
        self.progress(Fx::TabOpen(id)).map_or(w, |t| ((w as f64 * t).ceil() as u16).min(w))
    }

    /// The gap a closed tab leaves before strip position `index`: its width shrinking to nothing.
    pub fn tab_gap(&self) -> Option<(usize, u16)> {
        let (i, w, started) = self.seen.borrow().closing?;
        let t = ease_out(linear(started, TAB_CLOSE));
        let gap = (w as f64 * (1.0 - t)).round() as u16;
        (gap > 0).then_some((i, gap))
    }

    /// A visible tab's focused pane and its panes: a new focus flashes, a pane split off unfolds.
    pub fn observe_focus(&self, tab: TabId, focus: PaneId, panes: &[PaneId]) {
        let mut seen = self.seen.borrow_mut();
        let before = seen.focus.insert(tab, (focus, panes.to_vec()));
        drop(seen);
        let Some((old_focus, old_panes)) = before else { return };
        if old_focus == focus || panes.len() < 2 {
            return;
        }
        if !old_panes.contains(&focus) && panes.len() > old_panes.len() {
            self.start(Fx::Split(focus));
        }
        self.start(Fx::Focus(focus));
    }

    /// The agent sessions on Home: one that appears or changes state lights up its row.
    pub fn observe_agents(&self, sessions: impl Iterator<Item = (PaneId, AgentState)>) {
        let now: HashMap<PaneId, AgentState> = sessions.collect();
        let mut seen = self.seen.borrow_mut();
        let Some(before) = seen.agents.replace(now.clone()) else { return };
        drop(seen);
        for (pane, state) in now {
            if before.get(&pane) != Some(&state) {
                self.start(Fx::Agent(pane));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> Effects {
        let fx = Effects::default();
        fx.set_enabled(true);
        fx
    }

    #[test]
    fn off_means_final_state() {
        let fx = Effects::default();
        fx.start(Fx::Backdrop);
        assert_eq!(fx.progress(Fx::Backdrop), None);
        assert_eq!(fx.tween("x", 10.0, 1.0), 10.0);
        assert_eq!(fx.tween("x", 50.0, 1.0), 50.0);
        assert_eq!(fx.glide("l", 3), Glide { from: 3, to: 3, t: 1.0 });
        assert!(!fx.busy());
    }

    #[test]
    fn tween_starts_from_shown_value() {
        let fx = on();
        assert_eq!(fx.tween("cpu", 10.0, 1.0), 10.0, "first sight shows the value");
        assert!(!fx.busy());
        assert_eq!(fx.tween("cpu", 60.0, 1.0), 10.0, "a change starts where the bar was");
        assert!(fx.busy());
        let mid = fx.tween("cpu", 60.0, 1.0);
        assert!((10.0..=60.0).contains(&mid));
        // A tiny change jumps.
        let fx = on();
        fx.tween("ram", 40.0, 1.0);
        assert_eq!(fx.tween("ram", 40.4, 1.0), 40.4);
    }

    /// A sliding number wakes the screen only when its shown value moves on, and at its end.
    #[test]
    fn tween_paces_its_frames() {
        let fx = on();
        fx.tween("cpu", 10.0, 1.0);
        assert_eq!(fx.next_frame(), None);
        fx.tween("cpu", 11.0, 1.0);
        // 10 → 11 has two half-percent steps over 320 ms: the first comes well after one frame.
        let first = fx.next_frame().unwrap();
        assert!(first > FRAME && first < TWEEN, "{first:?}");
        // A big jump has many steps close together.
        fx.tween("ram", 0.0, 1.0);
        fx.tween("ram", 100.0, 1.0);
        assert!(fx.next_frame().unwrap() <= FRAME);
        // A fade draws every frame.
        let fx = on();
        fx.start(Fx::Hover);
        assert_eq!(fx.next_frame(), Some(FRAME));
    }

    #[test]
    fn glide_fades_rows() {
        let fx = on();
        assert_eq!(fx.glide("menu", 0).t, 1.0);
        let g = fx.glide("menu", 2);
        assert_eq!((g.from, g.to), (0, 2));
        assert!(g.t < 1.0);
        let (n, s) = (Color::Rgb(0, 0, 0), Color::Rgb(100, 100, 100));
        assert_eq!(g.bg(5, n, s), n);
        assert_eq!(Glide { from: 0, to: 2, t: 1.0 }.bg(2, n, s), s);
        assert_eq!(Glide { from: 0, to: 2, t: 1.0 }.bg(0, n, s), n);
        // Reopening the list starts fresh.
        fx.observe_overlay(true);
        assert_eq!(fx.glide("menu", 4).t, 1.0);
    }

    #[test]
    fn tabs_open_flash_and_close() {
        let fx = on();
        fx.observe_tabs(&[(1, None, 10, true)]);
        assert_eq!(fx.tab_width(1, 10), 10, "tabs at the first frame are not new");
        fx.observe_tabs(&[(1, None, 10, false), (2, None, 8, true)]);
        assert!(fx.tab_width(2, 8) < 8);
        fx.observe_tabs(&[(1, Some(TabAlert::Done), 10, false), (2, None, 8, true)]);
        assert!(fx.raw(Fx::TabFlash(1)).is_some());
        fx.observe_tabs(&[(2, None, 8, true)]);
        assert_eq!(fx.tab_gap().map(|g| g.0), Some(0));
    }

    #[test]
    fn focus_and_split() {
        let fx = on();
        fx.observe_focus(7, 1, &[1]);
        fx.observe_focus(7, 2, &[1, 2]);
        assert!(fx.raw(Fx::Split(2)).is_some());
        assert!(fx.raw(Fx::Focus(2)).is_some());
        fx.observe_focus(7, 1, &[1, 2]);
        assert!(fx.raw(Fx::Focus(1)).is_some());
        assert!(fx.raw(Fx::Split(1)).is_none());
    }

    #[test]
    fn agents_light_up_on_change() {
        let fx = on();
        fx.observe_agents([(1, AgentState::Working)].into_iter());
        assert!(fx.raw(Fx::Agent(1)).is_none(), "the first sight is not a change");
        fx.observe_agents([(1, AgentState::Idle)].into_iter());
        assert!(fx.raw(Fx::Agent(1)).is_some());
    }

    #[test]
    fn notice_pulse_fades() {
        let fx = on();
        assert_eq!(fx.notice_pulse(), 0.0);
        fx.observe_notice(true);
        assert!(fx.raw(Fx::Notice).is_some());
    }
}
