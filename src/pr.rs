//! Pull request and CI status of a project's current branch, read with the GitHub CLI (`gh pr view`).
//! This is optional context on Home: every failure (no `gh`, no pull request for the branch, not a GitHub
//! repository, not signed in, a timeout) gives `None` without a message.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::Duration;

use serde_json::Value;

use crate::event::{AppEvent, Tx};
use crate::util;

/// A pull request of the current branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrInfo {
    pub number: u32,
    pub title: String,
    pub state: PrState,
    pub url: String,
    pub checks: Checks,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrState {
    Open,
    Draft,
    Merged,
    Closed,
}

/// The CI results of a pull request: check runs and commit statuses, by outcome.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Checks {
    pub passed: u32,
    pub failed: u32,
    pub pending: u32,
}

/// Longest `gh` may take (a slow network, a login prompt that never comes).
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const FIELDS: &str = "number,title,state,isDraft,url,statusCheckRollup";

/// The pull request of the branch checked out in `dir`. `gh` exits with an error (and prints nothing on
/// stdout) when there is none, so an unparsable answer is `None` as well.
pub fn fetch(dir: &Path, gh: &Path) -> Option<PrInfo> {
    let mut cmd = util::command_for(gh);
    cmd.args(["pr", "view", "--json", FIELDS])
        .current_dir(dir)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("NO_COLOR", "1");
    parse(&crate::ai::run_command(cmd, FETCH_TIMEOUT).ok()?)
}

/// Background thread: each request (a project folder) is answered with its branch's pull request, `None`
/// when there is none. `gh` is looked up once; without it, or in an isolated run (`ai::isolated`), no
/// program runs and every answer is `None`.
pub fn spawn(tx: Tx, requests: Receiver<PathBuf>) {
    let _ = std::thread::Builder::new().name("pr".into()).spawn(move || {
        let gh = if crate::ai::isolated() { None } else { util::which("gh") };
        for dir in requests {
            let info = gh.as_deref().and_then(|gh| fetch(&dir, gh));
            if tx.send(AppEvent::Pr(dir, info)).is_err() {
                break;
            }
        }
    });
}

/// Parses the `gh pr view --json` output. `None` for anything that is not a pull request.
pub fn parse(json: &str) -> Option<PrInfo> {
    let v: Value = serde_json::from_str(json).ok()?;
    let number = u32::try_from(v.get("number")?.as_u64()?).ok()?;
    let title = v.get("title").and_then(Value::as_str).unwrap_or_default().to_string();
    let url = v.get("url")?.as_str()?.to_string();
    let draft = v.get("isDraft").and_then(Value::as_bool).unwrap_or(false);
    let state = match v.get("state")?.as_str()? {
        "MERGED" => PrState::Merged,
        "CLOSED" => PrState::Closed,
        "OPEN" if draft => PrState::Draft,
        "OPEN" => PrState::Open,
        _ => return None,
    };
    let checks =
        v.get("statusCheckRollup").and_then(Value::as_array).map(|items| count_checks(items)).unwrap_or_default();
    Some(PrInfo { number, title, state, url, checks })
}

/// The outcome of one entry of `statusCheckRollup`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Passed,
    Failed,
    Pending,
}

/// Counts the rollup's entries: `CheckRun`s by `status` and `conclusion`, `StatusContext`s by `state`.
fn count_checks(items: &[Value]) -> Checks {
    let mut checks = Checks::default();
    for item in items {
        let text = |key: &str| item.get(key).and_then(Value::as_str).unwrap_or("");
        let is_status =
            text("__typename") == "StatusContext" || (item.get("status").is_none() && item.get("state").is_some());
        let outcome = if is_status {
            match text("state") {
                "SUCCESS" => Outcome::Passed,
                "FAILURE" | "ERROR" => Outcome::Failed,
                // PENDING, EXPECTED and anything unknown.
                _ => Outcome::Pending,
            }
        } else if text("status") != "COMPLETED" {
            Outcome::Pending
        } else {
            match text("conclusion") {
                "SUCCESS" | "NEUTRAL" | "SKIPPED" => Outcome::Passed,
                "FAILURE" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED" | "STARTUP_FAILURE" | "STALE" => {
                    Outcome::Failed
                }
                _ => Outcome::Pending,
            }
        };
        let count = match outcome {
            Outcome::Passed => &mut checks.passed,
            Outcome::Failed => &mut checks.failed,
            Outcome::Pending => &mut checks.pending,
        };
        *count = count.saturating_add(1);
    }
    checks
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPEN: &str = r#"{
  "number": 42,
  "title": "Show pull requests and ports on the project card",
  "state": "OPEN",
  "isDraft": false,
  "url": "https://github.com/MertSoylu/noble/pull/42",
  "statusCheckRollup": [
    {"__typename": "CheckRun", "completedAt": "2026-10-14T10:02:11Z", "conclusion": "SUCCESS",
     "detailsUrl": "https://github.com/MertSoylu/noble/actions/runs/1/job/2", "name": "test (ubuntu-latest)",
     "startedAt": "2026-10-14T10:00:01Z", "status": "COMPLETED", "workflowName": "CI"},
    {"__typename": "CheckRun", "completedAt": "0001-01-01T00:00:00Z", "conclusion": "",
     "detailsUrl": "https://github.com/MertSoylu/noble/actions/runs/1/job/3", "name": "clippy",
     "startedAt": "2026-10-14T10:00:01Z", "status": "IN_PROGRESS", "workflowName": "CI"},
    {"__typename": "CheckRun", "completedAt": "2026-10-14T10:03:00Z", "conclusion": "FAILURE",
     "detailsUrl": "", "name": "audit", "startedAt": "2026-10-14T10:00:01Z", "status": "COMPLETED",
     "workflowName": "Audit"},
    {"__typename": "StatusContext", "context": "ci/external", "startedAt": "2026-10-14T10:00:00Z",
     "state": "SUCCESS", "targetUrl": "https://example.com/build/9"},
    {"__typename": "StatusContext", "context": "ci/pending", "startedAt": "2026-10-14T10:00:00Z",
     "state": "PENDING", "targetUrl": ""}
  ]
}"#;

    #[test]
    fn an_open_pull_request_counts_its_checks() {
        let pr = parse(OPEN).expect("parses");
        assert_eq!(pr.number, 42);
        assert_eq!(pr.title, "Show pull requests and ports on the project card");
        assert_eq!(pr.state, PrState::Open);
        assert_eq!(pr.url, "https://github.com/MertSoylu/noble/pull/42");
        assert_eq!(pr.checks, Checks { passed: 2, failed: 1, pending: 2 });
    }

    #[test]
    fn a_draft_is_an_open_pull_request_marked_draft() {
        let json = OPEN.replace(r#""isDraft": false"#, r#""isDraft": true"#);
        assert_eq!(parse(&json).unwrap().state, PrState::Draft);
    }

    #[test]
    fn merged_and_closed_pull_requests() {
        let merged = OPEN.replace(r#""state": "OPEN""#, r#""state": "MERGED""#);
        assert_eq!(parse(&merged).unwrap().state, PrState::Merged);
        let closed = OPEN.replace(r#""state": "OPEN""#, r#""state": "CLOSED""#);
        assert_eq!(parse(&closed).unwrap().state, PrState::Closed);
        // Merged pull requests keep their check results too.
        let merged_all_green = r#"{"number": 7, "title": "x", "state": "MERGED", "isDraft": false,
            "url": "https://github.com/a/b/pull/7", "statusCheckRollup": [
            {"__typename": "CheckRun", "conclusion": "SUCCESS", "status": "COMPLETED"}]}"#;
        let pr = parse(merged_all_green).unwrap();
        assert_eq!(pr.checks, Checks { passed: 1, failed: 0, pending: 0 });
    }

    #[test]
    fn empty_or_missing_rollups_have_no_checks() {
        let empty = r#"{"number": 1, "title": "t", "state": "OPEN", "isDraft": false,
            "url": "https://github.com/a/b/pull/1", "statusCheckRollup": []}"#;
        assert_eq!(parse(empty).unwrap().checks, Checks::default());
        let null = empty.replace("[]", "null");
        assert_eq!(parse(&null).unwrap().checks, Checks::default());
        let missing = r#"{"number": 1, "title": "t", "state": "OPEN", "url": "https://github.com/a/b/pull/1"}"#;
        assert_eq!(parse(missing).unwrap().checks, Checks::default());
    }

    #[test]
    fn every_check_outcome() {
        let one = |item: &str| {
            let json =
                format!(r#"{{"number": 1, "title": "t", "state": "OPEN", "url": "u", "statusCheckRollup": [{item}]}}"#);
            let c = parse(&json).unwrap().checks;
            (c.passed, c.failed, c.pending)
        };
        let run = |status: &str, conclusion: &str| {
            format!(r#"{{"__typename": "CheckRun", "status": "{status}", "conclusion": "{conclusion}"}}"#)
        };
        for conclusion in ["SUCCESS", "NEUTRAL", "SKIPPED"] {
            assert_eq!(one(&run("COMPLETED", conclusion)), (1, 0, 0), "{conclusion}");
        }
        for conclusion in ["FAILURE", "TIMED_OUT", "CANCELLED", "ACTION_REQUIRED", "STARTUP_FAILURE", "STALE"] {
            assert_eq!(one(&run("COMPLETED", conclusion)), (0, 1, 0), "{conclusion}");
        }
        for status in ["QUEUED", "IN_PROGRESS", "PENDING", "WAITING", "REQUESTED"] {
            assert_eq!(one(&run(status, "")), (0, 0, 1), "{status}");
        }
        let status = |state: &str| format!(r#"{{"__typename": "StatusContext", "state": "{state}"}}"#);
        assert_eq!(one(&status("SUCCESS")), (1, 0, 0));
        assert_eq!(one(&status("FAILURE")), (0, 1, 0));
        assert_eq!(one(&status("ERROR")), (0, 1, 0));
        assert_eq!(one(&status("PENDING")), (0, 0, 1));
        assert_eq!(one(&status("EXPECTED")), (0, 0, 1));
    }

    #[test]
    fn garbage_is_no_pull_request() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("no pull requests found for branch \"main\""), None);
        assert_eq!(parse("[1, 2, 3]"), None);
        assert_eq!(parse("{}"), None);
        assert_eq!(parse(r#"{"number": -4, "state": "OPEN", "url": "u"}"#), None);
        assert_eq!(parse(r#"{"number": 4, "state": "WEIRD", "url": "u"}"#), None);
        assert_eq!(parse(r#"{"number": 4, "state": "OPEN"}"#), None, "the url is required");
    }
}
