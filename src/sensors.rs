//! Sistem sensörleri: CPU, bellek, ağ, disk ve süreçler. Ayrı bir iş
//! parçacığında örneklenir; ana döngü yalnızca anlık görüntüleri alır.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use sysinfo::{Disks, Networks, Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System, UpdateKind};

use crate::event::{AppEvent, Tx};

#[derive(Clone, Debug, Default)]
pub struct StaticInfo {
    pub host: String,
    pub os: String,
    pub cpu_brand: String,
    pub cores: usize,
    pub total_mem: u64,
}

#[derive(Clone, Debug, Default)]
pub struct ProcInfo {
    pub pid: u32,
    pub name: String,
    /// Tüm çekirdeklere göre normalize (0..100).
    pub cpu: f32,
    pub mem: u64,
}

#[derive(Clone, Debug, Default)]
pub struct DiskInfo {
    pub mount: String,
    pub total: u64,
    pub used: u64,
}

#[derive(Clone, Debug, Default)]
pub struct SensorSample {
    pub cpu: f32,
    pub cores: Vec<f32>,
    pub freq_mhz: u64,
    pub mem_used: u64,
    pub mem_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub rx_rate: f64,
    pub tx_rate: f64,
    pub disks: Vec<DiskInfo>,
    pub procs: Vec<ProcInfo>,
    pub proc_count: usize,
    pub uptime: u64,
    /// Pil durumu; pili olmayan makinelerde `None`.
    pub battery: Option<crate::battery::Battery>,
}

pub enum SensorRequest {
    Kill {
        pid: u32,
    },
    /// Ekranda ne görünüyorsa ona göre örnekleme sıklığı; ikinci alan: pilde mi.
    Mode(SensorMode, bool),
}

/// Örnekleme modu: yalnızca görünen veriler sık okunur.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SensorMode {
    /// System ekranı: her saniye, süreç listesi dahil.
    Detail,
    /// Home: her saniye CPU/bellek/ağ; süreç listesi okunmaz.
    Summary,
    /// Sensörler görünmüyor (terminal, ayarlar): 5 saniyede bir, yalnızca
    /// geçmiş grafikleri ve pil uyarısı için.
    Background,
}

impl SensorMode {
    fn interval(self, on_battery: bool) -> Duration {
        match self {
            SensorMode::Detail => Duration::from_secs(1),
            // Pildeyken Home'un özeti iki saniyede bir (ekran da o sıklıkta çizilir).
            SensorMode::Summary if on_battery => Duration::from_secs(2),
            SensorMode::Summary => Duration::from_secs(1),
            SensorMode::Background => Duration::from_secs(5),
        }
    }
}

/// Uygulama tarafında tutulan geçmiş + son örnek.
#[derive(Default)]
pub struct Sensors {
    pub info: StaticInfo,
    pub last: Option<SensorSample>,
    pub cpu_hist: VecDeque<f32>,
    pub mem_hist: VecDeque<f32>,
    pub rx_hist: VecDeque<f64>,
    pub tx_hist: VecDeque<f64>,
    pub core_hist: Vec<VecDeque<f32>>,
    /// Pil yüzdesinin değişim hızı (işletim sistemi süre vermezse tahmin için).
    pub battery_trend: crate::battery::Trend,
    trend_base: Option<Instant>,
}

pub const HISTORY: usize = 240;

fn push<T>(q: &mut VecDeque<T>, v: T) {
    if q.len() >= HISTORY {
        q.pop_front();
    }
    q.push_back(v);
}

impl Sensors {
    pub fn ingest(&mut self, s: SensorSample) {
        push(&mut self.cpu_hist, s.cpu);
        let mem_pct = if s.mem_total > 0 { s.mem_used as f32 / s.mem_total as f32 * 100.0 } else { 0.0 };
        push(&mut self.mem_hist, mem_pct);
        push(&mut self.rx_hist, s.rx_rate);
        push(&mut self.tx_hist, s.tx_rate);
        if self.core_hist.len() != s.cores.len() {
            self.core_hist = vec![VecDeque::new(); s.cores.len()];
        }
        for (h, v) in self.core_hist.iter_mut().zip(&s.cores) {
            push(h, *v);
        }
        if let Some(b) = &s.battery {
            let base = *self.trend_base.get_or_insert_with(Instant::now);
            self.battery_trend.push(base.elapsed().as_secs_f64(), b);
        }
        self.last = Some(s);
    }

    /// Pil ve (varsa) kalan / dolma süresi.
    pub fn battery(&self) -> Option<(&crate::battery::Battery, Option<u64>)> {
        let b = self.last.as_ref()?.battery.as_ref()?;
        Some((b, crate::battery::eta(b, &self.battery_trend)))
    }

    pub fn cpu(&self) -> f32 {
        self.last.as_ref().map(|s| s.cpu).unwrap_or(0.0)
    }

    pub fn mem_pct(&self) -> f32 {
        self.mem_hist.back().copied().unwrap_or(0.0)
    }

    /// HUD durum satırı: NOMINAL / ELEVATED / CRITICAL.
    pub fn status(&self) -> SysStatus {
        let Some(_) = &self.last else { return SysStatus::Warming };
        // Anlık sıçramalara değil son birkaç saniyeye bakılır.
        let recent: Vec<f32> = self.cpu_hist.iter().rev().take(5).copied().collect();
        let cpu = recent.iter().sum::<f32>() / recent.len().max(1) as f32;
        let mem = self.mem_pct();
        if cpu >= 90.0 || mem >= 95.0 {
            SysStatus::Critical
        } else if cpu >= 70.0 || mem >= 85.0 {
            SysStatus::Elevated
        } else {
            SysStatus::Nominal
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SysStatus {
    Warming,
    Nominal,
    Elevated,
    Critical,
}

impl SysStatus {
    pub fn label(&self) -> &'static str {
        match self {
            SysStatus::Warming => "SENSORS WARMING",
            SysStatus::Nominal => "SYS NOMINAL",
            SysStatus::Elevated => "LOAD ELEVATED",
            SysStatus::Critical => "LOAD CRITICAL",
        }
    }
}

/// Örnekleme iş parçacığını başlatır.
pub fn spawn(tx: Tx, requests: Receiver<SensorRequest>) {
    let _ = std::thread::Builder::new().name("sensors".into()).spawn(move || run(tx, requests));
}

fn run(tx: Tx, requests: Receiver<SensorRequest>) {
    let mut sys = System::new_with_specifics(RefreshKind::nothing());
    sys.refresh_cpu_all();
    sys.refresh_memory();
    let info = StaticInfo {
        host: System::host_name().unwrap_or_else(|| "localhost".into()),
        os: System::long_os_version().or_else(System::name).unwrap_or_else(|| std::env::consts::OS.into()),
        cpu_brand: sys.cpus().first().map(|c| c.brand().trim().to_string()).unwrap_or_default(),
        cores: sys.cpus().len().max(1),
        total_mem: sys.total_memory(),
    };
    let cores = info.cores as f32;
    if tx.send(AppEvent::SensorStatic(Box::new(info))).is_err() {
        return;
    }

    let mut networks = Networks::new_with_refreshed_list();
    let mut disks = Disks::new_with_refreshed_list();
    let proc_kind = ProcessRefreshKind::nothing().with_cpu().with_memory().with_exe(UpdateKind::Never);
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, proc_kind);

    let mut mode = SensorMode::Summary;
    let mut on_battery = false;
    let mut last_tick = Instant::now();
    // Süreç listesi, diskler ve pil kendi aralıklarıyla okunur.
    let mut last_procs: Option<Instant> = None;
    let mut last_disks: Option<Instant> = None;
    let mut last_battery: Option<Instant> = None;
    let mut procs: Vec<ProcInfo> = Vec::new();
    let mut proc_count = 0;
    let mut disk_list: Vec<DiskInfo> = Vec::new();
    let mut battery = None;

    loop {
        // İstekleri bekle; aralık dolunca örnekle.
        let wait = mode.interval(on_battery).saturating_sub(last_tick.elapsed());
        match requests.recv_timeout(wait) {
            Ok(SensorRequest::Mode(m, battery)) => {
                on_battery = battery;
                // Mod değişince (ör. System açıldı) beklemeden yeni örnek al.
                if m == mode {
                    continue;
                }
                mode = m;
            }
            Ok(SensorRequest::Kill { pid }) => {
                let spid = Pid::from_u32(pid);
                sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[spid]), true, proc_kind);
                let (ok, name) = match sys.process(spid) {
                    Some(p) => (p.kill(), p.name().to_string_lossy().into_owned()),
                    None => (false, format!("pid {pid}")),
                };
                let _ = tx.send(AppEvent::KillResult { pid, name, ok });
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
        let elapsed = last_tick.elapsed().as_secs_f64().max(0.001);
        last_tick = Instant::now();

        sys.refresh_cpu_usage();
        sys.refresh_memory();
        networks.refresh(true);
        let (rx, txb) =
            networks.list().values().fold((0u64, 0u64), |(r, t), n| (r + n.received(), t + n.transmitted()));

        let due = |last: Option<Instant>, every: Duration| last.is_none_or(|t| t.elapsed() >= every);
        // Süreç listesi en pahalı okuma: yalnızca System ekranı açıkken.
        if mode == SensorMode::Detail && due(last_procs, Duration::from_millis(1900)) {
            last_procs = Some(Instant::now());
            sys.refresh_processes_specifics(ProcessesToUpdate::All, true, proc_kind);
            proc_count = sys.processes().len();
            let mut list: Vec<ProcInfo> = sys
                .processes()
                .values()
                .map(|p| ProcInfo {
                    pid: p.pid().as_u32(),
                    name: p.name().to_string_lossy().into_owned(),
                    cpu: (p.cpu_usage() / cores).clamp(0.0, 100.0),
                    mem: p.memory(),
                })
                .filter(|p| p.pid != 0)
                .collect();
            list.sort_by(|a, b| b.cpu.partial_cmp(&a.cpu).unwrap_or(std::cmp::Ordering::Equal).then(b.mem.cmp(&a.mem)));
            procs = list;
        }
        let slow = mode == SensorMode::Background;
        if due(last_battery, Duration::from_secs(if slow { 30 } else { 10 })) {
            last_battery = Some(Instant::now());
            battery = crate::battery::read();
        }
        if due(last_disks, Duration::from_secs(if slow { 60 } else { 10 })) {
            last_disks = Some(Instant::now());
            disks.refresh(true);
            disk_list = disks
                .list()
                .iter()
                .filter(|d| d.total_space() > 0)
                .map(|d| DiskInfo {
                    mount: d.mount_point().display().to_string(),
                    total: d.total_space(),
                    used: d.total_space().saturating_sub(d.available_space()),
                })
                .collect();
            disk_list.dedup_by(|a, b| a.mount == b.mount);
        }

        let sample = SensorSample {
            cpu: sys.global_cpu_usage().clamp(0.0, 100.0),
            cores: sys.cpus().iter().map(|c| c.cpu_usage().clamp(0.0, 100.0)).collect(),
            freq_mhz: sys.cpus().first().map(|c| c.frequency()).unwrap_or(0),
            mem_used: sys.used_memory(),
            mem_total: sys.total_memory(),
            swap_used: sys.used_swap(),
            swap_total: sys.total_swap(),
            rx_rate: rx as f64 / elapsed,
            tx_rate: txb as f64 / elapsed,
            disks: disk_list.clone(),
            // Süreç listesi yalnızca System ekranında gösterilir; diğer modlarda
            // her saniye kopyalanıp gönderilmez.
            procs: if mode == SensorMode::Detail { procs.clone() } else { Vec::new() },
            proc_count,
            uptime: System::uptime(),
            battery: battery.clone(),
        };
        if tx.send(AppEvent::Sensors(Box::new(sample))).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_bounded_and_status_tracks_load() {
        let mut s = Sensors::default();
        assert_eq!(s.status(), SysStatus::Warming);
        for _ in 0..(HISTORY + 20) {
            s.ingest(SensorSample {
                cpu: 10.0,
                cores: vec![5.0, 15.0],
                mem_used: 4,
                mem_total: 16,
                ..Default::default()
            });
        }
        assert_eq!(s.cpu_hist.len(), HISTORY);
        assert_eq!(s.core_hist.len(), 2);
        assert_eq!(s.status(), SysStatus::Nominal);
        for _ in 0..5 {
            s.ingest(SensorSample { cpu: 95.0, mem_used: 4, mem_total: 16, ..Default::default() });
        }
        assert_eq!(s.status(), SysStatus::Critical);
    }
}
