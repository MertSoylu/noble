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
    /// Quick tasks as shell commands (`cargo test`, `npm run dev`, `make build`), see `detect_tasks`.
    pub tasks: Vec<String>,
    /// When Claude Code last started a conversation in this folder, see `claude_activity`. Filled in by the scan
    /// thread (never by `scan` itself), so it is `None` until the first pass after the project is listed.
    pub claude: Option<SystemTime>,
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

/// How many commits and files are kept for the project card (a tall card shows more commits, see
/// `ui/bridge.rs` `project_card`).
pub const MAX_COMMITS: usize = 30;
pub const MAX_CHANGES: usize = 40;
/// How many quick tasks are kept per project.
pub const MAX_TASKS: usize = 12;

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
    // The desktop and documents folders by their real (possibly localized) names: `~/Schreibtisch`,
    // `~/Dokumente` on a German Linux (XDG user dirs); Windows and macOS keep the English names on disk.
    let desktop = dirs::desktop_dir().unwrap_or_else(|| home.join("Desktop"));
    let documents = dirs::document_dir().unwrap_or_else(|| home.join("Documents"));
    let mut roots: Vec<PathBuf> = vec![desktop, documents.clone(), documents.join("GitHub")];
    roots.extend(
        [
            "source/repos",
            "projects",
            "Projects",
            // macOS: the folder Apple suggests for code (Finder gives it its own icon, Xcode proposes it).
            "Developer",
            "code",
            "Code",
            "dev",
            "src",
            "repos",
            "git",
            "work",
        ]
        .iter()
        .map(|p| home.join(p)),
    );
    let mut out: Vec<PathBuf> = Vec::new();
    // Windows and macOS: `projects` exists for `Projects` too (case-insensitive file system); the
    // spelling on disk is kept, so paths show and are saved the way the folder is really named, and the
    // folder is added once (`same_path`). On Linux they are two folders.
    for p in roots.into_iter().filter(|p| p.is_dir()) {
        let p = if cfg!(any(windows, target_os = "macos")) { on_disk_case(&p) } else { p };
        if !out.iter().any(|o| crate::util::same_path(o, &p)) {
            out.push(p);
        }
    }
    out
}

/// `path` with its last component spelled as it is on disk (`~/projects` → `~/Projects` on a
/// case-insensitive file system). Unchanged when the parent cannot be read.
fn on_disk_case(path: &Path) -> PathBuf {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else { return path.to_path_buf() };
    let want = name.to_string_lossy().to_lowercase();
    std::fs::read_dir(parent)
        .ok()
        .and_then(|entries| {
            entries.flatten().map(|e| e.file_name()).find(|n| n == name || n.to_string_lossy().to_lowercase() == want)
        })
        .map(|real| parent.join(real))
        .unwrap_or_else(|| path.to_path_buf())
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
    let gd = if dot.is_dir() {
        dot
    } else if dot.is_file() {
        // Capped like the task files: a repository file is not trusted to be small.
        let text = read_small(&dot)?;
        let target = text.trim().strip_prefix("gitdir:")?.trim();
        // Not followed to another machine: on Windows reading `\\server\share\HEAD` connects there and sends the
        // user's credentials (NTLM). The repository is then treated as a plain folder.
        if crate::term::pane::is_network_path(target) {
            return None;
        }
        let p = PathBuf::from(target);
        if p.is_absolute() { p } else { repo.join(p) }
    } else {
        return None;
    };
    // Git reads `commondir` (a linked worktree's shared folder) and `gitdir` (the worktree's own `.git` file) from
    // the git directory itself, so a repository that names a network path there is not run either.
    let names_share = ["commondir", "gitdir"]
        .iter()
        .any(|f| read_small(&gd.join(f)).is_some_and(|t| crate::term::pane::is_network_path(t.trim())));
    (!names_share).then_some(gd)
}

/// Branch name from the `HEAD` file (short hash on a detached HEAD).
pub fn read_branch(git_dir: &Path) -> Option<String> {
    let head = read_small(&git_dir.join("HEAD"))?;
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

/// `~/.claude/projects`, where Claude Code keeps one folder per project. `None` without a home folder and when
/// `ai::isolated` is set (a test must not look at the user's Claude folders).
pub fn claude_projects_dir() -> Option<PathBuf> {
    if crate::ai::isolated() {
        return None;
    }
    Some(dirs::home_dir()?.join(".claude").join("projects"))
}

/// The name Claude Code gives a project's folder under `~/.claude/projects`: every character that is not an ASCII
/// letter or digit becomes `-` (`C:\Users\me\app` → `C--Users-me-app`, `/home/me/app` → `-home-me-app`). A character
/// outside the BMP becomes two, as it does in the JavaScript string Claude builds the name from.
pub fn claude_folder_name(dir: &Path) -> String {
    let mut out = String::new();
    for c in dir.to_string_lossy().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            out.extend(std::iter::repeat_n('-', c.len_utf16()));
        }
    }
    out
}

/// When Claude Code last started a conversation in `dir`: the modification time of its folder under `root`. Claude
/// adds a file per conversation, so the folder changes then. Only the folder itself is looked at, never a transcript.
pub fn claude_activity(root: &Path, dir: &Path) -> Option<SystemTime> {
    let folder = root.join(claude_folder_name(dir));
    std::fs::metadata(&folder).ok().filter(|m| m.is_dir())?.modified().ok()
}

/// Fills `Project::claude` for every listed folder (`root` is `claude_projects_dir`).
fn fill_claude(list: &mut [Project], root: Option<&Path>) {
    for p in list {
        p.claude = root.and_then(|r| claude_activity(r, &p.path));
    }
}

/// Whether a git repository sits inside `dir`, at most `depth` levels down (a quick check for the
/// add prompt: the scan stops at the first one found). `dir` itself is not checked.
pub fn holds_repos(dir: &Path, depth: usize, exclude: &[String]) -> bool {
    !scan(&[dir.to_path_buf()], depth, exclude, 1).is_empty()
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
                tasks: detect_tasks(&dir),
                path: dir,
                git: None,
                repo: true,
                claude: None,
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
    let tasks = detect_tasks(dir);
    Some(match git_dir(dir) {
        Some(gd) => Project {
            name,
            branch: read_branch(&gd),
            last_active: last_active(&gd),
            path: dir.to_path_buf(),
            git: None,
            repo: true,
            tasks,
            claude: None,
        },
        None => Project {
            name,
            branch: None,
            last_active: meta.modified().ok(),
            path: dir.to_path_buf(),
            git: None,
            repo: false,
            tasks,
            claude: None,
        },
    })
}

/// The quick tasks of a project folder, as the shell commands that run them: the Cargo commands for a
/// `Cargo.toml`, `<npm|pnpm|yarn|bun> run <script>` for each `package.json` script (the manager is the one
/// whose lockfile exists), `make <target>` for a Makefile and `just <recipe>` for a justfile. Only small
/// files are read and nothing runs. Duplicates are dropped and the list stops at `MAX_TASKS`.
pub fn detect_tasks(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if dir.join("Cargo.toml").is_file() {
        for cmd in ["cargo build", "cargo test", "cargo run"] {
            push_task(&mut out, cmd.into());
        }
    }
    if let Some(text) = read_small(&dir.join("package.json")) {
        let manager = node_package_manager(dir);
        for name in package_scripts(&text) {
            push_task(&mut out, format!("{manager} run {name}"));
        }
    }
    if let Some(text) = read_first(dir, &["Makefile", "makefile"]) {
        for name in makefile_targets(&text) {
            push_task(&mut out, format!("make {name}"));
        }
    }
    if let Some(text) = read_first(dir, &["justfile", "Justfile"]) {
        for name in just_recipes(&text) {
            push_task(&mut out, format!("just {name}"));
        }
    }
    out
}

fn push_task(out: &mut Vec<String>, cmd: String) {
    if out.len() < MAX_TASKS && !out.contains(&cmd) {
        out.push(cmd);
    }
}

/// The package manager a Node project uses, by its lockfile (npm when none is found).
fn node_package_manager(dir: &Path) -> &'static str {
    if dir.join("pnpm-lock.yaml").is_file() {
        "pnpm"
    } else if dir.join("yarn.lock").is_file() {
        "yarn"
    } else if dir.join("bun.lockb").is_file() || dir.join("bun.lock").is_file() {
        "bun"
    } else {
        "npm"
    }
}

/// The first of `names` that exists in `dir`, read as text.
fn read_first(dir: &Path, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        let path = dir.join(name);
        path.is_file().then(|| read_small(&path)).flatten()
    })
}

/// Most bytes read from a project file: a generated Makefile or lockfile-sized JSON must not stall the caller.
const MAX_TASK_FILE: u64 = 256 * 1024;

/// The start of a text file (at most `MAX_TASK_FILE` bytes); `None` when it cannot be read as UTF-8.
/// A file cut in the middle of a multi-byte character loses that last character, and a cut `package.json`
/// no longer parses (no tasks), which is the intended outcome for a file that large.
fn read_small(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(MAX_TASK_FILE).read_to_end(&mut bytes).ok()?;
    match String::from_utf8(bytes) {
        Ok(s) => Some(s),
        Err(e) => {
            let valid = e.utf8_error().valid_up_to();
            // Only a character cut by the limit is forgiven (the error is within the last 3 bytes).
            (e.as_bytes().len() - valid < 4).then(|| String::from_utf8_lossy(&e.as_bytes()[..valid]).into_owned())
        }
    }
}

/// A name that is safe to type into a shell as it is: letters, digits and `_-.:/@`.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || "_-.:/@".contains(c))
}

/// The script names of a `package.json` (in file order), without the ones a shell could not take as they are.
pub fn package_scripts(text: &str) -> Vec<String> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else { return Vec::new() };
    let Some(scripts) = json.get("scripts").and_then(|s| s.as_object()) else { return Vec::new() };
    scripts.keys().filter(|name| is_plain_name(name)).cloned().collect()
}

/// The plain targets of a Makefile: `name:` lines at column 0. Recipe lines (tab or space indented),
/// comments, variables (`X := a`, `X ?= a`), special targets (`.PHONY`) and pattern or multi-name rules
/// are skipped.
pub fn makefile_targets(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.starts_with(|c: char| c.is_whitespace() || c == '#') {
            continue;
        }
        let Some((head, rest)) = line.split_once(':') else { continue };
        // `X = a`, `X := a`, `X ::= a`: an assignment, not a rule.
        if rest.starts_with('=') || rest.starts_with(":=") {
            continue;
        }
        let name = head.trim_end();
        if name.starts_with('.') || !is_plain_name(name) || name.contains('%') {
            continue;
        }
        if !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
    }
    out
}

/// The recipes of a justfile: column-0 lines `name [params]:` (private `_` recipes and the
/// `set`/`alias`/`export` settings are skipped).
pub fn just_recipes(text: &str) -> Vec<String> {
    const KEYWORDS: [&str; 6] = ["set", "alias", "export", "import", "mod", "unexport"];
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.starts_with(|c: char| c.is_whitespace() || c == '#') {
            continue;
        }
        let Some((head, rest)) = line.split_once(':') else { continue };
        if rest.starts_with('=') {
            continue;
        }
        let Some(name) = head.split_whitespace().next() else { continue };
        let plain = name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if name.starts_with('_') || KEYWORDS.contains(&name) || !plain {
            continue;
        }
        if !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
    }
    out
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

/// Longest a background `git` call may take before it is killed (a hung network mount, a stuck hook).
const GIT_TIMEOUT: Duration = Duration::from_secs(20);

/// Runs `git -C <path> <args>` in the background: no optional locks (a status refresh must not take
/// the index lock a foreground `git commit` needs), no credential prompts, killed after [`GIT_TIMEOUT`].
/// `overrides` are the `-c` settings from [`repo_program_overrides`]. `None` on failure or timeout;
/// stdout is capped by `ai::run_command`.
fn git_run(path: &Path, git: &Path, overrides: &[String], args: &[&str]) -> Option<String> {
    let mut cmd = util::command_for(git);
    cmd.arg("--no-optional-locks");
    for setting in overrides {
        cmd.arg("-c").arg(setting);
    }
    cmd.arg("-C").arg(path).args(args).env("GIT_OPTIONAL_LOCKS", "0").env("GIT_TERMINAL_PROMPT", "0");
    crate::ai::run_command(cmd, GIT_TIMEOUT).ok()
}

/// `git -c` settings that stop a background git from running programs the repository names itself: the
/// `core.fsmonitor` hook (run on every status), `log.showSignature` (runs `gpg.program`) and the
/// `filter.<driver>` clean, smudge and process commands (run when a file's `.gitattributes` names the
/// driver). The driver names are read from the repository's own config (`.git/config` and
/// `config.worktree`) only: the user's global and system filters (git-lfs, usually) stay active.
/// Fails closed: `None` when that config cannot be read, so no status is shown instead of one that
/// runs the repository's programs. A repository with no filters gets the first two settings only.
fn repo_program_overrides(path: &Path, git: &Path) -> Option<Vec<String>> {
    let mut drivers = std::collections::BTreeSet::new();
    for scope in ["--local", "--worktree"] {
        let mut cmd = util::command_for(git);
        cmd.arg("--no-optional-locks").arg("-C").arg(path).args([
            "config",
            "--includes",
            "--null",
            "--name-only",
            "--list",
            scope,
        ]);
        let names = crate::ai::run_command(cmd, GIT_TIMEOUT).ok()?;
        drivers.extend(filter_drivers(&names)?);
    }
    let mut settings = vec!["core.fsmonitor=false".to_string(), "log.showSignature=false".to_string()];
    for driver in drivers {
        for key in ["clean", "smudge", "process"] {
            settings.push(format!("filter.{driver}.{key}="));
        }
        settings.push(format!("filter.{driver}.required=false"));
    }
    Some(settings)
}

/// The `filter.<driver>` names among `git config --name-only --null --list` output. `None` when a
/// name holds a line break or `=`, which a `-c` setting cannot carry (the caller then fails closed).
fn filter_drivers(names: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for name in names.split('\0') {
        let Some(rest) = name.strip_prefix("filter.") else { continue };
        let Some((driver, _key)) = rest.rsplit_once('.') else { continue };
        if driver.is_empty() {
            continue;
        }
        if driver.contains(['=', '\n', '\r']) {
            return None;
        }
        out.push(driver.to_string());
    }
    Some(out)
}

/// Is `git` a working git? macOS: `/usr/bin/git` always exists, but without the Command Line Tools it is
/// a stub that pops up the "install developer tools" dialog every time it runs; `xcode-select -p` tells
/// (without the dialog). Without them no git status is shown, as on a machine with no git at all.
/// Windows and Linux: a `git` on PATH is the real one.
fn git_is_real(git: &Path) -> bool {
    if !cfg!(target_os = "macos") || git != Path::new("/usr/bin/git") {
        return true;
    }
    let mut cmd = util::command_for(Path::new("/usr/bin/xcode-select"));
    cmd.arg("-p");
    crate::ai::run_command(cmd, Duration::from_secs(5)).is_ok_and(|out| !out.trim().is_empty())
}

fn git_info(path: &Path, git: &Path) -> Option<GitInfo> {
    // A non-zero exit (not a repo) prints nothing useful; `git status` on a repo always prints the branch line.
    let overrides = repo_program_overrides(path, git)?;
    let status = git_run(path, git, &overrides, &["status", "--porcelain=v1", "-b", "--untracked-files=normal"])?;
    if status.trim().is_empty() {
        return None;
    }
    let mut info = parse_status(&status);
    let log_args = ["log", &format!("-{MAX_COMMITS}"), "--format=%h%x1f%ct%x1f%an%x1f%s"];
    if let Some(log) = git_run(path, git, &overrides, &log_args) {
        info.commits = parse_log(&log);
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

/// Takes the requests that queued up behind a rescan. Further rescans are the same scan, so they are dropped; a
/// refresh is kept, in order, for after the scan. (`while let Ok(ProjectReq::Rescan) = req.try_recv()` used to take
/// the first request that was not a rescan and throw it away.)
fn drain_after_rescan(req: &std::sync::mpsc::Receiver<ProjectReq>, deferred: &mut Vec<PathBuf>) {
    while let Ok(next) = req.try_recv() {
        if let ProjectReq::Refresh(path) = next {
            deferred.push(path);
        }
    }
}

/// Sends one repo's git status for `ProjectReq::Refresh`. A plain folder (added by hand) has no status; inside
/// another repo `git status` would report that repo's instead. Returns false when the app is gone.
fn refresh_one(path: PathBuf, git: Option<&Path>, tx: &Tx) -> bool {
    if let Some(git) = git
        && git_dir(&path).is_some()
        && let Some(info) = git_info(&path, git)
        && tx.send(AppEvent::Git(path, info)).is_err()
    {
        return false;
    }
    true
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
        let git = util::which("git").filter(|g| git_is_real(g));
        let claude = claude_projects_dir();
        // Last activity times from the previous scan: no git run for unchanged repos.
        let mut seen: std::collections::HashMap<PathBuf, Option<SystemTime>> = Default::default();
        loop {
            let cfg = cfg.lock().map(|c| c.clone()).unwrap_or_default();
            let roots = roots_from(&cfg);
            let list = scan(&roots, cfg.max_depth.clamp(1, 6), &cfg.exclude, 300);
            let mut list = manual.lock().map(|m| m.clone()).unwrap_or_default().apply(list);
            // One stat per listed folder, here on the project thread (not on the UI thread).
            fill_claude(&mut list, claude.as_deref());
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
            let mut deferred: Vec<PathBuf> = Vec::new();
            loop {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                match req.recv_timeout(left) {
                    Ok(ProjectReq::Refresh(path)) => {
                        if !refresh_one(path, git.as_deref(), &tx) {
                            return;
                        }
                    }
                    Ok(ProjectReq::Rescan) | Err(RecvTimeoutError::Timeout) => {
                        drain_after_rescan(&req, &mut deferred);
                        break;
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            // Refreshes that queued up with a rescan run now, before that scan (the next pass of this loop).
            for path in deferred.drain(..) {
                if !refresh_one(path, git.as_deref(), &tx) {
                    return;
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

    /// A `.git` file that points at a network path is not followed (Windows would connect to the server with the
    /// user's credentials to read HEAD); a local target, absolute or relative, still names the repository.
    #[test]
    fn git_file_pointing_at_a_network_path_is_not_followed() {
        let base = std::env::temp_dir().join(format!("noble-gitfile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let real = base.join("real/.git/worktrees/wt");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("HEAD"), "ref: refs/heads/feature\n").unwrap();
        let local = base.join("local");
        std::fs::create_dir_all(&local).unwrap();
        std::fs::write(local.join(".git"), format!("gitdir: {}\n", real.display())).unwrap();
        let relative = base.join("relative");
        std::fs::create_dir_all(&relative).unwrap();
        std::fs::write(relative.join(".git"), "gitdir: ../real/.git/worktrees/wt\n").unwrap();
        assert_eq!(git_dir(&local), Some(real.clone()));
        assert_eq!(git_dir(&relative), Some(relative.join("../real/.git/worktrees/wt")));
        for target in [r"\\attacker\share\x", "//attacker/share/x", r"\\?\UNC\attacker\share\x", r"\\.\pipe\x"] {
            let bad = base.join(format!("bad-{}", target.len()));
            std::fs::create_dir_all(&bad).unwrap();
            std::fs::write(bad.join(".git"), format!("gitdir: {target}\n")).unwrap();
            assert_eq!(git_dir(&bad), None, "{target}");
            assert!(project_at(&bad).is_some_and(|p| !p.repo), "{target} is listed as a plain folder");
        }
        // Git reads `commondir` and a worktree's `gitdir` from the git directory: a network path there is refused too.
        for (file, text) in [("commondir", r"\\attacker\share\x"), ("gitdir", "//attacker/share/x/.git")] {
            let dir = base.join(format!("inside-{file}"));
            let gd = dir.join(".git");
            std::fs::create_dir_all(&gd).unwrap();
            std::fs::write(gd.join("HEAD"), "ref: refs/heads/main\n").unwrap();
            std::fs::write(gd.join(file), format!("{text}\n")).unwrap();
            assert_eq!(git_dir(&dir), None, "{file}");
        }
        // Scanning the folder finds the local repositories and not the ones that point at a share.
        let found = scan(std::slice::from_ref(&base), 3, &[], 100);
        assert!(found.iter().all(|p| !p.name.starts_with("bad-") && !p.name.starts_with("inside-")));
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

    /// Node projects run their scripts with the manager whose lockfile exists; a script name a shell
    /// could not take as it is stays out.
    #[test]
    fn package_json_scripts_become_tasks() {
        let base = std::env::temp_dir().join(format!("noble-tasks-node-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let scripts = r#"{"name": "app", "scripts": {"dev": "vite", "build": "vite build", "lint:fix": "eslint .", "bad name": "x"}}"#;
        let pnpm = base.join("pnpm");
        std::fs::create_dir_all(&pnpm).unwrap();
        std::fs::write(pnpm.join("package.json"), scripts).unwrap();
        std::fs::write(pnpm.join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(detect_tasks(&pnpm), ["pnpm run dev", "pnpm run build", "pnpm run lint:fix"]);
        let yarn = base.join("yarn");
        std::fs::create_dir_all(&yarn).unwrap();
        std::fs::write(yarn.join("package.json"), scripts).unwrap();
        std::fs::write(yarn.join("yarn.lock"), "").unwrap();
        assert_eq!(detect_tasks(&yarn)[0], "yarn run dev");
        let bun = base.join("bun");
        std::fs::create_dir_all(&bun).unwrap();
        std::fs::write(bun.join("package.json"), scripts).unwrap();
        std::fs::write(bun.join("bun.lock"), "").unwrap();
        assert_eq!(detect_tasks(&bun)[0], "bun run dev");
        let npm = base.join("npm");
        std::fs::create_dir_all(&npm).unwrap();
        std::fs::write(npm.join("package.json"), scripts).unwrap();
        assert_eq!(detect_tasks(&npm)[0], "npm run dev");
        // Not JSON, or no scripts object: no tasks.
        assert!(package_scripts("{ not json").is_empty());
        assert!(package_scripts(r#"{"name": "lib"}"#).is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn cargo_project_has_build_test_run() {
        let base = std::env::temp_dir().join(format!("noble-tasks-cargo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        assert!(detect_tasks(&base).is_empty());
        std::fs::write(base.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        assert_eq!(detect_tasks(&base), ["cargo build", "cargo test", "cargo run"]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn makefile_plain_targets_only() {
        let text = "# build file\nCC := gcc\nFLAGS = -O2\nexport PATH := /bin\n.PHONY: all build clean\n\
                    all: build\n\tcc -o app\nbuild: main.o\n\t$(CC) -o app main.o\n%.o: %.c\n\t$(CC) -c $<\n\
                    clean:\n\trm -f app\nbuild:\ntest lint:\ninstall dir:\nVAR ::= x\n# clean: no\n";
        assert_eq!(makefile_targets(text), ["all", "build", "clean"]);
    }

    #[test]
    fn justfile_public_recipes_only() {
        let text = "set shell := [\"bash\"]\nalias t := test\n# comment\nbuild:\n    cargo build\n\
                    test-all target=\"x\": build\n    echo\n_helper:\n    echo\n[private]\nfmt:\n";
        assert_eq!(just_recipes(text), ["build", "test-all", "fmt"]);
    }

    /// Tasks are deduplicated and the list stops at `MAX_TASKS`.
    #[test]
    fn make_tasks_are_capped() {
        let base = std::env::temp_dir().join(format!("noble-tasks-make-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let text: String = (0..20).map(|i| format!("t{i}:\n\techo {i}\n")).collect();
        std::fs::write(base.join("Makefile"), text).unwrap();
        let tasks = detect_tasks(&base);
        assert_eq!(tasks.len(), MAX_TASKS);
        assert_eq!(tasks[0], "make t0");
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
            tasks: Vec::new(),
            claude: None,
        };
        let list = vec![mk("/code/app"), mk("/code/app/vendor/lib"), mk("/code/apple")];
        let find = |c: &str| project_containing(&list, Path::new(c)).map(|p| p.name.as_str());
        assert_eq!(find("/code/app"), Some("/code/app"));
        assert_eq!(find("/code/app/src/x"), Some("/code/app"));
        assert_eq!(find("/code/app/vendor/lib/src"), Some("/code/app/vendor/lib"));
        assert_eq!(find("/code/apple/"), Some("/code/apple"));
        assert_eq!(find("/code/ap"), None);
    }

    #[test]
    fn claude_folder_names_follow_claude_code() {
        // The Windows name is the one Claude Code wrote for a local project (`:\` → `--`, each `\` → `-`).
        assert_eq!(claude_folder_name(Path::new(r"C:\Users\Mert\Desktop\Noble")), "C--Users-Mert-Desktop-Noble");
        assert_eq!(claude_folder_name(Path::new("/home/x/proj")), "-home-x-proj");
        // Every character that is not an ASCII letter or digit becomes one `-`: dots, underscores, spaces.
        assert_eq!(claude_folder_name(Path::new("/home/x/my_app.v2 beta")), "-home-x-my-app-v2-beta");
        // Outside the BMP a character is two UTF-16 units, so two dashes.
        assert_eq!(claude_folder_name(Path::new("/a/😀")), "-a---");
    }

    #[test]
    fn claude_activity_reads_only_the_folder_time() {
        let base = std::env::temp_dir().join(format!("noble-claude-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("projects");
        let project = base.join("work").join("app");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        // No folder for the project yet: no activity.
        assert_eq!(claude_activity(&root, &project), None);
        // A folder with a conversation file in it: its modification time is the activity.
        let folder = root.join(claude_folder_name(&project));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("session.jsonl"), "{}\n").unwrap();
        let seen = claude_activity(&root, &project).expect("the folder's time");
        let folder_time = std::fs::metadata(&folder).unwrap().modified().unwrap();
        assert_eq!(seen, folder_time);
        // A file where the folder should be is not a conversation folder.
        let other = base.join("work").join("other");
        std::fs::write(root.join(claude_folder_name(&other)), "x").unwrap();
        assert_eq!(claude_activity(&root, &other), None);
        // The scan thread's fill leaves the field empty when there is no folder to read.
        let mut list = vec![Project {
            name: "app".into(),
            path: project.clone(),
            branch: None,
            last_active: None,
            git: None,
            repo: false,
            tasks: Vec::new(),
            claude: Some(SystemTime::UNIX_EPOCH),
        }];
        fill_claude(&mut list, None);
        assert_eq!(list[0].claude, None);
        fill_claude(&mut list, Some(&root));
        assert_eq!(list[0].claude, Some(folder_time));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn filter_driver_names_from_config_names() {
        let names = "core.bare\0filter.lfs.clean\0filter.lfs.process\0filter.a.b.smudge\0filter.broken\0filter.\0";
        assert_eq!(filter_drivers(names), Some(vec!["lfs".to_string(), "lfs".into(), "a.b".into()]));
        // A name a `-c` setting cannot carry fails closed.
        assert_eq!(filter_drivers("filter.we=ird.clean\0"), None);
        assert_eq!(filter_drivers("filter.a\nb.clean\0"), None);
    }

    /// Requests that queued up behind a rescan: the rescans collapse, and every refresh is kept in order. A refresh
    /// that followed a rescan used to be dropped.
    #[test]
    fn refreshes_behind_a_rescan_are_kept() {
        let (tx, rx) = std::sync::mpsc::channel();
        let (a, b, c) = (PathBuf::from("a"), PathBuf::from("b"), PathBuf::from("c"));
        for req in
            [ProjectReq::Rescan, ProjectReq::Refresh(a.clone()), ProjectReq::Rescan, ProjectReq::Refresh(b.clone())]
        {
            tx.send(req).unwrap();
        }
        tx.send(ProjectReq::Refresh(c.clone())).unwrap();
        assert!(matches!(rx.recv(), Ok(ProjectReq::Rescan)));
        let mut deferred = Vec::new();
        drain_after_rescan(&rx, &mut deferred);
        assert_eq!(deferred, vec![a, b, c]);
        assert!(rx.try_recv().is_err(), "the queue is empty after the drain");
    }

    /// A background status must not run what the repository's own config names: `core.fsmonitor` (a hook
    /// run on every status), and the clean and process filters of a driver the attributes give the files.
    /// Each command touches its own marker. The control run (plain git) shows the commands do run; after
    /// `git_info` none may have run.
    #[test]
    fn background_git_runs_no_program_from_the_repo_config() {
        let _serial = crate::ai::SPAWN_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let git = crate::util::which("git").expect("git on PATH");
        let dir = std::env::temp_dir().join(format!("noble-hostile-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // The setup reads no user or system git config (a signing key or an `include` there would change what the
        // test does): the global config is an empty file, and the system config is off.
        let empty_config = std::env::temp_dir().join(format!("noble-hostile-git-{}.gitconfig", std::process::id()));
        std::fs::write(&empty_config, b"").unwrap();
        let run = |args: &[&str]| {
            let out = std::process::Command::new(&git)
                .arg("-C")
                .arg(&dir)
                .env("GIT_CONFIG_GLOBAL", &empty_config)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .args(["-c", "user.name=noble", "-c", "user.email=noble@example.invalid"])
                .args(["-c", "commit.gpgsign=false"])
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        run(&["init", "-q"]);
        std::fs::write(dir.join("tracked.txt"), "hi\n").unwrap();
        std::fs::write(dir.join(".gitattributes"), "* filter=evil\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init"]);
        // Same size, new content: git has to read the file, which runs the clean filter. The untracked
        // file keeps the status output non-empty (`git_info` returns nothing for a clean tree).
        std::fs::write(dir.join("tracked.txt"), "ho\n").unwrap();
        std::fs::write(dir.join("untracked.txt"), "new\n").unwrap();
        // Each command writes its marker with the shell's own redirection (git runs these through its `sh`, on
        // Windows too), so no external program such as `touch` is needed. `#` turns the arguments git appends to the
        // hook (protocol version and token) into a comment.
        let marker = |name: &str| dir.join(name).to_string_lossy().replace('\\', "/");
        run(&["config", "core.fsmonitor", &format!("echo x > \"{}\" #", marker("fsmonitor"))]);
        run(&["config", "filter.evil.clean", &format!("echo x > \"{}\"; cat", marker("clean"))]);
        run(&["config", "filter.evil.smudge", "cat"]);
        run(&["config", "filter.evil.process", &format!("echo x > \"{}\"", marker("process"))]);
        let names = ["fsmonitor", "clean", "process"];
        let ran = |dir: &Path| names.into_iter().filter(|n| dir.join(n).exists()).collect::<Vec<_>>();

        // The control's exit status does not matter: the hostile fsmonitor makes plain git fail, too.
        let _ = std::process::Command::new(&git)
            .arg("-C")
            .arg(&dir)
            .env("GIT_CONFIG_GLOBAL", &empty_config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args(["status", "--porcelain=v1"])
            .output();
        let control_ran = ran(&dir);
        for name in names {
            let _ = std::fs::remove_file(dir.join(name));
        }
        let info = git_info(&dir, &git);
        let guarded_ran = ran(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&empty_config);

        assert!(control_ran.contains(&"fsmonitor"), "the control run should show the hazard: {control_ran:?}");
        assert!(info.is_some(), "a background status still runs without the repo's programs");
        assert!(guarded_ran.is_empty(), "the repository's own config ran {guarded_ran:?}");
    }
}

#[cfg(test)]
mod live_tests {
    #[test]
    #[ignore]
    fn live_git_info() {
        // Runs a real background command: see `ai::SPAWN_TESTS`.
        let _serial = crate::ai::SPAWN_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let git = crate::util::which("git").expect("git");
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let info = super::git_info(&path, &git);
        println!("{git:?} {info:?}");
        assert!(info.is_some());
    }
}
