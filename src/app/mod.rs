//! Application state and event dispatch. Drawing lives in the `ui` module;
//! everything here mutates state and nothing writes to the screen directly.

mod agents;
pub mod bindings;
pub mod fx;
mod git;
pub mod help;
mod input;
mod live;
mod menu;
mod notify;
mod ops;
mod palette;
mod power;
mod redraw;
mod reload;
mod search;
mod session;
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
use crate::sensors::{self, KillOutcome, SensorMode, SensorRequest, Sensors};
use crate::store::{Recent, SavedTab, UsageHistory, Workspaces};
use crate::term::layout::{Divider, PaneId};
use crate::term::pane::{CommandResult, Pane, ShellSpec, resolve_shell};
use crate::term::{Tab, TabAlert};
use crate::theme::Theme;

pub use bindings::BindingsState;
pub use help::HelpState;
pub use live::PrEntry;
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
    /// Terminate the process listed as `pid` with start time `started` (checked again before the kill).
    Kill {
        pid: u32,
        started: u64,
        name: String,
    },
    /// The saved workspace by name (another window may have changed the list meanwhile).
    DeleteWorkspace(String),
    /// Save the open tabs under this name over the workspace that has it.
    ReplaceWorkspace(String),
    /// Remove the project at this path from the list.
    RemoveProject(std::path::PathBuf),
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
    /// Adds a project or a folder to scan (see `AddMode`).
    Add(AddMode),
}

/// What an add prompt does with the typed path. `Auto` decides on Enter: a git repository, or a
/// folder with none inside (added as a project), or a folder holding repositories (scanned).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddMode {
    Auto,
    /// A folder whose git repositories are scanned for projects.
    Folder,
    /// One folder added as a project, repository or not.
    Project,
}

impl AddMode {
    /// The next mode in the order Tab cycles through: auto, folder to scan, one project.
    pub fn next(self) -> Self {
        match self {
            AddMode::Auto => AddMode::Folder,
            AddMode::Folder => AddMode::Project,
            AddMode::Project => AddMode::Auto,
        }
    }
}

impl PromptPurpose {
    /// Maximum input length (paths can be long).
    pub fn max_len(&self) -> usize {
        match self {
            PromptPurpose::Add(_) => 260,
            _ => 40,
        }
    }

    /// The mode of an add prompt; `None` for the other prompts.
    pub fn add_mode(&self) -> Option<AddMode> {
        match self {
            PromptPurpose::Add(mode) => Some(*mode),
            _ => None,
        }
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

    /// Sets the add prompt's mode (title and purpose); the typed path stays. Tab cycles the
    /// modes and a click on the choice line picks one.
    pub fn set_add_mode(&mut self, mode: AddMode) {
        self.title = match mode {
            AddMode::Auto => "ADD PROJECT OR FOLDER",
            AddMode::Folder => "ADD FOLDER TO SCAN",
            AddMode::Project => "ADD PROJECT",
        }
        .into();
        self.purpose = PromptPurpose::Add(mode);
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
    /// The keyboard reference (`?`): its filter, selection and scroll.
    Help(HelpState),
    /// The key bindings editor (Settings → Key bindings).
    Bindings(BindingsState),
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

/// Where a notification leads when it is clicked (besides being dismissed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToastTarget {
    /// A config error: opens the config file at this line (the Edit Config File action without one).
    Config { line: Option<u32> },
}

pub struct Toast {
    /// Stable for the toast's life (the index in `App::toasts` shifts when another one leaves).
    pub id: u64,
    pub text: String,
    pub level: ToastLevel,
    pub born: Instant,
    /// How long it stays on screen; `None` until it is dismissed (errors).
    pub life: Option<Duration>,
    /// Time the pointer has rested on it so far: not counted against `life`.
    pub held: Duration,
    /// Since when the pointer rests on it (the countdown is paused meanwhile).
    pub hover_from: Option<Instant>,
    pub target: Option<ToastTarget>,
}

impl Toast {
    /// Time on screen that counts against `life` (the pointer on the toast does not).
    pub fn active(&self, now: Instant) -> Duration {
        let end = self.hover_from.unwrap_or(now).min(now);
        end.saturating_duration_since(self.born).saturating_sub(self.held)
    }

    /// When the toast leaves on its own; `None` while it is paused or sticky.
    pub fn expires_at(&self) -> Option<Instant> {
        if self.hover_from.is_some() {
            return None;
        }
        Some(self.born + self.held + self.life?)
    }

    /// The pointer rests on the toast (`on`) or has left it: the countdown pauses meanwhile.
    fn set_hovered(&mut self, on: bool, now: Instant) {
        match (on, self.hover_from) {
            (true, None) => self.hover_from = Some(now),
            (false, Some(from)) => {
                self.held += now.saturating_duration_since(from);
                self.hover_from = None;
            }
            _ => {}
        }
    }
}

/// A notification kept in the history (the `notifications` action): its full text and when it came.
#[derive(Clone, Debug)]
pub struct Notice {
    pub text: String,
    pub level: ToastLevel,
    pub at: chrono::DateTime<chrono::Local>,
}

/// How many notifications the history keeps (newest last).
pub const NOTICE_HISTORY: usize = 50;

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
    /// A quick task chip on the selected project's card (`Project::tasks` index).
    ProjectTask(usize),
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
    /// Row of the keyboard reference (selectable rows, `help::HelpState::selected` order).
    HelpRow(usize),
    /// Row of the key bindings editor (`App::binding_rows` order).
    BindingsRow(usize),
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
    /// Add prompt: switch to this mode (the choice line).
    PromptMode(AddMode),
    /// Update notice at the bottom right: update / dismiss.
    Update,
    UpdateDismiss,
    /// One-time offer to enable the Claude status hooks: enable / dismiss.
    HooksOffer,
    HooksOfferDismiss,
    /// "+N" chip of the tab bar: opens the command palette listing the hidden tabs.
    HiddenTabs,
    /// System: "/ Filter" button of the Processes panel (as the "/" key).
    SystemFilter,
    /// System: "K Terminate" button of the Processes panel (as the "K" key, asks first).
    SystemTerminate,
    /// Home: "/ search" label at the right end of the Projects panel (as the "/" key).
    HomeSearch,
    /// Home's project card: the selected project's pull request and CI (opens the PR's page).
    ProjectPr,
    /// A listening port on Home's project card (opens `http://localhost:<port>`).
    Port(u16),
    /// A notification (by its `Toast::id`): a click dismisses it, and runs its `ToastTarget`.
    Toast(u64),
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
    /// The scrollback text the matches were computed on; reused while the pane has no new output.
    pub text: Option<crate::term::pane::ScrollText>,
    /// More matches than `MAX_MATCHES`: only the newest are kept (the bar shows "10000+").
    pub capped: bool,
    /// Output or a resize changed the text since the matches were computed (see `App::search_output`).
    pub stale: bool,
    /// When the matches were last computed (throttles the refresh while output streams).
    pub refreshed: Option<Instant>,
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

/// Does the startup animation play on this launch? With `general.boot_animation` on it plays on the
/// first launch and on the first one after the version changed: `last_version` is the version that
/// last played it (`UiStateData::boot_version`). Off, it never plays.
pub fn boot_due(enabled: bool, last_version: &str, version: &str) -> bool {
    enabled && last_version != version
}

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
    /// Project folders whose listening ports are scanned (answered with `AppEvent::Ports`).
    pub ports_req: Sender<Vec<PathBuf>>,
    /// A project folder whose branch's pull request is looked up (answered with `AppEvent::Pr`).
    pub pr_req: Sender<PathBuf>,
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
    /// The Settings filter (`/`): rows whose name contains it; `settings_filtering` while it takes typed text.
    pub settings_filter: String,
    pub settings_filtering: bool,
    /// First line of the Settings page on screen; `settings_follow` scrolls it to the selection.
    pub settings_scroll: usize,
    pub settings_follow: bool,
    /// The page Settings was opened from (and its tab's focused pane), for esc.
    pub settings_back: Option<(View, Option<crate::term::layout::PaneId>)>,
    pub overlay: Option<Overlay>,
    pub toasts: Vec<Toast>,
    /// The last `NOTICE_HISTORY` notifications, oldest first.
    pub notices: std::collections::VecDeque<Notice>,
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
    /// Counts the screen changes: the status bar tip moves to the next one of the new screen.
    pub tip_turn: usize,
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
    /// Saved tabs that could not be started at the last restore. They stay in the session file (with this
    /// window's tag) so that the next start tries them again (`App::restore_last_session`).
    pub unrestored: Vec<SavedTab>,
    /// Tabs the user closed, most recent last, with their index at the time (`reopen_tab`). In memory only.
    pub closed_tabs: Vec<(usize, SavedTab)>,
    /// Running as `noble-dev`: marked in the top bar, session in a separate file.
    pub dev: bool,
    /// This window's id in the shared session file (`store::instance_id`).
    pub instance: String,
    /// Whether a restored pane's directory can be used; runs on a helper thread with a timeout
    /// (`ops::DIR_PROBE_TIMEOUT`). Replaceable so tests can simulate a hanging network share.
    #[doc(hidden)]
    pub dir_probe: fn(&Path) -> bool,
    /// Whether a folder holds repositories two levels down (the add prompt's auto mode), probed on a
    /// helper thread with the same timeout. Replaceable so tests can simulate a hanging folder.
    #[doc(hidden)]
    pub repo_probe: fn(&Path, &[String]) -> bool,
    pub operator: String,
    /// The time the clocks show instead of now (the README screenshots); `None` in the app.
    #[doc(hidden)]
    pub clock: Option<chrono::DateTime<chrono::Local>>,
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
    /// Listening TCP ports per project folder, from the last scan (`ports` thread).
    pub ports: HashMap<PathBuf, Vec<u16>>,
    /// Pull request answers per project folder (`pr` thread).
    pub prs: HashMap<PathBuf, PrEntry>,
    /// When the ports were last asked for (`None`: at once, e.g. on entering Home).
    ports_asked: Option<Instant>,
    /// A port scan is on its way.
    ports_in_flight: bool,
    /// The pull request request on its way: the folder and the branch it is for.
    pr_in_flight: Option<(PathBuf, String)>,
}

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
    /// `boot`: false when the launch skipped the startup animation (`--no-boot`); the animation then
    /// counts as not shown and plays on a later launch.
    pub fn start(paths: Paths, tx: Tx, size: (u16, u16), boot: bool) -> App {
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
        let (ports_tx, ports_rx) = std::sync::mpsc::channel();
        crate::ports::spawn(tx.clone(), ports_rx);
        let (pr_tx, pr_rx) = std::sync::mpsc::channel();
        crate::pr::spawn(tx.clone(), pr_rx);

        let services = Services {
            sensor_req: sensor_tx,
            rescan: rescan_tx,
            ai_refresh: ai_tx,
            proj_cfg,
            proj_manual,
            ai_cfg,
            ports_req: ports_tx,
            pr_req: pr_tx,
        };
        let cache = ai::load_cache(&paths.data_file("ai-cache.json"));
        let mut app = App::build(cfg, paths, Some(services), tx, size, recent, workspaces);
        app.ai_installed = ai::installed_providers();
        app.dev = crate::util::is_dev_build();
        app.usage_history = UsageHistory::load(app.paths.data_file("ai-history.json"));
        app.reload_schemes();
        app.ui_state = ui_state;
        let version = env!("CARGO_PKG_VERSION");
        // `noble` and `noble-dev` keep their own record, so alternating between them does not replay it.
        let dev = app.dev;
        let last = if dev { &app.ui_state.data.boot_version_dev } else { &app.ui_state.data.boot_version };
        if boot && boot_due(app.cfg.general.boot_animation, last, version) {
            app.boot = Some(Boot::now());
            app.edit_ui_state(|d| {
                let slot = if dev { &mut d.boot_version_dev } else { &mut d.boot_version };
                *slot = version.to_string();
            });
        }
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
            app.toast_target(ToastLevel::Error, err.text, Some(ToastTarget::Config { line: err.line }));
        }
        for w in app.keymap.warnings.clone() {
            app.toast(ToastLevel::Warn, w);
        }
        app.restore_last_session();
        app.session_autosave = true;
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
            settings_filter: String::new(),
            settings_filtering: false,
            settings_scroll: 0,
            settings_follow: true,
            settings_back: None,
            overlay: None,
            toasts: Vec::new(),
            notices: std::collections::VecDeque::new(),
            boot: None,
            hits: Vec::new(),
            hover: None,
            slide: None,
            zoom_anim: None,
            fx: fx::Effects::default(),
            drawn_view: None,
            tip_turn: 0,
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
            unrestored: Vec::new(),
            closed_tabs: Vec::new(),
            dev: false,
            instance: crate::store::instance_id(),
            dir_probe: |p| p.is_dir(),
            repo_probe: |p, exclude| crate::projects::holds_repos(p, 2, exclude),
            operator,
            clock: None,
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
            ports: HashMap::new(),
            prs: HashMap::new(),
            ports_asked: None,
            ports_in_flight: false,
            pr_in_flight: None,
            cfg,
        }
    }

    /// Terminal body: the area between the top strip and the status bar.
    pub fn body(&self) -> Rect {
        Rect::new(0, 1, self.size.0, self.size.1.saturating_sub(2))
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
            // Decided by `apply`: only a value that Home shows counts.
            AppEvent::Ports(_) | AppEvent::Pr(..) | AppEvent::AddDecided { .. } | AppEvent::PaneCwd { .. } => false,
        };
        let before = self.ui_fingerprint();
        // Switching to or from battery changes what every screen draws (clock seconds, cursor blink).
        let power = self.on_battery();
        let changed = self.apply(ev);
        shown || changed || self.ui_fingerprint() != before || self.on_battery() != power
    }

    /// The event's state change. `true`: something visible changed (output in a visible pane, a value Home shows).
    fn apply(&mut self, ev: AppEvent) -> bool {
        match ev {
            AppEvent::Input(e) => self.on_input(e),
            AppEvent::Quit => self.quit = true,
            AppEvent::PtyOutput => return self.on_pty_output(),
            AppEvent::PaneCwd { pane, reported, usable } => return self.on_pane_cwd(pane, &reported, usable),
            AppEvent::PtyExit(id) => {
                if let Some(cwd) = self.panes.get(&id).map(|p| p.cwd()) {
                    self.refresh_git_at(&cwd);
                }
                if self.panes.contains_key(&id) {
                    let label = self.panes.get(&id).map(|p| p.label()).unwrap_or_default();
                    // The shell exited on its own: not a close the user made, so nothing is kept for reopening.
                    self.close_pane_as(id, false);
                    self.toast(ToastLevel::Info, format!("{label} exited"));
                }
            }
            AppEvent::SensorStatic(info) => self.sensors.info = *info,
            AppEvent::Sensors(s) => {
                self.sensors.ingest(*s);
                self.check_battery();
            }
            AppEvent::KillResult { pid, name, outcome } => match outcome {
                KillOutcome::Terminated => self.toast(ToastLevel::Ok, format!("terminated {name} ({pid})")),
                KillOutcome::Failed => self.toast(ToastLevel::Error, format!("could not terminate {name} ({pid})")),
                KillOutcome::Gone => self.toast(ToastLevel::Info, format!("{name} ({pid}) has already exited")),
                KillOutcome::Replaced => self.toast(
                    ToastLevel::Warn,
                    format!("{name} has already exited: pid {pid} is another process now, nothing was terminated"),
                ),
            },
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
            AppEvent::Ports(list) => return self.set_ports(list),
            AppEvent::AddDecided { raw, scan } => {
                if scan {
                    self.add_project_root(&raw);
                } else {
                    self.add_project(&raw);
                }
            }
            AppEvent::Pr(path, info) => return self.set_pr(path, info),
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

    /// Periodic work: boot animation, notifications, config watching.
    pub fn tick(&mut self) {
        // `fx` effects are left out: their pacing already draws their final frame.
        let before = (self.ui_fingerprint(), self.moving());
        self.tick_inner();
        // When the animation ends, the final (static) frame must draw too.
        if (self.ui_fingerprint(), self.moving()) != before {
            self.dirty = true;
        }
    }

    fn tick_inner(&mut self) {
        // Store files found damaged by a write since the last tick (kept aside, see `store::keep_aside`).
        for text in crate::store::take_notices() {
            self.toast(ToastLevel::Error, text);
        }
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
        self.toasts.retain(|t| t.expires_at().is_none_or(|u| u + linger > now));
        self.request_visible_git();
        self.sync_power_state();
        self.request_home_live_data();
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
        // Output that came in during the refresh interval of the search: its matches are computed now.
        if self.refresh_search_when_due() {
            self.dirty = true;
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

    /// The startup animation: first launch, then once per version; off means never.
    #[test]
    fn boot_plays_on_the_first_launch_and_after_an_update() {
        assert!(boot_due(true, "", "1.2.0"), "first launch");
        assert!(!boot_due(true, "1.2.0", "1.2.0"), "same version again");
        assert!(boot_due(true, "1.1.0", "1.2.0"), "after an update");
        assert!(!boot_due(false, "", "1.2.0") && !boot_due(false, "1.1.0", "1.2.0"), "off: never");
    }

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
