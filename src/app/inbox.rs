//! The Agent Inbox: one overlay that lists every agent pane, across all tabs, whose agent needs the user, so several
//! waiting agents can be handled without jumping into each pane. A row shows the agent, where it runs, how long it
//! has waited and what it asks; the selected row also shows the last lines of its pane's screen.
//!
//! Quick answers exist for one case only: a Claude Code permission prompt that the pane still shows. The pane's
//! record must be Claude's own permission request, and the screen must show the dialog's numbered options (`Yes` as
//! the first one, a `No` one). The key sent is the digit the screen shows for the option, so a dialog with two options
//! (no "don't ask again") or three cannot make `n` pick the wrong one. Nothing is ever answered without a key press
//! or a click on the chip. Every other agent, and every other question of Claude's, is jump only.
//! Drawing is `ui/inbox.rs`; clicks are in `app/input.rs`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::agents::shown_state;
use super::*;
use crate::hooks::{Agent, HookRecord};

/// The inbox's selection. The row is followed by its pane, so a list that changes under the cursor (a new agent
/// waits, one is answered) keeps the same agent selected; when that pane stops waiting the same position stays
/// selected (`resolve`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InboxState {
    pub pane: Option<PaneId>,
    pub selected: usize,
}

/// A quick answer to a Claude permission prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickAnswer {
    Yes,
    No,
}

/// One agent that needs the user.
#[derive(Clone, Debug, PartialEq)]
pub struct InboxItem {
    pub tab: usize,
    pub pane: PaneId,
    /// The pane's place in its tab (1-based) and how many panes the tab has.
    pub pane_no: usize,
    pub panes_in_tab: usize,
    /// The agent's id (`claude`, `codex`…).
    pub kind: &'static str,
    pub tab_title: String,
    /// The project the pane's folder is in, else the folder's name.
    pub place: String,
    pub cwd: String,
    /// When the agent started waiting (its record's time); `None` for an agent known only from a bell or a title.
    pub since: Option<i64>,
    /// What it asks: the record's message, else a generic line.
    pub message: String,
    /// The record is Claude's own permission request: a quick answer may be offered (`App::inbox_quick`).
    pub permission: bool,
}

/// The numbers a permission dialog shows for its Yes and its No option.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PermissionOptions {
    pub yes: u8,
    pub no: u8,
}

impl PermissionOptions {
    /// The key that picks the option the answer names.
    pub fn key(&self, answer: QuickAnswer) -> char {
        let n = match answer {
            QuickAnswer::Yes => self.yes,
            QuickAnswer::No => self.no,
        };
        char::from(b'0' + n)
    }
}

/// Waiting longest first (the one that has been kept waiting the most is on top); an agent with no time
/// (known only from a bell or a title) comes last. Equal times keep the tab and pane order.
pub fn sort_items(items: &mut [InboxItem]) {
    items.sort_by_key(|i| (i.since.is_none(), i.since, i.tab, i.pane_no));
}

/// The selected row for the list as it is now: the pane the selection follows, else the same position (kept in
/// range). 0 for an empty list.
pub fn resolve(items: &[InboxItem], st: &InboxState) -> usize {
    st.pane
        .and_then(|p| items.iter().position(|i| i.pane == p))
        .unwrap_or_else(|| st.selected.min(items.len().saturating_sub(1)))
}

/// How long an agent has waited: "just now", "5m", "2h 05m".
pub fn wait_label(secs: i64) -> String {
    let secs = secs.max(0) as u64;
    if secs < 60 { "just now".into() } else { crate::util::fmt_duration(Duration::from_secs(secs)) }
}

/// Whether a record is Claude's own permission request: its event, or the text a permission prompt leaves
/// (`hooks::claude::record_message`). A question ("Claude asks a question") or another agent's request is not.
pub fn permission_record(rec: &HookRecord) -> bool {
    use crate::hooks::PERMISSION_REQUEST;
    if rec.agent() != Agent::Claude {
        return false;
    }
    rec.event == PERMISSION_REQUEST
        || rec.message.as_deref().is_some_and(|m| {
            m.starts_with("Claude asks to run ")
                || m == "Claude asks for permission"
                || m.starts_with("Claude needs your permission")
        })
}

/// An option line of a dialog: its number and text (`❯ 1. Yes`, `  3. No, and tell Claude what to do differently`).
fn option_line(row: &str) -> Option<(u8, &str)> {
    let rest = row.trim_start_matches(|c: char| c.is_whitespace() || "│┃❯>›▸→●*".contains(c));
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits > 2 {
        return None;
    }
    let n: u8 = rest[..digits].parse().ok()?;
    let text = rest[digits..].strip_prefix(['.', ')'])?;
    // The number is followed by a blank: "1.5 seconds" is no option.
    let text = text.strip_prefix(' ')?;
    Some((n, text.trim()))
}

/// The Yes and No options of a Claude permission dialog on the screen `rows`, or `None` when the screen shows none.
/// Strict on purpose: a false "no" only leaves the row jump only.
/// - the last run of numbered lines (1, 2, 3 … each a few rows below the one before) ends the screen's numbered
///   lines, so a numbered list printed elsewhere is not taken for the dialog;
/// - the run starts with a plain `Yes` (not "always" / "don't ask again") and has a later `No…` option;
/// - the question "Do you want …" stands just above the first option;
/// - the numbers are single digits, so one key picks them.
pub fn permission_options(rows: &[String]) -> Option<PermissionOptions> {
    let numbered: Vec<(usize, u8, &str)> =
        rows.iter().enumerate().filter_map(|(i, r)| option_line(r).map(|(n, t)| (i, n, t))).collect();
    let first = numbered.iter().rposition(|(_, n, _)| *n == 1)?;
    let run = &numbered[first..];
    for pair in run.windows(2) {
        let ((row, n, _), (next_row, next_n, _)) = (pair[0], pair[1]);
        if next_n != n + 1 || next_row <= row || next_row - row > 3 {
            return None;
        }
    }
    if run.len() < 2 || run.iter().any(|(_, n, _)| *n > 9) {
        return None;
    }
    let (first_row, _, yes_text) = run[0];
    let lower = yes_text.to_lowercase();
    let plain_yes = yes_text.starts_with("Yes")
        && !["don't ask", "do not ask", "always", "allow all", "all "].iter().any(|w| lower.contains(w));
    if !plain_yes {
        return None;
    }
    let no = run.iter().skip(1).rev().find(|(_, _, t)| t.starts_with("No"))?.1;
    let asked = rows[first_row.saturating_sub(4)..first_row].iter().any(|r| r.contains("Do you want"));
    asked.then_some(PermissionOptions { yes: 1, no })
}

/// The quick answer a key stands for: `y` or `1` is yes, `n` is no. No other key, and no Ctrl, Alt or Super with one.
/// (`1` is yes only as the key of the chip: the number sent is the one the screen shows, `PermissionOptions::key`.)
pub fn quick_answer_key(k: &KeyEvent) -> Option<QuickAnswer> {
    if k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER) {
        return None;
    }
    match k.code {
        KeyCode::Char('y' | 'Y' | '1') => Some(QuickAnswer::Yes),
        KeyCode::Char('n' | 'N') => Some(QuickAnswer::No),
        _ => None,
    }
}

/// The last `max` non-blank lines of a screen's rows (blank rows at the end of the screen are cut).
pub fn preview_lines(rows: &[String], max: usize) -> Vec<&str> {
    let end = rows.iter().rposition(|r| !r.trim().is_empty()).map_or(0, |i| i + 1);
    rows[end.saturating_sub(max)..end].iter().map(String::as_str).collect()
}

impl App {
    /// The agents that need the user, in every tab (`agent_state` says so: a hook record, or a bell on an agent
    /// without hooks), longest waiting first.
    pub fn inbox_items(&self) -> Vec<InboxItem> {
        let mut items = Vec::new();
        for (ti, tab) in self.tabs.iter().enumerate() {
            let panes = tab.panes();
            for (k, &pane) in panes.iter().enumerate() {
                let Some((kind, AgentState::NeedsYou)) = self.agent_state(pane) else { continue };
                // The record that makes it wait; a pane that waits on a bell only has none.
                let rec = self
                    .agent_hooks
                    .get(&pane)
                    .filter(|r| shown_state(r, self.agent_answered.get(&pane).copied()) == Some(AgentState::NeedsYou));
                let cwd = self.panes.get(&pane).map(|p| p.cwd()).unwrap_or_default();
                let place = crate::projects::project_containing(&self.projects, &cwd)
                    .map(|p| p.name.clone())
                    .or_else(|| cwd.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_default();
                let who = rec.map_or("The agent", |r| agent_name(r.agent()));
                items.push(InboxItem {
                    tab: ti,
                    pane,
                    pane_no: k + 1,
                    panes_in_tab: panes.len(),
                    kind,
                    tab_title: self.tab_title(ti),
                    place,
                    cwd: crate::util::tilde(&cwd),
                    since: rec.map(|r| r.ts),
                    message: rec
                        .and_then(|r| r.message.clone())
                        .unwrap_or_else(|| format!("{who} needs your attention")),
                    permission: rec.is_some_and(permission_record),
                });
            }
        }
        sort_items(&mut items);
        items
    }

    /// The Yes and No options of the row's pane when a quick answer is allowed: Claude's own permission request
    /// that is still on the pane's screen (`permission_options`). `None`: the row is jump only.
    pub fn inbox_quick(&self, item: &InboxItem) -> Option<PermissionOptions> {
        if !item.permission {
            return None;
        }
        permission_options(&self.panes.get(&item.pane)?.live_rows())
    }

    /// Opens the inbox with the agent that has waited longest selected.
    pub fn open_inbox(&mut self) {
        let first = self.inbox_items().first().map(|i| i.pane);
        self.overlay = Some(Overlay::Inbox(InboxState { pane: first, selected: 0 }));
    }

    /// The row the state selects now.
    pub fn inbox_selected(&self, st: &InboxState) -> Option<InboxItem> {
        let items = self.inbox_items();
        let at = resolve(&items, st);
        items.into_iter().nth(at)
    }

    /// Moves the selection to the row at `to` (clamped), keeping the pane it lands on.
    fn inbox_select(&self, st: &mut InboxState, to: usize) {
        let items = self.inbox_items();
        let to = to.min(items.len().saturating_sub(1));
        st.selected = to;
        st.pane = items.get(to).map(|i| i.pane);
    }

    /// Selects the row of `pane`.
    pub(super) fn inbox_pick(&mut self, pane: PaneId) {
        let Some(Overlay::Inbox(mut st)) = self.overlay.take() else { return };
        let at = self.inbox_items().iter().position(|i| i.pane == pane);
        if let Some(at) = at {
            self.inbox_select(&mut st, at);
        }
        self.overlay = Some(Overlay::Inbox(st));
    }

    /// The overlay's keys: ↑↓ select, ⏎ goes to the pane, `y` / `n` answer a Claude permission prompt, esc closes.
    /// `false` closes the overlay.
    pub(super) fn inbox_key(&mut self, st: &mut InboxState, k: KeyEvent) -> bool {
        let items = self.inbox_items();
        let at = resolve(&items, st);
        let last = items.len().saturating_sub(1);
        // The state holds the row as it is now: after an answer (the row leaves the list) the same place stays.
        st.selected = at;
        st.pane = items.get(at).map(|i| i.pane);
        if let Some(answer) = quick_answer_key(&k) {
            if let Some(item) = items.get(at) {
                self.inbox_answer(item.pane, answer);
            }
            return true;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let to = match k.code {
            KeyCode::Esc | KeyCode::Char('q') => return false,
            KeyCode::Enter => {
                if let Some(item) = items.get(at) {
                    self.inbox_jump(item.pane);
                    return false;
                }
                return true;
            }
            KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k') => at.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab | KeyCode::Char('j') => (at + 1).min(last),
            KeyCode::Char('p') if ctrl => at.saturating_sub(1),
            KeyCode::Char('n') if ctrl => (at + 1).min(last),
            KeyCode::PageUp => at.saturating_sub(5),
            KeyCode::PageDown => (at + 5).min(last),
            KeyCode::Home => 0,
            KeyCode::End => last,
            _ => return true,
        };
        st.selected = to;
        st.pane = items.get(to).map(|i| i.pane);
        true
    }

    /// Goes to the pane: its tab is shown and the pane focused, a zoomed tab that shows another pane goes back to
    /// its layout first (as `jump_to_agent`).
    pub(super) fn inbox_jump(&mut self, pane: PaneId) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.root.contains(pane)) else {
            self.toast(ToastLevel::Info, "that pane has closed");
            return;
        };
        if tab.zoomed && tab.focus != pane {
            tab.zoomed = false;
            // A zoom animation still running would draw its pane over the tiled layout.
            self.zoom_anim = None;
        }
        self.focus_pane(pane);
    }

    /// A click on the row of `pane`: the first click selects it, a click on the selected row goes to the pane.
    pub(super) fn inbox_click(&mut self, pane: PaneId) {
        let selected = match &self.overlay {
            Some(Overlay::Inbox(st)) => self.inbox_selected(st).map(|i| i.pane),
            _ => return,
        };
        if selected == Some(pane) {
            self.overlay = None;
            self.inbox_jump(pane);
        } else {
            self.inbox_pick(pane);
        }
    }

    /// A click on a quick answer chip: answers the selected row.
    pub(super) fn inbox_chip(&mut self, answer: QuickAnswer) {
        let items = self.inbox_items();
        let Some(Overlay::Inbox(st)) = &mut self.overlay else { return };
        let at = resolve(&items, st);
        st.selected = at;
        st.pane = items.get(at).map(|i| i.pane);
        if let Some(item) = items.get(at) {
            self.inbox_answer(item.pane, answer);
        }
    }

    /// Answers the pane's Claude permission prompt with the user's key: the number of the option the screen shows is
    /// typed into the pane like any key (`pane_key`, which also marks the pane as answered). Checked again right
    /// here against the record and the screen as they are now, so a prompt that is gone or changed gets nothing;
    /// otherwise the user is told why. Returns whether a key was sent.
    pub(super) fn inbox_answer(&mut self, pane: PaneId, answer: QuickAnswer) -> bool {
        let item = self.inbox_items().into_iter().find(|i| i.pane == pane);
        let Some(options) = item.as_ref().and_then(|i| self.inbox_quick(i)) else {
            self.toast(ToastLevel::Info, "no quick answer here · ⏎ goes to the pane");
            return false;
        };
        let key = KeyEvent::new(KeyCode::Char(options.key(answer)), KeyModifiers::NONE);
        if !self.pane_key(pane, key) {
            return false;
        }
        let word = match answer {
            QuickAnswer::Yes => "yes",
            QuickAnswer::No => "no",
        };
        let tab = item.map(|i| i.tab_title).unwrap_or_default();
        self.toast(ToastLevel::Info, format!("answered {word} in {tab}"));
        self.dirty = true;
        true
    }
}

/// How the inbox names an agent in its generic line.
fn agent_name(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "Claude",
        other => other.name(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(tab: usize, pane: PaneId, since: Option<i64>) -> InboxItem {
        InboxItem {
            tab,
            pane,
            pane_no: 1,
            panes_in_tab: 1,
            kind: "claude",
            tab_title: format!("tab{tab}"),
            place: "p".into(),
            cwd: "~/p".into(),
            since,
            message: "Claude asks to run Bash".into(),
            permission: true,
        }
    }

    fn rows(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    fn rec(event: &str, message: Option<&str>, agent: Option<Agent>) -> HookRecord {
        HookRecord {
            event: event.into(),
            message: message.map(Into::into),
            ts: 100,
            agent,
            session: None,
            subagents: 0,
        }
    }

    #[test]
    fn items_wait_longest_first_and_unknown_times_last() {
        let mut items = vec![item(0, 1, Some(300)), item(1, 2, None), item(2, 3, Some(100)), item(0, 4, Some(100))];
        sort_items(&mut items);
        let order: Vec<PaneId> = items.iter().map(|i| i.pane).collect();
        // Equal times keep the tab order; the agent with no time is last.
        assert_eq!(order, vec![4, 3, 1, 2]);
    }

    #[test]
    fn the_selection_follows_its_pane_and_else_keeps_its_place() {
        let items = vec![item(0, 1, Some(1)), item(0, 2, Some(2)), item(0, 3, Some(3))];
        assert_eq!(resolve(&items, &InboxState { pane: Some(3), selected: 0 }), 2, "the pane moves the row");
        // The selected pane stopped waiting: the row at its old place (now the next agent) is selected.
        let gone = vec![item(0, 1, Some(1)), item(0, 3, Some(3))];
        assert_eq!(resolve(&gone, &InboxState { pane: Some(2), selected: 1 }), 1);
        // Past the end: the last row.
        assert_eq!(resolve(&gone, &InboxState { pane: Some(9), selected: 7 }), 1);
        assert_eq!(resolve(&[], &InboxState { pane: Some(2), selected: 3 }), 0, "an empty list has no row");
    }

    #[test]
    fn wait_labels_are_compact() {
        assert_eq!(wait_label(-5), "just now");
        assert_eq!(wait_label(59), "just now");
        assert_eq!(wait_label(60), "1m");
        assert_eq!(wait_label(5 * 60 + 20), "5m");
        assert_eq!(wait_label(2 * 3600 + 5 * 60), "2h 05m");
    }

    #[test]
    fn only_claudes_permission_requests_qualify() {
        use crate::hooks::{NOTIFICATION, PERMISSION_REQUEST};
        assert!(permission_record(&rec(PERMISSION_REQUEST, None, None)));
        assert!(permission_record(&rec(NOTIFICATION, Some("Claude asks to run Bash"), None)));
        assert!(permission_record(&rec(NOTIFICATION, Some("Claude asks for permission"), Some(Agent::Claude))));
        assert!(permission_record(&rec(NOTIFICATION, Some("Claude needs your permission to use Bash"), None)));
        // A question's options are the agent's own: no quick answer.
        assert!(!permission_record(&rec(NOTIFICATION, Some("Claude asks a question"), None)));
        assert!(!permission_record(&rec(NOTIFICATION, Some("Pick a repo"), None)));
        assert!(!permission_record(&rec(NOTIFICATION, None, None)));
        // Another agent's permission request is jump only.
        assert!(!permission_record(&rec(PERMISSION_REQUEST, None, Some(Agent::Codex))));
    }

    #[test]
    fn a_three_option_dialog_gives_yes_and_no() {
        let screen = rows(&[
            "● Bash(npm test)",
            "",
            "╭──────────────────────────────╮",
            "│ Bash command                 │",
            "│   npm test                   │",
            "│ Do you want to proceed?      │",
            "│ ❯ 1. Yes                     │",
            "│   2. Yes, and don't ask again for npm test commands in /w │",
            "│   3. No, and tell Claude what to do differently (esc) │",
            "╰──────────────────────────────╯",
            "",
        ]);
        let o = permission_options(&screen).expect("a permission dialog");
        assert_eq!((o.yes, o.no), (1, 3));
        assert_eq!(o.key(QuickAnswer::Yes), '1');
        assert_eq!(o.key(QuickAnswer::No), '3');
    }

    /// Without the "don't ask again" option the No option is the second: `n` sends what the screen shows.
    #[test]
    fn a_two_option_dialog_sends_the_number_the_screen_shows() {
        let screen = rows(&[
            " Do you want to make this edit to src/main.rs?",
            " ❯ 1. Yes",
            "   2. No, and tell Claude what to do differently (esc)",
            "",
            " Esc to cancel",
        ]);
        let o = permission_options(&screen).expect("a permission dialog");
        assert_eq!((o.yes, o.no), (1, 2));
        assert_eq!(o.key(QuickAnswer::No), '2');
    }

    #[test]
    fn other_screens_offer_no_quick_answer() {
        // Nothing on the screen.
        assert_eq!(permission_options(&[]), None);
        assert_eq!(permission_options(&rows(&["$ ls", "src"])), None);
        // A numbered list in the output is no dialog: no question above it, no No option.
        assert_eq!(permission_options(&rows(&["Steps:", "1. Install", "2. Build", "3. Test"])), None);
        // The first option is not a plain yes (a trust or a question dialog).
        let pick = rows(&["Which branch do you want to use?", "❯ 1. main", "  2. dev", "  3. No change"]);
        assert_eq!(permission_options(&pick), None);
        let always = rows(&["Do you want to proceed?", "❯ 1. Yes, and always allow", "  2. No"]);
        assert_eq!(permission_options(&always), None);
        // No No option.
        let no_no = rows(&["Do you want to proceed?", "❯ 1. Yes", "  2. Yes, and don't ask again"]);
        assert_eq!(permission_options(&no_no), None);
        // The dialog is not the last numbered thing on the screen: something was printed after it.
        let after = rows(&["Do you want to proceed?", "❯ 1. Yes", "  2. No", "", "1. a later list", "2. more"]);
        assert_eq!(permission_options(&after), None);
        // Options with a gap in the numbering.
        let gap = rows(&["Do you want to proceed?", "❯ 1. Yes", "  3. No"]);
        assert_eq!(permission_options(&gap), None);
        // A question far above the options is not theirs.
        let far = rows(&["Do you want to proceed?", "", "", "", "", "", "❯ 1. Yes", "  2. No"]);
        assert_eq!(permission_options(&far), None);
        // A number inside a sentence is no option.
        let decimal = rows(&["Do you want to proceed?", "1.5 seconds", "  2. No"]);
        assert_eq!(permission_options(&decimal), None);
    }

    #[test]
    fn answer_keys_are_y_n_and_one_without_modifiers() {
        let key = |c, m| KeyEvent::new(KeyCode::Char(c), m);
        assert_eq!(quick_answer_key(&key('y', KeyModifiers::NONE)), Some(QuickAnswer::Yes));
        assert_eq!(quick_answer_key(&key('Y', KeyModifiers::SHIFT)), Some(QuickAnswer::Yes));
        assert_eq!(quick_answer_key(&key('1', KeyModifiers::NONE)), Some(QuickAnswer::Yes));
        assert_eq!(quick_answer_key(&key('n', KeyModifiers::NONE)), Some(QuickAnswer::No));
        // Only the options the screen shows are ever sent: 2 and 3 are no quick key.
        assert_eq!(quick_answer_key(&key('2', KeyModifiers::NONE)), None);
        assert_eq!(quick_answer_key(&key('3', KeyModifiers::NONE)), None);
        assert_eq!(quick_answer_key(&key('n', KeyModifiers::CONTROL)), None);
        assert_eq!(quick_answer_key(&key('y', KeyModifiers::ALT)), None);
        assert_eq!(quick_answer_key(&KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)), None);
        assert_eq!(quick_answer_key(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)), None);
    }

    /// Every digit a quick answer can send is a key that answers a prompt, so the pane reads as answered.
    #[test]
    fn a_sent_option_number_answers_the_prompt() {
        for n in 1..=9u8 {
            let o = PermissionOptions { yes: n, no: n };
            let k = KeyEvent::new(KeyCode::Char(o.key(QuickAnswer::Yes)), KeyModifiers::NONE);
            assert!(super::super::agents::answers_prompt(&k), "{n}");
        }
    }

    #[test]
    fn the_preview_is_the_last_lines_without_the_blank_tail() {
        let screen = rows(&["a", "b", "", "c", "d", "", ""]);
        assert_eq!(preview_lines(&screen, 2), vec!["c", "d"]);
        assert_eq!(preview_lines(&screen, 10), vec!["a", "b", "", "c", "d"]);
        assert!(preview_lines(&rows(&["", " "]), 3).is_empty());
        assert!(preview_lines(&[], 3).is_empty());
    }

    // The tests below drive a headless app with its own data folder, as the agent tests do.
    fn app_with_agents(n: usize) -> (App, Vec<PaneId>) {
        let mut app = App::headless(Config::default(), (100, 30));
        let _ = std::fs::create_dir_all(&app.paths.data);
        let mut panes = Vec::new();
        for _ in 0..n {
            app.open_project_shell(std::env::temp_dir());
            panes.push(app.tabs.last().expect("tab").focus);
        }
        (app, panes)
    }

    fn records(app: &mut App, recs: Vec<(PaneId, HookRecord)>) {
        app.apply_hook_records(recs.into_iter().collect());
    }

    #[test]
    fn the_list_holds_the_agents_that_need_you_and_follows_the_records() {
        use crate::hooks::{PERMISSION_REQUEST, PROMPT, STOP};
        let (mut app, panes) = app_with_agents(3);
        let now = chrono::Utc::now().timestamp();
        let mut ask = rec(PERMISSION_REQUEST, Some("Claude asks to run Bash"), None);
        ask.ts = now - 120;
        let mut late = rec(PERMISSION_REQUEST, Some("Claude asks to run Edit"), None);
        late.ts = now - 30;
        let mut work = rec(PROMPT, None, None);
        work.ts = now - 10;
        records(&mut app, vec![(panes[0], late.clone()), (panes[1], ask.clone()), (panes[2], work)]);
        let items = app.inbox_items();
        assert_eq!(items.iter().map(|i| i.pane).collect::<Vec<_>>(), vec![panes[1], panes[0]], "longest wait first");
        assert_eq!(items[0].message, "Claude asks to run Bash");
        assert_eq!(items[0].since, Some(now - 120));
        assert!(items[0].permission);
        assert_eq!(items[0].kind, "claude");

        // The agent that waited longest finishes: only the other one is left.
        let mut stop = rec(STOP, None, None);
        stop.ts = now;
        records(&mut app, vec![(panes[0], late), (panes[1], stop)]);
        assert_eq!(app.inbox_items().iter().map(|i| i.pane).collect::<Vec<_>>(), vec![panes[0]]);
    }

    /// The selected agent stops waiting while the inbox is open: the same position stays selected.
    #[test]
    fn the_selection_stays_sensible_when_the_selected_agent_stops_waiting() {
        use crate::hooks::{PERMISSION_REQUEST, STOP};
        let (mut app, panes) = app_with_agents(3);
        let now = chrono::Utc::now().timestamp();
        let at = |secs: i64| {
            let mut r = rec(PERMISSION_REQUEST, Some("Claude asks to run Bash"), None);
            r.ts = now - secs;
            r
        };
        records(&mut app, vec![(panes[0], at(300)), (panes[1], at(200)), (panes[2], at(100))]);
        app.open_inbox();
        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        app.on_key(key(KeyCode::Down));
        let Some(Overlay::Inbox(st)) = &app.overlay else { panic!("the inbox is open") };
        assert_eq!(st.pane, Some(panes[1]));
        // The middle agent stops waiting: the one that was below it is selected now.
        let mut stop = rec(STOP, None, None);
        stop.ts = now;
        records(&mut app, vec![(panes[0], at(300)), (panes[1], stop), (panes[2], at(100))]);
        let Some(Overlay::Inbox(st)) = &app.overlay else { panic!("still open") };
        let items = app.inbox_items();
        assert_eq!(items[resolve(&items, st)].pane, panes[2]);
        // The last one stops waiting too: the selection is the last row left.
        records(&mut app, vec![(panes[0], at(300))]);
        let Some(Overlay::Inbox(st)) = &app.overlay else { panic!("still open") };
        let items = app.inbox_items();
        assert_eq!(items[resolve(&items, st)].pane, panes[0]);
        // Nobody waits: an empty list, and the keys do nothing but close.
        records(&mut app, vec![]);
        assert!(app.inbox_items().is_empty());
        app.on_key(key(KeyCode::Enter));
        assert!(matches!(app.overlay, Some(Overlay::Inbox(_))), "⏎ on an empty list keeps the overlay");
        app.on_key(key(KeyCode::Esc));
        assert!(app.overlay.is_none());
    }

    /// ⏎ goes to the pane, in another tab, even from a zoomed tab that shows another pane.
    #[test]
    fn enter_goes_to_the_selected_pane() {
        use crate::hooks::PERMISSION_REQUEST;
        let (mut app, panes) = app_with_agents(2);
        let now = chrono::Utc::now().timestamp();
        let mut r = rec(PERMISSION_REQUEST, Some("Claude asks to run Bash"), None);
        r.ts = now - 5;
        records(&mut app, vec![(panes[0], r)]);
        app.view = View::Term(1);
        app.open_inbox();
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.overlay.is_none());
        assert_eq!(app.view, View::Term(0));
        assert_eq!(app.tabs[0].focus, panes[0]);
    }

    /// A key that is no quick answer, or an answer where none is offered, sends nothing to the pane and does not
    /// mark it answered: the pane shows no permission dialog here.
    #[test]
    fn no_quick_answer_without_a_dialog_on_the_screen() {
        use crate::hooks::PERMISSION_REQUEST;
        let (mut app, panes) = app_with_agents(1);
        let now = chrono::Utc::now().timestamp();
        let mut r = rec(PERMISSION_REQUEST, Some("Claude asks to run Bash"), None);
        r.ts = now - 5;
        records(&mut app, vec![(panes[0], r)]);
        let item = app.inbox_items().remove(0);
        assert!(item.permission);
        assert_eq!(app.inbox_quick(&item), None, "the record alone is not enough: the screen must show the dialog");
        app.open_inbox();
        app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(app.agent_answered.is_empty());
        assert_eq!(app.agents_waiting(), 1);
        assert!(matches!(app.overlay, Some(Overlay::Inbox(_))), "the overlay stays open");
    }

    /// With the dialog on the screen the quick answer is offered, and for Claude only: another agent's pane with the
    /// same screen is jump only.
    #[test]
    fn the_screen_and_the_record_must_both_say_permission_prompt() {
        use crate::hooks::{NOTIFICATION, PERMISSION_REQUEST};
        let (mut app, panes) = app_with_agents(3);
        let now = chrono::Utc::now().timestamp();
        let dialog: &[u8] =
            b"Do you want to proceed?\r\n\x1b[36m\xe2\x9d\xaf 1. Yes\x1b[m\r\n  2. No, and tell Claude what to do differently\r\n";
        for p in &panes {
            app.panes[p].parser().process(dialog);
        }
        let mut claude = rec(PERMISSION_REQUEST, Some("Claude asks to run Bash"), None);
        claude.ts = now - 5;
        let mut codex = rec(PERMISSION_REQUEST, Some("Codex wants to run a command"), Some(Agent::Codex));
        codex.ts = now - 6;
        let mut question = rec(NOTIFICATION, Some("Claude asks a question"), None);
        question.ts = now - 7;
        records(&mut app, vec![(panes[0], claude), (panes[1], codex), (panes[2], question)]);
        let items = app.inbox_items();
        assert_eq!(items.len(), 3);
        for item in &items {
            let quick = app.inbox_quick(item);
            if item.pane == panes[0] {
                assert_eq!(quick, Some(PermissionOptions { yes: 1, no: 2 }));
            } else {
                assert_eq!(quick, None, "{} is jump only", item.message);
            }
        }
        // Answering a jump-only row does nothing.
        assert!(!app.inbox_answer(panes[1], QuickAnswer::Yes));
        assert!(!app.inbox_answer(panes[2], QuickAnswer::No));
        assert!(app.agent_answered.is_empty());
    }
}
