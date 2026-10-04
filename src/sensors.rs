//! System sensors: CPU, memory, network, disks and processes. Sampled in a
//! separate thread; the main loop only receives the snapshots.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
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
    /// Normalized across all cores (0..100).
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
    /// Battery state; `None` on machines without a battery.
    pub battery: Option<crate::battery::Battery>,
}

pub enum SensorRequest {
    Kill {
        pid: u32,
    },
    /// Sampling rate based on what is on screen; second field: on battery or not.
    Mode(SensorMode, bool),
}

/// Sampling mode: only visible data is read often.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SensorMode {
    /// System screen: every second, including the process list.
    Detail,
    /// Home: CPU/memory/network every second; the process list is not read.
    Summary,
    /// Sensors not visible (terminal, settings): every 5 seconds, only for
    /// the history graphs and the battery warning.
    Background,
}

impl SensorMode {
    fn interval(self, on_battery: bool) -> Duration {
        match self {
            SensorMode::Detail => Duration::from_secs(1),
            // On battery the Home summary runs every two seconds (the screen draws at that rate too).
            SensorMode::Summary if on_battery => Duration::from_secs(2),
            SensorMode::Summary => Duration::from_secs(1),
            SensorMode::Background => Duration::from_secs(5),
        }
    }
}

/// History + last sample held on the app side.
#[derive(Default)]
pub struct Sensors {
    pub info: StaticInfo,
    pub last: Option<SensorSample>,
    pub cpu_hist: VecDeque<f32>,
    pub mem_hist: VecDeque<f32>,
    pub rx_hist: VecDeque<f64>,
    pub tx_hist: VecDeque<f64>,
    pub core_hist: Vec<VecDeque<f32>>,
    /// Rate of change of the battery percent (to estimate when the OS gives no time).
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

    /// The battery and (optionally) time left / time to full.
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

    /// HUD status line: NOMINAL / ELEVATED / CRITICAL.
    pub fn status(&self) -> SysStatus {
        let Some(_) = &self.last else { return SysStatus::Warming };
        // It looks at the last few seconds, not momentary spikes.
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

/// Starts the sampling thread.
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
    // Disks are read on a thread of their own: a dead network mount (sshfs, smb) can block `statvfs`
    // for minutes, and that must not freeze CPU, memory and network. While a read hangs no new one starts.
    let disks: Arc<Mutex<Option<Disks>>> = Arc::new(Mutex::new(None));
    let disks_busy = Arc::new(AtomicBool::new(false));
    let disk_shared: Arc<Mutex<Vec<DiskInfo>>> = Arc::new(Mutex::new(Vec::new()));
    let proc_kind = ProcessRefreshKind::nothing().with_cpu().with_memory().with_exe(UpdateKind::Never);
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, proc_kind);

    let mut mode = SensorMode::Summary;
    let mut on_battery = false;
    let mut last_tick = Instant::now();
    // Process list, disks and battery are read on their own intervals.
    let mut last_procs: Option<Instant> = None;
    let mut last_disks: Option<Instant> = None;
    let mut last_battery: Option<Instant> = None;
    let mut procs: Vec<ProcInfo> = Vec::new();
    let mut proc_count = 0;
    let mut battery = None;

    loop {
        // Wait for requests; sample when the interval is up.
        let wait = mode.interval(on_battery).saturating_sub(last_tick.elapsed());
        match requests.recv_timeout(wait) {
            Ok(SensorRequest::Mode(m, battery)) => {
                on_battery = battery;
                // When the mode changes (e.g. System opened) take a fresh sample without waiting.
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
        let (rx, txb) = networks
            .list()
            .iter()
            .filter(|(name, _)| counts_traffic(name))
            .fold((0u64, 0u64), |(r, t), (_, n)| (r + n.received(), t + n.transmitted()));

        let due = |last: Option<Instant>, every: Duration| last.is_none_or(|t| t.elapsed() >= every);
        // The process list is the most expensive read: only while the System screen is open.
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
        if due(last_disks, Duration::from_secs(if slow { 60 } else { 10 })) && !disks_busy.swap(true, Ordering::AcqRel)
        {
            last_disks = Some(Instant::now());
            let (disks, busy, shared) = (disks.clone(), disks_busy.clone(), disk_shared.clone());
            let spawned = std::thread::Builder::new().name("disks".into()).spawn(move || {
                if let Ok(mut d) = disks.lock() {
                    let d = d.get_or_insert_with(Disks::new);
                    d.refresh(true);
                    let list = disk_rows(d.list());
                    if let Ok(mut s) = shared.lock() {
                        *s = list;
                    }
                }
                busy.store(false, Ordering::Release);
            });
            if spawned.is_err() {
                disks_busy.store(false, Ordering::Release);
            }
        }
        let disk_list = disk_shared.lock().map(|d| d.clone()).unwrap_or_default();

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
            disks: disk_list,
            // The process list is only shown on the System screen; in other modes
            // it is not copied and sent every second.
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

/// Does this network interface's traffic count toward the rates? Loopback (local dev servers) is not
/// network traffic, and virtual links (VPN tunnels, container bridges, Apple Wireless Direct) carry
/// bytes that also cross the physical link, which would count them twice.
/// Linux: `lo`, `docker0`, `veth*`, `br-*`, `virbr*`, `tun*`/`tap*`, `wg*`; macOS: `lo0`, `utun*`, `awdl*`,
/// `llw*`, `bridge*`, `gif*`, `stf*`, `anpi*`, `ap*`; Windows: the loopback pseudo-interface (VPN adapters
/// have free-form names there and stay counted).
fn counts_traffic(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    const VIRTUAL: &[&str] = &[
        "lo",
        "docker",
        "veth",
        "br-",
        "virbr",
        "vmnet",
        "vboxnet",
        "tun",
        "tap",
        "wg",
        "utun",
        "awdl",
        "llw",
        "bridge",
        "gif",
        "stf",
        "anpi",
        "ap",
        "zt",
        "tailscale",
        "cni",
        "flannel",
        "kube",
    ];
    if n.contains("loopback") {
        return false;
    }
    !VIRTUAL.iter().any(|p| {
        n.strip_prefix(p).is_some_and(|rest| {
            // `ap1` is a macOS access-point link, but `apple`/`apfs` style names are not; the same for `lo`
            // (`lo`, `lo0`) against e.g. `local`. Longer prefixes are specific enough on their own.
            p.len() > 2 || rest.is_empty() || rest.starts_with(|c: char| c.is_ascii_digit())
        })
    })
}

/// The disk rows to show, from sysinfo's mount list. Skipped: empty and pseudo file systems, container and
/// snap images, and the second mount of one device (btrfs subvolumes `/` and `/home`, bind mounts).
/// macOS also lists the APFS system volumes (`/System/Volumes/Data`, `VM`, `Preboot`, which share `/`'s
/// container) and Xcode's simulator runtimes; Windows lists drive letters only, none of this applies.
fn disk_rows(list: &[sysinfo::Disk]) -> Vec<DiskInfo> {
    let mut rows: Vec<(String, DiskInfo)> = Vec::new();
    for d in list {
        if d.total_space() == 0 {
            continue;
        }
        let fs = d.file_system().to_string_lossy().to_ascii_lowercase();
        let mount = d.mount_point().display().to_string();
        if skip_mount(&mount, &fs) {
            continue;
        }
        let device = d.name().to_string_lossy().into_owned();
        let info =
            DiskInfo { mount, total: d.total_space(), used: d.total_space().saturating_sub(d.available_space()) };
        // One row per device (the first mount, usually `/`); a nameless device is compared by mount.
        let key = if device.is_empty() { info.mount.clone() } else { device };
        if rows.iter().any(|(k, r)| *k == key && r.total == info.total) {
            continue;
        }
        rows.push((key, info));
    }
    rows.into_iter().map(|(_, r)| r).collect()
}

/// Mounts that are not a disk the user thinks of (see `disk_rows`).
fn skip_mount(mount: &str, fs: &str) -> bool {
    const PSEUDO_FS: &[&str] = &["overlay", "squashfs", "tmpfs", "devtmpfs", "ramfs", "nsfs", "fuse.snapfuse"];
    if PSEUDO_FS.contains(&fs) {
        return true;
    }
    const SKIP_UNDER: &[&str] = &[
        "/var/lib/docker/",
        "/var/lib/containers/",
        "/snap/",
        "/var/snap/",
        "/System/Volumes",
        "/Library/Developer/CoreSimulator/",
        "/private/var/vm",
    ];
    SKIP_UNDER.iter().any(|p| mount.starts_with(p))
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

    #[test]
    fn traffic_skips_loopback_and_virtual_links() {
        for name in ["lo", "lo0", "utun3", "awdl0", "docker0", "veth12ab", "br-1a2b", "wg0", "tun0", "bridge100"] {
            assert!(!counts_traffic(name), "{name}");
        }
        assert!(!counts_traffic("Loopback Pseudo-Interface 1"));
        for name in ["eth0", "en0", "wlan0", "wlp3s0", "enp0s31f6", "Ethernet", "Wi-Fi", "local0", "apple0"] {
            assert!(counts_traffic(name), "{name}");
        }
    }

    #[test]
    fn container_and_system_mounts_are_skipped() {
        assert!(skip_mount("/var/lib/docker/overlay2/x/merged", "ext4"));
        assert!(skip_mount("/", "overlay"));
        assert!(skip_mount("/snap/core/1", "squashfs"));
        assert!(skip_mount("/System/Volumes/Data", "apfs"));
        assert!(skip_mount("/Library/Developer/CoreSimulator/Volumes/iOS_22", "apfs"));
        assert!(!skip_mount("/", "ext4"));
        assert!(!skip_mount("/home", "btrfs"));
        assert!(!skip_mount("C:\\", "ntfs"));
    }
}
