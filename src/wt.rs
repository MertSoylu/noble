//! Windows Terminal ayarlarından renk şemalarını okur: kullanıcının kendi
//! şemaları ve PowerShell profilinin kullandığı şema (profildeki zemin/metin
//! ezmeleriyle birlikte). Dosya yoksa ya da bozuksa sessizce boş döner.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::theme::{self, TermScheme, WINDOWS_TERMINAL};

/// Okunan ayarlar.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WtImport {
    /// `settings.json` → `schemes` (kimlikleri "wt:<ad>").
    pub schemes: Vec<TermScheme>,
    /// PowerShell profilinin (yoksa varsayılan profilin) şemasının adı.
    pub profile_scheme: Option<String>,
    /// Profilin şemayı ezen zemin/metin renkleri.
    pub background: Option<String>,
    pub foreground: Option<String>,
    pub profile_name: Option<String>,
}

/// Windows Terminal'in (kararlı, önizleme, paketsiz) ayar dosyası konumları.
pub fn settings_paths() -> Vec<PathBuf> {
    let Some(local) = dirs::data_local_dir() else { return Vec::new() };
    vec![
        local.join("Packages/Microsoft.WindowsTerminal_8wekyb3d8bbwe/LocalState/settings.json"),
        local.join("Packages/Microsoft.WindowsTerminalPreview_8wekyb3d8bbwe/LocalState/settings.json"),
        local.join("Microsoft/Windows Terminal/settings.json"),
    ]
}

pub fn load() -> Option<WtImport> {
    settings_paths().iter().find(|p| p.is_file()).and_then(|p| load_from(p))
}

pub fn load_from(path: &Path) -> Option<WtImport> {
    parse(&std::fs::read_to_string(path).ok()?)
}

/// Seçicide gösterilecek tüm şemalar: Windows Terminal'deki PowerShell şeması,
/// kullanıcının kendi şemaları, sonra yerleşikler.
pub fn all_schemes(import: Option<&WtImport>) -> Vec<TermScheme> {
    let builtins = theme::builtin_schemes();
    let mut out = Vec::new();
    if let Some(wt) = import {
        let name = wt.profile_scheme.clone().unwrap_or_else(|| "Campbell".into());
        // Kullanıcı şeması yerleşikle aynı adı taşıyorsa kullanıcınınki geçerlidir.
        let base = wt
            .schemes
            .iter()
            .find(|s| s.label.eq_ignore_ascii_case(&name))
            .or_else(|| theme::find_scheme(&builtins, &name));
        if let Some(base) = base {
            let mut s = base.clone();
            s.name = WINDOWS_TERMINAL.into();
            let who = wt.profile_name.clone().unwrap_or_else(|| "PowerShell".into());
            s.label = format!("{who} · {}", base.label);
            if let Some(c) = wt.background.as_deref().and_then(theme::parse_hex) {
                s.bg = c;
            }
            if let Some(c) = wt.foreground.as_deref().and_then(theme::parse_hex) {
                s.fg = c;
            }
            out.push(s);
        }
        out.extend(wt.schemes.iter().cloned());
    }
    out.extend(builtins);
    out
}

pub fn parse(text: &str) -> Option<WtImport> {
    let root: Value = serde_json::from_str(&strip_jsonc(text)).ok()?;
    let schemes = root
        .get("schemes")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(scheme_from).collect())
        .unwrap_or_default();
    let profiles = root.get("profiles");
    let defaults = profiles.and_then(|p| p.get("defaults"));
    // `profiles` eski sürümlerde doğrudan bir dizi olabilir.
    let list: Vec<&Value> = match profiles {
        Some(Value::Array(a)) => a.iter().collect(),
        Some(p) => p.get("list").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default(),
        None => Vec::new(),
    };
    let default_guid = root.get("defaultProfile").and_then(Value::as_str).unwrap_or("");
    let is_default =
        |p: &&Value| p.get("guid").and_then(Value::as_str).is_some_and(|g| g.eq_ignore_ascii_case(default_guid));
    let is_pwsh = |p: &&Value| {
        ["name", "commandline", "source"].iter().any(|k| {
            p.get(k).and_then(Value::as_str).is_some_and(|v| {
                let v = v.to_lowercase();
                v.contains("powershell") && !v.contains("developer") || v.contains("pwsh")
            })
        })
    };
    let visible = |p: &&Value| !p.get("hidden").and_then(Value::as_bool).unwrap_or(false);
    // Varsayılan profil PowerShell ise o; değilse görünen ilk PowerShell profili; o da yoksa varsayılan.
    let profile = list
        .iter()
        .find(|p| is_default(p) && is_pwsh(p))
        .or_else(|| list.iter().find(|p| visible(p) && is_pwsh(p)))
        .or_else(|| list.iter().find(|p| is_default(p)))
        .copied();
    let field = |key: &str| -> Option<&Value> {
        profile.and_then(|p| p.get(key)).or_else(|| defaults.and_then(|d| d.get(key)))
    };
    let profile_scheme = field("colorScheme").and_then(|v| match v {
        Value::String(s) => Some(s.clone()),
        // {"dark": "...", "light": "..."}: NOBLE koyu olanı kullanır.
        Value::Object(o) => o.get("dark").or_else(|| o.get("light")).and_then(Value::as_str).map(str::to_string),
        _ => None,
    });
    Some(WtImport {
        schemes,
        profile_scheme,
        background: field("background").and_then(Value::as_str).map(str::to_string),
        foreground: field("foreground").and_then(Value::as_str).map(str::to_string),
        profile_name: profile.and_then(|p| p.get("name")).and_then(Value::as_str).map(str::to_string),
    })
}

fn scheme_from(v: &Value) -> Option<TermScheme> {
    let name = v.get("name")?.as_str()?.trim().to_string();
    let color = |k: &str| v.get(k).and_then(Value::as_str).and_then(theme::parse_hex);
    const KEYS: [&str; 16] = [
        "black",
        "red",
        "green",
        "yellow",
        "blue",
        "purple",
        "cyan",
        "white",
        "brightBlack",
        "brightRed",
        "brightGreen",
        "brightYellow",
        "brightBlue",
        "brightPurple",
        "brightCyan",
        "brightWhite",
    ];
    let mut ansi = [ratatui::style::Color::Reset; 16];
    for (slot, key) in ansi.iter_mut().zip(KEYS) {
        *slot = color(key)?;
    }
    Some(TermScheme {
        name: format!("wt:{name}"),
        label: name,
        bg: color("background").unwrap_or(ansi[0]),
        fg: color("foreground").unwrap_or(ansi[7]),
        ansi,
    })
}

/// JSONC → JSON: `//` ve `/* */` yorumlarını ve sondaki virgülleri atar
/// (dizgelerin içine dokunmadan).
pub fn strip_jsonc(text: &str) -> String {
    // Önce yorumlar gider; ardından ikinci geçişte virgülden sonra yalnızca boşluk kalır.
    strip_pass(&strip_pass(text, true), false)
}

fn strip_pass(text: &str, comments: bool) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut in_str = false;
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match (c, chars.get(i + 1)) {
            ('"', _) => {
                in_str = true;
                out.push(c);
                i += 1;
            }
            ('/', Some('/')) if comments => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            ('/', Some('*')) if comments => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
            }
            (',', _) if !comments => {
                // Sonraki anlamlı karakter kapanışsa virgül fazladır.
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                if !matches!(chars.get(j), Some('}' | ']')) {
                    out.push(c);
                }
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r##"{
        // yorum
        "defaultProfile": "{61c54bbd-c2c6-5271-96e7-009a87ff44bf}",
        "profiles": {
            "defaults": { "colorScheme": "Dark+" },
            "list": [
                { "guid": "{61c54bbd-c2c6-5271-96e7-009a87ff44bf}", "name": "Windows PowerShell",
                  "commandline": "%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe" },
                { "guid": "{0caa0dad-35be-5f56-a8ff-afceeeaa6101}", "name": "Command Prompt", "colorScheme": "Vintage" },
            ]
        },
        /* kendi şemam */
        "schemes": [
            { "name": "Soft Gray", "background": "#B8B8B8", "foreground": "#202020",
              "black": "#000000", "red": "#AA0000", "green": "#00AA00", "yellow": "#AA5500",
              "blue": "#0000AA", "purple": "#AA00AA", "cyan": "#00AAAA", "white": "#AAAAAA",
              "brightBlack": "#555555", "brightRed": "#FF5555", "brightGreen": "#55FF55",
              "brightYellow": "#FFFF55", "brightBlue": "#5555FF", "brightPurple": "#FF55FF",
              "brightCyan": "#55FFFF", "brightWhite": "#FFFFFF" },
            { "name": "Broken" },
        ],
    }"##;

    #[test]
    fn reads_powershell_scheme_and_custom_schemes() {
        let wt = parse(SAMPLE).expect("parses");
        assert_eq!(wt.profile_scheme.as_deref(), Some("Dark+"));
        assert_eq!(wt.profile_name.as_deref(), Some("Windows PowerShell"));
        assert_eq!(wt.schemes.len(), 1);
        assert_eq!(wt.schemes[0].name, "wt:Soft Gray");
        assert_eq!(wt.schemes[0].bg, theme::parse_hex("#b8b8b8").unwrap());
        let all = all_schemes(Some(&wt));
        assert_eq!(all[0].name, WINDOWS_TERMINAL);
        assert_eq!(all[0].label, "Windows PowerShell · Dark+");
        assert_eq!(all[0].bg, theme::parse_hex("#1e1e1e").unwrap());
        assert_eq!(all[1].label, "Soft Gray");
        assert!(all.iter().any(|s| s.name == "campbell-powershell"));
    }

    #[test]
    fn profile_overrides_and_object_scheme() {
        let text = r##"{ "profiles": { "defaults": { "colorScheme": { "dark": "One Half Dark", "light": "One Half Light" } },
            "list": [ { "name": "PowerShell", "source": "Windows.Terminal.PowershellCore", "background": "#101010" } ] } }"##;
        let wt = parse(text).unwrap();
        assert_eq!(wt.profile_scheme.as_deref(), Some("One Half Dark"));
        let all = all_schemes(Some(&wt));
        assert_eq!(all[0].bg, theme::parse_hex("#101010").unwrap());
        assert_eq!(all[0].fg, theme::parse_hex("#dcdfe4").unwrap());
        assert!(parse("not json").is_none());
        assert_eq!(all_schemes(None).len(), theme::builtin_schemes().len());
    }

    #[test]
    fn jsonc_is_stripped_without_touching_strings() {
        let t = strip_jsonc("{\"a\": \"http://x//y\", /* c */ \"b\": [1, 2,], // z\n}");
        let v: Value = serde_json::from_str(&t).unwrap();
        assert_eq!(v["a"], "http://x//y");
        assert_eq!(v["b"][1], 2);
    }

    #[test]
    #[ignore]
    fn live_windows_terminal() {
        let wt = load();
        println!("{wt:#?}");
        let all = all_schemes(wt.as_ref());
        println!("{:?}", all.iter().map(|s| &s.label).collect::<Vec<_>>());
    }
}
