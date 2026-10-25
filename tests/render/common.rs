//! Shared helpers: drawing to text, clicks, the demo app and waiting for panes.

use crossterm::event::KeyModifiers;
use noble::ai::{Presence, ProviderState, Status, Usage, Window};
use noble::app::App;
use noble::config::Config;
use noble::event::AppEvent;
use noble::projects::{GitInfo, Project};
use noble::sensors::{DiskInfo, ProcInfo, SensorSample, StaticInfo};
use noble::store::{SavedTab, Workspace};
use noble::term::layout::SavedNode;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::{Duration, SystemTime};

/// The dump folder. The first call of the run empties it, before any dump is written: `Once` makes every other test
/// thread wait until the clearing is done. The e2e binary (`tests/e2e.rs`) writes its own `e2e-*.txt` dumps into the
/// same folder, so `clear_dumps` leaves those alone; cargo runs the two binaries one after another, but they can also
/// run side by side (`cargo test --test e2e` in one terminal, `--test render` in another).
pub(crate) fn audit_dir() -> PathBuf {
    static CLEARED: Once = Once::new();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("audit");
    CLEARED.call_once(|| clear_dumps(&dir));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Removes the files of an earlier run from `dir` (the folder itself stays), except the e2e binary's `e2e-*` dumps.
/// Files that cannot be removed (open in an editor on Windows) stay: a stale dump is better than a failed test.
pub(crate) fn clear_dumps(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let owned_by_e2e = entry.file_name().to_string_lossy().starts_with("e2e-");
        if path.is_file() && !owned_by_e2e {
            let _ = std::fs::remove_file(&path);
        }
    }
}

pub(crate) fn render(app: &mut App, w: u16, h: u16) -> String {
    app.size = (w, h);
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| noble::ui::draw(f, app)).unwrap();
    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..h {
        let mut skip = 0usize;
        for x in 0..w {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let sym = buf[(x, y)].symbol();
            out.push_str(sym);
            skip = unicode_width::UnicodeWidthStr::width(sym).saturating_sub(1);
        }
        out.push('\n');
    }
    out
}

/// A folder under the temp dir, removed with everything in it when the guard drops (also when the test returns
/// early or panics). Parallel tests give theirs a name with the process id in it.
pub(crate) struct TempDir(PathBuf);

impl TempDir {
    /// `name` plus the process id, created empty.
    pub(crate) fn new(name: &str) -> TempDir {
        TempDir::at(std::env::temp_dir().join(format!("{name}-{}", std::process::id())))
    }

    /// The folder at `path`, emptied first and removed when the guard drops.
    pub(crate) fn at(path: PathBuf) -> TempDir {
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn save(name: &str, text: &str) {
    std::fs::write(audit_dir().join(format!("{name}.txt")), text).unwrap();
}

pub(crate) fn demo_app(w: u16, h: u16) -> App {
    // Snapshots must not land on an in-between frame; the animations have their own tests.
    let mut cfg = Config::default();
    cfg.general.animations = false;
    let mut app = App::headless(cfg, (w, h));
    app.operator = "mert".into();
    app.handle(AppEvent::SensorStatic(Box::new(StaticInfo {
        host: "DESKTOP-SM3V".into(),
        os: "Windows 11 Home".into(),
        cpu_brand: "12th Gen Intel(R) Core(TM) i7-12700H".into(),
        cores: 12,
        total_mem: 32 * 1024 * 1024 * 1024,
    })));
    let procs: Vec<ProcInfo> = [
        ("code.exe", 12.4, 820u64),
        ("msedge.exe", 8.1, 640),
        ("noble.exe", 2.3, 24),
        ("pwsh.exe", 1.2, 96),
        ("explorer.exe", 0.8, 180),
        ("claude.exe", 0.6, 210),
        ("svchost.exe", 0.2, 40),
        ("Discord.exe", 0.1, 300),
    ]
    .iter()
    .enumerate()
    .map(|(i, (n, c, m))| ProcInfo {
        pid: 1000 + i as u32 * 17,
        started: 0,
        name: n.to_string(),
        cpu: *c,
        mem: m * 1024 * 1024,
    })
    .collect();
    for i in 0..120 {
        let t = i as f32 / 8.0;
        app.handle(AppEvent::Sensors(Box::new(SensorSample {
            cpu: 22.0 + 18.0 * t.sin() + if i % 17 == 0 { 30.0 } else { 0.0 },
            cores: (0..12).map(|c| (c as f32 * 7.0 + i as f32 * 3.0) % 90.0).collect(),
            freq_mhz: 2700,
            mem_used: 11 * 1024 * 1024 * 1024 + (i as u64 * 7_000_000),
            mem_total: 32 * 1024 * 1024 * 1024,
            swap_used: 1_200_000_000,
            swap_total: 4_000_000_000,
            rx_rate: 900_000.0 * (1.0 + (t * 1.3).sin() as f64),
            tx_rate: 120_000.0 * (1.0 + (t * 0.7).cos() as f64),
            disks: vec![
                DiskInfo { mount: "C:\\".into(), total: 512 << 30, used: 311 << 30 },
                DiskInfo { mount: "D:\\".into(), total: 1024 << 30, used: 900 << 30 },
            ],
            procs: procs.clone(),
            proc_count: 312,
            uptime: 5 * 3600 + 8 * 60,
            battery: Some(noble::battery::Battery {
                percent: 76.0,
                state: noble::battery::PowerState::Discharging,
                secs_left: Some(3 * 3600 + 12 * 60),
                secs_to_full: None,
            }),
        })));
    }
    let now = SystemTime::now();
    let mk = |name: &str, branch: &str, mins: u64, git: Option<GitInfo>| Project {
        name: name.into(),
        path: PathBuf::from(format!("C:\\Users\\Mert\\Desktop\\{name}")),
        branch: Some(branch.into()),
        last_active: Some(now - Duration::from_secs(mins * 60)),
        git,
        repo: true,
        tasks: Vec::new(),
    };
    app.handle(AppEvent::Projects(vec![
        mk(
            "noble-rs",
            "main",
            3,
            Some(GitInfo {
                dirty: 3,
                untracked: 1,
                ahead: 1,
                behind: 0,
                branch: Some("main".into()),
                last_subject: Some("feat: HUD bridge".into()),
                last_commit: Some(chrono::Utc::now().timestamp() - 7200),
                commits: [
                    ("a1b2c3d", 7200, "feat: HUD bridge"),
                    ("9f8e7d6", 5 * 3600, "fix: hover buttons no longer leak text"),
                    ("4c5d6e7", 26 * 3600, "docs: README keys table"),
                    ("0badc0d", 3 * 86_400, "refactor: split app into modules"),
                ]
                .iter()
                .map(|(h, ago, s)| noble::projects::Commit {
                    hash: h.to_string(),
                    time: chrono::Utc::now().timestamp() - ago,
                    author: "Mert".into(),
                    subject: s.to_string(),
                })
                .collect(),
                changes: vec![
                    (" M".into(), "src/ui/bridge.rs".into()),
                    ("M ".into(), "src/app/menu.rs".into()),
                    ("??".into(), "tests/new.rs".into()),
                ],
            }),
        ),
        mk("api-server", "feature/auth-refresh", 90, Some(GitInfo { dirty: 0, ..Default::default() })),
        mk("dotfiles", "main", 60 * 26, Some(GitInfo { dirty: 1, behind: 2, ..Default::default() })),
        mk("blog", "master", 60 * 24 * 9, None),
        mk("Noble", "main", 60 * 5, Some(GitInfo { dirty: 54, ..Default::default() })),
        mk("scratch", "dev", 60 * 24 * 40, None),
    ]));
    let reset = chrono::Utc::now().timestamp() + 2 * 3600 + 14 * 60;
    let states = vec![
        ProviderState {
            id: "claude",
            name: "Claude Code",
            login_hint: "claude",
            presence: Presence::Ready,
            status: Status::Ok,
            usage: Some(Usage {
                windows: vec![
                    Window { label: "5H".into(), used: 71, resets_at: Some(reset) },
                    Window { label: "WEEK".into(), used: 28, resets_at: Some(reset + 4 * 86_400) },
                ],
                plan: Some("MAX".into()),
                note: None,
            }),
            fetched_at: Some(chrono::Utc::now().timestamp() - 120),
        },
        ProviderState {
            id: "codex",
            name: "Codex",
            login_hint: "codex login",
            presence: Presence::Ready,
            status: Status::Error("offline".into()),
            usage: Some(Usage {
                windows: vec![
                    Window { label: "5H".into(), used: 18, resets_at: Some(reset + 3600) },
                    Window { label: "WEEK".into(), used: 92, resets_at: Some(reset + 86_400) },
                ],
                plan: Some("PLUS".into()),
                note: None,
            }),
            fetched_at: Some(chrono::Utc::now().timestamp() - 3600),
        },
    ];
    for s in states {
        app.handle(AppEvent::Ai(Box::new(s)));
    }
    app.workspaces.upsert(Workspace {
        name: "daily".into(),
        saved_at: 0,
        tabs: vec![SavedTab {
            name: None,
            origin: "noble-rs".into(),
            layout: SavedNode::Leaf { cwd: "C:\\".into(), launch: None },
            focus: 0,
        }],
        ..Default::default()
    });
    app.recent.record(&std::env::temp_dir());
    app
}

pub(crate) const SIZES: [(u16, u16); 7] = [(160, 45), (120, 34), (110, 30), (90, 28), (76, 24), (56, 18), (30, 8)];

pub(crate) fn click(app: &mut App, x: u16, y: u16) {
    use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
    for kind in [MouseEventKind::Down(MouseButton::Left), MouseEventKind::Up(MouseButton::Left)] {
        app.handle(AppEvent::Input(Event::Mouse(MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        })));
    }
}

pub(crate) fn find_hit(app: &App, pred: impl Fn(&noble::app::Hit) -> bool) -> Option<ratatui::layout::Rect> {
    app.hits.iter().rev().find(|(_, h)| pred(h)).map(|(r, _)| *r)
}

pub(crate) fn wait_for(app: &mut App, pane: noble::term::layout::PaneId, needle: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(40);
    loop {
        app.pump();
        let (lines, _) = app.panes[&pane].all_lines();
        if lines.iter().any(|l| l.trim() == needle) {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "output '{needle}' never arrived:\n{}", lines.join("\n"));
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Pumps and ticks the app for `for_how_long`, as the main loop does while output arrives: the timed work
/// (the search refresh after output, notifications) runs its course.
pub(crate) fn run_for(app: &mut App, for_how_long: Duration) {
    let end = std::time::Instant::now() + for_how_long;
    while std::time::Instant::now() < end {
        app.pump();
        app.tick();
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub(crate) fn find_hit_inner(app: &App) -> Option<ratatui::layout::Rect> {
    app.hits.iter().find_map(|(_, h)| match h {
        noble::app::Hit::Pane { inner, .. } => Some(*inner),
        _ => None,
    })
}

pub(crate) fn mouse(app: &mut App, kind: crossterm::event::MouseEventKind, x: u16, y: u16) {
    use crossterm::event::{Event, MouseEvent};
    app.handle(AppEvent::Input(Event::Mouse(MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE })));
}

/// Runs the real `noble hook <event>` the way Claude Code's hooks do: inside the pane's
/// environment (`NOBLE_INSTANCE`, `NOBLE_PANE`), with the event JSON on stdin.
pub(crate) fn noble_hook(app: &App, pane: noble::term::layout::PaneId, instance: u32, event: &str, stdin: &str) {
    use std::io::Write as _;
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_noble"))
        .args(["hook", event])
        .env("NOBLE_HOME", &app.paths.data)
        .env("NOBLE_INSTANCE", instance.to_string())
        .env("NOBLE_PANE", pane.to_string())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("noble hook");
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    assert!(child.wait().unwrap().success());
}

/// What `App::tick` does once a second while hooks are live.
pub(crate) fn scan_hooks(app: &mut App) {
    let records = noble::hooks::read_records(&app.paths.data, std::process::id());
    app.apply_hook_records(records);
}

/// Slow Windows CI shells (cmd.exe) can take well over 30 s to come back to the prompt.
pub(crate) fn pump_until(app: &mut App, what: &str, done: impl Fn(&App) -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        app.pump();
        if done(app) {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "{what} never happened\n{}", render(app, 110, 30));
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Presses Enter until `done` holds, sending it again every 300 ms. A key sent before a program is reading input
/// can be lost: the cmd.exe agent's `pause` (CI, Windows) kept waiting after one Enter sent as soon as its title
/// showed. An Enter that lands at the shell prompt only makes an empty line, so repeating it does no harm.
pub(crate) fn press_enter_until(
    app: &mut App,
    pane: noble::term::layout::PaneId,
    what: &str,
    done: impl Fn(&App) -> bool,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        app.panes[&pane].write(b"\r");
        let next = std::time::Instant::now() + Duration::from_millis(300);
        while std::time::Instant::now() < next {
            app.pump();
            if done(app) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(std::time::Instant::now() < deadline, "{what} never happened\n{}", render(app, 110, 30));
    }
}

/// A stand-in for an agent's TUI in `shell`: sets the window title the way the agent
/// does, then waits for a line (Enter "exits" it).
pub(crate) fn fake_agent(shell: &noble::term::pane::ShellSpec, title: &str) -> String {
    match shell.label().to_lowercase().as_str() {
        "pwsh" | "powershell" => {
            format!("Write-Host -NoNewline ([char]27 + ']0;{title}' + [char]7); $null = Read-Host")
        }
        // cmd.exe has no escape sequences: `title` sets the console title, which ConPTY reports as OSC 0.
        "cmd" => format!("title {title} & pause >nul"),
        // bash, zsh and fish all understand `\e` in printf. The wait is an external program:
        // fish's own `read` re-titles the terminal.
        _ => format!("printf '\\e]0;{title}\\a'; head -n 1 >/dev/null"),
    }
}

/// Waits until the shell's startup output settles: the screen is unchanged for 700 ms and not blank. Panics with
/// the screen when that does not happen within 15 s, so a shell that never gets to its prompt fails the test.
pub(crate) fn wait_idle(app: &mut App, pane: noble::term::layout::PaneId) {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut last = String::new();
    let mut stable = std::time::Instant::now();
    loop {
        app.pump();
        let now = app.panes[&pane].parser().screen().contents();
        if now != last {
            last = now;
            stable = std::time::Instant::now();
        } else if !last.trim().is_empty() && stable.elapsed() > Duration::from_millis(700) {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "the shell's output never settled:\n{last}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Asserts that a test over "every installed shell" checked at least one shell, and says which. The default shell is
/// always on the platform's image (cmd or pwsh on Windows, bash or sh on Unix), so an empty list means the skips hid
/// the whole test.
pub(crate) fn assert_checked<S: std::fmt::Debug>(checked: &[S], test: &str) {
    assert!(!checked.is_empty(), "{test}: no shell was checked");
    eprintln!("{test}: checked {checked:?}");
}

/// Degenerate sizes: zero width or height, 1×1, one row, one column.
pub(crate) const TINY: [(u16, u16); 12] =
    [(0, 0), (0, 10), (10, 0), (1, 1), (2, 2), (3, 1), (1, 3), (8, 2), (30, 8), (300, 1), (1, 120), (12, 5)];

pub(crate) fn session_file(dev: bool) -> &'static str {
    if dev { "session-dev.json" } else { "session.json" }
}

pub(crate) fn toast_texts(app: &App) -> Vec<String> {
    app.toasts.iter().map(|t| t.text.clone()).collect()
}

/// Whether this bash has `PS0` (4.4+; macOS `/bin/bash` is 3.2).
pub(crate) fn bash_has_ps0(shell: &str) -> bool {
    let program = if shell.is_empty() { "bash" } else { shell };
    let out = std::process::Command::new(program).arg("--version").output().expect("bash --version");
    let text = String::from_utf8_lossy(&out.stdout);
    let version = text.split("version ").nth(1).unwrap_or_default();
    let mut parts = version.split(|c: char| !c.is_ascii_digit()).filter_map(|p| p.parse::<u32>().ok());
    let (major, minor) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    (major, minor) >= (4, 4)
}

/// A hook record as `noble hook` writes it.
pub(crate) fn hook_rec(event: &str, ts: i64, subagents: usize) -> noble::hooks::HookRecord {
    noble::hooks::HookRecord {
        event: event.into(),
        message: (event == "notification").then(|| "Claude needs your permission to use Bash".into()),
        ts,
        subagents,
    }
}

/// Renders and returns the buffer too (to check colors).
pub(crate) fn render_buf(app: &mut App, w: u16, h: u16) -> (String, ratatui::buffer::Buffer) {
    let text = render(app, w, h);
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| noble::ui::draw(f, app)).unwrap();
    (text, term.backend().buffer().clone())
}

/// Plugs the laptop in (`plugged`) or runs it on battery.
pub(crate) fn set_power(app: &mut App, plugged: bool) {
    use noble::battery::{Battery, PowerState};
    let mut sample = app.sensors.last.clone().unwrap();
    let state = if plugged { PowerState::Full } else { PowerState::Discharging };
    sample.battery = Some(Battery { percent: 80.0, state, secs_left: None, secs_to_full: None });
    app.handle(AppEvent::Sensors(Box::new(sample)));
}

/// Nothing asks for fast frames: only the top bar's clock (once a minute) and the like.
pub(crate) fn idle_redraw(app: &App) -> bool {
    use chrono::Timelike;
    app.redraw_after().is_none_or(|d| d > Duration::from_millis(150)) || chrono::Local::now().second() == 59
}

pub(crate) const SPIN: &str = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏";

/// The text of the buffer cells under a rect (one row).
pub(crate) fn cells_text(buf: &ratatui::buffer::Buffer, r: ratatui::layout::Rect) -> String {
    (r.x..r.right()).map(|x| buf[(x, r.y)].symbol()).collect()
}

/// Two frame glyphs side by side (an overlay box right against the panel under it) read as a doubled border.
pub(crate) fn assert_no_doubled_frame(text: &str) {
    for pair in ["╭╭", "╮╮", "╰╰", "╯╯", "││"] {
        assert!(!text.contains(pair), "doubled border {pair}:\n{text}");
    }
}

/// The frame as cells (for style checks), drawn like `render`.
pub(crate) fn render_cells(app: &mut App, w: u16, h: u16) -> ratatui::buffer::Buffer {
    app.size = (w, h);
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| noble::ui::draw(f, app)).unwrap();
    term.backend().buffer().clone()
}

/// The first place `needle` starts in the frame, at or right of column `min_x` (ASCII text).
pub(crate) fn find_text(buf: &ratatui::buffer::Buffer, needle: &str, min_x: u16) -> Option<(u16, u16)> {
    let chars: Vec<char> = needle.chars().collect();
    for y in 0..buf.area.height {
        for x in min_x..buf.area.width {
            let hit = chars.iter().enumerate().all(|(i, c)| {
                let cx = x + i as u16;
                cx < buf.area.width && buf[(cx, y)].symbol() == c.to_string()
            });
            if hit {
                return Some((x, y));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_dir_clears_once_then_keeps_later_dumps() {
        // The first `save` of the run empties the folder; a file added after it stays for later dumps.
        save("clear-probe", "first");
        let stale = audit_dir().join("clear-probe-stale.txt");
        std::fs::write(&stale, "left over").unwrap();
        save("clear-probe", "second");
        assert!(stale.exists(), "a later dump cleared the folder again");
        std::fs::remove_file(&stale).unwrap();
    }

    #[test]
    fn clear_dumps_keeps_the_e2e_binarys_files() {
        let dir = TempDir::new("noble-audit-clear");
        let stale = dir.path().join("help-160x45.txt");
        let e2e = dir.path().join("e2e-welcome.txt");
        std::fs::write(&stale, "old").unwrap();
        std::fs::write(&e2e, "old").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        clear_dumps(dir.path());
        assert!(!stale.exists(), "a render dump survived the clearing");
        assert!(e2e.exists(), "the e2e binary's dump was removed");
        assert!(dir.path().join("sub").is_dir(), "a folder was removed");
    }

    #[test]
    fn clear_dumps_ignores_a_missing_folder() {
        let dir = TempDir::new("noble-audit-missing");
        let missing = dir.path().join("not-there");
        clear_dumps(&missing);
        assert!(!missing.exists());
    }
}
