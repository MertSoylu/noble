//! HUD temaları. Her tema; zemin, metin, çizgi ve üç vurgu renginden oluşur.

use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Debug)]
pub struct Theme {
    pub name: &'static str,
    pub label: &'static str,
    /// Uygulama zemini (şeffaf modda `Reset`).
    pub bg: Color,
    /// Panel içi hafif yükseltilmiş zemin.
    pub raised: Color,
    pub fg: Color,
    pub dim: Color,
    /// Çerçeve/çizgi rengi.
    pub line: Color,
    pub accent: Color,
    pub accent_dim: Color,
    pub accent2: Color,
    pub ok: Color,
    pub warn: Color,
    pub crit: Color,
    pub sel_bg: Color,
    /// Vurgu zemini üzerindeki metin rengi.
    pub on_accent: Color,
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub const THEMES: [Theme; 20] = [
    Theme {
        name: "amber",
        label: "Amber",
        bg: rgb(0x0c0a07),
        raised: rgb(0x16120c),
        fg: rgb(0xeadfc8),
        dim: rgb(0x8c7c62),
        line: rgb(0x3d3326),
        accent: rgb(0xffb020),
        accent_dim: rgb(0x7a5412),
        accent2: rgb(0x5fd7d0),
        ok: rgb(0x9bd67a),
        warn: rgb(0xff8a1f),
        crit: rgb(0xff4f4f),
        sel_bg: rgb(0x2b2113),
        on_accent: rgb(0x0c0a07),
    },
    Theme {
        name: "ice",
        label: "Ice",
        bg: rgb(0x070b10),
        raised: rgb(0x0d141c),
        fg: rgb(0xd8e7f1),
        dim: rgb(0x6c8596),
        line: rgb(0x223340),
        accent: rgb(0x5ce1e6),
        accent_dim: rgb(0x1d5a60),
        accent2: rgb(0xffb86b),
        ok: rgb(0x7ee081),
        warn: rgb(0xffc857),
        crit: rgb(0xff5c7a),
        sel_bg: rgb(0x10283a),
        on_accent: rgb(0x071014),
    },
    Theme {
        name: "phosphor",
        label: "Phosphor",
        bg: rgb(0x040904),
        raised: rgb(0x0a140a),
        fg: rgb(0xc6f3bd),
        dim: rgb(0x5c8c55),
        line: rgb(0x1d321b),
        accent: rgb(0x7cff6b),
        accent_dim: rgb(0x2d6a27),
        accent2: rgb(0xe8e27a),
        ok: rgb(0x7cff6b),
        warn: rgb(0xe8e27a),
        crit: rgb(0xff6b5e),
        sel_bg: rgb(0x123012),
        on_accent: rgb(0x041004),
    },
    Theme {
        name: "synth",
        label: "Synthwave",
        bg: rgb(0x0d0714),
        raised: rgb(0x160d21),
        fg: rgb(0xecdcf6),
        dim: rgb(0x8b74a2),
        line: rgb(0x33244a),
        accent: rgb(0xff4fd8),
        accent_dim: rgb(0x6d2162),
        accent2: rgb(0x4fe3ff),
        ok: rgb(0x6bffb0),
        warn: rgb(0xffd84f),
        crit: rgb(0xff4f6b),
        sel_bg: rgb(0x2c1340),
        on_accent: rgb(0x12061a),
    },
    Theme {
        name: "catppuccin",
        label: "Catppuccin",
        bg: rgb(0x1e1e2e),
        raised: rgb(0x181825),
        fg: rgb(0xcdd6f4),
        dim: rgb(0x7f849c),
        line: rgb(0x45475a),
        accent: rgb(0xcba6f7),
        accent_dim: rgb(0x6c5a8a),
        accent2: rgb(0x89dceb),
        ok: rgb(0xa6e3a1),
        warn: rgb(0xf9e2af),
        crit: rgb(0xf38ba8),
        sel_bg: rgb(0x313244),
        on_accent: rgb(0x1e1e2e),
    },
    Theme {
        name: "tokyonight",
        label: "Tokyo Night",
        bg: rgb(0x1a1b26),
        raised: rgb(0x16161e),
        fg: rgb(0xc0caf5),
        dim: rgb(0x565f89),
        line: rgb(0x3b4261),
        accent: rgb(0x7aa2f7),
        accent_dim: rgb(0x3d59a1),
        accent2: rgb(0xbb9af7),
        ok: rgb(0x9ece6a),
        warn: rgb(0xe0af68),
        crit: rgb(0xf7768e),
        sel_bg: rgb(0x283457),
        on_accent: rgb(0x1a1b26),
    },
    Theme {
        name: "nord",
        label: "Nord",
        bg: rgb(0x2e3440),
        raised: rgb(0x3b4252),
        fg: rgb(0xeceff4),
        dim: rgb(0x7b88a1),
        line: rgb(0x4c566a),
        accent: rgb(0x88c0d0),
        accent_dim: rgb(0x4c7a8a),
        accent2: rgb(0xb48ead),
        ok: rgb(0xa3be8c),
        warn: rgb(0xebcb8b),
        crit: rgb(0xbf616a),
        sel_bg: rgb(0x434c5e),
        on_accent: rgb(0x2e3440),
    },
    Theme {
        name: "gruvbox",
        label: "Gruvbox",
        bg: rgb(0x282828),
        raised: rgb(0x1d2021),
        fg: rgb(0xebdbb2),
        dim: rgb(0x928374),
        line: rgb(0x504945),
        accent: rgb(0xfabd2f),
        accent_dim: rgb(0x7c6f24),
        accent2: rgb(0x83a598),
        ok: rgb(0xb8bb26),
        warn: rgb(0xfe8019),
        crit: rgb(0xfb4934),
        sel_bg: rgb(0x3c3836),
        on_accent: rgb(0x282828),
    },
    Theme {
        name: "dracula",
        label: "Dracula",
        bg: rgb(0x282a36),
        raised: rgb(0x21222c),
        fg: rgb(0xf8f8f2),
        dim: rgb(0x6272a4),
        line: rgb(0x44475a),
        accent: rgb(0xbd93f9),
        accent_dim: rgb(0x5a4a8a),
        accent2: rgb(0x8be9fd),
        ok: rgb(0x50fa7b),
        warn: rgb(0xf1fa8c),
        crit: rgb(0xff5555),
        sel_bg: rgb(0x3a3c4e),
        on_accent: rgb(0x282a36),
    },
    Theme {
        name: "rosepine",
        label: "Rosé Pine",
        bg: rgb(0x191724),
        raised: rgb(0x1f1d2e),
        fg: rgb(0xe0def4),
        dim: rgb(0x6e6a86),
        line: rgb(0x403d52),
        accent: rgb(0xebbcba),
        accent_dim: rgb(0x7a5c5a),
        accent2: rgb(0xc4a7e7),
        ok: rgb(0x9ccfd8),
        warn: rgb(0xf6c177),
        crit: rgb(0xeb6f92),
        sel_bg: rgb(0x26233a),
        on_accent: rgb(0x191724),
    },
    Theme {
        name: "onedark",
        label: "One Dark",
        bg: rgb(0x282c34),
        raised: rgb(0x21252b),
        fg: rgb(0xabb2bf),
        dim: rgb(0x5c6370),
        line: rgb(0x3e4451),
        accent: rgb(0x61afef),
        accent_dim: rgb(0x2f5d85),
        accent2: rgb(0xc678dd),
        ok: rgb(0x98c379),
        warn: rgb(0xe5c07b),
        crit: rgb(0xe06c75),
        sel_bg: rgb(0x2c313c),
        on_accent: rgb(0x282c34),
    },
    Theme {
        name: "everforest",
        label: "Everforest",
        bg: rgb(0x2d353b),
        raised: rgb(0x272e33),
        fg: rgb(0xd3c6aa),
        dim: rgb(0x859289),
        line: rgb(0x475258),
        accent: rgb(0xa7c080),
        accent_dim: rgb(0x5a6b45),
        accent2: rgb(0x7fbbb3),
        ok: rgb(0xa7c080),
        warn: rgb(0xdbbc7f),
        crit: rgb(0xe67e80),
        sel_bg: rgb(0x3d484d),
        on_accent: rgb(0x2d353b),
    },
    Theme {
        name: "kanagawa",
        label: "Kanagawa",
        bg: rgb(0x1f1f28),
        raised: rgb(0x16161d),
        fg: rgb(0xdcd7ba),
        dim: rgb(0x727169),
        line: rgb(0x363646),
        accent: rgb(0x7e9cd8),
        accent_dim: rgb(0x3d4f75),
        accent2: rgb(0xe6c384),
        ok: rgb(0x98bb6c),
        warn: rgb(0xffa066),
        crit: rgb(0xe82424),
        sel_bg: rgb(0x2d4f67),
        on_accent: rgb(0x1f1f28),
    },
    Theme {
        name: "solarized",
        label: "Solarized Dark",
        bg: rgb(0x002b36),
        raised: rgb(0x073642),
        fg: rgb(0x93a1a1),
        dim: rgb(0x586e75),
        line: rgb(0x0f4552),
        accent: rgb(0xb58900),
        accent_dim: rgb(0x5c4a10),
        accent2: rgb(0x2aa198),
        ok: rgb(0x859900),
        warn: rgb(0xcb4b16),
        crit: rgb(0xdc322f),
        sel_bg: rgb(0x0a4050),
        on_accent: rgb(0x002b36),
    },
    Theme {
        name: "monokai",
        label: "Monokai",
        bg: rgb(0x272822),
        raised: rgb(0x1e1f1c),
        fg: rgb(0xf8f8f2),
        dim: rgb(0x75715e),
        line: rgb(0x49483e),
        accent: rgb(0xa6e22e),
        accent_dim: rgb(0x55731a),
        accent2: rgb(0x66d9ef),
        ok: rgb(0xa6e22e),
        warn: rgb(0xe6db74),
        crit: rgb(0xf92672),
        sel_bg: rgb(0x3e3d32),
        on_accent: rgb(0x272822),
    },
    Theme {
        name: "github",
        label: "GitHub Dark",
        bg: rgb(0x0d1117),
        raised: rgb(0x161b22),
        fg: rgb(0xc9d1d9),
        dim: rgb(0x8b949e),
        line: rgb(0x30363d),
        accent: rgb(0x58a6ff),
        accent_dim: rgb(0x1f4f8a),
        accent2: rgb(0xd2a8ff),
        ok: rgb(0x3fb950),
        warn: rgb(0xd29922),
        crit: rgb(0xf85149),
        sel_bg: rgb(0x1f2a37),
        on_accent: rgb(0x0d1117),
    },
    Theme {
        name: "mono",
        label: "Mono",
        bg: rgb(0x111111),
        raised: rgb(0x1a1a1a),
        fg: rgb(0xe5e5e5),
        dim: rgb(0x7a7a7a),
        line: rgb(0x333333),
        accent: rgb(0xffffff),
        accent_dim: rgb(0x666666),
        accent2: rgb(0xbbbbbb),
        ok: rgb(0xd0d0d0),
        warn: rgb(0xf0c060),
        crit: rgb(0xff6060),
        sel_bg: rgb(0x262626),
        on_accent: rgb(0x111111),
    },
    Theme {
        name: "latte",
        label: "Catppuccin Latte",
        bg: rgb(0xeff1f5),
        raised: rgb(0xe6e9ef),
        fg: rgb(0x4c4f69),
        dim: rgb(0x8c8fa1),
        line: rgb(0xccd0da),
        accent: rgb(0x8839ef),
        accent_dim: rgb(0xc6a9f5),
        accent2: rgb(0x1e66f5),
        ok: rgb(0x40a02b),
        warn: rgb(0xdf8e1d),
        crit: rgb(0xd20f39),
        sel_bg: rgb(0xdce0e8),
        on_accent: rgb(0xeff1f5),
    },
    Theme {
        name: "solarlight",
        label: "Solarized Light",
        bg: rgb(0xfdf6e3),
        raised: rgb(0xeee8d5),
        fg: rgb(0x586e75),
        dim: rgb(0x93a1a1),
        line: rgb(0xe0d9c3),
        accent: rgb(0xb58900),
        accent_dim: rgb(0xe0cc88),
        accent2: rgb(0x268bd2),
        ok: rgb(0x859900),
        warn: rgb(0xcb4b16),
        crit: rgb(0xdc322f),
        sel_bg: rgb(0xeee8d5),
        on_accent: rgb(0xfdf6e3),
    },
    Theme {
        name: "paper",
        label: "Paper",
        bg: rgb(0xfafafa),
        raised: rgb(0xf0f0f0),
        fg: rgb(0x2b2b2b),
        dim: rgb(0x8a8a8a),
        line: rgb(0xdcdcdc),
        accent: rgb(0x1f6feb),
        accent_dim: rgb(0xa8c7fa),
        accent2: rgb(0xd6336c),
        ok: rgb(0x2da44e),
        warn: rgb(0xbf8700),
        crit: rgb(0xcf222e),
        sel_bg: rgb(0xe8eef9),
        on_accent: rgb(0xffffff),
    },
];

impl Theme {
    /// Adıyla tema bulur; bilinmeyen adlar Amber'e düşer.
    pub fn by_name(name: &str, transparent: bool) -> Theme {
        let mut theme = THEMES.iter().find(|t| t.name.eq_ignore_ascii_case(name.trim())).unwrap_or(&THEMES[0]).clone();
        if transparent {
            theme.bg = Color::Reset;
            theme.raised = Color::Reset;
        }
        theme
    }

    pub fn base(&self) -> Style {
        Style::default().fg(self.fg).bg(self.bg)
    }
    pub fn text(&self) -> Style {
        Style::default().fg(self.fg)
    }
    pub fn dim(&self) -> Style {
        Style::default().fg(self.dim)
    }
    pub fn line(&self) -> Style {
        Style::default().fg(self.line)
    }
    pub fn accent(&self) -> Style {
        Style::default().fg(self.accent)
    }
    pub fn accent_bold(&self) -> Style {
        Style::default().fg(self.accent).add_modifier(Modifier::BOLD)
    }
    pub fn accent2(&self) -> Style {
        Style::default().fg(self.accent2)
    }
    pub fn chip(&self) -> Style {
        Style::default().fg(self.on_accent).bg(self.accent).add_modifier(Modifier::BOLD)
    }
    pub fn selected(&self) -> Style {
        Style::default().bg(self.sel_bg)
    }

    /// Fare üzerindeyken tıklanabilir öğenin zemini.
    pub fn hover(&self) -> Color {
        match self.bg {
            Color::Rgb(..) => Theme::mix(self.sel_bg, self.accent, 0.22),
            _ => self.sel_bg,
        }
    }

    /// Doluluk oranına göre renk: normal → uyarı → kritik.
    pub fn level(&self, pct: f64) -> Color {
        if pct >= 85.0 {
            self.crit
        } else if pct >= 60.0 {
            self.warn
        } else {
            self.accent
        }
    }

    /// İki renk arasında doğrusal geçiş (yalnızca RGB renkler için).
    pub fn mix(a: Color, b: Color, t: f64) -> Color {
        match (a, b) {
            (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
                let t = t.clamp(0.0, 1.0);
                let l = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round() as u8;
                Color::Rgb(l(r1, r2), l(g1, g2), l(b1, b2))
            }
            _ => {
                if t < 0.5 {
                    a
                } else {
                    b
                }
            }
        }
    }
}

/// Temaların sıralı ad listesi.
pub fn theme_names() -> Vec<&'static str> {
    THEMES.iter().map(|t| t.name).collect()
}

/// Sıradaki tema adı (döngüsel).
pub fn next_theme(current: &str) -> &'static str {
    let idx = THEMES.iter().position(|t| t.name == current).unwrap_or(0);
    THEMES[(idx + 1) % THEMES.len()].name
}

/// Terminal pane'lerinin renk şeması: arayüz temasından bağımsız zemin, metin ve
/// 16 ANSI rengi. Hazır şemalar + Windows Terminal'den okunanlar (`crate::wt`).
#[derive(Clone, Debug, PartialEq)]
pub struct TermScheme {
    /// Config'te kullanılan kimlik ("dark-plus", "windows-terminal", "wt:Benim şemam").
    pub name: String,
    pub label: String,
    pub bg: Color,
    pub fg: Color,
    /// ANSI 0–15: siyah, kırmızı, yeşil, sarı, mavi, mor, camgöbeği, beyaz; sonra parlakları.
    pub ansi: [Color; 16],
}

/// "Arayüz temasını izle" seçeneğinin kimliği.
pub const FOLLOW_THEME: &str = "theme";
/// Windows Terminal'deki varsayılan (PowerShell) profilinin şemasını izleyen seçenek.
pub const WINDOWS_TERMINAL: &str = "windows-terminal";

type SchemeRow = (&'static str, &'static str, u32, u32, [u32; 16]);

/// Windows Terminal ile gelen şemalar (aynı değerler) + NOBLE'ın kendi grileri.
const BUILTIN_SCHEMES: [SchemeRow; 14] = [
    ("campbell", "Campbell", 0x0c0c0c, 0xcccccc, CAMPBELL),
    ("campbell-powershell", "Campbell Powershell", 0x012456, 0xcccccc, CAMPBELL),
    (
        "vintage",
        "Vintage",
        0x000000,
        0xc0c0c0,
        [
            0x000000, 0x800000, 0x008000, 0x808000, 0x000080, 0x800080, 0x008080, 0xc0c0c0, 0x808080, 0xff0000,
            0x00ff00, 0xffff00, 0x0000ff, 0xff00ff, 0x00ffff, 0xffffff,
        ],
    ),
    (
        "one-half-dark",
        "One Half Dark",
        0x282c34,
        0xdcdfe4,
        [
            0x282c34, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xdcdfe4, 0x5a6374, 0xe06c75,
            0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xdcdfe4,
        ],
    ),
    (
        "one-half-light",
        "One Half Light",
        0xfafafa,
        0x383a42,
        [
            0x383a42, 0xe45649, 0x50a14f, 0xc18301, 0x0184bc, 0xa626a4, 0x0997b3, 0xfafafa, 0x4f525d, 0xdf6c75,
            0x98c379, 0xe4c07a, 0x61afef, 0xc577dd, 0x56b5c1, 0xffffff,
        ],
    ),
    ("solarized-dark", "Solarized Dark", 0x002b36, 0x839496, SOLARIZED),
    ("solarized-light", "Solarized Light", 0xfdf6e3, 0x657b83, SOLARIZED),
    ("tango-dark", "Tango Dark", 0x000000, 0xd3d7cf, TANGO),
    ("tango-light", "Tango Light", 0xffffff, 0x555753, TANGO),
    (
        "dark-plus",
        "Dark+",
        0x1e1e1e,
        0xcccccc,
        [
            0x000000, 0xcd3131, 0x0dbc79, 0xe5e510, 0x2472c8, 0xbc3fbc, 0x11a8cd, 0xe5e5e5, 0x666666, 0xf14c4c,
            0x23d18b, 0xf5f543, 0x3b8eea, 0xd670d6, 0x29b8db, 0xe5e5e5,
        ],
    ),
    (
        "cga",
        "CGA",
        0x000000,
        0xaaaaaa,
        [
            0x000000, 0xaa0000, 0x00aa00, 0xaa5500, 0x0000aa, 0xaa00aa, 0x00aaaa, 0xaaaaaa, 0x555555, 0xff5555,
            0x55ff55, 0xffff55, 0x5555ff, 0xff55ff, 0x55ffff, 0xffffff,
        ],
    ),
    // Gri zeminde "beyaz" metin görünmez kalmasın diye beyazlar koyu gri.
    (
        "light-gray",
        "Light Gray",
        0xc8c8c8,
        0x1e1e1e,
        [
            0x1e1e1e, 0xa3261b, 0x2e6b28, 0x7a5200, 0x234f9a, 0x76288c, 0x11636e, 0x4d4d4d, 0x5e5e5e, 0xbf3a2e,
            0x3a7f33, 0x8f6400, 0x2f63bd, 0x8c3fa8, 0x1a7581, 0x141414,
        ],
    ),
    (
        "graphite",
        "Graphite",
        0x2b2d30,
        0xd4d6d9,
        [
            0x3a3d41, 0xe06c6c, 0x98c379, 0xe5c07b, 0x6fa8e8, 0xc678dd, 0x56b6c2, 0xc8cacd, 0x6b6f75, 0xf08585,
            0xb1d88f, 0xf0d08e, 0x8cbcf2, 0xd694e6, 0x74cad4, 0xf2f3f4,
        ],
    ),
    (
        "paper",
        "Paper",
        0xfafafa,
        0x24292f,
        [
            0x24292f, 0xcf222e, 0x116329, 0x7d4e00, 0x0969da, 0x8250df, 0x1b7c83, 0x6e7781, 0x57606a, 0xa40e26,
            0x1a7f37, 0x633c01, 0x218bff, 0xa475f9, 0x3192aa, 0x2b2b2b,
        ],
    ),
];

const CAMPBELL: [u32; 16] = [
    0x0c0c0c, 0xc50f1f, 0x13a10e, 0xc19c00, 0x0037da, 0x881798, 0x3a96dd, 0xcccccc, 0x767676, 0xe74856, 0x16c60c,
    0xf9f1a5, 0x3b78ff, 0xb4009e, 0x61d6d6, 0xf2f2f2,
];
const SOLARIZED: [u32; 16] = [
    0x002b36, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5, 0x073642, 0xcb4b16, 0x586e75,
    0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
];
const TANGO: [u32; 16] = [
    0x000000, 0xcc0000, 0x4e9a06, 0xc4a000, 0x3465a4, 0x75507b, 0x06989a, 0xd3d7cf, 0x555753, 0xef2929, 0x8ae234,
    0xfce94f, 0x729fcf, 0xad7fa8, 0x34e2e2, 0xeeeeec,
];

/// Yerleşik şemalar.
pub fn builtin_schemes() -> Vec<TermScheme> {
    BUILTIN_SCHEMES
        .iter()
        .map(|(name, label, bg, fg, ansi)| TermScheme {
            name: name.to_string(),
            label: label.to_string(),
            bg: rgb(*bg),
            fg: rgb(*fg),
            ansi: ansi.map(rgb),
        })
        .collect()
}

/// Şema listesinde ada göre arama; Windows Terminal'deki görünen adlar
/// ("Dark+", "Campbell Powershell") da kabul edilir.
pub fn find_scheme<'a>(list: &'a [TermScheme], name: &str) -> Option<&'a TermScheme> {
    let n = name.trim();
    list.iter()
        .find(|s| s.name.eq_ignore_ascii_case(n))
        .or_else(|| list.iter().find(|s| s.label.eq_ignore_ascii_case(n)))
}

/// "#rrggbb" ya da "rrggbb" → renk.
pub fn parse_hex(text: &str) -> Option<Color> {
    let h = text.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    u32::from_str_radix(h, 16).ok().map(rgb)
}

/// Pane çizimi için çözülmüş renkler.
#[derive(Clone, Debug, PartialEq)]
pub struct TermPalette {
    pub bg: Color,
    pub fg: Color,
    /// `None`: ANSI renkleri dış terminalin paletine bırakılır (tema modu).
    pub ansi: Option<[Color; 16]>,
    pub sel_bg: Color,
    pub match_bg: Color,
}

impl TermPalette {
    /// Config'teki şema + isteğe bağlı zemin/metin rengi. Bilinmeyen şema adı
    /// ya da geçersiz renk sessizce temaya düşer.
    pub fn resolve(
        schemes: &[TermScheme],
        scheme: &str,
        background: &str,
        foreground: &str,
        th: &Theme,
    ) -> TermPalette {
        let mut pal = match find_scheme(schemes, scheme) {
            Some(s) => TermPalette {
                bg: s.bg,
                fg: s.fg,
                ansi: Some(s.ansi),
                sel_bg: Theme::mix(s.bg, th.accent, 0.3),
                match_bg: Theme::mix(s.bg, th.accent, 0.45),
            },
            None => TermPalette {
                bg: th.bg,
                fg: th.fg,
                ansi: None,
                sel_bg: Theme::mix(th.accent_dim, th.bg, 0.2),
                match_bg: Theme::mix(th.accent_dim, th.bg, 0.45),
            },
        };
        if let Some(bg) = parse_hex(background) {
            pal.bg = bg;
            pal.sel_bg = Theme::mix(bg, th.accent, 0.3);
            pal.match_bg = Theme::mix(bg, th.accent, 0.45);
        }
        if let Some(fg) = parse_hex(foreground) {
            pal.fg = fg;
        }
        pal
    }

    /// vt100 rengini çizim rengine çevirir; `default` zemin ya da metin varsayılanı.
    pub fn color(&self, c: vt100::Color, default: Color) -> Color {
        match c {
            vt100::Color::Default => default,
            vt100::Color::Idx(i) if i < 16 => self.ansi.map(|a| a[i as usize]).unwrap_or(Color::Indexed(i)),
            vt100::Color::Idx(i) => Color::Indexed(i),
            vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
        }
    }
}

#[cfg(test)]
mod term_tests {
    use super::*;

    #[test]
    fn palettes_resolve() {
        let th = Theme::by_name("amber", false);
        let list = builtin_schemes();
        let follow = TermPalette::resolve(&list, FOLLOW_THEME, "", "", &th);
        assert_eq!((follow.bg, follow.fg, follow.ansi), (th.bg, th.fg, None));
        assert_eq!(follow.color(vt100::Color::Idx(1), th.fg), Color::Indexed(1));
        let gray = TermPalette::resolve(&list, "Light-Gray", "", "", &th);
        assert_eq!(gray.bg, rgb(0xc8c8c8));
        assert_eq!(gray.color(vt100::Color::Idx(1), gray.fg), rgb(0xa3261b));
        assert_eq!(gray.color(vt100::Color::Idx(200), gray.fg), Color::Indexed(200));
        let custom = TermPalette::resolve(&list, "light-gray", "#d0d0d0", "112233", &th);
        assert_eq!((custom.bg, custom.fg), (rgb(0xd0d0d0), rgb(0x112233)));
        assert_eq!(TermPalette::resolve(&list, "bogus", "nope", "", &th).bg, th.bg);
        assert_eq!(parse_hex("#abc"), None);
        // Windows Terminal'deki görünen adla da bulunur.
        assert_eq!(find_scheme(&list, "Dark+").map(|s| s.name.as_str()), Some("dark-plus"));
        assert_eq!(find_scheme(&list, "campbell powershell").map(|s| s.bg), Some(rgb(0x012456)));
    }
}
