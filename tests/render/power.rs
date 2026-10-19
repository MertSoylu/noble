//! Redraw budget, the clock and battery, and providers that must not spin Home.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use noble::ai::{Presence, ProviderState, Status};
use noble::app::View;
use noble::event::AppEvent;
use noble::keys::Action;
use std::time::Duration;

use super::common::*;

/// Battery: shown on Home and System, warns once each at 20% and 10% while on
/// battery, never appears on a machine without a battery.
#[test]
fn battery_is_shown_and_warns_when_low() {
    use noble::battery::{Battery, PowerState};
    let mut app = demo_app(160, 45);
    let home = render(&mut app, 160, 45);
    save("bridge-battery-160x45", &home);
    assert!(home.contains("BATTERY") && home.contains("76%") && home.contains("3h 12m left"), "{home}");
    // The battery sits at the very bottom of the system panel (right above its frame).
    let lines: Vec<&str> = home.lines().collect();
    let status_row = lines.iter().position(|l| l.contains("3h 12m left")).unwrap();
    assert!(lines[status_row + 1].contains('╰'), "{home}");
    app.run(Action::System);
    let sys = render(&mut app, 110, 30);
    save("system-battery-110x30", &sys);
    assert!(sys.contains("76%") && sys.contains("3h 12m left"), "{sys}");
    let sys_big = render(&mut app, 160, 45);
    save("system-battery-160x45", &sys_big);
    assert!(sys_big.contains("BATTERY"), "{sys_big}");
    // A ↯ mark while charging.
    let mut charging = app.sensors.last.clone().unwrap();
    charging.battery =
        Some(Battery { percent: 64.0, state: PowerState::Charging, secs_left: None, secs_to_full: Some(2700) });
    app.handle(AppEvent::Sensors(Box::new(charging)));
    app.run(Action::Bridge);
    let text = render(&mut app, 160, 45);
    save("bridge-battery-charging-160x45", &text);
    assert!(text.contains("64% ↯") && text.contains("full in 45m"), "{text}");
    app.run(Action::System);

    let base = app.sensors.last.clone().unwrap();
    let sample = |percent: f32, state: PowerState| {
        let mut s = base.clone();
        s.battery = Some(Battery { percent, state, secs_left: None, secs_to_full: None });
        AppEvent::Sensors(Box::new(s))
    };
    app.toasts.clear();
    let ev = sample(19.0, PowerState::Discharging);
    app.handle(ev);
    let ev = sample(18.0, PowerState::Discharging);
    app.handle(ev);
    assert_eq!(app.toasts.len(), 1, "one warning at 20%");
    assert!(app.toasts[0].text.starts_with("battery at 19%"), "{}", app.toasts[0].text);
    let ev = sample(9.0, PowerState::Discharging);
    app.handle(ev);
    assert_eq!(app.toasts.len(), 2);
    // Plugging in and draining again re-arms the warning.
    let ev = sample(9.0, PowerState::Charging);
    app.handle(ev);
    app.toasts.clear();
    let ev = sample(9.0, PowerState::Discharging);
    app.handle(ev);
    assert_eq!(app.toasts.len(), 1);

    let mut desk = demo_app(160, 45);
    let mut s = desk.sensors.last.clone().unwrap();
    s.battery = None;
    desk.handle(AppEvent::Sensors(Box::new(s)));
    let text = render(&mut desk, 160, 45);
    assert!(!text.contains("BATTERY") && !text.contains("left"), "{text}");
}

/// Battery friendly drawing: invisible data never wakes the screen; an idle
/// terminal redraws once a minute, Home once a second, animation/palette more often.
#[test]
fn redraw_only_when_something_visible_changes() {
    use noble::battery::{Battery, PowerState};
    let mut app = demo_app(110, 30);
    let mut sample = app.sensors.last.clone().unwrap();
    // Plugged in: drawn within 1 s for Home's clock with seconds.
    sample.battery = Some(Battery { percent: 80.0, state: PowerState::Full, secs_left: None, secs_to_full: None });
    assert!(app.handle(AppEvent::Sensors(Box::new(sample.clone()))));
    assert!(app.live_clock());
    let home = app.redraw_after().unwrap();
    assert!(home <= Duration::from_millis(1005), "{home:?}");
    // On battery: seconds and blinking stop, the clock draws once a minute.
    sample.battery =
        Some(Battery { percent: 80.0, state: PowerState::Discharging, secs_left: None, secs_to_full: None });
    app.handle(AppEvent::Sensors(Box::new(sample.clone())));
    assert!(!app.live_clock());
    let home = app.redraw_after().unwrap();
    assert!(home > Duration::from_millis(1005) || chrono::Local::now().format("%S").to_string() == "59", "{home:?}");
    // Terminal: sensor/project events do not wake the screen, the clock draws once a minute.
    app.new_tab(std::env::temp_dir(), None, Some("quiet".into()));
    app.toasts.clear();
    assert!(!app.handle(AppEvent::Sensors(Box::new(sample.clone()))));
    assert!(!app.handle(AppEvent::Projects(app.projects.clone())));
    let term = app.redraw_after().unwrap();
    assert!(term <= Duration::from_secs(61), "{term:?}");
    // A key always redraws (alt+p is the shell's in a terminal); on battery the palette cursor stays lit.
    let key = |c, m| AppEvent::Input(crossterm::event::Event::Key(KeyEvent::new(KeyCode::Char(c), m)));
    assert!(app.handle(key('a', KeyModifiers::CONTROL)));
    assert!(app.handle(key(':', KeyModifiers::NONE)));
    assert!(app.cursor_lit());
    let palette = app.redraw_after().unwrap();
    // At second 59 the top strip's HH:MM is due within a second (as for Home above).
    assert!(
        palette > Duration::from_millis(500) || chrono::Local::now().format("%S").to_string() == "59",
        "{palette:?}"
    );
    // Plugged in, it blinks twice a second. Plugging in or unplugging redraws at once, even over a terminal.
    sample.battery = Some(Battery { percent: 80.0, state: PowerState::Full, secs_left: None, secs_to_full: None });
    assert!(app.handle(AppEvent::Sensors(Box::new(sample.clone()))));
    assert!(app.redraw_after().unwrap() <= Duration::from_millis(500));
    sample.battery =
        Some(Battery { percent: 80.0, state: PowerState::Discharging, secs_left: None, secs_to_full: None });
    assert!(app.handle(AppEvent::Sensors(Box::new(sample))));
    assert!(app.cursor_lit());
    app.overlay = None;
    // We wake at the moment a notification must expire and disappear.
    app.toast(noble::app::ToastLevel::Info, "hello");
    assert!(app.take_dirty());
    assert!(app.redraw_after().unwrap() <= Duration::from_millis(3100));
    app.run(Action::CloseTab);
}

/// Home's 120 ms loading spinner only runs for providers that are actually shown:
/// a configured but not installed provider (Pending forever) or a signed-out one
/// must not keep Home redrawing ~8 times a second.
#[test]
fn hidden_providers_do_not_spin_home() {
    let mut app = demo_app(110, 30);
    assert_eq!(app.view, View::Bridge);
    let hidden = |id: &'static str, presence: Presence, status: Status| {
        AppEvent::Ai(Box::new(ProviderState {
            id,
            name: id,
            login_hint: id,
            presence,
            status,
            usage: None,
            fetched_at: None,
        }))
    };
    let spinner = Duration::from_millis(120);
    assert!(app.redraw_after().unwrap() > spinner);
    // The collector reports a missing CLI as Pending and a signed-out one as SignIn: neither is drawn.
    app.handle(hidden("antigravity", Presence::NotInstalled, Status::Pending));
    app.handle(hidden("kilo", Presence::NoLogin, Status::SignIn));
    let home = app.redraw_after().unwrap();
    assert!(home > spinner, "hidden provider keeps Home spinning: {home:?}");
    // A shown provider that is loading does spin.
    app.handle(hidden("opencode-go", Presence::Ready, Status::Loading));
    assert!(app.redraw_after().unwrap() <= spinner);
    // With the quota panel disabled nothing is shown, so nothing spins.
    let mut cfg = app.cfg.clone();
    cfg.ai.enabled = false;
    app.apply_config(cfg);
    let home = app.redraw_after().unwrap();
    assert!(home > spinner, "disabled quota panel keeps Home spinning: {home:?}");
}

/// The big Home clock draws without seconds on battery and with seconds plugged in.
#[test]
fn clock_hides_seconds_on_battery() {
    use noble::battery::{Battery, PowerState};
    let mut app = demo_app(160, 45);
    let mut sample = app.sensors.last.clone().unwrap();
    sample.battery = Some(Battery { percent: 80.0, state: PowerState::Charging, secs_left: None, secs_to_full: None });
    app.handle(AppEvent::Sensors(Box::new(sample.clone())));
    let plugged = render(&mut app, 160, 45);
    sample.battery =
        Some(Battery { percent: 80.0, state: PowerState::Discharging, secs_left: None, secs_to_full: None });
    app.handle(AppEvent::Sensors(Box::new(sample)));
    let battery = render(&mut app, 160, 45);
    save("bridge-clock-battery-160x45", &battery);
    // The small seconds area right of the big clock (the digits' last row).
    let secs_row = |t: &str| t.lines().nth(4).unwrap_or("").chars().take(24).collect::<String>();
    let digits = |s: String| s.chars().filter(|c| c.is_ascii_digit()).count();
    assert_eq!(digits(secs_row(&plugged)), 2, "seconds shown when plugged in:\n{plugged}");
    assert_eq!(digits(secs_row(&battery)), 0, "no seconds on battery:\n{battery}");
}
