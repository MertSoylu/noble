//! Agent duel against a real git: the worktrees and branches are created from a repository's HEAD, the two sides are
//! compared (commits, uncommitted and new files), and ending removes the worktrees and keeps the branches unless
//! asked. The tests skip, with a note, only where there is no `git` at all (every CI platform has one).

use std::path::{Path, PathBuf};
use std::process::Command;

use noble::duel::{self, DeleteBranches, Plan, Side};

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!("noble-duel-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Real git is needed; a machine without it (rare) skips these tests.
fn have_git() -> bool {
    let found = noble::util::which("git").is_some();
    if !found {
        eprintln!("skipped: git is not installed");
    }
    found
}

/// Runs git in `dir`, with the user's global configuration kept out (a signing key, hooks, a template).
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A repository on `main` with two committed files.
fn repo(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q"]);
    git(dir, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    git(dir, &["config", "user.email", "duel@example.com"]);
    git(dir, &["config", "user.name", "Duel Test"]);
    // NOBLE's own git runs read the system config (Git for Windows sets `core.autocrlf = true` there), these test
    // commands do not (`GIT_CONFIG_NOSYSTEM`): the repository's own setting makes both check out and commit the same
    // line endings, so a side's diff counts only the lines the test changed.
    git(dir, &["config", "core.autocrlf", "false"]);
    std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(dir.join("b.txt"), "alpha\nbeta\n").unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "first"]);
}

fn plan(project: &Path, data: &Path, id: &str) -> Plan {
    Plan {
        project: project.to_path_buf(),
        name: "my project".into(),
        data: data.to_path_buf(),
        id: id.into(),
        prompt: "add tests".into(),
        a: ("claude".into(), "claude".into()),
        b: ("codex".into(), "codex".into()),
    }
}

fn branches(project: &Path) -> Vec<String> {
    git(project, &["branch", "--list", "--format=%(refname:short)"]).lines().map(str::to_string).collect()
}

#[test]
fn worktrees_are_created_compared_and_removed() {
    if !have_git() {
        return;
    }
    let tmp = TempDir::new("flow");
    let (project, data) = (tmp.0.join("project"), tmp.0.join("data"));
    repo(&project);
    let head = git(&project, &["rev-parse", "HEAD"]);

    let rec = duel::create(&plan(&project, &data, "abc123")).expect("the duel is created");
    assert_eq!(
        (rec.id.as_str(), rec.base.as_str(), rec.base_branch.as_deref()),
        ("abc123", head.as_str(), Some("main"))
    );
    assert_eq!((rec.a.branch.as_str(), rec.b.branch.as_str()), ("noble/duel-abc123-a", "noble/duel-abc123-b"));
    assert_eq!((rec.a.agent.as_str(), rec.b.agent.as_str()), ("claude", "codex"));
    for side in Side::BOTH {
        let s = rec.side(side);
        let wt = Path::new(&s.worktree);
        assert!(wt.join("a.txt").is_file(), "{side:?}: the tree of HEAD is checked out");
        assert_eq!(git(wt, &["rev-parse", "--abbrev-ref", "HEAD"]), s.branch);
        assert_eq!(git(wt, &["rev-parse", "HEAD"]), head);
        assert!(
            wt.starts_with(&data) && !wt.starts_with(&project),
            "{side:?}: under the data folder, never in the repository"
        );
    }
    assert!(
        rec.a.worktree.ends_with(&format!("my-project-abc123{}a", std::path::MAIN_SEPARATOR)),
        "{}",
        rec.a.worktree
    );
    // Only the worktrees are new in the project: its own working tree is untouched.
    assert_eq!(git(&project, &["status", "--porcelain"]), "");

    // Side A commits a change and a new file, edits another file and leaves a new one unsaved; side B does nothing.
    let a = PathBuf::from(&rec.a.worktree);
    std::fs::write(a.join("a.txt"), "one\ntwo\nthree\nfour\n").unwrap();
    std::fs::write(a.join("c.txt"), "x\n").unwrap();
    git(&a, &["add", "-A"]);
    git(&a, &["commit", "-q", "-m", "work"]);
    std::fs::write(a.join("b.txt"), "alpha\n").unwrap();
    std::fs::write(a.join("new.txt"), "1\n2\n3\n").unwrap();

    let report = duel::compare(&rec);
    let (ra, rb) = (&report.a, &report.b);
    assert_eq!((ra.commits, rb.commits), (1, 0));
    assert!(ra.worktree_exists && ra.error.is_none(), "{:?}", ra.error);
    let find =
        |name: &str| ra.files.iter().find(|f| f.path == name).unwrap_or_else(|| panic!("{name} in {:?}", ra.files));
    let a_txt = find("a.txt");
    assert_eq!((a_txt.added, a_txt.removed, a_txt.committed, a_txt.uncommitted), (Some(2), Some(0), true, false));
    let c_txt = find("c.txt");
    assert_eq!((c_txt.added, c_txt.committed), (Some(1), true));
    let b_txt = find("b.txt");
    assert_eq!((b_txt.added, b_txt.removed, b_txt.committed, b_txt.uncommitted), (Some(0), Some(1), false, true));
    let new = find("new.txt");
    assert_eq!((new.added, new.removed, new.untracked, new.uncommitted), (Some(3), Some(0), true, true));
    assert_eq!((ra.file_count, ra.added, ra.removed, ra.uncommitted), (4, 6, 1, 2));
    assert!(rb.files.is_empty() && rb.added == 0 && rb.uncommitted == 0, "{rb:?}");

    // Ending: the uncommitted work stops a plain removal; the clean side goes, the dirty one stays.
    assert_eq!(duel::dirty_counts(&rec), Ok((2, 0)));
    let first = duel::end(&rec, false, DeleteBranches::None);
    assert!(!first.done() && first.a.error.is_some() && !first.a.removed, "{first:?}");
    assert!(first.b.removed && !Path::new(&rec.b.worktree).exists());
    assert!(a.join("new.txt").is_file(), "nothing of the dirty side was touched");
    // Forced, and the user also picked side A's branch: A's worktree and branch go, B's branch stays.
    let second = duel::end(&rec, true, DeleteBranches::A);
    assert!(second.done() && second.a.branch_deleted && second.a.error.is_none(), "{second:?}");
    assert!(!a.exists());
    assert!(!Path::new(&rec.a.worktree).parent().unwrap().exists(), "the duel's own folder is gone");
    let left = branches(&project);
    assert!(left.contains(&"main".to_string()) && left.contains(&rec.b.branch), "{left:?}");
    assert!(!left.contains(&rec.a.branch), "only the branch that was picked is deleted: {left:?}");
    assert_eq!(git(&project, &["worktree", "list", "--porcelain"]).matches("worktree ").count(), 1);
}

#[test]
fn ending_keeps_both_branches_unless_asked() {
    if !have_git() {
        return;
    }
    let tmp = TempDir::new("keep");
    let (project, data) = (tmp.0.join("project"), tmp.0.join("data"));
    repo(&project);
    let rec = duel::create(&plan(&project, &data, "keep01")).unwrap();
    assert_eq!(duel::dirty_counts(&rec), Ok((0, 0)));
    let report = duel::end(&rec, false, DeleteBranches::None);
    assert!(report.done() && !report.a.branch_deleted && !report.b.branch_deleted, "{report:?}");
    let left = branches(&project);
    assert!(left.contains(&rec.a.branch) && left.contains(&rec.b.branch), "the branches hold the work: {left:?}");
}

#[test]
fn compare_reads_a_side_whose_worktree_folder_is_gone() {
    if !have_git() {
        return;
    }
    let tmp = TempDir::new("gone");
    let (project, data) = (tmp.0.join("project"), tmp.0.join("data"));
    repo(&project);
    let rec = duel::create(&plan(&project, &data, "gone01")).unwrap();
    let b = PathBuf::from(&rec.b.worktree);
    std::fs::write(b.join("a.txt"), "changed\n").unwrap();
    git(&b, &["commit", "-q", "-am", "b work"]);
    std::fs::remove_dir_all(&b).unwrap();
    let report = duel::compare(&rec);
    assert!(!report.b.worktree_exists);
    assert_eq!((report.b.commits, report.b.file_count), (1, 1), "the branch is still read: {:?}", report.b);
    // Ending a duel whose folder was deleted by hand prunes the entry instead of failing.
    let ended = duel::end(&rec, false, DeleteBranches::None);
    assert!(ended.done(), "{ended:?}");
    assert_eq!(git(&project, &["worktree", "list", "--porcelain"]).matches("worktree ").count(), 1);
}

#[test]
fn an_existing_branch_is_never_reused_or_deleted() {
    if !have_git() {
        return;
    }
    let tmp = TempDir::new("exists");
    let (project, data) = (tmp.0.join("project"), tmp.0.join("data"));
    repo(&project);
    git(&project, &["branch", "noble/duel-dup001-b"]);
    let err = duel::create(&plan(&project, &data, "dup001")).unwrap_err();
    assert!(err.contains("already exists") && err.contains("noble/duel-dup001-b"), "{err}");
    assert!(branches(&project).contains(&"noble/duel-dup001-b".to_string()), "the user's branch stays");
    assert!(!branches(&project).contains(&"noble/duel-dup001-a".to_string()), "nothing was left behind");
    assert!(!data.join("duels").exists() || std::fs::read_dir(data.join("duels")).unwrap().next().is_none());
}

#[test]
fn things_that_cannot_start_a_duel_say_why() {
    if !have_git() {
        return;
    }
    let tmp = TempDir::new("refuse");
    let data = tmp.0.join("data");
    // Not a repository.
    let plain = tmp.0.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let err = duel::create(&plan(&plain, &data, "plain1")).unwrap_err();
    assert!(err.contains("not a git repository"), "{err}");
    // A repository with no commit yet: nothing to branch from.
    let empty = tmp.0.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    git(&empty, &["init", "-q"]);
    let err = duel::create(&plan(&empty, &data, "empty1")).unwrap_err();
    assert!(err.contains("no commits"), "{err}");
    // A data folder inside the project would put the worktrees in the repository.
    let project = tmp.0.join("project");
    repo(&project);
    let err = duel::create(&plan(&project, &project.join(".noble"), "inside")).unwrap_err();
    assert!(err.contains("inside the project"), "{err}");
    // An id that is not one of ours never reaches a branch name.
    let err = duel::create(&plan(&project, &data, "../evil")).unwrap_err();
    assert!(err.contains("invalid duel id"), "{err}");
    assert_eq!(git(&project, &["status", "--porcelain"]), "");
}
