//! Pil durumu: yüzde, şarj durumu ve kalan süre. Windows'ta `GetSystemPowerStatus`,
//! Linux'ta `/sys/class/power_supply`, macOS'ta `pmset -g batt`. Pili olmayan
//! makinelerde `None` döner ve arayüz pil satırını hiç göstermez.

use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerState {
    Discharging,
    Charging,
    Full,
    /// Prizde ama şarj olmuyor (ör. pil koruma modu %80'de durdurur).
    PluggedIn,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Battery {
    pub percent: f32,
    pub state: PowerState,
    /// İşletim sisteminin kalan süre tahmini (saniye, pildeyken).
    pub secs_left: Option<u64>,
    /// İşletim sisteminin dolma süresi tahmini (saniye, şarjdayken).
    pub secs_to_full: Option<u64>,
}

/// Anlık pil durumu; pil yoksa ya da okunamazsa `None`.
pub fn read() -> Option<Battery> {
    #[cfg(windows)]
    {
        read_windows()
    }
    #[cfg(target_os = "linux")]
    {
        read_linux(std::path::Path::new("/sys/class/power_supply"))
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("pmset").args(["-g", "batt"]).output().ok()?;
        parse_pmset(&String::from_utf8_lossy(&out.stdout))
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

#[cfg(windows)]
fn read_windows() -> Option<Battery> {
    #[repr(C)]
    #[derive(Default)]
    struct SystemPowerStatus {
        ac_line_status: u8,
        battery_flag: u8,
        battery_life_percent: u8,
        system_status_flag: u8,
        battery_life_time: u32,
        battery_full_life_time: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetSystemPowerStatus(status: *mut SystemPowerStatus) -> i32;
    }
    let mut s = SystemPowerStatus::default();
    // SAFETY: yapı Win32 SYSTEM_POWER_STATUS ile birebir aynı düzende; çağrı yalnızca onu doldurur.
    if unsafe { GetSystemPowerStatus(&mut s) } == 0 {
        return None;
    }
    from_windows(s.ac_line_status, s.battery_flag, s.battery_life_percent, s.battery_life_time)
}

/// `SYSTEM_POWER_STATUS` alanlarını yorumlar (ayrı fonksiyon: test edilebilir).
pub fn from_windows(ac: u8, flag: u8, percent: u8, life_secs: u32) -> Option<Battery> {
    // 128 = sistem pili yok, 255 = bilinmiyor.
    if flag & 128 != 0 || flag == 255 || percent > 100 {
        return None;
    }
    let charging = flag & 8 != 0;
    let state = match (ac, charging) {
        (1, true) => PowerState::Charging,
        (1, false) if percent >= 100 => PowerState::Full,
        (1, false) => PowerState::PluggedIn,
        _ => PowerState::Discharging,
    };
    let secs_left = (state == PowerState::Discharging && life_secs != u32::MAX).then_some(life_secs as u64);
    Some(Battery { percent: percent as f32, state, secs_left, secs_to_full: None })
}

/// Linux: ilk `type == Battery` girdisini okur.
pub fn read_linux(root: &std::path::Path) -> Option<Battery> {
    let entries = std::fs::read_dir(root).ok()?;
    for e in entries.flatten() {
        let dir = e.path();
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok().map(|s| s.trim().to_string());
        let num = |name: &str| read(name).and_then(|s| s.parse::<f64>().ok());
        if read("type").as_deref() != Some("Battery") {
            continue;
        }
        let percent = num("capacity")? as f32;
        let status = read("status").unwrap_or_default();
        let state = match status.as_str() {
            "Charging" => PowerState::Charging,
            "Full" => PowerState::Full,
            "Not charging" => PowerState::PluggedIn,
            _ => PowerState::Discharging,
        };
        // Enerji (µWh/µW) ya da yük (µAh/µA) üzerinden süre.
        let (now, full, rate) = match (num("energy_now"), num("energy_full"), num("power_now")) {
            (Some(n), Some(f), Some(r)) => (n, f, r),
            _ => {
                (num("charge_now").unwrap_or(0.0), num("charge_full").unwrap_or(0.0), num("current_now").unwrap_or(0.0))
            }
        };
        let hours = |amount: f64| (rate > 0.0 && amount > 0.0).then(|| (amount / rate * 3600.0) as u64);
        return Some(Battery {
            percent,
            state,
            secs_left: if state == PowerState::Discharging { hours(now) } else { None },
            secs_to_full: if state == PowerState::Charging { hours(full - now) } else { None },
        });
    }
    None
}

/// macOS `pmset -g batt` çıktısı: "-InternalBattery-0 (id=…)\t85%; discharging; 4:12 remaining present: true".
pub fn parse_pmset(text: &str) -> Option<Battery> {
    let line = text.lines().find(|l| l.contains("InternalBattery"))?;
    let tail = line.split('\t').nth(1).unwrap_or(line);
    let mut parts = tail.split(';').map(str::trim);
    let percent: f32 = parts.next()?.trim_end_matches('%').parse().ok()?;
    let status = parts.next().unwrap_or("");
    let state = match status {
        "charging" => PowerState::Charging,
        "charged" | "finishing charge" => PowerState::Full,
        "AC attached" => PowerState::PluggedIn,
        _ => PowerState::Discharging,
    };
    let secs = parts.next().and_then(|r| {
        let (h, m) = r.split_whitespace().next()?.split_once(':')?;
        Some(h.parse::<u64>().ok()? * 3600 + m.parse::<u64>().ok()? * 60)
    });
    Some(Battery {
        percent,
        state,
        secs_left: if state == PowerState::Discharging { secs } else { None },
        secs_to_full: if state == PowerState::Charging { secs } else { None },
    })
}

/// İşletim sistemi süre vermediğinde yüzdenin değişim hızından tahmin.
/// Yalnızca aynı durumdaki (pilde / şarjda) son örneklere bakar.
#[derive(Default)]
pub struct Trend {
    samples: VecDeque<(f64, f32)>,
    state: Option<PowerState>,
}

/// Tahmin için gereken en kısa gözlem süresi ve en az değişim.
const MIN_SPAN_SECS: f64 = 180.0;
const MIN_DELTA: f32 = 1.0;
const WINDOW_SECS: f64 = 20.0 * 60.0;

impl Trend {
    /// `t` saniye cinsinden artan bir zaman damgası.
    pub fn push(&mut self, t: f64, b: &Battery) {
        if self.state != Some(b.state) {
            self.samples.clear();
            self.state = Some(b.state);
        }
        self.samples.push_back((t, b.percent));
        while self.samples.front().is_some_and(|(t0, _)| t - t0 > WINDOW_SECS) {
            self.samples.pop_front();
        }
    }

    /// Tahmini kalan süre (pilde) ya da dolma süresi (şarjda), saniye.
    pub fn estimate(&self) -> Option<u64> {
        let (&(t0, p0), &(t1, p1)) = (self.samples.front()?, self.samples.back()?);
        let (span, delta) = (t1 - t0, p1 - p0);
        if span < MIN_SPAN_SECS || delta.abs() < MIN_DELTA {
            return None;
        }
        let per_sec = delta as f64 / span;
        let secs = match self.state? {
            PowerState::Discharging if per_sec < 0.0 => p1 as f64 / -per_sec,
            PowerState::Charging if per_sec > 0.0 => (100.0 - p1 as f64) / per_sec,
            _ => return None,
        };
        // 2 günden uzun tahminler anlamsız (neredeyse boşta).
        (secs < 48.0 * 3600.0).then_some(secs as u64)
    }
}

/// Kalan süre: işletim sisteminin değeri, yoksa eğilim tahmini.
pub fn eta(b: &Battery, trend: &Trend) -> Option<u64> {
    match b.state {
        PowerState::Discharging => b.secs_left.or_else(|| trend.estimate()),
        PowerState::Charging => b.secs_to_full.or_else(|| trend.estimate()),
        _ => None,
    }
}

/// Pil bloğundaki en kısa durum: "3h 12m left", "full in 45m", "estimating…".
pub fn short(b: &Battery, eta: Option<u64>) -> String {
    let dur = |s: u64| crate::util::fmt_duration(std::time::Duration::from_secs(s));
    match (b.state, eta) {
        (PowerState::Discharging, Some(s)) => format!("{} left", dur(s)),
        (PowerState::Discharging, None) => "estimating…".into(),
        (PowerState::Charging, Some(s)) => format!("full in {}", dur(s)),
        (PowerState::Charging, None) => "charging".into(),
        (PowerState::Full, _) => "fully charged".into(),
        (PowerState::PluggedIn, _) => "plugged in".into(),
    }
}

/// Kısa durum metni: "3h 12m left", "charging · full in 45m", "plugged in".
pub fn describe(b: &Battery, eta: Option<u64>) -> String {
    let dur = |s: u64| crate::util::fmt_duration(std::time::Duration::from_secs(s));
    match (b.state, eta) {
        (PowerState::Discharging, Some(s)) => format!("{} left", dur(s)),
        (PowerState::Discharging, None) => "on battery · estimating…".into(),
        (PowerState::Charging, Some(s)) => format!("charging · full in {}", dur(s)),
        (PowerState::Charging, None) => "charging".into(),
        (PowerState::Full, _) => "fully charged".into(),
        (PowerState::PluggedIn, _) => "plugged in · not charging".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bat(percent: f32, state: PowerState) -> Battery {
        Battery { percent, state, secs_left: None, secs_to_full: None }
    }

    #[test]
    fn windows_fields() {
        assert_eq!(from_windows(1, 128, 255, u32::MAX), None, "desktop without battery");
        let b = from_windows(0, 1, 76, 3 * 3600).unwrap();
        assert_eq!((b.state, b.percent, b.secs_left), (PowerState::Discharging, 76.0, Some(10_800)));
        assert_eq!(from_windows(0, 1, 76, u32::MAX).unwrap().secs_left, None);
        assert_eq!(from_windows(1, 8, 40, u32::MAX).unwrap().state, PowerState::Charging);
        assert_eq!(from_windows(1, 0, 100, u32::MAX).unwrap().state, PowerState::Full);
        assert_eq!(from_windows(1, 0, 80, u32::MAX).unwrap().state, PowerState::PluggedIn);
    }

    #[test]
    fn pmset_output() {
        let text = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=123)\t85%; discharging; 4:12 remaining present: true\n";
        let b = parse_pmset(text).unwrap();
        assert_eq!((b.percent, b.state, b.secs_left), (85.0, PowerState::Discharging, Some(4 * 3600 + 12 * 60)));
        let c = parse_pmset(" -InternalBattery-0 (id=1)\t40%; charging; 0:45 remaining present: true").unwrap();
        assert_eq!((c.state, c.secs_to_full), (PowerState::Charging, Some(45 * 60)));
        assert!(parse_pmset("Now drawing from 'AC Power'\n").is_none());
    }

    #[test]
    fn linux_sysfs() {
        let root = std::env::temp_dir().join(format!("noble-bat-{}", std::process::id()));
        let bat = root.join("BAT0");
        std::fs::create_dir_all(&bat).unwrap();
        std::fs::create_dir_all(root.join("AC")).unwrap();
        std::fs::write(root.join("AC/type"), "Mains\n").unwrap();
        for (k, v) in [
            ("type", "Battery"),
            ("capacity", "50"),
            ("status", "Discharging"),
            ("energy_now", "25000000"),
            ("energy_full", "50000000"),
            ("power_now", "10000000"),
        ] {
            std::fs::write(bat.join(k), format!("{v}\n")).unwrap();
        }
        let b = read_linux(&root).unwrap();
        assert_eq!((b.percent, b.state, b.secs_left), (50.0, PowerState::Discharging, Some(9_000)));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn trend_estimates_from_percent_changes() {
        let mut t = Trend::default();
        t.push(0.0, &bat(80.0, PowerState::Discharging));
        t.push(60.0, &bat(80.0, PowerState::Discharging));
        assert_eq!(t.estimate(), None, "too short to tell");
        // %1 / 5 dk → %75 kalan ≈ 375 dk.
        t.push(300.0, &bat(79.0, PowerState::Discharging));
        assert_eq!(t.estimate(), Some(79 * 300));
        // Şarja geçince geçmiş sıfırlanır.
        t.push(400.0, &bat(79.0, PowerState::Charging));
        assert_eq!(t.estimate(), None);
        t.push(700.0, &bat(81.0, PowerState::Charging));
        assert_eq!(t.estimate(), Some(19 * 150));
        let b = bat(81.0, PowerState::Charging);
        assert_eq!(describe(&b, eta(&b, &t)), "charging · full in 47m");
        assert_eq!(describe(&bat(100.0, PowerState::Full), None), "fully charged");
    }

    #[test]
    #[ignore]
    fn live_battery() {
        println!("{:?}", read());
    }
}
