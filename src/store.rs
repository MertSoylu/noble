//! Small persistent data: recent dirs (frecency), session, saved workspaces, UI state (`UiState`: welcome,
//! pinned/hidden/added projects, update notice) and AI usage history (`UsageHistory`).

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
pub(crate) fn write_json<T: Serialize>(file: &Path, value: &T) {
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(value) {
        // A name of its own per write: windows saving the same file at once must not share it.
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let tmp =
            file.with_extension(format!("json.{}-{}.tmp", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
        // A failed write (disk full) or rename (target locked on Windows, a directory in its
        // place) must not leave the temp file behind.
        if std::fs::write(&tmp, text).is_err() || std::fs::rename(&tmp, file).is_err() {
            let _ = std::fs::remove_file(&tmp);
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
        match self.entries.iter_mut().find(|e| crate::util::same_path(Path::new(&e.path), path)) {
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
        self.entries.retain(|e| !crate::util::same_path(Path::new(&e.path), Path::new(path)));
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

/// How long a window waits for another one's session lock before it saves without it.
const LOCK_WAIT: Duration = Duration::from_secs(15);
/// A lock whose owner is still running is only taken over once it is this old (the owner hangs).
const LOCK_STALE_LIVE: Duration = Duration::from_secs(60);
/// A lock that names no owner (being written right now, or left by an older NOBLE) is taken
/// over once it is this old.
const LOCK_STALE_UNKNOWN: Duration = Duration::from_secs(10);

/// Runs `f` while holding `<file>.lock`, so windows that close at the same time do not lose
/// each other's tabs. See `with_lock_waiting`.
fn with_lock<T>(file: &Path, f: impl FnOnce() -> T) -> T {
    with_lock_waiting(file, LOCK_WAIT, f)
}

/// The lock names its owner (`<pid>-<process start time>`, as in `instance_id`). A lock whose
/// owner has exited (crash, kill) is taken over at once; one held by a running process is
/// waited for, up to `wait`, and taken over only when it is older than `LOCK_STALE_LIVE`.
/// After `wait`, or when no lock can be created at all, `f` runs anyway: saving without the
/// lock beats never saving (or never closing the window).
fn with_lock_waiting<T>(file: &Path, wait: Duration, f: impl FnOnce() -> T) -> T {
    use std::io::Write;
    let lock = file.with_extension("json.lock");
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let started = Instant::now();
    let deadline = started + wait;
    // Asking the OS about the owner costs more than trying to create the file: at most 4 times a second.
    let mut next_check = Instant::now();
    let held = loop {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&lock) {
            Ok(mut handle) => {
                let _ = handle.write_all(lock_owner().as_bytes());
                break true;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if Instant::now() >= next_check {
                    next_check = Instant::now() + Duration::from_millis(250);
                    if lock_is_stale(&lock) {
                        let _ = std::fs::remove_file(&lock);
                        continue;
                    }
                }
                if Instant::now() > deadline {
                    break false;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            // Windows: a lock that was just removed while someone still had it open lingers
            // for a moment and refuses to be created again. Unix frees the name at once, so
            // there (as for a directory that is not writable) this is final.
            Err(e)
                if cfg!(windows)
                    && e.kind() == std::io::ErrorKind::PermissionDenied
                    && started.elapsed() < Duration::from_secs(2) =>
            {
                std::thread::sleep(Duration::from_millis(20));
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

/// What this process writes into a lock it holds.
fn lock_owner() -> String {
    let pid = std::process::id();
    format!("{pid}-{}", crate::util::process_start(pid).unwrap_or(0))
}

/// May the lock be taken over? Yes when its owner is gone, or when it is too old (see the
/// `LOCK_STALE_*` limits). A lock that cannot be read counts as just created.
fn lock_is_stale(lock: &Path) -> bool {
    let age = std::fs::metadata(lock)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .unwrap_or(Duration::ZERO);
    let owner = std::fs::read_to_string(lock).unwrap_or_default();
    let owner = owner.trim();
    let named = owner.split('-').count() == 2 && owner.split('-').all(|p| p.parse::<u64>().is_ok());
    if !named { age > LOCK_STALE_UNKNOWN } else { !instance_alive(owner) || age > LOCK_STALE_LIVE }
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

/// UI state: whether the welcome screen was seen, pinned, hidden and manually added projects.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiStateData {
    pub welcomed: bool,
    /// The prefix key was pressed at least once: the status bar stops pointing it out.
    pub prefix_used: bool,
    pub pins: Vec<String>,
    /// Projects removed from the list: the scan skips these paths.
    pub hidden: Vec<String>,
    /// Folders added as projects by hand: always listed, even when the scan does not find them.
    pub added: Vec<String>,
    /// Last update check (unix seconds) and the version found.
    pub update_checked: i64,
    pub update_latest: String,
    /// The version whose notice was dismissed: hidden until the next release.
    pub update_skipped: String,
}

impl UiStateData {
    pub fn is_pinned(&self, path: &Path) -> bool {
        self.pins.iter().any(|p| crate::util::same_path(Path::new(p), path))
    }

    pub fn is_hidden(&self, path: &Path) -> bool {
        self.hidden.iter().any(|h| crate::util::same_path(Path::new(h), path))
    }
}

/// How long a UI state write waits for another window's lock (normally held for milliseconds)
/// before it writes without it: the main thread must not stall for long.
const UI_LOCK_WAIT: Duration = Duration::from_secs(2);

/// `state.json`, shared by every window. Each change is a read-modify-write under
/// `state.json.lock` (`modify`), so a window never overwrites what another one wrote since it
/// started (its pins, hidden or added projects, the update check …).
pub struct UiState {
    file: Option<PathBuf>,
    /// The state as last read or written. Change it only through `modify`.
    pub data: UiStateData,
}

impl UiState {
    pub fn load(file: PathBuf) -> UiState {
        UiState { data: read_json(&file).unwrap_or_default(), file: Some(file) }
    }

    pub fn memory() -> UiState {
        UiState { file: None, data: UiStateData::default() }
    }

    /// Applies one change: under the lock the file is read again (other windows may have
    /// written it), `f` changes only what it is about, and the result is written atomically
    /// and becomes the in-memory state, other windows' changes included. A missing or
    /// unreadable file starts from the in-memory state. Without a file (tests) only memory changes.
    pub fn modify<T>(&mut self, f: impl FnOnce(&mut UiStateData) -> T) -> T {
        let Some(file) = self.file.clone() else { return f(&mut self.data) };
        let current = &self.data;
        let (data, out) = with_lock_waiting(&file, UI_LOCK_WAIT, || {
            let mut data: UiStateData = read_json(&file).unwrap_or_else(|| current.clone());
            let out = f(&mut data);
            write_json(&file, &data);
            (data, out)
        });
        self.data = data;
        out
    }

    pub fn is_pinned(&self, path: &Path) -> bool {
        self.data.is_pinned(path)
    }

    /// Toggles the pin; returns the new state.
    pub fn toggle_pin(&mut self, path: &Path) -> bool {
        let key = path.to_string_lossy().into_owned();
        self.modify(|d| {
            if d.is_pinned(path) {
                d.pins.retain(|p| !crate::util::same_path(Path::new(p), path));
                false
            } else {
                d.pins.push(key);
                true
            }
        })
    }

    pub fn is_hidden(&self, path: &Path) -> bool {
        self.data.is_hidden(path)
    }

    /// Removes a project from the list for good: hidden from every scan, unpinned and
    /// dropped from the manually added folders.
    pub fn hide_project(&mut self, path: &Path) {
        let key = path.to_string_lossy().into_owned();
        self.modify(|d| {
            d.pins.retain(|p| !crate::util::same_path(Path::new(p), path));
            d.added.retain(|a| !crate::util::same_path(Path::new(a), path));
            if !d.is_hidden(path) {
                d.hidden.push(key);
            }
        })
    }

    /// Adds a folder as a project by hand (and shows it again if it was hidden).
    /// Returns false when it was already added.
    pub fn add_project(&mut self, path: &Path) -> bool {
        self.modify(|d| {
            let was_hidden = d.is_hidden(path);
            d.hidden.retain(|h| !crate::util::same_path(Path::new(h), path));
            let known = d.added.iter().any(|a| crate::util::same_path(Path::new(a), path));
            if !known {
                d.added.push(path.to_string_lossy().into_owned());
            }
            !known || was_hidden
        })
    }

    /// Hidden and manually added projects, as the project scan applies them.
    pub fn manual_projects(&self) -> crate::projects::Manual {
        crate::projects::Manual {
            hidden: self.data.hidden.clone(),
            added: self.data.added.iter().filter(|a| !a.is_empty()).map(PathBuf::from).collect(),
        }
    }

    /// Is this folder one of the manually added projects?
    pub fn is_added(&self, path: &Path) -> bool {
        self.data.added.iter().any(|a| !a.is_empty() && crate::util::same_path(Path::new(a), path))
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

    /// Removed and manually added projects survive a restart; removing unpins, adding a
    /// removed project brings it back, and paths compare like the file system does.
    #[test]
    fn hidden_and_added_projects_round_trip() {
        let dir = std::env::temp_dir().join(format!("noble-manual-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("state.json");
        let (a, b) = (dir.join("alpha"), dir.join("beta"));
        let mut ui = UiState::load(file.clone());
        ui.toggle_pin(&a);
        ui.hide_project(&a);
        assert!(ui.is_hidden(&a) && !ui.is_pinned(&a), "removing unpins");
        assert!(ui.add_project(&b));
        assert!(!ui.add_project(&b), "added once");
        let back = UiState::load(file.clone());
        assert!(back.is_hidden(&a));
        assert_eq!(back.manual_projects().added, vec![b.clone()]);
        assert_eq!(back.manual_projects().hidden, vec![a.to_string_lossy().into_owned()]);
        // A trailing separator names the same folder.
        assert!(back.is_hidden(Path::new(&format!("{}{}", a.display(), std::path::MAIN_SEPARATOR))));
        // Windows and macOS ignore case in paths; Linux does not.
        let upper = PathBuf::from(a.to_string_lossy().to_uppercase());
        assert_eq!(back.is_hidden(&upper), cfg!(any(windows, target_os = "macos")));
        // Adding a removed project shows it again; removing an added one forgets it.
        assert!(ui.add_project(&a));
        assert!(!ui.is_hidden(&a));
        ui.hide_project(&b);
        let back = UiState::load(file);
        assert_eq!(back.manual_projects().added, vec![a.clone()]);
        assert!(back.is_hidden(&b) && !back.is_hidden(&a));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two windows share `state.json`: a change made by one survives a later, unrelated write
    /// by the other (which loaded the file before that change), and the other window sees it.
    #[test]
    fn ui_state_writes_keep_other_windows_changes() {
        let dir = std::env::temp_dir().join(format!("noble-ui-merge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("state.json");
        let (a, b, c) = (dir.join("alpha"), dir.join("beta"), dir.join("gamma"));
        let mut one = UiState::load(file.clone());
        let mut two = UiState::load(file.clone());
        one.hide_project(&a);
        one.add_project(&b);
        one.toggle_pin(&b);
        // The second window still holds the state from before and writes unrelated fields.
        two.modify(|d| d.prefix_used = true);
        two.modify(|d| {
            d.update_checked = 42;
            d.update_latest = "9.9.9".into();
        });
        two.modify(|d| d.update_skipped = "9.9.9".into());
        two.toggle_pin(&c);
        assert!(two.is_hidden(&a) && two.is_pinned(&b), "the other window's changes are read back");
        let back = UiState::load(file.clone());
        assert!(back.is_hidden(&a), "hidden project survived");
        assert_eq!(back.manual_projects().added, vec![b.clone()]);
        assert!(back.is_pinned(&b) && back.is_pinned(&c));
        assert!(back.data.prefix_used);
        assert_eq!((back.data.update_checked, back.data.update_skipped.as_str()), (42, "9.9.9"));
        // And the other way round: the first window's next write keeps the second one's fields.
        one.modify(|d| d.welcomed = true);
        let back = UiState::load(file.clone());
        assert!(back.data.welcomed && back.data.prefix_used && back.is_pinned(&c));
        assert!(!file.with_extension("json.lock").exists(), "the lock is released");
        // Many windows writing at once: every change lands.
        let writers: Vec<_> = (0..6)
            .map(|i| {
                let (file, path) = (file.clone(), dir.join(format!("p{i}")));
                std::thread::spawn(move || UiState::load(file).hide_project(&path))
            })
            .collect();
        writers.into_iter().for_each(|w| w.join().unwrap());
        let back = UiState::load(file);
        assert!((0..6).all(|i| back.is_hidden(&dir.join(format!("p{i}")))));
        let _ = std::fs::remove_dir_all(&dir);
    }

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
        // 10% in 30 minutes with 40% left: the quota runs out in 2 hours at that pace.
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

    /// Recent entries and pins compare paths like the file system does (`util::same_path`): a trailing
    /// separator is the same folder everywhere, a different case only on Windows and macOS.
    #[test]
    fn recent_and_pins_compare_paths_platform_aware() {
        let base = std::env::temp_dir().join("Noble-Same-Path");
        let slash = PathBuf::from(format!("{}{}", base.display(), std::path::MAIN_SEPARATOR));
        let upper = PathBuf::from(base.to_string_lossy().to_uppercase());
        let ignores_case = cfg!(any(windows, target_os = "macos"));

        let mut r = Recent::memory();
        r.record(&base);
        r.record(&slash);
        assert_eq!(r.entries.len(), 1, "a trailing separator names the same folder");
        assert_eq!(r.entries[0].count, 2);
        r.record(&upper);
        assert_eq!(r.entries.len(), if ignores_case { 1 } else { 2 }, "case only matters on Linux");
        r.remove(&slash.to_string_lossy());
        assert_eq!(r.entries.len(), if ignores_case { 0 } else { 1 });

        let mut ui = UiState::memory();
        assert!(ui.toggle_pin(&base));
        assert!(ui.is_pinned(&slash));
        assert_eq!(ui.is_pinned(&upper), ignores_case);
        assert!(!ui.toggle_pin(&slash), "unpinned through the other spelling");
        assert!(ui.data.pins.is_empty());
    }

    /// A write that cannot be moved into place (a directory sits at the target, on every
    /// platform) leaves the old content and no temp file behind; a normal write leaves none either.
    #[test]
    fn write_json_cleans_up_its_temp_file() {
        let dir = std::env::temp_dir().join(format!("noble-store-tmp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let blocked = dir.join("blocked.json");
        std::fs::create_dir_all(blocked.join("inside")).unwrap();
        write_json(&blocked, &[1, 2, 3]);
        assert!(blocked.is_dir(), "the directory in the way is untouched");
        let ok = dir.join("ok.json");
        write_json(&ok, &[1, 2, 3]);
        assert_eq!(read_json::<Vec<i32>>(&ok), Some(vec![1, 2, 3]));
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files left behind: {leftovers:?}");
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

    fn lock_dir(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("noble-lock-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("session.json");
        let lock = file.with_extension("json.lock");
        (dir, file, lock)
    }

    /// The lock names its owner; a lock left by a process that is gone (or by a pid that was
    /// reused) is taken over at once instead of after a timeout.
    #[test]
    fn session_lock_takes_over_from_dead_owners() {
        let (dir, file, lock) = lock_dir("dead");
        let seen = with_lock(&file, || std::fs::read_to_string(&lock).unwrap());
        assert_eq!(seen, lock_owner());
        assert!(!lock.exists(), "the lock is released after use");
        for dead in ["4000000000-1".to_string(), format!("{}-1", std::process::id())] {
            std::fs::write(&lock, &dead).unwrap();
            let started = Instant::now();
            let ran = with_lock_waiting(&file, Duration::from_secs(30), || lock.exists());
            assert!(ran, "{dead}: f runs holding the lock");
            assert!(started.elapsed() < Duration::from_secs(5), "{dead}: waited {:?}", started.elapsed());
            assert!(!lock.exists());
        }
        // A lock with no owner in it (an older NOBLE, or one being written) counts only by age.
        std::fs::write(&lock, "").unwrap();
        assert!(!lock_is_stale(&lock));
        let old = std::time::SystemTime::now() - Duration::from_secs(120);
        std::fs::File::options().write(true).open(&lock).unwrap().set_modified(old).unwrap();
        assert!(lock_is_stale(&lock));
        // A running owner's lock only when it is very old.
        std::fs::write(&lock, lock_owner()).unwrap();
        assert!(!lock_is_stale(&lock));
        std::fs::File::options().write(true).open(&lock).unwrap().set_modified(old).unwrap();
        assert!(lock_is_stale(&lock));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A lock held by a running window is waited for (longer than the old 2 s limit), so two
    /// windows closing together both keep their tabs; one that is never released still lets
    /// the save go through once the wait is over.
    #[test]
    fn session_lock_waits_for_a_live_owner() {
        let (dir, file, lock) = lock_dir("live");
        std::fs::write(&lock, lock_owner()).unwrap();
        let release = {
            let lock = lock.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(2500));
                std::fs::remove_file(&lock).unwrap();
            })
        };
        let started = Instant::now();
        // The other window's lock is gone by the time `f` runs, and `f` holds its own.
        let owner = with_lock(&file, || std::fs::read_to_string(&lock).unwrap_or_default());
        assert!(started.elapsed() >= Duration::from_millis(2400), "ran after {:?}", started.elapsed());
        assert_eq!(owner, lock_owner());
        release.join().unwrap();
        assert!(!lock.exists());

        // Never released: no deadlock, and the other window's lock stays in place.
        std::fs::write(&lock, lock_owner()).unwrap();
        let started = Instant::now();
        assert!(with_lock_waiting(&file, Duration::from_millis(400), || true));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(lock.exists(), "a live owner's lock is not removed");

        // Two savers at once: both tabs end up in the session.
        std::fs::remove_file(&lock).unwrap();
        let tab = |origin: &str| SavedTab {
            name: None,
            origin: origin.into(),
            layout: SavedNode::Leaf { cwd: ".".into(), launch: None },
            focus: 0,
        };
        let savers: Vec<_> = ["a", "b", "c", "d"]
            .into_iter()
            .map(|who| {
                let (file, tab) = (file.clone(), tab(who));
                std::thread::spawn(move || session_save(&file, who, vec![tab], false))
            })
            .collect();
        savers.into_iter().for_each(|s| s.join().unwrap());
        let session = load_session(&file).unwrap();
        let mut origins: Vec<_> = session.tabs.iter().map(|t| t.tab.origin.as_str()).collect();
        origins.sort();
        assert_eq!(origins, ["a", "b", "c", "d"]);
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
