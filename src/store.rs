//! Kalıcı küçük veri: son dizinler (frecency), oturum ve kayıtlı çalışma alanları.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::term::layout::SavedNode;

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn read_json<T: for<'de> Deserialize<'de>>(file: &Path) -> Option<T> {
    let text = std::fs::read_to_string(file).ok()?;
    serde_json::from_str(&text).ok()
}

/// Atomik yazım: önce geçici dosya, sonra yeniden adlandırma.
fn write_json<T: Serialize>(file: &Path, value: &T) {
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(value) {
        let tmp = file.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, file);
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RecentEntry {
    pub path: String,
    pub count: u32,
    pub last: i64,
}

impl RecentEntry {
    /// Sıklık × yakınlık: bir gün önceki kullanım yarı ağırlıkta sayılır.
    pub fn score(&self, now: i64) -> f64 {
        let hours = ((now - self.last).max(0) as f64) / 3600.0;
        self.count as f64 / (1.0 + hours / 24.0)
    }
}

pub struct Recent {
    file: Option<PathBuf>,
    pub entries: Vec<RecentEntry>,
    /// Var olan dizinlerin skor sıralı listesi (her karede diske gitmemek için).
    cache: Vec<RecentEntry>,
}

impl Recent {
    pub fn load(file: PathBuf) -> Recent {
        let entries = read_json(&file).unwrap_or_default();
        let mut r = Recent { file: Some(file), entries, cache: Vec::new() };
        r.rebuild();
        r
    }

    pub fn memory() -> Recent {
        Recent { file: None, entries: Vec::new(), cache: Vec::new() }
    }

    fn rebuild(&mut self) {
        let t = now();
        let mut list: Vec<RecentEntry> = self.entries.iter().filter(|e| Path::new(&e.path).is_dir()).cloned().collect();
        list.sort_by(|a, b| b.score(t).total_cmp(&a.score(t)));
        self.cache = list;
    }

    fn key(path: &Path) -> String {
        path.display().to_string()
    }

    pub fn record(&mut self, path: &Path) {
        let key = Self::key(path);
        let t = now();
        match self.entries.iter_mut().find(|e| e.path.eq_ignore_ascii_case(&key)) {
            Some(e) => {
                e.count = e.count.saturating_add(1);
                e.last = t;
            }
            None => self.entries.push(RecentEntry { path: key, count: 1, last: t }),
        }
        // En düşük skorlu girdileri at.
        if self.entries.len() > 60 {
            self.entries.sort_by(|a, b| b.score(t).total_cmp(&a.score(t)));
            self.entries.truncate(50);
        }
        self.rebuild();
        self.save();
    }

    pub fn remove(&mut self, path: &str) {
        self.entries.retain(|e| e.path != path);
        self.rebuild();
        self.save();
    }

    /// Var olan dizinler, skora göre.
    pub fn top(&self, n: usize) -> Vec<RecentEntry> {
        self.cache.iter().take(n).cloned().collect()
    }

    fn save(&self) {
        if let Some(f) = &self.file {
            write_json(f, &self.entries);
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SavedTab {
    pub name: Option<String>,
    pub origin: String,
    pub layout: SavedNode,
    /// Odaktaki pane'in yaprak sırasındaki indeksi.
    pub focus: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Workspace {
    pub name: String,
    pub saved_at: i64,
    pub tabs: Vec<SavedTab>,
}

impl Workspace {
    pub fn pane_count(&self) -> usize {
        fn count(n: &SavedNode) -> usize {
            match n {
                SavedNode::Leaf { .. } => 1,
                SavedNode::Split { a, b, .. } => count(a) + count(b),
            }
        }
        self.tabs.iter().map(|t| count(&t.layout)).sum()
    }
}

pub fn load_session(file: &Path) -> Option<Workspace> {
    read_json(file)
}

pub fn save_session(file: &Path, ws: &Workspace) {
    if ws.tabs.is_empty() {
        let _ = std::fs::remove_file(file);
    } else {
        write_json(file, ws);
    }
}

pub struct Workspaces {
    file: Option<PathBuf>,
    pub list: Vec<Workspace>,
}

impl Workspaces {
    pub fn load(file: PathBuf) -> Workspaces {
        let list = read_json(&file).unwrap_or_default();
        Workspaces { file: Some(file), list }
    }

    pub fn memory() -> Workspaces {
        Workspaces { file: None, list: Vec::new() }
    }

    /// Aynı isimli kaydı değiştirir, yoksa en başa ekler.
    pub fn upsert(&mut self, mut ws: Workspace) {
        ws.saved_at = now();
        self.list.retain(|w| !w.name.eq_ignore_ascii_case(&ws.name));
        self.list.insert(0, ws);
        self.list.truncate(20);
        self.save();
    }

    pub fn remove(&mut self, name: &str) {
        self.list.retain(|w| w.name != name);
        self.save();
    }

    fn save(&self) {
        if let Some(f) = &self.file {
            write_json(f, &self.list);
        }
    }
}

/// Arayüz durumu: karşılama ekranı görüldü mü, sabitlenmiş projeler.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiStateData {
    pub welcomed: bool,
    pub pins: Vec<String>,
}

pub struct UiState {
    file: Option<PathBuf>,
    pub data: UiStateData,
}

impl UiState {
    pub fn load(file: PathBuf) -> UiState {
        UiState { data: read_json(&file).unwrap_or_default(), file: Some(file) }
    }

    pub fn memory() -> UiState {
        UiState { file: None, data: UiStateData::default() }
    }

    pub fn save(&self) {
        if let Some(f) = &self.file {
            write_json(f, &self.data);
        }
    }

    pub fn is_pinned(&self, path: &Path) -> bool {
        let key = path.to_string_lossy();
        self.data.pins.iter().any(|p| p.eq_ignore_ascii_case(&key))
    }

    /// Sabitlemeyi çevirir; yeni durumu döndürür.
    pub fn toggle_pin(&mut self, path: &Path) -> bool {
        let key = path.to_string_lossy().into_owned();
        let pinned = if self.is_pinned(path) {
            self.data.pins.retain(|p| !p.eq_ignore_ascii_case(&key));
            false
        } else {
            self.data.pins.push(key);
            true
        };
        self.save();
        pinned
    }
}

/// AI kota kullanım geçmişi: "sağlayıcı/pencere" → (zaman, yüzde) örnekleri.
/// Home'daki küçük grafik için tutulur; birkaç günden eskisi atılır.
pub struct UsageHistory {
    file: Option<PathBuf>,
    pub series: std::collections::BTreeMap<String, Vec<(i64, u8)>>,
}

/// Geçmişin saklandığı süre.
const HISTORY_KEEP_SECS: i64 = 8 * 86_400;

impl UsageHistory {
    pub fn load(file: PathBuf) -> UsageHistory {
        let series = read_json(&file).unwrap_or_default();
        UsageHistory { file: Some(file), series }
    }

    pub fn memory() -> UsageHistory {
        UsageHistory { file: None, series: Default::default() }
    }

    pub fn key(provider: &str, window: &str) -> String {
        format!("{provider}/{window}")
    }

    /// Örnek ekler. Bir dakikadan yakın örnekler birleşir (el ile yenilemeler şişirmesin).
    pub fn record(&mut self, key: &str, ts: i64, used: u8) {
        let list = self.series.entry(key.to_string()).or_default();
        match list.last_mut() {
            Some(last) if (ts - last.0).abs() < 60 => *last = (ts, used),
            Some(last) if ts < last.0 => {}
            _ => list.push((ts, used)),
        }
        list.retain(|(t, _)| ts - t <= HISTORY_KEEP_SECS);
    }

    pub fn save(&self) {
        if let Some(f) = &self.file {
            write_json(f, &self.series);
        }
    }

    /// Bu hızla pencerenin ne zaman dolacağı (saniye): pencere başından (ya da
    /// son 2 saatten) beri kullanım en az 15 dakikadır artıyorsa.
    pub fn pace_eta(&self, key: &str, window_start: i64, now: i64, used: u8) -> Option<u64> {
        let list = self.series.get(key)?;
        let since = window_start.max(now - 2 * 3600);
        let recent: Vec<&(i64, u8)> = list.iter().filter(|(t, _)| *t >= since && *t <= now).collect();
        let (&(t0, u0), &(t1, u1)) = (*recent.first()?, *recent.last()?);
        if t1 - t0 < 15 * 60 || u1 <= u0 || used >= 100 {
            return None;
        }
        let per_sec = (u1 - u0) as f64 / (t1 - t0) as f64;
        Some(((100 - used) as f64 / per_sec) as u64)
    }

    /// `[from, to]` aralığını `buckets` eşit parçaya böler; her parçanın değeri o
    /// ana kadarki son örnektir (ölçüm yoksa `None`).
    pub fn resample(&self, key: &str, from: i64, to: i64, buckets: usize) -> Vec<Option<u8>> {
        let Some(list) = self.series.get(key) else { return vec![None; buckets] };
        if buckets == 0 || to <= from {
            return Vec::new();
        }
        let span = (to - from) as f64 / buckets as f64;
        (0..buckets)
            .map(|b| {
                let end = from + (span * (b + 1) as f64) as i64;
                list.iter().rev().find(|(t, _)| *t <= end && *t >= from - 6 * 3600).map(|(_, u)| *u)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_history_merges_and_resamples() {
        let mut h = UsageHistory::memory();
        let k = UsageHistory::key("claude", "5H");
        h.record(&k, 1_000, 10);
        h.record(&k, 1_030, 12);
        h.record(&k, 4_600, 40);
        assert_eq!(h.series[&k], vec![(1_030, 12), (4_600, 40)]);
        let r = h.resample(&k, 0, 7_200, 4);
        assert_eq!(r, vec![Some(12), Some(12), Some(40), Some(40)]);
        h.record(&k, 1_000 + HISTORY_KEEP_SECS + 5_000, 5);
        assert_eq!(h.series[&k].len(), 1);
        assert_eq!(h.resample("x/none", 0, 10, 2), vec![None, None]);
        // 30 dakikada %10 → %40 kalan 2 saatte dolar.
        let mut p = UsageHistory::memory();
        p.record("c/5H", 10_000, 50);
        p.record("c/5H", 11_800, 60);
        assert_eq!(p.pace_eta("c/5H", 9_000, 11_800, 60), Some(7_200));
        assert_eq!(p.pace_eta("c/5H", 11_000, 11_800, 60), None, "only one sample in this window");
    }

    #[test]
    fn frecency_prefers_frequent_and_recent() {
        let t = 1_000_000;
        let old_frequent = RecentEntry { path: "a".into(), count: 10, last: t - 86_400 * 3 };
        let fresh_once = RecentEntry { path: "b".into(), count: 1, last: t };
        let fresh_often = RecentEntry { path: "c".into(), count: 5, last: t - 60 };
        assert!(fresh_often.score(t) > old_frequent.score(t));
        assert!(old_frequent.score(t) > fresh_once.score(t));
    }

    #[test]
    fn recent_record_and_roundtrip() {
        let dir = std::env::temp_dir().join(format!("noble-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("recent.json");
        let mut r = Recent::load(file.clone());
        r.record(&std::env::temp_dir());
        r.record(&std::env::temp_dir());
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].count, 2);
        let again = Recent::load(file);
        assert_eq!(again.entries.len(), 1);
        assert_eq!(again.top(5).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn workspace_serialization() {
        let ws = Workspace {
            name: "work".into(),
            saved_at: 0,
            tabs: vec![SavedTab {
                name: None,
                origin: "noble".into(),
                layout: SavedNode::Split {
                    dir: crate::term::layout::Dir::Row,
                    ratio: 0.5,
                    a: Box::new(SavedNode::Leaf { cwd: "C:\\a".into() }),
                    b: Box::new(SavedNode::Leaf { cwd: "C:\\b".into() }),
                },
                focus: 1,
            }],
        };
        let text = serde_json::to_string(&ws).unwrap();
        let back: Workspace = serde_json::from_str(&text).unwrap();
        assert_eq!(back, ws);
        assert_eq!(back.pane_count(), 2);
    }
}
