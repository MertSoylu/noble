//! README ekran görüntülerini üretir: uygulama başsız (headless) kurulur, sahte
//! verilerle doldurulur ve her kare renkli bir SVG'ye dönüştürülür.
//!
//! Çalıştır: `cargo run --example screenshots` → `docs/assets/*.svg`
//!
//! Kutu çizgileri, blok ve braille karakterleri yazı tipine bırakılmaz, vektör
//! olarak çizilir; böylece görüntü her tarayıcıda ve yazı tipinde aynı görünür.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

use noble::ai::{Presence, ProviderState, Status, Usage, Window};
use noble::app::{App, View};
use noble::config::Config;
use noble::event::AppEvent;
use noble::keys::Action;
use noble::projects::{Commit, GitInfo, Project};
use noble::sensors::{DiskInfo, ProcInfo, SensorSample, StaticInfo};
use noble::theme::THEMES;

/// Hücre boyutu (piksel) ve yazı tipi.
const CW: f64 = 8.4;
const CH: f64 = 18.0;
const FONT: &str = "ui-monospace,'Cascadia Mono','SF Mono',Menlo,Consolas,'DejaVu Sans Mono',monospace";

fn main() {
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs").join("assets");
    std::fs::create_dir_all(&out).expect("docs/assets oluşturulamadı");

    // Ana görsel: Home.
    let (w, h) = (150, 42);
    let mut app = demo("amber", w, h);
    write(&out, "home", &window_svg(&snapshot(&mut app, w, h), &app, "noble"));

    // System ekranı.
    let mut app = demo("amber", w, h);
    app.run(Action::System);
    write(&out, "system", &window_svg(&snapshot(&mut app, w, h), &app, "noble — system"));

    // Settings ekranı.
    let (sw, sh) = (120, 40);
    let mut app = demo("amber", sw, sh);
    app.view = View::Settings;
    write(&out, "settings", &window_svg(&snapshot(&mut app, sw, sh), &app, "noble — settings"));

    // Komut paleti.
    let (pw, ph) = (120, 34);
    let mut app = demo("amber", pw, ph);
    app.run(Action::Palette);
    write(&out, "palette", &window_svg(&snapshot(&mut app, pw, ph), &app, "noble — command palette"));

    // Terminaller: bölünmüş iki pane, sahte içerik.
    let mut app = demo("amber", w, h);
    terminals(&mut app);
    let mut buf = snapshot(&mut app, w, h);
    // Pane başlığındaki gerçek (geçici) klasör yerine sahte proje yolu.
    let tmp = noble::util::tilde(&std::env::temp_dir());
    replace_in(&mut buf, &format!(" {tmp} "), r" D:\dev\noble ");
    replace_in(&mut buf, &format!(" {} ", tmp.trim_end_matches(['\\', '/'])), r" D:\dev\noble ");
    write(&out, "terminals", &window_svg(&buf, &app, "noble — terminals"));
    app.panes.clear();

    // Tema galerisi: aynı Home ekranı altı temada.
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
    std::fs::write(&path, svg).expect("svg yazılamadı");
    println!("  {} ({} KB)", path.display(), svg.len() / 1024);
}

/// Buffer'da bir metni daha kısa bir metinle değiştirir; artan hücreler
/// eski metnin hemen sağındaki hücreyle (çerçeve çizgisi) doldurulur.
fn replace_in(buf: &mut Buffer, from: &str, to: &str) {
    let (w, h) = (buf.area.width, buf.area.height);
    let from: Vec<char> = from.chars().collect();
    let to: Vec<char> = to.chars().collect();
    for y in 0..h {
        let mut x = 0;
        while x + from.len() as u16 <= w {
            let hit = from.iter().enumerate().all(|(i, c)| buf[(x + i as u16, y)].symbol().starts_with(*c));
            if !hit {
                x += 1;
                continue;
            }
            let end = x + from.len() as u16;
            let filler = if end < w { buf[(end, y)].clone() } else { buf[(x, y)].clone() };
            for i in 0..from.len() as u16 {
                let cell = &mut buf[(x + i, y)];
                match to.get(i as usize) {
                    Some(c) => {
                        cell.set_char(*c);
                    }
                    None => *cell = filler.clone(),
                }
            }
            x = end;
        }
    }
}

fn snapshot(app: &mut App, w: u16, h: u16) -> Buffer {
    app.size = (w, h);
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| noble::ui::draw(f, app)).unwrap();
    term.backend().buffer().clone()
}

// ─── Sahte veri ──────────────────────────────────────────────────────────────

fn demo(theme: &str, w: u16, h: u16) -> App {
    let mut cfg = Config::default();
    cfg.general.animations = false;
    cfg.general.theme = theme.into();
    let mut app = App::headless(cfg, (w, h));
    app.operator = "ada".into();
    // Görüntü bu makinede hangi CLI'ların kurulu olduğuna bağlı olmasın.
    for (l, ok) in app.launchers.iter_mut() {
        *ok = matches!(l.command.as_str(), "claude" | "codex");
    }
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
    .map(|(i, (n, c, m))| ProcInfo { pid: 2400 + i as u32 * 37, name: n.to_string(), cpu: *c, mem: m << 20 })
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
    };
    app.handle(AppEvent::Projects(vec![
        mk(
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
        ),
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

/// İki pane'li bir terminal sekmesi ve arka planda ikinci bir sekme. Gerçek bir
/// kabuk açılır ama ekran içeriği sahte ANSI çıktısıyla değiştirilir.
fn terminals(app: &mut App) {
    if cfg!(windows) {
        // cmd.exe hızlı açılır ve kendi başına ekrana bir şey yazmaz.
        let mut cfg = app.cfg.clone();
        cfg.terminal.shell = "cmd.exe".into();
        app.apply_config(cfg);
    }
    let idle = if cfg!(windows) { "ping -n 30 127.0.0.1 >nul" } else { "sleep 30" };
    let cwd = std::env::temp_dir();
    app.new_tab(cwd.clone(), Some(idle), Some("api-gateway".into()));
    app.new_tab(cwd, Some(idle), Some("noble".into()));
    app.run(Action::SplitRight);
    // İlk çizim pane'leri son boyutlarına getirir; ConPTY boyut değişince ekranı
    // yeniden çizdiği için sahte içerik bundan sonra yazılır.
    let (w, h) = app.size;
    snapshot(app, w, h);
    let deadline = Instant::now() + Duration::from_millis(2500);
    while Instant::now() < deadline {
        app.pump();
        std::thread::sleep(Duration::from_millis(50));
    }
    let tab = app.tabs.len() - 1;
    let ids = app.tabs[tab].panes();
    let left = LEFT.replace('\n', "\r\n");
    let right = RIGHT.replace('\n', "\r\n");
    for (id, (title, cwd, body)) in ids.iter().zip([
        ("cargo", "file://workstation/D:/dev/noble", left.as_str()),
        ("git", "file://workstation/D:/dev/noble", right.as_str()),
    ]) {
        let seq = format!("\x1b]0;{title}\x07\x1b]7;{cwd}\x07\x1b[2J\x1b[H{body}");
        app.panes[id].parser().process(seq.as_bytes());
    }
    if let Some(&first) = app.tabs[0].panes().first() {
        let seq = "\x1b]0;node\x07\x1b]7;file://workstation/D:/dev/api-gateway\x07\x1b[2J\x1b[H";
        app.panes[&first].parser().process(seq.as_bytes());
    }
    // Arka plan sekmesinde uzun komut bitti: ◆ işareti.
    app.tabs[0].alert = true;
    app.tabs[tab].focus = ids[0];
}

const LEFT: &str = "\x1b[1;36mD:\\dev\\noble\x1b[0m on \x1b[1;35m main\x1b[0m \x1b[31m[!+?]\x1b[0m
\x1b[1;32m❯\x1b[0m cargo test
\x1b[1;32m   Compiling\x1b[0m noble v1.0.0 (D:\\dev\\noble)
\x1b[1;32m    Finished\x1b[0m `test` profile [unoptimized + debuginfo] target(s) in 6.84s
\x1b[1;32m     Running\x1b[0m unittests src\\lib.rs

running 72 tests
test ai::json::tests::labels_normalize ... \x1b[32mok\x1b[0m
test ai::providers::tests::claude_payload ... \x1b[32mok\x1b[0m
test ai::providers::tests::codex_payload ... \x1b[32mok\x1b[0m
test config::tests::default_template_parses ... \x1b[32mok\x1b[0m
test projects::tests::status_parsing ... \x1b[32mok\x1b[0m
test term::layout::tests::split_and_close ... \x1b[32mok\x1b[0m
test term::link::tests::file_line_detection ... \x1b[32mok\x1b[0m

test result: \x1b[32mok\x1b[0m. 72 passed; 0 failed; 3 ignored

\x1b[1;32m     Running\x1b[0m tests\\render.rs
running 35 tests
test bridge_renders_at_all_sizes ... \x1b[32mok\x1b[0m
test real_terminal_session ... \x1b[32mok\x1b[0m
test terminal_compatibility_basics ... \x1b[32mok\x1b[0m

test result: \x1b[32mok\x1b[0m. 34 passed; 0 failed; 1 ignored

\x1b[1;36mD:\\dev\\noble\x1b[0m on \x1b[1;35m main\x1b[0m took \x1b[33m14s\x1b[0m
\x1b[1;32m❯\x1b[0m ";

const RIGHT: &str = "\x1b[1;32m❯\x1b[0m git log --oneline --graph
\x1b[31m*\x1b[0m \x1b[33m7c1e9a2\x1b[0m \x1b[1;36m(HEAD -> main)\x1b[0m feat: project card on Home
\x1b[31m*\x1b[0m \x1b[33mb24f0d8\x1b[0m feat: quick launch for 13 AI CLIs
\x1b[31m*\x1b[0m \x1b[33me91c3b4\x1b[0m fix: keep git status visible on hover
\x1b[31m*\x1b[0m   \x1b[33md40a1f3\x1b[0m Merge branch 'battery'
\x1b[31m|\x1b[32m\\\x1b[0m
\x1b[31m|\x1b[0m \x1b[32m*\x1b[0m \x1b[33m9b7e2c0\x1b[0m feat: battery estimate
\x1b[31m|\x1b[0m \x1b[32m*\x1b[0m \x1b[33m18f6a3d\x1b[0m feat: battery sensor
\x1b[31m|\x1b[32m/\x1b[0m
\x1b[31m*\x1b[0m \x1b[33m5a0d7e1\x1b[0m perf: redraw only when something changes
\x1b[31m*\x1b[0m \x1b[33mc3f8b62\x1b[0m docs: keyboard reference

\x1b[1;32m❯\x1b[0m git status -sb
\x1b[32m## main...origin/main\x1b[0m [ahead \x1b[32m2\x1b[0m]
 \x1b[31mM\x1b[0m src/app/settings.rs
 \x1b[31mM\x1b[0m src/ui/bridge.rs
\x1b[32mM\x1b[0m  README.md
 \x1b[31mD\x1b[0m docs/old-notes.md
\x1b[31m??\x1b[0m examples/screenshots.rs

\x1b[1;32m❯\x1b[0m ";

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

/// Kutu çizgisi kolları (yukarı, sağ, aşağı, sol): 0 yok, 1 ince, 2 kalın.
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

/// Karakter vektör olarak çizilebiliyorsa SVG parçasını ekler.
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
    // Yuvarlak köşeler: iki kol ve aradaki yay.
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
        // ▁▂▃▄▅▆▇: alttan sekizde bir adımlarla dolu.
        _ if (0x2581..=0x2587).contains(&code) => {
            let hh = CH * (code - 0x2580) as f64 / 8.0;
            rect(out, x, y + CH - hh, CW, hh);
        }
        // ▉▊▋▍▎▏: soldan dolu.
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
        // Çeyrek bloklar ▖▗▘▙▚▛▜▝▞▟: (sol üst, sağ üst, sol alt, sağ alt).
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
        // Braille: 2×4 nokta.
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

/// Buffer'ı SVG gövdesine çevirir (0,0 kökenli, `w*CW × h*CH`).
fn buffer_svg(buf: &Buffer, bg: (u8, u8, u8), fg: (u8, u8, u8)) -> String {
    let area = buf.area;
    let mut back = String::new();
    let mut shapes = String::new();
    let mut text = String::new();
    let resolve = |c: Color, default: (u8, u8, u8)| if c == Color::Reset { default } else { rgb_of(c) };
    for y in 0..area.height {
        let py = y as f64 * CH;
        // Zemin: aynı renkteki ardışık hücreler tek dikdörtgen.
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

        // Metin: aynı stildeki ardışık karakterler tek <text>, her biri kendi hücresinde.
        let mut x = 0;
        while x < area.width {
            let cell = &buf[(x, y)];
            let sym = cell.symbol();
            let width = unicode_width::UnicodeWidthStr::width(sym).max(1) as u16;
            let rev = cell.modifier.contains(Modifier::REVERSED);
            let mut color = if rev { resolve(cell.bg, bg) } else { resolve(cell.fg, fg) };
            if cell.modifier.contains(Modifier::DIM) {
                let mix = |a: u8, b: u8| ((a as u16 + b as u16) / 2) as u8;
                color = (mix(color.0, bg.0), mix(color.1, bg.1), mix(color.2, bg.2));
            }
            let chex = hex(color);
            let px = x as f64 * CW;
            let first = sym.chars().next().unwrap_or(' ');
            if sym.trim().is_empty() || draw_glyph(&mut shapes, first, px, py, &chex) {
                x += width;
                continue;
            }
            // Aynı stildeki komşuları topla.
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
                let col2 = if rev2 { resolve(c2.bg, bg) } else { resolve(c2.fg, fg) };
                let f2 = s2.chars().next().unwrap_or(' ');
                let same = c2.modifier == cell.modifier && (col2 == color || s2.trim().is_empty());
                if !same || (!s2.trim().is_empty() && (box_arms(f2).is_some() || is_graphic(f2))) {
                    break;
                }
                if !s2.trim().is_empty() {
                    chars.push_str(&esc(s2));
                    // Birleşik karakterlerde tek konum yeter.
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

fn is_graphic(c: char) -> bool {
    let code = c as u32;
    matches!(c, '╭' | '╮' | '╯' | '╰') || (0x2580..=0x259f).contains(&code) || (0x2801..=0x28ff).contains(&code)
}

/// Kareyi başlık çubuklu, gölgeli bir pencere olarak tam SVG belgesine sarar.
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

/// Pencereleri ızgara hâlinde tek SVG'de birleştirir (iç içe <svg> ile ölçekli).
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
        // Her karonun filtre/kırpma kimlikleri çakışmasın.
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
