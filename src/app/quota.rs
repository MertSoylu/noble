//! Quota warnings beyond Home's panel. All of them use the cached numbers: nothing here makes a request.
//! A full quota marks its launcher chip and says when it resets at launch; an agent pane's title shows the fullest
//! window past `warn_at`; a window that passed `warn_at` is noticed when its cached reset time comes.

use super::*;

/// The notice for a window whose reset time came: "Claude Code 5h quota reset".
fn reset_text(name: &str, label: &str) -> String {
    format!("{name} {} quota reset", crate::ai::window_name(label))
}

impl App {
    /// The full window of the quota that the launcher `command`'s agent runs on, with its provider's state.
    fn full_quota(&self, command: &str, now: i64) -> Option<(&ProviderState, &crate::ai::Window)> {
        let kind = crate::ai::agent_kind(Some(command), "")?;
        let state = self.quota_providers().find(|p| Some(p.id) == crate::ai::quota_provider(kind))?;
        Some((state, crate::ai::full_window(state, now)?))
    }

    /// Does the launcher's agent have a full quota now? Its chip shows ⚠ (`ui::bridge`).
    pub fn launcher_quota_full(&self, command: &str) -> bool {
        self.full_quota(command, chrono::Utc::now().timestamp()).is_some()
    }

    /// Launching an agent whose quota is full says when it resets ("Claude Code 5h quota is full · resets in 1h12m").
    /// The launch itself goes on.
    pub(super) fn warn_full_quota(&mut self, command: &str) {
        let now = chrono::Utc::now().timestamp();
        let text = match self.full_quota(command, now) {
            Some((state, window)) => {
                let name = crate::ai::window_name(&window.label);
                let reset = window
                    .resets_at
                    .map(|t| format!(" · resets in {}", crate::ai::reset_in(t, now)))
                    .unwrap_or_default();
                format!("{} {name} quota is full{reset}", state.name)
            }
            None => return,
        };
        self.toast(ToastLevel::Warn, text);
    }

    /// The agent pane's title warning ("5h 92%"): the fullest window of its quota at or past `warn_at` that has not
    /// reset yet. `None` while none is, or when `warn_at` is 0.
    pub fn agent_quota_warning(&self, kind: &str) -> Option<String> {
        let state = self.quota_providers().find(|p| Some(p.id) == crate::ai::quota_provider(kind))?;
        let w = crate::ai::warned_window(state, self.cfg.ai.warn_at, chrono::Utc::now().timestamp())?;
        Some(format!("{} {}%", crate::ai::window_name(&w.label), w.used))
    }

    /// Arms the windows at or past `warn_at` until they reset (`check_quota_resets` notices that). A window whose reset
    /// time moved on to a later one has ended already (a reset gives a later window), so it is noticed at once.
    pub(super) fn arm_quota_resets(&mut self, state: &ProviderState, windows: &[crate::ai::Window], now: i64) {
        let limit = self.cfg.ai.warn_at;
        for w in windows {
            let key = (state.id, w.label.clone());
            if let Some(&(name, t)) = self.quota_armed.get(&key)
                && w.resets_at.is_some_and(|r| r > t)
            {
                self.quota_armed.remove(&key);
                self.toast(ToastLevel::Ok, reset_text(name, &w.label));
            }
            if let Some(t) = w.resets_at
                && w.used >= limit
                && t > now
            {
                self.quota_armed.insert(key, (state.name, t));
            }
        }
    }

    /// Notices the armed windows whose cached reset time has come ("Claude Code 5h quota reset"). Runs on every tick;
    /// `quota_reset_wake` wakes the loop for the next one. Off while `warn_at` is 0 or AI is disabled.
    pub(super) fn check_quota_resets(&mut self, now: i64) {
        if self.cfg.ai.warn_at == 0 || !self.cfg.ai.enabled {
            self.quota_armed.clear();
            return;
        }
        let mut notes = Vec::new();
        self.quota_armed.retain(|(_, label), (name, t)| {
            let due = *t <= now;
            if due {
                notes.push(reset_text(name, label));
            }
            !due
        });
        for text in notes {
            self.toast(ToastLevel::Ok, text);
        }
    }

    /// When the next armed reset comes, for `redraw_after`: only times still ahead, so a due one never spins the loop.
    pub(super) fn quota_reset_wake(&self) -> Option<Duration> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        self.quota_armed
            .values()
            .map(|&(_, t)| t.saturating_mul(1000) - now_ms)
            .filter(|ms| *ms > 0)
            .min()
            // A few milliseconds past the second, so the check sees the reset time as reached.
            .map(|ms| Duration::from_millis(ms as u64 + 5))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{Presence, Status, Usage, Window};

    fn state(id: &'static str, name: &'static str, windows: Vec<Window>) -> ProviderState {
        ProviderState {
            id,
            name,
            login_hint: "login",
            presence: Presence::Ready,
            status: Status::Ok,
            usage: Some(Usage { windows, plan: None, note: None }),
            fetched_at: Some(1),
        }
    }

    fn win(label: &str, used: u8, resets_at: Option<i64>) -> Window {
        Window { label: label.into(), used, resets_at }
    }

    fn now() -> i64 {
        chrono::Utc::now().timestamp()
    }

    fn texts(app: &App) -> Vec<String> {
        app.toasts.iter().map(|t| t.text.clone()).collect()
    }

    #[test]
    fn reset_notice_comes_at_the_cached_reset_time() {
        let mut app = App::headless(Config::default(), (100, 30));
        let t = now() + 600;
        app.handle(AppEvent::Ai(Box::new(state("claude", "Claude Code", vec![win("5H", 92, Some(t))]))));
        // The crossing raises the usual warning; only the reset notice is checked here.
        app.toasts.clear();
        // The loop wakes once for it, about when the reset time comes.
        let wake = app.quota_reset_wake().expect("armed");
        assert!(wake > Duration::from_secs(590) && wake <= Duration::from_secs(601), "{wake:?}");
        app.check_quota_resets(t - 1);
        assert!(app.toasts.is_empty(), "not before the reset time");
        app.check_quota_resets(t);
        assert_eq!(texts(&app), vec!["Claude Code 5h quota reset".to_string()]);
        app.check_quota_resets(t + 60);
        assert_eq!(app.toasts.len(), 1, "notified once per window");
        assert_eq!(app.quota_reset_wake(), None);
    }

    #[test]
    fn a_window_below_warn_at_is_not_armed() {
        let mut app = App::headless(Config::default(), (100, 30));
        let t = now() + 600;
        app.handle(AppEvent::Ai(Box::new(state("claude", "Claude Code", vec![win("5H", 50, Some(t))]))));
        assert_eq!(app.quota_reset_wake(), None);
        app.check_quota_resets(t + 1);
        assert!(app.toasts.is_empty());
    }

    #[test]
    fn a_changed_reset_time_ends_the_armed_window_at_once() {
        let mut app = App::headless(Config::default(), (100, 30));
        let t = now() + 600;
        app.handle(AppEvent::Ai(Box::new(state("codex", "Codex", vec![win("WEEK", 95, Some(t))]))));
        app.toasts.clear();
        // The provider reports a new window (a later reset time): the old one reset early.
        app.handle(AppEvent::Ai(Box::new(state("codex", "Codex", vec![win("WEEK", 10, Some(t + 5000))]))));
        assert_eq!(texts(&app), vec!["Codex week quota reset".to_string()]);
    }

    #[test]
    fn a_reset_time_that_moves_back_is_not_a_reset() {
        let mut app = App::headless(Config::default(), (100, 30));
        let t = now() + 4000;
        app.handle(AppEvent::Ai(Box::new(state("claude", "Claude Code", vec![win("5H", 100, Some(t))]))));
        app.toasts.clear();
        // An earlier reset time with the window still high: the window has not ended, so no reset is noticed now.
        app.handle(AppEvent::Ai(Box::new(state("claude", "Claude Code", vec![win("5H", 92, Some(t - 400))]))));
        assert!(!texts(&app).iter().any(|s| s.ends_with("quota reset")), "{:?}", texts(&app));
        // The reset is noticed at the reset time the provider gives now.
        app.toasts.clear();
        app.check_quota_resets(t - 400);
        assert_eq!(texts(&app), vec!["Claude Code 5h quota reset".to_string()]);
    }

    #[test]
    fn turning_warnings_off_drops_the_armed_resets() {
        let mut app = App::headless(Config::default(), (100, 30));
        let t = now() + 600;
        app.handle(AppEvent::Ai(Box::new(state("claude", "Claude Code", vec![win("5H", 92, Some(t))]))));
        assert!(app.quota_reset_wake().is_some());
        app.toasts.clear();
        app.cfg.ai.warn_at = 0;
        app.check_quota_resets(t + 1);
        assert!(app.toasts.is_empty() && app.quota_reset_wake().is_none());
    }

    #[test]
    fn full_quota_marks_the_launcher_and_says_when_it_resets() {
        let mut app = App::headless(Config::default(), (100, 30));
        // 1h12m ahead: the reset text rounds down, so a few seconds of test time do not change it.
        let t = now() + 4320 + 30;
        app.handle(AppEvent::Ai(Box::new(state("claude", "Claude Code", vec![win("5H", 100, Some(t))]))));
        assert!(app.launcher_quota_full("claude"));
        assert!(!app.launcher_quota_full("codex"), "no codex state here");
        app.toasts.clear();
        app.warn_full_quota("claude");
        assert_eq!(texts(&app), vec!["Claude Code 5h quota is full · resets in 1h12m".to_string()]);
        // Its reset passed: the cached 100% says nothing about the quota now.
        app.handle(AppEvent::Ai(Box::new(state("claude", "Claude Code", vec![win("5H", 100, Some(now() - 5))]))));
        assert!(!app.launcher_quota_full("claude"));
    }

    #[test]
    fn agent_title_warning_only_past_warn_at() {
        let mut app = App::headless(Config::default(), (100, 30));
        let t = now() + 3600;
        app.handle(AppEvent::Ai(Box::new(state("claude", "Claude Code", vec![win("5H", 92, Some(t))]))));
        assert_eq!(app.agent_quota_warning("claude").as_deref(), Some("5h 92%"));
        assert_eq!(app.agent_quota_warning("codex"), None, "no codex quota");
        app.handle(AppEvent::Ai(Box::new(state("claude", "Claude Code", vec![win("5H", 85, Some(t))]))));
        assert_eq!(app.agent_quota_warning("claude"), None, "below warn_at");
        app.handle(AppEvent::Ai(Box::new(state("claude", "Claude Code", vec![win("5H", 97, Some(t))]))));
        app.cfg.ai.warn_at = 0;
        assert_eq!(app.agent_quota_warning("claude"), None, "warn_at 0 is off");
    }
}
