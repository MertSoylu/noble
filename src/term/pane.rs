//! Tek bir terminal paneli: gerçek PTY (ConPTY/Unix pty) + vt100 emülatörü.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Context, Result};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::config::TerminalCfg;
use crate::event::AppEvent;
use crate::term::layout::PaneId;
use crate::util;

/// vt100 geri çağrıları: başlık, çalışma dizini, terminal sorgu yanıtları, pano.
#[derive(Default)]
pub struct Callbacks {
    pub title: String,
    pub cwd: Option<String>,
    /// Emülatörün PTY'ye geri yazması gereken yanıtlar (DSR, DA).
    pub responses: Vec<u8>,
    pub clipboard: Option<String>,
    pub bell: bool,
    /// Shell yeni bir prompt çizdi (OSC 7 / 9;9 / 133): önceki komut bitti.
    pub prompt: bool,
    /// Uygulamanın gönderdiği masaüstü bildirimi (OSC 9 metni, OSC 777;notify).
    pub notice: Option<String>,
    /// OSC 8 bağlantıları (en yenisi sonda, sınırlı sayıda).
    pub hyperlinks: std::collections::VecDeque<Hyperlink>,
    /// Açılmış ama henüz kapanmamış OSC 8 bağlantısı: (mutlak satır, sütun, adres).
    open_link: Option<(usize, u16, String)>,
}

/// OSC 8 ile işaretlenmiş metin: mutlak satır (0 = en eski geçmiş satırı) ve sütun aralığı.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hyperlink {
    pub line: usize,
    pub from: u16,
    pub to: u16,
    pub url: String,
}

/// Saklanan en fazla OSC 8 bağlantısı.
const MAX_HYPERLINKS: usize = 1000;

/// Geçmişin uzunluğu (görünen kaydırmayı bozmadan).
fn history_len(screen: &mut vt100::Screen) -> usize {
    let current = screen.scrollback();
    screen.set_scrollback(usize::MAX);
    let len = screen.scrollback();
    screen.set_scrollback(current);
    len
}

impl vt100::Callbacks for Callbacks {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = String::from_utf8_lossy(title).trim().to_string();
    }

    fn audible_bell(&mut self, _: &mut vt100::Screen) {
        self.bell = true;
    }

    fn copy_to_clipboard(&mut self, _: &mut vt100::Screen, _ty: &[u8], data: &[u8]) {
        if let Some(bytes) = util::base64_decode(data) {
            self.clipboard = Some(String::from_utf8_lossy(&bytes).into_owned());
        }
    }

    fn unhandled_osc(&mut self, screen: &mut vt100::Screen, params: &[&[u8]]) {
        let join = |parts: &[&[u8]]| {
            parts.iter().map(|p| String::from_utf8_lossy(p).into_owned()).collect::<Vec<_>>().join(";")
        };
        match params {
            // OSC 7: file://host/path
            [b"7", rest @ ..] if !rest.is_empty() => {
                self.cwd = Some(parse_cwd_url(&join(rest)));
                self.prompt = true;
            }
            // OSC 9;9: Windows Terminal'in cwd bildirimi.
            [b"9", b"9", rest @ ..] if !rest.is_empty() => {
                self.cwd = Some(join(rest).trim_matches('"').to_string());
                self.prompt = true;
            }
            // OSC 9;<metin>: iTerm2 tarzı bildirim. Sayısal alt kodlar (9;4 ilerleme
            // gibi ConEmu uzantıları) bildirim değildir.
            [b"9", rest @ ..] if !rest.is_empty() && !rest[0].iter().all(u8::is_ascii_digit) => {
                self.set_notice(join(rest));
            }
            // OSC 777;notify;başlık;gövde (rxvt / Ghostty / WezTerm).
            [b"777", b"notify", rest @ ..] => {
                let parts: Vec<String> = rest.iter().map(|p| String::from_utf8_lossy(p).trim().to_string()).collect();
                self.set_notice(parts.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" · "));
            }
            // OSC 8;parametreler;adres … OSC 8;; — adres boşsa bağlantı kapanır.
            // Adres ";" içerebilir, bu yüzden kalan parçalar birleştirilir.
            [b"8", _params, uri @ ..] => {
                let url = join(uri);
                let (row, col) = screen.cursor_position();
                let line = history_len(screen) + row as usize;
                if let Some((l0, c0, open)) = self.open_link.take() {
                    let cols = screen.size().1;
                    if line == l0 {
                        self.push_link(Hyperlink { line, from: c0, to: col, url: open });
                    } else {
                        // Satır atlayan bağlantı: ilk satırın sonu ve son satırın başı.
                        self.push_link(Hyperlink { line: l0, from: c0, to: cols, url: open.clone() });
                        self.push_link(Hyperlink { line, from: 0, to: col, url: open });
                    }
                }
                if !url.is_empty() {
                    self.open_link = Some((line, col, url));
                }
            }
            // OSC 133: FinalTerm/VS Code prompt işaretleri. A = prompt başladı, D = komut bitti.
            [b"133", kind, ..] if matches!(kind.first(), Some(b'A' | b'D')) => self.prompt = true,
            _ => {}
        }
    }

    fn visual_bell(&mut self, _: &mut vt100::Screen) {
        self.bell = true;
    }

    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let first = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        match (i1, c) {
            // DSR: imleç konumu. ConPTY açılışta bunu sorar ve yanıt bekler.
            (None, 'n') if first == 6 => {
                let (row, col) = screen.cursor_position();
                self.responses.extend_from_slice(format!("\x1b[{};{}R", row + 1, col + 1).as_bytes());
            }
            (None, 'n') if first == 5 => self.responses.extend_from_slice(b"\x1b[0n"),
            // Birincil cihaz özellikleri.
            (None, 'c') if first == 0 => self.responses.extend_from_slice(b"\x1b[?1;2c"),
            // İkincil cihaz özellikleri.
            (Some(b'>'), 'c') => self.responses.extend_from_slice(b"\x1b[>0;10;1c"),
            _ => {}
        }
    }
}

impl Callbacks {
    fn push_link(&mut self, link: Hyperlink) {
        if link.to > link.from && !link.url.is_empty() {
            self.hyperlinks.push_back(link);
            if self.hyperlinks.len() > MAX_HYPERLINKS {
                self.hyperlinks.pop_front();
            }
        }
    }

    fn set_notice(&mut self, text: String) {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if !text.is_empty() {
            self.notice = Some(util::truncate(&text, 120));
        }
    }
}

/// OSC 7 URL'sini yerel yola çevirir: `file://host/C:/x` → `C:/x`.
pub fn parse_cwd_url(raw: &str) -> String {
    let mut p = raw.trim().to_string();
    if let Some(after) = p.strip_prefix("file://") {
        p = match after.find('/') {
            Some(i) => after[i..].to_string(),
            None => after.to_string(),
        };
        p = percent_decode(&p);
        let b = p.as_bytes();
        if b.len() >= 3 && b[0] == b'/' && b[2] == b':' && b[1].is_ascii_alphabetic() {
            p.remove(0);
        }
    }
    p
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Pane başlığı: shell'in OSC başlığından süreç adı. `C:\WINDOWS\system32\cmd.exe`
/// → `cmd`, `vim - notes.md` → `vim`. Başlık yoksa `fallback`.
pub fn process_label(title: &str, fallback: &str) -> String {
    let source = if title.trim().is_empty() { fallback.trim() } else { title.trim() };
    if source.is_empty() {
        return "shell".into();
    }
    let head = source.split(" - ").next().unwrap_or(source).split(" | ").next().unwrap_or(source).trim();
    // Yol gibi görünüyorsa son parçayı al.
    let leaf = if head.contains('\\') || head.contains('/') {
        head.replace('\\', "/").trim_end_matches('/').rsplit('/').next().unwrap_or(head).to_string()
    } else {
        head.to_string()
    };
    let lower = leaf.to_lowercase();
    let bare = [".exe", ".cmd", ".bat", ".com"]
        .iter()
        .find_map(|ext| lower.strip_suffix(ext).map(|s| leaf[..s.len()].to_string()))
        .unwrap_or(leaf);
    let bare = bare.trim();
    let label = if bare.is_empty() { "shell" } else { bare };
    util::truncate(label, 28)
}

#[derive(Clone, Debug)]
pub struct ShellSpec {
    pub program: String,
    pub args: Vec<String>,
}

impl ShellSpec {
    pub fn label(&self) -> String {
        process_label("", &self.program)
    }

    fn kind(&self) -> ShellKind {
        let name = self.label().to_lowercase();
        if name == "pwsh" || name == "powershell" {
            ShellKind::PowerShell
        } else if name == "cmd" {
            ShellKind::Cmd
        } else {
            ShellKind::Posix
        }
    }

    /// Shell'i, önce `command`'ı çalıştırıp sonra etkileşimli kalacak şekilde
    /// başlatacak argümanlar. PowerShell'e çalışma dizinini bildiren prompt
    /// kancası da eklenir (bölme ve oturum kaydı gerçek dizini bilsin diye).
    pub fn args_with_command(&self, command: Option<&str>) -> Vec<String> {
        let mut args = self.args.clone();
        let cmd = command.filter(|c| !c.trim().is_empty());
        match self.kind() {
            ShellKind::PowerShell => {
                let script = match cmd {
                    Some(c) => format!("{PWSH_CWD_HOOK}; {c}"),
                    None => PWSH_CWD_HOOK.to_string(),
                };
                args.extend(["-NoExit".into(), "-Command".into(), script]);
            }
            ShellKind::Cmd => {
                // Komut `NOBLE_LAUNCH` değişkeninden açılır (bkz. `extra_env`): cmd.exe
                // `\"` kaçışını tanımadığı için tırnaklı yollar argüman olarak bozulurdu.
                if cmd.is_some() {
                    args.extend(["/K".into(), "%NOBLE_LAUNCH%".into()]);
                }
            }
            ShellKind::Posix => {
                if let Some(c) = cmd {
                    args.extend(["-c".into(), format!("{c}; exec {}", self.program)]);
                }
            }
        }
        args
    }

    /// Başlatıcı komutunu bu shell için hazırlar: Windows'ta program PATH'te
    /// `.exe`/`.cmd` olarak çözülür ve tam yolla çağrılır; böylece PowerShell
    /// betik politikasına takılan `.ps1` shim'leri devreye girmez.
    pub fn invocation(&self, command: &str) -> String {
        let command = command.trim();
        if !cfg!(windows) {
            return command.to_string();
        }
        let (program, rest) = match command.split_once(char::is_whitespace) {
            Some((p, r)) => (p, r.trim_start()),
            None => (command, ""),
        };
        let resolved = util::which(program).filter(|p| {
            let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            matches!(ext.as_str(), "exe" | "cmd" | "bat" | "com")
        });
        let Some(path) = resolved else { return command.to_string() };
        let tail = if rest.is_empty() { String::new() } else { format!(" {rest}") };
        match self.kind() {
            ShellKind::PowerShell => format!("& '{}'{tail}", path.display().to_string().replace('\'', "''")),
            ShellKind::Cmd => format!("\"{}\"{tail}", path.display()),
            ShellKind::Posix => command.to_string(),
        }
    }

    /// Shell'e özel ortam değişkenleri: cmd için dizin bildiren PROMPT ve
    /// çalıştırılacak komut (`NOBLE_LAUNCH`; tırnaklar olduğu gibi kalsın diye).
    pub fn extra_env(&self, command: Option<&str>) -> Vec<(String, String)> {
        let mut env = Vec::new();
        if let ShellKind::Cmd = self.kind() {
            if std::env::var("PROMPT").is_err() {
                env.push(("PROMPT".into(), r"$E]9;9;$P$E\$P$G".into()));
            }
            if let Some(c) = command.filter(|c| !c.trim().is_empty()) {
                env.push(("NOBLE_LAUNCH".into(), c.to_string()));
            }
        }
        env
    }
}

/// Her prompt'ta OSC 9;9 ile çalışma dizinini bildirir; mevcut prompt'u (oh-my-posh
/// dahil) sarmalar. Çift tırnak içermez ki komut satırı alıntılaması bozulmasın.
pub const PWSH_CWD_HOOK: &str = r"$global:__nobleP=$function:prompt; function global:prompt { [Console]::Write([char]27+']9;9;'+$executionContext.SessionState.Path.CurrentLocation.ProviderPath+[char]27+'\'); & $global:__nobleP }";

enum ShellKind {
    PowerShell,
    Cmd,
    Posix,
}

/// Kullanılacak shell'i belirler.
pub fn resolve_shell(cfg: &TerminalCfg) -> ShellSpec {
    let program = if !cfg.shell.trim().is_empty() {
        cfg.shell.trim().to_string()
    } else if cfg!(windows) {
        util::which("pwsh")
            .or_else(|| util::which("powershell"))
            .map(|p| p.display().to_string())
            .or_else(|| std::env::var("COMSPEC").ok())
            .unwrap_or_else(|| "cmd.exe".into())
    } else {
        std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/sh".into())
    };
    let mut spec = ShellSpec { program, args: cfg.shell_args.clone() };
    if spec.args.is_empty() && matches!(spec.kind(), ShellKind::PowerShell) {
        spec.args.push("-NoLogo".into());
    }
    spec
}

/// Fareyle seçim; pane içi (satır, sütun) koordinatları.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Selection {
    pub anchor: (u16, u16),
    pub head: (u16, u16),
}

impl Selection {
    pub fn ordered(&self) -> ((u16, u16), (u16, u16)) {
        if self.anchor <= self.head { (self.anchor, self.head) } else { (self.head, self.anchor) }
    }

    pub fn contains(&self, row: u16, col: u16) -> bool {
        let (s, e) = self.ordered();
        (row, col) >= s && (row, col) <= e
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

pub struct Pane {
    pub id: PaneId,
    pub parser: Arc<Mutex<vt100::Parser<Callbacks>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    pub dirty: Arc<AtomicBool>,
    /// (satır, sütun)
    pub size: (u16, u16),
    pub start_cwd: PathBuf,
    pub shell_label: String,
    pub command: Option<String>,
    pub selection: Option<Selection>,
    pub pid: Option<u32>,
    /// Kullanıcının son Enter'a bastığı an: komut süresini ölçmek için
    /// (prompt geri gelince sıfırlanır).
    pub command_started: Option<std::time::Instant>,
}

pub struct SpawnSpec<'a> {
    pub cwd: &'a Path,
    pub command: Option<&'a str>,
    pub shell: &'a ShellSpec,
    pub rows: u16,
    pub cols: u16,
    pub scrollback: usize,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Pane {
    pub fn spawn(id: PaneId, spec: SpawnSpec<'_>, tx: Sender<AppEvent>) -> Result<Pane> {
        let rows = spec.rows.max(2);
        let cols = spec.cols.max(4);
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .context("could not open a pseudo terminal")?;
        let mut cmd = CommandBuilder::new(&spec.shell.program);
        cmd.args(spec.shell.args_with_command(spec.command));
        let cwd = if spec.cwd.is_dir() {
            spec.cwd.to_path_buf()
        } else {
            dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
        };
        cmd.cwd(&cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "NOBLE");
        cmd.env("NOBLE_PANE", id.to_string());
        // Claude Code hook'ları durumu doğru NOBLE örneğine yazsın diye.
        cmd.env("NOBLE_INSTANCE", std::process::id().to_string());
        for (k, v) in spec.shell.extra_env(spec.command) {
            cmd.env(k, v);
        }
        let mut child =
            pair.slave.spawn_command(cmd).with_context(|| format!("could not start {}", spec.shell.program))?;
        drop(pair.slave);
        let pid = child.process_id();
        let killer = child.clone_killer();
        let mut reader = pair.master.try_clone_reader().context("pty reader")?;
        let writer: Arc<Mutex<Box<dyn Write + Send>>> =
            Arc::new(Mutex::new(pair.master.take_writer().context("pty writer")?));
        let parser =
            Arc::new(Mutex::new(vt100::Parser::new_with_callbacks(rows, cols, spec.scrollback, Callbacks::default())));
        let dirty = Arc::new(AtomicBool::new(false));

        {
            let parser = parser.clone();
            let writer = writer.clone();
            let dirty = dirty.clone();
            let tx = tx.clone();
            std::thread::Builder::new().name(format!("pty-read-{id}")).spawn(move || {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let responses = {
                                let mut p = lock(&parser);
                                // Emülatör hatası tüm pane'i öldürmesin: bu parçayı atla.
                                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.process(&buf[..n])));
                                std::mem::take(&mut p.callbacks_mut().responses)
                            };
                            if !responses.is_empty() {
                                let mut w = lock(&writer);
                                let _ = w.write_all(&responses);
                                let _ = w.flush();
                            }
                            if !dirty.swap(true, Ordering::AcqRel) && tx.send(AppEvent::PtyOutput).is_err() {
                                break;
                            }
                        }
                    }
                }
            })?;
        }
        {
            let tx = tx.clone();
            std::thread::Builder::new().name(format!("pty-wait-{id}")).spawn(move || {
                let _ = child.wait();
                let _ = tx.send(AppEvent::PtyExit(id));
            })?;
        }

        Ok(Pane {
            id,
            parser,
            writer,
            master: pair.master,
            killer,
            dirty,
            size: (rows, cols),
            start_cwd: cwd,
            shell_label: spec.shell.label(),
            command: spec.command.map(str::to_string),
            selection: None,
            pid,
            command_started: None,
        })
    }

    pub fn write(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let mut w = lock(&self.writer);
        let _ = w.write_all(bytes);
        let _ = w.flush();
    }

    /// Metni yapıştırır; uygulama istiyorsa bracketed paste ile sarar.
    pub fn paste(&self, text: &str) {
        let bracketed = lock(&self.parser).screen().bracketed_paste();
        let body = text.replace("\r\n", "\r").replace('\n', "\r");
        if bracketed {
            self.write(format!("\x1b[200~{body}\x1b[201~").as_bytes());
        } else {
            self.write(body.as_bytes());
        }
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let (rows, cols) = (rows.max(2), cols.max(4));
        if self.size == (rows, cols) {
            return;
        }
        self.size = (rows, cols);
        let _ = self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
        lock(&self.parser).screen_mut().set_size(rows, cols);
        self.selection = None;
    }

    pub fn parser(&self) -> MutexGuard<'_, vt100::Parser<Callbacks>> {
        lock(&self.parser)
    }

    /// Geçmişte kaydırır; pozitif = yukarı (daha eski).
    pub fn scroll(&self, delta: i32) {
        let mut p = lock(&self.parser);
        let cur = p.screen().scrollback() as i32;
        p.screen_mut().set_scrollback((cur + delta).max(0) as usize);
    }

    pub fn scroll_reset(&self) {
        let mut p = lock(&self.parser);
        if p.screen().scrollback() != 0 {
            p.screen_mut().set_scrollback(0);
        }
    }

    pub fn scroll_offset(&self) -> usize {
        lock(&self.parser).screen().scrollback()
    }

    pub fn label(&self) -> String {
        let p = lock(&self.parser);
        process_label(&p.callbacks().title, &self.shell_label)
    }

    pub fn cwd(&self) -> PathBuf {
        let p = lock(&self.parser);
        p.callbacks().cwd.as_ref().map(PathBuf::from).filter(|p| p.is_dir()).unwrap_or_else(|| self.start_cwd.clone())
    }

    /// Uygulamanın OSC 52 ile istediği pano içeriği (bir kez).
    pub fn take_clipboard(&self) -> Option<String> {
        lock(&self.parser).callbacks_mut().clipboard.take()
    }

    pub fn take_bell(&self) -> bool {
        std::mem::take(&mut lock(&self.parser).callbacks_mut().bell)
    }

    pub fn take_prompt(&self) -> bool {
        std::mem::take(&mut lock(&self.parser).callbacks_mut().prompt)
    }

    pub fn take_notice(&self) -> Option<String> {
        lock(&self.parser).callbacks_mut().notice.take()
    }

    pub fn selected_text(&self) -> Option<String> {
        let sel = self.selection?;
        if sel.is_empty() {
            return None;
        }
        let ((r0, c0), (r1, c1)) = sel.ordered();
        let p = lock(&self.parser);
        let text = p.screen().contents_between(r0, c0, r1, c1.saturating_add(1));
        let text: Vec<&str> = text.lines().map(|l| l.trim_end()).collect();
        let joined = text.join("\n");
        (!joined.trim().is_empty()).then_some(joined)
    }

    pub fn kill(&mut self) {
        let _ = self.killer.kill();
    }

    /// Geçmiş dahil tüm satırlar (0 = en eski) ve geçmiş uzunluğu. Görünen
    /// kaydırma konumu korunur.
    pub fn all_lines(&self) -> (Vec<String>, usize) {
        let mut p = lock(&self.parser);
        let screen = p.screen_mut();
        let original = screen.scrollback();
        screen.set_scrollback(usize::MAX);
        let history = screen.scrollback();
        let (rows, cols) = screen.size();
        let total = history + rows as usize;
        let mut lines: Vec<String> = Vec::with_capacity(total);
        let mut top = 0usize;
        while top < total {
            let offset = history.saturating_sub(top);
            screen.set_scrollback(offset);
            let start = history - offset;
            for (i, row) in screen.rows(0, cols).enumerate() {
                if start + i == lines.len() {
                    lines.push(row);
                }
            }
            top = start + rows as usize;
            if offset == 0 {
                break;
            }
        }
        screen.set_scrollback(original);
        (lines, history)
    }

    /// Mutlak satırı (0 = en eski) ekranın ortasına getirecek şekilde kaydırır.
    pub fn scroll_to_line(&self, line: usize, history: usize) {
        let rows = self.size.0 as usize;
        let top = line.saturating_sub(rows / 2);
        let offset = history.saturating_sub(top);
        lock(&self.parser).screen_mut().set_scrollback(offset);
    }

    /// Ekrandaki (satır, sütun) konumundaki OSC 8 bağlantısı: adres ve sütun aralığı.
    pub fn hyperlink_at(&self, row: u16, col: u16) -> Option<(String, u16, u16)> {
        let mut p = lock(&self.parser);
        let offset = p.screen().scrollback();
        let line = history_len(p.screen_mut()).saturating_sub(offset) + row as usize;
        p.callbacks()
            .hyperlinks
            .iter()
            .rev()
            .find(|h| h.line == line && col >= h.from && col < h.to)
            .map(|h| (h.url.clone(), h.from, h.to))
    }

    /// Görünen satır metni (bağlantı tespiti için).
    pub fn visible_row(&self, row: u16) -> Option<String> {
        let p = lock(&self.parser);
        let cols = p.screen().size().1;
        p.screen().rows(0, cols).nth(row as usize)
    }
}

/// Aramada bir eşleşme: mutlak satır (0 = en eski geçmiş satırı) ve sütun aralığı.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub line: usize,
    pub col: u16,
    pub width: u16,
}

/// Büyük/küçük harf duyarsız düz metin araması; sütunlar görüntü genişliğiyle hesaplanır.
pub fn find_matches(lines: &[String], query: &str) -> Vec<Match> {
    let fold = |c: char| c.to_lowercase().next().unwrap_or(c);
    let q: Vec<char> = query.chars().map(fold).collect();
    if q.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (line_no, line) in lines.iter().enumerate() {
        let chars: Vec<(char, u16)> =
            line.chars().map(|c| (fold(c), unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) as u16)).collect();
        if chars.len() < q.len() {
            continue;
        }
        let mut col = 0u16;
        let mut i = 0;
        while i + q.len() <= chars.len() {
            if chars[i..i + q.len()].iter().zip(&q).all(|((a, _), b)| a == b) {
                let width = chars[i..i + q.len()].iter().map(|(_, w)| *w).sum();
                out.push(Match { line: line_no, col, width });
                col += width;
                i += q.len();
            } else {
                col += chars[i].1;
                i += 1;
            }
        }
    }
    out
}

impl Drop for Pane {
    fn drop(&mut self) {
        let _ = self.killer.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(process_label("C:\\WINDOWS\\system32\\cmd.exe", "x"), "cmd");
        assert_eq!(process_label("", "C:\\Program Files\\PowerShell\\7\\pwsh.exe"), "pwsh");
        assert_eq!(process_label("vim - notes.md", "bash"), "vim");
        assert_eq!(process_label("", ""), "shell");
        assert_eq!(process_label("/bin/zsh", "x"), "zsh");
        assert_eq!(process_label("✳ Claude Code", "pwsh"), "✳ Claude Code");
    }

    #[test]
    fn cwd_urls() {
        assert_eq!(parse_cwd_url("file://host/home/me/my%20dir"), "/home/me/my dir");
        assert_eq!(parse_cwd_url("file://DESKTOP/C:/Users/Mert"), "C:/Users/Mert");
        assert_eq!(parse_cwd_url("C:\\x"), "C:\\x");
    }

    #[test]
    fn shell_command_args() {
        let pwsh = ShellSpec { program: "C:\\pwsh.exe".into(), args: vec!["-NoLogo".into()] };
        let with = pwsh.args_with_command(Some("claude"));
        assert_eq!(&with[..3], ["-NoLogo", "-NoExit", "-Command"]);
        assert!(with[3].starts_with(PWSH_CWD_HOOK) && with[3].ends_with("; claude"));
        assert!(!PWSH_CWD_HOOK.contains('"'));
        assert_eq!(pwsh.args_with_command(None)[3], PWSH_CWD_HOOK);
        assert!(pwsh.extra_env(Some("claude")).is_empty());
        let cmd = ShellSpec { program: "cmd.exe".into(), args: vec![] };
        assert_eq!(cmd.args_with_command(Some("codex")), vec!["/K", "%NOBLE_LAUNCH%"]);
        let quoted = r#""C:\Program Files\x\claude.exe" --resume"#;
        assert!(cmd.extra_env(Some(quoted)).contains(&("NOBLE_LAUNCH".into(), quoted.into())));
        assert!(cmd.args_with_command(None).is_empty());
        let bash = ShellSpec { program: "/bin/bash".into(), args: vec![] };
        assert_eq!(bash.args_with_command(Some("gemini")), vec!["-c", "gemini; exec /bin/bash"]);
    }

    #[test]
    fn launcher_invocation() {
        let pwsh = ShellSpec { program: "powershell.exe".into(), args: vec![] };
        // PATH'te olmayan komut olduğu gibi kalır.
        assert_eq!(pwsh.invocation("definitely-not-a-real-tool --x"), "definitely-not-a-real-tool --x");
        if cfg!(windows) {
            let inv = pwsh.invocation("cmd /c echo hi");
            assert!(
                inv.starts_with("& '") && inv.to_lowercase().contains("cmd.exe'") && inv.ends_with(" /c echo hi"),
                "{inv}"
            );
            let cmd = ShellSpec { program: "cmd.exe".into(), args: vec![] };
            assert!(cmd.invocation("cmd").starts_with('"'));
        }
    }

    #[test]
    fn callbacks_answer_queries() {
        let mut parser = vt100::Parser::new_with_callbacks(10, 40, 0, Callbacks::default());
        parser.process(b"abc\x1b[6n\x1b]0;C:\\bin\\cmd.exe\x07\x1b]7;file://h/tmp/x\x07");
        let cb = parser.callbacks();
        assert_eq!(cb.responses, b"\x1b[1;4R");
        assert_eq!(cb.title, "C:\\bin\\cmd.exe");
        assert_eq!(cb.cwd.as_deref(), Some("/tmp/x"));
        assert!(cb.prompt);
    }

    #[test]
    fn osc8_hyperlinks_are_tracked() {
        let mut parser = vt100::Parser::new_with_callbacks(5, 40, 100, Callbacks::default());
        parser.process(b"see \x1b]8;id=1;https://example.com/a;b\x1b\\docs\x1b]8;;\x1b\\ now");
        let links = &parser.callbacks().hyperlinks;
        assert_eq!(links.len(), 1);
        assert_eq!(links[0], Hyperlink { line: 0, from: 4, to: 8, url: "https://example.com/a;b".into() });
        // Ekran kaydıkça mutlak satır numarası sabit kalır.
        parser.process(b"\r\n\n\n\n\n\n\x1b]8;;file:///tmp/x.rs\x07x.rs\x1b]8;;\x07");
        let last = parser.callbacks().hyperlinks.back().unwrap().clone();
        assert_eq!((last.from, last.to), (0, 4));
        assert!(last.line >= 6, "{last:?}");
    }

    #[test]
    fn notifications_and_prompt_marks() {
        let mut parser = vt100::Parser::new_with_callbacks(10, 40, 0, Callbacks::default());
        // İlerleme çubuğu (9;4) bildirim değildir.
        parser.process(b"]9;4;1;50");
        assert_eq!(parser.callbacks().notice, None);
        assert!(!parser.callbacks().prompt);
        parser.process(
            b"]9;

Claude needs your permission",
        );
        assert_eq!(parser.callbacks().notice.as_deref(), Some("Claude needs your permission"));
        parser.process(b"]777;notify;Build;done in 3s");
        assert_eq!(parser.callbacks().notice.as_deref(), Some("Build · done in 3s"));
        parser.process(b"]133;D;0");
        assert!(parser.callbacks().prompt);
    }

    #[test]
    fn search_is_case_insensitive_and_width_aware() {
        let lines = vec!["Error: x".to_string(), "日本 error error".to_string(), "none".to_string()];
        let m = find_matches(&lines, "ERROR");
        assert_eq!(
            m,
            vec![
                Match { line: 0, col: 0, width: 5 },
                Match { line: 1, col: 5, width: 5 },
                Match { line: 1, col: 11, width: 5 },
            ]
        );
        assert!(find_matches(&lines, "").is_empty());
    }

    #[test]
    fn selection_order() {
        let s = Selection { anchor: (3, 5), head: (1, 2) };
        assert_eq!(s.ordered(), ((1, 2), (3, 5)));
        assert!(s.contains(2, 0));
        assert!(!s.contains(3, 6));
    }
}
