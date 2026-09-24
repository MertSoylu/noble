//! `config.toml`: yükleme, varsayılanlar, yol çözümü ve küçük yerinde düzenlemeler.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub terminal: TerminalCfg,
    pub keys: KeysCfg,
    pub projects: ProjectsCfg,
    pub ai: AiCfg,
    pub launchers: Vec<Launcher>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct General {
    pub theme: String,
    pub transparent: bool,
    pub boot_animation: bool,
    /// Sayfa kaydırma ve pane büyütme animasyonları.
    pub animations: bool,
    pub clock_24h: bool,
    pub show_seconds: bool,
    /// Karşılama metnindeki isim; boşsa kullanıcı adı kullanılır.
    pub operator: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TerminalCfg {
    /// Boşsa otomatik: pwsh → powershell → cmd (Windows), $SHELL (Unix).
    pub shell: String,
    pub shell_args: Vec<String>,
    pub scrollback: usize,
    pub restore_session: bool,
    pub copy_on_select: bool,
    /// Pane renk şeması: "windows-terminal" (PowerShell profilinin şeması), "theme"
    /// (arayüz temasını izle) ya da bir şema adı ("dark-plus", "light-gray", "wt:<ad>"…).
    pub colors: String,
    /// Şemanın zeminini/metnini ezen "#rrggbb" renkleri (boş = şemanınki).
    pub background: String,
    pub foreground: String,
    /// Arka plandaki sekme dikkat isteyince bildirim göster (+ dış terminale zil).
    pub notify: bool,
    /// Arka plandaki bir komut en az bu kadar saniye sürdüyse bitişi bildirilir (0 = kapalı).
    pub notify_after: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct KeysCfg {
    pub prefix: String,
    /// Prefix'ten sonra basılan tuş → eylem (varsayılanların üzerine yazar).
    pub prefix_bindings: BTreeMap<String, String>,
    /// Prefix'siz genel kısayollar → eylem.
    pub direct_bindings: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProjectsCfg {
    /// Boşsa yaygın klasörler otomatik taranır.
    pub roots: Vec<String>,
    pub max_depth: usize,
    pub exclude: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AiCfg {
    pub enabled: bool,
    pub refresh_minutes: u64,
    pub providers: Vec<String>,
    /// Bir kota penceresi bu yüzdeye ulaşınca uyar (0 = kapalı).
    pub warn_at: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Launcher {
    pub key: String,
    pub name: String,
    pub command: String,
    /// Home'da düğme/kısayol olarak görünsün mü (yüklü değilse zaten gizlenir).
    #[serde(default = "yes")]
    pub show: bool,
}

fn yes() -> bool {
    true
}

impl Default for General {
    fn default() -> Self {
        Self {
            theme: "amber".into(),
            transparent: false,
            boot_animation: true,
            animations: true,
            clock_24h: true,
            show_seconds: true,
            operator: String::new(),
        }
    }
}

impl Default for TerminalCfg {
    fn default() -> Self {
        Self {
            shell: String::new(),
            shell_args: Vec::new(),
            scrollback: 5000,
            restore_session: true,
            copy_on_select: true,
            colors: "windows-terminal".into(),
            background: String::new(),
            foreground: String::new(),
            notify: true,
            notify_after: 10,
        }
    }
}

impl Default for KeysCfg {
    fn default() -> Self {
        Self { prefix: "ctrl+a".into(), prefix_bindings: BTreeMap::new(), direct_bindings: BTreeMap::new() }
    }
}

impl Default for ProjectsCfg {
    fn default() -> Self {
        Self { roots: Vec::new(), max_depth: 4, exclude: Vec::new() }
    }
}

impl Default for AiCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            refresh_minutes: 5,
            providers: ["claude", "codex", "antigravity", "opencode-go", "kilo", "command-code"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            warn_at: 90,
        }
    }
}

/// Başlatıcılara atanabilecek Home tuşları: Home'un kendi kısayollarıyla
/// (k j t m s p q w r a o) çakışmayanlar.
pub const LAUNCH_KEYS: [&str; 15] = ["c", "x", "l", "g", "d", "e", "f", "b", "n", "u", "v", "y", "z", "i", "h"];

/// Varsayılan başlatıcılar (tuş, ad, komut). Kurulu olmayanlar Home'da görünmez.
const DEFAULT_LAUNCHERS: [(&str, &str, &str); 13] = [
    ("c", "claude", "claude"),
    ("x", "codex", "codex"),
    ("e", "opencode", "opencode"),
    ("i", "copilot", "copilot"),
    ("g", "antigravity", "agy"),
    ("v", "pi", "pi"),
    ("b", "oh-my-pi", "omp"),
    ("f", "freebuff", "freebuff"),
    ("z", "grok build", "grok"),
    ("u", "cursor", "cursor-agent"),
    // `cmd` takma adı Windows'un cmd.exe'siyle çakışır.
    ("d", "command code", "command-code"),
    ("l", "cline", "cline"),
    ("n", "kilo code", "kilo"),
];

pub fn default_launchers() -> Vec<Launcher> {
    DEFAULT_LAUNCHERS
        .iter()
        .map(|(k, n, c)| Launcher { key: k.to_string(), name: n.to_string(), command: c.to_string(), show: true })
        .collect()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: General::default(),
            terminal: TerminalCfg::default(),
            keys: KeysCfg::default(),
            projects: ProjectsCfg::default(),
            ai: AiCfg::default(),
            launchers: default_launchers(),
        }
    }
}

/// İlk çalıştırmada yazılan, açıklamalı şablon.
pub const DEFAULT_CONFIG: &str = r#"# NOBLE yapılandırması — dosya kaydedildiğinde uygulama kendini yeniden yükler.

[general]
theme = "amber"          # Ayarlar sekmesinden seçilebilir (20 tema)
transparent = false      # true: terminalin kendi arka planı (şeffaflık) görünür
boot_animation = true
animations = true        # sayfa geçişi ve tam ekran animasyonları
clock_24h = true
show_seconds = true
operator = ""            # karşılama ismi; boşsa kullanıcı adı

[terminal]
shell = ""               # boş = otomatik (pwsh → powershell → cmd | $SHELL)
shell_args = []
scrollback = 5000
restore_session = true   # çıkışta sekmeleri kaydet, açılışta geri yükle
copy_on_select = true    # fareyle seçilen metni panoya kopyala
colors = "windows-terminal"  # pane renkleri: windows-terminal (PowerShell şeman) | theme | dark-plus | campbell | light-gray …
background = ""          # şema zeminini ez, ör. '#c8c8c8' (boş = şemanınki)
foreground = ""          # şema metin rengini ez, ör. '#2e2e2e'
notify = true            # arka plan sekmesi dikkat isteyince bildir (zil, uygulama bildirimi)
notify_after = 10        # arka planda en az bu kadar saniye süren komutun bitişini bildir (0 = kapalı)

[keys]
prefix = "ctrl+a"        # prefix tuşu; iki kez basınca shell'e gönderilir
# Prefix'ten sonra basılan tuş → eylem. Örnek:
# [keys.prefix_bindings]
# "%" = "split_right"
# Prefix'siz kısayollar. Örnek:
# [keys.direct_bindings]
# "alt+enter" = "zoom"

[projects]
roots = []               # boş = Desktop, Documents, source/repos, projects, code, dev ...
max_depth = 4
exclude = []

[ai]
enabled = true
refresh_minutes = 5      # yalnızca Home açıkken; Home'a dönünce hemen yenilenir
providers = ["claude", "codex", "antigravity", "opencode-go", "kilo", "command-code"]
warn_at = 90             # bir kota penceresi bu yüzdeye ulaşınca uyar (0 = kapalı)

# Home'da seçili projede tek tuşla çalışan komutlar. Yalnızca PATH'te bulunanlar
# görünür; `show = false` gizler (Settings → Quick launch).
[[launchers]]
key = "c"
name = "claude"
command = "claude"

[[launchers]]
key = "x"
name = "codex"
command = "codex"

[[launchers]]
key = "e"
name = "opencode"
command = "opencode"

[[launchers]]
key = "i"
name = "copilot"
command = "copilot"

[[launchers]]
key = "g"
name = "antigravity"
command = "agy"

[[launchers]]
key = "v"
name = "pi"
command = "pi"

[[launchers]]
key = "b"
name = "oh-my-pi"
command = "omp"

[[launchers]]
key = "f"
name = "freebuff"
command = "freebuff"

[[launchers]]
key = "z"
name = "grok build"
command = "grok"

[[launchers]]
key = "u"
name = "cursor"
command = "cursor-agent"

[[launchers]]
key = "d"
name = "command code"
command = "command-code"

[[launchers]]
key = "l"
name = "cline"
command = "cline"

[[launchers]]
key = "n"
name = "kilo code"
command = "kilo"
"#;

/// Uygulamanın dosya yolları. `NOBLE_HOME` ortam değişkeni hepsini tek klasöre alır.
#[derive(Clone, Debug)]
pub struct Paths {
    pub config: PathBuf,
    pub data: PathBuf,
}

impl Paths {
    pub fn resolve(config_override: Option<PathBuf>) -> Paths {
        let base_override = std::env::var_os("NOBLE_HOME").map(PathBuf::from);
        let config_dir = base_override
            .clone()
            .or_else(|| dirs::config_dir().map(|d| d.join("noble")))
            .unwrap_or_else(|| PathBuf::from(".noble"));
        let data = base_override
            .or_else(|| dirs::data_local_dir().map(|d| d.join("noble")))
            .unwrap_or_else(|| PathBuf::from(".noble"));
        Paths { config: config_override.unwrap_or_else(|| config_dir.join("config.toml")), data }
    }

    pub fn data_file(&self, name: &str) -> PathBuf {
        self.data.join(name)
    }
}

/// Yükleme sonucu: config ve (varsa) hata mesajı. Hata ölümcül değildir.
pub struct Loaded {
    pub config: Config,
    pub error: Option<String>,
    pub mtime: Option<SystemTime>,
}

pub fn parse(text: &str) -> Result<Config, String> {
    let mut cfg: Config = toml::from_str(text).map_err(|e| e.message().to_string())?;
    // Desteği kaldırılan sağlayıcılar sessizce düşer.
    cfg.ai.providers.retain(|p| {
        matches!(
            p.to_ascii_lowercase().as_str(),
            "claude" | "codex" | "antigravity" | "opencode-go" | "kilo" | "command-code"
        )
    });
    if cfg.launchers.is_empty() && !text.contains("[[launchers]]") {
        cfg.launchers = default_launchers();
    } else {
        add_missing_launchers(&mut cfg.launchers);
    }
    Ok(cfg)
}

/// Eski config'lerde olmayan varsayılan başlatıcıları sona ekler (komutu
/// listede olmayanlar). Tuşu doluysa boştaki ilk tuş verilir. Gizlemek için
/// silmek yerine `show = false` kullanılır; o yüzden geri eklenmeleri sorun değil.
fn add_missing_launchers(list: &mut Vec<Launcher>) {
    for d in default_launchers() {
        if list.iter().any(|l| l.command.eq_ignore_ascii_case(&d.command)) {
            continue;
        }
        let taken = |k: &str| list.iter().any(|l| l.key == k);
        let key = if taken(&d.key) {
            LAUNCH_KEYS.iter().find(|k| !taken(k)).map(|k| k.to_string())
        } else {
            Some(d.key.clone())
        };
        let Some(key) = key else { continue };
        list.push(Launcher { key, ..d });
    }
}

/// Config'i okur; dosya yoksa açıklamalı şablonu yazar.
pub fn load(path: &Path) -> Loaded {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, DEFAULT_CONFIG);
    }
    let mtime = mtime_of(path);
    match std::fs::read_to_string(path) {
        Ok(text) => match parse(&text) {
            Ok(config) => Loaded { config, error: None, mtime },
            Err(e) => Loaded { config: Config::default(), error: Some(format!("config: {e}")), mtime },
        },
        Err(_) => Loaded { config: Config::default(), error: None, mtime },
    }
}

pub fn mtime_of(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Bir bölümdeki `key = value` satırını yorumları koruyarak günceller; satır
/// yoksa bölümün sonuna ekler, bölüm yoksa dosyanın sonuna oluşturur.
pub fn set_value(text: &str, section: &str, key: &str, value_toml: &str) -> String {
    let header = format!("[{section}]");
    let mut out: Vec<String> = Vec::new();
    let mut in_section = false;
    let mut section_seen = false;
    let mut done = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if in_section && !done {
                // Bölümün sonuna ekle (sondaki boş satırların önüne).
                let mut tail = Vec::new();
                while out.last().is_some_and(|l: &String| l.trim().is_empty()) {
                    tail.push(out.pop().unwrap());
                }
                out.push(format!("{key} = {value_toml}"));
                out.extend(tail);
                done = true;
            }
            in_section = trimmed == header;
            section_seen |= in_section;
        } else if in_section && !done {
            let lhs = trimmed.split('=').next().unwrap_or("").trim();
            if lhs == key && !trimmed.starts_with('#') {
                let comment = line.find(" #").map(|i| &line[i..]).unwrap_or("");
                let new_line = format!("{key} = {value_toml}");
                let padded = if comment.is_empty() {
                    new_line
                } else {
                    let pad = 25usize.saturating_sub(new_line.len()).max(1);
                    format!("{new_line}{}{}", " ".repeat(pad), comment.trim_start())
                };
                out.push(padded);
                done = true;
                continue;
            }
        }
        out.push(line.to_string());
    }
    if !done {
        if in_section {
            out.push(format!("{key} = {value_toml}"));
        } else if !section_seen {
            out.push(String::new());
            out.push(header);
            out.push(format!("{key} = {value_toml}"));
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// Tüm `[[launchers]]` bloklarını verilen listeyle değiştirir; diğer her şey
/// (yorumlar dahil) korunur. Bloklar ilk bloğun olduğu yere, yoksa sona yazılır.
pub fn set_launchers(text: &str, list: &[Launcher]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut insert_at = None;
    let mut in_block = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_block = trimmed == "[[launchers]]";
            if in_block {
                // Blok içeriği (sondaki boş satırlar dahil) atlanır.
                insert_at.get_or_insert(out.len());
                continue;
            }
        }
        if !in_block {
            out.push(line.to_string());
        }
    }
    let quote = |s: &str| toml::Value::String(s.to_string()).to_string();
    let mut blocks = Vec::new();
    for (i, l) in list.iter().enumerate() {
        if i > 0 {
            blocks.push(String::new());
        }
        blocks.push("[[launchers]]".into());
        blocks.push(format!("key = {}", quote(&l.key)));
        blocks.push(format!("name = {}", quote(&l.name)));
        blocks.push(format!("command = {}", quote(&l.command)));
        if !l.show {
            blocks.push("show = false".into());
        }
    }
    let at = insert_at.unwrap_or_else(|| {
        if out.last().is_some_and(|l| !l.trim().is_empty()) {
            out.push(String::new());
        }
        out.len()
    });
    // Sonrasında başka bölüm varsa araya boş satır.
    if at < out.len() && !out[at].trim().is_empty() {
        blocks.push(String::new());
    }
    out.splice(at..at, blocks);
    let mut s = out.join("\n");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_template_parses() {
        let cfg = parse(DEFAULT_CONFIG).unwrap();
        assert_eq!(cfg.general.theme, "amber");
        assert_eq!(cfg.launchers.len(), DEFAULT_LAUNCHERS.len());
        assert_eq!(cfg.keys.prefix, "ctrl+a");
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn partial_config_uses_defaults() {
        let cfg = parse("[general]\ntheme = \"ice\"\n").unwrap();
        assert_eq!(cfg.general.theme, "ice");
        assert!(cfg.general.boot_animation);
        assert_eq!(cfg.terminal.scrollback, 5000);
        assert_eq!(cfg.launchers.len(), DEFAULT_LAUNCHERS.len());
    }

    #[test]
    fn set_launchers_rewrites_blocks() {
        let mut list = default_launchers();
        list[1].show = false;
        list[0].key = "l".into();
        let out = set_launchers(DEFAULT_CONFIG, &list);
        let cfg = parse(&out).unwrap();
        assert_eq!(cfg.launchers, list);
        assert_eq!(out.matches("[[launchers]]").count(), list.len());
        assert!(out.contains("show = false") && out.contains("# Home'da seçili projede"));
        // Başka bölümle arada kalan bloklar da doğru değişir.
        let text = "[[launchers]]\nkey = \"c\"\nname = \"a\"\ncommand = \"a\"\n\n[general]\ntheme = \"ice\"\n";
        let cfg = parse(&set_launchers(text, &list)).unwrap();
        assert_eq!(cfg.launchers, list);
        assert_eq!(cfg.general.theme, "ice");
    }

    #[test]
    fn old_config_gets_new_launchers() {
        // Eski config: yalnızca claude/codex, "e" tuşu başka komutta, codex gizli.
        let text = "[[launchers]]
key = \"c\"
name = \"claude\"
command = \"claude\"

                    [[launchers]]
key = \"x\"
name = \"codex\"
command = \"codex\"
show = false

                    [[launchers]]
key = \"e\"
name = \"aider\"
command = \"aider\"
";
        let cfg = parse(text).unwrap();
        let names: Vec<_> = cfg.launchers.iter().map(|l| (l.key.as_str(), l.name.as_str())).collect();
        assert_eq!(names[..5], [("c", "claude"), ("x", "codex"), ("e", "aider"), ("l", "opencode"), ("i", "copilot")]);
        assert_eq!(cfg.launchers.len(), DEFAULT_LAUNCHERS.len() + 1);
        // Tuşlar çakışmaz.
        let mut keys: Vec<_> = cfg.launchers.iter().map(|l| l.key.clone()).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), cfg.launchers.len());
        assert!(!cfg.launchers[1].show);
        // Sağlayıcı listesi kullanıcının seçimi olarak kalır.
        let cfg = parse(
            "[ai]
providers = [\"claude\", \"gemini\"]
",
        )
        .unwrap();
        assert_eq!(cfg.ai.providers, ["claude"]);
    }

    #[test]
    fn bad_config_reports_error() {
        assert!(parse("[general\ntheme=").is_err());
    }

    #[test]
    fn set_value_keeps_comments() {
        let out = set_value(DEFAULT_CONFIG, "general", "theme", "\"synth\"");
        assert!(out.contains("theme = \"synth\""));
        assert!(out.contains("# Ayarlar sekmesinden"));
        assert_eq!(parse(&out).unwrap().general.theme, "synth");
        let added = set_value("[general]\nclock_24h = true\n\n[ai]\n", "general", "theme", "\"ice\"");
        assert_eq!(parse(&added).unwrap().general.theme, "ice");
        let created = set_value("", "general", "theme", "\"ice\"");
        assert_eq!(parse(&created).unwrap().general.theme, "ice");
    }
}
