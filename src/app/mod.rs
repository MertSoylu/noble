//! Application state and event dispatch. Drawing lives in the `ui` module;
//! everything here mutates state and nothing writes to the screen directly.

mod agents;
pub mod fx;
mod input;
mod menu;
mod ops;
mod palette;
mod search;
mod settings;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
use crate::term::layout::{Divider, PaneId};
use crate::term::pane::{CommandResult, Pane, ShellSpec, resolve_shell};
use crate::term::{Tab, TabAlert};
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
    /// The tab by its stable id (its panes may close before the confirmation).
    CloseTab(crate::term::TabId),
    Paste {
        pane: PaneId,
        text: String,
    },
    Kill {
        pid: u32,
        name: String,
    },
    /// The saved workspace by name (another window may have changed the list meanwhile).
    DeleteWorkspace(String),
}

pub struct Confirm {
    pub title: String,
    pub body: String,
    pub action: ConfirmAction,
}

pub enum PromptPurpose {
    /// The tab by its stable id (an index would go stale when another tab closes, a pane id when
    /// that pane closes in a split tab).
    RenameTab(crate::term::TabId),
    SaveWorkspace,
    /// New root folder to scan projects in.
    AddRoot,
    /// One folder added as a project by hand.
    AddProject,
}

impl PromptPurpose {
    /// Maximum input length (paths can be long).
    pub fn max_len(&self) -> usize {
        match self {
            PromptPurpose::AddRoot | PromptPurpose::AddProject => 260,
            _ => 40,
        }
    }

    /// The add prompt (either mode): Tab switches between adding a project and a folder to scan.
    pub fn is_add(&self) -> bool {
        matches!(self, PromptPurpose::AddRoot | PromptPurpose::AddProject)
    }
}

pub struct Prompt {
    pub title: String,
    pub value: String,
    pub purpose: PromptPurpose,
    /// Cursor position as a character index into `value` (0..=len).
    pub cursor: usize,
    /// Validation error of the last submit, shown inside the prompt.
    pub error: Option<String>,
}

impl Prompt {
    /// A prompt with the cursor at the end of `value`.
    pub fn new(title: impl Into<String>, value: String, purpose: PromptPurpose) -> Self {
        let cursor = value.chars().count();
        Prompt { title: title.into(), value, purpose, cursor, error: None }
    }

    fn byte_at(&self, chars: usize) -> usize {
        self.value.char_indices().nth(chars).map_or(self.value.len(), |(i, _)| i)
    }

    /// Inserts text at the cursor, respecting the length limit; clears the error.
    pub fn insert(&mut self, text: &str) {
        let room = self.purpose.max_len().saturating_sub(self.value.chars().count());
        let text: String = text.chars().take(room).collect();
        if text.is_empty() {
            return;
        }
        let at = self.byte_at(self.cursor);
        self.value.insert_str(at, &text);
        self.cursor += text.chars().count();
        self.error = None;
    }

    /// Deletes the character before the cursor.
    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let at = self.byte_at(self.cursor - 1);
            self.value.remove(at);
            self.cursor -= 1;
            self.error = None;
        }
    }

    /// Deletes the character under the cursor.
    pub fn delete(&mut self) {
        if self.cursor < self.value.chars().count() {
            let at = self.byte_at(self.cursor);
            self.value.remove(at);
            self.error = None;
        }
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
        self.error = None;
    }

    pub fn move_cursor(&mut self, to: usize) {
        self.cursor = to.min(self.value.chars().count());
    }

    /// Switches the add prompt between one project (`project`) and a folder to scan; the
    /// typed path stays (Tab or a click on the choice line).
    pub fn set_add_mode(&mut self, project: bool) {
        let (title, purpose) = if project {
            ("ADD PROJECT", PromptPurpose::AddProject)
        } else {
            ("ADD FOLDER TO SCAN", PromptPurpose::AddRoot)
        };
        self.title = title.into();
        self.purpose = purpose;
    }
}

/// Terminal color scheme selector: previews while navigating, esc reverts.
pub struct SchemePicker {
    pub selected: usize,
    pub original: String,
}

/// Theme selector: the whole UI previews the theme under the cursor, esc reverts.
pub struct ThemePicker {
    pub selected: usize,
    /// The theme in use when the selector opened (`THEMES` name).
    pub original: String,
}

/// A row of the first launch setup card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WelcomeRow {
    Theme,
    Colors,
    Shell,
    Prefix,
}

/// First launch setup: theme and terminal colors preview live, the rest applies on ⏎; esc
/// undoes the previews and keeps the defaults.
pub struct WelcomeSetup {
    /// Selected row (`WelcomeSetup::rows` order).
    pub row: usize,
    /// Indexes into `THEMES`, `schemes`, `shells` and `PREFIXES`.
    pub theme: usize,
    pub colors: usize,
    pub shell: usize,
    pub prefix: usize,
    /// `App::scheme_options` and `settings::shell_options` when the card opened.
    pub schemes: Vec<String>,
    pub shells: Vec<String>,
    /// Theme and terminal colors before the card opened (restored by esc).
    pub original_theme: String,
    pub original_colors: String,
    /// `colors` when the card opened: only a changed choice is written to the config.
    pub colors_at_open: usize,
    /// `prefix` when the card opened: a custom prefix (not in `PREFIXES`) is only replaced when
    /// another one is picked.
    pub prefix_at_open: usize,
}

impl WelcomeSetup {
    /// The rows shown: terminal colors and shell only when there is something to choose.
    pub fn rows(&self) -> Vec<WelcomeRow> {
        let mut rows = vec![WelcomeRow::Theme];
        if self.schemes.len() > 1 {
            rows.push(WelcomeRow::Colors);
        }
        // `shells[0]` is "auto": a choice needs at least two installed shells.
        if self.shells.len() > 2 {
            rows.push(WelcomeRow::Shell);
        }
        rows.push(WelcomeRow::Prefix);
        rows
    }
}

pub enum Overlay {
    /// First launch: short setup (theme, terminal colors, shell, prefix key) and the main keys.
    Welcome(WelcomeSetup),
    Palette(PaletteState),
    Schemes(SchemePicker),
    Themes(ThemePicker),
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
    pub born: Instant,
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
    /// "commands" chip at the right end of the status bar: opens the command palette.
    Palette,
    Pane {
        pane: PaneId,
        inner: Rect,
    },
    PaneZoom(PaneId),
    PaneClose(PaneId),
    /// "↓ live" chip of a pane scrolled back: back to the newest output.
    ScrollLive(PaneId),
    /// Position bar of a pane scrolled back: a click jumps to that part of the scrollback.
    ScrollTrack {
        pane: PaneId,
        track: Rect,
    },
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
    /// Row in the theme selector (`THEMES` order).
    ThemeOption(usize),
    /// Quick launch popup: show/hide and shortcut (`launchers` order).
    LaunchShow(usize),
    LaunchKey(usize),
    OpenFiles,
    Proc(u32),
    SortCol(SortKey),
    PaletteItem(usize),
    ConfirmYes,
    ConfirmNo,
    /// Welcome card: a setup row (`WelcomeSetup::rows` order), its ‹ › arrows and a prefix chip.
    WelcomeRow(usize),
    WelcomeStep(usize, i32),
    WelcomePrefix(usize),
    WelcomeDone,
    MenuItem(usize),
    /// Top row of the pane frame (for the right-click menu).
    PaneTitle(PaneId),
    /// Quick-action button on a project row (row, action).
    ProjectAct(usize, ProjectAct),
    /// Add prompt: switch to adding one project (`true`) or a folder to scan (`false`).
    PromptMode(bool),
    /// Update notice at the bottom right: update / dismiss.
    Update,
    UpdateDismiss,
    /// One-time offer to enable the Claude status hooks: enable / dismiss.
    HooksOffer,
    HooksOfferDismiss,
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
    /// Scatters the boot particles differently on every launch.
    pub seed: u64,
}

impl Boot {
    pub fn now() -> Boot {
        let seed =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
        Boot { started: Instant::now(), seed }
    }
}

pub const BOOT_DURATION: Duration = Duration::from_millis(3000);
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

    /// How much the state asks for attention: the tab strip's dot shows the most urgent
    /// state among a tab's panes.
    pub fn urgency(&self) -> u8 {
        match self {
            AgentState::NeedsYou => 3,
            AgentState::Idle => 2,
            AgentState::Working => 1,
            AgentState::Running => 0,
        }
    }
}

/// What a pane's title shows about the agent running in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentBadge {
    pub kind: &'static str,
    pub state: AgentState,
    /// Since when it has been working (Unix seconds); only for a Working hook session.
    pub working_since: Option<i64>,
    /// Subagents of the session still running.
    pub subagents: usize,
}

/// The pane "jump to waiting agent" goes to: in `order` (every pane, in tab order, with its
/// agent's state), the first one after `current` (cyclic, `current` itself last) that needs
/// you, else the first whose agent finished its answer. `None`: no agent is waiting.
pub fn next_waiting_agent(order: &[(PaneId, Option<AgentState>)], current: Option<PaneId>) -> Option<PaneId> {
    let n = order.len();
    let start = current.and_then(|c| order.iter().position(|(p, _)| *p == c)).map_or(0, |i| i + 1);
    [AgentState::NeedsYou, AgentState::Idle]
        .into_iter()
        .find_map(|want| (0..n).map(|k| &order[(start + k) % n]).find(|(_, s)| *s == Some(want)).map(|(p, _)| *p))
}

/// Open AI sessions per project path: (agent, most urgent state).
pub type AgentsByProject = std::collections::HashMap<std::path::PathBuf, Vec<(&'static str, AgentState)>>;

/// A row of the session list.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentSession {
    pub tab: usize,
    pub pane: PaneId,
    pub kind: &'static str,
    pub place: String,
    pub state: AgentState,
}

/// How long after its pane's last output a working agent still counts as live (`App::agent_live`).
pub const AGENT_LIVE: Duration = Duration::from_secs(3);

/// Signals collected from a pane in a single frame.
struct PaneSignal {
    id: PaneId,
    visible: bool,
    bell: bool,
    notice: Option<String>,
    /// The shell marked a command start (`OSC 133;C`) at this moment.
    started: Option<Instant>,
    /// If the prompt returned and the user started a command, that command's duration.
    finished: Option<Duration>,
    /// The pane's directory when its prompt came back.
    cwd: Option<PathBuf>,
    /// The exit code reported with the prompt (OSC 133;D).
    exit: Option<i32>,
    /// The command typed at the prompt ended with a non-zero exit code (never for a launcher command
    /// or the shell's first prompt; set by `on_pty_output`).
    failed: bool,
}

/// Channels to the background services (absent in headless tests).
pub struct Services {
    pub sensor_req: Sender<SensorRequest>,
    pub rescan: Sender<ProjectReq>,
    pub ai_refresh: Sender<crate::ai::AiReq>,
    pub proj_cfg: Arc<Mutex<config::ProjectsCfg>>,
    /// Hidden and manually added projects for the scan thread (kept in sync with `ui_state`).
    pub proj_manual: Arc<Mutex<crate::projects::Manual>>,
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
    /// The "zsh integration is blocked by the system zshenv" warning was shown (once per run).
    zsh_warned: bool,
    /// A headless app's own data folder under the temp dir, deleted when the app is dropped.
    owned_data: Option<PathBuf>,
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
    /// First line of the Settings page on screen; `settings_follow` scrolls it to the selection.
    pub settings_scroll: usize,
    pub settings_follow: bool,
    /// The page Settings was opened from (and its tab's focused pane), for esc.
    pub settings_back: Option<(View, Option<crate::term::layout::PaneId>)>,
    pub overlay: Option<Overlay>,
    pub toasts: Vec<Toast>,
    pub boot: Option<Boot>,
    pub hits: Vec<(Rect, Hit)>,
    /// Mouse cursor position (for hover effects).
    pub hover: Option<(u16, u16)>,
    pub slide: Option<Slide>,
    pub zoom_anim: Option<ZoomAnim>,
    /// Short UI effects (fades, flashes, glides, tweens); see `fx`.
    pub fx: fx::Effects,
    /// Last drawn page and body image (to start a transition).
    pub drawn_view: Option<View>,
    pub last_body: Option<ratatui::buffer::Buffer>,
    pub(crate) drag: Option<Drag>,
    last_click: Option<(Instant, u16, u16)>,
    /// Consecutive quick clicks on the same cell: 1 single, 2 double (word), 3 triple (line).
    click_streak: u8,
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
    /// This window's id in the shared session file (`store::instance_id`).
    pub instance: String,
    /// Whether a restored pane's directory can be used; runs on a helper thread with a timeout
    /// (`ops::DIR_PROBE_TIMEOUT`). Replaceable so tests can simulate a hanging network share.
    #[doc(hidden)]
    pub dir_probe: fn(&Path) -> bool,
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
    /// When each hook-tracked agent started its current stretch of work (Unix seconds): the
    /// record's time when it turned Working, kept while it stays Working (a `stop` with
    /// subagents still running does not restart it). Shown as "working 2m" in the pane title.
    agent_working_since: HashMap<PaneId, i64>,
    last_hook_scan: Instant,
    /// Whether the NOBLE hooks are installed in `~/.claude/settings.json` (shown in Settings).
    pub hooks_installed: bool,
    /// The one-time "show Claude status in NOBLE?" notice is up (`note_agent_offer`).
    pub hooks_offer: bool,
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
    /// Text of the notification to forward to the outer terminal (OSC 9 / 777, see `outer`); taken by the frame loop.
    pub outer_notice: Option<String>,
    /// Whether the NOBLE window has focus, as far as the outer terminal reports it (`None`: no report yet).
    pub window_focused: Option<bool>,
    /// A newer published release (announced bottom right).
    pub update_available: Option<String>,
    /// Next update check (`None`: no checking, e.g. headless mode, `noble-dev`).
    next_update_check: Option<Instant>,
    /// Periodic session save (`autosave_session`). Off in headless mode and tests unless they switch it on.
    pub session_autosave: bool,
    /// The tabs as last written to the session file (or restored from it).
    session_saved: Vec<crate::store::SavedTab>,
    /// Since when the tabs differ from `session_saved`; the save follows `SESSION_SAVE_DELAY` later.
    session_dirty_since: Option<Instant>,
    /// When the tabs were last compared with `session_saved`.
    session_checked: Instant,
}

/// How long the tabs must have differed from the saved session before it is written again (debounce).
const SESSION_SAVE_DELAY: Duration = Duration::from_secs(2);
/// How often the tabs are compared with the saved session.
const SESSION_CHECK_EVERY: Duration = Duration::from_secs(1);
/// How long the periodic save waits for another window's session lock (the main thread must not stall).
const SESSION_AUTOSAVE_LOCK_WAIT: Duration = Duration::from_secs(1);

fn launcher_availability(list: &[Launcher]) -> Vec<(Launcher, bool)> {
    list.iter()
        .map(|l| {
            let program = l.command.split_whitespace().next().unwrap_or("");
            (l.clone(), crate::util::which(program).is_some())
        })
        .collect()
}

impl Drop for App {
    /// A headless app removes its temp data folder (and the shared `noble-headless` parent once it is
    /// empty). The panes go first: their shells may still hold the integration scripts open (Windows).
    fn drop(&mut self) {
        let Some(dir) = self.owned_data.take() else { return };
        self.panes.clear();
        let _ = std::fs::remove_dir_all(&dir);
        if let Some(parent) = dir.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
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
        // A folder of its own per app (tests run in parallel), removed again on drop: tests and the
        // screenshot example would otherwise leave shell scripts and records in the temp dir on every run.
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let data = std::env::temp_dir().join("noble-headless").join(format!("{}-{seq}", std::process::id()));
        let paths = Paths { config: PathBuf::from("config.toml"), data: data.clone() };
        let mut app = App::build(cfg, paths, None, tx, size, Recent::memory(), Workspaces::memory());
        app.boot = None;
        app.rx = Some(rx);
        app.owned_data = Some(data);
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
        // Read before the first scan so hidden/added projects apply to it.
        let ui_state = crate::store::UiState::load(paths.data_file("state.json"));
        let proj_manual = Arc::new(Mutex::new(ui_state.manual_projects()));
        let (rescan_tx, rescan_rx) = std::sync::mpsc::channel();
        crate::projects::spawn(proj_cfg.clone(), proj_manual.clone(), tx.clone(), rescan_rx);
        let ai_cfg = Arc::new(Mutex::new(cfg.ai.clone()));
        let (ai_tx, ai_rx) = std::sync::mpsc::channel();
        ai::spawn(ai_cfg.clone(), paths.data_file("ai-cache.json"), tx.clone(), ai_rx);

        let services =
            Services { sensor_req: sensor_tx, rescan: rescan_tx, ai_refresh: ai_tx, proj_cfg, proj_manual, ai_cfg };
        let cache = ai::load_cache(&paths.data_file("ai-cache.json"));
        let mut app = App::build(cfg, paths, Some(services), tx, size, recent, workspaces);
        app.ai_installed = ai::installed_providers();
        app.dev = crate::util::is_dev_build();
        app.usage_history = UsageHistory::load(app.paths.data_file("ai-history.json"));
        app.reload_schemes();
        app.ui_state = ui_state;
        crate::hooks::prune(&app.paths.data, std::process::id());
        app.hooks_installed = crate::hooks::settings_path().is_some_and(|p| {
            // Hooks from an older NOBLE get the events added since (e.g. the subagent ones).
            let _ = crate::hooks::upgrade(&p);
            crate::hooks::is_installed(&p)
        });
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
        app.restore_last_session();
        app.session_autosave = true;
        app
    }

    /// Startup: registers this window in the session file (`session.json`, `session-dev.json`
    /// for `noble-dev`) and, when it is the first window of the run, reopens the saved tabs of
    /// every window. Only with `restore_session` on.
    pub fn restore_last_session(&mut self) {
        if !self.cfg.terminal.restore_session {
            return;
        }
        let file = self.paths.data_file(self.session_file());
        let Some(tabs) = crate::store::session_begin(&file, &self.instance) else { return };
        if tabs.is_empty() {
            return;
        }
        let ws = crate::store::Workspace { name: "last session".into(), saved_at: 0, tabs };
        self.restored_tabs = self.open_workspace(&ws);
        self.view = View::Bridge;
        // What actually opened (directories that fell back to home included) replaces the
        // taken-over tabs right away, so a crash keeps them as they are now.
        let tabs = self.snapshot("last session").tabs;
        crate::store::session_save(&file, &self.instance, tabs.clone(), false);
        self.session_saved = tabs;
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
        let boot = cfg.general.boot_animation.then(Boot::now);
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
            zsh_warned: false,
            owned_data: None,
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
            settings_scroll: 0,
            settings_follow: true,
            settings_back: None,
            overlay: None,
            toasts: Vec::new(),
            boot,
            hits: Vec::new(),
            hover: None,
            slide: None,
            zoom_anim: None,
            fx: fx::Effects::default(),
            drawn_view: None,
            last_body: None,
            drag: None,
            last_click: None,
            click_streak: 0,
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
            instance: crate::store::instance_id(),
            dir_probe: |p| p.is_dir(),
            operator,
            git_requested: HashMap::new(),
            search: None,
            link_hover: None,
            usage_history: UsageHistory::memory(),
            ui_state: crate::store::UiState::memory(),
            agent_hooks: HashMap::new(),
            agent_cleared: HashMap::new(),
            agent_working_since: HashMap::new(),
            last_hook_scan: Instant::now(),
            hooks_installed: false,
            hooks_offer: false,
            sensor_mode: None,
            last_view: None,
            sensor_on_battery: false,
            toast_serial: 0,
            dirty: false,
            term_schemes: crate::wt::all_schemes(None),
            battery_warned: 100,
            quota_warned: Default::default(),
            outer_bell: false,
            outer_notice: None,
            window_focused: None,
            update_available: None,
            next_update_check: None,
            session_autosave: false,
            session_saved: Vec::new(),
            session_dirty_since: None,
            session_checked: Instant::now(),
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
        let born = Instant::now();
        self.toasts.push(Toast { text, level, born, until: born + Duration::from_secs(secs) });
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
        self.boot.is_some() || self.slide.is_some() || self.zoom_anim.is_some() || self.fx.busy() || self.toast_moving()
    }

    /// Whether animations play now: the setting, and on battery the "animations on battery" one.
    pub fn anim_on(&self) -> bool {
        let g = &self.cfg.general;
        g.animations && (g.animations_on_battery || !self.on_battery())
    }

    /// A notification sliding in or out.
    fn toast_moving(&self) -> bool {
        self.anim_on()
            && self.toasts.iter().any(|t| t.born.elapsed() < fx::TOAST_IN + fx::GRACE || Instant::now() >= t.until)
    }

    /// How long notifications stay on screen after their time is up (they slide out meanwhile).
    pub fn toast_linger(&self) -> Duration {
        if self.anim_on() { fx::TOAST_OUT } else { Duration::ZERO }
    }

    /// Handles the event; `true` when something visible changed (redraw).
    /// Changes in invisible data (e.g. sensors while in a terminal) never wake
    /// the screen: battery friendly.
    pub fn handle(&mut self, ev: AppEvent) -> bool {
        let shown = match &ev {
            AppEvent::Input(_)
            | AppEvent::PtyExit(_)
            | AppEvent::KillResult { .. }
            | AppEvent::LaunchFailed(_)
            | AppEvent::Update(_)
            | AppEvent::Quit => true,
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
    fn ui_fingerprint(&self) -> (u64, usize, Vec<(bool, Option<TabAlert>)>) {
        (self.toast_serial, self.toasts.len(), self.tabs.iter().map(|t| (t.activity, t.alert)).collect())
    }

    /// The event's state change. `true`: output arrived in a visible pane.
    fn apply(&mut self, ev: AppEvent) -> bool {
        match ev {
            AppEvent::Input(e) => self.on_input(e),
            AppEvent::Quit => self.quit = true,
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
            AppEvent::LaunchFailed(text) => self.toast(ToastLevel::Error, text),
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
                let checked = chrono::Utc::now().timestamp();
                self.edit_ui_state(|d| {
                    d.update_checked = checked;
                    d.update_latest = version.clone();
                });
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

    /// Providers on Home's quota panel: signed in, and only while the panel is enabled.
    pub fn quota_providers(&self) -> impl Iterator<Item = &crate::ai::ProviderState> {
        let enabled = self.cfg.ai.enabled;
        self.ai.iter().filter(move |p| {
            enabled && p.presence == crate::ai::Presence::Ready && p.status != crate::ai::Status::SignIn
        })
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
        if let Some(t) = self.toasts.iter().map(|t| t.until).min() {
            want(t.saturating_duration_since(Instant::now()) + Duration::from_millis(5));
        }
        // The countdown line under a notification moves one cell at a time.
        if self.anim_on()
            && let Some(step) = self.toasts.iter().map(|t| (t.until - t.born) / 48).min()
        {
            want(step.max(Duration::from_millis(40)));
        }
        if matches!(self.overlay, Some(Overlay::Palette(_) | Overlay::Prompt(_))) {
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
            self.edit_ui_state(|d| d.update_skipped = v);
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

    fn set_projects(&mut self, mut list: Vec<Project>) {
        // The scan thread already drops hidden projects; one hidden since it started is dropped here.
        let manual = self.ui_state.manual_projects();
        manual.filter(&mut list);
        // A folder added by hand after the scan started is missing from its result: keep the
        // entry `add_project` listed (while the folder exists) until a later scan includes it.
        let missing: Vec<Project> = self
            .projects
            .iter()
            .filter(|p| self.ui_state.is_added(&p.path) && !manual.is_hidden(&p.path))
            .filter(|p| !list.iter().any(|q| crate::util::same_path(&q.path, &p.path)) && p.path.is_dir())
            .cloned()
            .collect();
        if !missing.is_empty() {
            list.extend(missing);
            crate::projects::sort_projects(&mut list);
        }
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
                let started = pane.take_started();
                let prompt = pane.take_prompt();
                signals.push(PaneSignal {
                    id: *id,
                    visible: visible.contains(id),
                    bell: pane.take_bell(),
                    notice: pane.take_notice(),
                    started,
                    finished: prompt.and_then(|_| started.or(pane.command_started).map(|t| t.elapsed())),
                    cwd: prompt.map(|_| pane.cwd()),
                    exit: prompt.flatten(),
                    failed: false,
                });
            }
        }
        let mut shown = signals.iter().any(|s| s.visible);
        for mut sig in signals {
            if let Some(p) = self.panes.get_mut(&sig.id) {
                p.last_output = Some(Instant::now());
                // The shell marks command starts: from now on only that mark starts the clock.
                if let Some(t) = sig.started {
                    p.start_marks = true;
                    p.ran = true;
                    p.command_started = Some(t);
                }
            }
            if let Some(cwd) = &sig.cwd {
                // Command finished: the repo may have changed.
                self.refresh_git_at(cwd);
                // An agent there has exited; the tab title in the top bar may change too.
                shown |= self.agent_state(sig.id).is_some();
                self.clear_agent(sig.id);
                if let Some(p) = self.panes.get_mut(&sig.id) {
                    // Only a command typed at a prompt counts (`Pane::command_ran`): the shell's first
                    // prompt (after it starts, or after a launcher command, whose code the shell does
                    // not know) and an Enter on an empty line leave the last result in place.
                    if let Some(took) = sig.finished.filter(|_| p.command_ran()) {
                        p.last_result = Some(CommandResult { code: sig.exit, took, at: Instant::now() });
                        sig.failed = sig.exit.is_some_and(|code| code != 0);
                    }
                    p.command_started = None;
                    p.edited = false;
                    p.ran = false;
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
        let took = long.map(crate::util::fmt_duration);
        let message = if let Some(n) = &sig.notice {
            Some(format!("{title} · {n}"))
        } else if let Some(took) = took {
            Some(match sig.exit.filter(|_| sig.failed) {
                Some(code) => format!("{title} · failed (exit {code}) after {took}"),
                None => format!("{title} · done in {took}"),
            })
        } else if sig.bell {
            Some(format!("{title} · needs attention"))
        } else {
            None
        };
        // The marker on the tab strip: the most urgent thing that happened. A short command that
        // fails leaves a marker only (no toast, no bell): failures of quick commands are routine.
        let kind = [
            (sig.notice.is_some() || sig.bell, TabAlert::Notice),
            (sig.failed, TabAlert::Failed),
            (long.is_some(), TabAlert::Done),
        ]
        .into_iter()
        .filter_map(|(on, kind)| on.then_some(kind))
        .max();
        let tab = &mut self.tabs[ti];
        if sig.visible {
            // On the visible tab only the app's own notice is shown.
            if let Some(n) = &sig.notice {
                self.toast(ToastLevel::Info, n.clone());
            }
            return;
        }
        tab.activity = true;
        if let Some(kind) = kind {
            tab.raise(kind);
        }
        let Some(message) = message else { return };
        if self.cfg.terminal.notify {
            self.outer_notice = Some(message.clone());
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
            .filter(|p| p.repo && p.git.is_none() && !self.git_requested.contains_key(&p.path))
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
        let Some(p) = self.panes.get_mut(&pane) else { return };
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
        let linger = self.toast_linger();
        self.toasts.retain(|t| t.until + linger > now);
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
            t.alert = None;
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
        self.autosave_session();
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
        // An open welcome card or theme/scheme selector previews on top of the old values: it starts
        // over from the file, so esc cannot put the stale values back.
        self.refresh_welcome();
        match self.overlay {
            Some(Overlay::Themes(_)) => self.open_theme_picker(),
            Some(Overlay::Schemes(_)) => self.open_scheme_picker(),
            _ => {}
        }
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

    /// Keeps the session file current while NOBLE runs, so a crash, a kill or a power loss loses
    /// at most a few seconds of tabs, splits and directories. The tabs are compared with what was
    /// last saved at most once a second (`tick` runs on every event and at least every 2 s, so an
    /// idle NOBLE gains no wake-ups); a difference that lasts `SESSION_SAVE_DELAY` is written.
    fn autosave_session(&mut self) {
        if !self.session_autosave || !self.cfg.terminal.restore_session {
            return;
        }
        if self.session_checked.elapsed() < SESSION_CHECK_EVERY {
            return;
        }
        self.session_checked = Instant::now();
        let tabs = self.snapshot("last session").tabs;
        if tabs == self.session_saved {
            self.session_dirty_since = None;
            return;
        }
        let since = *self.session_dirty_since.get_or_insert_with(Instant::now);
        if since.elapsed() >= SESSION_SAVE_DELAY {
            let file = self.paths.data_file(self.session_file());
            crate::store::session_save_waiting(&file, &self.instance, tabs.clone(), false, SESSION_AUTOSAVE_LOCK_WAIT);
            self.session_saved = tabs;
            self.session_dirty_since = None;
        }
    }

    /// When the main loop must wake up for the session save: only while unsaved changes wait.
    pub fn session_save_due(&self) -> Option<Duration> {
        let since = self.session_dirty_since?;
        let due = (since + SESSION_SAVE_DELAY).max(self.session_checked + SESSION_CHECK_EVERY);
        Some(due.saturating_duration_since(Instant::now()))
    }

    /// Session file: separate so `noble-dev` does not overwrite the stable build's tabs.
    fn session_file(&self) -> &'static str {
        if self.dev { "session-dev.json" } else { "session.json" }
    }

    /// On exit: save the session.
    pub fn shutdown(&mut self) {
        if self.cfg.terminal.restore_session {
            // Merged with the other windows' tabs; this window's earlier entries are replaced.
            let tabs = self.snapshot("last session").tabs;
            crate::store::session_save(&self.paths.data_file(self.session_file()), &self.instance, tabs, true);
        }
        self.panes.clear();
        // Windows: lets the console-close handler return (the process ends then); on Unix nothing waits for it.
        crate::termination::finished();
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

#[cfg(test)]
mod tests {
    use super::*;

    /// "Jump to waiting agent": a pane that needs you wins over one whose agent finished, the
    /// search starts after the focused pane and wraps around (the focused pane comes last), and
    /// working or unknown agents are never a target.
    #[test]
    fn jump_order_prefers_needs_you_then_idle() {
        use AgentState::*;
        let order =
            [(1, None), (2, Some(Idle)), (3, Some(Working)), (4, Some(NeedsYou)), (5, Some(Running)), (6, Some(Idle))];
        // From Home (no focused pane) the search starts at the first pane.
        assert_eq!(next_waiting_agent(&order, None), Some(4));
        assert_eq!(next_waiting_agent(&order, Some(1)), Some(4));
        // Already on the only pane that needs you: it stays there.
        assert_eq!(next_waiting_agent(&order, Some(4)), Some(4));
        // Two panes need you: they alternate, wrapping past the end.
        let two = [(1, Some(NeedsYou)), (2, Some(Idle)), (3, Some(NeedsYou))];
        assert_eq!(next_waiting_agent(&two, Some(1)), Some(3));
        assert_eq!(next_waiting_agent(&two, Some(3)), Some(1));
        // Nobody needs you: the next finished one after the focused pane, cyclic.
        let idle = [(1, Some(Idle)), (2, None), (3, Some(Idle)), (4, Some(Working))];
        assert_eq!(next_waiting_agent(&idle, Some(1)), Some(3));
        assert_eq!(next_waiting_agent(&idle, Some(3)), Some(1));
        assert_eq!(next_waiting_agent(&idle, Some(4)), Some(1));
        // A focused pane that is gone counts as none.
        assert_eq!(next_waiting_agent(&idle, Some(99)), Some(1));
        // Nothing waiting.
        assert_eq!(next_waiting_agent(&[(1, Some(Working)), (2, Some(Running)), (3, None)], Some(1)), None);
        assert_eq!(next_waiting_agent(&[], None), None);
    }
}
