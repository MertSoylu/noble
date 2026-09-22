//! Uygulama durumu ve olay yönlendirme. Çizim `ui` modülündedir; buradaki
//! her şey durumu değiştirir, hiçbir şey ekrana doğrudan yazmaz.

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
pub use settings::{PREFIXES, SettingItem, SettingKey};

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
    Kill { pid: u32, name: String },
}

pub struct Confirm {
    pub title: String,
    pub body: String,
    pub action: ConfirmAction,
}

pub enum PromptPurpose {
    RenameTab(usize),
    SaveWorkspace,
    /// Projelerin aranacağı yeni kök klasör.
    AddRoot,
}

impl PromptPurpose {
    /// Girilebilecek en uzun metin (yollar uzun olabilir).
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

/// Terminal renk şeması seçicisi: gezinirken önizler, esc ile geri alır.
pub struct SchemePicker {
    pub selected: usize,
    pub original: String,
}

pub enum Overlay {
    /// İlk açılış: kısa tanıtım ve prefix tuşu seçimi (`PREFIXES` sırası).
    Welcome {
        prefix: usize,
    },
    Palette(PaletteState),
    Schemes(SchemePicker),
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

/// Fareyle tıklanabilir bölgeler; her karede çizim sırasında yeniden kurulur.
#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    Backdrop,
    /// Overlay kutusu: tıklama arka plana geçmesin.
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
    /// Şema seçicideki satır (`scheme_options` sırası).
    TermScheme(usize),
    OpenFiles,
    Proc(u32),
    SortCol(SortKey),
    PaletteItem(usize),
    ConfirmYes,
    ConfirmNo,
    WelcomePrefix(usize),
    WelcomeDone,
    MenuItem(usize),
    /// Pane çerçevesinin üst satırı (sağ tık menüsü için).
    PaneTitle(PaneId),
    /// Proje satırındaki hızlı eylem düğmesi (satır, eylem).
    ProjectAct(usize, ProjectAct),
}

pub(crate) enum Drag {
    Divider {
        tab: usize,
        div: Divider,
    },
    /// Sekme sürükleyerek yeniden sıralama.
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

/// Terminal geçmişinde arama (odaktaki pane'de, alt kenarda arama çubuğu).
pub struct SearchState {
    pub pane: PaneId,
    pub query: String,
    pub matches: Vec<crate::term::pane::Match>,
    /// Seçili eşleşme (`matches` içindeki sıra).
    pub current: Option<usize>,
    /// Eşleşmeler hesaplanırken geçmişin uzunluğu (mutlak satır → ekran satırı).
    pub history: usize,
}

/// ctrl basılıyken fare altındaki bağlantı: pane, ekran satırı, sütun aralığı.
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

/// Sayfa geçişi: önceki sayfanın gövde görüntüsü yana kayarak çıkar.
pub struct Slide {
    pub from: ratatui::buffer::Buffer,
    /// +1: yeni sayfa sağdan gelir, -1: soldan.
    pub dir: i32,
    pub started: Instant,
}

/// Pane tam ekran animasyonu: `from` → `to` dikdörtgeni arasında büyür/küçülür.
pub struct ZoomAnim {
    pub pane: PaneId,
    pub from: Rect,
    pub to: Rect,
    pub started: Instant,
}

/// Yumuşak yavaşlama (ease-out cubic), 0..1.
pub fn ease(started: Instant, dur: Duration) -> f64 {
    let t = (started.elapsed().as_secs_f64() / dur.as_secs_f64()).clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Bir AI oturumunun durumu. Hook kuruluysa Claude'un kendi olaylarından,
/// değilse pane başlığı/komutundan tahmin edilir (`Running`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentState {
    /// İstem gönderildi, Claude çalışıyor.
    Working,
    /// İzin ya da girdi bekliyor.
    NeedsYou,
    /// Cevabını bitirdi; sıra sende.
    Idle,
    /// Açık ama durumu bilinmiyor (hook yok).
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

/// Oturum listesi satırı.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentSession {
    pub tab: usize,
    pub pane: PaneId,
    pub kind: &'static str,
    pub place: String,
    pub state: AgentState,
}

/// Bir pane'den tek karede toplanan sinyaller.
struct PaneSignal {
    id: PaneId,
    visible: bool,
    bell: bool,
    notice: Option<String>,
    /// Prompt geri geldiyse ve kullanıcı bir komut başlatmışsa, komutun süresi.
    finished: Option<Duration>,
    /// Prompt geri geldiyse pane'in dizini.
    cwd: Option<PathBuf>,
}

/// Arka plan hizmetlerine giden kanallar (başsız testlerde yok).
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
    pub view: View,
    pub tabs: Vec<Tab>,
    pub panes: HashMap<PaneId, Pane>,
    next_id: PaneId,
    pub prefix_armed: bool,
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
    /// Fare imlecinin konumu (hover efekti için).
    pub hover: Option<(u16, u16)>,
    pub slide: Option<Slide>,
    pub zoom_anim: Option<ZoomAnim>,
    /// Son çizilen sayfa ve gövde görüntüsü (geçiş başlatmak için).
    pub drawn_view: Option<View>,
    pub last_body: Option<ratatui::buffer::Buffer>,
    pub(crate) drag: Option<Drag>,
    last_click: Option<(Instant, u16, u16)>,
    pub size: (u16, u16),
    tx: Tx,
    /// Başsız modda olay alıcısı (testler `pump` ile boşaltır).
    rx: Option<std::sync::mpsc::Receiver<AppEvent>>,
    services: Option<Services>,
    /// Başlatıcılar ve PATH'te bulunup bulunmadıkları.
    pub launchers: Vec<(Launcher, bool)>,
    pub quit: bool,
    pub started: Instant,
    pub restored_tabs: usize,
    pub operator: String,
    /// Git durumu istenen depolar ve istek zamanı (tekrarları seyreltmek için).
    git_requested: HashMap<PathBuf, Instant>,
    pub search: Option<SearchState>,
    pub link_hover: Option<LinkHover>,
    pub usage_history: UsageHistory,
    pub ui_state: crate::store::UiState,
    /// Claude Code hook'larından gelen son olaylar (pane → kayıt).
    pub agent_hooks: HashMap<PaneId, crate::hooks::HookRecord>,
    last_hook_scan: Instant,
    /// `~/.claude/settings.json`'da NOBLE hook'ları kurulu mu (Settings'te gösterilir).
    pub hooks_installed: bool,
    /// Sensör iş parçacığına en son bildirilen mod (görünen ekrana göre).
    sensor_mode: Option<SensorMode>,
    /// Önceki karedeki görünüm (Home'a dönüşü yakalamak için).
    last_view: Option<View>,
    /// Sensörlere en son bildirilen güç durumu (pilde mi).
    sensor_on_battery: bool,
    /// Her bildirimde artar; yeniden çizim gerekip gerekmediğini anlamak için.
    toast_serial: u64,
    /// Olay dışı yollarla (tick, config yeniden yükleme) ekran değişti.
    dirty: bool,
    /// Seçilebilir terminal renk şemaları (Windows Terminal'den okunanlar + yerleşikler).
    pub term_schemes: Vec<crate::theme::TermScheme>,
    /// Düşük pil uyarısı verilen en son eşik (%20, %10); şarja takılınca sıfırlanır.
    battery_warned: u8,
    /// Uyarısı verilmiş kota pencereleri (sağlayıcı, pencere, sıfırlanma zamanı).
    quota_warned: std::collections::HashSet<(String, String, Option<i64>)>,
    /// Dış terminale bir kez BEL gönderilecek (görev çubuğu yanıp söner).
    pub outer_bell: bool,
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
    /// Arka plan hizmetleri olmadan (testler ve ekran görüntüleri için).
    pub fn headless(cfg: Config, size: (u16, u16)) -> App {
        let (tx, rx) = std::sync::mpsc::channel();
        let paths = Paths { config: PathBuf::from("config.toml"), data: std::env::temp_dir().join("noble-headless") };
        let mut app = App::build(cfg, paths, None, tx, size, Recent::memory(), Workspaces::memory());
        app.boot = None;
        app.rx = Some(rx);
        app
    }

    /// Başsız modda bekleyen olayları işler (PTY çıktısı vb.).
    pub fn pump(&mut self) {
        let events: Vec<AppEvent> = match &self.rx {
            Some(rx) => rx.try_iter().collect(),
            None => return,
        };
        for ev in events {
            self.handle(ev);
        }
    }

    /// Gerçek uygulama: config'i yükler, geçmişi okur, hizmetleri başlatır.
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
        app.usage_history = UsageHistory::load(app.paths.data_file("ai-history.json"));
        app.reload_schemes();
        app.ui_state = crate::store::UiState::load(app.paths.data_file("state.json"));
        crate::hooks::prune(&app.paths.data);
        app.hooks_installed = crate::hooks::settings_path().is_some_and(|p| crate::hooks::is_installed(&p));
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
            && let Some(ws) = crate::store::load_session(&app.paths.data_file("session.json"))
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
        let shell = resolve_shell(&cfg.terminal);
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
            view: View::Bridge,
            tabs: Vec::new(),
            panes: HashMap::new(),
            next_id: 1,
            prefix_armed: false,
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
            quit: false,
            started: Instant::now(),
            restored_tabs: 0,
            operator,
            git_requested: HashMap::new(),
            search: None,
            link_hover: None,
            usage_history: UsageHistory::memory(),
            ui_state: crate::store::UiState::memory(),
            agent_hooks: HashMap::new(),
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

    /// Terminal gövdesi: üst şerit ve durum çubuğu arasındaki alan.
    pub fn body(&self) -> Rect {
        Rect::new(0, 1, self.size.0, self.size.1.saturating_sub(2))
    }

    /// Animasyon için sık yeniden çizim gerekiyor mu?
    pub fn animating(&self) -> bool {
        self.boot.is_some() || self.slide.is_some() || self.zoom_anim.is_some()
    }

    /// Olayı işler; ekranda görünen bir şey değiştiyse `true` (yeniden çizim).
    /// Görünmeyen verideki değişiklikler (ör. terminaldeyken sensörler) ekranı
    /// uyandırmaz: pil dostu.
    pub fn handle(&mut self, ev: AppEvent) -> bool {
        let shown = match &ev {
            AppEvent::Input(_) | AppEvent::PtyExit(_) | AppEvent::KillResult { .. } => true,
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

    /// Son çizimden beri olay dışı bir değişiklik oldu mu (bir kez okunur).
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Ekranın her görünümde gösterdiği ortak durum: bildirimler ve sekme işaretleri.
    fn ui_fingerprint(&self) -> (u64, usize, Vec<(bool, bool)>) {
        (self.toast_serial, self.toasts.len(), self.tabs.iter().map(|t| (t.activity, t.alert)).collect())
    }

    /// Olayın durum değişikliği. `true`: görünen bir pane'e çıktı geldi.
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
        }
        false
    }

    /// Hiçbir olay olmasa da ekranın ne zaman değişeceği: saat, animasyon,
    /// yükleme göstergesi, bildirimin süresinin dolması… `None`: değişmez.
    /// Ana döngü yalnızca bu an gelince (ya da bir olay olunca) çizer.
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
            // Home: büyük saatin saniyesi ve yanıp sönen iki noktası (pildeyken yok).
            View::Bridge if self.live_clock() => want(to_second),
            // Diğer ekranlarda yalnızca üst şeritteki HH:MM.
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

    /// Pilde %20 ve %10'a inilince birer kez uyarır.
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

    /// Yeni kota verisi: geçmişe yazılır, eşiği aşan pencere için bir kez uyarılır.
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
                    let left = (t - chrono::Utc::now().timestamp()).max(0) as u64;
                    format!(" · resets in {}", crate::util::fmt_duration(Duration::from_secs(left)))
                })
                .unwrap_or_default();
            let window = crate::ai::window_name(&w.label);
            self.toast(ToastLevel::Warn, format!("{} {window} usage at {}%{reset}", state.name, w.used));
        }
    }

    /// Hook kayıtlarını işler: değişen durumlar arka plan sekmesindeyse bildirilir.
    pub fn apply_hook_records(&mut self, records: HashMap<PaneId, crate::hooks::HookRecord>) {
        let visible: Vec<PaneId> = match self.view {
            View::Term(i) => self.tabs.get(i).map(|t| t.panes()).unwrap_or_default(),
            _ => Vec::new(),
        };
        let mut changed = Vec::new();
        for (pane, rec) in &records {
            if !self.panes.contains_key(pane) {
                crate::hooks::remove_record(&self.paths.data, std::process::id(), *pane);
                continue;
            }
            if self.agent_hooks.get(pane) != Some(rec) {
                changed.push((*pane, rec.clone()));
            }
        }
        self.agent_hooks = records.into_iter().filter(|(p, _)| self.panes.contains_key(p)).collect();
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

    /// Pane'de çalışan AI aracı ve durumu.
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
        let kind = crate::ai::agent_kind(p.command.as_deref(), &p.label())?;
        let alert = self.tabs.iter().any(|t| t.alert && t.root.contains(pane));
        Some((kind, if alert { AgentState::NeedsYou } else { AgentState::Running }))
    }

    /// Tüm sekmelerdeki AI oturumları (sekme sırasıyla).
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

    /// Projedeki açık AI oturumları: (araç, durum). Aynı araç birden çok
    /// pane'deyse en çok dikkat isteyen durum gösterilir.
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
        // Önceki git bilgisini koru (yeniden taramada titremesin).
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

    /// Filtreye uyan projelerin indeksleri.
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
        let shown = signals.iter().any(|s| s.visible);
        for sig in signals {
            if let Some(cwd) = &sig.cwd {
                // Komut bitti: depoda değişiklik olmuş olabilir.
                self.refresh_git_at(cwd);
                if let Some(p) = self.panes.get_mut(&sig.id) {
                    p.command_started = None;
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

    /// Arka plan sekmelerinden gelen sinyalleri sekme işaretine ve bildirime çevirir.
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
            // Görünen sekmede yalnızca uygulamanın açık bildirimi gösterilir.
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

    /// Dizüstü pilden mi çalışıyor (prizdeyse ya da pil yoksa `false`).
    pub fn on_battery(&self) -> bool {
        self.sensors.battery().is_some_and(|(b, _)| b.state == crate::battery::PowerState::Discharging)
    }

    /// Home'daki saat saniyeleri ve yanıp sönen iki nokta: pildeyken durur
    /// (ekran dakikada bir çizilsin diye).
    pub fn live_clock(&self) -> bool {
        !self.on_battery()
    }

    /// Görünen ekrana göre arka plan işlerini ayarlar: sensör sıklığı ve Home'a
    /// dönünce görünen projelerin git durumunun (en fazla dakikada bir) tazelenmesi.
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
        // Kota paneli yalnızca Home'da: yenileme yalnızca orada, girişte hemen.
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

    /// `cwd`'nin bulunduğu projenin git durumunu (sık değilse) yeniden ister.
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

    /// Home'da görünen ama henüz git durumu olmayan projeleri ister (ilk
    /// taramada yalnızca en son kullanılanların durumu alınır).
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
            // Başarısız depolar (git yok vb.) tekrar tekrar istenmesin.
            self.request_git(path, Duration::MAX);
        }
    }

    pub fn set_clipboard(&mut self, text: &str, announce: bool) {
        match arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_string())) {
            Ok(()) => {
                if announce {
                    let n = text.chars().count();
                    self.toast(ToastLevel::Ok, format!("copied {n} chars"));
                }
            }
            Err(_) => self.toast(ToastLevel::Warn, "clipboard unavailable"),
        }
    }

    /// Periyodik işler: açılış animasyonu, bildirimler, config izleme.
    pub fn tick(&mut self) {
        let before = (self.ui_fingerprint(), self.animating());
        self.tick_inner();
        // Animasyon bittiyse son (durağan) kare de çizilmeli.
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
        // Arama, pane kapanır ya da odak başka yere geçerse kapanır.
        if let Some(s) = &self.search
            && self.focused_pane() != Some(s.pane)
        {
            self.search = None;
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

    /// Windows Terminal ayarlarını yeniden okur (başsız modda dokunulmaz).
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
        self.shell = resolve_shell(&self.cfg.terminal);
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

    /// Çıkışta: oturumu kaydet.
    pub fn shutdown(&mut self) {
        if self.cfg.terminal.restore_session {
            let ws = self.snapshot("last session");
            crate::store::save_session(&self.paths.data_file("session.json"), &ws);
        }
        self.panes.clear();
    }

    /// Süren sürükleme türü (hover efekti için).
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
