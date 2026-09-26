//! Application state and event dispatch. Drawing lives in the `ui` module;
//! everything here mutates state and nothing writes to the screen directly.

mod input;
mod menu;
mod ops;
mod palette;
mod search;
mod settings;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use ratatui::layout::Rect;

use crate::ai::{self, ProviderState};
use crate::config::{self, Config, Launcher, Paths};
use crate::event::{AppEvent, Tx};
use crate::keys::Keymap;
use crate::projects::{Project, ProjectReq};
use crate::sensors::{self, SensorMode, SensorRequest, Sensors};
use crate::store::{Recent, UsageHistory, Workspaces};
use crate::term::Tab;
use crate::term::layout::{Divider, PaneId};
use crate::term::pane::{Pane, ShellSpec, resolve_shell};
use crate::theme::Theme;

pub use menu::{Menu, MenuCmd, MenuItem, ProjectAct};
pub use palette::{PaletteCmd, PaletteItem, PaletteState};
pub use settings::{LAUNCH_KEYS, PREFIXES, PROVIDER_KEYS, SettingItem, SettingKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Bridge,
    System,
    Settings,
    Term(usize),
}

#[derive(Default)]
pub struct BridgeState {
    pub proj_sel: usize,
    pub filter: String,
    pub filtering: bool,
    /// Quick-action button of the selected row focused with → (★ pin, ⋯ menu).
    pub proj_act: Option<ProjectAct>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Cpu,
    Mem,
    Pid,
    Name,
}

pub struct SystemState {
    pub sort: SortKey,
    pub desc: bool,
    pub filter: String,
    pub filtering: bool,
    pub selected_pid: Option<u32>,
}

impl Default for SystemState {
    fn default() -> Self {
        Self { sort: SortKey::Cpu, desc: true, filter: String::new(), filtering: false, selected_pid: None }
    }
}

pub enum ConfirmAction {
    Quit,
    ClosePane(PaneId),
    /// The tab holding this pane.
    CloseTab(PaneId),
    Paste {
        pane: PaneId,
        text: String,
    },
    Kill {
        pid: u32,
        name: String,
    },
}

pub struct Confirm {
    pub title: String,
    pub body: String,
    pub action: ConfirmAction,
}

pub enum PromptPurpose {
    RenameTab(usize),
    SaveWorkspace,
    /// New root folder to scan projects in.
    AddRoot,
}

impl PromptPurpose {
    /// Maximum input length (paths can be long).
    pub fn max_len(&self) -> usize {
        match self {
            PromptPurpose::AddRoot => 260,
            _ => 40,
        }
    }
}

pub struct Prompt {
    pub title: String,
    pub value: String,
    pub purpose: PromptPurpose,
}

/// Terminal color scheme selector: previews while navigating, esc reverts.
pub struct SchemePicker {
    pub selected: usize,
    pub original: String,
}

pub enum Overlay {
    /// First launch: short intro and prefix key choice (`PREFIXES` order).
    Welcome {
        prefix: usize,
    },
    Palette(PaletteState),
    Schemes(SchemePicker),
    /// Quick launch settings: `selected`, the row in the installed launchers list.
    Launchers {
        selected: usize,
    },
    Menu(Menu),
    Help {
        scroll: u16,
    },
    Confirm(Confirm),
    Prompt(Prompt),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Ok,
    Warn,
    Error,
}

pub struct Toast {
    pub text: String,
    pub level: ToastLevel,
    pub until: Instant,
}

/// Mouse hit targets; rebuilt on every frame while drawing.
#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    Backdrop,
    /// Overlay box: clicks must not fall through to the background.
    Inert,
    TabBridge,
    TabSystem,
    TabSettings,
    Tab(usize),
    TabClose(usize),
    NewTab,
    Pane {
        pane: PaneId,
        inner: Rect,
    },
    PaneZoom(PaneId),
    PaneClose(PaneId),
    PaneSplit {
        pane: PaneId,
        dir: crate::term::layout::Dir,
    },
    Divider {
        tab: usize,
        div: Divider,
    },
    Project(usize),
    OpenSelected,
    Launcher(usize),
    AiRefresh,
    Setting(usize),
    /// Row in the scheme selector (`scheme_options` order).
    TermScheme(usize),
    /// Quick launch popup: show/hide and shortcut (`launchers` order).
    LaunchShow(usize),
    LaunchKey(usize),
    OpenFiles,
    Proc(u32),
    SortCol(SortKey),
    PaletteItem(usize),
    ConfirmYes,
    ConfirmNo,
    WelcomePrefix(usize),
    WelcomeDone,
    MenuItem(usize),
    /// Top row of the pane frame (for the right-click menu).
    PaneTitle(PaneId),
    /// Quick-action button on a project row (row, action).
    ProjectAct(usize, ProjectAct),
    /// Update notice at the bottom right: update / dismiss.
    Update,
    UpdateDismiss,
}

pub(crate) enum Drag {
    Divider {
        tab: usize,
        div: Divider,
    },
    /// Tab drag reordering.
    Tab {
        index: usize,
    },
    Select {
        pane: PaneId,
        inner: Rect,
    },
    Forward {
        pane: PaneId,
        inner: Rect,
    },
}

/// Scrollback search (in the focused pane, search bar on the bottom edge).
pub struct SearchState {
    pub pane: PaneId,
    pub query: String,
    pub matches: Vec<crate::term::pane::Match>,
    /// Selected match (index within `matches`).
    pub current: Option<usize>,
    /// Scrollback length when the matches were computed (absolute line → screen line).
    pub history: usize,
}

/// Link under the mouse while ctrl is held: pane, screen row, column span.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkHover {
    pub pane: PaneId,
    pub row: u16,
    pub from: u16,
    pub to: u16,
}

pub struct Boot {
    pub started: Instant,
}

pub const BOOT_DURATION: Duration = Duration::from_millis(1900);
pub const SLIDE_DURATION: Duration = Duration::from_millis(240);
pub const ZOOM_DURATION: Duration = Duration::from_millis(200);

/// Page transition: the previous page's body slides out sideways.
pub struct Slide {
    pub from: ratatui::buffer::Buffer,
    /// +1: the new page comes from the right, -1: from the left.
    pub dir: i32,
    pub started: Instant,
}

/// Pane fullscreen animation: grows/shrinks between the `from` and `to` rectangles.
pub struct ZoomAnim {
    pub pane: PaneId,
    pub from: Rect,
    pub to: Rect,
    pub started: Instant,
}

/// Smooth easing (ease-out cubic), 0..1.
pub fn ease(started: Instant, dur: Duration) -> f64 {
    let t = (started.elapsed().as_secs_f64() / dur.as_secs_f64()).clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// State of an AI session. With the hook installed it comes from Claude's own
/// events, otherwise it is guessed from the pane title/command (`Running`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentState {
    /// Prompt sent, Claude is working.
    Working,
    /// Waiting for permission or input.
    NeedsYou,
    /// Finished its answer; it is your turn.
    Idle,
    /// Open but the state is unknown (no hook).
    Running,
}

impl AgentState {
    pub fn label(&self) -> &'static str {
        match self {
            AgentState::Working => "working",
            AgentState::NeedsYou => "needs you",
            AgentState::Idle => "your turn",
            AgentState::Running => "running",
        }
    }
}

/// A row of the session list.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentSession {
    pub tab: usize,
    pub pane: PaneId,
    pub kind: &'static str,
    pub place: String,
    pub state: AgentState,
}

/// Signals collected from a pane in a single frame.
struct PaneSignal {
    id: PaneId,
    visible: bool,
    bell: bool,
    notice: Option<String>,
    /// If the prompt returned and the user started a command, that command's duration.
    finished: Option<Duration>,
    /// The pane's directory when its prompt came back.
    cwd: Option<PathBuf>,
}

/// Channels to the background services (absent in headless tests).
pub struct Services {
    pub sensor_req: Sender<SensorRequest>,
    pub rescan: Sender<ProjectReq>,
    pub ai_refresh: Sender<crate::ai::AiReq>,
    pub proj_cfg: Arc<Mutex<config::ProjectsCfg>>,
    pub ai_cfg: Arc<Mutex<config::AiCfg>>,
}

pub struct App {
    pub paths: Paths,
    pub cfg: Config,
    cfg_mtime: Option<SystemTime>,
    last_cfg_check: Instant,
    pub theme: Theme,
    pub keymap: Keymap,
    pub shell: ShellSpec,
    pub clipboard: crate::clipboard::Clipboard,
    pub view: View,
    pub tabs: Vec<Tab>,
    pub panes: HashMap<PaneId, Pane>,
    next_id: PaneId,
    pub prefix_armed: bool,
    /// The next key goes straight to this pane while it is focused (`passthrough = "once"`, prefix i).
    pub pass_next: Option<PaneId>,
    pub sensors: Sensors,
    pub projects: Vec<Project>,
    pub projects_loaded: bool,
    pub recent: Recent,
    pub workspaces: Workspaces,
    pub ai: Vec<ProviderState>,
    pub bridge: BridgeState,
    pub system: SystemState,
    pub settings_sel: usize,
    pub overlay: Option<Overlay>,
    pub toasts: Vec<Toast>,
    pub boot: Option<Boot>,
    pub hits: Vec<(Rect, Hit)>,
    /// Mouse cursor position (for hover effects).
    pub hover: Option<(u16, u16)>,
    pub slide: Option<Slide>,
    pub zoom_anim: Option<ZoomAnim>,
    /// Last drawn page and body image (to start a transition).
    pub drawn_view: Option<View>,
    pub last_body: Option<ratatui::buffer::Buffer>,
    pub(crate) drag: Option<Drag>,
    last_click: Option<(Instant, u16, u16)>,
    pub size: (u16, u16),
    tx: Tx,
    /// Event receiver in headless mode (tests drain it with `pump`).
    rx: Option<std::sync::mpsc::Receiver<AppEvent>>,
    services: Option<Services>,
    /// Launchers and whether they are on PATH.
    pub launchers: Vec<(Launcher, bool)>,
    /// AI providers installed on this machine (Settings lists only these).
    pub ai_installed: Vec<&'static str>,
    pub quit: bool,
    pub started: Instant,
    pub restored_tabs: usize,
    /// Running as `noble-dev`: marked in the top bar, session in a separate file.
    pub dev: bool,
    pub operator: String,
    /// Repos whose git status was requested and when (to thin out repeats).
    git_requested: HashMap<PathBuf, Instant>,
    pub search: Option<SearchState>,
    pub link_hover: Option<LinkHover>,
    pub usage_history: UsageHistory,
    pub ui_state: crate::store::UiState,
    /// Latest events from the Claude Code hooks (pane → record).
    pub agent_hooks: HashMap<PaneId, crate::hooks::HookRecord>,
    /// When each pane's shell prompt last came back (Unix seconds): a hook record written
    /// until then belongs to a program that has exited (`clear_agent`).
    agent_cleared: HashMap<PaneId, i64>,
    last_hook_scan: Instant,
    /// Whether the NOBLE hooks are installed in `~/.claude/settings.json` (shown in Settings).
    pub hooks_installed: bool,
    /// Last mode reported to the sensor thread (based on the visible screen).
    sensor_mode: Option<SensorMode>,
    /// The previous frame's view (to catch the return to Home).
    last_view: Option<View>,
    /// Last power state reported to the sensors (on battery or not).
    sensor_on_battery: bool,
    /// Incremented on every notification; tells us whether a redraw is needed.
    toast_serial: u64,
    /// The screen changed through a non-event path (tick, config reload).
    dirty: bool,
    /// Selectable terminal color schemes (read from Windows Terminal + built-ins).
    pub term_schemes: Vec<crate::theme::TermScheme>,
    /// Last low-battery threshold warned (20%, 10%); reset when plugged in.
    battery_warned: u8,
    /// Quota windows already warned about (provider, window, reset time).
    quota_warned: std::collections::HashSet<(String, String, Option<i64>)>,
    /// Send BEL once to the outer terminal (flashes the taskbar).
    pub outer_bell: bool,
    /// A newer published release (announced bottom right).
    pub update_available: Option<String>,
    /// Next update check (`None`: no checking, e.g. headless mode, `noble-dev`).
    next_update_check: Option<Instant>,
}

fn launcher_availability(list: &[Launcher]) -> Vec<(Launcher, bool)> {
    list.iter()
        .map(|l| {
            let program = l.command.split_whitespace().next().unwrap_or("");
            (l.clone(), crate::util::which(program).is_some())
        })
        .collect()
}

impl App {
    /// Launchers to show on Home: the ones found on PATH and not hidden
    /// (in their `launchers` order).
    pub fn quick_launchers(&self) -> impl Iterator<Item = (usize, &Launcher)> {
        self.launchers.iter().enumerate().filter(|(_, (l, ok))| *ok && l.show).map(|(i, (l, _))| (i, l))
    }

    /// Without background services (for tests and screenshots).
    pub fn headless(cfg: Config, size: (u16, u16)) -> App {
        let (tx, rx) = std::sync::mpsc::channel();
        let paths = Paths { config: PathBuf::from("config.toml"), data: std::env::temp_dir().join("noble-headless") };
        let mut app = App::build(cfg, paths, None, tx, size, Recent::memory(), Workspaces::memory());
        app.boot = None;
        app.rx = Some(rx);
        app
    }

    /// Handles pending events in headless mode (PTY output etc.).
    pub fn pump(&mut self) {
        let events: Vec<AppEvent> = match &self.rx {
            Some(rx) => rx.try_iter().collect(),
            None => return,
        };
        for ev in events {
            self.handle(ev);
        }
    }

    /// The real app: loads config, reads history, starts the services.
    pub fn start(paths: Paths, tx: Tx, size: (u16, u16)) -> App {
        let loaded = config::load(&paths.config);
        let cfg = loaded.config;
        let recent = Recent::load(paths.data_file("recent.json"));
        let workspaces = Workspaces::load(paths.data_file("workspaces.json"));

        let (sensor_tx, sensor_rx) = std::sync::mpsc::channel();
        sensors::spawn(tx.clone(), sensor_rx);
        let proj_cfg = Arc::new(Mutex::new(cfg.projects.clone()));
        let (rescan_tx, rescan_rx) = std::sync::mpsc::channel();
        crate::projects::spawn(proj_cfg.clone(), tx.clone(), rescan_rx);
        let ai_cfg = Arc::new(Mutex::new(cfg.ai.clone()));
        let (ai_tx, ai_rx) = std::sync::mpsc::channel();
        ai::spawn(ai_cfg.clone(), paths.data_file("ai-cache.json"), tx.clone(), ai_rx);

        let services = Services { sensor_req: sensor_tx, rescan: rescan_tx, ai_refresh: ai_tx, proj_cfg, ai_cfg };
        let cache = ai::load_cache(&paths.data_file("ai-cache.json"));
        let mut app = App::build(cfg, paths, Some(services), tx, size, recent, workspaces);
        app.ai_installed = ai::installed_providers();
        app.dev = crate::util::is_dev_build();
        app.usage_history = UsageHistory::load(app.paths.data_file("ai-history.json"));
        app.reload_schemes();
        app.ui_state = crate::store::UiState::load(app.paths.data_file("state.json"));
        crate::hooks::prune(&app.paths.data, std::process::id());
        app.hooks_installed = crate::hooks::settings_path().is_some_and(|p| crate::hooks::is_installed(&p));
        crate::update::cleanup_old();
        app.init_updates();
        if !app.ui_state.data.welcomed {
            app.show_welcome();
        }
        app.cfg_mtime = loaded.mtime;
        if app.cfg.ai.enabled {
            app.ai = ai::initial_states(&app.cfg.ai, &cache);
        }
        if let Some(err) = loaded.error {
            app.toast(ToastLevel::Error, err);
        }
        for w in app.keymap.warnings.clone() {
            app.toast(ToastLevel::Warn, w);
        }
        if app.cfg.terminal.restore_session
            && let Some(ws) = crate::store::load_session(&app.paths.data_file(app.session_file()))
        {
            app.restored_tabs = app.open_workspace(&ws);
            app.view = View::Bridge;
        }
        app
    }

    fn build(
        cfg: Config,
        paths: Paths,
        services: Option<Services>,
        tx: Tx,
        size: (u16, u16),
        recent: Recent,
        workspaces: Workspaces,
    ) -> App {
        let theme = Theme::by_name(&cfg.general.theme, cfg.general.transparent);
        let keymap = Keymap::from_config(&cfg.keys);
        let shell = resolve_shell(&cfg.terminal, &paths.data);
        let operator = if cfg.general.operator.trim().is_empty() {
            std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_else(|_| "operator".into())
        } else {
            cfg.general.operator.trim().to_string()
        };
        let launchers = launcher_availability(&cfg.launchers);
        let boot = cfg.general.boot_animation.then(|| Boot { started: Instant::now() });
        App {
            paths,
            cfg_mtime: None,
            last_cfg_check: Instant::now(),
            theme,
            keymap,
            shell,
            // OSC 52 goes to the real terminal only, never into test output.
            clipboard: crate::clipboard::Clipboard::new(services.is_some()),
            view: View::Bridge,
            tabs: Vec::new(),
            panes: HashMap::new(),
            next_id: 1,
            prefix_armed: false,
            pass_next: None,
            sensors: Sensors::default(),
            projects: Vec::new(),
            projects_loaded: false,
            recent,
            workspaces,
            ai: Vec::new(),
            bridge: BridgeState::default(),
            system: SystemState::default(),
            settings_sel: 0,
            overlay: None,
            toasts: Vec::new(),
            boot,
            hits: Vec::new(),
            hover: None,
            slide: None,
            zoom_anim: None,
            drawn_view: None,
            last_body: None,
            drag: None,
            last_click: None,
            size,
            tx,
            rx: None,
            services,
            launchers,
            ai_installed: ai::providers::registry().iter().map(|d| d.id).collect(),
            quit: false,
            started: Instant::now(),
            restored_tabs: 0,
            dev: false,
            operator,
            git_requested: HashMap::new(),
            search: None,
            link_hover: None,
            usage_history: UsageHistory::memory(),
            ui_state: crate::store::UiState::memory(),
            agent_hooks: HashMap::new(),
            agent_cleared: HashMap::new(),
            last_hook_scan: Instant::now(),
            hooks_installed: false,
            sensor_mode: None,
            last_view: None,
            sensor_on_battery: false,
            toast_serial: 0,
            dirty: false,
            term_schemes: crate::wt::all_schemes(None),
            battery_warned: 100,
            quota_warned: Default::default(),
            outer_bell: false,
            update_available: None,
            next_update_check: None,
            cfg,
        }
    }

    pub fn toast(&mut self, level: ToastLevel, text: impl Into<String>) {
        let text = text.into();
        self.toast_serial += 1;
        self.dirty = true;
        let secs = match level {
            ToastLevel::Error => 6,
            ToastLevel::Warn => 4,
            _ => 3,
        };
        self.toasts.retain(|t| t.text != text);
        self.toasts.push(Toast { text, level, until: Instant::now() + Duration::from_secs(secs) });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    /// Terminal body: the area between the top strip and the status bar.
    pub fn body(&self) -> Rect {
        Rect::new(0, 1, self.size.0, self.size.1.saturating_sub(2))
    }

    /// Does an animation need frequent redraws?
    pub fn animating(&self) -> bool {
        self.boot.is_some() || self.slide.is_some() || self.zoom_anim.is_some()
    }

    /// Handles the event; `true` when something visible changed (redraw).
    /// Changes in invisible data (e.g. sensors while in a terminal) never wake
    /// the screen: battery friendly.
    pub fn handle(&mut self, ev: AppEvent) -> bool {
        let shown = match &ev {
            AppEvent::Input(_) | AppEvent::PtyExit(_) | AppEvent::KillResult { .. } | AppEvent::Update(_) => true,
            AppEvent::PtyOutput => false,
            AppEvent::SensorStatic(_) | AppEvent::Sensors(_) => matches!(self.view, View::Bridge | View::System),
            AppEvent::Projects(_) | AppEvent::Git(..) | AppEvent::Ai(_) => {
                self.view == View::Bridge || self.overlay.is_some()
            }
        };
        let before = self.ui_fingerprint();
        let pty_shown = self.apply(ev);
        shown || pty_shown || self.ui_fingerprint() != before
    }

    /// Whether a non-event change happened since the last draw (read once).
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// State shown by every screen: notifications and tab markers.
    fn ui_fingerprint(&self) -> (u64, usize, Vec<(bool, bool)>) {
        (self.toast_serial, self.toasts.len(), self.tabs.iter().map(|t| (t.activity, t.alert)).collect())
    }

    /// The event's state change. `true`: output arrived in a visible pane.
    fn apply(&mut self, ev: AppEvent) -> bool {
        match ev {
            AppEvent::Input(e) => self.on_input(e),
            AppEvent::PtyOutput => return self.on_pty_output(),
            AppEvent::PtyExit(id) => {
                if let Some(cwd) = self.panes.get(&id).map(|p| p.cwd()) {
                    self.refresh_git_at(&cwd);
                }
                if self.panes.contains_key(&id) {
                    let label = self.panes.get(&id).map(|p| p.label()).unwrap_or_default();
                    self.close_pane(id);
                    self.toast(ToastLevel::Info, format!("{label} exited"));
                }
            }
            AppEvent::SensorStatic(info) => self.sensors.info = *info,
            AppEvent::Sensors(s) => {
                self.sensors.ingest(*s);
                self.check_battery();
            }
            AppEvent::KillResult { pid, name, ok } => {
                if ok {
                    self.toast(ToastLevel::Ok, format!("terminated {name} ({pid})"));
                } else {
                    self.toast(ToastLevel::Error, format!("could not terminate {name} ({pid})"));
                }
            }
            AppEvent::Projects(list) => self.set_projects(list),
            AppEvent::Git(path, info) => {
                if let Some(p) = self.projects.iter_mut().find(|p| p.path == path) {
                    if info.branch.is_some() {
                        p.branch = info.branch.clone();
                    }
                    p.git = Some(info);
                }
            }
            AppEvent::Ai(state) => {
                let state = *state;
                if state.status == crate::ai::Status::Ok {
                    self.on_fresh_usage(&state);
                }
                match self.ai.iter_mut().find(|s| s.id == state.id) {
                    Some(s) => *s = state,
                    None => self.ai.push(state),
                }
                let order =
                    |id: &str| crate::ai::providers::registry().iter().position(|d| d.id == id).unwrap_or(usize::MAX);
                self.ai.sort_by_key(|s| order(s.id));
            }
            AppEvent::Update(Ok(version)) => {
                self.ui_state.data.update_checked = chrono::Utc::now().timestamp();
                self.ui_state.data.update_latest = version.clone();
                self.ui_state.save();
                self.set_latest_version(&version);
            }
            // No network or GitHub did not answer: retry in an hour.
            AppEvent::Update(Err(_)) => {
                if self.next_update_check.is_some() {
                    self.next_update_check = Some(Instant::now() + Duration::from_secs(3600));
                }
            }
        }
        false
    }

    /// When the screen changes even with no events: clock, animation,
    /// loading indicator, notification timeout… `None`: never changes.
    /// The main loop draws only when that moment arrives (or an event fires).
    pub fn redraw_after(&self) -> Option<Duration> {
        use chrono::Timelike;
        if self.animating() {
            return Some(Duration::from_millis(16));
        }
        let mut best: Option<Duration> = None;
        let mut want = |d: Duration| best = Some(best.map_or(d, |b| b.min(d)));
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
                !self.projects_loaded
                    || self
                        .ai
                        .iter()
                        .any(|p| matches!(p.status, crate::ai::Status::Loading | crate::ai::Status::Pending))
                    || self.sensors.last.is_none()
            }
            View::System => self.sensors.last.is_none(),
            _ => false,
        };
        if loading {
            want(Duration::from_millis(120));
        }
        if let Some(t) = self.toasts.iter().map(|t| t.until).min() {
            want(t.saturating_duration_since(Instant::now()) + Duration::from_millis(5));
        }
        if matches!(self.overlay, Some(Overlay::Palette(_) | Overlay::Prompt(_))) {
            want(Duration::from_millis(500));
        }
        let charging = self.sensors.battery().is_some_and(|(b, _)| b.state == crate::battery::PowerState::Charging);
        if charging && matches!(self.view, View::Bridge | View::System) {
            want(Duration::from_millis(450));
        }
        best
    }

    /// Sets up the update check: the previous check's result shows right away,
    /// and if it is older than a day the first `tick` performs a new one.
    fn init_updates(&mut self) {
        if !crate::update::check_allowed() {
            return;
        }
        let latest = self.ui_state.data.update_latest.clone();
        self.set_latest_version(&latest);
        let age = chrono::Utc::now().timestamp().saturating_sub(self.ui_state.data.update_checked);
        let wait = (crate::update::CHECK_INTERVAL - age).clamp(0, crate::update::CHECK_INTERVAL);
        self.next_update_check = Some(Instant::now() + Duration::from_secs(wait as u64));
    }

    /// Latest known release: announced when newer than this one and not dismissed.
    pub fn set_latest_version(&mut self, version: &str) {
        let newer = crate::update::is_newer(version, crate::update::current());
        let skipped = self.ui_state.data.update_skipped == version;
        self.update_available = (newer && !skipped).then(|| version.to_string());
    }

    /// New version to show bottom right (none when the setting is off).
    pub fn update_notice(&self) -> Option<&str> {
        self.update_available.as_deref().filter(|_| self.cfg.general.check_updates)
    }

    /// Runs `noble update` in a new tab; the progress shows there.
    pub fn start_update(&mut self) {
        let Ok(exe) = std::env::current_exe() else { return };
        let command = self.shell.invocation_of(&exe, "update");
        let cwd = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        self.update_available = None;
        self.new_tab(cwd, Some(&command), Some("NOBLE update".into()));
    }

    /// Dismisses the notice: never shown again for the same version.
    pub fn dismiss_update(&mut self) {
        if let Some(v) = self.update_available.take() {
            self.ui_state.data.update_skipped = v;
            self.ui_state.save();
        }
    }

    /// Warns once each when dropping to 20% and 10% while on battery.
    fn check_battery(&mut self) {
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

    /// New quota data: recorded in history, warns once per window over the threshold.
    fn on_fresh_usage(&mut self, state: &ProviderState) {
        let Some(usage) = &state.usage else { return };
        let ts = state.fetched_at.unwrap_or_else(|| chrono::Utc::now().timestamp());
        for w in &usage.windows {
            self.usage_history.record(&UsageHistory::key(state.id, &w.label), ts, w.used);
        }
        self.usage_history.save();
        let limit = self.cfg.ai.warn_at;
        if limit == 0 {
            return;
        }
        for w in &usage.windows {
            let key = (state.id.to_string(), w.label.clone(), w.resets_at);
            if w.used < limit || self.quota_warned.contains(&key) {
                continue;
            }
            self.quota_warned.insert(key);
            let reset = w
                .resets_at
                .map(|t| {
                    let left = t.saturating_sub(chrono::Utc::now().timestamp()).max(0) as u64;
                    format!(" · resets in {}", crate::util::fmt_duration(Duration::from_secs(left)))
                })
                .unwrap_or_default();
            let window = crate::ai::window_name(&w.label);
            self.toast(ToastLevel::Warn, format!("{} {window} usage at {}%{reset}", state.name, w.used));
        }
    }

    /// Processes the hook records: notifies when a changed state is in a background tab.
    pub fn apply_hook_records(&mut self, records: HashMap<PaneId, crate::hooks::HookRecord>) {
        let visible: Vec<PaneId> = match self.view {
            View::Term(i) => self.tabs.get(i).map(|t| t.panes()).unwrap_or_default(),
            _ => Vec::new(),
        };
        let mut changed = Vec::new();
        let mut live = HashMap::new();
        for (pane, rec) in records {
            // Orphans (pane closed or never ours), finished sessions, and records from
            // before the pane's shell prompt came back (the agent has exited since).
            let stale = !self.panes.contains_key(&pane)
                || rec.event == crate::hooks::SESSION_END
                || self.agent_cleared.get(&pane).is_some_and(|&t| rec.ts <= t);
            if stale {
                crate::hooks::remove_record(&self.paths.data, std::process::id(), pane);
                continue;
            }
            if self.agent_hooks.get(&pane) != Some(&rec) {
                changed.push((pane, rec.clone()));
            }
            live.insert(pane, rec);
        }
        self.agent_hooks = live;
        for (pane, rec) in changed {
            let notice = match rec.event.as_str() {
                "notification" => Some(rec.message.clone().unwrap_or_else(|| "Claude needs your attention".into())),
                "stop" => Some("Claude finished".into()),
                _ => None,
            };
            let Some(notice) = notice else { continue };
            let sig = PaneSignal {
                id: pane,
                visible: visible.contains(&pane),
                bell: false,
                notice: None,
                finished: None,
                cwd: None,
            };
            if sig.visible {
                continue;
            }
            let signal = PaneSignal { notice: Some(notice), ..sig };
            self.notify(&signal);
        }
    }

    /// The pane's shell prompt came back, so whatever agent ran there has exited:
    /// its hook record goes, and records written until now are ignored if they show up
    /// later (a hook finishing its write just as the agent exits).
    ///
    /// Only the shell emits the prompt marks (OSC 7 / 9;9 / 133); Claude Code sets
    /// the title and OSC 9;4 progress but none of these, so a running agent is not
    /// cleared by it. Should one ever emit them, its next hook event (every prompt,
    /// notification and stop) writes a newer record and the state comes back.
    fn clear_agent(&mut self, pane: PaneId) {
        self.agent_cleared.insert(pane, chrono::Utc::now().timestamp());
        if self.agent_hooks.remove(&pane).is_some() {
            crate::hooks::remove_record(&self.paths.data, std::process::id(), pane);
        }
    }

    /// Forgets a closed pane's agent state and deletes its hook record.
    pub(crate) fn forget_agent(&mut self, pane: PaneId) {
        self.agent_cleared.remove(&pane);
        if self.agent_hooks.remove(&pane).is_some() {
            crate::hooks::remove_record(&self.paths.data, std::process::id(), pane);
        }
    }

    /// The AI agent running in a pane and its state: from Claude's hook record, else
    /// guessed from the launcher command (while it still runs) or the window title.
    pub fn agent_state(&self, pane: PaneId) -> Option<(&'static str, AgentState)> {
        if let Some(rec) = self.agent_hooks.get(&pane) {
            let state = match rec.event.as_str() {
                "prompt" => AgentState::Working,
                "notification" => AgentState::NeedsYou,
                "stop" | "session-start" => AgentState::Idle,
                _ => return None,
            };
            return Some(("claude", state));
        }
        let p = self.panes.get(&pane)?;
        // Once the launcher command has exited, the pane runs whatever was typed at the prompt.
        let command = p.command.as_deref().filter(|_| p.launch_running());
        let kind = crate::ai::agent_kind(command, &p.label())?;
        let alert = self.tabs.iter().any(|t| t.alert && t.root.contains(pane));
        Some((kind, if alert { AgentState::NeedsYou } else { AgentState::Running }))
    }

    /// AI sessions across all tabs (in tab order).
    pub fn all_agent_sessions(&self) -> Vec<AgentSession> {
        let mut out = Vec::new();
        for (ti, tab) in self.tabs.iter().enumerate() {
            for pane in tab.panes() {
                let Some((kind, state)) = self.agent_state(pane) else { continue };
                let cwd = self.panes.get(&pane).map(|p| p.cwd()).unwrap_or_default();
                let place = crate::projects::project_containing(&self.projects, &cwd)
                    .map(|p| p.name.clone())
                    .or_else(|| cwd.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_default();
                out.push(AgentSession { tab: ti, pane, kind, place, state });
            }
        }
        out
    }

    /// Open AI sessions in the project: (agent, state). If the same agent is in
    /// several panes, the state needing most attention is shown.
    pub fn agent_sessions(&self, project: &std::path::Path) -> Vec<(&'static str, AgentState)> {
        let rank = |s: AgentState| match s {
            AgentState::NeedsYou => 3,
            AgentState::Idle => 2,
            AgentState::Working => 1,
            AgentState::Running => 0,
        };
        let mut out: Vec<(&'static str, AgentState)> = Vec::new();
        for s in self.all_agent_sessions() {
            let Some(p) = self.panes.get(&s.pane) else { continue };
            let here = crate::projects::project_containing(&self.projects, &p.cwd())
                .is_some_and(|pr| pr.path.as_path() == project);
            if !here {
                continue;
            }
            match out.iter_mut().find(|(k, _)| *k == s.kind) {
                Some(entry) if rank(s.state) > rank(entry.1) => entry.1 = s.state,
                Some(_) => {}
                None => out.push((s.kind, s.state)),
            }
        }
        out
    }

    fn set_projects(&mut self, mut list: Vec<Project>) {
        // Keep the previous git info (so a re-scan does not flicker).
        for p in &mut list {
            if let Some(old) = self.projects.iter().find(|o| o.path == p.path) {
                p.git = old.git.clone();
            }
        }
        let selected = self.visible_projects().get(self.bridge.proj_sel).map(|i| self.projects[*i].path.clone());
        self.projects = list;
        self.projects_loaded = true;
        if let Some(path) = selected
            && let Some(pos) = self.visible_projects().iter().position(|i| self.projects[*i].path == path)
        {
            self.bridge.proj_sel = pos;
        }
        self.sort_pinned();
        let n = self.visible_projects().len();
        self.bridge.proj_sel = self.bridge.proj_sel.min(n.saturating_sub(1));
    }

    /// Indices of the projects that match the filter.
    pub fn visible_projects(&self) -> Vec<usize> {
        let q = self.bridge.filter.trim();
        if q.is_empty() {
            return (0..self.projects.len()).collect();
        }
        let mut scored: Vec<(usize, i32)> = self
            .projects
            .iter()
            .enumerate()
            .filter_map(|(i, p)| crate::util::fuzzy_score(q, &p.name).map(|s| (i, s)))
            .collect();
        scored.sort_by_key(|(_, score)| std::cmp::Reverse(*score));
        scored.into_iter().map(|(i, _)| i).collect()
    }

    pub fn selected_project(&self) -> Option<&Project> {
        let idx = *self.visible_projects().get(self.bridge.proj_sel)?;
        self.projects.get(idx)
    }

    fn on_pty_output(&mut self) -> bool {
        let visible: Vec<PaneId> = match self.view {
            View::Term(i) => self.tabs.get(i).map(|t| t.panes()).unwrap_or_default(),
            _ => Vec::new(),
        };
        let mut clip: Option<String> = None;
        let mut signals = Vec::new();
        for (id, pane) in &self.panes {
            if pane.dirty.swap(false, std::sync::atomic::Ordering::AcqRel) {
                if let Some(c) = pane.take_clipboard() {
                    clip = Some(c);
                }
                let prompt = pane.take_prompt();
                signals.push(PaneSignal {
                    id: *id,
                    visible: visible.contains(id),
                    bell: pane.take_bell(),
                    notice: pane.take_notice(),
                    finished: prompt.then(|| pane.command_started.map(|t| t.elapsed())).flatten(),
                    cwd: prompt.then(|| pane.cwd()),
                });
            }
        }
        let mut shown = signals.iter().any(|s| s.visible);
        for sig in signals {
            if let Some(cwd) = &sig.cwd {
                // Command finished: the repo may have changed.
                self.refresh_git_at(cwd);
                // An agent there has exited; the tab title in the top bar may change too.
                shown |= self.agent_state(sig.id).is_some();
                self.clear_agent(sig.id);
                if let Some(p) = self.panes.get_mut(&sig.id) {
                    p.command_started = None;
                    p.prompted = true;
                }
            }
            if self.search.as_ref().is_some_and(|s| s.pane == sig.id) {
                self.refresh_search();
            }
            self.notify(&sig);
        }
        if let Some(text) = clip {
            self.set_clipboard(&text, false);
        }
        shown
    }

    /// Turns signals from background tabs into tab markers and notifications.
    fn notify(&mut self, sig: &PaneSignal) {
        let Some(ti) = self.tabs.iter().position(|t| t.root.contains(sig.id)) else { return };
        let title = self.tab_title(ti);
        let long = sig
            .finished
            .filter(|d| self.cfg.terminal.notify_after > 0 && d.as_secs() >= self.cfg.terminal.notify_after);
        let message = if let Some(n) = &sig.notice {
            Some(format!("{title} · {n}"))
        } else if let Some(d) = long {
            Some(format!("{title} · done in {}", crate::util::fmt_duration(d)))
        } else if sig.bell {
            Some(format!("{title} · needs attention"))
        } else {
            None
        };
        let tab = &mut self.tabs[ti];
        if sig.visible {
            // On the visible tab only the app's own notice is shown.
            if let Some(n) = &sig.notice {
                self.toast(ToastLevel::Info, n.clone());
            }
            return;
        }
        tab.activity = true;
        let Some(message) = message else { return };
        tab.alert = true;
        if self.cfg.terminal.notify {
            self.toast(ToastLevel::Info, message);
            self.outer_bell = true;
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

    /// Tunes background work to the visible screen: sensor rate and refreshing
    /// (at most once a minute) the git status of the projects shown on Home.
    fn sync_power_state(&mut self) {
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
            let vis = self.visible_projects();
            let from = self.bridge.proj_sel.saturating_sub(10);
            let paths: Vec<PathBuf> = vis.iter().skip(from).take(25).map(|i| self.projects[*i].path.clone()).collect();
            for p in paths {
                self.request_git(p, Duration::from_secs(60));
            }
        }
        self.last_view = Some(self.view);
    }

    /// Re-requests the git status of the project containing `cwd` (unless too frequent).
    pub fn refresh_git_at(&mut self, cwd: &std::path::Path) {
        if let Some(path) = crate::projects::project_containing(&self.projects, cwd).map(|p| p.path.clone()) {
            self.request_git(path, Duration::from_secs(2));
        }
    }

    fn request_git(&mut self, path: PathBuf, min_gap: Duration) {
        let Some(s) = &self.services else { return };
        if self.git_requested.get(&path).is_some_and(|t| t.elapsed() < min_gap) {
            return;
        }
        if s.rescan.send(ProjectReq::Refresh(path.clone())).is_ok() {
            self.git_requested.insert(path, Instant::now());
        }
    }

    /// Requests git status for projects shown on Home that do not have one yet
    /// (the first scan only fetches the most recently used ones).
    fn request_visible_git(&mut self) {
        if self.view != View::Bridge || self.services.is_none() {
            return;
        }
        let vis = self.visible_projects();
        let from = self.bridge.proj_sel.saturating_sub(20);
        let wanted: Vec<PathBuf> = vis
            .iter()
            .skip(from)
            .take(60)
            .map(|i| &self.projects[*i])
            .filter(|p| p.git.is_none() && !self.git_requested.contains_key(&p.path))
            .take(8)
            .map(|p| p.path.clone())
            .collect();
        for path in wanted {
            // Do not keep re-requesting failed repos (no git etc.).
            self.request_git(path, Duration::MAX);
        }
    }

    pub fn set_clipboard(&mut self, text: &str, announce: bool) {
        match self.clipboard.set(text) {
            Some(via) => {
                if announce {
                    let n = text.chars().count();
                    let how = if via == crate::clipboard::Copied::Terminal { " via the terminal" } else { "" };
                    self.toast(ToastLevel::Ok, format!("copied {n} chars{how}"));
                }
            }
            None => self.toast(ToastLevel::Warn, "clipboard unavailable"),
        }
    }

    /// Pastes the system clipboard into a pane.
    pub fn paste_clipboard(&mut self, pane: PaneId) {
        match self.clipboard.get() {
            Some(text) => self.paste_into(pane, text),
            None => self.toast(ToastLevel::Warn, "clipboard unavailable — paste with your terminal's shortcut"),
        }
    }

    /// Pastes text into a pane. Text with a line break would run as commands in an app without bracketed paste
    /// (cmd, older PowerShell, a plain `sh`): that asks first.
    pub fn paste_into(&mut self, pane: PaneId, text: String) {
        let Some(p) = self.panes.get(&pane) else { return };
        let bracketed = p.parser().screen().bracketed_paste();
        if !bracketed && text.contains(['\n', '\r']) {
            let lines = text.lines().count().max(1);
            let s = if lines == 1 { "" } else { "s" };
            self.overlay = Some(Overlay::Confirm(Confirm {
                title: "PASTE".into(),
                body: format!("{lines} line{s} — each may run as a command. Paste?"),
                action: ConfirmAction::Paste { pane, text },
            }));
            return;
        }
        p.scroll_reset();
        p.paste(&text);
    }

    /// Periodic work: boot animation, notifications, config watching.
    pub fn tick(&mut self) {
        let before = (self.ui_fingerprint(), self.animating());
        self.tick_inner();
        // When the animation ends, the final (static) frame must draw too.
        if (self.ui_fingerprint(), self.animating()) != before {
            self.dirty = true;
        }
    }

    fn tick_inner(&mut self) {
        if self.boot.as_ref().is_some_and(|b| b.started.elapsed() >= BOOT_DURATION) {
            self.boot = None;
        }
        if self.slide.as_ref().is_some_and(|s| s.started.elapsed() >= SLIDE_DURATION) {
            self.slide = None;
        }
        if self.zoom_anim.as_ref().is_some_and(|z| z.started.elapsed() >= ZOOM_DURATION) {
            self.zoom_anim = None;
        }
        let now = Instant::now();
        self.toasts.retain(|t| t.until > now);
        self.request_visible_git();
        self.sync_power_state();
        let hooks_live = self.hooks_installed || !self.agent_hooks.is_empty();
        if self.services.is_some() && hooks_live && self.last_hook_scan.elapsed() >= Duration::from_secs(1) {
            self.last_hook_scan = now;
            let records = crate::hooks::read_records(&self.paths.data, std::process::id());
            self.apply_hook_records(records);
        }
        if let View::Term(i) = self.view
            && let Some(t) = self.tabs.get_mut(i)
        {
            t.activity = false;
            t.alert = false;
        }
        // The search closes when the pane closes or focus moves elsewhere.
        if let Some(s) = &self.search
            && self.focused_pane() != Some(s.pane)
        {
            self.search = None;
        }
        if self.services.is_some() && self.cfg.general.check_updates && self.next_update_check.is_some_and(|t| now >= t)
        {
            self.next_update_check = Some(now + Duration::from_secs(crate::update::CHECK_INTERVAL as u64));
            crate::update::spawn_check(self.tx.clone());
        }
        if self.services.is_some() && self.last_cfg_check.elapsed() >= Duration::from_secs(2) {
            self.last_cfg_check = now;
            let m = config::mtime_of(&self.paths.config);
            if m.is_some() && m != self.cfg_mtime {
                self.cfg_mtime = m;
                self.reload_config(false);
            }
        }
    }

    /// Re-reads the Windows Terminal settings (untouched in headless mode).
    pub fn reload_schemes(&mut self) {
        if self.services.is_some() {
            self.term_schemes = crate::wt::all_schemes(crate::wt::load().as_ref());
        }
    }

    pub fn reload_config(&mut self, announce: bool) {
        self.reload_schemes();
        let loaded = config::load(&self.paths.config);
        self.cfg_mtime = loaded.mtime;
        if let Some(err) = loaded.error {
            self.toast(ToastLevel::Error, err);
            return;
        }
        self.apply_config(loaded.config);
        if announce || self.keymap.warnings.is_empty() {
            self.toast(ToastLevel::Ok, "config reloaded");
        }
        for w in self.keymap.warnings.clone() {
            self.toast(ToastLevel::Warn, w);
        }
    }

    pub fn apply_config(&mut self, cfg: Config) {
        let old = std::mem::replace(&mut self.cfg, cfg);
        self.theme = Theme::by_name(&self.cfg.general.theme, self.cfg.general.transparent);
        self.keymap = Keymap::from_config(&self.cfg.keys);
        self.shell = resolve_shell(&self.cfg.terminal, &self.paths.data);
        self.launchers = launcher_availability(&self.cfg.launchers);
        if !self.cfg.general.operator.trim().is_empty() {
            self.operator = self.cfg.general.operator.trim().to_string();
        }
        if let Some(s) = &self.services {
            if old.projects != self.cfg.projects {
                if let Ok(mut c) = s.proj_cfg.lock() {
                    *c = self.cfg.projects.clone();
                }
                let _ = s.rescan.send(crate::projects::ProjectReq::Rescan);
            }
            if old.ai != self.cfg.ai {
                if let Ok(mut c) = s.ai_cfg.lock() {
                    *c = self.cfg.ai.clone();
                }
                if !self.cfg.ai.enabled {
                    self.ai.clear();
                } else {
                    self.ai.retain(|p| self.cfg.ai.providers.iter().any(|x| x.eq_ignore_ascii_case(p.id)));
                }
                let _ = s.ai_refresh.send(crate::ai::AiReq::Refresh);
            }
        }
    }

    /// Session file: separate so `noble-dev` does not overwrite the stable build's tabs.
    fn session_file(&self) -> &'static str {
        if self.dev { "session-dev.json" } else { "session.json" }
    }

    /// On exit: save the session.
    pub fn shutdown(&mut self) {
        if self.cfg.terminal.restore_session {
            let ws = self.snapshot("last session");
            crate::store::save_session(&self.paths.data_file(self.session_file()), &ws);
        }
        self.panes.clear();
    }

    /// The ongoing drag kind (for hover effects).
    pub fn drag_kind(&self) -> Option<&'static str> {
        self.drag.as_ref().map(|d| match d {
            Drag::Divider { .. } => "divider",
            Drag::Select { .. } => "select",
            Drag::Forward { .. } => "forward",
            Drag::Tab { .. } => "tab",
        })
    }

    pub fn pane_count(&self) -> usize {
        self.panes.len()
    }
}
