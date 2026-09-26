//! Small persistent data: recent dirs (frecency), session and saved workspaces.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::term::layout::SavedNode;

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn read_json<T: for<'de> Deserialize<'de>>(file: &Path) -> Option<T> {
    let text = std::fs::read_to_string(file).ok()?;
    serde_json::from_str(crate::util::strip_bom(&text)).ok()
}

/// Atomic write: temp file first, then rename.
fn write_json<T: Serialize>(file: &Path, value: &T) {
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(value) {
        // A name of its own per write: windows saving the same file at once must not share it.
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let tmp =
            file.with_extension(format!("json.{}-{}.tmp", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
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
    /// Frequency × recency: a use from a day ago counts at half weight.
    pub fn score(&self, now: i64) -> f64 {
        let hours = (now.saturating_sub(self.last).max(0) as f64) / 3600.0;
        self.count as f64 / (1.0 + hours / 24.0)
    }
}

pub struct Recent {
    file: Option<PathBuf>,
    pub entries: Vec<RecentEntry>,
    /// Score-sorted list of existing directories (to avoid hitting the disk every frame).
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
        // Drop the lowest scoring entries.
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

    /// Existing directories, by score.
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
    /// Index of the focused pane in leaf order.
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

// ─── Session: one file shared by every window of a build ─────────────────

/// A tab in the session file, tagged with the window that saved it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SessionTab {
    /// The window's `instance_id`; empty in files written by older versions.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub instance: String,
    #[serde(flatten)]
    pub tab: SavedTab,
}

/// `session.json` (`session-dev.json` for `noble-dev`): the tabs of every window, merged. A
/// file from an older version (a plain `Workspace`) loads as well.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Session {
    #[serde(default)]
    pub saved_at: i64,
    /// Windows running right now (`instance_id`s). While one of them is alive, a newly opened
    /// window is not the first of the run and does not restore.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub running: Vec<String>,
    #[serde(default)]
    pub tabs: Vec<SessionTab>,
}

/// An id for one window: `<pid>-<process start time>-<n>`. The start time tells a live window
/// from a dead one whose pid was reused; `n` tells apart several apps in one process (tests).
pub fn instance_id() -> String {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let pid = std::process::id();
    let start = crate::util::process_start(pid).unwrap_or(0);
    format!("{pid}-{start}-{}", SEQ.fetch_add(1, Ordering::Relaxed))
}

/// Is the window with this id still running? (Its process exists with the same start time.)
fn instance_alive(id: &str) -> bool {
    let mut parts = id.split('-').map(|p| p.parse::<u64>().ok());
    let (Some(Some(pid)), Some(Some(start))) = (parts.next(), parts.next()) else { return false };
    let Ok(pid) = u32::try_from(pid) else { return false };
    crate::util::process_start(pid).is_some_and(|s| start == 0 || s == start)
}

pub fn load_session(file: &Path) -> Option<Session> {
    read_json(file)
}

/// A window starts and registers itself as running. The first window of a run (no other
/// window of this build alive) gets every saved tab back to restore, and takes them over so a
/// crash before its next save keeps them; a later window gets `None` and starts empty.
pub fn session_begin(file: &Path, me: &str) -> Option<Vec<SavedTab>> {
    with_lock(file, || {
        let mut session: Session = read_json(file).unwrap_or_default();
        session.running.retain(|id| id != me && instance_alive(id));
        let first = session.running.is_empty();
        session.running.push(me.to_string());
        let restore = first.then(|| {
            session.tabs.iter_mut().for_each(|t| t.instance = me.to_string());
            session.tabs.iter().map(|t| t.tab.clone()).collect()
        });
        write_json(file, &session);
        restore
    })
}

/// Merges a window's tabs into the session file: replaces what this window saved before and
/// keeps the other windows' tabs. `leaving`: the window is closing.
pub fn session_save(file: &Path, me: &str, tabs: Vec<SavedTab>, leaving: bool) {
    with_lock(file, || {
        let mut session: Session = read_json(file).unwrap_or_default();
        session.tabs.retain(|t| t.instance != me);
        session.tabs.extend(tabs.into_iter().map(|tab| SessionTab { instance: me.to_string(), tab }));
        if leaving {
            session.running.retain(|id| id != me);
        }
        session.saved_at = now();
        if session.tabs.is_empty() && session.running.is_empty() {
            let _ = std::fs::remove_file(file);
        } else {
            write_json(file, &session);
        }
    })
}

/// Runs `f` while holding `<file>.lock`, so windows that close at the same time do not lose
/// each other's tabs. A lock older than 10 s is left over from a crash and taken over; after
/// 2 s of waiting (or when the lock cannot be created at all) `f` runs anyway.
fn with_lock<T>(file: &Path, f: impl FnOnce() -> T) -> T {
    let lock = file.with_extension("json.lock");
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    let held = loop {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&lock) {
            Ok(_) => break true,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let stale = std::fs::metadata(&lock)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age > Duration::from_secs(10));
                if stale {
                    let _ = std::fs::remove_file(&lock);
                } else if Instant::now() > deadline {
                    break false;
                } else {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            Err(_) => break false,
        }
    };
    let out = f();
    if held {
        let _ = std::fs::remove_file(&lock);
    }
    out
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

    /// Replaces the entry with the same name, or inserts it at the top.
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

/// UI state: whether the welcome screen was seen, pinned projects.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiStateData {
    pub welcomed: bool,
    pub pins: Vec<String>,
    /// Last update check (unix seconds) and the version found.
    pub update_checked: i64,
    pub update_latest: String,
    /// The version whose notice was dismissed: hidden until the next release.
    pub update_skipped: String,
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

    /// Toggles the pin; returns the new state.
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

/// AI quota usage history: "provider/window" → (time, percent) samples.
/// Kept for the small graph on Home; anything older than a few days is dropped.
pub struct UsageHistory {
    file: Option<PathBuf>,
    pub series: std::collections::BTreeMap<String, Vec<(i64, u8)>>,
}

/// How long the history is kept.
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

    /// Adds a sample. Samples within a minute are merged (so manual refreshes do not bloat it).
    pub fn record(&mut self, key: &str, ts: i64, used: u8) {
        let list = self.series.entry(key.to_string()).or_default();
        match list.last_mut() {
            Some(last) if ts.abs_diff(last.0) < 60 => *last = (ts, used),
            Some(last) if ts < last.0 => {}
            _ => list.push((ts, used)),
        }
        list.retain(|(t, _)| ts.saturating_sub(*t) <= HISTORY_KEEP_SECS);
    }

    pub fn save(&self) {
        if let Some(f) = &self.file {
            write_json(f, &self.series);
        }
    }

    /// When the window fills at this pace (seconds): only while usage has been
    /// rising for at least 15 minutes (since the window start or the last 2 hours).
    pub fn pace_eta(&self, key: &str, window_start: i64, now: i64, used: u8) -> Option<u64> {
        let list = self.series.get(key)?;
        let since = window_start.max(now.saturating_sub(2 * 3600));
        let recent: Vec<&(i64, u8)> = list.iter().filter(|(t, _)| *t >= since && *t <= now).collect();
        let (&(t0, u0), &(t1, u1)) = (*recent.first()?, *recent.last()?);
        if t1.saturating_sub(t0) < 15 * 60 || u1 <= u0 || used >= 100 {
            return None;
        }
        let per_sec = (u1 - u0) as f64 / t1.saturating_sub(t0) as f64;
        Some(((100 - used) as f64 / per_sec) as u64)
    }

    /// Splits the `[from, to]` range into `buckets` equal parts; each part takes
    /// the last sample up to that moment (`None` when there is no measurement).
    pub fn resample(&self, key: &str, from: i64, to: i64, buckets: usize) -> Vec<Option<u8>> {
        let Some(list) = self.series.get(key) else { return vec![None; buckets] };
        if buckets == 0 || to <= from {
            return Vec::new();
        }
        let span = to.saturating_sub(from) as f64 / buckets as f64;
        (0..buckets)
            .map(|b| {
                let end = from.saturating_add((span * (b + 1) as f64) as i64);
                list.iter().rev().find(|(t, _)| *t <= end && *t >= from.saturating_sub(6 * 3600)).map(|(_, u)| *u)
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

    /// Corrupt or hostile data files load as empty/defaults (or as the valid values they hold),
    /// and nothing that uses them afterwards panics.
    #[test]
    fn corrupt_files_never_panic() {
        let dir = std::env::temp_dir().join(format!("noble-store-corrupt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("data.json");
        let bodies: [&[u8]; 14] = [
            b"",
            b"\xef\xbb\xbf",
            b"\xff\xfe\x00{",
            b"{",
            b"[{\"path\":",
            b"null",
            b"42",
            b"{\"welcomed\": \"yes\", \"pins\": [1, 2]}",
            b"{\"pins\": [\"\xc3\xa9\xe6\x97\xa5\", \"\"], \"update_checked\": -9223372036854775808}",
            b"{\"update_checked\": 9223372036854775807, \"update_latest\": \"v\xf0\x9f\x99\x82.\"}",
            b"[{\"path\": \"/\", \"count\": 4294967295, \"last\": -9223372036854775808}]",
            b"[{\"path\": \"/\", \"count\": 1, \"last\": 9223372036854775807}]",
            b"{\"claude/5h\": [[-9223372036854775808, 255], [9223372036854775807, 0], [0, 200]]}",
            b"[{\"name\": \"w\", \"saved_at\": 0, \"tabs\": [{\"name\": null, \"origin\": \"\", \"focus\": 99, \
               \"layout\": {\"type\": \"split\", \"dir\": \"Row\", \"ratio\": -1e39, \"a\": {\"type\": \"leaf\", \
               \"cwd\": \"\"}, \"b\": {\"type\": \"leaf\", \"cwd\": \"\\u0000\"}}}]}]",
        ];
        for body in bodies {
            std::fs::write(&file, body).unwrap();
            let mut recent = Recent::load(file.clone());
            let _ = recent.top(5);
            recent.file = None;
            recent.record(&dir);
            let _ = load_session(&file).map(|s| s.tabs.len());
            let ws = Workspaces::load(file.clone());
            let panes = ws.list.iter().map(Workspace::pane_count).sum::<usize>();
            if body.starts_with(b"[{\"name\"") {
                assert_eq!(panes, 2, "the damaged but well-formed workspace still loads");
            }
            let mut ui = UiState::load(file.clone());
            ui.file = None;
            let _ = ui.is_pinned(&dir);
            ui.toggle_pin(&dir);
            let mut h = UsageHistory::load(file.clone());
            h.file = None;
            let t = now();
            let _ = h.pace_eta("claude/5h", t - 3600, t, 50);
            let _ = h.pace_eta("claude/5h", i64::MIN, t, 99);
            let _ = h.resample("claude/5h", t - 5 * 3600, t, 40);
            h.record("claude/5h", t, 30);
            // Startup and shutdown of the session merge on the same damaged file (they rewrite it).
            let _ = session_begin(&file, "1-1-0");
            session_save(&file, "1-1-0", Vec::new(), true);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn instance_ids_tell_live_windows_from_dead_ones() {
        let me = instance_id();
        assert!(instance_alive(&me), "{me}");
        assert_ne!(instance_id(), me, "two windows in one process get different ids");
        assert!(!instance_alive("4000000000-1-0"));
        // Our pid, but another start time: the pid was reused, the window is gone.
        assert!(!instance_alive(&format!("{}-1-0", std::process::id())));
        assert!(!instance_alive("garbage"));
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
                    a: Box::new(SavedNode::Leaf { cwd: "C:\\a".into(), launch: Some("claude".into()) }),
                    b: Box::new(SavedNode::Leaf { cwd: "C:\\b".into(), launch: None }),
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
