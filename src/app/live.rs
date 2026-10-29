//! Live project data on Home: the listening ports of the project folders, the pull request + CI of the
//! selected project and Claude Code's background sessions. Requests go out only while Home is open and the
//! background services run; the answers arrive as `AppEvent::Ports` / `AppEvent::Pr` / `AppEvent::BgSessions`
//! and redraw Home only when they change what it shows.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::{App, View};
use crate::pr::PrInfo;
use crate::projects::Project;

/// How often the listening ports are scanned while Home is open (battery: less often).
const PORTS_EVERY: Duration = Duration::from_secs(10);
const PORTS_EVERY_ON_BATTERY: Duration = Duration::from_secs(30);
/// How old a pull request answer may get before the selected project is asked again.
const PR_MAX_AGE: Duration = Duration::from_secs(60);
const PR_MAX_AGE_ON_BATTERY: Duration = Duration::from_secs(180);
/// How often Claude Code's background sessions are listed while Home is open (battery: less often). Each one
/// starts `claude`, so not more than once a minute; entering Home asks again only once the last ask is that old.
const BG_EVERY: Duration = Duration::from_secs(60);
const BG_EVERY_ON_BATTERY: Duration = Duration::from_secs(180);

/// The pull request answer for one project folder, for the branch it was asked for.
pub struct PrEntry {
    /// The branch the request was made for.
    pub branch: Option<String>,
    /// `None`: no pull request for that branch (or `gh` could not tell).
    pub info: Option<PrInfo>,
    pub fetched: Instant,
}

impl App {
    /// Sends the port scan and the pull request request when they are due. Called from `tick`.
    pub(super) fn request_home_live_data(&mut self) {
        if self.view != View::Bridge {
            return;
        }
        let Some(services) = &self.services else { return };
        let on_battery = self.on_battery();

        // Ports: every project folder, on a timer; entering Home asks at once (`ports_asked` is reset there).
        let every = if on_battery { PORTS_EVERY_ON_BATTERY } else { PORTS_EVERY };
        if !self.ports_in_flight && !self.projects.is_empty() && self.ports_asked.is_none_or(|t| t.elapsed() >= every) {
            let roots: Vec<PathBuf> = self.projects.iter().map(|p| p.path.clone()).collect();
            if services.ports_req.send(roots).is_ok() {
                self.ports_in_flight = true;
                self.ports_asked = Some(Instant::now());
            }
        }

        // Background sessions: only with an installed `claude` launcher (the attach runs `claude`).
        let every = if on_battery { BG_EVERY_ON_BATTERY } else { BG_EVERY };
        if !self.bg_in_flight
            && self.bg_asked.is_none_or(|t| t.elapsed() >= every)
            && self.claude_launcher().is_some()
            && services.bg_req.send(()).is_ok()
        {
            self.bg_in_flight = true;
            self.bg_asked = Some(Instant::now());
        }

        // Pull request: only the selected project, one request at a time.
        if self.pr_in_flight.is_some() {
            return;
        }
        let max_age = if on_battery { PR_MAX_AGE_ON_BATTERY } else { PR_MAX_AGE };
        let wanted = self
            .selected_project()
            .filter(|p| p.repo)
            .and_then(|p| p.branch.clone().map(|branch| (p.path.clone(), branch)));
        let Some((path, branch)) = wanted else { return };
        let stale = match self.prs.get(&path) {
            None => true,
            Some(e) => e.branch.as_deref() != Some(branch.as_str()) || e.fetched.elapsed() >= max_age,
        };
        if stale && services.pr_req.send(path.clone()).is_ok() {
            self.pr_in_flight = Some((path, branch));
        }
    }

    /// The pull request of a project, when the answer is for the project's current branch.
    pub fn project_pr(&self, p: &Project) -> Option<&PrInfo> {
        let entry = self.prs.get(&p.path)?;
        (entry.branch.is_some() && entry.branch == p.branch).then_some(entry.info.as_ref()).flatten()
    }

    /// The listening TCP ports of a project (sorted, possibly none).
    pub fn project_ports(&self, p: &Project) -> &[u16] {
        self.ports.get(&p.path).map_or(&[], Vec::as_slice)
    }

    /// Stores a port scan. `true` when Home shows something different.
    pub(super) fn set_ports(&mut self, list: Vec<(PathBuf, Vec<u16>)>) -> bool {
        self.ports_in_flight = false;
        let next: HashMap<PathBuf, Vec<u16>> = list.into_iter().collect();
        let changed = next != self.ports;
        self.ports = next;
        changed && (self.view == View::Bridge || self.overlay.is_some())
    }

    /// The background sessions Home lists: running ones, and finished ones for a day (`BgSession::shown`).
    pub fn shown_bg_sessions(&self) -> Vec<(usize, &crate::bg::BgSession)> {
        let now = chrono::Utc::now().timestamp();
        self.bg_sessions.iter().enumerate().filter(|(_, s)| s.shown(now)).collect()
    }

    /// Stores the background sessions. `true` when Home shows something different.
    pub fn set_bg_sessions(&mut self, list: Vec<crate::bg::BgSession>) -> bool {
        self.bg_in_flight = false;
        let changed = list != self.bg_sessions;
        self.bg_sessions = list;
        changed && (self.view == View::Bridge || self.overlay.is_some())
    }

    /// Stores a pull request answer for the folder the request was made for. `true` when Home shows
    /// something different.
    ///
    /// `gh` answers for the branch checked out when it ran, which may not be the branch the request was
    /// made for (a checkout during the fetch). The answer is stored under the branch it reports; without
    /// an answer, or without a branch name in it, under the requested one.
    pub(super) fn set_pr(&mut self, path: PathBuf, info: Option<PrInfo>) -> bool {
        let Some((_, asked)) = self.pr_in_flight.take_if(|(p, _)| *p == path) else { return false };
        let branch = match &info {
            Some(pr) if !pr.branch.is_empty() => pr.branch.clone(),
            _ => asked,
        };
        let changed = match self.prs.get(&path) {
            None => true,
            Some(old) => old.branch.as_deref() != Some(branch.as_str()) || old.info != info,
        };
        self.prs.insert(path, PrEntry { branch: Some(branch), info, fetched: Instant::now() });
        changed && (self.view == View::Bridge || self.overlay.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::pr::{Checks, PrState};

    fn pr(branch: &str) -> PrInfo {
        PrInfo {
            number: 42,
            title: "t".into(),
            state: PrState::Open,
            url: "https://example.com/pull/42".into(),
            branch: branch.into(),
            checks: Checks::default(),
        }
    }

    fn project(branch: &str) -> Project {
        Project {
            name: "app".into(),
            path: PathBuf::from("/work/app"),
            branch: Some(branch.into()),
            last_active: None,
            git: None,
            repo: true,
            tasks: Vec::new(),
            claude: None,
        }
    }

    /// The request was made for feature-a, but gh ran after the checkout had moved on to feature-b.
    #[test]
    fn an_answer_is_shown_only_on_the_branch_gh_reports() {
        let mut app = App::headless(Config::default(), (110, 30));
        let path = PathBuf::from("/work/app");
        app.pr_in_flight = Some((path.clone(), "feature-a".into()));
        app.set_pr(path.clone(), Some(pr("feature-b")));
        assert!(app.project_pr(&project("feature-a")).is_none(), "feature-b's pull request on feature-a");
        assert!(app.project_pr(&project("feature-b")).is_some(), "shown once the project is on feature-b");
        assert_eq!(app.prs[&path].branch.as_deref(), Some("feature-b"));
    }

    #[test]
    fn a_missing_answer_is_stored_for_the_requested_branch() {
        let mut app = App::headless(Config::default(), (110, 30));
        let path = PathBuf::from("/work/app");
        app.pr_in_flight = Some((path.clone(), "feature-a".into()));
        app.set_pr(path.clone(), None);
        assert_eq!(app.prs[&path].branch.as_deref(), Some("feature-a"));
        assert!(app.project_pr(&project("feature-a")).is_none());
    }

    /// An answer without a branch name is taken for the requested branch (never for an empty one).
    #[test]
    fn an_answer_without_a_branch_name_keeps_the_requested_branch() {
        let mut app = App::headless(Config::default(), (110, 30));
        let path = PathBuf::from("/work/app");
        app.pr_in_flight = Some((path.clone(), "feature-a".into()));
        app.set_pr(path.clone(), Some(pr("")));
        assert!(app.project_pr(&project("feature-a")).is_some());
    }
}
