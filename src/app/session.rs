//! Session auto-save and restore: tabs are saved shortly after they change and restored by the first window.

use super::*;

/// How long the tabs must have differed from the saved session before it is written again (debounce).
const SESSION_SAVE_DELAY: Duration = Duration::from_secs(2);
/// How often the tabs are compared with the saved session.
const SESSION_CHECK_EVERY: Duration = Duration::from_secs(1);
/// How long the periodic save waits for another window's session lock (the main thread must not stall).
const SESSION_AUTOSAVE_LOCK_WAIT: Duration = Duration::from_secs(1);

impl App {
    /// Startup: registers this window in the session file (`session.json`, `session-dev.json`
    /// for `noble-dev`) and, when it is the first window of the run, reopens the saved tabs of
    /// every window. Only with `restore_session` on.
    pub fn restore_last_session(&mut self) {
        if !self.cfg.terminal.restore_session {
            return;
        }
        let file = self.paths.data_file(self.session_file());
        let Some(tabs) = crate::store::session_begin(&file, &self.instance) else { return };
        if tabs.is_empty() {
            return;
        }
        let ws = crate::store::Workspace { name: "last session".into(), saved_at: 0, tabs };
        self.restored_tabs = self.open_workspace(&ws);
        self.view = View::Bridge;
        // What actually opened (directories that fell back to home included) replaces the
        // taken-over tabs right away, so a crash keeps them as they are now.
        let tabs = self.snapshot("last session").tabs;
        crate::store::session_save(&file, &self.instance, tabs.clone(), false);
        self.session_saved = tabs;
    }

    /// Keeps the session file current while NOBLE runs, so a crash, a kill or a power loss loses
    /// at most a few seconds of tabs, splits and directories. The tabs are compared with what was
    /// last saved at most once a second (`tick` runs on every event and at least every 2 s, so an
    /// idle NOBLE gains no wake-ups); a difference that lasts `SESSION_SAVE_DELAY` is written.
    pub(super) fn autosave_session(&mut self) {
        if !self.session_autosave || !self.cfg.terminal.restore_session {
            return;
        }
        if self.session_checked.elapsed() < SESSION_CHECK_EVERY {
            return;
        }
        self.session_checked = Instant::now();
        let tabs = self.snapshot("last session").tabs;
        if tabs == self.session_saved {
            self.session_dirty_since = None;
            return;
        }
        let since = *self.session_dirty_since.get_or_insert_with(Instant::now);
        if since.elapsed() >= SESSION_SAVE_DELAY {
            let file = self.paths.data_file(self.session_file());
            crate::store::session_save_waiting(&file, &self.instance, tabs.clone(), false, SESSION_AUTOSAVE_LOCK_WAIT);
            self.session_saved = tabs;
            self.session_dirty_since = None;
        }
    }

    /// When the main loop must wake up for the session save: only while unsaved changes wait.
    pub fn session_save_due(&self) -> Option<Duration> {
        let since = self.session_dirty_since?;
        let due = (since + SESSION_SAVE_DELAY).max(self.session_checked + SESSION_CHECK_EVERY);
        Some(due.saturating_duration_since(Instant::now()))
    }

    /// Session file: separate so `noble-dev` does not overwrite the stable build's tabs.
    fn session_file(&self) -> &'static str {
        if self.dev { "session-dev.json" } else { "session.json" }
    }

    /// On exit: save the session.
    pub fn shutdown(&mut self) {
        if self.cfg.terminal.restore_session {
            // Merged with the other windows' tabs; this window's earlier entries are replaced.
            let tabs = self.snapshot("last session").tabs;
            crate::store::session_save(&self.paths.data_file(self.session_file()), &self.instance, tabs, true);
        }
        self.panes.clear();
        // Background commands still running (git, gh, AI quota) are killed with their process trees, so none
        // outlives NOBLE (`ai::run_command`).
        crate::ai::kill_background_children();
        // Windows: lets the console-close handler return (the process ends then); on Unix nothing waits for it.
        crate::termination::finished();
    }
}
