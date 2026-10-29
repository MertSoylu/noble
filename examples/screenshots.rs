//! Generates the README screenshots: the app is built headless, filled with fake
//! data and every frame is turned into a colorful SVG.
//!
//! Run: `cargo run --example screenshots` → `docs/assets/*.svg`
//!
//! Box drawing, block and braille characters are not left to the font but drawn
//! as vectors, so the image looks the same in every browser and font.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::{Buffer, Cell};
use ratatui::style::{Color, Modifier};

use chrono::TimeZone;
use noble::ai::{Presence, ProviderState, Status, Usage, Window};
use noble::app::{App, PrEntry, View};
use noble::bg::BgSession;
use noble::config::Config;
use noble::event::AppEvent;
use noble::hooks::{Agent, HookRecord, NOTIFICATION, PROMPT, SESSION_START};
use noble::keys::Action;
use noble::pr::{Checks, PrInfo, PrState};
use noble::projects::{Commit, GitInfo, Project};
use noble::sensors::{DiskInfo, ProcInfo, SensorSample, StaticInfo};
use noble::theme::THEMES;

/// Cell size (pixels) and font.
const CW: f64 = 8.4;
const CH: f64 = 18.0;
const FONT: &str = "ui-monospace,'Cascadia Mono','SF Mono',Menlo,Consolas,'DejaVu Sans Mono',monospace";

fn main() {
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs").join("assets");
    std::fs::create_dir_all(&out).expect("docs/assets not created");

    // The main image: Home.
    let (w, h) = (150, 42);
    let mut app = demo("amber", w, h);
    write(&out, "home", &window_svg(&snapshot(&mut app, w, h), &app, "noble"));

    // The System screen.
    let mut app = demo("amber", w, h);
    app.run(Action::System);
    write(&out, "system", &window_svg(&snapshot(&mut app, w, h), &app, "noble — system"));

    // The Settings screen.
    let (sw, sh) = (120, 40);
    let mut app = demo("amber", sw, sh);
    app.view = View::Settings;
    write(&out, "settings", &window_svg(&snapshot(&mut app, sw, sh), &app, "noble — settings"));

    // Command palette.
    let (pw, ph) = (120, 34);
    let mut app = demo("amber", pw, ph);
    app.run(Action::Palette);
    write(&out, "palette", &window_svg(&snapshot(&mut app, pw, ph), &app, "noble — command palette"));

    // Terminals: two split panes, fake content.
    let mut app = demo("amber", w, h);
    terminals(&mut app);
    let mut buf = snapshot(&mut app, w, h);
    // A fake project path instead of the real (temporary) folder in the pane title.
    let tmp = noble::util::tilde(&std::env::temp_dir());
    replace_in(&mut buf, &format!(" {tmp} "), r" D:\dev\noble ");
    replace_in(&mut buf, &format!(" {} ", tmp.trim_end_matches(['\\', '/'])), r" D:\dev\noble ");
    write(&out, "terminals", &window_svg(&buf, &app, "noble — terminals"));
    app.panes.clear();

    // Theme gallery: the same Home screen in six themes.
    let (tw, th) = (110, 32);
    let tiles: Vec<(String, String)> = ["ice", "synth", "catppuccin", "gruvbox", "nord", "latte"]
        .iter()
        .map(|name| {
            let mut app = demo(name, tw, th);
            let label = THEMES.iter().find(|t| t.name == *name).map(|t| t.label).unwrap_or(name);
            (window_svg(&snapshot(&mut app, tw, th), &app, &format!("theme: {label}")), label.to_string())
        })
        .collect();
    write(&out, "themes", &grid_svg(&tiles, 3));
    println!("screenshots written to {}", out.display());
}

fn write(dir: &Path, name: &str, svg: &str) {
    let path = dir.join(format!("{name}.svg"));
    std::fs::write(&path, svg).expect("could not write svg");
    println!("  {} ({} KB)", path.display(), svg.len() / 1024);
}

/// Replaces text in the buffer, writing the whole replacement. A longer text takes the frame line
/// (`─`) that follows it, a shorter one is followed by frame cells, so the rest of the row keeps its place.
/// Written characters take the style of the first replaced cell.
fn replace_in(buf: &mut Buffer, from: &str, to: &str) {
    let (w, h) = (buf.area.width as usize, buf.area.height);
    let from: Vec<char> = from.chars().collect();
    let to: Vec<char> = to.chars().collect();
    for y in 0..h {
        let mut row: Vec<Cell> = (0..w as u16).map(|x| buf[(x, y)].clone()).collect();
        let mut x = 0;
        while x + from.len() <= row.len() {
            let hit = from.iter().enumerate().all(|(i, c)| row[x + i].symbol().starts_with(*c));
            if !hit {
                x += 1;
                continue;
            }
            let end = x + from.len();
            let filler = if end < row.len() { row[end].clone() } else { row[x].clone() };
            let written: Vec<Cell> = to
                .iter()
                .map(|c| {
                    let mut cell = row[x].clone();
                    cell.set_char(*c);
                    cell
                })
                .collect();
            let mut tail: Vec<Cell> = row[end..].to_vec();
            if to.len() > from.len() {
                // Take the frame cells after the old text; what is missing shifts the rest of the row right.
                let extra = to.len() - from.len();
                let eat = tail.iter().take_while(|c| c.symbol() == filler.symbol()).count().min(extra);
                tail.drain(..eat);
            } else {
                tail.splice(0..0, std::iter::repeat_n(filler, from.len() - to.len()));
            }
            row.truncate(x);
            row.extend(written.iter().cloned());
            row.extend(tail);
            row.truncate(w);
            x += to.len();
        }
        for (x, cell) in row.into_iter().enumerate() {
            buf[(x as u16, y)] = cell;
        }
    }
}

fn snapshot(app: &mut App, w: u16, h: u16) -> Buffer {
    app.size = (w, h);
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| noble::ui::draw(f, app)).unwrap();
    term.backend().buffer().clone()
}

// ─── Fake data ───────────────────────────────────────────────────────────────

fn demo(theme: &str, w: u16, h: u16) -> App {
    let mut cfg = Config::default();
    cfg.general.animations = false;
    cfg.general.theme = theme.into();
    let mut app = App::headless(cfg, (w, h));
    app.operator = "ada".into();
    // A fixed time, so the images change only when the screens do.
    app.clock = chrono::Local.with_ymd_and_hms(2026, 9, 30, 18, 20, 0).single();
    // Show the screen tips, not the first-run prefix hint.
    app.ui_state.data.prefix_used = true;
    // The image must not depend on which CLIs are installed on this machine.
    for (l, ok) in app.launchers.iter_mut() {
        *ok = matches!(l.command.as_str(), "claude" | "codex");
    }
    // Every agent has its status hooks installed, so their states show in the panes and on Home.
    app.agents_found = Agent::ALL.to_vec();
    app.hooks_on = Agent::ALL.to_vec();
    app.handle(AppEvent::SensorStatic(Box::new(StaticInfo {
        host: "workstation".into(),
        os: "Windows 11 Pro".into(),
        cpu_brand: "AMD Ryzen 9 7940HS".into(),
        cores: 16,
        total_mem: 32 << 30,
    })));
    let procs: Vec<ProcInfo> = [
        ("code.exe", 9.4, 910u64),
        ("cargo.exe", 7.8, 420),
        ("rust-analyzer.exe", 5.1, 1300),
        ("firefox.exe", 3.2, 780),
        ("node.exe", 1.9, 240),
        ("pwsh.exe", 0.9, 96),
        ("noble.exe", 0.3, 22),
        ("explorer.exe", 0.2, 160),
        ("svchost.exe", 0.1, 40),
        ("docker.exe", 0.9, 350),
        ("postgres.exe", 0.6, 180),
        ("slack.exe", 0.5, 420),
        ("WindowsTerminal.exe", 0.4, 120),
        ("spotify.exe", 0.3, 260),
        ("OneDrive.exe", 0.1, 90),
        ("dwm.exe", 1.1, 110),
        ("audiodg.exe", 0.2, 30),
        ("git.exe", 0.1, 12),
        ("ssh-agent.exe", 0.0, 8),
    ]
    .iter()
    .enumerate()
    .map(|(i, (n, c, m))| ProcInfo {
        pid: 2400 + i as u32 * 37,
        started: 0,
        name: n.to_string(),
        cpu: *c,
        mem: m << 20,
    })
    .collect();
    for i in 0..160 {
        let t = i as f32 / 9.0;
        app.handle(AppEvent::Sensors(Box::new(SensorSample {
            cpu: if i == 159 {
                37.0
            } else {
                (30.0 + 16.0 * t.sin() + 9.0 * (t * 2.7).cos() + if i % 23 < 3 { 28.0 } else { 0.0 }).clamp(2.0, 98.0)
            },
            cores: (0..16).map(|c| ((c as f32 * 11.0 + i as f32 * 4.0) % 85.0) + 5.0).collect(),
            freq_mhz: 4200,
            mem_used: (13 << 30) + (i as u64 * 9_000_000),
            mem_total: 32 << 30,
            swap_used: 900_000_000,
            swap_total: 4_000_000_000,
            rx_rate: 1_400_000.0 * (1.0 + (t * 1.3).sin() as f64),
            tx_rate: 180_000.0 * (1.0 + (t * 0.7).cos() as f64),
            disks: vec![
                DiskInfo { mount: "C:\\".into(), total: 1024 << 30, used: 402 << 30 },
                DiskInfo { mount: "D:\\".into(), total: 2048 << 30, used: 1320 << 30 },
            ],
            procs: procs.clone(),
            proc_count: 287,
            uptime: 3 * 3600 + 41 * 60,
            battery: Some(noble::battery::Battery {
                percent: 84.0,
                state: noble::battery::PowerState::Discharging,
                secs_left: Some(4 * 3600 + 20 * 60),
                secs_to_full: None,
            }),
        })));
    }

    let now = SystemTime::now();
    let ts = chrono::Utc::now().timestamp();
    let commit = |hash: &str, ago: i64, subject: &str| Commit {
        hash: hash.into(),
        time: ts - ago,
        author: "ada".into(),
        subject: subject.into(),
    };
    let mk = |name: &str, branch: &str, mins: u64, git: Option<GitInfo>| Project {
        name: name.into(),
        path: PathBuf::from(format!("D:\\dev\\{name}")),
        branch: Some(branch.into()),
        last_active: Some(now - Duration::from_secs(mins * 60)),
        git,
        repo: true,
        tasks: Vec::new(),
        claude: None,
    };
    app.handle(AppEvent::Projects(vec![
        // Its last Claude conversation (the `↻ claude` line) and its quick tasks.
        Project {
            claude: Some(now - Duration::from_secs(2 * 3600 + 14 * 60)),
            tasks: vec!["cargo test".into(), "cargo clippy --all-targets".into(), "cargo fmt".into()],
            ..mk(
                "noble",
                "main",
                2,
                Some(GitInfo {
                    dirty: 4,
                    untracked: 1,
                    ahead: 2,
                    branch: Some("main".into()),
                    last_subject: Some("feat: project card on Home".into()),
                    last_commit: Some(ts - 1500),
                    commits: vec![
                        commit("7c1e9a2", 1500, "feat: project card on Home"),
                        commit("b24f0d8", 3 * 3600, "feat: quick launch for 13 AI CLIs"),
                        commit("e91c3b4", 20 * 3600, "fix: keep git status visible on hover"),
                        commit("5a0d7e1", 2 * 86_400, "perf: redraw only when something changes"),
                        commit("c3f8b62", 4 * 86_400, "docs: keyboard reference"),
                    ],
                    changes: vec![
                        (" M".into(), "src/ui/bridge.rs".into()),
                        (" M".into(), "src/app/settings.rs".into()),
                        ("M ".into(), "README.md".into()),
                        (" D".into(), "docs/old-notes.md".into()),
                        ("??".into(), "examples/screenshots.rs".into()),
                    ],
                    ..Default::default()
                }),
            )
        },
        mk(
            "api-gateway",
            "feature/rate-limit",
            45,
            Some(GitInfo { branch: Some("feature/rate-limit".into()), ..Default::default() }),
        ),
        mk("dotfiles", "main", 60 * 20, Some(GitInfo { dirty: 1, behind: 3, ..Default::default() })),
        mk("website", "main", 60 * 30, Some(GitInfo { ahead: 1, ..Default::default() })),
        mk("ml-notebooks", "exp/lora", 60 * 24 * 3, Some(GitInfo { dirty: 12, untracked: 7, ..Default::default() })),
        mk("infra", "main", 60 * 24 * 12, Some(GitInfo::default())),
        mk("blog", "master", 60 * 24 * 40, None),
    ]));
    // The selected project's card: its pull request with all checks passing, and a dev server port.
    let noble_path = PathBuf::from(r"D:\dev\noble");
    app.prs.insert(
        noble_path.clone(),
        PrEntry {
            branch: Some("main".into()),
            info: Some(PrInfo {
                number: 42,
                title: "Show pull requests and ports on the project card".into(),
                state: PrState::Open,
                url: "https://github.com/MertSoylu/noble/pull/42".into(),
                branch: "main".into(),
                checks: Checks { passed: 12, failed: 0, pending: 0 },
            }),
            fetched: Instant::now(),
        },
    );
    app.ports.insert(noble_path, vec![5173]);
    // Claude Code's background sessions: running, waiting for input, and one finished today.
    app.handle(AppEvent::BgSessions(vec![
        BgSession {
            id: "a1f3c9e2".into(),
            cwd: Some(PathBuf::from(r"D:\dev\noble")),
            name: "fix flaky pane reflow test".into(),
            state: "running".into(),
            started: ts - 1800,
        },
        BgSession {
            id: "b7e20d41".into(),
            cwd: Some(PathBuf::from(r"D:\dev\api-gateway")),
            name: "rate limit headers".into(),
            state: "waiting".into(),
            started: ts - 3600,
        },
        BgSession {
            id: "c04a9f77".into(),
            cwd: Some(PathBuf::from(r"D:\dev\website")),
            name: "update pricing page copy".into(),
            state: "done".into(),
            started: ts - 7200,
        },
    ]));

    let reset = ts + 2 * 3600 + 14 * 60;
    let provider = |id: &'static str, name: &'static str, plan: &str, windows: Vec<Window>| ProviderState {
        id,
        name,
        login_hint: "",
        presence: Presence::Ready,
        status: Status::Ok,
        usage: Some(Usage { windows, plan: Some(plan.into()), note: None }),
        fetched_at: Some(ts - 60),
    };
    let win = |label: &str, used: u8, resets: i64| Window { label: label.into(), used, resets_at: Some(resets) };
    for s in [
        provider("claude", "Claude Code", "MAX", vec![win("5H", 64, reset), win("WEEK", 31, reset + 4 * 86_400)]),
        provider("codex", "Codex", "PLUS", vec![win("5H", 18, reset + 3600), win("WEEK", 47, reset + 2 * 86_400)]),
    ] {
        app.handle(AppEvent::Ai(Box::new(s)));
    }
    app
}

/// A terminal tab with two panes and a second tab in the background. A real
/// shell is opened but its screen content is replaced with fake ANSI output.
fn terminals(app: &mut App) {
    if cfg!(windows) {
        // cmd.exe opens fast and writes nothing to the screen on its own.
        let mut cfg = app.cfg.clone();
        cfg.terminal.shell = "cmd.exe".into();
        app.apply_config(cfg);
    }
    let idle = if cfg!(windows) { "ping -n 30 127.0.0.1 >nul" } else { "sleep 30" };
    let cwd = std::env::temp_dir();
    // Tabs are named explicitly, so the names do not depend on the folder the demo runs in.
    tab_named(app, "api-gateway", &cwd, idle);
    tab_named(app, "noble", &cwd, idle);
    let tab = app.tabs.len() - 1;
    app.run(Action::SplitRight);
    // Two more tabs, each running one agent: their states show as the tab strip's dots.
    tab_named(app, "docs", &cwd, idle);
    tab_named(app, "infra", &cwd, idle);
    // The first draw brings the panes to their final size; since ConPTY redraws
    // the screen when the size changes, the fake content is written afterwards.
    app.view = View::Term(tab);
    let (w, h) = app.size;
    snapshot(app, w, h);
    let deadline = Instant::now() + Duration::from_millis(2500);
    while Instant::now() < deadline {
        app.pump();
        std::thread::sleep(Duration::from_millis(50));
    }
    let ids = app.tabs[tab].panes();
    // Claude Code in the left pane, Codex on the right: content is drawn to the pane's real size.
    let screens: [(&str, Screen); 2] = [("claude", claude_screen), ("codex", codex_screen)];
    for (id, (title, screen)) in ids.iter().zip(screens) {
        let (rows, cols) = noble::term::screen_size(app.panes[id].parser().screen());
        let body = screen(rows, cols);
        let seq = format!("\x1b]0;{title}\x07\x1b[2J\x1b[H{body}");
        app.panes[id].parser().process(seq.as_bytes());
    }
    // Claude Code reports its progress (OSC 9;4): the title shows 42%.
    app.panes[&ids[0]].parser().process(b"\x1b]9;4;1;42\x07");
    if let Some(&first) = app.tabs[0].panes().first() {
        let seq = "\x1b]0;node\x07\x1b[2J\x1b[H";
        app.panes[&first].parser().process(seq.as_bytes());
    }
    // The long command in the background tab finished: the ✓ marker.
    app.tabs[0].alert = Some(noble::term::TabAlert::Done);
    // Hook records as the agents write them: Claude working, Codex waiting for you (its pane is visible, so no
    // notice), Copilot idle at its prompt, OpenCode working. `apply_hook_records` reads them like the real ones.
    let ts = chrono::Utc::now().timestamp();
    let record = |agent: Agent, event: &str| HookRecord {
        event: event.into(),
        message: None,
        ts,
        agent: Some(agent),
        session: None,
        subagents: 0,
    };
    let states = [
        (ids[0], record(Agent::Claude, PROMPT)),
        (ids[1], record(Agent::Codex, NOTIFICATION)),
        (app.tabs[2].focus, record(Agent::Copilot, SESSION_START)),
        (app.tabs[3].focus, record(Agent::OpenCode, PROMPT)),
    ];
    app.apply_hook_records(states.into_iter().collect());
    app.tabs[tab].focus = ids[0];
}

/// Opens a tab running `command` in `cwd` and names it.
fn tab_named(app: &mut App, name: &str, cwd: &Path, command: &str) {
    app.new_tab(cwd.to_path_buf(), Some(command), Some(name.into()));
    if let Some(tab) = app.tabs.last_mut() {
        tab.name = Some(name.into());
    }
}

/// A function producing screen content from a pane size (rows, cols).
type Screen = fn(u16, u16) -> String;

/// 24-bit foreground color.
fn fg(r: u8, g: u8, b: u8) -> String {
    format!("\x1b[38;2;{r};{g};{b}m")
}

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";

/// Writes the top content, places the bottom block at the screen floor and moves
/// the cursor to `cursor` (row, col) inside that bottom block.
fn finish(top: String, bottom: String, rows: u16, cursor: (u16, u16)) -> String {
    let bottom_lines = bottom.lines().count() as u16;
    let at = rows.saturating_sub(bottom_lines) + 1;
    format!(
        "{}\x1b[{at};1H{}\x1b[{};{}H",
        top.replace('\n', "\r\n"),
        bottom.trim_end_matches('\n').replace('\n', "\r\n"),
        at + cursor.0,
        cursor.1
    )
}

/// The project folder and the file shown in the agent screens.
const CWD: &str = r"D:\dev\noble";
const SRC: &str = r"src\config.rs";

/// Claude Code 2.1 at work, as it draws in a pane: the logo header, the request, a finished read, a reply,
/// a running tool call with the spinner below it, and the prompt between two rules.
fn claude_screen(rows: u16, cols: u16) -> String {
    let orange = fg(215, 119, 87);
    let gray = fg(153, 153, 153);
    let rule_c = fg(136, 136, 136);
    let white = fg(255, 255, 255);
    let code = fg(177, 185, 249);
    let amber = fg(255, 193, 7);
    let black_bg = "\x1b[48;2;0;0;0m";
    let msg_bg = "\x1b[48;2;55;55;55m";
    let cols = cols as usize;
    let mut top = String::from("\n");
    // The logo: its eyes are cells with a black background.
    top.push_str(&format!(" {orange}▐{black_bg}▛███▛█{RESET}   {BOLD}Claude Code{RESET} {gray}v2.1.312{RESET}\n"));
    top.push_str(&format!(
        "{orange}▝▜{black_bg}█████{RESET}{orange}█▀{RESET}  {gray}Opus 5.5 with high effort · Claude Max{RESET}\n"
    ));
    top.push_str(&format!(" {orange}▝▝   ▝▝{RESET}   {gray}{CWD}{RESET}\n\n\n"));
    let request = "add opencode to the quick-launch defaults";
    let pad = cols.saturating_sub(request.chars().count() + 3);
    top.push_str(&format!("{msg_bg}{gray}❯ {white}{request}{}{RESET}\n\n", " ".repeat(pad)));
    top.push_str(&format!("  {gray}Read {BOLD}2{RESET}{gray} files{RESET}\n\n"));
    top.push_str(&format!(
        "{white}●{RESET} The defaults live in {code}DEFAULT_LAUNCHERS{RESET} in {code}src/config.rs{RESET}. I'll\n"
    ));
    top.push_str(&format!("  add {code}opencode{RESET} next to {code}codex{RESET} and cover it with a test.\n\n"));
    top.push_str(&format!("  Reading {SRC}\n"));
    top.push_str(&format!("  {gray}⎿  {SRC}{RESET}\n"));
    let rule = format!("{rule_c}{}{RESET}", "─".repeat(cols));
    let bottom = format!(
        "{orange}✽ Meandering…{RESET} {gray}(42s · ↓ 1.2k tokens · thinking with high effort){RESET}\n\n{rule}\n\
         {gray}❯{RESET} \n{rule}\n  {amber}⏵⏵ auto mode on{RESET} {gray}(shift+tab to cycle){RESET}\n"
    );
    finish(top, bottom, rows, (3, 3))
}

/// Codex 0.159 after a turn, as it draws in a pane: the header, the request, the reply with its timing,
/// the composer and the status line.
fn codex_screen(rows: u16, _cols: u16) -> String {
    let dim = "\x1b[2m";
    let blue = fg(99, 168, 248);
    let model = fg(246, 226, 183);
    let path = fg(171, 223, 167);
    let title = fg(245, 194, 231);
    let mut top = String::from("\n");
    top.push_str(&format!("  {blue}>_{RESET} {BOLD}OpenAI Codex{RESET} {dim}(v0.159.0){RESET}\n"));
    top.push_str(&format!("     {dim}{CWD}{RESET}\n\n"));
    top.push_str(&format!("  {blue}Use /review to check your changes before you commit.{RESET}\n\n\n"));
    top.push_str(&format!("{dim}›{RESET} review battery.rs for edge cases in the time estimate\n\n\n"));
    top.push_str(&format!("{dim}•{RESET} Two edge cases in the estimate:\n\n"));
    top.push_str("  1. A charge rate of 0 divides by zero; guard it and fall back\n");
    top.push_str("     to the time the OS reports.\n");
    top.push_str("  2. Readings older than 10 minutes skew the rate; drop them\n");
    top.push_str("     before averaging.\n\n");
    top.push_str("  Should I fix both and add tests for them?\n\n");
    top.push_str(&format!("  {dim}Worked for 38s • 18:19{RESET}\n"));
    let bottom = format!(
        "{BOLD}›{RESET} {dim}Ask Codex to do anything{RESET}\n\n  {model}GPT-6-Luna high{RESET} · {path}{CWD}{RESET} · \
         {title}Battery estimate edge cases{RESET}\n  {BOLD}←{RESET} for agents · {BOLD}?{RESET} for shortcuts\n"
    );
    finish(top, bottom, rows, (0, 3))
}

// ─── SVG ─────────────────────────────────────────────────────────────────────

fn hex(c: (u8, u8, u8)) -> String {
    format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
}

fn rgb_of(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Indexed(i) => xterm256(i),
        Color::Black => xterm256(0),
        Color::Red => xterm256(1),
        Color::Green => xterm256(2),
        Color::Yellow => xterm256(3),
        Color::Blue => xterm256(4),
        Color::Magenta => xterm256(5),
        Color::Cyan => xterm256(6),
        Color::Gray => xterm256(7),
        Color::DarkGray => xterm256(8),
        Color::LightRed => xterm256(9),
        Color::LightGreen => xterm256(10),
        Color::LightYellow => xterm256(11),
        Color::LightBlue => xterm256(12),
        Color::LightMagenta => xterm256(13),
        Color::LightCyan => xterm256(14),
        Color::White => xterm256(15),
        Color::Reset => (0, 0, 0),
    }
}

fn xterm256(i: u8) -> (u8, u8, u8) {
    const BASE: [(u8, u8, u8); 16] = [
        (0x0c, 0x0c, 0x0c),
        (0xc5, 0x0f, 0x1f),
        (0x13, 0xa1, 0x0e),
        (0xc1, 0x9c, 0x00),
        (0x00, 0x37, 0xda),
        (0x88, 0x17, 0x98),
        (0x3a, 0x96, 0xdd),
        (0xcc, 0xcc, 0xcc),
        (0x76, 0x76, 0x76),
        (0xe7, 0x48, 0x56),
        (0x16, 0xc6, 0x0c),
        (0xf9, 0xf1, 0xa5),
        (0x3b, 0x78, 0xff),
        (0xb4, 0x00, 0x9e),
        (0x61, 0xd6, 0xd6),
        (0xf2, 0xf2, 0xf2),
    ];
    match i {
        0..=15 => BASE[i as usize],
        16..=231 => {
            let n = i - 16;
            let v = |x: u8| if x == 0 { 0 } else { 55 + x * 40 };
            (v(n / 36), v((n / 6) % 6), v(n % 6))
        }
        _ => {
            let g = 8 + (i - 232) * 10;
            (g, g, g)
        }
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Box drawing arms (up, right, down, left): 0 none, 1 thin, 2 thick.
fn box_arms(c: char) -> Option<[u8; 4]> {
    Some(match c {
        '─' => [0, 1, 0, 1],
        '━' => [0, 2, 0, 2],
        '│' => [1, 0, 1, 0],
        '┃' => [2, 0, 2, 0],
        '┌' => [0, 1, 1, 0],
        '┐' => [0, 0, 1, 1],
        '└' => [1, 1, 0, 0],
        '┘' => [1, 0, 0, 1],
        '┏' => [0, 2, 2, 0],
        '┓' => [0, 0, 2, 2],
        '┗' => [2, 2, 0, 0],
        '┛' => [2, 0, 0, 2],
        '├' => [1, 1, 1, 0],
        '┤' => [1, 0, 1, 1],
        '┬' => [0, 1, 1, 1],
        '┴' => [1, 1, 0, 1],
        '┼' => [1, 1, 1, 1],
        '┣' => [2, 2, 2, 0],
        '┫' => [2, 0, 2, 2],
        '┳' => [0, 2, 2, 2],
        '┻' => [2, 2, 0, 2],
        '╋' => [2, 2, 2, 2],
        '╴' => [0, 0, 0, 1],
        '╵' => [1, 0, 0, 0],
        '╶' => [0, 1, 0, 0],
        '╷' => [0, 0, 1, 0],
        '╸' => [0, 0, 0, 2],
        '╹' => [2, 0, 0, 0],
        '╺' => [0, 2, 0, 0],
        '╻' => [0, 0, 2, 0],
        _ => return None,
    })
}

/// Appends an SVG piece when the character can be drawn as a vector.
fn draw_glyph(out: &mut String, c: char, x: f64, y: f64, color: &str) -> bool {
    let (cx, cy) = (x + CW / 2.0, y + CH / 2.0);
    let stroke = |weight: u8| if weight == 2 { 2.6 } else { 1.2 };
    if let Some(arms) = box_arms(c) {
        let ends = [(cx, y), (x + CW, cy), (cx, y + CH), (x, cy)];
        for (arm, (ex, ey)) in arms.iter().zip(ends) {
            if *arm > 0 {
                let _ = write!(
                    out,
                    r#"<line x1="{cx:.1}" y1="{cy:.1}" x2="{ex:.1}" y2="{ey:.1}" stroke="{color}" stroke-width="{}" stroke-linecap="square"/>"#,
                    stroke(*arm)
                );
            }
        }
        return true;
    }
    // Rounded corners: two arms and the arc between them.
    let r = CW / 2.0;
    let rounded = match c {
        '╭' => {
            Some(format!("M{cx:.1},{:.1} V{:.1} A{r},{r} 0 0 1 {:.1},{cy:.1} H{:.1}", y + CH, cy + r, cx + r, x + CW))
        }
        '╮' => Some(format!("M{x:.1},{cy:.1} H{:.1} A{r},{r} 0 0 1 {cx:.1},{:.1} V{:.1}", cx - r, cy + r, y + CH)),
        '╯' => Some(format!("M{cx:.1},{y:.1} V{:.1} A{r},{r} 0 0 1 {:.1},{cy:.1} H{x:.1}", cy - r, cx - r)),
        '╰' => Some(format!("M{cx:.1},{y:.1} V{:.1} A{r},{r} 0 0 0 {:.1},{cy:.1} H{:.1}", cy - r, cx + r, x + CW)),
        _ => None,
    };
    if let Some(d) = rounded {
        let _ = write!(out, r#"<path d="{d}" fill="none" stroke="{color}" stroke-width="1.2"/>"#);
        return true;
    }
    let rect = |out: &mut String, rx: f64, ry: f64, rw: f64, rh: f64| {
        let _ = write!(out, r#"<rect x="{rx:.2}" y="{ry:.2}" width="{rw:.2}" height="{rh:.2}" fill="{color}"/>"#);
    };
    let code = c as u32;
    match c {
        '█' => rect(out, x, y, CW, CH),
        '▀' => rect(out, x, y, CW, CH / 2.0),
        '▌' => rect(out, x, y, CW / 2.0, CH),
        '▐' => rect(out, x + CW / 2.0, y, CW / 2.0, CH),
        '▔' => rect(out, x, y, CW, CH / 8.0),
        '▕' => rect(out, x + CW * 7.0 / 8.0, y, CW / 8.0, CH),
        // ▁▂▃▄▅▆▇: filled from the bottom in eighth steps.
        _ if (0x2581..=0x2587).contains(&code) => {
            let hh = CH * (code - 0x2580) as f64 / 8.0;
            rect(out, x, y + CH - hh, CW, hh);
        }
        // ▉▊▋▍▎▏: filled from the left.
        _ if (0x2589..=0x258f).contains(&code) => rect(out, x, y, CW * (0x2590 - code) as f64 / 8.0, CH),
        '░' | '▒' | '▓' => {
            let op = match c {
                '░' => 0.25,
                '▒' => 0.5,
                _ => 0.75,
            };
            let _ = write!(
                out,
                r#"<rect x="{x:.2}" y="{y:.2}" width="{CW}" height="{CH}" fill="{color}" fill-opacity="{op}"/>"#
            );
        }
        // Quarter blocks ▖▗▘▙▚▛▜▝▞▟: (top left, top right, bottom left, bottom right).
        _ if (0x2596..=0x259f).contains(&code) => {
            const Q: [[bool; 4]; 10] = [
                [false, false, true, false],
                [false, false, false, true],
                [true, false, false, false],
                [true, false, true, true],
                [true, false, false, true],
                [true, true, true, false],
                [true, true, false, true],
                [false, true, false, false],
                [false, true, true, false],
                [false, true, true, true],
            ];
            let q = Q[(code - 0x2596) as usize];
            let (hw, hh) = (CW / 2.0, CH / 2.0);
            for (i, on) in q.iter().enumerate() {
                if *on {
                    rect(out, x + hw * (i % 2) as f64, y + hh * (i / 2) as f64, hw, hh);
                }
            }
        }
        // Braille: 2×4 dots.
        _ if (0x2801..=0x28ff).contains(&code) => {
            let bits = code - 0x2800;
            const DOTS: [(u32, f64, f64); 8] = [
                (0, 0., 0.),
                (1, 0., 1.),
                (2, 0., 2.),
                (3, 1., 0.),
                (4, 1., 1.),
                (5, 1., 2.),
                (6, 0., 3.),
                (7, 1., 3.),
            ];
            for (bit, col, row) in DOTS {
                if bits & (1 << bit) != 0 {
                    let dx = x + CW * (0.28 + 0.44 * col);
                    let dy = y + CH * (0.16 + 0.225 * row);
                    let _ = write!(out, r#"<circle cx="{dx:.2}" cy="{dy:.2}" r="1.35" fill="{color}"/>"#);
                }
            }
        }
        _ => return false,
    }
    true
}

/// Turns the buffer into the SVG body (0,0 origin, `w*CW × h*CH`).
fn buffer_svg(buf: &Buffer, bg: (u8, u8, u8), fg: (u8, u8, u8)) -> String {
    let area = buf.area;
    let mut back = String::new();
    let mut shapes = String::new();
    let mut text = String::new();
    let resolve = |c: Color, default: (u8, u8, u8)| if c == Color::Reset { default } else { rgb_of(c) };
    for y in 0..area.height {
        let py = y as f64 * CH;
        // Background: consecutive cells of the same color become one rectangle.
        let mut run: Option<(u16, (u8, u8, u8))> = None;
        let flush = |back: &mut String, start: u16, end: u16, color: (u8, u8, u8)| {
            if color != bg {
                let _ = write!(
                    back,
                    r#"<rect x="{:.1}" y="{py:.1}" width="{:.1}" height="{CH}" fill="{}"/>"#,
                    start as f64 * CW,
                    (end - start) as f64 * CW + 0.3,
                    hex(color)
                );
            }
        };
        for x in 0..area.width {
            let cell = &buf[(x, y)];
            let rev = cell.modifier.contains(Modifier::REVERSED);
            let cbg = if rev { resolve(cell.fg, fg) } else { resolve(cell.bg, bg) };
            match run {
                Some((_, c)) if c == cbg => {}
                Some((s, c)) => {
                    flush(&mut back, s, x, c);
                    run = Some((x, cbg));
                }
                None => run = Some((x, cbg)),
            }
        }
        if let Some((s, c)) = run {
            flush(&mut back, s, area.width, c);
        }

        // Text: consecutive same-style characters become one <text>, each in its own cell.
        let mut x = 0;
        while x < area.width {
            let cell = &buf[(x, y)];
            let sym = cell.symbol();
            let width = unicode_width::UnicodeWidthStr::width(sym).max(1) as u16;
            let rev = cell.modifier.contains(Modifier::REVERSED);
            let mut color = if rev { resolve(cell.bg, bg) } else { resolve(cell.fg, fg) };
            if cell.modifier.contains(Modifier::DIM) {
                color = dim(color, bg);
            }
            let chex = hex(color);
            let px = x as f64 * CW;
            let first = sym.chars().next().unwrap_or(' ');
            if sym.trim().is_empty() || draw_glyph(&mut shapes, first, px, py, &chex) {
                x += width;
                continue;
            }
            // Collect neighbors with the same style.
            let bold = cell.modifier.contains(Modifier::BOLD);
            let italic = cell.modifier.contains(Modifier::ITALIC);
            let under = cell.modifier.contains(Modifier::UNDERLINED);
            let mut chars = String::new();
            let mut xs = Vec::new();
            let mut cx = x;
            while cx < area.width {
                let c2 = &buf[(cx, y)];
                let s2 = c2.symbol();
                let w2 = unicode_width::UnicodeWidthStr::width(s2).max(1) as u16;
                let rev2 = c2.modifier.contains(Modifier::REVERSED);
                let mut col2 = if rev2 { resolve(c2.bg, bg) } else { resolve(c2.fg, fg) };
                if c2.modifier.contains(Modifier::DIM) {
                    col2 = dim(col2, bg);
                }
                let f2 = s2.chars().next().unwrap_or(' ');
                let same = c2.modifier == cell.modifier && (col2 == color || s2.trim().is_empty());
                if !same || (!s2.trim().is_empty() && (box_arms(f2).is_some() || is_graphic(f2))) {
                    break;
                }
                if !s2.trim().is_empty() {
                    chars.push_str(&esc(s2));
                    // For combining characters a single position is enough.
                    xs.push(format!("{:.1}", cx as f64 * CW));
                    for _ in 1..s2.chars().count() {
                        xs.push(format!("{:.1}", cx as f64 * CW));
                    }
                }
                cx += w2;
            }
            let mut attrs = String::new();
            if bold {
                attrs.push_str(r#" font-weight="700""#);
            }
            if italic {
                attrs.push_str(r#" font-style="italic""#);
            }
            if under {
                attrs.push_str(r#" text-decoration="underline""#);
            }
            let _ = write!(
                text,
                r#"<text x="{}" y="{:.1}" fill="{chex}"{attrs}>{chars}</text>"#,
                xs.join(" "),
                py + CH * 0.74
            );
            x = cx.max(x + 1);
        }
    }
    format!(
        r#"<rect width="{:.1}" height="{:.1}" fill="{}"/>{back}{shapes}<g font-family="{FONT}" font-size="14" xml:space="preserve">{text}</g>"#,
        area.width as f64 * CW,
        area.height as f64 * CH,
        hex(bg)
    )
}

/// Dim text: halfway between its color and the background.
fn dim(c: (u8, u8, u8), bg: (u8, u8, u8)) -> (u8, u8, u8) {
    let mix = |a: u8, b: u8| ((a as u16 + b as u16) / 2) as u8;
    (mix(c.0, bg.0), mix(c.1, bg.1), mix(c.2, bg.2))
}

fn is_graphic(c: char) -> bool {
    let code = c as u32;
    matches!(c, '╭' | '╮' | '╯' | '╰') || (0x2580..=0x259f).contains(&code) || (0x2801..=0x28ff).contains(&code)
}

/// Wraps the frame into a full SVG document as a window with a title bar and shadow.
fn window_svg(buf: &Buffer, app: &App, title: &str) -> String {
    let th = &app.theme;
    let bg = rgb_of(th.bg);
    let fg = rgb_of(th.fg);
    let bar = rgb_of(th.raised);
    let dim = hex(rgb_of(th.dim));
    let line = hex(rgb_of(th.line));
    let (bw, bh) = (buf.area.width as f64 * CW, buf.area.height as f64 * CH);
    let (pad, top, margin) = (14.0, 34.0, 24.0);
    let (ww, wh) = (bw + pad * 2.0, bh + pad + top);
    let (tw, tht) = (ww + margin * 2.0, wh + margin * 2.0);
    let body = buffer_svg(buf, bg, fg);
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{tw:.0}" height="{tht:.0}" viewBox="0 0 {tw:.1} {tht:.1}" role="img" aria-label="NOBLE {title}">
<defs><filter id="s" x="-10%" y="-10%" width="120%" height="130%"><feDropShadow dx="0" dy="8" stdDeviation="10" flood-color="#000" flood-opacity="0.35"/></filter>
<clipPath id="c"><rect width="{bw:.1}" height="{bh:.1}"/></clipPath></defs>
<g transform="translate({margin},{margin})">
<rect width="{ww:.1}" height="{wh:.1}" rx="10" fill="{}" stroke="{line}" filter="url(#s)"/>
<path d="M0,10 a10,10 0 0 1 10,-10 H{:.1} a10,10 0 0 1 10,10 V{top} H0 Z" fill="{}"/>
<circle cx="20" cy="17" r="6" fill="#ff5f57"/><circle cx="40" cy="17" r="6" fill="#febc2e"/><circle cx="60" cy="17" r="6" fill="#28c840"/>
<text x="{:.1}" y="22" text-anchor="middle" font-family="{FONT}" font-size="13" fill="{dim}">{}</text>
<g transform="translate({pad},{top})" clip-path="url(#c)">{body}</g>
</g></svg>
"##,
        hex(bg),
        ww - 10.0,
        hex(bar),
        ww / 2.0,
        esc(title),
    )
}

/// Combines the windows in a grid into a single SVG (scaled with nested <svg>).
fn grid_svg(tiles: &[(String, String)], cols: usize) -> String {
    let dims = |svg: &str| -> (f64, f64) {
        let get = |key: &str| -> f64 {
            let start = svg.find(&format!("{key}=\"")).map(|i| i + key.len() + 2).unwrap_or(0);
            svg[start..].split('"').next().and_then(|v| v.parse().ok()).unwrap_or(0.0)
        };
        (get("width"), get("height"))
    };
    let (tw, th) = tiles.first().map(|(s, _)| dims(s)).unwrap_or((0.0, 0.0));
    let scale = 0.5;
    let (cw, ch) = (tw * scale, th * scale);
    let rows = tiles.len().div_ceil(cols);
    let (w, h) = (cw * cols as f64, ch * rows as f64);
    let mut out = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w:.0}" height="{h:.0}" viewBox="0 0 {w:.1} {h:.1}" role="img" aria-label="NOBLE themes">"#
    );
    for (i, (svg, _)) in tiles.iter().enumerate() {
        let (x, y) = ((i % cols) as f64 * cw, (i / cols) as f64 * ch);
        // So each tile's filter/clip ids do not collide.
        let inner = svg
            .replacen("<svg ", &format!(r#"<svg x="{x:.1}" y="{y:.1}" "#), 1)
            .replacen(
                &format!(r#"width="{tw:.0}" height="{th:.0}""#),
                &format!(r#"width="{cw:.1}" height="{ch:.1}""#),
                1,
            )
            .replace("id=\"s\"", &format!("id=\"s{i}\""))
            .replace("url(#s)", &format!("url(#s{i})"))
            .replace("id=\"c\"", &format!("id=\"c{i}\""))
            .replace("url(#c)", &format!("url(#c{i})"));
        out.push_str(&inner);
    }
    out.push_str("</svg>\n");
    out
}
