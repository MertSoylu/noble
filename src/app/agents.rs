//! Agent tracking: the state of the AI agents running in panes (from Claude's hook records or guessed from
//! the launcher command / window title), their badges and the session lists shown on Home and in the tab strip.

use std::collections::HashMap;

use super::*;

/// An agent session's state from its last hook event. While subagents still run the
/// session is working even after the main answer ended (`stop`); a question from the agent
/// or one of its subagents (a permission prompt) still needs you, at once; a failed turn is an error. A tool the
/// agent ran (Cursor reports those) means it is working.
pub(super) fn hook_state(rec: &crate::hooks::HookRecord) -> Option<AgentState> {
    use crate::hooks::{NOTIFICATION, PERMISSION_REQUEST, PROMPT, SESSION_START, STOP, STOP_FAILURE, TOOL_USE};
    let state = match rec.event.as_str() {
        PROMPT | TOOL_USE => AgentState::Working,
        NOTIFICATION | PERMISSION_REQUEST => AgentState::NeedsYou,
        STOP_FAILURE => return Some(AgentState::Error),
        STOP | SESSION_START => AgentState::Idle,
        _ => return None,
    };
    Some(if state == AgentState::Idle && rec.subagents > 0 { AgentState::Working } else { state })
}

/// Two records of one permission prompt: the agent sends a permission request and a notification for it, in
/// either order. Both need you, so only the first one is announced.
fn same_prompt(old: &crate::hooks::HookRecord, new: &crate::hooks::HookRecord) -> bool {
    use crate::hooks::{NOTIFICATION, PERMISSION_REQUEST};
    matches!(
        (old.event.as_str(), new.event.as_str()),
        (PERMISSION_REQUEST, NOTIFICATION) | (NOTIFICATION, PERMISSION_REQUEST)
    ) && hook_state(old) == hook_state(new)
}

/// How a notice names the agent: Claude's own name is short.
fn agent_who(agent: crate::hooks::Agent) -> &'static str {
    match agent {
        crate::hooks::Agent::Claude => "Claude",
        other => other.name(),
    }
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
            // The conversation the agent reported: kept after its record goes (prompt, session end), so a later
            // save of the pane can resume it. A session id that is not safe is ignored.
            if let Some(id) = rec.session.as_deref().and_then(crate::hooks::resume::valid_id) {
                let resume = crate::hooks::resume::Resume { agent: rec.agent(), id };
                self.agent_resume.insert(pane, resume);
            }
            // A new event notifies, and so does the state changing without one: the last
            // background subagent ending after `stop` turns Working into Idle with the same
            // event. A count change that keeps the state (2 subagents to 1) does not.
            let new_event = self.agent_hooks.get(&pane).is_none_or(|old| {
                (&old.event, &old.message, old.ts, old.agent()) != (&rec.event, &rec.message, rec.ts, rec.agent())
                    || hook_state(old) != hook_state(&rec)
            });
            let announced = self.agent_hooks.get(&pane).is_some_and(|old| same_prompt(old, &rec));
            if new_event && !announced {
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
            let who = agent_who(rec.agent());
            let notice = match hook_state(&rec) {
                Some(AgentState::NeedsYou) => {
                    Some(rec.message.clone().unwrap_or_else(|| format!("{who} needs your attention")))
                }
                Some(AgentState::Error) => {
                    Some(rec.message.clone().unwrap_or_else(|| format!("{who} stopped with an error")))
                }
                Some(AgentState::Idle) if rec.event == crate::hooks::STOP => Some(format!("{who} finished")),
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
        self.agent_resume.remove(&pane);
        self.agent_cleared.remove(&pane);
        self.agent_working_since.remove(&pane);
        if self.agent_hooks.remove(&pane).is_some() {
            crate::hooks::remove_record(&self.paths.data, std::process::id(), pane);
        }
    }

    /// Called once after an agent's hooks were written: Codex runs user-level hooks only after the user trusted
    /// them in its `/hooks` view, so NOBLE says so. Other agents need nothing.
    pub fn announce_hook_install(&mut self, agent: crate::hooks::Agent) {
        if agent == crate::hooks::Agent::Codex {
            self.toast(ToastLevel::Info, crate::hooks::codex::TRUST_NOTICE);
        }
    }

    /// The AI agent running in a pane and its state: from Claude's hook record, else
    /// guessed from the launcher command (while it still runs) or the window title.
    pub fn agent_state(&self, pane: PaneId) -> Option<(&'static str, AgentState)> {
        // A record whose event this build does not map falls through to the guess below.
        if let Some(rec) = self.agent_hooks.get(&pane)
            && let Some(state) = hook_state(rec)
        {
            return Some((rec.agent().id(), state));
        }
        let p = self.panes.get(&pane)?;
        // Once the launcher command has exited, the pane runs whatever was typed at the prompt.
        let command = p.command.as_deref().filter(|_| p.launch_running());
        let kind = crate::ai::agent_kind(command, &p.label())?;
        // Only a notice (bell, notification) means the agent asks for the user; a finished or failed command does not.
        let alert = self.tabs.iter().any(|t| t.alert == Some(TabAlert::Notice) && t.root.contains(pane));
        // Without hooks the progress report (OSC 9;4) is the only sign that the agent works.
        let state = if alert {
            AgentState::NeedsYou
        } else if self.pane_busy(pane) {
            AgentState::Working
        } else {
            AgentState::Running
        };
        Some((kind, state))
    }

    /// The pane's program reports work in progress (OSC 9;4 states 1 and 3, `Progress::busy`).
    pub fn pane_busy(&self, pane: PaneId) -> bool {
        self.pane_progress(pane).is_some_and(|g| g.busy())
    }

    /// The pane's progress report, if it has one (`Pane::progress`).
    pub fn pane_progress(&self, pane: PaneId) -> Option<crate::term::pane::Progress> {
        self.panes.get(&pane)?.progress()
    }

    /// A tab with a busy progress report in any of its panes: a busy marker on the tab strip.
    pub fn tab_busy(&self, tab: usize) -> bool {
        self.tabs.get(tab).is_some_and(|t| t.panes().into_iter().any(|p| self.pane_busy(p)))
    }

    /// The agent part of a pane's title: its state, how long it has been working and its subagents.
    pub fn agent_badge(&self, pane: PaneId) -> Option<AgentBadge> {
        let (kind, state) = self.agent_state(pane)?;
        let rec = self.agent_hooks.get(&pane);
        let working_since = (state == AgentState::Working)
            .then(|| self.agent_working_since.get(&pane).copied().or(rec.map(|r| r.ts)))
            .flatten();
        let quota = self.agent_quota_warning(kind);
        Some(AgentBadge { kind, state, working_since, subagents: rec.map_or(0, |r| r.subagents), quota })
    }

    /// The most urgent agent state among a tab's panes (the dot in the tab strip).
    pub fn tab_agent_state(&self, tab: usize) -> Option<AgentState> {
        let tab = self.tabs.get(tab)?;
        tab.panes().into_iter().filter_map(|p| self.agent_state(p).map(|(_, s)| s)).max_by_key(AgentState::urgency)
    }

    /// Agents that wait for the user (`AgentState::NeedsYou`) in any tab, visible or not: the outer terminal's
    /// progress pause and the count in its title (`App::outer_sequences`).
    pub fn agents_waiting(&self) -> usize {
        self.tabs
            .iter()
            .flat_map(|t| t.panes())
            .filter(|&p| matches!(self.agent_state(p), Some((_, AgentState::NeedsYou))))
            .count()
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
    /// A busy progress report with no percentage (`progress_spins`) spins the same way.
    pub fn agent_spinning(&self) -> bool {
        !self.on_battery() && self.visible_panes().into_iter().any(|p| self.agent_live(p) || self.progress_spins(p))
    }

    /// The pane's progress report is busy with no percentage to show (`Progress::spins`).
    pub fn progress_spins(&self, pane: PaneId) -> bool {
        self.pane_progress(pane).is_some_and(|g| g.spins())
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

/// Whether the one-time hooks offer should show for a launch: the launched agent is installed here with its hooks
/// off (`missing`), and the offer was never made.
pub(super) fn should_offer_hooks(missing: bool, offered: bool) -> bool {
    missing && !offered
}

impl App {
    /// After a quick launch: the first time an agent is started whose status hooks are off, offers to enable the
    /// hooks of every installed agent that lacks them (a clickable notice, see `Hit::HooksOffer`). Recorded in
    /// `state.json`, so it is offered only once.
    pub(super) fn note_agent_offer(&mut self, launch_command: &str) {
        let Some(agent) = crate::ai::agent_kind(Some(launch_command), "").and_then(crate::hooks::Agent::from_id) else {
            return;
        };
        let missing = self.missing_hooks().contains(&agent);
        if !should_offer_hooks(missing, self.ui_state.data.hooks_offered) {
            return;
        }
        self.edit_ui_state(|d| d.hooks_offered = true);
        self.hooks_offer = true;
        self.dirty = true;
    }

    /// The offer is showing (and still makes sense: an installed agent lacks its hooks).
    pub fn hooks_offer_notice(&self) -> bool {
        self.hooks_offer && self.agents_found.iter().any(|a| !self.hooks_on(*a))
    }

    /// Click on the offer: installs the hooks of every installed agent that lacks them (the Settings popup's path).
    pub(super) fn accept_hooks_offer(&mut self) {
        self.hooks_offer = false;
        let missing = self.missing_hooks();
        self.install_agent_hooks(&missing);
    }

    /// Click on the notice's x, or the palette's "Dismiss Agent Status Offer": hides the offer for this run.
    pub(super) fn dismiss_hooks_offer(&mut self) {
        self.hooks_offer = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::HookRecord;

    fn rec(event: &str, ts: i64, subagents: usize) -> HookRecord {
        HookRecord { event: event.into(), message: None, ts, agent: None, session: None, subagents }
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
    fn hooks_offer_only_for_a_missing_agent_and_once() {
        assert!(should_offer_hooks(true, false));
        assert!(!should_offer_hooks(false, false), "the launched agent has its hooks on or is not installed here");
        assert!(!should_offer_hooks(true, true), "offered before");
    }

    /// The launcher's command names the agent the offer is about: `cursor-agent` is Cursor's, and the IDE's `cursor`
    /// is not (the offer and the pane's agent name read the same mapping).
    #[test]
    fn a_launched_agent_is_named_by_its_command() {
        use crate::hooks::Agent;
        let named = |cmd: &str| crate::ai::agent_kind(Some(cmd), "").and_then(Agent::from_id);
        assert_eq!(named("cursor-agent"), Some(Agent::Cursor));
        assert_eq!(named("cursor"), None);
        assert_eq!(named("agy"), Some(Agent::Antigravity));
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
        // A tool the agent ran (Cursor's tool events) is work, and a turn that ended after it is idle again.
        assert_eq!(hook_state(&rec("tool-use", 1, 0)), Some(AgentState::Working));
        assert_eq!(hook_state(&rec("tool-use", 1, 2)), Some(AgentState::Working));
        // Subagents still running keep the session working after the main answer; a question still needs you.
        assert_eq!(hook_state(&rec("stop", 1, 2)), Some(AgentState::Working));
        assert_eq!(hook_state(&rec("notification", 1, 2)), Some(AgentState::NeedsYou));
        assert_eq!(hook_state(&rec("something-else", 1, 0)), None);
    }

    /// A permission request needs you at once (also with subagents running); a failed turn is an error
    /// and does not wait for subagents.
    #[test]
    fn permission_requests_and_failed_turns() {
        use crate::hooks::{PERMISSION_REQUEST, STOP_FAILURE};
        assert_eq!(hook_state(&rec(PERMISSION_REQUEST, 1, 2)), Some(AgentState::NeedsYou));
        assert_eq!(hook_state(&rec(STOP_FAILURE, 1, 0)), Some(AgentState::Error));
        assert_eq!(hook_state(&rec(STOP_FAILURE, 1, 2)), Some(AgentState::Error));
        // One prompt, two records (in either order): the second is not a new question.
        let (req, note) = (rec(PERMISSION_REQUEST, 1, 0), rec("notification", 2, 0));
        assert!(same_prompt(&req, &note) && same_prompt(&note, &req));
        assert!(!same_prompt(&rec("stop", 1, 0), &note), "a stop in between is another turn");
        assert!(!same_prompt(&req, &rec(STOP_FAILURE, 2, 0)));
    }

    /// The records of an agent other than Claude show under that agent's name; a record without one is Claude's.
    #[test]
    fn records_show_under_their_own_agent() {
        use crate::hooks::Agent;
        let (mut app, pane) = app_with_pane();
        let mut codex = rec("prompt", 100, 0);
        codex.agent = Some(Agent::Codex);
        apply(&mut app, pane, codex);
        assert_eq!(app.agent_state(pane), Some(("codex", AgentState::Working)));
        apply(&mut app, pane, rec("stop", 110, 0));
        assert_eq!(app.agent_state(pane), Some(("claude", AgentState::Idle)));
    }

    /// A record whose event is not mapped does not hide the pane's agent: the launcher's guess shows instead.
    #[test]
    fn an_unmapped_record_falls_back_to_the_guess() {
        let (mut app, pane) = app_with_pane();
        apply(&mut app, pane, rec("something-else", 100, 0));
        assert_eq!(app.agent_state(pane), None, "no launcher or title names an agent here");
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

    /// Output of a pane, as the reader thread would feed it (the main loop then takes the change).
    fn feed_pane(app: &App, pane: PaneId, bytes: &[u8]) {
        app.panes[&pane].parser().process(bytes);
    }

    /// An agent without hooks (guessed from its command) counts as working while its progress report is busy; a clear
    /// (state 0) brings it back to running. A plain shell's busy report marks its tab but names no agent, and the
    /// shell's prompt ends whatever report the program left.
    #[test]
    fn a_busy_progress_report_marks_an_agent_without_hooks() {
        let (mut app, pane) = app_with_pane();
        let tab = app.tabs.iter().position(|t| t.root.contains(pane)).expect("tab");
        app.panes.get_mut(&pane).unwrap().command = Some("codex".into());
        assert_eq!(app.agent_state(pane), Some(("codex", AgentState::Running)));
        assert!(!app.tab_busy(tab));
        feed_pane(&app, pane, b"\x1b]9;4;3;0\x07");
        assert_eq!(app.agent_state(pane), Some(("codex", AgentState::Working)));
        assert!(app.tab_busy(tab));
        assert!(app.progress_spins(pane), "busy with no percentage: a spinner");
        // A percentage is shown in the title instead of a spinner, and the agent still works.
        feed_pane(&app, pane, b"\x1b]9;4;1;40\x07");
        assert_eq!(app.agent_state(pane), Some(("codex", AgentState::Working)));
        assert!(!app.progress_spins(pane));
        feed_pane(&app, pane, b"\x1b]9;4;0\x07");
        assert_eq!(app.agent_state(pane), Some(("codex", AgentState::Running)));
        assert!(!app.tab_busy(tab));
        // A paused or failed report is no sign of work.
        feed_pane(&app, pane, b"\x1b]9;4;4;10\x07");
        assert_eq!(app.agent_state(pane), Some(("codex", AgentState::Running)));
        feed_pane(&app, pane, b"\x1b]9;4;2;10\x07");
        assert_eq!(app.agent_state(pane), Some(("codex", AgentState::Running)));
        // The shell's prompt ends the report (the agent's exit itself is handled on the main loop).
        feed_pane(&app, pane, b"\x1b]9;4;3;0\x07\x1b]133;A\x07");
        assert!(app.panes[&pane].progress().is_none());
        assert!(!app.tab_busy(tab));

        // A plain shell: no agent state, but its tab is busy while the report is.
        app.open_project_shell(std::env::temp_dir());
        let shell_tab = app.tabs.len() - 1;
        let shell = app.tabs[shell_tab].focus;
        feed_pane(&app, shell, b"\x1b]9;4;1;5\x07");
        assert_eq!(app.agent_state(shell), None);
        assert!(app.tab_busy(shell_tab));
        feed_pane(&app, shell, b"\x1b]9;4;0\x07");
        assert!(!app.tab_busy(shell_tab));
    }

    /// The dev build's outer title says `NOBLE dev`, like its top bar.
    #[test]
    fn outer_title_names_the_dev_build() {
        let (mut app, _) = app_with_pane();
        app.dev = true;
        assert_eq!(app.outer_sequences(true).as_deref(), Some("\x1b]0;NOBLE dev\x07"));
    }

    /// The outer terminal's progress pause and title follow the agents that need you; only a change is written.
    #[test]
    fn outer_status_follows_the_agents_that_need_you() {
        let (mut app, pane) = app_with_pane();
        app.open_project_shell(std::env::temp_dir());
        let other = app.tabs.last().expect("tab").focus;
        // Both records in one snapshot: a snapshot replaces the records it does not list.
        let both = |app: &mut App, a: &str, b: &str| {
            app.apply_hook_records(HashMap::from([(pane, rec(a, 100, 0)), (other, rec(b, 100, 0))]));
        };
        // The first write sets the title; nothing waits, so no pause.
        assert_eq!(app.outer_sequences(true).as_deref(), Some("\x1b]0;NOBLE\x07"));
        assert_eq!(app.outer_sequences(true), None, "nothing changed");
        both(&mut app, "notification", "stop");
        assert_eq!(app.agents_waiting(), 1);
        assert_eq!(app.outer_sequences(true).as_deref(), Some("\x1b]9;4;4;0\x1b\\\x1b]0;NOBLE · ◆ 1\x07"));
        assert_eq!(app.outer_sequences(true), None);
        // A second agent waits: the count moves, the pause is not repeated.
        both(&mut app, "notification", crate::hooks::PERMISSION_REQUEST);
        assert_eq!(app.agents_waiting(), 2);
        assert_eq!(app.outer_sequences(true).as_deref(), Some("\x1b]0;NOBLE · ◆ 2\x07"));
        // Both finish: the pause is cleared with the last one.
        both(&mut app, "stop", "stop");
        assert_eq!(app.agents_waiting(), 0);
        assert_eq!(app.outer_sequences(true).as_deref(), Some("\x1b]9;4;0;0\x1b\\\x1b]0;NOBLE\x07"));
        // A terminal without the progress report (Windows console host) gets the title only.
        both(&mut app, "notification", "stop");
        assert_eq!(app.outer_sequences(false).as_deref(), Some("\x1b]0;NOBLE · ◆ 1\x07"));
    }
}
