//! Page slide and zoom transitions, the small effects and hover highlights.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use noble::app::App;
use noble::event::AppEvent;
use noble::keys::Action;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::time::Duration;

use super::common::*;

fn animated_app() -> App {
    let mut app = demo_app(110, 30);
    let mut cfg = app.cfg.clone();
    cfg.general.animations = true;
    app.apply_config(cfg);
    app
}

#[test]
fn page_slide_transition() {
    let mut app = animated_app();
    render(&mut app, 110, 30);
    app.run(Action::Settings);
    // First frame: the transition starts, the new page is not fully in place yet.
    let first = render(&mut app, 110, 30);
    assert!(app.slide.is_some());
    save("slide-start-110x30", &first);
    // Move the transition to its middle.
    if let Some(s) = &mut app.slide {
        s.started = std::time::Instant::now() - Duration::from_millis(90);
    }
    let mid = render(&mut app, 110, 30);
    save("slide-mid-110x30", &mid);
    std::thread::sleep(noble::app::SLIDE_DURATION + Duration::from_millis(20));
    app.tick();
    assert!(app.slide.is_none());
    let done = render(&mut app, 110, 30);
    assert!(done.contains("Transparent background"));
    assert_ne!(mid, done);
    // The way back slides in the opposite direction.
    app.run(Action::Bridge);
    render(&mut app, 110, 30);
    assert_eq!(app.slide.as_ref().map(|s| s.dir), Some(-1));
    // On a size change the transition is cancelled safely.
    render(&mut app, 80, 20);
    render(&mut app, 30, 8);
}

#[test]
fn zoom_animation_grows_from_tile() {
    let mut app = animated_app();
    app.new_tab(std::env::temp_dir(), None, Some("zoom".into()));
    app.run(Action::SplitRight);
    render(&mut app, 110, 30);
    let pane = app.tabs[0].focus;
    app.run(Action::Zoom);
    let z = app.zoom_anim.as_ref().expect("zoom animation");
    assert_eq!(z.pane, pane);
    assert_eq!(z.to, app.body());
    assert!(z.from.width < z.to.width, "should grow from its tile");
    if let Some(z) = &mut app.zoom_anim {
        z.started = std::time::Instant::now() - Duration::from_millis(60);
    }
    save("zoom-in-mid-110x30", &render(&mut app, 110, 30));
    std::thread::sleep(noble::app::ZOOM_DURATION + Duration::from_millis(20));
    app.tick();
    assert!(app.zoom_anim.is_none());
    // Coming back: it shrinks from fullscreen back to its place.
    app.run(Action::Zoom);
    let z = app.zoom_anim.as_ref().unwrap();
    assert_eq!(z.from, app.body());
    assert!(z.to.width < z.from.width);
    save("zoom-out-start-110x30", &render(&mut app, 110, 30));
    app.run(Action::CloseTab);
}

/// The small effects: each starts on the frame that sees the change and shows an in-between state,
/// nothing breaks mid-way at small sizes, and once they settle the screen is the one drawn without them.
#[test]
fn effects_play_and_settle() {
    use noble::app::fx::Fx;
    use noble::app::{Hit, SettingItem, SettingKey, ToastLevel};
    let sizes = [(160, 45), (80, 20), (30, 8)];
    let settle = |app: &mut App| {
        std::thread::sleep(Duration::from_millis(400));
        app.tick();
    };
    let mut app = animated_app();
    render(&mut app, 110, 30);
    // A new tab grows open in the strip.
    app.new_tab(std::env::temp_dir(), None, Some("grow".into()));
    let title = app.tab_title(0);
    let first = render(&mut app, 110, 30);
    assert!(!first.lines().next().unwrap().contains(&title), "{first}");
    for (w, h) in sizes {
        render(&mut app, w, h);
    }
    settle(&mut app);
    assert!(render(&mut app, 110, 30).lines().next().unwrap().contains(&title));
    // A split unfolds from the divider and the new pane's frame lights up.
    app.run(Action::SplitRight);
    render(&mut app, 110, 30);
    let new = app.tabs[0].focus;
    assert!(app.fx.raw(Fx::Split(new)).is_some() && app.fx.raw(Fx::Focus(new)).is_some());
    let width = |app: &App| find_hit(app, |h| matches!(h, Hit::Pane { pane, .. } if *pane == new)).map(|r| r.width);
    let early = width(&app);
    for (w, h) in sizes {
        render(&mut app, w, h);
    }
    settle(&mut app);
    render(&mut app, 110, 30);
    assert!(early < width(&app), "{early:?} {:?}", width(&app));
    // A notification slides in from the right edge.
    app.toast(ToastLevel::Info, "effects toast");
    assert!(!render(&mut app, 110, 30).contains("effects toast"));
    settle(&mut app);
    assert!(render(&mut app, 110, 30).contains("effects toast"));
    // Home fades in row by row; Home and System mid-way at every size.
    for action in [Action::Bridge, Action::System, Action::Bridge] {
        app.run(action);
        for (w, h) in sizes {
            render(&mut app, w, h);
        }
    }
    assert!(app.fx.raw(Fx::HomeIn).is_some());
    // The settled comparison below runs over Settings: a live shell may redraw its prompt between two frames.
    app.run(Action::Settings);
    settle(&mut app);
    // An overlay fades its backdrop in; its selection glides.
    app.run(Action::Palette);
    render(&mut app, 110, 30);
    assert!(app.fx.raw(Fx::Backdrop).is_some());
    app.handle(AppEvent::Input(crossterm::event::Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))));
    for (w, h) in sizes {
        render(&mut app, w, h);
    }
    settle(&mut app);
    // Settled: the same picture as without animations.
    // The "Animations" row itself shows the setting and is left out, and the clock's digits are masked:
    // a minute may turn between the two frames. (The palette cursor stays lit: the demo runs on battery.)
    // The palette's one-cell gap cuts the row's label behind it short, so the row is matched by the prefix
    // that survives the cut.
    let without_setting = |s: String| {
        let mut lines = s.lines().filter(|l| !l.contains("Animat")).map(str::to_string).collect::<Vec<_>>();
        lines[0] = lines[0].replace(|c: char| c.is_ascii_digit(), "#");
        lines.join("\n")
    };
    let animated = without_setting(render(&mut app, 110, 30));
    let mut cfg = app.cfg.clone();
    cfg.general.animations = false;
    app.apply_config(cfg);
    assert_eq!(animated, without_setting(render(&mut app, 110, 30)));
    // A laptop (the demo runs on battery) gets "Animations on battery"; off, it stops them on battery.
    let item = SettingItem::Setting(SettingKey::AnimBattery);
    assert!(app.settings_items().contains(&item));
    let mut cfg = app.cfg.clone();
    cfg.general.animations = true;
    app.apply_config(cfg);
    assert!(app.on_battery() && app.anim_on());
    app.activate_setting(item, 1);
    assert!(!app.anim_on());
    app.overlay = None;
    app.run(Action::CloseTab);
}

#[test]
fn hover_highlights_clickable_items() {
    use crossterm::event::{Event, MouseEvent, MouseEventKind};
    use noble::app::Hit;
    let mut app = demo_app(110, 30);
    render(&mut app, 110, 30);
    let row = find_hit(&app, |h| *h == Hit::Project(2)).unwrap();
    let base = {
        let mut term = Terminal::new(TestBackend::new(110, 30)).unwrap();
        term.draw(|f| noble::ui::draw(f, &mut app)).unwrap();
        term.backend().buffer()[(row.x + 5, row.y)].bg
    };
    app.handle(AppEvent::Input(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column: row.x + 5,
        row: row.y,
        modifiers: KeyModifiers::NONE,
    })));
    let mut term = Terminal::new(TestBackend::new(110, 30)).unwrap();
    term.draw(|f| noble::ui::draw(f, &mut app)).unwrap();
    let hovered = term.backend().buffer()[(row.x + 5, row.y)].bg;
    assert_ne!(base, hovered, "row should highlight under the mouse");
    assert_eq!(hovered, app.theme.hover());
}
