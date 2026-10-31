//! Live config reload: the config file, the terminal color schemes and the settings applied to the running app.

use super::*;

impl App {
    /// Re-reads the Windows Terminal settings (untouched in headless mode).
    pub fn reload_schemes(&mut self) {
        if self.services.is_some() {
            self.term_schemes = crate::wt::all_schemes(crate::wt::load().as_ref());
        }
    }

    pub fn reload_config(&mut self, announce: bool) {
        self.reload_schemes();
        let loaded = config::load(&self.paths.config);
        self.cfg_mtime = loaded.mtime;
        if let Some(err) = loaded.error {
            // The unknown-key warning of the file that loaded before is out of date too; errors stay.
            self.toasts
                .retain(|t| !(t.level == ToastLevel::Warn && matches!(t.target, Some(ToastTarget::Config { .. }))));
            self.toast_target(ToastLevel::Error, err.text, Some(ToastTarget::Config { line: err.line }));
            return;
        }
        // The file loads now: the config toasts (an old error, the unknown keys) are out of date. Errors never
        // expire by themselves, so they would stay otherwise.
        self.toasts.retain(|t| !matches!(t.target, Some(ToastTarget::Config { .. })));
        self.apply_config(loaded.config);
        // An open welcome card or theme/scheme selector previews on top of the old values: it starts
        // over from the file, so esc cannot put the stale values back.
        self.refresh_welcome();
        match self.overlay {
            Some(Overlay::Themes(_)) => self.open_theme_picker(),
            Some(Overlay::Schemes(_)) => self.open_scheme_picker(),
            _ => {}
        }
        if announce || self.keymap.warnings.is_empty() {
            self.toast(ToastLevel::Ok, "config reloaded");
        }
        for w in self.keymap.warnings.clone() {
            self.toast(ToastLevel::Warn, w);
        }
        if let Some((text, line)) = config::unknown_keys_warning(&loaded.unknown) {
            self.toast_target(ToastLevel::Warn, text, Some(ToastTarget::Config { line }));
        }
    }

    pub fn apply_config(&mut self, cfg: Config) {
        let old = std::mem::replace(&mut self.cfg, cfg);
        // The running flag follows the file only when the file's own value changed (prefix f may have set
        // it since then).
        if old.terminal.focus_mode != self.cfg.terminal.focus_mode {
            self.focus_mode = self.cfg.terminal.focus_mode;
        }
        self.theme = Theme::by_name(&self.cfg.general.theme, self.cfg.general.transparent);
        self.keymap = Keymap::from_config(&self.cfg.keys);
        self.shell = resolve_shell(&self.cfg.terminal, &self.paths.data);
        self.launchers = launcher_availability(&self.cfg.launchers);
        if !self.cfg.general.operator.trim().is_empty() {
            self.operator = self.cfg.general.operator.trim().to_string();
        }
        if let Some(s) = &self.services {
            if old.projects != self.cfg.projects {
                if let Ok(mut c) = s.proj_cfg.lock() {
                    *c = self.cfg.projects.clone();
                }
                let _ = s.rescan.send(crate::projects::ProjectReq::Rescan);
            }
            if old.ai != self.cfg.ai {
                if let Ok(mut c) = s.ai_cfg.lock() {
                    *c = self.cfg.ai.clone();
                }
                if !self.cfg.ai.enabled {
                    self.ai.clear();
                } else {
                    self.ai.retain(|p| self.cfg.ai.providers.iter().any(|x| x.eq_ignore_ascii_case(p.id)));
                }
                let _ = s.ai_refresh.send(crate::ai::AiReq::Refresh);
            }
        }
    }
}
