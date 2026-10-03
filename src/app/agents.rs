//! Agent tracking: the state of the AI agents running in panes (from Claude's hook records or guessed from
//! the launcher command / window title), their badges and the session lists shown on Home and in the tab strip.

use std::collections::HashMap;

use super::*;

/// A Claude session's state from its last hook event. While subagents still run the
/// session is working even after the main answer ended (`stop`); a question from Claude
/// or one of its subagents (a permission prompt) still needs you.
pub(super) fn hook_state(rec: &crate::hooks::HookRecord) -> Option<AgentState> {
    let state = match rec.event.as_str() {
        "prompt" => AgentState::Working,
        "notification" => AgentState::NeedsYou,
        "stop" | "session-start" => AgentState::Idle,
        _ => return None,
    };
    Some(if state == AgentState::Idle && rec.subagents > 0 { AgentState::Working } else { state })
}

impl App {
    /// The state and running subagents of every pane with a hook record (in pane order), to see
    /// whether they changed.
    fn hook_states(&self) -> Vec<(PaneId, Option<AgentState>, usize)> {
        let mut states: Vec<_> = self.agent_hooks.iter().map(|(p, rec)| (*p, hook_state(rec), rec.subagents)).collect();
        states.sort_by_key(|(p, ..)| *p);
        states
    }

    /// Processes the hook records: notifies when a changed state is in a background tab.
    pub fn apply_hook_records(&mut self, records: HashMap<PaneId, crate::hooks::HookRecord>) {
        let visible: Vec<PaneId> = match self.view {
            View::Term(i) => self.tabs.get(i).map(|t| t.panes()).unwrap_or_default(),
            _ => Vec::new(),
        };
        let mut changed = Vec::new();
        let mut live = HashMap::new();
        for (pane, rec) in records {
            // Orphans (pane closed or never ours), finished sessions, and records from
            // before the pane's shell prompt came back (the agent has exited since).
            let stale = !self.panes.contains_key(&pane)
                || rec.event == crate::hooks::SESSION_END
                || self.agent_cleared.get(&pane).is_some_and(|&t| rec.ts <= t);
            if stale {
                crate::hooks::remove_record(&self.paths.data, std::process::id(), pane);
                continue;
            }
            // A new event notifies, and so does the state changing without one: the last
            // background subagent ending after `stop` turns Working into Idle with the same
            // event. A count change that keeps the state (2 subagents to 1) does not.
            let new_event = self.agent_hooks.get(&pane).is_none_or(|old| {
                (&old.event, &old.message, old.ts) != (&rec.event, &rec.message, rec.ts)
                    || hook_state(old) != hook_state(&rec)
            });
            if new_event {
                changed.push((pane, rec.clone()));
            }
            live.insert(pane, rec);
        }
        // A session that stays Working keeps the time it started working; a new stretch starts
        // at the record that turned it Working.
        let since: HashMap<PaneId, i64> = live
            .iter()
            .filter(|(_, rec)| hook_state(rec) == Some(AgentState::Working))
            .map(|(pane, rec)| {
                let was_working = self.agent_hooks.get(pane).and_then(hook_state) == Some(AgentState::Working);
                let kept = self.agent_working_since.get(pane).copied().filter(|_| was_working);
                (*pane, kept.unwrap_or(rec.ts))
            })
            .collect();
        self.agent_working_since = since;
        let before = self.hook_states();
        self.agent_hooks = live;
        // The states show on Home, in the pane titles and as the tab strip's dots (the top bar is on
        // every screen), and those only redraw by the clock (once a minute on battery): a changed state
        // asks for its own frame. A changed subagent count ("+2") only shows in a visible pane's title.
        let after = self.hook_states();
        let state_of = |v: &[(PaneId, Option<AgentState>, usize)]| -> Vec<(PaneId, Option<AgentState>)> {
            v.iter().map(|(p, s, _)| (*p, *s)).collect()
        };
        let shown = |v: &[(PaneId, Option<AgentState>, usize)]| -> Vec<(PaneId, usize)> {
            v.iter().filter(|(p, ..)| visible.contains(p)).map(|(p, _, n)| (*p, *n)).collect()
        };
        if state_of(&after) != state_of(&before) || shown(&after) != shown(&before) {
            self.dirty = true;
        }
        for (pane, rec) in changed {
            let notice = match hook_state(&rec) {
                Some(AgentState::NeedsYou) => {
                    Some(rec.message.clone().unwrap_or_else(|| "Claude needs your attention".into()))
                }
                Some(AgentState::Idle) if rec.event == "stop" => Some("Claude finished".into()),
                _ => None,
            };
            let Some(notice) = notice else { continue };
            let sig = PaneSignal {
                id: pane,
                visible: visible.contains(&pane),
                bell: false,
                notice: None,
                started: None,
                finished: None,
                cwd: None,
                exit: None,
                failed: false,
            };
            if sig.visible {
                continue;
            }
            let signal = PaneSignal { notice: Some(notice), ..sig };
            self.notify(&signal);
        }
    }

    /// The pane's shell prompt came back, so whatever agent ran there has exited:
    /// its hook record goes, and records written until now are ignored if they show up
    /// later (a hook finishing its write just as the agent exits).
    ///
    /// Only the shell emits the prompt marks (OSC 7 / 9;9 / 133); Claude Code sets
    /// the title and OSC 9;4 progress but none of these, so a running agent is not
    /// cleared by it. Should one ever emit them, its next hook event (every prompt,
    /// notification and stop) writes a newer record and the state comes back.
    pub(super) fn clear_agent(&mut self, pane: PaneId) {
        self.agent_cleared.insert(pane, chrono::Utc::now().timestamp());
        self.agent_working_since.remove(&pane);
        if self.agent_hooks.remove(&pane).is_some() {
            crate::hooks::remove_record(&self.paths.data, std::process::id(), pane);
        }
    }

    /// Forgets a closed pane's agent state and deletes its hook record.
    pub(crate) fn forget_agent(&mut self, pane: PaneId) {
        self.agent_cleared.remove(&pane);
        self.agent_working_since.remove(&pane);
        if self.agent_hooks.remove(&pane).is_some() {
            crate::hooks::remove_record(&self.paths.data, std::process::id(), pane);
        }
    }

    /// The AI agent running in a pane and its state: from Claude's hook record, else
    /// guessed from the launcher command (while it still runs) or the window title.
    pub fn agent_state(&self, pane: PaneId) -> Option<(&'static str, AgentState)> {
        if let Some(rec) = self.agent_hooks.get(&pane) {
            return hook_state(rec).map(|state| ("claude", state));
        }
        let p = self.panes.get(&pane)?;
        // Once the launcher command has exited, the pane runs whatever was typed at the prompt.
        let command = p.command.as_deref().filter(|_| p.launch_running());
        let kind = crate::ai::agent_kind(command, &p.label())?;
        // Only a notice (bell, notification) means the agent asks for the user; a finished or failed command does not.
        let alert = self.tabs.iter().any(|t| t.alert == Some(TabAlert::Notice) && t.root.contains(pane));
        Some((kind, if alert { AgentState::NeedsYou } else { AgentState::Running }))
    }

    /// The agent part of a pane's title: its state, how long it has been working and its subagents.
    pub fn agent_badge(&self, pane: PaneId) -> Option<AgentBadge> {
        let (kind, state) = self.agent_state(pane)?;
        let rec = self.agent_hooks.get(&pane);
        let working_since = (state == AgentState::Working)
            .then(|| self.agent_working_since.get(&pane).copied().or(rec.map(|r| r.ts)))
            .flatten();
        Some(AgentBadge { kind, state, working_since, subagents: rec.map_or(0, |r| r.subagents) })
    }

    /// The most urgent agent state among a tab's panes (the dot in the tab strip).
    pub fn tab_agent_state(&self, tab: usize) -> Option<AgentState> {
        let tab = self.tabs.get(tab)?;
        tab.panes().into_iter().filter_map(|p| self.agent_state(p).map(|(_, s)| s)).max_by_key(AgentState::urgency)
    }

    /// Panes drawn in the visible terminal tab (only the focused one while zoomed).
    pub fn visible_panes(&self) -> Vec<PaneId> {
        match self.view {
            View::Term(i) => self.tabs.get(i).map(|t| if t.zoomed { vec![t.focus] } else { t.panes() }),
            _ => None,
        }
        .unwrap_or_default()
    }

    /// Working agents spin (`hud::agent_spinner`) only while one in a visible pane is live
    /// (`agent_live`) and not on battery; otherwise they show a static "…" and ask for no extra frames.
    pub fn agent_spinning(&self) -> bool {
        !self.on_battery() && self.visible_panes().into_iter().any(|p| self.agent_live(p))
    }

    /// The pane's agent is working and shows it: its pane printed something within `AGENT_LIVE`
    /// (an agent's own screen animates while it works). A session stays Working after an interrupt
    /// (Esc sends no hook event) or with a leftover subagent marker; such a stale state must not
    /// keep the screen redrawing.
    pub fn agent_live(&self, pane: PaneId) -> bool {
        matches!(self.agent_state(pane), Some((_, AgentState::Working)))
            && self.panes.get(&pane).and_then(|p| p.last_output).is_some_and(|t| t.elapsed() < AGENT_LIVE)
    }

    /// AI sessions across all tabs (in tab order).
    pub fn all_agent_sessions(&self) -> Vec<AgentSession> {
        let mut out = Vec::new();
        for (ti, tab) in self.tabs.iter().enumerate() {
            for pane in tab.panes() {
                let Some((kind, state)) = self.agent_state(pane) else { continue };
                let cwd = self.panes.get(&pane).map(|p| p.cwd()).unwrap_or_default();
                let place = crate::projects::project_containing(&self.projects, &cwd)
                    .map(|p| p.name.clone())
                    .or_else(|| cwd.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_default();
                out.push(AgentSession { tab: ti, pane, kind, place, state });
            }
        }
        out
    }

    /// Open AI sessions in the project: (agent, state). If the same agent is in
    /// several panes, the state needing most attention is shown.
    pub fn agent_sessions(&self, project: &std::path::Path) -> Vec<(&'static str, AgentState)> {
        self.agent_sessions_by_project(&self.all_agent_sessions()).remove(project).unwrap_or_default()
    }

    /// `agent_sessions` for every project at once, from an already collected session list: Home builds this
    /// once per frame instead of scanning the panes for every project row.
    pub fn agent_sessions_by_project(&self, sessions: &[AgentSession]) -> AgentsByProject {
        let mut out: AgentsByProject = std::collections::HashMap::new();
        for s in sessions {
            let Some(p) = self.panes.get(&s.pane) else { continue };
            let Some(project) = crate::projects::project_containing(&self.projects, &p.cwd()) else { continue };
            let list = out.entry(project.path.clone()).or_default();
            match list.iter_mut().find(|(k, _)| *k == s.kind) {
                Some(entry) if s.state.urgency() > entry.1.urgency() => entry.1 = s.state,
                Some(_) => {}
                None => list.push((s.kind, s.state)),
            }
        }
        out
    }
}

/// Whether the one-time hooks offer should show for a launch: Claude itself is starting, the hooks are not
/// installed, `~/.claude/settings.json` is usable (not isolated, real app) and the offer was never made.
pub(super) fn should_offer_hooks(is_claude: bool, installed: bool, available: bool, offered: bool) -> bool {
    is_claude && !installed && available && !offered
}

impl App {
    /// After a quick launch: the first time Claude is started while its status hooks are off, offers to enable
    /// them (a clickable notice, see `Hit::HooksOffer`). Recorded in `state.json`, so it is offered only once.
    pub(super) fn note_agent_offer(&mut self, launch_command: &str) {
        let is_claude = crate::ai::agent_kind(Some(launch_command), "") == Some("claude");
        // Same guard as `toggle_claude_hooks`: never in tests or with `NOBLE_NO_SYSTEM_INTEGRATIONS`.
        let available = self.services.is_some() && crate::hooks::settings_path().is_some();
        if !should_offer_hooks(is_claude, self.hooks_installed, available, self.ui_state.data.hooks_offered) {
            return;
        }
        self.edit_ui_state(|d| d.hooks_offered = true);
        self.hooks_offer = true;
        self.dirty = true;
    }

    /// The offer is showing (and still makes sense).
    pub fn hooks_offer_notice(&self) -> bool {
        self.hooks_offer && !self.hooks_installed
    }

    /// Click on the offer: the same code path as the Settings toggle.
    pub(super) fn accept_hooks_offer(&mut self) {
        self.hooks_offer = false;
        if !self.hooks_installed {
            self.toggle_claude_hooks();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::HookRecord;

    fn rec(event: &str, ts: i64, subagents: usize) -> HookRecord {
        HookRecord { event: event.into(), message: None, ts, subagents }
    }

    /// A headless app with one shell pane and its own data folder (the records are deleted from there;
    /// the folder goes when the app is dropped).
    fn app_with_pane() -> (App, PaneId) {
        let mut app = App::headless(Config::default(), (100, 30));
        let _ = std::fs::create_dir_all(&app.paths.data);
        app.open_project_shell(std::env::temp_dir());
        let pane = app.tabs.last().expect("tab").focus;
        (app, pane)
    }

    fn apply(app: &mut App, pane: PaneId, r: HookRecord) {
        app.apply_hook_records(HashMap::from([(pane, r)]));
    }

    fn state(app: &App, pane: PaneId) -> Option<AgentState> {
        app.agent_hooks.get(&pane).and_then(hook_state)
    }

    #[test]
    fn hooks_offer_only_once_for_claude_without_hooks() {
        assert!(should_offer_hooks(true, false, true, false));
        assert!(!should_offer_hooks(false, false, true, false), "other agents do not need Claude hooks");
        assert!(!should_offer_hooks(true, true, true, false), "already installed");
        assert!(!should_offer_hooks(true, false, false, false), "isolated or headless");
        assert!(!should_offer_hooks(true, false, true, true), "offered before");
    }

    #[test]
    fn headless_launch_never_offers() {
        let mut app = App::headless(Config::default(), (100, 30));
        app.note_agent_offer("claude");
        assert!(!app.hooks_offer && !app.ui_state.data.hooks_offered);
    }

    #[test]
    fn hook_state_maps_events_and_subagents() {
        assert_eq!(hook_state(&rec("prompt", 1, 0)), Some(AgentState::Working));
        assert_eq!(hook_state(&rec("notification", 1, 0)), Some(AgentState::NeedsYou));
        assert_eq!(hook_state(&rec("stop", 1, 0)), Some(AgentState::Idle));
        assert_eq!(hook_state(&rec("session-start", 1, 0)), Some(AgentState::Idle));
        // Subagents still running keep the session working after the main answer; a question still needs you.
        assert_eq!(hook_state(&rec("stop", 1, 2)), Some(AgentState::Working));
        assert_eq!(hook_state(&rec("notification", 1, 2)), Some(AgentState::NeedsYou));
        assert_eq!(hook_state(&rec("something-else", 1, 0)), None);
    }

    #[test]
    fn records_drive_working_needs_you_and_your_turn() {
        let (mut app, pane) = app_with_pane();
        apply(&mut app, pane, rec("prompt", 100, 0));
        assert_eq!(state(&app, pane), Some(AgentState::Working));
        assert_eq!(app.agent_working_since.get(&pane), Some(&100));
        // Still working with a newer record: the start of the stretch is kept.
        apply(&mut app, pane, rec("prompt", 110, 1));
        assert_eq!(app.agent_working_since.get(&pane), Some(&100));
        apply(&mut app, pane, rec("notification", 120, 0));
        assert_eq!(state(&app, pane), Some(AgentState::NeedsYou));
        assert!(!app.agent_working_since.contains_key(&pane));
        apply(&mut app, pane, rec("stop", 130, 0));
        assert_eq!(state(&app, pane), Some(AgentState::Idle));
        // The agent badge reports the hook session.
        assert_eq!(app.agent_state(pane), Some(("claude", AgentState::Idle)));
        // A new stretch of work starts at its own record.
        apply(&mut app, pane, rec("prompt", 140, 0));
        assert_eq!(app.agent_working_since.get(&pane), Some(&140));
    }

    #[test]
    fn running_subagents_keep_the_session_working_after_stop() {
        let (mut app, pane) = app_with_pane();
        apply(&mut app, pane, rec("prompt", 100, 0));
        apply(&mut app, pane, rec("stop", 110, 2));
        assert_eq!(state(&app, pane), Some(AgentState::Working));
        assert_eq!(app.agent_working_since.get(&pane), Some(&100));
        assert_eq!(app.agent_badge(pane).map(|b| b.subagents), Some(2));
        // The last subagent ends: the same stop record now reads as the user's turn.
        apply(&mut app, pane, rec("stop", 110, 0));
        assert_eq!(state(&app, pane), Some(AgentState::Idle));
    }

    #[test]
    fn older_orphan_and_ended_records_are_ignored() {
        let (mut app, pane) = app_with_pane();
        apply(&mut app, pane, rec("prompt", 100, 0));
        assert!(app.agent_hooks.contains_key(&pane));
        // The prompt came back: the agent is gone and records written before that are ignored.
        app.clear_agent(pane);
        assert!(!app.agent_hooks.contains_key(&pane));
        apply(&mut app, pane, rec("prompt", 100, 0));
        assert!(!app.agent_hooks.contains_key(&pane));
        // A record newer than the clearing is a new session.
        apply(&mut app, pane, rec("prompt", chrono::Utc::now().timestamp() + 5, 0));
        assert_eq!(state(&app, pane), Some(AgentState::Working));
        // The session ends: the record goes.
        apply(&mut app, pane, rec(crate::hooks::SESSION_END, chrono::Utc::now().timestamp() + 6, 0));
        assert!(!app.agent_hooks.contains_key(&pane));
        // A record of a pane that does not exist is dropped.
        apply(&mut app, 9999, rec("prompt", 100, 0));
        assert!(!app.agent_hooks.contains_key(&9999));
    }
}
