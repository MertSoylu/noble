//! Live project data on Home: the listening ports of the project folders and the pull request + CI of the
//! selected project. Requests go out only while Home is open and the background services run; the answers
//! arrive as `AppEvent::Ports` / `AppEvent::Pr` and redraw Home only when they change what it shows.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::{App, View};
use crate::pr::PrInfo;
use crate::projects::Project;

/// How often the listening ports are scanned while Home is open (battery: less often).
const PORTS_EVERY: Duration = Duration::from_secs(5);
const PORTS_EVERY_ON_BATTERY: Duration = Duration::from_secs(15);
/// How old a pull request answer may get before the selected project is asked again.
const PR_MAX_AGE: Duration = Duration::from_secs(60);
const PR_MAX_AGE_ON_BATTERY: Duration = Duration::from_secs(180);

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

    /// Stores a pull request answer for the folder the request was made for. `true` when Home shows
    /// something different.
    pub(super) fn set_pr(&mut self, path: PathBuf, info: Option<PrInfo>) -> bool {
        let Some((_, branch)) = self.pr_in_flight.take_if(|(p, _)| *p == path) else { return false };
        let changed = match self.prs.get(&path) {
            None => true,
            Some(old) => old.branch.as_deref() != Some(branch.as_str()) || old.info != info,
        };
        self.prs.insert(path, PrEntry { branch: Some(branch), info, fetched: Instant::now() });
        changed && (self.view == View::Bridge || self.overlay.is_some())
    }
}
