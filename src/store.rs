//! Small persistent data: recent dirs (frecency), session, saved workspaces, UI state (`UiState`: welcome,
//! pinned/hidden/added projects, update notice) and AI usage history (`UsageHistory`).

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
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

/// Atomic text write (see [`write_bytes`]).
pub(crate) fn write_text(file: &Path, text: &str) -> std::io::Result<()> {
    write_bytes(file, text.as_bytes())
}

/// Atomic write of a whole file: the bytes go to a temp file next to the target, which is flushed to disk
/// (`sync_all`), then renamed over the target, and the folder is flushed too (Unix). Every error of the write
/// itself is returned, and a failed write leaves neither a temp file nor a half-written target behind; after a
/// power loss the target holds either the old or the new content, never a truncated one. The folder flush comes
/// after the rename and is best effort: some filesystems do not support it, the new content is in place by then,
/// so its error is ignored (only the durability of the rename across a crash depends on it).
/// A symlinked file (a dotfiles manager's `config.toml`) is written through, so the link stays a link,
/// and the new file keeps the old one's permissions (a private 0600 file stays private). A dangling link
/// has no target: that is an error, not a reason to replace the link by a regular file.
pub(crate) fn write_bytes(file: &Path, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
    let target = write_target(file)?;
    if let Some(parent) = target.parent() {
        let _ = crate::util::create_private_dir(parent);
    }
    // A name of its own per write: windows saving the same file at once must not share it.
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let ext = target.extension().and_then(|e| e.to_str()).unwrap_or("txt");
    let tmp =
        target.with_extension(format!("{ext}.{}-{}.tmp", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    // A failed write (disk full), flush or rename (target locked on Windows, a directory in its place) must
    // not leave the temp file behind.
    let result = create_new_file(&tmp, bytes)
        .and_then(|()| keep_permissions(&target, &tmp))
        .and_then(|()| replace_file(&tmp, &target));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    } else {
        // Best effort: the new content is already in place; only its durability across a crash depends on this.
        let _ = sync_dir(target.parent().unwrap_or(Path::new(".")));
    }
    result
}

/// The file a write goes to: the target of a symlink (the link itself is kept). A dangling link is an error.
fn write_target(file: &Path) -> std::io::Result<PathBuf> {
    match std::fs::symlink_metadata(file) {
        Ok(m) if m.file_type().is_symlink() => std::fs::canonicalize(file).map_err(|e| {
            std::io::Error::new(e.kind(), format!("{} is a link whose target cannot be reached: {e}", file.display()))
        }),
        _ => Ok(file.to_path_buf()),
    }
}

/// Writes `bytes` to a file that must not exist yet (a temp file with a name of its own), and flushes them to
/// disk before returning. Unix: 0600 from the start (`util::private_file_options`), so `keep_permissions` only
/// ever gives it the mode of the file it replaces.
pub(crate) fn create_new_file(path: &Path, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = crate::util::private_file_options().write(true).create_new(true).open(path)?;
    file.write_all(bytes.as_ref())?;
    file.sync_all()
}

/// Renames `tmp` over `file`. Windows: a rename over a file another process has open fails for a moment (a
/// sharing violation), so it is retried with short pauses, about 150 ms in all, before the error is returned.
/// Unix: the rename replaces the file in one step whatever else has it open, so it is tried once.
pub(crate) fn replace_file(tmp: &Path, file: &Path) -> std::io::Result<()> {
    let mut result = std::fs::rename(tmp, file);
    if cfg!(windows) {
        for pause in [10, 20, 40, 80] {
            if result.is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(pause));
            result = std::fs::rename(tmp, file);
        }
    }
    result
}

/// Flushes a folder, so a rename into it survives a power loss. Unix: `fsync` on the folder. Windows: std
/// cannot open a folder to flush it; the file itself is flushed before the rename (`create_new_file`) and
/// NTFS journals the rename, so nothing is done there.
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    if cfg!(unix) { std::fs::File::open(dir)?.sync_all() } else { Ok(()) }
}

/// Gives `tmp` the mode bits of the file it is about to replace. Unix: a file that does not exist yet keeps
/// the 0600 it was created with, and an existing one hands its own mode over (a private 0600 file stays
/// private, and nothing is loosened). Windows: the file's ACL comes from its folder and the
/// read-only attribute must not be copied (it would block the next save), so nothing is done.
fn keep_permissions(file: &Path, tmp: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    if let Ok(meta) = std::fs::metadata(file) {
        std::fs::set_permissions(tmp, meta.permissions())?;
    }
    #[cfg(not(unix))]
    let _ = (file, tmp);
    Ok(())
}

/// Atomic JSON write (see [`write_text`]); errors are ignored.
pub(crate) fn write_json<T: Serialize>(file: &Path, value: &T) {
    if let Ok(text) = serde_json::to_string_pretty(value) {
        let _ = write_text(file, &text);
    }
}

/// Messages about store files that could not be read, kept aside or saved, waiting for the App to show them
/// (`take_notices`).
static NOTICES: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Every message sent in this run. A condition that repeats on each save (a file that cannot be written, a
/// damaged file that keeps coming back) is reported once, not as a new toast every time.
static SENT: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// How many damaged copies (`<name>.corrupt`, `<name>.corrupt.2` …) are kept per file. Past that the oldest copy
/// is dropped to make room (`keep_aside`): a newer build sharing the file, or a bug, must not fill the folder
/// with copies, and must not stop the file from being saved either.
const MAX_COPIES: u32 = 5;

/// Takes the messages about store files since the last call; the App shows them as toasts.
pub fn take_notices() -> Vec<String> {
    std::mem::take(&mut *NOTICES.lock().unwrap_or_else(|e| e.into_inner()))
}

fn notify(text: String) {
    let fresh = SENT.lock().unwrap_or_else(|e| e.into_inner()).insert(text.clone());
    if fresh {
        NOTICES.lock().unwrap_or_else(|e| e.into_inner()).push(text);
    }
}

/// The name of the `n`th damaged copy of `file`: `<name>.corrupt`, then `<name>.corrupt.2` …
fn copy_name(file: &Path, n: u32) -> PathBuf {
    let name = file.file_name().unwrap_or_default().to_string_lossy();
    let suffix = if n == 1 { ".corrupt".to_string() } else { format!(".corrupt.{n}") };
    file.with_file_name(format!("{name}{suffix}"))
}

/// Reads a store file that is about to be overwritten. `Ok(Some(value))`: valid content. `Ok(None)`: no file,
/// an empty one, or a damaged one that was kept aside (`keep_aside`), so the caller starts from its own copy.
/// `Err(())`: the file could not be read or kept aside; the caller must leave it alone and not write it.
/// Only content that does not parse is damage. Any other read error (permissions, a sharing violation, a
/// directory in the file's place) may hit a healthy file, so nothing is copied or overwritten.
fn read_stored<T: for<'de> Deserialize<'de>>(file: &Path) -> Result<Option<T>, ()> {
    let bytes = match std::fs::read(file) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            notify(format!("{} could not be read ({e}): this change was not saved", crate::util::tilde(file)));
            return Err(());
        }
    };
    // An empty file holds no state (a crash right after it was created): there is nothing to keep.
    if bytes.is_empty() {
        return Ok(None);
    }
    let parsed: Option<T> =
        std::str::from_utf8(&bytes).ok().and_then(|text| serde_json::from_str(crate::util::strip_bom(text)).ok());
    match parsed {
        Some(value) => Ok(Some(value)),
        // Also what a newer build sharing the file wrote: the bytes are kept whatever wrote them.
        None => keep_aside(file, &bytes).map(|()| None),
    }
}

/// Keeps the bytes of a damaged file in its first free copy (`copy_name`) and queues a notice. Bytes that an
/// existing copy already holds are not copied again, and then nothing is said. The copy is made from the bytes
/// that were read (not by path, which an unlocked writer may have changed meanwhile) and created exclusively,
/// so an older copy is never overwritten. `Err` when nothing could be kept: the caller must not write over it.
fn keep_aside(file: &Path, bytes: &[u8]) -> Result<(), ()> {
    use std::io::Write;
    let shown = crate::util::tilde(file);
    let kept = |n: u32| std::fs::read(copy_name(file, n)).is_ok_and(|old| old == bytes);
    if (1..=MAX_COPIES).any(kept) {
        return Ok(());
    }
    // All slots taken: the oldest copy (number 1) makes room and every later one moves down a number, so the
    // newest damage always gets the last slot and a file that keeps coming back cannot block saving for good.
    if (1..=MAX_COPIES).all(|n| copy_name(file, n).exists()) {
        let _ = std::fs::remove_file(copy_name(file, 1));
        for n in 2..=MAX_COPIES {
            let _ = std::fs::rename(copy_name(file, n), copy_name(file, n - 1));
        }
    }
    for n in 1..=MAX_COPIES {
        let copy = copy_name(file, n);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&copy) {
            Ok(mut out) => {
                if let Err(e) = out.write_all(bytes) {
                    // A partial copy would look like a good one next time: it goes.
                    drop(out);
                    let _ = std::fs::remove_file(&copy);
                    notify(format!(
                        "{shown} could not be read, and its copy could not be written ({e}): this change was not saved"
                    ));
                    return Err(());
                }
                notify(format!("{shown} could not be read; a copy was kept at {}", crate::util::tilde(&copy)));
                return Ok(());
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => {
                notify(format!(
                    "{shown} could not be read, and its copy could not be made ({e}): this change was not saved"
                ));
                return Err(());
            }
        }
    }
    notify(format!("{shown} could not be read, and no free copy name could be found: this change was not saved"));
    Err(())
}

/// Writes a store file after its read-modify-write. A write that fails (disk full, a locked file on Windows)
/// leaves the change unsaved, and that is reported (once per run, see `notify`).
fn save_stored<T: Serialize>(file: &Path, value: &T) {
    let result =
        serde_json::to_string_pretty(value).map_err(std::io::Error::from).and_then(|text| write_text(file, &text));
    if let Err(e) = result {
        notify(format!("{} could not be saved ({e}): this change was not saved", crate::util::tilde(file)));
    }
}

/// One locked read-modify-write of a store file (see `UiState::modify`): the file is read again under
/// `<file>.lock` (`fallback` when it is missing, empty or damaged), `f` changes the value, and the result is
/// written atomically unless the file could not be read or kept aside (`read_stored`). Returns the value and
/// `f`'s result.
fn update_file<V, R>(file: &Path, wait: Duration, fallback: impl FnOnce() -> V, f: impl FnOnce(&mut V) -> R) -> (V, R)
where
    V: Serialize + for<'de> Deserialize<'de>,
{
    with_lock_waiting(file, wait, || {
        let (mut value, writable) = match read_stored(file) {
            Ok(Some(value)) => (value, true),
            Ok(None) => (fallback(), true),
            Err(()) => (fallback(), false),
        };
        let out = f(&mut value);
        if writable {
            save_stored(file, &value);
        }
        (value, out)
    })
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
    /// Directories that are gone, or that did not answer a long probe (`LONG_PROBE_TIMEOUT`), in this run. They are
    /// not probed again (see `probe_dirs`).
    failed: HashSet<PathBuf>,
    /// Directories that did not answer a short probe in this run. The startup list leaves them out and does not
    /// probe them again; a longer probe (the restore's) still does.
    slow: HashSet<PathBuf>,
}

/// How long the startup check of the recent directories waits for a slow (network) path.
const RECENT_PROBE_TIMEOUT: Duration = Duration::from_millis(500);
/// How long a probe may wait for an answer before a directory that gave none is remembered for the whole run (the
/// restore's probe, `app::ops::DIR_PROBE_TIMEOUT`). A shorter wait only marks the directory as slow.
pub const LONG_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Whether `dir` does not exist at all (as opposed to being unreadable, or a file).
fn is_missing(dir: &Path) -> bool {
    matches!(std::fs::metadata(dir), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

/// One locked read-modify-write of a JSON list shared by every window (see `update_file`). A missing
/// file starts from `current`; a damaged one is kept aside first. Returns the new list.
fn modify_list<T, R>(file: &Path, current: &[T], f: impl FnOnce(&mut Vec<T>) -> R) -> (Vec<T>, R)
where
    T: Clone + Serialize + for<'de> Deserialize<'de>,
{
    update_file(file, UI_LOCK_WAIT, || current.to_vec(), f)
}

impl Recent {
    pub fn load(file: PathBuf) -> Recent {
        let entries = read_json(&file).unwrap_or_default();
        let mut r =
            Recent { file: Some(file), entries, cache: Vec::new(), failed: HashSet::new(), slow: HashSet::new() };
        r.rebuild(None);
        r
    }

    pub fn memory() -> Recent {
        Recent { file: None, entries: Vec::new(), cache: Vec::new(), failed: HashSet::new(), slow: HashSet::new() }
    }

    /// The directories among `dirs` that answer `probe` within `timeout` (`util::dir_answers`). What is remembered
    /// depends on the answer: a directory that is gone is not probed again in this run, and one that gave no answer
    /// within `LONG_PROBE_TIMEOUT` is not either (each such probe costs up to `timeout` on the UI thread, and a hung
    /// one keeps its thread until it returns). A directory that gave no answer to a shorter probe only is marked slow:
    /// a slow network share can answer the restore's longer probe, so that one still asks it. Used by the recent list
    /// and by workspace restore.
    pub fn probe_dirs(&mut self, dirs: &[PathBuf], timeout: Duration, probe: fn(&Path) -> bool) -> HashSet<PathBuf> {
        let fresh: Vec<PathBuf> = dirs.iter().filter(|d| !self.failed.contains(*d)).cloned().collect();
        let mut usable = HashSet::new();
        for (dir, answer) in crate::util::dir_answers(&fresh, timeout, probe) {
            match answer {
                Some(true) => {
                    usable.insert(dir);
                }
                // Not a directory, but it answered: only a missing one is final (an unreadable one may be a share
                // that is back next time).
                Some(false) if is_missing(&dir) => {
                    self.failed.insert(dir);
                }
                Some(false) => {}
                None if timeout >= LONG_PROBE_TIMEOUT => {
                    self.failed.insert(dir);
                }
                None => {
                    self.slow.insert(dir);
                }
            }
        }
        usable
    }

    /// Re-sorts the list of existing directories. A directory already in the cache is not looked at again, and
    /// neither is `trusted` (the one just opened), so a new tab never waits for a directory that is known. Any other
    /// path (startup, or another window's entry) is probed once in parallel with a time limit (`probe_dirs`): a dead
    /// or slow one is probed once per run, not on every new tab.
    fn rebuild(&mut self, trusted: Option<&str>) {
        let t = now();
        let unknown: Vec<PathBuf> = {
            let known: HashSet<&str> = self.cache.iter().map(|e| e.path.as_str()).chain(trusted).collect();
            self.entries
                .iter()
                .filter(|e| !known.contains(e.path.as_str()) && !self.slow.contains(Path::new(&e.path)))
                .map(|e| PathBuf::from(&e.path))
                .collect()
        };
        let usable = self.probe_dirs(&unknown, RECENT_PROBE_TIMEOUT, |p| p.is_dir());
        let known: HashSet<&str> = self.cache.iter().map(|e| e.path.as_str()).chain(trusted).collect();
        let mut list: Vec<RecentEntry> = self
            .entries
            .iter()
            .filter(|e| known.contains(e.path.as_str()) || usable.contains(Path::new(&e.path)))
            .cloned()
            .collect();
        list.sort_by(|a, b| b.score(t).total_cmp(&a.score(t)));
        self.cache = list;
    }

    fn key(path: &Path) -> String {
        path.display().to_string()
    }

    /// Applies one change to the entry list: with a file, as a locked read-modify-write so
    /// another window's entries are kept; without one (tests) only memory changes.
    fn modify<R>(&mut self, f: impl FnOnce(&mut Vec<RecentEntry>) -> R) -> R {
        match self.file.clone() {
            Some(file) => {
                let (list, out) = modify_list(&file, &self.entries, f);
                self.entries = list;
                out
            }
            None => f(&mut self.entries),
        }
    }

    pub fn record(&mut self, path: &Path) {
        let key = Self::key(path);
        let t = now();
        let trusted = self.modify(|entries| {
            let idx = match entries.iter().position(|e| crate::util::same_path(Path::new(&e.path), path)) {
                Some(i) => {
                    entries[i].count = entries[i].count.saturating_add(1);
                    entries[i].last = t;
                    i
                }
                None => {
                    entries.push(RecentEntry { path: key, count: 1, last: t });
                    entries.len() - 1
                }
            };
            let trusted = entries[idx].path.clone();
            // Drop the lowest scoring entries.
            if entries.len() > 60 {
                entries.sort_by(|a, b| b.score(t).total_cmp(&a.score(t)));
                entries.truncate(50);
            }
            trusted
        });
        self.rebuild(Some(&trusted));
    }

    pub fn remove(&mut self, path: &str) {
        self.modify(|entries| {
            entries.retain(|e| !crate::util::same_path(Path::new(&e.path), Path::new(path)));
        });
        self.rebuild(None);
    }

    /// Existing directories, by score.
    pub fn top(&self, n: usize) -> Vec<RecentEntry> {
        self.cache.iter().take(n).cloned().collect()
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

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Workspace {
    pub name: String,
    pub saved_at: i64,
    pub tabs: Vec<SavedTab>,
    /// Fields this build does not know (a newer build wrote them): kept as they are when the file is saved again.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
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
    /// Fields this build does not know (a newer build wrote them): kept as they are when the file is saved again.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
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
        let Ok(stored) = read_stored::<Session>(file) else { return None };
        let mut session = stored.unwrap_or_default();
        session.running.retain(|id| id != me && instance_alive(id));
        let first = session.running.is_empty();
        session.running.push(me.to_string());
        let restore = first.then(|| {
            session.tabs.iter_mut().for_each(|t| t.instance = me.to_string());
            session.tabs.iter().map(|t| t.tab.clone()).collect()
        });
        save_stored(file, &session);
        restore
    })
}

/// Merges a window's tabs into the session file: replaces what this window saved before and
/// keeps the other windows' tabs. `leaving`: the window is closing. Waits for the lock like
/// `with_lock_waiting`: the closing window saves even when the lock could not be had.
pub fn session_save(file: &Path, me: &str, tabs: Vec<SavedTab>, leaving: bool) {
    with_lock_waiting(file, LOCK_WAIT, || merge_session(file, me, tabs, leaving));
}

/// A save only under the session lock (`try_with_lock`), waiting at most `wait` for another window's. Used by the
/// periodic save and by the panic path, which must not write without the lock (another window may be saving; a
/// write then could drop its tabs). `false` when the lock was not taken: nothing was written, and the file keeps the
/// tabs saved last; the caller saves again later (it must not count the tabs as saved).
pub fn session_save_waiting(file: &Path, me: &str, tabs: Vec<SavedTab>, leaving: bool, wait: Duration) -> bool {
    try_with_lock(file, wait, || merge_session(file, me, tabs, leaving)).is_some()
}

/// The merge itself, run under the session lock (`session_save`, `session_save_waiting`).
fn merge_session(file: &Path, me: &str, tabs: Vec<SavedTab>, leaving: bool) {
    let Ok(stored) = read_stored::<Session>(file) else { return };
    let mut session = stored.unwrap_or_default();
    session.tabs.retain(|t| t.instance != me);
    session.tabs.extend(tabs.into_iter().map(|tab| SessionTab { instance: me.to_string(), tab }));
    if leaving {
        session.running.retain(|id| id != me);
    }
    session.saved_at = now();
    if session.tabs.is_empty() && session.running.is_empty() {
        let _ = std::fs::remove_file(file);
    } else {
        save_stored(file, &session);
    }
}

/// How long a window waits for another one's session lock before it saves without it (the closing window).
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

/// A held `<file>.lock`. Dropping it removes the lock again, but only while the file still holds this
/// window's owner line: a lock that was taken over as stale is not this window's to remove.
struct FileLock {
    path: PathBuf,
    owner: String,
}

impl Drop for FileLock {
    fn drop(&mut self) {
        if std::fs::read_to_string(&self.path).is_ok_and(|text| text == self.owner) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// The lock names its owner (`<pid>-<process start time>`, as in `instance_id`). A lock whose
/// owner has exited (crash, kill) is taken over at once; one held by a running process is
/// waited for, up to `wait`, and taken over only when it is older than `LOCK_STALE_LIVE`.
/// After `wait`, or when no lock can be created at all, `f` runs anyway: this is the fallback for
/// the startup restore and the closing window, where saving without the lock beats never saving (or
/// never closing the window). The cost is that another window saving at the same moment can lose
/// its tabs. Periodic saves use `try_with_lock` instead and skip the save.
fn with_lock_waiting<T>(file: &Path, wait: Duration, f: impl FnOnce() -> T) -> T {
    let _lock = acquire_lock(file, wait);
    f()
}

/// Runs `f` only while holding `<file>.lock` (waiting up to `wait`, see `acquire_lock`). `None` when the lock
/// could not be taken: `f` did not run, and the caller tries again later.
fn try_with_lock<T>(file: &Path, wait: Duration, f: impl FnOnce() -> T) -> Option<T> {
    let lock = acquire_lock(file, wait)?;
    let out = f();
    drop(lock);
    Some(out)
}

/// Creates `<file>.lock` with this window's owner line, waiting up to `wait` for a live owner to release it
/// and taking over a stale lock (`lock_is_stale`, `take_over_stale`). `None` when it could not be taken.
fn acquire_lock(file: &Path, wait: Duration) -> Option<FileLock> {
    use std::io::Write;
    let lock = file.with_extension("json.lock");
    if let Some(parent) = file.parent() {
        let _ = crate::util::create_private_dir(parent);
    }
    let started = Instant::now();
    let deadline = started + wait;
    // Asking the OS about the owner costs more than trying to create the file: at most 4 times a second.
    let mut next_check = Instant::now();
    loop {
        match crate::util::private_file_options().write(true).create_new(true).open(&lock) {
            Ok(mut handle) => {
                let owner = lock_owner();
                if handle.write_all(owner.as_bytes()).is_err() {
                    drop(handle);
                    let _ = std::fs::remove_file(&lock);
                    return None;
                }
                return Some(FileLock { path: lock, owner });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if Instant::now() >= next_check {
                    next_check = Instant::now() + Duration::from_millis(250);
                    if lock_is_stale(&lock) && take_over_stale(&lock) {
                        continue;
                    }
                }
                if Instant::now() > deadline {
                    return None;
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
            Err(_) => return None,
        }
    }
}

/// Takes over a lock that `lock_is_stale` judged stale. The lock is first renamed to a name of its own, so
/// only the file that was judged is removed: when its owner renewed it in the meantime (the check and the
/// rename are not atomic together), the lock goes back to its name and the caller keeps waiting. `true`
/// when the stale lock was removed (or was already gone).
fn take_over_stale(lock: &Path) -> bool {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let mut parked = lock.as_os_str().to_os_string();
    parked.push(format!(".stale-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    let parked = PathBuf::from(parked);
    if std::fs::rename(lock, &parked).is_err() {
        // Gone already, or moved by another window: the caller looks again.
        return !lock.exists();
    }
    if lock_is_stale(&parked) {
        let _ = std::fs::remove_file(&parked);
        return true;
    }
    // Not stale after all: back under its name, unless another window has taken the name since (then that
    // one's lock stays and this one is dropped). A hard link keeps it in place where the file system has them.
    if !lock.exists() {
        let _ = std::fs::hard_link(&parked, lock).or_else(|_| std::fs::rename(&parked, lock));
    }
    let _ = std::fs::remove_file(&parked);
    false
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

/// Most saved workspaces kept; saving one more drops the oldest.
pub const MAX_WORKSPACES: usize = 20;

/// What `Workspaces::upsert` did besides adding the entry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UpsertOutcome {
    /// A workspace with the same name existed and was replaced.
    pub replaced: bool,
    /// The oldest workspace, dropped because the list was full.
    pub dropped: Option<String>,
}

impl Workspaces {
    pub fn load(file: PathBuf) -> Workspaces {
        let list = read_json(&file).unwrap_or_default();
        Workspaces { file: Some(file), list }
    }

    pub fn memory() -> Workspaces {
        Workspaces { file: None, list: Vec::new() }
    }

    /// Applies one change to the list: with a file, as a locked read-modify-write so another
    /// window's workspaces are kept; without one (tests) only memory changes.
    fn modify<R>(&mut self, f: impl FnOnce(&mut Vec<Workspace>) -> R) -> R {
        match self.file.clone() {
            Some(file) => {
                let (list, out) = modify_list(&file, &self.list, f);
                self.list = list;
                out
            }
            None => f(&mut self.list),
        }
    }

    /// Replaces the entry with the same name, or inserts it at the top.
    pub fn upsert(&mut self, mut ws: Workspace) -> UpsertOutcome {
        ws.saved_at = now();
        self.modify(|list| {
            let before = list.len();
            list.retain(|w| !w.name.eq_ignore_ascii_case(&ws.name));
            let replaced = list.len() != before;
            list.insert(0, ws);
            let dropped = (list.len() > MAX_WORKSPACES).then(|| list.pop().map(|w| w.name)).flatten();
            list.truncate(MAX_WORKSPACES);
            UpsertOutcome { replaced, dropped }
        })
    }

    /// Deletes the workspace with this name; false when there was none.
    pub fn remove(&mut self, name: &str) -> bool {
        self.modify(|list| {
            let before = list.len();
            list.retain(|w| w.name != name);
            list.len() != before
        })
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
    /// The Claude status hooks were offered once (after the first Claude launch): never offered again.
    pub hooks_offered: bool,
    /// The NOBLE version that last played the startup animation ("" before the first launch).
    pub boot_version: String,
    /// The same for `noble-dev` (its own state, so alternating with `noble` does not replay the animation).
    pub boot_version_dev: String,
    /// Shell keys (as "alt+t") whose first use already told the user where NOBLE's action is.
    pub shell_keys_told: Vec<String>,
    /// Palette commands run lately, newest first (`App::palette_key`); listed first on an empty query.
    pub palette_recent: Vec<String>,
    /// Fields this build does not know (a newer build wrote them): kept as they are when the file is saved again,
    /// so an older build sharing `state.json` does not drop them.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
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
    /// and becomes the in-memory state, other windows' changes included. A missing file starts from the
    /// in-memory state; a damaged one is kept as `<name>.corrupt` first (`keep_aside`). Without a file
    /// (tests) only memory changes.
    pub fn modify<T>(&mut self, f: impl FnOnce(&mut UiStateData) -> T) -> T {
        let Some(file) = self.file.clone() else { return f(&mut self.data) };
        let current = &self.data;
        let (data, out) = update_file(&file, UI_LOCK_WAIT, || current.clone(), f);
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
            // A damaged file is kept aside first; when that fails it is not written over.
            if read_stored::<std::collections::BTreeMap<String, Vec<(i64, u8)>>>(f).is_ok() {
                save_stored(f, &self.series);
            }
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

    /// A symlinked file (dotfiles managers) stays a link and its target gets the text; a private file
    /// stays private. Unix only: creating symlinks on Windows needs developer mode or admin rights, and
    /// Windows has no mode bits to keep.
    #[cfg(unix)]
    #[test]
    fn write_text_keeps_symlinks_and_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("noble-write-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let real = dir.join("real.toml");
        let link = dir.join("config.toml");
        std::fs::write(&real, "old").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write_text(&link, "new").unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "the link is kept");
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new");
        assert_eq!(std::fs::metadata(&real).unwrap().permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A new store file is 0600 from the start (whatever the umask says), and a file that exists keeps its mode.
    /// Windows: a new file takes the ACL of its folder, so there are no mode bits to check.
    #[cfg(unix)]
    #[test]
    fn new_store_files_are_private_and_existing_modes_are_kept() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("noble-write-mode-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let fresh = dir.join("data").join("session.json");
        write_text(&fresh, "{}").unwrap();
        assert_eq!(std::fs::metadata(&fresh).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(fresh.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        let old = dir.join("old.json");
        std::fs::write(&old, "x").unwrap();
        std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_text(&old, "y").unwrap();
        assert_eq!(std::fs::metadata(&old).unwrap().permissions().mode() & 0o777, 0o644);
        let _ = std::fs::remove_dir_all(&dir);
    }

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

    /// The notices about files in `dir` (named by its unique name). Tests run in parallel and share the queue,
    /// so this takes only its own notices, under the queue's lock, and leaves the others queued.
    fn notices_for(dir: &Path) -> Vec<String> {
        let key = dir.file_name().unwrap().to_string_lossy().into_owned();
        let mut queue = NOTICES.lock().unwrap_or_else(|e| e.into_inner());
        let (mine, others): (Vec<String>, Vec<String>) =
            std::mem::take(&mut *queue).into_iter().partition(|n| n.contains(&key));
        *queue = others;
        mine
    }

    /// The `*.corrupt*` copies in `dir`, sorted.
    fn copies_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".corrupt"))
            .collect();
        names.sort();
        names
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A damaged file is copied to `<name>.corrupt` before a write replaces it: the copy has the original
    /// bytes, the new file holds the change, and the App is told. An older copy is never overwritten.
    #[test]
    fn damaged_files_are_kept_aside_before_they_are_overwritten() {
        let dir = scratch_dir("noble-store-aside");
        let read = |name: &str| std::fs::read(dir.join(name)).unwrap();
        let project = dir.join("alpha");

        let state = dir.join("state.json");
        let broken: &[u8] = b"{\"pins\": [\"/a\", ";
        std::fs::write(&state, broken).unwrap();
        let mut ui = UiState::load(state.clone());
        assert!(ui.toggle_pin(&project));
        assert_eq!(read("state.json.corrupt"), broken, "the original bytes are kept");
        assert!(UiState::load(state.clone()).is_pinned(&project), "the new file holds the change");
        // Damaged again with other bytes: the older copy stays, the newer one gets its own name.
        std::fs::write(&state, b"\xff\xfe not json").unwrap();
        ui.modify(|d| d.welcomed = true);
        assert_eq!(read("state.json.corrupt"), broken, "an older copy is never overwritten");
        assert_eq!(read("state.json.corrupt.2"), b"\xff\xfe not json");
        let back = UiState::load(state);
        assert!(back.data.welcomed && back.is_pinned(&project), "memory is written when the file is damaged");

        let workspaces = dir.join("workspaces.json");
        std::fs::write(&workspaces, b"[{\"name\": ").unwrap();
        Workspaces::load(workspaces.clone()).upsert(ws("fresh"));
        assert_eq!(read("workspaces.json.corrupt"), b"[{\"name\": ");
        let names: Vec<_> = Workspaces::load(workspaces).list.into_iter().map(|w| w.name).collect();
        assert_eq!(names, ["fresh"]);

        let recent = dir.join("recent.json");
        std::fs::write(&recent, b"[{\"path\"").unwrap();
        Recent::load(recent.clone()).record(&dir);
        assert_eq!(read("recent.json.corrupt"), b"[{\"path\"");
        assert_eq!(Recent::load(recent).entries.len(), 1);

        let session = dir.join("session.json");
        std::fs::write(&session, b"{\"tabs\": [").unwrap();
        let tab = SavedTab {
            name: None,
            origin: "kept".into(),
            layout: SavedNode::Leaf { cwd: ".".into(), launch: None },
            focus: 0,
        };
        session_save(&session, "1-1-0", vec![tab], false);
        assert_eq!(read("session.json.corrupt"), b"{\"tabs\": [");
        assert_eq!(load_session(&session).map(|s| s.tabs.len()), Some(1));

        let history = dir.join("ai-history.json");
        std::fs::write(&history, b"{\"claude/5h\": [[1").unwrap();
        let mut h = UsageHistory::load(history.clone());
        h.record("claude/5h", 100, 10);
        h.save();
        assert_eq!(read("ai-history.json.corrupt"), b"{\"claude/5h\": [[1");
        assert!(UsageHistory::load(history).series.contains_key("claude/5h"));

        let notices = notices_for(&dir);
        for name in ["state.json", "workspaces.json", "recent.json", "session.json", "ai-history.json"] {
            let shown = crate::util::tilde(&dir.join(name));
            assert!(
                notices.iter().any(|n| n.starts_with(&shown) && n.contains("could not be read; a copy was kept at")),
                "{name}: {notices:?}"
            );
        }
        assert_eq!(notices.len(), 6, "one notice per copy, the second state copy included: {notices:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Damage that comes back with the same bytes (a write that failed, or a newer build sharing the file)
    /// keeps one copy and says so once; identical bytes from an older damage are not copied again.
    #[test]
    fn repeated_damage_keeps_one_copy_and_one_notice() {
        let dir = scratch_dir("noble-store-repeat");
        let state = dir.join("state.json");
        let broken: &[u8] = b"{\"pins\": [\"/a\", ";
        let other: &[u8] = b"\xff\xfe not json";
        let mut ui = UiState::load(state.clone());
        for _ in 0..4 {
            std::fs::write(&state, broken).unwrap();
            ui.modify(|d| d.welcomed = true);
        }
        assert_eq!(copies_in(&dir), ["state.json.corrupt"]);
        assert_eq!(std::fs::read(dir.join("state.json.corrupt")).unwrap(), broken);
        assert_eq!(notices_for(&dir).len(), 1);

        std::fs::write(&state, other).unwrap();
        ui.modify(|d| d.welcomed = true);
        std::fs::write(&state, broken).unwrap();
        ui.modify(|d| d.welcomed = true);
        assert_eq!(copies_in(&dir), ["state.json.corrupt", "state.json.corrupt.2"]);
        assert_eq!(notices_for(&dir).len(), 1, "the older bytes came back: nothing new to say");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Past `MAX_COPIES` the oldest copy makes room: the newest damage takes the last slot, the older ones move
    /// down a number, and the change is saved (a file that keeps coming back cannot stop saving for good).
    #[test]
    fn copies_rotate_past_the_cap() {
        let dir = scratch_dir("noble-store-cap");
        let state = dir.join("state.json");
        let mut ui = UiState::load(state.clone());
        let damage = |i: u32| format!("{{\"pins\": [{i}").into_bytes();
        for i in 0..MAX_COPIES {
            std::fs::write(&state, damage(i)).unwrap();
            ui.modify(|d| d.welcomed = true);
        }
        assert_eq!(copies_in(&dir).len(), MAX_COPIES as usize);
        std::fs::write(&state, damage(MAX_COPIES)).unwrap();
        ui.modify(|d| d.hidden.push("/over-the-cap".into()));
        assert_eq!(copies_in(&dir).len(), MAX_COPIES as usize, "still at the cap: {:?}", copies_in(&dir));
        // Damage 0 (the oldest) is gone; damage 1 .. MAX_COPIES hold slots 1 .. MAX_COPIES, the newest last.
        for n in 1..=MAX_COPIES {
            assert_eq!(std::fs::read(copy_name(&state, n)).unwrap(), damage(n), "copy {n}");
        }
        let back = UiState::load(state);
        assert!(back.is_hidden(Path::new("/over-the-cap")), "the change was saved");
        assert!(back.data.welcomed);
        let notices = notices_for(&dir);
        assert!(notices.iter().all(|n| !n.contains("not saved") && !n.contains("could not be saved")), "{notices:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file that cannot be read (a directory in its place here) is never copied or written over: the change
    /// still applies in memory, and the App is told that it was not saved.
    #[test]
    fn unreadable_files_are_left_alone() {
        let dir = scratch_dir("noble-store-unreadable");
        let state = dir.join("state.json");
        std::fs::create_dir_all(state.join("inside")).unwrap();
        let project = dir.join("alpha");
        let mut ui = UiState::load(state.clone());
        assert!(ui.toggle_pin(&project), "the change still applies in memory");
        assert!(state.join("inside").is_dir(), "the directory is untouched");
        assert!(copies_in(&dir).is_empty(), "nothing is copied aside");
        let notices = notices_for(&dir);
        assert!(
            notices.iter().any(|n| n.starts_with(&crate::util::tilde(&state))
                && n.contains("could not be read")
                && n.ends_with("this change was not saved")),
            "{notices:?}"
        );
        assert!(!notices.iter().any(|n| n.contains("could not be saved")), "no write was attempted: {notices:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file that cannot be written (its folder is a file here) is reported, and the same failure on the next
    /// save is not reported again.
    #[test]
    fn a_failed_save_is_reported_once() {
        let dir = scratch_dir("noble-store-save");
        let blocker = dir.join("file-in-the-way");
        std::fs::write(&blocker, b"x").unwrap();
        let file = blocker.join("state.json");
        save_stored(&file, &[1, 2]);
        save_stored(&file, &[3]);
        let notices = notices_for(&dir);
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert!(notices[0].contains("could not be saved") && notices[0].ends_with("this change was not saved"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An empty file is an empty state: it is overwritten without a copy and without a notice.
    #[test]
    fn an_empty_file_is_an_empty_state() {
        let dir = scratch_dir("noble-store-empty");
        let state = dir.join("state.json");
        std::fs::write(&state, b"").unwrap();
        let project = dir.join("alpha");
        UiState::load(state.clone()).toggle_pin(&project);
        assert!(UiState::load(state).is_pinned(&project));
        let session = dir.join("session.json");
        std::fs::write(&session, b"").unwrap();
        assert_eq!(session_begin(&session, "1-1-0"), Some(Vec::new()));
        assert!(copies_in(&dir).is_empty(), "{:?}", copies_in(&dir));
        assert!(notices_for(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Stores that are valid (written by the store itself, then changed and read again) never produce a copy
    /// or a notice.
    #[test]
    fn a_valid_file_is_never_kept_aside() {
        let dir = scratch_dir("noble-store-valid");
        let project = dir.join("alpha");
        let mut ui = UiState::load(dir.join("state.json"));
        ui.toggle_pin(&project);
        ui.add_project(&project);
        ui.modify(|d| d.welcomed = true);
        let ui = UiState::load(dir.join("state.json"));
        assert!(ui.is_pinned(&project) && ui.is_added(&project) && ui.data.welcomed);
        let mut ws_list = Workspaces::load(dir.join("workspaces.json"));
        ws_list.upsert(ws("one"));
        ws_list.upsert(ws("two"));
        ws_list.remove("one");
        assert_eq!(Workspaces::load(dir.join("workspaces.json")).list.len(), 1);
        Recent::load(dir.join("recent.json")).record(&dir);
        let session = dir.join("session.json");
        let tab = SavedTab {
            name: None,
            origin: "kept".into(),
            layout: SavedNode::Leaf { cwd: ".".into(), launch: None },
            focus: 0,
        };
        session_begin(&session, "1-1-0");
        session_save(&session, "1-1-0", vec![tab], false);
        session_save(&session, "1-1-0", Vec::new(), true);
        let mut history = UsageHistory::load(dir.join("ai-history.json"));
        history.record("claude/5h", now(), 10);
        history.save();
        UsageHistory::load(dir.join("ai-history.json")).save();
        assert!(copies_in(&dir).is_empty(), "{:?}", copies_in(&dir));
        assert!(notices_for(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A dangling link is no place to write: the write fails, the link stays a link, and no file appears at its
    /// target. The config loader reports the link rather than silently using the defaults. Unix only: creating
    /// symlinks on Windows needs developer mode or admin rights.
    #[cfg(unix)]
    #[test]
    fn writes_through_a_dangling_link_are_refused() {
        let dir = scratch_dir("noble-store-dangling");
        let missing = dir.join("missing.json");
        let link = dir.join("settings.json");
        std::os::unix::fs::symlink(&missing, &link).unwrap();
        assert!(write_text(&link, "{}").is_err());
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "the link is kept");
        assert!(!missing.exists(), "nothing is created at the target");
        assert!(copies_in(&dir).is_empty());
        let config = crate::config::load(&link);
        assert!(config.error.is_some_and(|e| e.text.contains("missing file")), "the link is reported");
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Fields a newer build wrote into the shared files are kept when this build saves them again (the top
    /// level of `state.json` and `session.json`, and a workspace entry).
    #[test]
    fn unknown_fields_survive_a_save_by_an_older_build() {
        let dir = scratch_dir("noble-store-extra");
        let state = dir.join("state.json");
        std::fs::write(&state, r#"{"welcomed": false, "future_flag": {"on": [1, 2]}}"#).unwrap();
        UiState::load(state.clone()).modify(|d| d.welcomed = true);
        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&state).unwrap()).unwrap();
        assert_eq!(json["welcomed"], true);
        assert_eq!(json["future_flag"], serde_json::json!({"on": [1, 2]}));

        let workspaces = dir.join("workspaces.json");
        std::fs::write(&workspaces, r#"[{"name": "w", "saved_at": 0, "tabs": [], "colour": "teal"}]"#).unwrap();
        Workspaces::load(workspaces.clone()).upsert(ws("other"));
        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&workspaces).unwrap()).unwrap();
        assert_eq!(json[1]["name"], "w");
        assert_eq!(json[1]["colour"], "teal");

        let session = dir.join("session.json");
        std::fs::write(&session, r#"{"saved_at": 1, "tabs": [], "layout_version": 3}"#).unwrap();
        session_begin(&session, "1-1-0");
        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&session).unwrap()).unwrap();
        assert_eq!(json["layout_version"], 3);
        assert_eq!(json["running"], serde_json::json!(["1-1-0"]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A stale lock is taken over only as the file that was judged: a lock whose owner is alive is put back
    /// under its name, a dead owner's lock is removed, and releasing a lock never removes another window's.
    #[test]
    fn lock_takeover_and_release_touch_only_their_own_lock() {
        let (dir, file, lock) = lock_dir("takeover");
        std::fs::write(&lock, lock_owner()).unwrap();
        assert!(!take_over_stale(&lock), "a live owner's lock is not taken");
        assert_eq!(std::fs::read_to_string(&lock).unwrap(), lock_owner(), "and it is back under its name");
        let parked: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".stale-"))
            .collect();
        assert!(parked.is_empty(), "{parked:?}");

        std::fs::write(&lock, "4000000000-1").unwrap();
        assert!(take_over_stale(&lock));
        assert!(!lock.exists());

        let mine = acquire_lock(&file, Duration::from_secs(1)).expect("the lock is free");
        assert_eq!(std::fs::read_to_string(&lock).unwrap(), lock_owner());
        // Another window took the name over while this one held it: releasing leaves that lock alone.
        std::fs::write(&lock, "4000000000-1").unwrap();
        drop(mine);
        assert_eq!(std::fs::read_to_string(&lock).unwrap(), "4000000000-1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The periodic session save is skipped, not run without the lock, while another window holds it: nothing is
    /// written and `false` tells the caller to save again later.
    #[test]
    fn a_periodic_save_skips_while_the_lock_is_held() {
        let (dir, file, lock) = lock_dir("skip");
        std::fs::write(&lock, lock_owner()).unwrap();
        let tab = |origin: &str| SavedTab {
            name: None,
            origin: origin.into(),
            layout: SavedNode::Leaf { cwd: ".".into(), launch: None },
            focus: 0,
        };
        let started = Instant::now();
        assert!(!session_save_waiting(&file, "1-1-0", vec![tab("skipped")], false, Duration::from_millis(300)));
        assert!(started.elapsed() < Duration::from_secs(5), "waited {:?}", started.elapsed());
        assert!(!file.exists(), "nothing was written");
        assert!(lock.exists(), "the other window's lock stays");
        std::fs::remove_file(&lock).unwrap();
        assert!(session_save_waiting(&file, "1-1-0", vec![tab("saved")], false, Duration::from_millis(300)));
        let origins: Vec<_> = load_session(&file).unwrap().tabs.into_iter().map(|t| t.tab.origin).collect();
        assert_eq!(origins, ["saved"]);
        assert!(!lock.exists(), "the lock is released");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The periodic save's wait for a lock another live window holds is one short try (it runs on the UI thread):
    /// it returns at once, writes nothing and keeps the other window's lock.
    #[test]
    fn a_periodic_save_with_a_short_wait_returns_at_once() {
        let (dir, file, lock) = lock_dir("short");
        std::fs::write(&lock, lock_owner()).unwrap();
        let started = Instant::now();
        assert!(!session_save_waiting(&file, "1-1-0", Vec::new(), false, Duration::from_millis(10)));
        assert!(started.elapsed() < Duration::from_millis(500), "waited {:?}", started.elapsed());
        assert!(!file.exists() && lock.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The panic path's save waits only its short wait for a live window's lock. When the lock is still held it writes
    /// nothing: the file keeps the tabs saved last (no write without the lock, which could drop the other window's
    /// tabs), and the other window's lock stays. Once the lock is free the save goes through.
    #[test]
    fn a_save_with_a_held_lock_keeps_the_last_saved_file() {
        let (dir, file, lock) = lock_dir("held");
        let tab = |origin: &str| SavedTab {
            name: None,
            origin: origin.into(),
            layout: SavedNode::Leaf { cwd: ".".into(), launch: None },
            focus: 0,
        };
        session_save(&file, "1-1-0", vec![tab("saved")], false);
        std::fs::write(&lock, lock_owner()).unwrap();
        let started = Instant::now();
        assert!(!session_save_waiting(&file, "1-1-0", vec![tab("panic")], true, Duration::from_millis(300)));
        assert!(started.elapsed() < Duration::from_secs(5), "waited {:?}", started.elapsed());
        let origins = |file: &Path| -> Vec<String> {
            load_session(file).unwrap().tabs.into_iter().map(|t| t.tab.origin).collect()
        };
        assert_eq!(origins(&file), ["saved"], "the last saved tabs stay");
        assert!(lock.exists(), "the other window's lock stays");
        std::fs::remove_file(&lock).unwrap();
        assert!(session_save_waiting(&file, "1-1-0", vec![tab("panic")], true, Duration::from_millis(300)));
        assert_eq!(origins(&file), ["panic"]);
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

    /// A crash leaves the session file listing the dead window as running, and its lock behind (the
    /// owner is gone). The next window is the first of the run: it gets the tabs at once, takes the
    /// lock over and registers itself, leaving no lock behind.
    #[test]
    fn a_crashed_run_restores_past_its_stale_lock() {
        let (dir, file, lock) = lock_dir("crash");
        let dead = "4000000000-1-0";
        let json = serde_json::json!({
            "saved_at": 0,
            "running": [dead],
            "tabs": [{ "instance": dead, "name": null, "origin": "crashed",
                       "layout": { "type": "leaf", "cwd": "." }, "focus": 0 }],
        });
        std::fs::write(&file, json.to_string()).unwrap();
        std::fs::write(&lock, "4000000000-1").unwrap();
        let started = Instant::now();
        let tabs = session_begin(&file, "5-6-0").expect("the first window of the run restores");
        assert!(started.elapsed() < Duration::from_secs(5), "waited {:?} for a dead owner", started.elapsed());
        assert_eq!(tabs.iter().map(|t| t.origin.as_str()).collect::<Vec<_>>(), ["crashed"]);
        assert!(!lock.exists(), "the lock is released after use");
        let session = load_session(&file).unwrap();
        assert_eq!(session.running, ["5-6-0"]);
        assert!(session.tabs.iter().all(|t| t.instance == "5-6-0"), "the tabs are taken over");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A lock that nobody can release must not block startup or a save for good: a lock with no readable
    /// owner is taken over once it is old, and a lock naming a running owner once it is older than
    /// `LOCK_STALE_LIVE` (the owner hangs). Each case returns at once, not after the waiting limit.
    #[test]
    fn a_stale_session_lock_never_blocks_restore_or_save() {
        let (dir, file, lock) = lock_dir("stale");
        let tab = |origin: &str| SavedTab {
            name: None,
            origin: origin.into(),
            layout: SavedNode::Leaf { cwd: ".".into(), launch: None },
            focus: 0,
        };
        let old = std::time::SystemTime::now() - Duration::from_secs(3600);
        let age = |lock: &Path| std::fs::File::options().write(true).open(lock).unwrap().set_modified(old).unwrap();
        let quick = |what: &str, started: Instant| {
            assert!(started.elapsed() < Duration::from_secs(5), "{what}: waited {:?}", started.elapsed());
        };

        session_save(&file, "1-1-0", vec![tab("first")], false);
        std::fs::write(&lock, "not an owner").unwrap();
        age(&lock);
        let started = Instant::now();
        let tabs = session_begin(&file, "2-2-0").expect("restores");
        quick("an old lock with no owner", started);
        assert_eq!(tabs.iter().map(|t| t.origin.as_str()).collect::<Vec<_>>(), ["first"]);
        assert!(!lock.exists());

        std::fs::write(&lock, lock_owner()).unwrap();
        age(&lock);
        let started = Instant::now();
        session_save(&file, "2-2-0", vec![tab("second")], false);
        quick("an old lock of a running owner", started);
        assert!(!lock.exists());
        let session = load_session(&file).unwrap();
        assert!(session.tabs.iter().any(|t| t.tab.origin == "second"), "the save went through");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A session file cut short anywhere (a torn write, a disk error), or damaged another way, must not
    /// stop the window from starting: nothing is restored, the damaged bytes are kept as
    /// `session.json.corrupt` (an empty file is not damage: it keeps no copy), and the window registers itself
    /// in a fresh file. Checked at every cut.
    #[test]
    fn a_truncated_session_file_is_kept_aside_on_startup() {
        let dir = std::env::temp_dir().join(format!("noble-store-truncated-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let full = serde_json::json!({
            "saved_at": 1,
            "running": ["4000000000-1-0"],
            "tabs": [{ "instance": "4000000000-1-0", "name": "ñ日本", "origin": "a", "focus": 1,
                       "layout": { "type": "split", "dir": "Row", "ratio": 0.5,
                                   "a": { "type": "leaf", "cwd": "." }, "b": { "type": "leaf", "cwd": "." } } }],
        })
        .to_string();
        let mut bodies: Vec<Vec<u8>> = (0..full.len()).map(|cut| full.as_bytes()[..cut].to_vec()).collect();
        bodies.extend([b"\xff\xfe\x00 garbage".to_vec(), b"{\"tabs\": 3}".to_vec(), b"null".to_vec()]);
        for (i, body) in bodies.iter().enumerate() {
            let case = dir.join(format!("case-{i}"));
            std::fs::create_dir_all(&case).unwrap();
            let file = case.join("session.json");
            std::fs::write(&file, body).unwrap();
            let restored = session_begin(&file, "1-1-0");
            assert_eq!(restored, Some(Vec::new()), "case {i}: {:?}", String::from_utf8_lossy(body));
            if body.is_empty() {
                assert!(copies_in(&case).is_empty(), "case {i}: an empty file is not kept");
            } else {
                assert_eq!(std::fs::read(case.join("session.json.corrupt")).unwrap(), *body, "case {i}");
            }
            let session = load_session(&file).expect("a fresh session file is written");
            assert_eq!(session.running, ["1-1-0"], "case {i}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn ws(name: &str) -> Workspace {
        Workspace { name: name.into(), saved_at: 0, tabs: Vec::new(), ..Default::default() }
    }

    /// Two windows saving workspaces / recent directories into one file keep each other's entries
    /// (a locked read-modify-write), and the list limit and replace/delete are reported.
    #[test]
    fn workspaces_and_recent_merge_between_windows() {
        let dir = std::env::temp_dir().join(format!("noble-store-multi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let file = dir.join("workspaces.json");
        let mut a = Workspaces::load(file.clone());
        let mut b = Workspaces::load(file.clone());
        assert_eq!(a.upsert(ws("from-a")), UpsertOutcome::default());
        assert_eq!(b.upsert(ws("from-b")), UpsertOutcome::default());
        let names: Vec<_> = Workspaces::load(file.clone()).list.into_iter().map(|w| w.name).collect();
        assert_eq!(names, ["from-b", "from-a"], "window b did not overwrite window a's workspace");
        assert_eq!(b.list.len(), 2, "b sees a's workspace right away");

        assert!(a.upsert(ws("FROM-B")).replaced);
        assert!(a.remove("from-a"));
        assert!(!a.remove("from-a"), "already gone");
        assert_eq!(Workspaces::load(file.clone()).list.len(), 1);

        for i in 0..MAX_WORKSPACES {
            let out = a.upsert(ws(&format!("w{i}")));
            // FROM-B is the oldest of the 20th save on.
            assert_eq!(out.dropped.as_deref(), (i == MAX_WORKSPACES - 1).then_some("FROM-B"), "{i}");
        }
        assert_eq!(a.list.len(), MAX_WORKSPACES);

        let recent_file = dir.join("recent.json");
        let (one, two) = (dir.join("one"), dir.join("two"));
        std::fs::create_dir_all(&one).unwrap();
        std::fs::create_dir_all(&two).unwrap();
        let mut ra = Recent::load(recent_file.clone());
        let mut rb = Recent::load(recent_file.clone());
        ra.record(&one);
        rb.record(&two);
        assert_eq!(Recent::load(recent_file).entries.len(), 2, "both windows' directories are kept");
        assert_eq!(rb.top(5).len(), 2, "b lists a's directory, checked once");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A directory that gave no answer to a long probe is not probed again in this run: each probe costs up to the
    /// timeout on the UI thread, and a hung one keeps its thread. The probe counts its calls to check that.
    #[test]
    fn unanswered_directories_are_not_probed_again() {
        static HANG_PROBES: AtomicU32 = AtomicU32::new(0);
        fn probe(p: &Path) -> bool {
            if p.ends_with("hangs") {
                HANG_PROBES.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_secs(3));
            }
            p.is_dir()
        }
        let here = std::env::temp_dir();
        let dirs = vec![PathBuf::from("hangs"), here.clone()];
        let mut r = Recent::memory();
        let usable = r.probe_dirs(&dirs, LONG_PROBE_TIMEOUT, probe);
        assert_eq!(usable, HashSet::from([here.clone()]), "the answering directory is usable");
        let again = r.probe_dirs(&dirs, LONG_PROBE_TIMEOUT, probe);
        assert_eq!(again, HashSet::from([here]), "the answering one is still usable");
        assert_eq!(HANG_PROBES.load(Ordering::SeqCst), 1, "the hung directory was probed again");
    }

    /// A slow directory (no answer to the startup's short probe) is not lost: the restore's longer probe asks it
    /// again, and it counts as usable once it answers. The startup list leaves it out meanwhile.
    #[test]
    fn a_short_probe_timeout_does_not_block_a_later_long_probe() {
        static SLOW_PROBES: AtomicU32 = AtomicU32::new(0);
        fn probe(p: &Path) -> bool {
            if p.ends_with("share") && SLOW_PROBES.fetch_add(1, Ordering::SeqCst) == 0 {
                // Answers only after the short probe gave up on it.
                std::thread::sleep(Duration::from_millis(400));
            }
            p.ends_with("share") || p.is_dir()
        }
        let dirs = vec![PathBuf::from("share")];
        let mut r = Recent::memory();
        assert!(r.probe_dirs(&dirs, Duration::from_millis(100), probe).is_empty(), "no answer in time");
        assert!(r.slow.contains(Path::new("share")) && !r.failed.contains(Path::new("share")));
        assert_eq!(r.probe_dirs(&dirs, LONG_PROBE_TIMEOUT, probe), HashSet::from([PathBuf::from("share")]));
        assert_eq!(SLOW_PROBES.load(Ordering::SeqCst), 2, "the long probe asked again");
    }

    /// A directory that is gone is remembered for the run, while one that answers "no" but is still there (a file,
    /// an unreadable folder) is asked again: it may come back.
    #[test]
    fn missing_directories_are_remembered_and_others_are_asked_again() {
        static CALLS: AtomicU32 = AtomicU32::new(0);
        fn probe(_: &Path) -> bool {
            CALLS.fetch_add(1, Ordering::SeqCst);
            false
        }
        let dir = std::env::temp_dir().join(format!("noble-store-probe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let gone = dir.join("gone");
        let file = dir.join("a-file");
        std::fs::write(&file, b"x").unwrap();
        let dirs = vec![gone.clone(), file.clone()];
        let mut r = Recent::memory();
        assert!(r.probe_dirs(&dirs, LONG_PROBE_TIMEOUT, probe).is_empty());
        assert_eq!(CALLS.load(Ordering::SeqCst), 2);
        assert!(r.probe_dirs(&dirs, LONG_PROBE_TIMEOUT, probe).is_empty());
        assert_eq!(CALLS.load(Ordering::SeqCst), 3, "only the file was asked again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A directory that disappears is dropped at the next start, not with every new tab.
    #[test]
    fn recent_checks_directories_at_load_not_at_record() {
        let dir = std::env::temp_dir().join(format!("noble-store-recent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (gone, kept) = (dir.join("gone"), dir.join("kept"));
        std::fs::create_dir_all(&gone).unwrap();
        std::fs::create_dir_all(&kept).unwrap();
        let file = dir.join("recent.json");
        let mut r = Recent::load(file.clone());
        r.record(&gone);
        r.record(&kept);
        std::fs::remove_dir_all(&gone).unwrap();
        r.record(&kept);
        assert_eq!(r.top(5).len(), 2, "no stat per new tab: still listed");
        assert_eq!(Recent::load(file).top(5).len(), 1, "a restart drops it");
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
                    a: Box::new(SavedNode::Leaf { cwd: "C:\\a".into(), launch: Some("claude".into()) }),
                    b: Box::new(SavedNode::Leaf { cwd: "C:\\b".into(), launch: None }),
                },
                focus: 1,
            }],
            ..Default::default()
        };
        let text = serde_json::to_string(&ws).unwrap();
        let back: Workspace = serde_json::from_str(&text).unwrap();
        assert_eq!(back, ws);
        assert_eq!(back.pane_count(), 2);
    }
}
