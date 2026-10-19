//! Git status requests: the repository of a project is asked again when a command in it finished or
//! when Home shows its projects; requests closer together than the caller's gap are dropped.

use super::*;

/// The project whose git status is asked for when a pane's folder changes. None for a network folder
/// (`\\server\share`, `//server/share`): git run there connects to that server, and a pane that was opened in a
/// share keeps it as its folder (`Pane::cwd`). Otherwise the project containing `cwd`.
pub(super) fn git_project_path(projects: &[crate::projects::Project], cwd: &std::path::Path) -> Option<PathBuf> {
    if crate::term::pane::is_network_path(&cwd.to_string_lossy()) {
        return None;
    }
    crate::projects::project_containing(projects, cwd).map(|p| p.path.clone())
}

impl App {
    /// Re-requests the git status of the project containing `cwd` (unless too frequent).
    pub fn refresh_git_at(&mut self, cwd: &std::path::Path) {
        if let Some(path) = git_project_path(&self.projects, cwd) {
            self.request_git(path, Duration::from_secs(2));
        }
    }

    pub(super) fn request_git(&mut self, path: PathBuf, min_gap: Duration) {
        let Some(s) = &self.services else { return };
        if self.git_requested.get(&path).is_some_and(|t| t.elapsed() < min_gap) {
            return;
        }
        if s.rescan.send(ProjectReq::Refresh(path.clone())).is_ok() {
            self.git_requested.insert(path, Instant::now());
        }
    }

    /// Requests git status for projects shown on Home that do not have one yet
    /// (the first scan only fetches the most recently used ones).
    pub(super) fn request_visible_git(&mut self) {
        if self.view != View::Bridge || self.services.is_none() {
            return;
        }
        let vis = self.visible_projects();
        let from = self.bridge.proj_sel.saturating_sub(20);
        let wanted: Vec<PathBuf> = vis
            .iter()
            .skip(from)
            .take(60)
            .map(|i| &self.projects[*i])
            .filter(|p| p.repo && p.git.is_none() && !self.git_requested.contains_key(&p.path))
            .take(8)
            .map(|p| p.path.clone())
            .collect();
        for path in wanted {
            // Do not keep re-requesting failed repos (no git etc.).
            self.request_git(path, Duration::MAX);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pane in a network folder asks for no git status, even when a project lists that folder; a local folder
    /// inside a project asks for that project.
    #[test]
    fn network_folders_never_ask_for_git() {
        let project = |path: &str| crate::projects::Project {
            name: "p".into(),
            path: PathBuf::from(path),
            branch: None,
            last_active: None,
            git: None,
            repo: true,
            tasks: Vec::new(),
        };
        let projects = [project(r"\\attacker\share\repo"), project("/home/me/repo")];
        assert_eq!(git_project_path(&projects, Path::new("/home/me/repo/src")), Some(PathBuf::from("/home/me/repo")));
        for share in [r"\\attacker\share\repo", r"\\attacker\share\repo\src", "//attacker/share/repo"] {
            assert_eq!(git_project_path(&projects, Path::new(share)), None, "{share}");
        }
    }
}
