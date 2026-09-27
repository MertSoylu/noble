//! Project discovery: finds git repos under the root folders, reads the branch
//! and last activity from the file system and collects change counts with `git` in the background.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::config::ProjectsCfg;
use crate::event::{AppEvent, Tx};
use crate::util;

#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub name: String,
    pub path: PathBuf,
    pub branch: Option<String>,
    pub last_active: Option<SystemTime>,
    pub git: Option<GitInfo>,
    /// A git repository. Folders added by hand may be plain folders: they are listed without git status.
    pub repo: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GitInfo {
    /// Total uncommitted changes (new files included).
    pub dirty: u32,
    /// Of those, the new files git does not track yet.
    pub untracked: u32,
    pub ahead: u32,
    pub behind: u32,
    pub branch: Option<String>,
    pub last_subject: Option<String>,
    pub last_commit: Option<i64>,
    /// Recent commits, newest first (for the project card on Home).
    pub commits: Vec<Commit>,
    /// Changed files: two-letter porcelain status and path (at most `MAX_CHANGES`).
    pub changes: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Commit {
    pub hash: String,
    pub time: i64,
    pub author: String,
    pub subject: String,
}

/// How many commits and files are kept for the project card.
pub const MAX_COMMITS: usize = 8;
pub const MAX_CHANGES: usize = 40;

const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "vendor",
    "venv",
    ".venv",
    "__pycache__",
    "AppData",
    "Library",
    "Applications",
    "$Recycle.Bin",
    "My Music",
    "My Pictures",
    "My Videos",
];

/// Default root folders: the ones that exist.
pub fn default_roots() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else { return Vec::new() };
    let mut roots: Vec<PathBuf> = [
        "Desktop",
        "Documents",
        "Documents/GitHub",
        "source/repos",
        "projects",
        "Projects",
        "code",
        "Code",
        "dev",
        "src",
        "repos",
        "git",
        "work",
    ]
    .iter()
    .map(|p| home.join(p))
    .filter(|p| p.is_dir())
    .collect();
    // On Windows and macOS `projects` and `Projects` are the same folder: it must not be
    // added twice. On Linux they are two folders.
    if cfg!(any(windows, target_os = "macos")) {
        roots.dedup_by(|a, b| a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy()));
    }
    roots
}

pub fn roots_from(cfg: &ProjectsCfg) -> Vec<PathBuf> {
    if cfg.roots.is_empty() {
        return default_roots();
    }
    cfg.roots
        .iter()
        .map(|r| {
            if let Some(rest) = r.strip_prefix('~') {
                dirs::home_dir()
                    .map(|h| h.join(rest.trim_start_matches(['/', '\\'])))
                    .unwrap_or_else(|| PathBuf::from(r))
            } else {
                PathBuf::from(r)
            }
        })
        .filter(|p| p.is_dir())
        .collect()
}

/// `.git` may be a directory or a file containing `gitdir:` (for worktrees/submodules).
pub fn git_dir(repo: &Path) -> Option<PathBuf> {
    let dot = repo.join(".git");
    if dot.is_dir() {
        return Some(dot);
    }
    if dot.is_file() {
        let text = std::fs::read_to_string(&dot).ok()?;
        let target = text.trim().strip_prefix("gitdir:")?.trim();
        let p = PathBuf::from(target);
        return Some(if p.is_absolute() { p } else { repo.join(p) });
    }
    None
}

/// Branch name from the `HEAD` file (short hash on a detached HEAD).
pub fn read_branch(git_dir: &Path) -> Option<String> {
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    if let Some(r) = head.strip_prefix("ref:") {
        let r = r.trim();
        Some(r.strip_prefix("refs/heads/").unwrap_or(r).to_string())
    } else {
        Some(head.chars().take(7).collect())
    }
}

fn last_active(git_dir: &Path) -> Option<SystemTime> {
    ["index", "HEAD", "logs/HEAD", "FETCH_HEAD"]
        .iter()
        .filter_map(|f| std::fs::metadata(git_dir.join(f)).and_then(|m| m.modified()).ok())
        .max()
}

/// Scans the root folders (depth limited, at most `limit` projects).
pub fn scan(roots: &[PathBuf], max_depth: usize, exclude: &[String], limit: usize) -> Vec<Project> {
    let mut out: Vec<Project> = Vec::new();
    let mut visited = 0usize;
    let mut stack: Vec<(PathBuf, usize)> = roots.iter().map(|r| (r.clone(), 0)).collect();
    let mut seen: std::collections::HashSet<String> = Default::default();
    while let Some((dir, depth)) = stack.pop() {
        visited += 1;
        if visited > 20_000 || out.len() >= limit {
            break;
        }
        let key = dir.to_string_lossy().to_lowercase();
        if !seen.insert(key) {
            continue;
        }
        if let Some(gd) = git_dir(&dir) {
            out.push(Project {
                name: dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| dir.display().to_string()),
                branch: read_branch(&gd),
                last_active: last_active(&gd),
                path: dir,
                git: None,
                repo: true,
            });
            continue;
        }
        if depth >= max_depth {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if !ft.is_dir() || ft.is_symlink() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || SKIP_DIRS.contains(&name.as_str()) || exclude.iter().any(|e| e == &name) {
                continue;
            }
            stack.push((entry.path(), depth + 1));
        }
    }
    sort_projects(&mut out);
    out
}

/// A folder added by hand: a git repo reads its branch and activity like a scanned one, any
/// other folder is listed without git (its modification time as the activity). `None` when
/// the folder is gone.
pub fn project_at(dir: &Path) -> Option<Project> {
    let meta = std::fs::metadata(dir).ok().filter(|m| m.is_dir())?;
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dir.display().to_string());
    Some(match git_dir(dir) {
        Some(gd) => Project {
            name,
            branch: read_branch(&gd),
            last_active: last_active(&gd),
            path: dir.to_path_buf(),
            git: None,
            repo: true,
        },
        None => Project {
            name,
            branch: None,
            last_active: meta.modified().ok(),
            path: dir.to_path_buf(),
            git: None,
            repo: false,
        },
    })
}

/// Hidden and manually added projects (`state.json`), shared with the scan thread.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Manual {
    pub hidden: Vec<String>,
    pub added: Vec<PathBuf>,
}

impl Manual {
    pub fn is_hidden(&self, path: &Path) -> bool {
        self.hidden.iter().any(|h| util::same_path(Path::new(h), path))
    }

    /// Drops the hidden projects from a list (cheap: paths only, no file system access).
    pub fn filter(&self, list: &mut Vec<Project>) {
        if !self.hidden.is_empty() {
            list.retain(|p| !self.is_hidden(&p.path));
        }
    }

    /// The scan result as listed: hidden projects dropped, manually added folders merged in
    /// once (a folder the scan found too is not listed twice; a folder that is gone is skipped).
    pub fn apply(&self, mut list: Vec<Project>) -> Vec<Project> {
        self.filter(&mut list);
        for dir in &self.added {
            if self.is_hidden(dir) || list.iter().any(|p| util::same_path(&p.path, dir)) {
                continue;
            }
            if let Some(p) = project_at(dir) {
                list.push(p);
            }
        }
        sort_projects(&mut list);
        list
    }
}

pub fn sort_projects(list: &mut [Project]) {
    list.sort_by(|a, b| {
        b.last_active.cmp(&a.last_active).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

/// Parses `git status --porcelain=v1 -b` output.
pub fn parse_status(output: &str) -> GitInfo {
    let mut info = GitInfo::default();
    for line in output.lines() {
        if let Some(header) = line.strip_prefix("## ") {
            let (branch_part, tracking) = match header.find(" [") {
                Some(i) => (&header[..i], header[i + 2..].trim_end_matches(']')),
                None => (header, ""),
            };
            let branch = branch_part.split("...").next().unwrap_or(branch_part);
            let branch = branch.strip_prefix("No commits yet on ").unwrap_or(branch);
            if !branch.starts_with("HEAD (no branch)") {
                info.branch = Some(branch.to_string());
            }
            for part in tracking.split(", ") {
                if let Some(n) = part.strip_prefix("ahead ") {
                    info.ahead = n.trim_end_matches(']').parse().unwrap_or(0);
                } else if let Some(n) = part.strip_prefix("behind ") {
                    info.behind = n.trim_end_matches(']').parse().unwrap_or(0);
                }
            }
        } else if !line.trim().is_empty() {
            info.dirty += 1;
            if line.starts_with("??") {
                info.untracked += 1;
            }
            // "XY path", or "XY old -> new" for a rename. `get` so that an unexpected line
            // (not starting with two ASCII status letters) is skipped instead of panicking.
            if info.changes.len() < MAX_CHANGES
                && let (Some(xy), Some(rest)) = (line.get(..2), line.get(3..))
                && !rest.is_empty()
            {
                let path = rest.rsplit(" -> ").next().unwrap_or(rest).trim_matches('"');
                info.changes.push((xy.to_string(), path.to_string()));
            }
        }
    }
    info
}

fn git_info(path: &Path, git: &Path) -> Option<GitInfo> {
    let status = util::command_for(git)
        .arg("-C")
        .arg(path)
        .args(["status", "--porcelain=v1", "-b", "--untracked-files=normal"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !status.status.success() {
        return None;
    }
    let mut info = parse_status(&String::from_utf8_lossy(&status.stdout));
    if let Ok(log) = util::command_for(git)
        .arg("-C")
        .arg(path)
        .args(["log", &format!("-{MAX_COMMITS}"), "--format=%h%x1f%ct%x1f%an%x1f%s"])
        .stderr(std::process::Stdio::null())
        .output()
    {
        info.commits = parse_log(&String::from_utf8_lossy(&log.stdout));
        if let Some(c) = info.commits.first() {
            info.last_commit = Some(c.time);
            info.last_subject = Some(c.subject.clone());
        }
    }
    Some(info)
}

/// Parses `git log --format=%h%x1f%ct%x1f%an%x1f%s` output.
pub fn parse_log(output: &str) -> Vec<Commit> {
    output
        .lines()
        .filter_map(|line| {
            let mut f = line.splitn(4, '\u{1f}');
            Some(Commit {
                hash: f.next()?.to_string(),
                time: f.next()?.parse().ok()?,
                author: f.next()?.to_string(),
                subject: f.next().unwrap_or("").to_string(),
            })
        })
        .collect()
}

/// Requests to the project thread.
pub enum ProjectReq {
    /// Rescan the root folders from scratch.
    Rescan,
    /// Refresh one repo's git status right away (command finished, list scrolled…).
    Refresh(PathBuf),
}

/// Automatic rescan interval. Git status is also requested when a command
/// finishes and when Home opens; this scan only catches new/deleted repos.
const RESCAN_EVERY: Duration = Duration::from_secs(300);
/// How many projects (the most recently used) get their git status right after
/// a scan; the rest are requested with `Refresh` as they appear in the list.
pub const EAGER_GIT: usize = 40;

/// The deepest project containing `cwd` (e.g. `repo/src/x` → `repo`).
pub fn project_containing<'a>(list: &'a [Project], cwd: &Path) -> Option<&'a Project> {
    let norm = |p: &Path| {
        let s = p.to_string_lossy().replace('\\', "/");
        let s = s.trim_end_matches('/').to_string();
        // Windows and macOS (APFS) ignore case: `cd ~/projects/noble` is the same folder as
        // `~/Projects/Noble`. Linux file systems are case-sensitive.
        if cfg!(any(windows, target_os = "macos")) { s.to_lowercase() } else { s }
    };
    let c = norm(cwd);
    list.iter()
        .filter(|p| {
            let root = norm(&p.path);
            !root.is_empty() && (c == root || c.starts_with(&format!("{root}/")))
        })
        .max_by_key(|p| p.path.as_os_str().len())
}

/// Scan + git status thread. Rescans when `Rescan` arrives or the interval is
/// up; `Refresh` updates a single repo's status in between.
pub fn spawn(
    cfg: std::sync::Arc<std::sync::Mutex<ProjectsCfg>>,
    manual: std::sync::Arc<std::sync::Mutex<Manual>>,
    tx: Tx,
    req: std::sync::mpsc::Receiver<ProjectReq>,
) {
    use std::sync::mpsc::RecvTimeoutError;
    let _ = std::thread::Builder::new().name("projects".into()).spawn(move || {
        let git = util::which("git");
        // Last activity times from the previous scan: no git run for unchanged repos.
        let mut seen: std::collections::HashMap<PathBuf, Option<SystemTime>> = Default::default();
        loop {
            let cfg = cfg.lock().map(|c| c.clone()).unwrap_or_default();
            let roots = roots_from(&cfg);
            let list = scan(&roots, cfg.max_depth.clamp(1, 6), &cfg.exclude, 300);
            let list = manual.lock().map(|m| m.clone()).unwrap_or_default().apply(list);
            let targets: Vec<PathBuf> = list
                .iter()
                .take(EAGER_GIT)
                .filter(|p| p.repo && seen.get(&p.path) != Some(&p.last_active))
                .map(|p| p.path.clone())
                .collect();
            seen = list.iter().map(|p| (p.path.clone(), p.last_active)).collect();
            if tx.send(AppEvent::Projects(list)).is_err() {
                return;
            }
            if let Some(git) = &git {
                // A few parallel workers; the most recently used projects first.
                let chunks: Vec<Vec<PathBuf>> = targets.chunks(4).map(|c| c.to_vec()).collect();
                let handles: Vec<_> = chunks
                    .into_iter()
                    .map(|chunk| {
                        let tx = tx.clone();
                        let git = git.clone();
                        std::thread::spawn(move || {
                            for path in chunk {
                                if let Some(info) = git_info(&path, &git)
                                    && tx.send(AppEvent::Git(path, info)).is_err()
                                {
                                    return;
                                }
                            }
                        })
                    })
                    .collect();
                for h in handles {
                    let _ = h.join();
                }
            }
            let deadline = std::time::Instant::now() + RESCAN_EVERY;
            loop {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                match req.recv_timeout(left) {
                    Ok(ProjectReq::Refresh(path)) => {
                        // A plain folder (added by hand) has no status; inside another repo
                        // `git status` would report that repo's instead.
                        if let Some(git) = &git
                            && git_dir(&path).is_some()
                            && let Some(info) = git_info(&path, git)
                            && tx.send(AppEvent::Git(path, info)).is_err()
                        {
                            return;
                        }
                    }
                    Ok(ProjectReq::Rescan) | Err(RecvTimeoutError::Timeout) => {
                        // Swallow extra rescan requests arriving at the same time.
                        while let Ok(ProjectReq::Rescan) = req.try_recv() {}
                        break;
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_parsing() {
        let out = "## main...origin/main [ahead 2, behind 1]\n M src/a.rs\n?? new.txt\n";
        let info = parse_status(out);
        assert_eq!(info.branch.as_deref(), Some("main"));
        assert_eq!(info.ahead, 2);
        assert_eq!(info.behind, 1);
        assert_eq!(info.dirty, 2);
        assert_eq!(info.untracked, 1);
        let clean = parse_status("## dev\n");
        assert_eq!(clean.branch.as_deref(), Some("dev"));
        assert_eq!(clean.dirty, 0);
        let fresh = parse_status("## No commits yet on main\n");
        assert_eq!(fresh.branch.as_deref(), Some("main"));
        let gone = parse_status("## feat...origin/feat [gone]\n");
        assert_eq!(gone.ahead, 0);
        assert_eq!(info.changes, vec![(" M".into(), "src/a.rs".into()), ("??".into(), "new.txt".into())]);
        let renamed = parse_status("## main\nR  old.rs -> new.rs\n");
        assert_eq!(renamed.changes, vec![("R ".into(), "new.rs".into())]);
        // Unexpected lines (non-ASCII at the byte offsets) are counted but never panic.
        let odd = parse_status(
            "## ağaç [ahead 1]
日本 x
é
",
        );
        assert_eq!(odd.branch.as_deref(), Some("ağaç"));
        assert_eq!(odd.ahead, 1);
        assert_eq!(odd.dirty, 2);
        assert!(odd.changes.is_empty());
    }

    #[test]
    fn log_parsing() {
        let out = "a1b2c3d\u{1f}1700000000\u{1f}Mert\u{1f}feat: x \u{1f} y\nbad line\n";
        let log = parse_log(out);
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].hash, "a1b2c3d");
        assert_eq!(log[0].time, 1_700_000_000);
        assert_eq!(log[0].author, "Mert");
        assert_eq!(log[0].subject, "feat: x \u{1f} y");
    }

    #[test]
    fn scan_finds_repos() {
        let base = std::env::temp_dir().join(format!("noble-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("alpha/.git")).unwrap();
        std::fs::write(base.join("alpha/.git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::create_dir_all(base.join("group/beta/.git")).unwrap();
        std::fs::write(base.join("group/beta/.git/HEAD"), "0123456789abcdef\n").unwrap();
        std::fs::create_dir_all(base.join("node_modules/gamma/.git")).unwrap();
        std::fs::create_dir_all(base.join("deep/a/b/c/delta/.git")).unwrap();
        let found = scan(std::slice::from_ref(&base), 3, &[], 100);
        let names: Vec<&str> = found.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"beta"));
        assert!(!names.contains(&"gamma"));
        assert!(!names.contains(&"delta"));
        let alpha = found.iter().find(|p| p.name == "alpha").unwrap();
        assert_eq!(alpha.branch.as_deref(), Some("main"));
        let beta = found.iter().find(|p| p.name == "beta").unwrap();
        assert_eq!(beta.branch.as_deref(), Some("0123456"));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Hidden projects leave the scan result, added folders join it once (a git repo or a
    /// plain folder), and an added folder that no longer exists is skipped.
    #[test]
    fn manual_projects_filter_and_merge() {
        let base = std::env::temp_dir().join(format!("noble-manual-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for repo in ["alpha", "junk", "deep/a/b/c/d/far"] {
            std::fs::create_dir_all(base.join(repo).join(".git")).unwrap();
            std::fs::write(base.join(repo).join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        }
        std::fs::create_dir_all(base.join("notes")).unwrap();
        let scanned = scan(std::slice::from_ref(&base), 2, &[], 100);
        let names = |l: &[Project]| {
            let mut v: Vec<String> = l.iter().map(|p| p.name.clone()).collect();
            v.sort();
            v
        };
        assert_eq!(names(&scanned), ["alpha", "junk"]);
        let upper = |p: PathBuf| PathBuf::from(p.to_string_lossy().to_uppercase());
        let manual = Manual {
            hidden: vec![base.join("junk").to_string_lossy().into_owned()],
            added: vec![
                base.join("deep/a/b/c/d/far"),
                base.join("notes"),
                base.join("gone"),
                // Found by the scan too: listed once.
                base.join("alpha"),
                // Same folder with another case (Windows/macOS) or a different folder (Linux).
                if cfg!(any(windows, target_os = "macos")) { upper(base.join("alpha")) } else { base.join("alpha") },
            ],
        };
        let list = manual.apply(scanned);
        assert_eq!(names(&list), ["alpha", "far", "notes"]);
        let far = list.iter().find(|p| p.name == "far").unwrap();
        assert!(far.repo && far.branch.as_deref() == Some("main"));
        let notes = list.iter().find(|p| p.name == "notes").unwrap();
        assert!(!notes.repo && notes.branch.is_none() && notes.last_active.is_some());
        // An added folder that is also hidden stays hidden.
        let both = Manual { hidden: vec![base.join("notes").to_string_lossy().into_owned()], added: manual.added };
        assert!(!both.apply(Vec::new()).iter().any(|p| p.name == "notes"));
        assert!(project_at(&base.join("gone")).is_none());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn containing_project_is_deepest_match() {
        let mk = |p: &str| Project {
            name: p.into(),
            path: PathBuf::from(p),
            branch: None,
            last_active: None,
            git: None,
            repo: true,
        };
        let list = vec![mk("/code/app"), mk("/code/app/vendor/lib"), mk("/code/apple")];
        let find = |c: &str| project_containing(&list, Path::new(c)).map(|p| p.name.as_str());
        assert_eq!(find("/code/app"), Some("/code/app"));
        assert_eq!(find("/code/app/src/x"), Some("/code/app"));
        assert_eq!(find("/code/app/vendor/lib/src"), Some("/code/app/vendor/lib"));
        assert_eq!(find("/code/apple/"), Some("/code/apple"));
        assert_eq!(find("/code/ap"), None);
    }
}

#[cfg(test)]
mod live_tests {
    #[test]
    #[ignore]
    fn live_git_info() {
        let git = crate::util::which("git").expect("git");
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let info = super::git_info(&path, &git);
        println!("{git:?} {info:?}");
        assert!(info.is_some());
    }
}
