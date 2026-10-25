//! Agent status hooks in the app: which agents are installed on this machine and whether their hooks are on.
//! Settings, the "Agent status hooks" popup and the one-time offer all go through here. The agents' files are
//! read at startup and when the popup opens, and after each change; nothing is written unless the user asks.

use std::path::{Path, PathBuf};

use super::{App, Overlay, ToastLevel};
use crate::hooks::Agent;

impl App {
    /// Whether the agent's hooks are on (installed in its settings file).
    pub fn hooks_on(&self, agent: Agent) -> bool {
        self.hooks_on.contains(&agent)
    }

    /// The Settings row is shown when an agent is installed here (or has hooks on).
    pub fn agent_hooks_available(&self) -> bool {
        !self.agents_found.is_empty()
    }

    /// (agents with hooks on, agents installed here): the Settings row's "N of M on".
    pub fn agent_hooks_count(&self) -> (usize, usize) {
        let on = self.agents_found.iter().filter(|a| self.hooks_on(**a)).count();
        (on, self.agents_found.len())
    }

    /// Reads every agent's hook state and whether it is installed here (its command is on PATH, or its hooks are
    /// on). Writes nothing. Called at startup and when the popup opens.
    pub(crate) fn scan_agent_hooks(&mut self) {
        self.hooks_on.clear();
        self.agents_found.clear();
        for agent in Agent::ALL {
            let on = agent.settings_path().is_some_and(|p| agent.installed_state(&p).unwrap_or(false));
            if on {
                self.hooks_on.push(agent);
            }
            if on || agent.on_path() {
                self.agents_found.push(agent);
            }
        }
    }

    fn set_hook_on(&mut self, agent: Agent, on: bool) {
        self.hooks_on.retain(|a| *a != agent);
        if on {
            self.hooks_on.push(agent);
        }
    }

    /// Reads one agent's state back from its settings file after a change. An unreadable file is a toast.
    fn refresh_agent_hook(&mut self, agent: Agent, path: &Path) {
        match agent.installed_state(path) {
            Ok(on) => self.set_hook_on(agent, on),
            Err(e) => self.toast(ToastLevel::Error, e),
        }
    }

    /// The agent's settings file for a change. `None` (with a toast) when it is not available here: isolated
    /// (`NOBLE_NO_SYSTEM_INTEGRATIONS`), no home folder, or a headless app.
    fn agent_settings_file(&mut self, agent: Agent) -> Option<PathBuf> {
        let path = agent.settings_path().filter(|_| self.services.is_some());
        if path.is_none() {
            self.toast(ToastLevel::Warn, format!("{} settings are not available here", agent.name()));
        }
        path
    }

    /// Opens the "Agent status hooks" popup. The agents are read again first, so one installed since startup
    /// gets its row (a headless app keeps what it has).
    pub fn open_agent_hooks(&mut self) {
        if self.services.is_some() {
            self.scan_agent_hooks();
        }
        self.overlay = Some(Overlay::AgentHooks { selected: 0 });
    }

    /// Turns one agent's hooks on or off: on writes them into the agent's settings (only for an agent found here),
    /// off removes NOBLE's entries and nothing else.
    pub fn toggle_agent_hooks(&mut self, agent: Agent) {
        if self.hooks_on(agent) {
            let Some(path) = self.agent_settings_file(agent) else { return };
            match agent.uninstall(&path) {
                Ok(()) => {
                    self.refresh_agent_hook(agent, &path);
                    self.toast(ToastLevel::Ok, format!("{} hooks removed", agent.name()));
                }
                Err(e) => self.toast(ToastLevel::Error, e),
            }
        } else if !self.agents_found.contains(&agent) {
            self.toast(ToastLevel::Warn, format!("{} is not installed on this machine", agent.name()));
        } else if self.agent_settings_file(agent).is_some() {
            self.install_agent_hooks(&[agent]);
        }
    }

    /// Installs the hooks of each agent given (the offer's accept, a popup row). One toast names the agents that got
    /// them, failures are toasts too, and Codex's trust notice follows its install (it runs user hooks only after
    /// `/hooks` trusts them).
    pub(super) fn install_agent_hooks(&mut self, agents: &[Agent]) {
        let base = crate::hooks::command_base();
        let mut done = Vec::new();
        for &agent in agents {
            let Some(path) = agent.settings_path().filter(|_| self.services.is_some()) else { continue };
            match agent.install(&path, &base) {
                Ok(()) => {
                    self.refresh_agent_hook(agent, &path);
                    done.push(agent);
                }
                Err(e) => self.toast(ToastLevel::Error, e),
            }
        }
        let names: Vec<&str> = done.iter().filter(|a| **a != Agent::Codex).map(|a| a.name()).collect();
        if !names.is_empty() {
            self.toast(ToastLevel::Ok, format!("hooks installed for {} · applies to new sessions", names.join(", ")));
        }
        if done.contains(&Agent::Codex) {
            self.announce_hook_install(Agent::Codex);
        }
    }

    /// Agents installed here whose hooks are off and can be written: what the one-time offer installs.
    pub(super) fn missing_hooks(&self) -> Vec<Agent> {
        if self.services.is_none() {
            return Vec::new();
        }
        Agent::ALL
            .into_iter()
            .filter(|a| self.agents_found.contains(a) && !self.hooks_on(*a) && a.settings_path().is_some())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn counts_only_the_agents_installed_here() {
        let mut app = App::headless(Config::default(), (100, 30));
        assert_eq!(app.agent_hooks_count(), (0, 0));
        app.agents_found = vec![Agent::Claude, Agent::Codex];
        app.hooks_on = vec![Agent::Claude, Agent::Copilot];
        assert_eq!(app.agent_hooks_count(), (1, 2), "Copilot is not installed here");
        assert!(app.hooks_on(Agent::Claude) && !app.hooks_on(Agent::Codex));
    }

    #[test]
    fn toggling_an_agent_not_found_here_only_toasts() {
        let mut app = App::headless(Config::default(), (100, 30));
        app.toggle_agent_hooks(Agent::OpenCode);
        assert!(!app.hooks_on(Agent::OpenCode), "nothing is written for an agent that is not here");
        assert!(app.agents_found.is_empty());
    }

    #[test]
    fn headless_install_never_writes_an_agent_file() {
        let mut app = App::headless(Config::default(), (100, 30));
        app.agents_found = Agent::ALL.to_vec();
        app.install_agent_hooks(&Agent::ALL);
        assert!(app.hooks_on.is_empty(), "headless apps have no services and touch no settings");
        assert!(app.missing_hooks().is_empty());
    }
}
