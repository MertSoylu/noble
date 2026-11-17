//! Quota pace forecast: when a quota window fills at the current rate of use, in time and on the clock.
//!
//! Everything here is a pure function of the usage history, the window and `now` (unix seconds); the wall clock and the
//! local time zone are passed in, so the forecast, its text and the once-per-window warning gate are testable with
//! fixed times. `ui/bridge.rs` draws the lines on Home, `app/quota.rs` raises the warning when new quota data arrives.

use chrono::{DateTime, Datelike, TimeZone};

use crate::ai::{ProviderState, Window};
use crate::store::UsageHistory;

/// How a window kind is forecast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    /// The history that counts, ending now (seconds): the rate is the one of this recent stretch.
    pub look_back: i64,
    /// The first and last sample inside the look-back must be at least this far apart (seconds).
    pub min_span: i64,
    /// Length of the window (seconds): its start is the reset time minus this.
    pub length: i64,
}

/// The 5-hour window. The last 2 hours are the current burst of work (40% of the window): old enough to smooth a
/// pause between two prompts, recent enough that a long idle start of the window does not dilute the rate. 15 minutes
/// of rise are needed because a quota moves in 1% steps and two samples a minute apart would give a wild rate.
pub const FIVE_HOUR: Rule = Rule { look_back: 2 * 3600, min_span: 15 * 60, length: 5 * 3600 };

/// The 7-day windows. Use over a week is bursty (work hours, nights, weekends), so a 2-hour look-back would either
/// extrapolate one sprint over days or read an idle evening as "never". The last 24 hours hold one whole work/sleep
/// cycle, so the rate is a daily average (overnight idle included), which is what the remaining days will look like.
/// At least 3 hours between the first and last sample, because history only grows while Home is open and a rise
/// measured over a shorter stretch is mostly rounding (1% steps) and one session's burst.
pub const WEEKLY: Rule = Rule { look_back: 24 * 3600, min_span: 3 * 3600, length: 7 * 86_400 };

/// The rule for a window label. Providers' labels are normalized (`ai::json::normalize_label`,
/// `label_for_duration`): every weekly window (`seven_day`, `weekly`, `secondary`, a 7-day duration) is "WEEK", and
/// Claude's separate Opus allowance (`seven_day_opus`) is a 7-day window as well, labeled "OPUS". The other labels
/// (DAY, MONTH, a custom minute count) have no forecast.
pub fn rule(label: &str) -> Option<Rule> {
    match label {
        "5H" => Some(FIVE_HOUR),
        "WEEK" | "OPUS" => Some(WEEKLY),
        _ => None,
    }
}

/// A window that fills before it resets at the current pace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Forecast {
    /// The window's label ("5H", "WEEK").
    pub label: String,
    /// Seconds until it is full.
    pub eta: u64,
    /// When it is full (unix seconds).
    pub full_at: i64,
    /// When the window resets (unix seconds); `full_at` is before it.
    pub resets_at: i64,
}

/// The forecast of one window: `None` without a rule, a reset time, enough rising history, or when the window
/// resets before it would fill (or already has).
pub fn forecast(history: &UsageHistory, provider: &str, w: &Window, now: i64) -> Option<Forecast> {
    let rule = rule(&w.label)?;
    let resets_at = w.resets_at.filter(|r| *r > now)?;
    let key = UsageHistory::key(provider, &w.label);
    let start = resets_at.saturating_sub(rule.length);
    let eta = history.pace_eta(&key, start, now, w.used, rule.look_back, rule.min_span)?;
    let full_at = now.saturating_add(eta.min(i64::MAX as u64) as i64);
    (full_at < resets_at).then(|| Forecast { label: w.label.clone(), eta, full_at, resets_at })
}

/// The forecasts of a provider's windows, in the order the provider lists them.
pub fn forecasts(history: &UsageHistory, p: &ProviderState, now: i64) -> Vec<Forecast> {
    let Some(usage) = &p.usage else { return Vec::new() };
    usage.windows.iter().filter_map(|w| forecast(history, p.id, w, now)).collect()
}

/// A moment as a local clock time: "14:20" on the day of `now`, "Thu 14:20" on another day (a weekly window fills days
/// ahead). 24-hour on every platform and locale.
pub fn clock<Tz: TimeZone>(ts: i64, now: i64, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let at: Option<DateTime<Tz>> = tz.timestamp_opt(ts, 0).single();
    let Some(at) = at else { return String::new() };
    let same_day = tz.timestamp_opt(now, 0).single().is_some_and(|n| n.num_days_from_ce() == at.num_days_from_ce());
    if same_day { at.format("%H:%M").to_string() } else { at.format("%a %H:%M").to_string() }
}

/// The Home line, shortened until it fits `width` cells:
/// `▲ 5h full ~14:20 (in 1h20m) · resets 15:00`, then without the reset, then without the duration, then bare.
/// The line is always one row, so the panel's height does not depend on the width.
pub fn line<Tz: TimeZone>(f: &Forecast, now: i64, tz: &Tz, width: usize) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let name = crate::ai::window_name(&f.label);
    let at = clock(f.full_at, now, tz);
    let left = crate::ai::reset_in(f.full_at, now);
    let reset = clock(f.resets_at, now, tz);
    let shapes = [
        format!("▲ {name} full ~{at} (in {left}) · resets {reset}"),
        format!("▲ {name} full ~{at} (in {left})"),
        format!("▲ {name} full ~{at}"),
        format!("▲ {name} ~{at}"),
    ];
    let last = shapes[3].clone();
    shapes.into_iter().find(|s| crate::util::width(s) <= width).unwrap_or(last)
}

/// The notification text: "Claude Code 5h quota will run out ~14:20 at this pace".
pub fn warning_text<Tz: TimeZone>(name: &str, f: &Forecast, now: i64, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    format!(
        "{name} {} quota will run out ~{} at this pace",
        crate::ai::window_name(&f.label),
        clock(f.full_at, now, tz)
    )
}

/// How far a window's reset time may move between two fetches and still be the same window. Providers round their reset
/// times differently from one response to the next (a few seconds, sometimes a minute), and a window's reset time
/// only jumps by its whole length (hours) when a new window starts.
pub const SAME_WINDOW_SLACK: i64 = 30 * 60;

/// May the pace warning be raised for a window that resets at `resets_at`, when `warned` is the reset time of the
/// window it was last raised for? Once per window: not again while the reset time stays (within the slack) or moves
/// back; a later reset time is a new window.
pub fn warn_due(warned: Option<i64>, resets_at: i64) -> bool {
    warned.is_none_or(|w| resets_at > w.saturating_add(SAME_WINDOW_SLACK))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    const H: i64 = 3600;

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    fn win(label: &str, used: u8, resets_at: Option<i64>) -> Window {
        Window { label: label.into(), used, resets_at }
    }

    /// 2026-01-07 (Wednesday) 12:00:00 UTC.
    const T0: i64 = 1_767_787_200;

    #[test]
    fn five_hour_window_forecast() {
        let mut h = UsageHistory::memory();
        h.record("claude/5H", T0 - H, 50);
        h.record("claude/5H", T0, 70);
        // 20% an hour, 30% left: full in 90 minutes, but the window resets in 1 hour: no forecast.
        assert_eq!(forecast(&h, "claude", &win("5H", 70, Some(T0 + H)), T0), None);
        let f = forecast(&h, "claude", &win("5H", 70, Some(T0 + 4 * H)), T0).unwrap();
        assert_eq!((f.eta, f.full_at, f.resets_at), (90 * 60, T0 + 90 * 60, T0 + 4 * H));
        // The sample before the window started (reset 5 hours ahead starts it now) is not used.
        assert_eq!(forecast(&h, "claude", &win("5H", 70, Some(T0 + 5 * H)), T0), None);
        // A window that has reset already says nothing, and neither does a missing reset time.
        assert_eq!(forecast(&h, "claude", &win("5H", 70, Some(T0 - 1)), T0), None);
        assert_eq!(forecast(&h, "claude", &win("5H", 70, None), T0), None);
    }

    #[test]
    fn weekly_window_uses_a_day_of_history() {
        let mut h = UsageHistory::memory();
        // 24% over the last 24 hours: 1% an hour, 64% left, 64 hours to full.
        h.record("codex/WEEK", T0 - 24 * H, 12);
        h.record("codex/WEEK", T0, 36);
        let reset = T0 + 6 * 86_400;
        let f = forecast(&h, "codex", &win("WEEK", 36, Some(reset)), T0).unwrap();
        assert_eq!(f.eta, 64 * H as u64);
        assert!(f.full_at < reset, "fills before the reset");
        // The same pace with the reset 2 days away: the week resets first.
        assert_eq!(forecast(&h, "codex", &win("WEEK", 36, Some(T0 + 2 * 86_400)), T0), None);
        // Under 3 hours of history is not enough for a week.
        let mut short = UsageHistory::memory();
        short.record("codex/WEEK", T0 - 2 * H, 20);
        short.record("codex/WEEK", T0, 30);
        assert_eq!(forecast(&short, "codex", &win("WEEK", 30, Some(reset)), T0), None, "under 3 h of history");
        // Samples older than 24 hours are not part of the rate: the burst before them does not count.
        let mut old = UsageHistory::memory();
        old.record("codex/WEEK", T0 - 40 * H, 0);
        old.record("codex/WEEK", T0 - 30 * H, 40);
        old.record("codex/WEEK", T0 - 5 * H, 40);
        old.record("codex/WEEK", T0, 40);
        assert_eq!(forecast(&old, "codex", &win("WEEK", 40, Some(reset)), T0), None, "flat over the last day");
    }

    #[test]
    fn every_weekly_label_has_the_weekly_rule() {
        // What the providers produce for a 7-day window.
        for key in ["seven_day", "weekly", "secondary", "7d", "week"] {
            assert_eq!(rule(&crate::ai::json::normalize_label(key)), Some(WEEKLY), "{key}");
        }
        assert_eq!(rule(&crate::ai::json::label_for_duration(7.0 * 86_400.0)), Some(WEEKLY));
        assert_eq!(rule("OPUS"), Some(WEEKLY), "seven_day_opus");
        for key in ["five_hour", "hourly", "primary", "session"] {
            assert_eq!(rule(&crate::ai::json::normalize_label(key)), Some(FIVE_HOUR), "{key}");
        }
        assert_eq!(rule(&crate::ai::json::label_for_duration(5.0 * 3600.0)), Some(FIVE_HOUR));
        assert_eq!(rule("MONTH"), None);
        assert_eq!(rule("DAY"), None);
    }

    #[test]
    fn clock_names_the_day_only_when_it_is_not_today() {
        assert_eq!(clock(T0 + 2 * H + 20 * 60, T0, &utc()), "14:20");
        assert_eq!(clock(T0 + 2 * 86_400 + 9 * H, T0, &utc()), "Fri 21:00");
        // Midnight is the other day, whatever the distance.
        assert_eq!(clock(T0 + 12 * H, T0, &utc()), "Thu 00:00");
        // The zone shifts the clock and the day boundary: 12:00 UTC is 23:00 at +11, so 14:20 UTC is tomorrow there.
        let east = FixedOffset::east_opt(11 * 3600).unwrap();
        assert_eq!(clock(T0 + 60, T0, &east), "23:01");
        assert_eq!(clock(T0 + 2 * H + 20 * 60, T0, &east), "Thu 01:20");
        let west = FixedOffset::west_opt(5 * 3600).unwrap();
        assert_eq!(clock(T0 + 2 * H + 20 * 60, T0, &west), "09:20");
    }

    #[test]
    fn line_shortens_to_the_width() {
        let f = Forecast { label: "5H".into(), eta: 4800, full_at: T0 + 2 * H + 20 * 60, resets_at: T0 + 3 * H };
        let full = "▲ 5h full ~14:20 (in 2h20m) · resets 15:00";
        assert_eq!(line(&f, T0, &utc(), 80), full);
        assert_eq!(line(&f, T0, &utc(), 42), full);
        assert_eq!(line(&f, T0, &utc(), 41), "▲ 5h full ~14:20 (in 2h20m)");
        assert_eq!(line(&f, T0, &utc(), 27), "▲ 5h full ~14:20 (in 2h20m)");
        assert_eq!(line(&f, T0, &utc(), 26), "▲ 5h full ~14:20");
        assert_eq!(line(&f, T0, &utc(), 16), "▲ 5h full ~14:20");
        assert_eq!(line(&f, T0, &utc(), 15), "▲ 5h ~14:20");
        assert_eq!(line(&f, T0, &utc(), 3), "▲ 5h ~14:20", "the drawing cuts what still does not fit");
        let week =
            Forecast { label: "WEEK".into(), eta: 0, full_at: T0 + 2 * 86_400 + 9 * H, resets_at: T0 + 3 * 86_400 };
        assert_eq!(line(&week, T0, &utc(), 80), "▲ week full ~Fri 21:00 (in 2d9h) · resets Sat 12:00");
    }

    #[test]
    fn warning_text_names_provider_window_and_time() {
        let f = Forecast { label: "5H".into(), eta: 0, full_at: T0 + 2 * H + 20 * 60, resets_at: T0 + 3 * H };
        assert_eq!(
            warning_text("Claude Code", &f, T0, &utc()),
            "Claude Code 5h quota will run out ~14:20 at this pace"
        );
    }

    #[test]
    fn warning_gate_is_once_per_reset_period() {
        let reset = T0 + 3 * H;
        assert!(warn_due(None, reset), "never warned");
        assert!(!warn_due(Some(reset), reset), "the same window");
        assert!(!warn_due(Some(reset), reset + 59), "a reset time rounded differently is the same window");
        assert!(!warn_due(Some(reset), reset - 400), "an earlier reset time is not a new window");
        assert!(warn_due(Some(reset), reset + 5 * H), "the next 5-hour window");
        assert!(warn_due(Some(reset), reset + 7 * 86_400), "the next week");
    }
}
