//! Session auto-save and restore: tabs are saved shortly after they change and restored by the first window.

use super::*;

/// How long the tabs must have differed from the saved session before it is written again (debounce).
const SESSION_SAVE_DELAY: Duration = Duration::from_secs(2);
/// How often the tabs are compared with the saved session.
const SESSION_CHECK_EVERY: Duration = Duration::from_secs(1);
/// How long the periodic save waits for another window's session lock. It runs on the UI thread, so the wait is
/// one short try (the lock is held for milliseconds at most); a save that misses it is retried at the next check.
const SESSION_AUTOSAVE_LOCK_WAIT: Duration = Duration::from_millis(10);
/// How long the save on a panic waits for another window's lock. The panic message is out already; past this the
/// save is skipped (the file keeps the tabs saved last), so the exit is not held up for the full wait.
pub(super) const PANIC_SAVE_LOCK_WAIT: Duration = Duration::from_millis(500);

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
        let (opened, failed) = self.open_saved_tabs(&tabs);
        self.restored_tabs = opened;
        // A saved tab that could not start at all (no pane could be spawned) is not dropped: it stays in
        // the session file, and the next start tries it again.
        self.unrestored = failed;
        if !self.unrestored.is_empty() {
            let n = self.unrestored.len();
            self.toast(
                ToastLevel::Warn,
                format!("{n} saved tabs could not be started; they are kept for the next start"),
            );
        }
        self.view = View::Bridge;
        // What actually opened (directories that fell back to home included) replaces the
        // taken-over tabs right away, so a crash keeps them as they are now.
        let tabs = self.session_tabs();
        crate::store::session_save(&file, &self.instance, tabs.clone(), false);
        self.session_saved = tabs;
    }

    /// The tabs this window keeps in the session file: the open ones, then the saved tabs that could not be
    /// started at the last restore (`unrestored`).
    fn session_tabs(&self) -> Vec<crate::store::SavedTab> {
        let mut tabs = self.snapshot("last session").tabs;
        tabs.extend(self.unrestored.iter().cloned());
        tabs
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
        let tabs = self.session_tabs();
        if tabs == self.session_saved {
            self.session_dirty_since = None;
            return;
        }
        let since = *self.session_dirty_since.get_or_insert_with(Instant::now);
        if since.elapsed() >= SESSION_SAVE_DELAY {
            let file = self.paths.data_file(self.session_file());
            // Only a save that happened counts: when another window holds the lock, nothing was written and
            // the tabs stay dirty, so the next check saves them again.
            if crate::store::session_save_waiting(
                &file,
                &self.instance,
                tabs.clone(),
                false,
                SESSION_AUTOSAVE_LOCK_WAIT,
            ) {
                self.session_saved = tabs;
                self.session_dirty_since = None;
            }
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

    /// On exit: end the background commands, then save the session.
    pub fn shutdown(&mut self) {
        self.end_session(None);
    }

    /// On a panic on the main thread: `shutdown`, with a short wait for another window's session lock.
    pub fn shutdown_after_panic(&mut self) {
        self.end_session(Some(PANIC_SAVE_LOCK_WAIT));
    }

    /// `shutdown` and `shutdown_after_panic`. `lock_wait` `None` waits the usual long time for the session lock.
    fn end_session(&mut self, lock_wait: Option<Duration>) {
        // First: background commands still running (git, gh, AI quota) are killed with their process trees, so none
        // outlives NOBLE (`ai::run_command`), and none starts after this (the registry is closed). The session
        // save can wait a long time for another window's lock, and on Windows the console handler gives up after 4 s.
        // Only an app with background services does this: the registry is process-wide and closes for good, so a
        // headless app (tests, screenshots) shutting down would stop every other app in the process from starting one.
        // A headless app starts no background command of its own that outlives a call.
        if self.services.is_some() {
            crate::ai::kill_background_children();
        }
        if self.cfg.terminal.restore_session {
            // Merged with the other windows' tabs; this window's earlier entries are replaced.
            let tabs = self.session_tabs();
            let file = self.paths.data_file(self.session_file());
            match lock_wait {
                // Skipped when the lock is not had in time: a write without it could overwrite another window's tabs
                // saved meanwhile. The file keeps this window's tabs as they were at the last save, as after a crash.
                Some(wait) => {
                    crate::store::session_save_waiting(&file, &self.instance, tabs, true, wait);
                }
                None => crate::store::session_save(&file, &self.instance, tabs, true),
            }
        }
        self.panes.clear();
        // Windows: lets the console-close handler return (the process ends then); on Unix nothing waits for it.
        crate::termination::finished();
    }
}
