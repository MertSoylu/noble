//! Uçtan uca: derlenmiş `noble` gerçek bir sözde terminalde çalıştırılır,
//! tuşlar gönderilir ve ekran vt100 ile okunur. Kullanıcının config'ine
//! dokunmamak için `NOBLE_HOME` geçici bir klasöre yönlendirilir.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

use noble::term::pane::Callbacks;

struct Harness {
    parser: Arc<Mutex<vt100::Parser<Callbacks>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    _master: Box<dyn portable_pty::MasterPty + Send>,
}

impl Harness {
    fn start(home: &PathBuf, rows: u16, cols: u16) -> Harness {
        let pty = native_pty_system();
        let pair = pty.openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }).unwrap();
        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_noble"));
        cmd.arg("--no-boot");
        cmd.env("NOBLE_HOME", home);
        cmd.cwd(home);
        let child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer: Arc<Mutex<Box<dyn Write + Send>>> = Arc::new(Mutex::new(pair.master.take_writer().unwrap()));
        let parser = Arc::new(Mutex::new(vt100::Parser::new_with_callbacks(rows, cols, 0, Callbacks::default())));
        {
            let parser = parser.clone();
            let writer = writer.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 65536];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let resp = {
                        let mut p = parser.lock().unwrap();
                        p.process(&buf[..n]);
                        std::mem::take(&mut p.callbacks_mut().responses)
                    };
                    if !resp.is_empty() {
                        let mut w = writer.lock().unwrap();
                        let _ = w.write_all(&resp);
                        let _ = w.flush();
                    }
                }
            });
        }
        Harness { parser, writer, child, _master: pair.master }
    }

    fn screen(&self) -> String {
        self.parser.lock().unwrap().screen().contents()
    }

    fn send(&self, bytes: &[u8]) {
        let mut w = self.writer.lock().unwrap();
        w.write_all(bytes).unwrap();
        w.flush().unwrap();
        drop(w);
        std::thread::sleep(Duration::from_millis(150));
    }

    fn wait_for(&self, what: &str, secs: u64) -> String {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let s = self.screen();
            if s.contains(what) {
                return s;
            }
            if Instant::now() > deadline {
                panic!("timed out waiting for {what:?}; screen:\n{s}");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn wait_exit(&mut self, secs: u64) -> bool {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn save(name: &str, text: &str) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("audit");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(format!("e2e-{name}.txt")), text);
}

#[test]
fn full_session_lifecycle() {
    let home = std::env::temp_dir().join(format!("noble-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();

    let mut h = Harness::start(&home, 34, 120);
    // İlk açılış: karşılama kartı; esc ile geçilir ve bir daha çıkmaz.
    let s = h.wait_for("WELCOME TO NOBLE", 20);
    save("welcome", &s);
    h.send(b"");
    let deadline = Instant::now() + Duration::from_secs(5);
    while h.screen().contains("WELCOME TO NOBLE") {
        assert!(Instant::now() < deadline, "welcome did not close");
        std::thread::sleep(Duration::from_millis(100));
    }
    let state = std::fs::read_to_string(home.join("state.json")).unwrap_or_default();
    assert!(state.contains("\"welcomed\": true"), "welcome not remembered: {state}");
    let s = h.wait_for("Projects", 20);
    save("bridge", &s);
    assert!(s.contains("Settings"));
    // Config şablonu ilk açılışta yazılır.
    assert!(home.join("config.toml").exists());

    // Yeni sekme + komut.
    h.send(b"t");
    h.wait_for("drag border", 10);
    std::thread::sleep(Duration::from_millis(1500));
    h.send(b"echo e2e-marker-$((6*7))\r");
    h.send(b"echo e2e-marker-42\r");
    let s = h.wait_for("e2e-marker-42", 20);
    save("terminal", &s);

    // Bölme: prefix + v.
    h.send(b"\x01");
    h.send(b"v");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let s = h.screen();
        if s.matches('⤢').count() >= 2 {
            save("split", &s);
            break;
        }
        assert!(Instant::now() < deadline, "split never showed:\n{s}");
        std::thread::sleep(Duration::from_millis(100));
    }

    // Köprüye dönüş: SESSIONS paneli sekmeyi göstermeli.
    h.send(b"\x1b0");
    let s = h.wait_for("2 terminals open", 10);
    save("bridge-with-session", &s);

    // Palette açılır, kapanır.
    h.send(b"\x1bp");
    h.wait_for("COMMAND", 5);
    h.send(b"\x1b");
    std::thread::sleep(Duration::from_millis(300));

    // Çıkış: onay penceresi, sonra oturum kaydı.
    h.send(b"q");
    let s = h.wait_for("QUIT NOBLE", 5);
    save("quit", &s);
    h.send(b"y");
    assert!(h.wait_exit(15), "noble did not exit");
    let session = home.join("session.json");
    assert!(session.exists(), "session.json not written");
    let text = std::fs::read_to_string(&session).unwrap();
    assert!(text.contains("\"split\""), "split layout not saved: {text}");
    drop(h);

    // Yeniden açılış: sekme geri gelir.
    let mut h = Harness::start(&home, 34, 120);
    let s = h.wait_for("2 terminals open", 25);
    save("restored", &s);
    assert!(
        !s.contains("WELCOME TO NOBLE"),
        "welcome shown again:
{s}"
    );
    h.send(b"q");
    h.wait_for("QUIT NOBLE", 5);
    h.send(b"y");
    assert!(h.wait_exit(15));
    drop(h);
    let _ = std::fs::remove_dir_all(&home);
}

/// Boşta kaynak kullanımı: bridge açıkken ortalama CPU düşük kalmalı.
/// `cargo test --release --test e2e idle -- --ignored --nocapture`
/// Süreç `secs` saniye boyunca ne kadar CPU harcadı: (dakika başına ms, bellek).
/// Toplam CPU süresi farkı kullanılır; anlık yüzdelerden çok daha az gürültülü.
/// Süre `NOBLE_IDLE_SECS` ile uzatılabilir (periyodik işleri yakalamak için ≥100).
fn measure(h: &Harness, secs: u64) -> (f64, u64) {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let secs = std::env::var("NOBLE_IDLE_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(secs);
    let pid = Pid::from_u32(h.child.process_id().unwrap());
    let mut sys = System::new();
    let kind = ProcessRefreshKind::nothing().with_cpu().with_memory();
    let read = |sys: &mut System| {
        sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, kind);
        sys.process(pid).map(|p| (p.accumulated_cpu_time(), p.memory())).unwrap_or((0, 0))
    };
    let (start, _) = read(&mut sys);
    let t0 = Instant::now();
    std::thread::sleep(Duration::from_secs(secs));
    let (end, mem) = read(&mut sys);
    let per_min = end.saturating_sub(start) as f64 * 60.0 / t0.elapsed().as_secs_f64();
    (per_min, mem)
}

fn idle_home(tag: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("noble-idle-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    // Tipik boşta durum ölçülür: karşılama kartı daha önce geçilmiş.
    std::fs::write(home.join("state.json"), r#"{"welcomed": true}"#).unwrap();
    home
}

/// Boşta kaynak kullanımı: Home açıkken ortalama CPU düşük kalmalı.
#[test]
#[ignore]
fn idle_cpu_is_low() {
    let home = idle_home("home");
    let h = Harness::start(&home, 34, 120);
    h.wait_for("Projects", 20);
    std::thread::sleep(Duration::from_secs(3));
    let (ms, mem) = measure(&h, 20);
    println!("idle home: {ms:.0} ms CPU per minute ({:.2}% of one core) · rss {} MB", ms / 600.0, mem / 1024 / 1024);
    assert!(ms < 15_000.0, "idle CPU too high: {ms} ms/min");
    drop(h);
    let _ = std::fs::remove_dir_all(&home);
}

/// Boşta terminal sekmesi: shell bekliyor, NOBLE neredeyse hiç iş yapmamalı.
#[test]
#[ignore]
fn idle_terminal_cpu_is_low() {
    let home = idle_home("term");
    let h = Harness::start(&home, 34, 120);
    h.wait_for("Projects", 20);
    h.send(b"t");
    h.wait_for("drag border", 10);
    std::thread::sleep(Duration::from_secs(4));
    let (ms, mem) = measure(&h, 20);
    println!(
        "idle terminal: {ms:.0} ms CPU per minute ({:.2}% of one core) · rss {} MB",
        ms / 600.0,
        mem / 1024 / 1024
    );
    assert!(ms < 15_000.0, "idle CPU too high: {ms} ms/min");
    drop(h);
    let _ = std::fs::remove_dir_all(&home);
}
