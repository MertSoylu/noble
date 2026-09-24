//! Proje keşfi: kök klasörlerde git depolarını bulur, branch ve son etkinliği
//! dosya sisteminden okur, değişiklik sayısını arka planda `git` ile toplar.

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
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GitInfo {
    /// Commit edilmemiş toplam değişiklik (yeni dosyalar dahil).
    pub dirty: u32,
    /// Bunlardan git'in henüz izlemediği yeni dosyalar.
    pub untracked: u32,
    pub ahead: u32,
    pub behind: u32,
    pub branch: Option<String>,
    pub last_subject: Option<String>,
    pub last_commit: Option<i64>,
    /// Son commit'ler, en yenisi önce (Home'daki proje kartı için).
    pub commits: Vec<Commit>,
    /// Değişen dosyalar: iki harflik porcelain durumu ve yol (en fazla `MAX_CHANGES`).
    pub changes: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Commit {
    pub hash: String,
    pub time: i64,
    pub author: String,
    pub subject: String,
}

/// Proje kartı için tutulan en fazla commit ve dosya sayısı.
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

/// Varsayılan kök klasörler: var olanlar.
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
    // Windows'ta büyük/küçük harf farkı aynı klasörü iki kez eklemesin.
    roots.dedup_by(|a, b| a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy()));
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

/// `.git` bir klasör ya da (worktree/submodule için) `gitdir:` içeren bir dosya olabilir.
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

/// `HEAD` dosyasından branch adı (ayrık HEAD'de kısa hash).
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

/// Kök klasörleri tarar (derinlik sınırlı, en fazla `limit` proje).
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

pub fn sort_projects(list: &mut [Project]) {
    list.sort_by(|a, b| {
        b.last_active.cmp(&a.last_active).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

/// `git status --porcelain=v1 -b` çıktısını çözer.
pub fn parse_status(output: &str) -> GitInfo {
    let mut info = GitInfo::default();
    for line in output.lines() {
        if let Some(header) = line.strip_prefix("## ") {
            let (branch_part, tracking) = match header.find(" [") {
                Some(i) => (&header[..i], &header[i + 2..header.len().saturating_sub(1).max(i + 2)]),
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
            if info.changes.len() < MAX_CHANGES && line.len() > 3 {
                // "XY yol" ya da yeniden adlandırmada "XY eski -> yeni".
                let path = line[3..].rsplit(" -> ").next().unwrap_or(&line[3..]).trim_matches('"');
                info.changes.push((line[..2].to_string(), path.to_string()));
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

/// `git log --format=%h%x1f%ct%x1f%an%x1f%s` çıktısını çözer.
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

/// Proje iş parçacığına istekler.
pub enum ProjectReq {
    /// Kök klasörleri baştan tara.
    Rescan,
    /// Tek bir deponun git durumunu hemen yenile (komut bitti, liste kaydırıldı…).
    Refresh(PathBuf),
}

/// Otomatik yeniden tarama aralığı. Git durumu komut bitince ve Home açılınca
/// ayrıca istenir; bu tarama yalnızca yeni/silinen repoları yakalar.
const RESCAN_EVERY: Duration = Duration::from_secs(300);
/// Taramadan hemen sonra git durumu alınan (en son kullanılan) proje sayısı;
/// kalanlar listede göründükçe `Refresh` ile istenir.
pub const EAGER_GIT: usize = 40;

/// `cwd`'yi içeren en derin proje (ör. `repo/src/x` → `repo`).
pub fn project_containing<'a>(list: &'a [Project], cwd: &Path) -> Option<&'a Project> {
    let norm = |p: &Path| {
        let s = p.to_string_lossy().replace('\\', "/");
        let s = s.trim_end_matches('/').to_string();
        if cfg!(windows) { s.to_lowercase() } else { s }
    };
    let c = norm(cwd);
    list.iter()
        .filter(|p| {
            let root = norm(&p.path);
            !root.is_empty() && (c == root || c.starts_with(&format!("{root}/")))
        })
        .max_by_key(|p| p.path.as_os_str().len())
}

/// Tarama + git durumu iş parçacığı. `Rescan` gelince ya da süre dolunca
/// yeniden tarar; `Refresh` tek bir deponun durumunu arada günceller.
pub fn spawn(cfg: std::sync::Arc<std::sync::Mutex<ProjectsCfg>>, tx: Tx, req: std::sync::mpsc::Receiver<ProjectReq>) {
    use std::sync::mpsc::RecvTimeoutError;
    let _ = std::thread::Builder::new().name("projects".into()).spawn(move || {
        let git = util::which("git");
        // Önceki taramadaki son etkinlik zamanları: değişmeyen repo için git çalıştırılmaz.
        let mut seen: std::collections::HashMap<PathBuf, Option<SystemTime>> = Default::default();
        loop {
            let cfg = cfg.lock().map(|c| c.clone()).unwrap_or_default();
            let roots = roots_from(&cfg);
            let list = scan(&roots, cfg.max_depth.clamp(1, 6), &cfg.exclude, 300);
            let targets: Vec<PathBuf> = list
                .iter()
                .take(EAGER_GIT)
                .filter(|p| seen.get(&p.path) != Some(&p.last_active))
                .map(|p| p.path.clone())
                .collect();
            seen = list.iter().map(|p| (p.path.clone(), p.last_active)).collect();
            if tx.send(AppEvent::Projects(list)).is_err() {
                return;
            }
            if let Some(git) = &git {
                // Birkaç paralel işçi; en son kullanılan projeler önce.
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
                        if let Some(git) = &git
                            && let Some(info) = git_info(&path, git)
                            && tx.send(AppEvent::Git(path, info)).is_err()
                        {
                            return;
                        }
                    }
                    Ok(ProjectReq::Rescan) | Err(RecvTimeoutError::Timeout) => {
                        // Aynı anda gelen fazladan tarama isteklerini yut.
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

    #[test]
    fn containing_project_is_deepest_match() {
        let mk =
            |p: &str| Project { name: p.into(), path: PathBuf::from(p), branch: None, last_active: None, git: None };
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
