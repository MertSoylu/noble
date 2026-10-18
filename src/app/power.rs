//! Battery and clock: what running on battery changes (sensor rate, clock seconds, cursor blink, warnings).

use super::*;

impl App {
    /// Warns once each when dropping to 20% and 10% while on battery.
    pub(super) fn check_battery(&mut self) {
        let Some((b, eta)) = self.sensors.battery() else { return };
        if b.state != crate::battery::PowerState::Discharging {
            self.battery_warned = 100;
            return;
        }
        let level = [10u8, 20].into_iter().find(|l| b.percent <= *l as f32 && *l < self.battery_warned);
        if let Some(level) = level {
            let text = format!("battery at {:.0}% · {}", b.percent, crate::battery::describe(b, eta));
            self.battery_warned = level;
            self.toast(if level <= 10 { ToastLevel::Error } else { ToastLevel::Warn }, text);
        }
    }

    /// Running on laptop battery (`false` when plugged in or without a battery).
    pub fn on_battery(&self) -> bool {
        self.sensors.battery().is_some_and(|(b, _)| b.state == crate::battery::PowerState::Discharging)
    }

    /// Seconds and the two blinking dots of the Home clock: they stop on battery
    /// (so the screen draws only once a minute).
    pub fn live_clock(&self) -> bool {
        !self.on_battery()
    }

    /// The time the clocks show: now, or the fixed `clock` of the screenshots.
    pub fn now(&self) -> chrono::DateTime<chrono::Local> {
        self.clock.unwrap_or_else(chrono::Local::now)
    }

    /// Whether a text field's cursor is drawn now: it blinks every 500 ms, and like the
    /// Home clock it stays lit on battery so an open palette does not redraw twice a second.
    pub fn cursor_lit(&self) -> bool {
        !self.live_clock() || (self.started.elapsed().as_millis() / 500).is_multiple_of(2)
    }

    /// Tunes background work to the visible screen: sensor rate and refreshing
    /// (at most once a minute) the git status of the projects shown on Home.
    pub(super) fn sync_power_state(&mut self) {
        let want = match self.view {
            View::System => SensorMode::Detail,
            View::Bridge => SensorMode::Summary,
            _ => SensorMode::Background,
        };
        let on_battery = self.on_battery();
        if self.sensor_mode != Some(want) || self.sensor_on_battery != on_battery {
            if let Some(s) = &self.services {
                let _ = s.sensor_req.send(SensorRequest::Mode(want, on_battery));
            }
            self.sensor_mode = Some(want);
            self.sensor_on_battery = on_battery;
        }
        // The quota panel only exists on Home: refresh only there, immediately on entry.
        let home = self.view == View::Bridge;
        if home != (self.last_view == Some(View::Bridge))
            && let Some(s) = &self.services
        {
            let _ = s.ai_refresh.send(crate::ai::AiReq::Visible(home));
        }
        if self.view == View::Bridge && self.last_view != Some(View::Bridge) {
            self.ports_asked = None;
            let vis = self.visible_projects();
            let from = self.bridge.proj_sel.saturating_sub(10);
            let paths: Vec<PathBuf> = vis.iter().skip(from).take(25).map(|i| self.projects[*i].path.clone()).collect();
            for p in paths {
                self.request_git(p, Duration::from_secs(60));
            }
        }
        self.last_view = Some(self.view);
    }
}
