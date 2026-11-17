//! Agent duel: the same task given to two coding agents, each in its own git worktree of one project, side by side,
//! and then compared. Everything here is independent of the UI: names, the command line that starts an agent with
//! its task, the git calls (they block, so `app/duel.rs` runs them on a background thread) and the parsers of what
//! git prints. NOBLE never merges, commits or pushes anything: it creates two worktrees and two branches, shows
//! what each side changed, and removes the worktrees when asked (the branches stay unless the user also asks for
//! them to go).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::projects::git_checked;
use crate::store::{DuelRecord, DuelSide};
use crate::term::pane::ShellSpec;
use crate::util;

/// The longest task NOBLE passes on. It ends up on one command line (cmd.exe's limit is 8191 characters).
pub const MAX_PROMPT: usize = 4000;
/// How many files a side's list keeps (the totals count them all).
pub const MAX_FILES: usize = 400;
/// Quick git questions (`rev-parse`, `status`, `diff --numstat`).
const GIT_QUICK: Duration = Duration::from_secs(20);
/// `git worktree add` checks out the whole tree: a big repository takes a while.
const GIT_CHECKOUT: Duration = Duration::from_secs(180);
/// Counting the lines of an untracked file reads at most this much of it.
const COUNT_LIMIT: u64 = 1 << 20;

/// One of the two sides of a duel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    A,
    B,
}

impl Side {
    pub const BOTH: [Side; 2] = [Side::A, Side::B];

    /// "a" / "b": the branch suffix and the worktree folder name.
    pub fn letter(self) -> &'static str {
        match self {
            Side::A => "a",
            Side::B => "b",
        }
    }

    pub fn other(self) -> Side {
        match self {
            Side::A => Side::B,
            Side::B => Side::A,
        }
    }
}

impl DuelRecord {
    pub fn side(&self, side: Side) -> &DuelSide {
        match side {
            Side::A => &self.a,
            Side::B => &self.b,
        }
    }

    /// The base commit as shown to people (`1a2b3c4`).
    pub fn base_short(&self) -> &str {
        self.base.get(..7).unwrap_or(&self.base)
    }

    /// The tab's name (`⚔ duel k3x9qa`).
    pub fn tab_name(&self) -> String {
        tab_name(&self.id)
    }

    /// The folder that holds both worktrees (`<data>/duels/<project>-<id>`).
    pub fn root(&self) -> Option<PathBuf> {
        Path::new(&self.a.worktree).parent().map(Path::to_path_buf)
    }
}

pub fn tab_name(id: &str) -> String {
    format!("⚔ duel {id}")
}

// ─── Names ───────────────────────────────────────────────────────────────────────────────────────

const ID_LEN: usize = 6;

/// A new short id: six lowercase letters and digits, random per call. Part of both branch names and of the folder.
pub fn new_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    hasher.write_u32(std::process::id());
    id_from(hasher.finish())
}

/// The id a number stands for: [`ID_LEN`] base-36 digits.
pub fn id_from(mut n: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = String::with_capacity(ID_LEN);
    for _ in 0..ID_LEN {
        out.push(DIGITS[(n % 36) as usize] as char);
        n /= 36;
    }
    out
}

/// Is `id` one this module hands out (it goes into branch names and folder names, so nothing else is trusted)?
pub fn valid_id(id: &str) -> bool {
    id.len() == ID_LEN && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// `noble/duel-<id>-a`.
pub fn branch_name(id: &str, side: Side) -> String {
    format!("noble/duel-{id}-{}", side.letter())
}

/// The project's name as a folder name: letters, digits, `-`, `_` and `.` stay, anything else becomes `-`; no leading
/// or trailing `-`/`.`; at most 32 characters; `project` when nothing is left.
pub fn folder_part(project: &str) -> String {
    let mapped: String = project
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' })
        .collect();
    let mut out = String::new();
    for c in mapped.chars() {
        // `--` runs collapse into one `-`.
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }
    let out: String = out.trim_matches(['-', '.']).chars().take(32).collect();
    let out = out.trim_matches(['-', '.']).to_string();
    if out.is_empty() { "project".into() } else { out }
}

/// Where all duels live: `<data>/duels`.
pub fn duels_root(data: &Path) -> PathBuf {
    data.join("duels")
}

/// The folder of one duel: `<data>/duels/<project>-<id>`.
pub fn duel_dir(data: &Path, project_name: &str, id: &str) -> PathBuf {
    duels_root(data).join(format!("{}-{id}", folder_part(project_name)))
}

/// A side's worktree: `<data>/duels/<project>-<id>/a` (or `b`).
pub fn worktree_path(data: &Path, project_name: &str, id: &str, side: Side) -> PathBuf {
    duel_dir(data, project_name, id).join(side.letter())
}

/// A path as a comparable string: forward slashes, no trailing one, lowercase where the file system ignores case.
fn norm(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = s.trim_end_matches('/').to_string();
    // macOS: a shell may report `/private/var/...` for the folder it was started in as `/var/...`.
    let s = if cfg!(target_os = "macos") { without_private(&s).to_string() } else { s };
    if cfg!(any(windows, target_os = "macos")) { s.to_lowercase() } else { s }
}

/// `/private/var/x` as `/var/x` (also `/private/tmp` and `/private/etc`): the macOS symlinks to those folders.
fn without_private(path: &str) -> &str {
    for dir in ["var", "tmp", "etc"] {
        if let Some(rest) = path.strip_prefix("/private/").and_then(|r| r.strip_prefix(dir))
            && (rest.is_empty() || rest.starts_with('/'))
        {
            return &path["/private".len()..];
        }
    }
    path
}

/// Is `path` the folder `root` or inside it?
pub fn is_inside(path: &Path, root: &Path) -> bool {
    let (p, r) = (norm(path), norm(root));
    !r.is_empty() && (p == r || p.starts_with(&format!("{r}/")))
}

/// Which side's worktree `cwd` is in, if it is in either.
pub fn side_of(rec: &DuelRecord, cwd: &Path) -> Option<Side> {
    Side::BOTH.into_iter().find(|s| is_inside(cwd, Path::new(&rec.side(*s).worktree)))
}

// ─── Starting the agents ────────────────────────────────────────────────────────────────────────

/// How an agent's interactive command line takes the task to start with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptForm {
    /// `agent "task"`.
    Positional,
    /// `agent <flag> "task"`.
    Flag(&'static str),
}

/// The documented way to start `agent` (an `ai::agent_kind` id) in its interactive mode with a first task, `None`
/// for an agent whose form is not known: it is left out of the duel instead of guessing.
///
/// - Claude Code: `claude "task"` starts a session with that first prompt.
/// - Codex: `codex "task"`, the CLI's `[PROMPT]` argument.
/// - GitHub Copilot: `copilot -i "task"` (`--interactive`) starts interactive mode and runs the prompt; `-p` would
///   run it and exit.
/// - OpenCode: `opencode --prompt "task"` for the TUI; its positional argument is a project folder.
///
/// Not offered: Antigravity (`agy`) and the other launchers (pi, oh-my-pi, freebuff, grok, command code, cline, kilo
/// code) have no form NOBLE could check; Cursor's `cursor-agent` takes a positional prompt in its documentation, but
/// NOBLE's own notes on it are unverified, so it stays out until a live run confirms it.
pub fn prompt_form(agent: &str) -> Option<PromptForm> {
    match agent {
        "claude" | "codex" => Some(PromptForm::Positional),
        "copilot" => Some(PromptForm::Flag("-i")),
        "opencode" => Some(PromptForm::Flag("--prompt")),
        _ => None,
    }
}

/// The agent a launcher command runs, when the duel can start it with a task. The program the command starts must be
/// the agent itself (`claude`, `/usr/bin/codex`, `copilot.cmd` …): a wrapper script or a shell that merely mentions
/// it is not, since the task would land on the wrong command line.
pub fn duel_agent(launcher_command: &str) -> Option<&'static str> {
    let program = crate::ai::program_word(launcher_command);
    let name = program.rsplit(['/', '\\']).next()?;
    let stem = Path::new(name).file_stem()?.to_string_lossy().to_lowercase();
    ["claude", "codex", "copilot", "opencode"].into_iter().find(|agent| *agent == stem)
}

/// The task as it is passed on: line breaks as `\n`, other control characters dropped (tabs become a space), no
/// blank space around it, cut to [`MAX_PROMPT`] characters.
pub fn clean_prompt(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let kept: String = text
        .chars()
        .filter_map(|c| match c {
            '\n' => Some('\n'),
            '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .take(MAX_PROMPT + 1)
        .collect();
    kept.trim().chars().take(MAX_PROMPT).collect()
}

/// The command line that starts the agent of `launcher_command` with `prompt`, quoted for `shell` (any word of the
/// task stays text: no expansion, no command substitution). The caller runs it through `ShellSpec::invocation`.
/// Errors name what to change: an empty task, a task that starts with `-` (the agent would read an option), an
/// agent without a known form, or cmd.exe, which has no way to quote `"`, `%`, `!` or a line break.
pub fn launch_line(launcher_command: &str, prompt: &str, shell: &ShellSpec) -> Result<String, String> {
    let agent = duel_agent(launcher_command).ok_or("this launcher has no known way to start with a task")?;
    let form = prompt_form(agent).ok_or_else(|| format!("{agent} has no known way to start with a task"))?;
    let prompt = clean_prompt(prompt);
    if prompt.is_empty() {
        return Err("type the task first".into());
    }
    if prompt.starts_with('-') {
        return Err("the task cannot start with “-”: the agent would read it as an option".into());
    }
    let base = launcher_command.trim();
    let head = match form {
        PromptForm::Positional => base.to_string(),
        PromptForm::Flag(flag) => format!("{base} {flag}"),
    };
    shell.command_with_arg(&head, &prompt).ok_or_else(|| {
        "this shell (cmd.exe) cannot pass line breaks, quotes, % or ! in the task: reword it or use PowerShell".into()
    })
}

// ─── Creating the worktrees ─────────────────────────────────────────────────────────────────────

/// What to create (everything the background thread needs; the thread touches no `App` state).
#[derive(Clone, Debug)]
pub struct Plan {
    pub project: PathBuf,
    pub name: String,
    /// NOBLE's data folder: the worktrees go under `<data>/duels`.
    pub data: PathBuf,
    pub id: String,
    pub prompt: String,
    /// `(agent id, launcher command)` of each side.
    pub a: (String, String),
    pub b: (String, String),
}

/// Creates the two worktrees and their branches from the project's current HEAD and returns the record to keep.
/// A failure removes whatever was created before it. Blocks (git checks out two trees): background thread only.
pub fn create(plan: &Plan) -> Result<DuelRecord, String> {
    if !valid_id(&plan.id) {
        return Err("invalid duel id".into());
    }
    if projects_git_dir(&plan.project).is_none() {
        return Err(format!("{} is not a git repository", plan.name));
    }
    let dir = duel_dir(&plan.data, &plan.name, &plan.id);
    // Never inside the repository: git would see the worktrees as untracked files of the project.
    if is_inside(&dir, &plan.project) {
        return Err("NOBLE's data folder is inside the project: the worktrees would end up in the repository".into());
    }
    let base = git_checked(&plan.project, &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"], GIT_QUICK)
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|h| h.len() >= 40 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or("the repository has no commits yet: there is nothing to branch from")?;
    let base_branch = git_checked(&plan.project, &["symbolic-ref", "--quiet", "--short", "HEAD"], GIT_QUICK)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    // A branch that exists is never reused, and never deleted by the clean-up below: say so before anything is made.
    for side in Side::BOTH {
        let branch = branch_name(&plan.id, side);
        let exists = git_checked(
            &plan.project,
            &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")],
            GIT_QUICK,
        )
        .is_ok_and(|out| !out.trim().is_empty());
        if exists {
            return Err(format!("the branch {branch} already exists"));
        }
    }
    util::create_private_dir(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let mut made: Vec<(Side, PathBuf)> = Vec::new();
    for side in Side::BOTH {
        let path = worktree_path(&plan.data, &plan.name, &plan.id, side);
        let branch = branch_name(&plan.id, side);
        let step = path
            .to_str()
            .ok_or_else(|| "the worktree path is not valid text".to_string())
            .and_then(|p| git_checked(&plan.project, &["worktree", "add", "-b", &branch, p, &base], GIT_CHECKOUT));
        match step {
            Ok(_) => made.push((side, path)),
            Err(e) => {
                rollback(&plan.project, &plan.id, &made, &dir);
                // The branch of the side that failed may exist when the checkout (not the branch) went wrong; it did
                // not exist before (checked above), so it is ours.
                let _ = git_checked(&plan.project, &["branch", "-D", &branch], GIT_QUICK);
                return Err(format!("side {}: {}", side.letter().to_uppercase(), clean_git_error(&e)));
            }
        }
    }
    let side = |s: Side, (agent, launcher): &(String, String)| DuelSide {
        agent: agent.clone(),
        launcher: launcher.clone(),
        branch: branch_name(&plan.id, s),
        worktree: worktree_path(&plan.data, &plan.name, &plan.id, s).display().to_string(),
        extra: Default::default(),
    };
    Ok(DuelRecord {
        id: plan.id.clone(),
        project: plan.project.display().to_string(),
        name: plan.name.clone(),
        base,
        base_branch,
        prompt: plan.prompt.clone(),
        created: chrono::Utc::now().timestamp(),
        a: side(Side::A, &plan.a),
        b: side(Side::B, &plan.b),
        extra: Default::default(),
    })
}

/// Takes back what `create` made before it failed: the worktrees (nothing was in them yet), their branches and the
/// folder.
fn rollback(project: &Path, id: &str, made: &[(Side, PathBuf)], dir: &Path) {
    for (side, path) in made {
        if let Some(p) = path.to_str() {
            let _ = git_checked(project, &["worktree", "remove", "--force", p], GIT_QUICK);
        }
        let _ = git_checked(project, &["branch", "-D", &branch_name(id, *side)], GIT_QUICK);
    }
    let _ = std::fs::remove_dir(dir);
}

/// `fatal: a branch named 'x' already exists` → `a branch named 'x' already exists`.
fn clean_git_error(text: &str) -> String {
    let text = text.trim();
    let text = text.strip_prefix("fatal: ").or_else(|| text.strip_prefix("error: ")).unwrap_or(text);
    util::truncate(text, 160)
}

fn projects_git_dir(project: &Path) -> Option<PathBuf> {
    crate::projects::git_dir(project)
}

// ─── Comparing ──────────────────────────────────────────────────────────────────────────────────

/// One file a side changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    /// Lines added and removed against the base commit; `None` for a binary file or one too large to count.
    pub added: Option<u32>,
    pub removed: Option<u32>,
    /// The file is in a commit of the branch.
    pub committed: bool,
    /// The file has changes in the worktree that are not committed (an untracked file included).
    pub uncommitted: bool,
    /// A new file git does not track yet.
    pub untracked: bool,
}

/// What one side did, against the duel's base commit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SideReport {
    pub agent: String,
    pub branch: String,
    pub worktree: PathBuf,
    /// The worktree folder is still there.
    pub worktree_exists: bool,
    /// Commits on the branch since the base.
    pub commits: u32,
    /// The files, by path (at most [`MAX_FILES`]).
    pub files: Vec<FileChange>,
    /// How many files in all (more than `files.len()` when the list was cut).
    pub file_count: usize,
    /// Lines added and removed over every file with a known count.
    pub added: u32,
    pub removed: u32,
    /// How many files have uncommitted changes.
    pub uncommitted: usize,
    /// Why part of this side could not be read.
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompareReport {
    pub id: String,
    pub a: SideReport,
    pub b: SideReport,
}

impl CompareReport {
    pub fn side(&self, side: Side) -> &SideReport {
        match side {
            Side::A => &self.a,
            Side::B => &self.b,
        }
    }
}

/// `git diff --numstat` lines: `added<TAB>removed<TAB>path`, with `-` for a binary file. A line that does not have
/// that shape is skipped.
pub fn parse_numstat(text: &str) -> Vec<(String, Option<u32>, Option<u32>)> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.splitn(3, '\t');
            let (added, removed, path) = (f.next()?, f.next()?, f.next()?);
            let count = |s: &str| if s == "-" { Some(None) } else { s.parse::<u32>().ok().map(Some) };
            let path = unquote(path.trim_end_matches('\r'));
            if path.is_empty() {
                return None;
            }
            Some((path, count(added)?, count(removed)?))
        })
        .collect()
}

/// One line of `git status --porcelain=v1`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusEntry {
    /// The two status letters (`??` for a file git does not track).
    pub code: String,
    /// The path; the new one for a rename.
    pub path: String,
}

impl StatusEntry {
    pub fn untracked(&self) -> bool {
        self.code == "??"
    }
}

/// `git status --porcelain=v1` lines (`XY path`, `XY old -> new` for a rename). Lines that do not have that shape are
/// skipped; at most `limit` entries are returned.
pub fn parse_porcelain(text: &str, limit: usize) -> Vec<StatusEntry> {
    text.lines()
        .filter_map(|line| {
            let code = line.get(..2)?;
            let rest = line.get(3..)?;
            if rest.is_empty() || !code.is_ascii() || line.as_bytes().get(2) != Some(&b' ') {
                return None;
            }
            // A rename shows `old -> new`; the quoted forms keep a literal " -> " inside their quotes, which a
            // plain split cannot tell apart, so only an unquoted line is split.
            let path = if code.contains('R') || code.contains('C') {
                if rest.starts_with('"') {
                    unquote(rest.rsplit_once("\" -> ").map_or(rest, |(_, new)| new))
                } else {
                    unquote(rest.rsplit(" -> ").next().unwrap_or(rest))
                }
            } else {
                unquote(rest)
            };
            (!path.is_empty()).then(|| StatusEntry { code: code.to_string(), path })
        })
        .take(limit)
        .collect()
}

/// A path as git prints it: wrapped in `"` with `\"`, `\\`, `\t` and `\n` escapes when it has unusual characters
/// (`core.quotepath=false` keeps non-ASCII text as it is). An unquoted path comes back as it is.
fn unquote(text: &str) -> String {
    let t = text.trim_end();
    let Some(inner) = t.strip_prefix('"').and_then(|s| s.strip_suffix('"')) else { return t.to_string() };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// The file list of one side: every file the worktree differs from the base in (`net`: numstat of the base against
/// the working tree, or against the branch when the worktree is gone), plus the files git does not track
/// (`count_lines` counts their lines). `committed` is the numstat of the commits, `status` the porcelain entries of
/// the worktree; they only set the flags. A file that was changed and put back has no entry in `net`: it is not listed.
pub fn merge_files(
    net: &[(String, Option<u32>, Option<u32>)],
    committed: &[(String, Option<u32>, Option<u32>)],
    status: &[StatusEntry],
    count_lines: &dyn Fn(&str) -> Option<u32>,
) -> Vec<FileChange> {
    let mut map: BTreeMap<String, FileChange> = BTreeMap::new();
    for (path, added, removed) in net {
        map.insert(
            path.clone(),
            FileChange { path: path.clone(), added: *added, removed: *removed, ..Default::default() },
        );
    }
    for entry in status.iter().filter(|e| e.untracked()) {
        let lines = count_lines(&entry.path);
        map.insert(
            entry.path.clone(),
            FileChange {
                path: entry.path.clone(),
                added: lines,
                removed: lines.map(|_| 0),
                uncommitted: true,
                untracked: true,
                ..Default::default()
            },
        );
    }
    for (path, ..) in committed {
        if let Some(f) = map.get_mut(path) {
            f.committed = true;
        }
    }
    for entry in status.iter().filter(|e| !e.untracked()) {
        if let Some(f) = map.get_mut(&entry.path) {
            f.uncommitted = true;
        }
    }
    map.into_values().collect()
}

/// The lines of a file in a worktree, for a new file that numstat does not show. `None` for a link, a binary file
/// (a NUL byte), one that is not a plain file, or one larger than [`COUNT_LIMIT`].
fn count_file_lines(worktree: &Path, rel: &str) -> Option<u32> {
    let path = worktree.join(rel);
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > COUNT_LIMIT {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    let breaks = bytes.iter().filter(|b| **b == b'\n').count();
    let open_line = usize::from(bytes.last().is_some_and(|b| *b != b'\n'));
    u32::try_from(breaks + open_line).ok()
}

/// Reads what each side did. Blocks (about eight quick git calls): background thread only.
pub fn compare(rec: &DuelRecord) -> CompareReport {
    let side = |s: Side| compare_side(rec, s);
    CompareReport { id: rec.id.clone(), a: side(Side::A), b: side(Side::B) }
}

fn compare_side(rec: &DuelRecord, side: Side) -> SideReport {
    let s = rec.side(side);
    let project = Path::new(&rec.project);
    let worktree = PathBuf::from(&s.worktree);
    let mut report = SideReport {
        agent: s.agent.clone(),
        branch: s.branch.clone(),
        worktree_exists: worktree.is_dir(),
        worktree: worktree.clone(),
        ..Default::default()
    };
    let quote = ["-c", "core.quotepath=false"];
    let numstat = |dir: &Path, rest: &[&str]| -> Result<Vec<_>, String> {
        let mut args = quote.to_vec();
        args.extend(["diff", "--numstat", "--no-renames"]);
        args.extend(rest);
        git_checked(dir, &args, GIT_QUICK).map(|t| parse_numstat(&t))
    };
    // The commits of the branch: from the project, so it works while the worktree is gone too.
    let range = format!("{}...{}", rec.base, s.branch);
    let committed = match numstat(project, &[&range]) {
        Ok(list) => list,
        Err(e) => {
            report.error = Some(clean_git_error(&e));
            Vec::new()
        }
    };
    report.commits = git_checked(project, &["rev-list", "--count", &format!("{}..{}", rec.base, s.branch)], GIT_QUICK)
        .ok()
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(0);
    let (net, status) = if report.worktree_exists {
        let net = numstat(&worktree, &[&rec.base]);
        let status = git_checked(&worktree, &["status", "--porcelain=v1", "--untracked-files=all"], GIT_QUICK)
            .map(|t| parse_porcelain(&t, MAX_FILES * 4));
        match (net, status) {
            (Ok(net), Ok(status)) => (net, status),
            (net, status) => {
                let why = net.err().or(status.err()).unwrap_or_default();
                report.error.get_or_insert_with(|| clean_git_error(&why));
                (committed.clone(), Vec::new())
            }
        }
    } else {
        (committed.clone(), Vec::new())
    };
    let mut files = merge_files(&net, &committed, &status, &|rel| count_file_lines(&worktree, rel));
    report.file_count = files.len();
    report.added = files.iter().filter_map(|f| f.added).sum();
    report.removed = files.iter().filter_map(|f| f.removed).sum();
    report.uncommitted = files.iter().filter(|f| f.uncommitted).count();
    files.truncate(MAX_FILES);
    report.files = files;
    report
}

/// The git commands that look at a side and take it into the project, for the compare overlay to show and copy.
/// Never run by NOBLE.
pub fn side_commands(rec: &DuelRecord, side: Side) -> Vec<String> {
    let s = rec.side(side);
    let (project, worktree) = (crate::hooks::quote_for_shell(&rec.project), crate::hooks::quote_for_shell(&s.worktree));
    vec![
        format!("git -C {project} diff {}...{}", rec.base_short(), s.branch),
        format!("git -C {worktree} status -s"),
        format!("git -C {project} merge {}", s.branch),
    ]
}

// ─── Ending ─────────────────────────────────────────────────────────────────────────────────────

/// Which branches to delete along with the worktrees: only what the user picked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeleteBranches {
    #[default]
    None,
    A,
    B,
    Both,
}

impl DeleteBranches {
    pub fn has(self, side: Side) -> bool {
        matches!((self, side), (DeleteBranches::Both, _) | (DeleteBranches::A, Side::A) | (DeleteBranches::B, Side::B))
    }

    pub const ALL: [DeleteBranches; 4] =
        [DeleteBranches::None, DeleteBranches::A, DeleteBranches::B, DeleteBranches::Both];
}

/// How many files each worktree has with uncommitted changes (an untracked file counts); 0 for a folder that is gone.
/// `Err` when git could not tell. Background thread only.
pub fn dirty_counts(rec: &DuelRecord) -> Result<(usize, usize), String> {
    let count = |s: Side| -> Result<usize, String> {
        let path = Path::new(&rec.side(s).worktree);
        if !path.is_dir() {
            return Ok(0);
        }
        git_checked(path, &["status", "--porcelain=v1", "--untracked-files=all"], GIT_QUICK)
            .map(|t| t.lines().filter(|l| !l.trim().is_empty()).count())
            .map_err(|e| clean_git_error(&e))
    };
    Ok((count(Side::A)?, count(Side::B)?))
}

/// What ending a duel did to one side.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SideEnd {
    /// The worktree is gone (removed now, or it was already).
    pub removed: bool,
    /// The branch was deleted (only when asked for).
    pub branch_deleted: bool,
    /// What went wrong.
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EndReport {
    pub id: String,
    pub a: SideEnd,
    pub b: SideEnd,
}

impl EndReport {
    pub fn side(&self, side: Side) -> &SideEnd {
        match side {
            Side::A => &self.a,
            Side::B => &self.b,
        }
    }

    /// Both worktrees are gone: the duel is over.
    pub fn done(&self) -> bool {
        self.a.removed && self.b.removed
    }
}

/// `git worktree remove`. Windows: a pane that was closed a moment ago may still hold its folder, so a refusal that is
/// not about uncommitted changes is tried again for a few seconds. Linux and macOS remove a folder a process still
/// has open at once, so they ask once.
fn remove_worktree(project: &Path, args: &[&str]) -> Result<String, String> {
    let tries = if cfg!(windows) { 6 } else { 1 };
    let mut last = Err(String::new());
    for n in 0..tries {
        last = git_checked(project, args, GIT_QUICK);
        let about_changes =
            last.as_ref().err().is_some_and(|e| e.contains("modified or untracked") || e.contains("--force"));
        if last.is_ok() || about_changes {
            break;
        }
        if n + 1 < tries {
            std::thread::sleep(Duration::from_millis(500));
        }
    }
    last
}

/// Removes both worktrees (`git worktree remove`, with `--force` only when `force`), then the branches the user chose
/// to delete. A worktree that is already gone is only pruned. The branches of the others stay: they hold the work.
/// Background thread only.
pub fn end(rec: &DuelRecord, force: bool, delete: DeleteBranches) -> EndReport {
    let project = Path::new(&rec.project);
    let one = |side: Side| -> SideEnd {
        let s = rec.side(side);
        let path = Path::new(&s.worktree);
        let mut out = SideEnd::default();
        if path.exists() {
            let Some(p) = path.to_str() else {
                out.error = Some("the worktree path is not valid text".into());
                return out;
            };
            let mut args = vec!["worktree", "remove"];
            if force {
                args.push("--force");
            }
            args.push(p);
            match remove_worktree(project, &args) {
                Ok(_) => out.removed = true,
                Err(e) => {
                    out.error = Some(clean_git_error(&e));
                    return out;
                }
            }
        } else {
            // Gone already (deleted by hand): git still lists it until it is pruned.
            let _ = git_checked(project, &["worktree", "prune"], GIT_QUICK);
            out.removed = true;
        }
        if delete.has(side) {
            match git_checked(project, &["branch", "-D", &s.branch], GIT_QUICK) {
                Ok(_) => out.branch_deleted = true,
                Err(e) => out.error = Some(format!("worktree removed, branch kept: {}", clean_git_error(&e))),
            }
        }
        out
    };
    let report = EndReport { id: rec.id.clone(), a: one(Side::A), b: one(Side::B) };
    if report.done() {
        // The folder that held both is empty now; anything else in it stays.
        if let Some(root) = rec.root() {
            let _ = std::fs::remove_dir(root);
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(program: &str) -> ShellSpec {
        ShellSpec::new(program, Vec::new())
    }

    #[test]
    fn ids_are_short_lowercase_and_vary() {
        let id = new_id();
        assert!(valid_id(&id), "{id}");
        assert_eq!(id_from(0), "000000");
        assert_eq!(id_from(35), "z00000");
        assert_eq!(id_from(36), "010000");
        assert!(valid_id(&id_from(u64::MAX)));
        assert!(!valid_id("ABC123") && !valid_id("abc12") && !valid_id("abc1234") && !valid_id("../etc"));
        let many: std::collections::HashSet<String> = (0..50).map(|_| new_id()).collect();
        assert!(many.len() > 45, "ids are random per call");
    }

    #[test]
    fn branches_and_folders_are_named_by_the_id() {
        assert_eq!(branch_name("k3x9qa", Side::A), "noble/duel-k3x9qa-a");
        assert_eq!(branch_name("k3x9qa", Side::B), "noble/duel-k3x9qa-b");
        assert_eq!(tab_name("k3x9qa"), "⚔ duel k3x9qa");
        let data = Path::new("data");
        assert_eq!(
            worktree_path(data, "my app", "k3x9qa", Side::A),
            Path::new("data").join("duels").join("my-app-k3x9qa").join("a")
        );
        assert_eq!(duel_dir(data, "noble", "k3x9qa"), Path::new("data").join("duels").join("noble-k3x9qa"));
    }

    #[test]
    fn project_names_become_safe_folder_names() {
        assert_eq!(folder_part("noble"), "noble");
        assert_eq!(folder_part("My App (v2)"), "My-App-v2");
        assert_eq!(folder_part("../../etc"), "etc");
        assert_eq!(folder_part(".hidden"), "hidden");
        assert_eq!(folder_part("a/b\\c:d"), "a-b-c-d");
        assert_eq!(folder_part("???"), "project");
        assert_eq!(folder_part(""), "project");
        assert_eq!(folder_part(&"x".repeat(80)).len(), 32);
        assert_eq!(folder_part("日本語"), "project", "non-ASCII names do not become folder names");
    }

    #[test]
    fn inside_checks_whole_components() {
        assert!(is_inside(Path::new("/a/b/c"), Path::new("/a/b")));
        assert!(is_inside(Path::new("/a/b"), Path::new("/a/b/")));
        assert!(!is_inside(Path::new("/a/bc"), Path::new("/a/b")));
        assert!(is_inside(Path::new("C:\\Users\\x\\duels\\a"), Path::new("C:/Users/x/duels")));
        assert!(!is_inside(Path::new("/a"), Path::new("")));
    }

    #[test]
    fn macos_private_links_are_the_same_folder() {
        // The helper is plain string work, so every platform checks it (only macOS applies it).
        assert_eq!(without_private("/private/var/folders/x"), "/var/folders/x");
        assert_eq!(without_private("/private/tmp"), "/tmp");
        assert_eq!(without_private("/private/etc/hosts"), "/etc/hosts");
        assert_eq!(without_private("/private/variable"), "/private/variable");
        assert_eq!(without_private("/var/x"), "/var/x");
        assert_eq!(without_private("/Users/me/private/var"), "/Users/me/private/var");
    }

    #[test]
    fn what_git_prints_never_breaks_the_parsers() {
        // Output is outside data: odd bytes, cut quotes, lone backslashes, huge fields, multi-byte text.
        let long = "x".repeat(100_000);
        let samples = [
            "",
            "\0",
            "\"",
            "\"\\",
            "?? \"unterminated",
            "R  a -> ",
            "R  \" -> \"",
            " M",
            "é\u{301}x y",
            "日本語\t日本語\t日本語",
            "1\t2",
            "1\t2\t",
            "99999999999\t1\tpath",
            "\t\t\t",
            "-\t5\tmixed",
            "\u{202e}rtl\u{202c}",
            &long,
        ];
        for s in samples {
            let _ = parse_numstat(s);
            let _ = parse_porcelain(s, 10);
            let _ = unquote(s);
            let both = format!("{s}\n{s}\r\n{s}");
            let _ = parse_numstat(&both);
            let _ = parse_porcelain(&both, 10);
        }
        assert_eq!(parse_porcelain("?? \"a\\", 10)[0].path, "\"a\\", "a cut quote is kept as text");
        assert!(parse_numstat("99999999999\t1\tpath").is_empty(), "a count that does not fit is skipped");
    }

    #[test]
    fn only_agents_with_a_known_prompt_form_are_offered() {
        for (cmd, agent) in [
            ("claude", "claude"),
            ("claude --model opus", "claude"),
            ("codex", "codex"),
            ("copilot --allow-all", "copilot"),
            ("opencode", "opencode"),
            ("/usr/local/bin/claude", "claude"),
        ] {
            assert_eq!(duel_agent(cmd), Some(agent), "{cmd}");
        }
        for cmd in ["agy", "cursor-agent", "pi", "omp", "freebuff", "grok", "cline", "kilo", "command-code", "bash", ""]
        {
            assert_eq!(duel_agent(cmd), None, "{cmd} has no known form");
        }
    }

    #[test]
    fn prompts_are_cleaned_before_use() {
        assert_eq!(clean_prompt("  fix the bug \r\n then test\tit \u{7}"), "fix the bug \n then test it");
        assert_eq!(clean_prompt("\u{1b}[31mred"), "[31mred", "an escape character is dropped");
        assert_eq!(clean_prompt("a\rb"), "a\nb");
        assert_eq!(clean_prompt(&"x".repeat(MAX_PROMPT + 50)).chars().count(), MAX_PROMPT);
        assert_eq!(clean_prompt(" \n "), "");
    }

    #[test]
    fn the_task_is_one_quoted_word_in_every_shell() {
        let task = "fix it; rm -rf / && echo $HOME `id` \"q\" 'it' %PATH% !x";
        // Unix shells: single quotes, nothing expands. fish also escapes the backslash.
        for program in ["bash", "zsh", "sh"] {
            let line = launch_line("claude", task, &sh(program)).unwrap();
            assert_eq!(line, "claude 'fix it; rm -rf / && echo $HOME `id` \"q\" '\\''it'\\'' %PATH% !x'", "{program}");
        }
        let fish = launch_line("claude", "it's a\\b", &sh("fish")).unwrap();
        assert_eq!(fish, "claude 'it\\'s a\\\\b'");
        // PowerShell: a doubled quote; the typographic quotes it also reads as quotes are doubled too.
        let pwsh = launch_line("codex", "it's “fine” ‘x’", &sh("pwsh")).unwrap();
        assert_eq!(pwsh, "codex 'it''s “fine” ‘‘x’’'");
        // cmd.exe: double quotes, and the characters it cannot quote are refused.
        assert_eq!(launch_line("codex", "add a test", &sh("cmd")).unwrap(), "codex \"add a test\"");
        for bad in ["say \"hi\"", "50% done", "wow!", "two\nlines"] {
            let err = launch_line("codex", bad, &sh("cmd")).unwrap_err();
            assert!(err.contains("cmd.exe"), "{bad}: {err}");
        }
        // A task with line breaks is fine in the others.
        assert_eq!(launch_line("claude", "one\ntwo", &sh("bash")).unwrap(), "claude 'one\ntwo'");
        assert_eq!(launch_line("claude", "one\ntwo", &sh("pwsh")).unwrap(), "claude 'one\ntwo'");
    }

    #[test]
    fn each_agent_gets_its_own_form_of_the_task() {
        let bash = sh("bash");
        assert_eq!(launch_line("claude --model opus", "go", &bash).unwrap(), "claude --model opus 'go'");
        assert_eq!(launch_line("codex", "go", &bash).unwrap(), "codex 'go'");
        assert_eq!(launch_line("copilot --allow-all", "go", &bash).unwrap(), "copilot --allow-all -i 'go'");
        assert_eq!(launch_line("opencode", "go", &bash).unwrap(), "opencode --prompt 'go'");
    }

    #[test]
    fn a_task_that_cannot_be_passed_is_refused_with_a_reason() {
        let bash = sh("bash");
        assert!(launch_line("claude", "   ", &bash).unwrap_err().contains("task"));
        assert!(launch_line("claude", "--help", &bash).unwrap_err().contains("option"));
        assert!(launch_line("agy", "go", &bash).is_err(), "no known form: not guessed");
        assert!(launch_line("cursor-agent", "go", &bash).is_err());
        assert!(launch_line("bash", "go", &bash).is_err(), "a shell is no agent");
    }

    #[test]
    fn numstat_is_parsed_with_binary_files_and_quoted_paths() {
        let text = "10\t2\tsrc/a.rs\n-\t-\tlogo.png\n0\t5\t\"we\\\"ird\\tname.txt\"\nnot a line\n3\tx\tbad.rs\n\n7\t0\tdir with space/b.rs\n";
        assert_eq!(
            parse_numstat(text),
            vec![
                ("src/a.rs".to_string(), Some(10), Some(2)),
                ("logo.png".to_string(), None, None),
                ("we\"ird\tname.txt".to_string(), Some(0), Some(5)),
                ("dir with space/b.rs".to_string(), Some(7), Some(0)),
            ]
        );
        assert!(parse_numstat("").is_empty());
    }

    #[test]
    fn porcelain_is_parsed_with_renames_untracked_and_quotes() {
        let text = " M src/a.rs\nA  new.rs\n?? scratch.txt\nR  old.rs -> moved.rs\nMM \"sp\\\"ace.rs\"\n?? dir/with space.txt\nxx\n";
        let entries = parse_porcelain(text, 100);
        let got: Vec<(&str, &str)> = entries.iter().map(|e| (e.code.as_str(), e.path.as_str())).collect();
        assert_eq!(
            got,
            vec![
                (" M", "src/a.rs"),
                ("A ", "new.rs"),
                ("??", "scratch.txt"),
                ("R ", "moved.rs"),
                ("MM", "sp\"ace.rs"),
                ("??", "dir/with space.txt"),
            ]
        );
        assert!(entries[2].untracked() && !entries[0].untracked());
        assert_eq!(parse_porcelain(text, 2).len(), 2, "the limit holds");
        assert!(parse_porcelain("é\u{301}x y", 10).is_empty(), "a line that is no status line is skipped, not a panic");
    }

    #[test]
    fn the_file_list_merges_commits_working_tree_and_new_files() {
        let committed = vec![("a.rs".to_string(), Some(5), Some(1)), ("gone.rs".to_string(), Some(1), Some(0))];
        // Net against the base: a.rs was edited again in the working tree; b.rs only there; gone.rs was put back.
        let net = vec![("a.rs".to_string(), Some(6), Some(1)), ("b.rs".to_string(), Some(2), Some(2))];
        let status = vec![
            StatusEntry { code: " M".into(), path: "a.rs".into() },
            StatusEntry { code: " M".into(), path: "b.rs".into() },
            StatusEntry { code: "??".into(), path: "new.txt".into() },
            StatusEntry { code: "??".into(), path: "blob.bin".into() },
        ];
        let lines = |p: &str| (p == "new.txt").then_some(12);
        let files = merge_files(&net, &committed, &status, &lines);
        let names: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(names, ["a.rs", "b.rs", "blob.bin", "new.txt"], "sorted, and the reverted file is not listed");
        let a = &files[0];
        assert_eq!(
            (a.added, a.removed, a.committed, a.uncommitted, a.untracked),
            (Some(6), Some(1), true, true, false)
        );
        let b = &files[1];
        assert_eq!((b.committed, b.uncommitted), (false, true));
        let blob = &files[2];
        assert_eq!((blob.added, blob.removed, blob.untracked, blob.uncommitted), (None, None, true, true));
        let new = &files[3];
        assert_eq!((new.added, new.removed), (Some(12), Some(0)));
    }

    #[test]
    fn delete_choices_name_their_sides() {
        assert!(!DeleteBranches::None.has(Side::A) && !DeleteBranches::None.has(Side::B));
        assert!(DeleteBranches::A.has(Side::A) && !DeleteBranches::A.has(Side::B));
        assert!(!DeleteBranches::B.has(Side::A) && DeleteBranches::B.has(Side::B));
        assert!(DeleteBranches::Both.has(Side::A) && DeleteBranches::Both.has(Side::B));
        assert_eq!(DeleteBranches::default(), DeleteBranches::None, "nothing is deleted unless asked");
    }

    #[test]
    fn side_commands_inspect_and_merge_without_running_anything() {
        let rec = DuelRecord {
            id: "k3x9qa".into(),
            project: "/home/me/app".into(),
            base: "1a2b3c4d5e6f".into(),
            a: DuelSide {
                branch: "noble/duel-k3x9qa-a".into(),
                worktree: "/data/duels/app-k3x9qa/a".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let cmds = side_commands(&rec, Side::A);
        assert_eq!(cmds.len(), 3);
        assert!(cmds[0].contains("diff 1a2b3c4...noble/duel-k3x9qa-a"), "{}", cmds[0]);
        assert!(cmds[1].contains("status -s") && cmds[1].contains("app-k3x9qa"), "{}", cmds[1]);
        assert!(cmds[2].contains("merge noble/duel-k3x9qa-a"), "{}", cmds[2]);
    }
}
