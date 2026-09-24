//! NOBLE giriş noktası: terminal kurulumu, girdi iş parçacığı ve kare döngüsü.

use std::io::{Write, stdout};
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use noble::app::App;
use noble::config::Paths;
use noble::event::AppEvent;

const FRAME: Duration = Duration::from_millis(16);
/// Olay yokken en uzun uyku: config ve hook denetimleri bu aralıkla yapılır.
const IDLE_TICK: Duration = Duration::from_secs(2);

fn usage() {
    println!(
        "NOBLE {} — retro-futurist HUD terminal workspace\n\n\
         USAGE: noble [--config <path>] [--no-boot]\n\n\
         OPTIONS:\n  \
           --config <path>   use an alternate config file\n  \
           --no-boot         skip the boot sequence\n  \
           --paths           print config and data locations\n  \
           -V, --version     print version\n  \
           -h, --help        print this help",
        env!("CARGO_PKG_VERSION")
    );
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let mut out = stdout();
    #[cfg(not(windows))]
    let _ = execute!(out, crossterm::event::DisableBracketedPaste);
    let _ = execute!(out, DisableMouseCapture, LeaveAlternateScreen, crossterm::cursor::Show);
    let _ = out.flush();
}

fn main() -> anyhow::Result<()> {
    // `noble hook <olay>`: Claude Code hook'undan çağrılır; terminale dokunmaz,
    // her durumda sessizce başarıyla çıkar.
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.first().map(String::as_str) == Some("hook") {
        use std::io::Read;
        let mut input = String::new();
        let _ = std::io::stdin().take(1 << 20).read_to_string(&mut input);
        let event = argv.get(1).map(String::as_str).unwrap_or("");
        let data = Paths::resolve(None).data;
        let (instance, pane) = (std::env::var("NOBLE_INSTANCE").ok(), std::env::var("NOBLE_PANE").ok());
        noble::hooks::run_cli(event, &input, &data, instance.as_deref(), pane.as_deref());
        return Ok(());
    }
    let mut config_override: Option<PathBuf> = None;
    let mut no_boot = false;
    let mut args = argv.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                usage();
                return Ok(());
            }
            "-V" | "--version" => {
                let dev = if noble::util::is_dev_build() { " (dev build)" } else { "" };
                println!("noble {}{dev}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--config" => match args.next() {
                Some(p) => config_override = Some(PathBuf::from(p)),
                None => anyhow::bail!("--config needs a path"),
            },
            "--no-boot" => no_boot = true,
            "--paths" => {
                let p = Paths::resolve(config_override.clone());
                println!("config: {}\ndata:   {}", p.config.display(), p.data.display());
                return Ok(());
            }
            other => anyhow::bail!("unknown argument '{other}' (try --help)"),
        }
    }
    let paths = Paths::resolve(config_override);

    enable_raw_mode()?;
    let mut out = stdout();
    execute!(
        out,
        EnterAlternateScreen,
        EnableMouseCapture,
        crossterm::terminal::SetTitle(if noble::util::is_dev_build() { "NOBLE dev" } else { "NOBLE" })
    )?;
    #[cfg(not(windows))]
    execute!(out, crossterm::event::EnableBracketedPaste)?;
    // Ana iş parçacığı paniklerse terminali geri yükle; arka plan panikleri
    // ekranı bozmasın diye yalnızca log dosyasına yazılır.
    let default_hook = std::panic::take_hook();
    let log_file = paths.data_file("noble.log");
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().name() == Some("main") {
            restore_terminal();
            default_hook(info);
        } else {
            let _ = std::fs::create_dir_all(log_file.parent().unwrap_or(std::path::Path::new(".")));
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log_file) {
                let thread = std::thread::current().name().unwrap_or("?").to_string();
                let _ = writeln!(f, "[{}] panic in thread '{thread}': {info}", chrono::Local::now().format("%F %T"));
            }
        }
    }));

    let result = run(paths, no_boot);
    restore_terminal();
    result
}

fn run(paths: Paths, no_boot: bool) -> anyhow::Result<()> {
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    terminal.clear()?;
    let (tx, rx) = mpsc::channel::<AppEvent>();
    {
        let tx = tx.clone();
        std::thread::Builder::new().name("input".into()).spawn(move || {
            while let Ok(ev) = event::read() {
                if tx.send(AppEvent::Input(ev)).is_err() {
                    break;
                }
            }
        })?;
    }
    let size = terminal.size()?;
    let mut app = App::start(paths, tx, (size.width, size.height));
    if no_boot {
        app.boot = None;
    }

    // Çizim yalnızca iki durumda yapılır: görünen bir şeyi değiştiren bir olay
    // geldiğinde (`pending`, en fazla FRAME sıklığında) ya da ekranda zamanla
    // değişen bir şeyin (saat, animasyon, bildirim süresi) anı geldiğinde.
    // Aksi halde süreç uyur; boştaki terminal dakikada bir çizilir.
    let mut last_draw = Instant::now() - FRAME;
    let mut pending = true;
    loop {
        let timed = app.redraw_after().map(|d| Instant::now() + d);
        let wake = if pending { Some(last_draw + FRAME) } else { timed };
        let wait = wake.map(|w| w.saturating_duration_since(Instant::now())).unwrap_or(IDLE_TICK).min(IDLE_TICK);
        match rx.recv_timeout(wait) {
            Ok(ev) => pending |= app.handle(ev),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        // Birikmiş olayları topla ama çizimi aç bırakma.
        let drain_start = Instant::now();
        while drain_start.elapsed() < Duration::from_millis(8) {
            match rx.try_recv() {
                Ok(ev) => pending |= app.handle(ev),
                Err(_) => break,
            }
        }
        app.tick();
        pending |= app.take_dirty();
        if app.quit {
            break;
        }
        let due = pending || timed.is_some_and(|t| Instant::now() >= t);
        if due && last_draw.elapsed() < FRAME {
            // Kare sınırı: bir sonraki turda çizilir.
            pending = true;
        } else if due {
            terminal.draw(|f| noble::ui::draw(f, &mut app))?;
            if std::mem::take(&mut app.outer_bell) {
                // Dış terminale zil: Windows Terminal sekmeyi işaretler, görev çubuğu yanıp söner.
                let mut out = stdout();
                let _ = out.write_all(b"\x07");
                let _ = out.flush();
            }
            last_draw = Instant::now();
            pending = false;
        }
    }
    app.shutdown();
    Ok(())
}
