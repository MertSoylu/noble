//! Notifications: toasts and their history, the update notice, and the pane output that raises them.

use super::*;

impl App {
    pub fn toast(&mut self, level: ToastLevel, text: impl Into<String>) {
        self.toast_target(level, text, None);
    }

    /// A notification whose click does more than dismiss it (see `ToastTarget`).
    pub fn toast_target(&mut self, level: ToastLevel, text: impl Into<String>, target: Option<ToastTarget>) {
        let text = text.into();
        self.toast_serial += 1;
        self.dirty = true;
        // Errors stay until they are clicked away; the others leave after a few seconds.
        let life = match level {
            ToastLevel::Error => None,
            ToastLevel::Warn => Some(Duration::from_secs(4)),
            _ => Some(Duration::from_secs(3)),
        };
        self.notices.push_back(Notice { text: text.clone(), level, at: chrono::Local::now() });
        while self.notices.len() > NOTICE_HISTORY {
            self.notices.pop_front();
        }
        self.toasts.retain(|t| t.text != text);
        self.toasts.push(Toast {
            id: self.toast_serial,
            text,
            level,
            born: Instant::now(),
            life,
            held: Duration::ZERO,
            hover_from: None,
            target,
        });
        // At most four on screen: the oldest one that leaves by itself goes first (a sticky error only
        // when nothing else is left).
        if self.toasts.len() > 4 {
            let oldest = self.toasts.iter().position(|t| t.life.is_some()).unwrap_or(0);
            self.toasts.remove(oldest);
        }
    }

    /// A notification sliding in or out.
    pub(super) fn toast_moving(&self) -> bool {
        self.anim_on()
            && self.toasts.iter().any(|t| {
                t.born.elapsed() < fx::TOAST_IN + fx::GRACE || t.expires_at().is_some_and(|u| Instant::now() >= u)
            })
    }

    /// How long notifications stay on screen after their time is up (they slide out meanwhile).
    pub fn toast_linger(&self) -> Duration {
        if self.anim_on() { fx::TOAST_OUT } else { Duration::ZERO }
    }

    pub(super) fn on_pty_output(&mut self) -> bool {
        let visible: Vec<PaneId> = match self.view {
            View::Term(i) => self.tabs.get(i).map(|t| t.panes()).unwrap_or_default(),
            _ => Vec::new(),
        };
        let mut clip: Option<String> = None;
        let mut signals = Vec::new();
        for (id, pane) in &self.panes {
            if pane.dirty.swap(false, std::sync::atomic::Ordering::AcqRel) {
                // Taken either way (the request is spent); ignored when the setting is off.
                if let Some(c) = pane.take_clipboard().filter(|_| self.cfg.terminal.osc52_write) {
                    clip = Some(c);
                }
                let started = pane.take_started();
                let prompt = pane.take_prompt();
                signals.push(PaneSignal {
                    id: *id,
                    visible: visible.contains(id),
                    bell: pane.take_bell(),
                    notice: pane.take_notice(),
                    started,
                    finished: prompt.and_then(|_| started.or(pane.command_started).map(|t| t.elapsed())),
                    cwd: prompt.map(|_| pane.cwd()),
                    exit: prompt.flatten(),
                    failed: false,
                });
            }
        }
        let mut shown = signals.iter().any(|s| s.visible);
        for mut sig in signals {
            if let Some(p) = self.panes.get_mut(&sig.id) {
                p.last_output = Some(Instant::now());
                // The shell marks command starts: from now on only that mark starts the clock.
                if let Some(t) = sig.started {
                    p.start_marks = true;
                    p.ran = true;
                    p.command_started = Some(t);
                }
            }
            if let Some(cwd) = &sig.cwd {
                // Command finished: the repo may have changed.
                self.refresh_git_at(cwd);
                // An agent there has exited; the tab title in the top bar may change too.
                shown |= self.agent_state(sig.id).is_some();
                self.clear_agent(sig.id);
                if let Some(p) = self.panes.get_mut(&sig.id) {
                    // Only a command typed at a prompt counts (`Pane::command_ran`): the shell's first
                    // prompt (after it starts, or after a launcher command, whose code the shell does
                    // not know) and an Enter on an empty line leave the last result in place.
                    if let Some(took) = sig.finished.filter(|_| p.command_ran()) {
                        p.last_result = Some(CommandResult { code: sig.exit, took, at: Instant::now() });
                        sig.failed = sig.exit.is_some_and(|code| code != 0);
                    }
                    p.command_started = None;
                    p.edited = false;
                    p.ran = false;
                    p.prompted = true;
                }
            }
            self.search_output(sig.id);
            self.notify(&sig);
        }
        if let Some(text) = clip {
            self.set_clipboard(&text, false);
        }
        shown
    }

    /// A folder check came back (`Pane::cwd`). While it was pending the prompt's git refresh ran on the start
    /// folder, so the folder the pane shows now is refreshed here. `true` when the answer was stored.
    pub(super) fn on_pane_cwd(&mut self, id: PaneId, reported: &str, usable: bool) -> bool {
        let Some(pane) = self.panes.get(&id) else { return false };
        if !pane.set_cwd_usable(reported, usable) {
            return false;
        }
        let cwd = pane.cwd();
        if cwd != pane.start_cwd {
            self.refresh_git_at(&cwd);
        }
        true
    }

    /// Turns signals from background tabs into tab markers and notifications.
    pub(super) fn notify(&mut self, sig: &PaneSignal) {
        let Some(ti) = self.tabs.iter().position(|t| t.root.contains(sig.id)) else { return };
        let title = self.tab_title(ti);
        let long = sig
            .finished
            .filter(|d| self.cfg.terminal.notify_after > 0 && d.as_secs() >= self.cfg.terminal.notify_after);
        let took = long.map(crate::util::fmt_duration);
        let message = if let Some(n) = &sig.notice {
            Some(format!("{title} · {n}"))
        } else if let Some(took) = took {
            Some(match sig.exit.filter(|_| sig.failed) {
                Some(code) => format!("{title} · failed (exit {code}) after {took}"),
                None => format!("{title} · done in {took}"),
            })
        } else if sig.bell {
            Some(format!("{title} · needs attention"))
        } else {
            None
        };
        // The marker on the tab strip: the most urgent thing that happened. A short command that
        // fails leaves a marker only (no toast, no bell): failures of quick commands are routine.
        let kind = [
            (sig.notice.is_some() || sig.bell, TabAlert::Notice),
            (sig.failed, TabAlert::Failed),
            (long.is_some(), TabAlert::Done),
        ]
        .into_iter()
        .filter_map(|(on, kind)| on.then_some(kind))
        .max();
        let tab = &mut self.tabs[ti];
        if sig.visible {
            // On the visible tab only the app's own notice is shown.
            if let Some(n) = &sig.notice {
                self.toast(ToastLevel::Info, n.clone());
            }
            return;
        }
        tab.activity = true;
        if let Some(kind) = kind {
            tab.raise(kind);
        }
        let Some(message) = message else { return };
        if self.cfg.terminal.notify {
            self.outer_notice = Some(message.clone());
            self.toast(ToastLevel::Info, message);
            self.outer_bell = true;
        }
    }

    /// Sets up the update check: the previous check's result shows right away,
    /// and if it is older than a day the first `tick` performs a new one.
    pub(super) fn init_updates(&mut self) {
        if !crate::update::check_allowed() {
            return;
        }
        let latest = self.ui_state.data.update_latest.clone();
        self.set_latest_version(&latest);
        let age = chrono::Utc::now().timestamp().saturating_sub(self.ui_state.data.update_checked);
        let wait = (crate::update::CHECK_INTERVAL - age).clamp(0, crate::update::CHECK_INTERVAL);
        self.next_update_check = Some(Instant::now() + Duration::from_secs(wait as u64));
    }

    /// Latest known release: announced when newer than this one and not dismissed.
    pub fn set_latest_version(&mut self, version: &str) {
        let newer = crate::update::is_newer(version, crate::update::current());
        let skipped = self.ui_state.data.update_skipped == version;
        self.update_available = (newer && !skipped).then(|| version.to_string());
    }

    /// New version to show bottom right (none when the setting is off).
    pub fn update_notice(&self) -> Option<&str> {
        self.update_available.as_deref().filter(|_| self.cfg.general.check_updates)
    }

    /// Runs `noble update` in a new tab; the progress shows there.
    pub fn start_update(&mut self) {
        let Ok(exe) = std::env::current_exe() else { return };
        let Some(command) = self.shell.invocation_of(&exe, "update") else {
            self.toast(ToastLevel::Warn, "cmd.exe cannot take this program's path: run `noble update` in a terminal");
            return;
        };
        let cwd = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        self.update_available = None;
        self.new_tab(cwd, Some(&command), Some("NOBLE update".into()));
    }

    /// Dismisses the notice: never shown again for the same version.
    pub fn dismiss_update(&mut self) {
        if let Some(v) = self.update_available.take() {
            self.edit_ui_state(|d| d.update_skipped = v);
        }
    }
}
