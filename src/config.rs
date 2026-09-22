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
            providers: ["claude", "codex"].iter().map(|s| s.to_string()).collect(),
            warn_at: 90,
        }
    }
}

pub fn default_launchers() -> Vec<Launcher> {
    [("c", "claude", "claude"), ("x", "codex", "codex")]
        .iter()
        .map(|(k, n, c)| Launcher { key: k.to_string(), name: n.to_string(), command: c.to_string() })
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
providers = ["claude", "codex"]
warn_at = 90             # bir kota penceresi bu yüzdeye ulaşınca uyar (0 = kapalı)

# LAUNCH panelinde seçili projede tek tuşla çalışan komutlar.
[[launchers]]
key = "c"
name = "claude"
command = "claude"

[[launchers]]
key = "x"
name = "codex"
command = "codex"
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
    cfg.ai.providers.retain(|p| matches!(p.to_ascii_lowercase().as_str(), "claude" | "codex"));
    if cfg.launchers.is_empty() && !text.contains("[[launchers]]") {
        cfg.launchers = default_launchers();
    }
    Ok(cfg)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_template_parses() {
        let cfg = parse(DEFAULT_CONFIG).unwrap();
        assert_eq!(cfg.general.theme, "amber");
        assert_eq!(cfg.launchers.len(), 2);
        assert_eq!(cfg.keys.prefix, "ctrl+a");
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn partial_config_uses_defaults() {
        let cfg = parse("[general]\ntheme = \"ice\"\n").unwrap();
        assert_eq!(cfg.general.theme, "ice");
        assert!(cfg.general.boot_animation);
        assert_eq!(cfg.terminal.scrollback, 5000);
        assert_eq!(cfg.launchers.len(), 2);
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
