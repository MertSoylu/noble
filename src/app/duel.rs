//! Agent duel on the App side: the start form, the compare overlay, the end dialog and the tab with the two agents.
//! The git work (`duel.rs`) runs on a background thread per request and comes back as `AppEvent::Duel`; state only
//! changes here, on the main thread. NOBLE never merges: it creates the worktrees, shows what each side did and
//! removes the worktrees when asked.

use std::ops::Range;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::input::{alt_held, ctrl_held};
use super::{App, Overlay, ToastLevel, View};
use crate::duel::{self, CompareReport, DeleteBranches, EndReport, Plan, Side};
use crate::event::AppEvent;
use crate::projects::Project;
use crate::store::DuelRecord;
use crate::term::layout::{Dir, PaneId};

/// What a background duel request answers with.
pub enum DuelEvent {
    /// The worktrees were created (or why not).
    Created(Result<Box<DuelRecord>, String>),
    /// What each side did (the duel's id with it, since the overlay may have moved on).
    Compared(String, Box<CompareReport>),
    /// How many files each worktree has with uncommitted changes, before ending (or why git could not tell).
    Inspected(String, Result<(usize, usize), String>),
    /// The worktrees were removed (as far as they could be).
    Ended(Box<EndReport>, DeleteBranches),
}

/// An agent the form offers: an installed quick launcher whose agent has a known way to start with a task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuelAgent {
    /// The launcher's name as the quick launch lists it.
    pub name: String,
    /// The launcher's command (what the pane runs, the task after it).
    pub command: String,
    /// `ai::agent_kind` id.
    pub agent: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuelField {
    A,
    B,
    Task,
}

/// The start form: two agents and the task.
pub struct DuelForm {
    pub project: PathBuf,
    pub name: String,
    /// The branch and commit the worktrees start from, for the header line.
    pub from: String,
    pub agents: Vec<DuelAgent>,
    /// Chosen agents (indexes into `agents`).
    pub a: usize,
    pub b: usize,
    pub task: String,
    /// Cursor in `task`, as a character index.
    pub cursor: usize,
    pub field: DuelField,
    pub error: Option<String>,
    /// The worktrees are being created: the form waits.
    pub busy: bool,
}

impl DuelForm {
    fn byte_at(&self, chars: usize) -> usize {
        self.task.char_indices().nth(chars).map_or(self.task.len(), |(i, _)| i)
    }

    /// Inserts text at the cursor (line breaks stay, other control characters go), up to the longest task.
    pub fn insert(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let room = duel::MAX_PROMPT.saturating_sub(self.task.chars().count());
        let text: String = text
            .chars()
            .filter_map(|c| match c {
                '\n' => Some('\n'),
                '\t' => Some(' '),
                c if c.is_control() => None,
                c => Some(c),
            })
            .take(room)
            .collect();
        if text.is_empty() {
            return;
        }
        let at = self.byte_at(self.cursor);
        self.task.insert_str(at, &text);
        self.cursor += text.chars().count();
        self.error = None;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let at = self.byte_at(self.cursor - 1);
            self.task.remove(at);
            self.cursor -= 1;
            self.error = None;
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.task.chars().count() {
            let at = self.byte_at(self.cursor);
            self.task.remove(at);
            self.error = None;
        }
    }

    pub fn move_to(&mut self, to: usize) {
        self.cursor = to.min(self.task.chars().count());
    }

    /// The first character of the line the cursor is on, and the character after its last (the line break's place).
    fn line_span(&self) -> (usize, usize) {
        let chars: Vec<char> = self.task.chars().collect();
        let cursor = self.cursor.min(chars.len());
        let start = chars[..cursor].iter().rposition(|c| *c == '\n').map_or(0, |i| i + 1);
        let end = chars[cursor..].iter().position(|c| *c == '\n').map_or(chars.len(), |i| cursor + i);
        (start, end)
    }

    pub fn line_start(&mut self) {
        self.cursor = self.line_span().0;
    }

    pub fn line_end(&mut self) {
        self.cursor = self.line_span().1;
    }

    /// Moves the cursor one line up or down, keeping its column where the line is long enough. `false` when there is
    /// no such line (the cursor stays).
    pub fn move_line(&mut self, down: bool) -> bool {
        let chars: Vec<char> = self.task.chars().collect();
        let (start, end) = self.line_span();
        let col = self.cursor - start;
        if down {
            if end >= chars.len() {
                return false;
            }
            let next_start = end + 1;
            let next_end = chars[next_start..].iter().position(|c| *c == '\n').map_or(chars.len(), |i| next_start + i);
            self.cursor = (next_start + col).min(next_end);
        } else {
            if start == 0 {
                return false;
            }
            let prev_end = start - 1;
            let prev_start = chars[..prev_end].iter().rposition(|c| *c == '\n').map_or(0, |i| i + 1);
            self.cursor = (prev_start + col).min(prev_end);
        }
        true
    }

    /// The task as display rows of at most `width` columns: a row ends at a line break or where the next character
    /// would not fit. Each row is a range of character indexes (the line break itself belongs to no row).
    pub fn rows(&self, width: usize) -> Vec<Range<usize>> {
        let width = width.max(1);
        let mut rows = Vec::new();
        let (mut start, mut used) = (0usize, 0usize);
        for (i, c) in self.task.chars().enumerate() {
            if c == '\n' {
                rows.push(start..i);
                (start, used) = (i + 1, 0);
                continue;
            }
            let w = crate::util::width(c.encode_utf8(&mut [0; 4]));
            if used + w > width && i > start {
                rows.push(start..i);
                (start, used) = (i, 0);
            }
            used += w;
        }
        rows.push(start..self.task.chars().count());
        rows
    }

    /// The row the cursor is on, among `rows(width)`.
    pub fn cursor_row(&self, rows: &[Range<usize>]) -> usize {
        rows.iter().rposition(|r| r.start <= self.cursor).unwrap_or(0)
    }

    fn step_field(&mut self, delta: i32) {
        const ORDER: [DuelField; 3] = [DuelField::A, DuelField::B, DuelField::Task];
        let at = ORDER.iter().position(|f| *f == self.field).unwrap_or(0) as i32;
        self.field = ORDER[(at + delta).clamp(0, 2) as usize];
    }

    /// Picks the next or previous agent for a side.
    fn cycle(&mut self, side: Side, delta: isize) {
        let n = self.agents.len().max(1) as isize;
        let slot = if side == Side::A { &mut self.a } else { &mut self.b };
        *slot = (*slot as isize + delta).rem_euclid(n) as usize;
        self.error = None;
    }
}

/// The compare overlay: what each side did.
pub struct CompareState {
    pub rec: DuelRecord,
    /// `None` while git is being asked.
    pub report: Option<CompareReport>,
    /// The side the commands and the copy refer to (and the only one shown in a narrow window).
    pub side: Side,
    /// First file row shown, per side.
    pub scroll: [usize; 2],
}

/// What the end dialog is waiting for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndStep {
    /// The question: remove the worktrees, and which branches (if any) to delete as well.
    Ask,
    /// git is looking for uncommitted changes.
    Checking,
    /// Both worktrees have uncommitted changes (A, B files); only an explicit force removes them.
    Dirty(usize, usize),
    /// The worktrees are being removed.
    Removing,
}

pub struct EndState {
    pub rec: DuelRecord,
    pub delete: DeleteBranches,
    pub step: EndStep,
    pub error: Option<String>,
}

pub enum DuelView {
    Form(Box<DuelForm>),
    Compare(Box<CompareState>),
    End(Box<EndState>),
}

/// Mouse targets of the duel overlays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuelHit {
    /// An agent chip: picks `agents[index]` for the side.
    Pick(Side, usize),
    /// A form row: moves the focus there.
    Field(DuelField),
    Start,
    Close,
    /// Compare: focuses a side.
    Side(Side),
    Copy,
    Refresh,
    /// End: the branch choice, the confirmation, the forced removal.
    Delete(DeleteBranches),
    Confirm,
    Force,
}

impl App {
    // ─── Which duel, which project ───────────────────────────────────────────────────────────────

    /// The duels not ended yet, oldest first.
    pub fn duels(&self) -> &[DuelRecord] {
        &self.ui_state.data.duels
    }

    /// The duel whose worktree one of the tab's panes is in.
    pub fn duel_of_tab(&self, ti: usize) -> Option<&DuelRecord> {
        let tab = self.tabs.get(ti)?;
        let panes: Vec<PathBuf> = tab.panes().iter().filter_map(|id| self.panes.get(id)).map(|p| p.cwd()).collect();
        let named = tab.name.as_deref();
        self.duels().iter().rev().find(|rec| {
            panes.iter().any(|cwd| duel::side_of(rec, cwd).is_some()) || named == Some(rec.tab_name().as_str())
        })
    }

    /// The duel Compare and End act on: the one of the visible tab, else the newest of the project selected on Home.
    pub fn target_duel(&self) -> Option<&DuelRecord> {
        match self.view {
            View::Term(ti) => self.duel_of_tab(ti),
            View::Bridge => {
                let project = self.selected_project()?;
                self.duels()
                    .iter()
                    .rev()
                    .find(|d| crate::util::same_path(std::path::Path::new(&d.project), &project.path))
            }
            _ => None,
        }
    }

    /// The project a new duel starts in: the duel's own project inside a duel tab, else the project of the focused
    /// pane's folder, or the one selected on Home.
    pub fn duel_project(&self) -> Option<&Project> {
        match self.view {
            View::Bridge => self.selected_project(),
            View::Term(ti) => {
                if let Some(rec) = self.duel_of_tab(ti) {
                    return self
                        .projects
                        .iter()
                        .find(|p| crate::util::same_path(&p.path, std::path::Path::new(&rec.project)));
                }
                crate::projects::project_containing(&self.projects, &self.focused_cwd()?)
            }
            _ => None,
        }
    }

    /// The agents the form offers: installed quick launchers whose agent has a known way to start with a task.
    pub fn duel_agents(&self) -> Vec<DuelAgent> {
        self.launchers
            .iter()
            .filter(|(_, installed)| *installed)
            .filter_map(|(l, _)| {
                duel::duel_agent(&l.command).map(|agent| DuelAgent {
                    name: l.name.clone(),
                    command: l.command.clone(),
                    agent,
                })
            })
            .collect()
    }

    // ─── Start ───────────────────────────────────────────────────────────────────────────────────

    /// Opens the start form for `path`'s project (or the current one).
    pub fn open_duel_form(&mut self, path: Option<PathBuf>) {
        let project = match &path {
            Some(path) => self.projects.iter().find(|p| crate::util::same_path(&p.path, path)),
            None => self.duel_project(),
        };
        let Some(project) = project.cloned() else {
            return self.toast(ToastLevel::Info, "select a project on Home (or open a terminal in one) first");
        };
        if !project.repo || crate::projects::git_dir(&project.path).is_none() {
            return self
                .toast(ToastLevel::Warn, format!("{} is not a git repository — a duel needs one", project.name));
        }
        let agents = self.duel_agents();
        if agents.is_empty() {
            return self.toast(
                ToastLevel::Warn,
                "no installed agent can start with a task — a duel needs claude, codex, copilot or opencode",
            );
        }
        let from = match &project.branch {
            Some(b) => format!("{b} (HEAD now)"),
            None => "HEAD now".into(),
        };
        let b = usize::from(agents.len() > 1);
        self.overlay = Some(Overlay::Duel(DuelView::Form(Box::new(DuelForm {
            project: project.path.clone(),
            name: project.name.clone(),
            from,
            agents,
            a: 0,
            b,
            task: String::new(),
            cursor: 0,
            field: DuelField::Task,
            error: None,
            busy: false,
        }))));
    }

    /// Checks the form, then asks a background thread to create the worktrees. The form stays up, busy, until the
    /// answer comes (`DuelEvent::Created`).
    fn submit_duel_form(&mut self, f: &mut DuelForm) {
        if f.busy {
            return;
        }
        let task = duel::clean_prompt(&f.task);
        let (Some(a), Some(b)) = (f.agents.get(f.a).cloned(), f.agents.get(f.b).cloned()) else { return };
        // Both command lines are built here already, so what the shell cannot take is told in the form.
        for agent in [&a, &b] {
            if let Err(e) = duel::launch_line(&agent.command, &task, &self.shell) {
                f.error = Some(e);
                return;
            }
        }
        let plan = Plan {
            project: f.project.clone(),
            name: f.name.clone(),
            data: self.paths.data.clone(),
            id: duel::new_id(),
            prompt: task,
            a: (a.agent.to_string(), a.command),
            b: (b.agent.to_string(), b.command),
        };
        let tx = self.tx.clone();
        let spawned = std::thread::Builder::new().name("duel-create".into()).spawn(move || {
            let result = duel::create(&plan);
            let _ = tx.send(AppEvent::Duel(DuelEvent::Created(result.map(Box::new))));
        });
        match spawned {
            Ok(_) => {
                f.busy = true;
                f.error = None;
            }
            Err(e) => f.error = Some(format!("could not start the git thread: {e}")),
        }
    }

    /// Opens the duel's tab: two panes side by side, each in its worktree running its agent with the task.
    fn open_duel_tab(&mut self, rec: &DuelRecord) -> bool {
        let mut lines = Vec::new();
        for side in Side::BOTH {
            match duel::launch_line(&rec.side(side).launcher, &rec.prompt, &self.shell) {
                Ok(line) => lines.push(self.shell.invocation(&line)),
                Err(e) => {
                    self.toast(ToastLevel::Error, format!("duel {}: {e}", rec.id));
                    return false;
                }
            }
        }
        let (a, b) = (&rec.a, &rec.b);
        for launcher in [&a.launcher, &b.launcher] {
            self.warn_full_quota(launcher);
        }
        let first = self.tabs.len();
        if !self.new_tab(PathBuf::from(&a.worktree), Some(&lines[0]), Some(rec.tab_name())) {
            return false;
        }
        let Some(tab) = self.tabs.last_mut() else { return false };
        tab.name = Some(rec.tab_name());
        let pane_a = tab.focus;
        // A restored pane runs the plain launcher again (and resumes its conversation): the task is not repeated.
        self.remember_launcher(pane_a, &a.launcher);
        match self.split_with_in(Dir::Row, Some(PathBuf::from(&b.worktree)), Some(&lines[1])) {
            Some(pane_b) => {
                self.remember_launcher(pane_b, &b.launcher);
                if let Some(tab) = self.tabs.last_mut() {
                    tab.focus = pane_a;
                }
            }
            None => {
                // No room for two panes: the second agent gets a tab of its own.
                let name = format!("{} · B", rec.tab_name());
                if self.new_tab(PathBuf::from(&b.worktree), Some(&lines[1]), Some(name.clone())) {
                    if let Some(tab) = self.tabs.last_mut() {
                        tab.name = Some(name);
                        let pane_b = tab.focus;
                        self.remember_launcher(pane_b, &b.launcher);
                    }
                    self.view = View::Term(first);
                    self.toast(ToastLevel::Info, "no room to split: the second agent is in its own tab");
                }
            }
        }
        self.note_agent_offer(&a.launcher);
        self.note_agent_offer(&b.launcher);
        true
    }

    fn remember_launcher(&mut self, pane: PaneId, launcher: &str) {
        if let Some(p) = self.panes.get_mut(&pane) {
            p.launcher = Some(launcher.to_string());
        }
    }

    // ─── Compare ─────────────────────────────────────────────────────────────────────────────────

    /// Opens the compare overlay for the current duel and asks for what each side did.
    pub fn open_compare(&mut self) {
        let Some(rec) = self.target_duel().cloned() else {
            return self.toast(ToastLevel::Info, "no duel here — Start Agent Duel… begins one");
        };
        self.spawn_compare(rec.clone());
        self.overlay = Some(Overlay::Duel(DuelView::Compare(Box::new(CompareState {
            rec,
            report: None,
            side: Side::A,
            scroll: [0, 0],
        }))));
    }

    fn spawn_compare(&mut self, rec: DuelRecord) {
        let tx = self.tx.clone();
        let spawned = std::thread::Builder::new().name("duel-compare".into()).spawn(move || {
            let report = duel::compare(&rec);
            let _ = tx.send(AppEvent::Duel(DuelEvent::Compared(rec.id.clone(), Box::new(report))));
        });
        if spawned.is_err() {
            self.toast(ToastLevel::Error, "could not start the git thread");
        }
    }

    // ─── End ─────────────────────────────────────────────────────────────────────────────────────

    /// Opens the end dialog for the current duel.
    pub fn request_end_duel(&mut self) {
        let Some(rec) = self.target_duel().cloned() else {
            return self.toast(ToastLevel::Info, "no duel here — Start Agent Duel… begins one");
        };
        self.overlay = Some(Overlay::Duel(DuelView::End(Box::new(EndState {
            rec,
            delete: DeleteBranches::None,
            step: EndStep::Ask,
            error: None,
        }))));
    }

    /// The first step of ending: ask git for uncommitted changes, with the panes still open.
    fn begin_end(&mut self, st: &mut EndState) {
        st.step = EndStep::Checking;
        st.error = None;
        let (tx, rec) = (self.tx.clone(), st.rec.clone());
        let spawned = std::thread::Builder::new().name("duel-inspect".into()).spawn(move || {
            let result = duel::dirty_counts(&rec);
            let _ = tx.send(AppEvent::Duel(DuelEvent::Inspected(rec.id.clone(), result)));
        });
        if spawned.is_err() {
            st.step = EndStep::Ask;
            st.error = Some("could not start the git thread".into());
        }
    }

    /// Closes the duel's panes (the agents stop; a shell in a removed folder would be left behind) and asks a
    /// background thread to remove the worktrees.
    fn remove_worktrees(&mut self, rec: &DuelRecord, force: bool, delete: DeleteBranches) {
        let inside: Vec<PaneId> =
            self.panes.iter().filter(|(_, p)| duel::side_of(rec, &p.cwd()).is_some()).map(|(id, _)| *id).collect();
        for id in inside {
            self.close_pane_as(id, false);
        }
        let (tx, rec) = (self.tx.clone(), rec.clone());
        let spawned = std::thread::Builder::new().name("duel-end".into()).spawn(move || {
            let report = duel::end(&rec, force, delete);
            let _ = tx.send(AppEvent::Duel(DuelEvent::Ended(Box::new(report), delete)));
        });
        if spawned.is_err() {
            self.toast(ToastLevel::Error, "could not start the git thread");
        }
    }

    // ─── Answers from the background threads ─────────────────────────────────────────────────────

    pub(super) fn on_duel_event(&mut self, ev: DuelEvent) {
        match ev {
            DuelEvent::Created(Ok(rec)) => {
                let rec = *rec;
                self.edit_ui_state(|d| d.duels.push(rec.clone()));
                if matches!(&self.overlay, Some(Overlay::Duel(DuelView::Form(_)))) {
                    self.overlay = None;
                }
                if self.open_duel_tab(&rec) {
                    let (a, b) = (&rec.a.agent, &rec.b.agent);
                    self.toast(ToastLevel::Ok, format!("duel {} · {a} vs {b} · {}", rec.id, rec.name));
                }
            }
            DuelEvent::Created(Err(e)) => {
                // In the form (still open) and in the notification list: why nothing started.
                self.toast(ToastLevel::Warn, format!("duel not started: {e}"));
                if let Some(Overlay::Duel(DuelView::Form(f))) = &mut self.overlay {
                    f.busy = false;
                    f.error = Some(e);
                }
            }
            DuelEvent::Compared(id, report) => {
                if let Some(Overlay::Duel(DuelView::Compare(st))) = &mut self.overlay
                    && st.rec.id == id
                {
                    st.report = Some(*report);
                }
            }
            DuelEvent::Inspected(id, result) => {
                let next = {
                    let Some(Overlay::Duel(DuelView::End(st))) = &mut self.overlay else { return };
                    if st.rec.id != id || st.step != EndStep::Checking {
                        return;
                    }
                    match result {
                        Ok((0, 0)) => {
                            st.step = EndStep::Removing;
                            Some((st.rec.clone(), st.delete))
                        }
                        Ok((a, b)) => {
                            st.step = EndStep::Dirty(a, b);
                            None
                        }
                        Err(e) => {
                            st.step = EndStep::Ask;
                            st.error = Some(e);
                            None
                        }
                    }
                };
                if let Some((rec, delete)) = next {
                    self.remove_worktrees(&rec, false, delete);
                }
            }
            DuelEvent::Ended(report, delete) => self.on_duel_ended(&report, delete),
        }
    }

    fn on_duel_ended(&mut self, report: &EndReport, delete: DeleteBranches) {
        if matches!(&self.overlay, Some(Overlay::Duel(DuelView::End(st))) if st.rec.id == report.id) {
            self.overlay = None;
        }
        let Some(rec) = self.duels().iter().find(|d| d.id == report.id).cloned() else { return };
        if report.done() {
            self.edit_ui_state(|d| d.duels.retain(|r| r.id != report.id));
        }
        let mut problems = Vec::new();
        for side in Side::BOTH {
            if let Some(e) = &report.side(side).error {
                problems.push(format!("{}: {e}", side.letter().to_uppercase()));
            }
        }
        if !problems.is_empty() {
            let text = format!("duel {} not fully ended — {}", rec.id, problems.join("; "));
            return self.toast(ToastLevel::Error, text);
        }
        let kept: Vec<String> =
            Side::BOTH.into_iter().filter(|s| !delete.has(*s)).map(|s| rec.side(s).branch.clone()).collect();
        let text = if kept.is_empty() {
            format!("duel {} ended · worktrees and branches removed", rec.id)
        } else {
            format!("duel {} ended · worktrees removed, branches kept: {}", rec.id, kept.join(", "))
        };
        self.toast(ToastLevel::Ok, text);
    }

    // ─── Keys, paste and clicks ──────────────────────────────────────────────────────────────────

    /// A key in a duel overlay; `false` closes it.
    pub(super) fn duel_key(&mut self, view: &mut DuelView, k: KeyEvent) -> bool {
        match view {
            DuelView::Form(f) => self.form_key(f, k),
            DuelView::Compare(st) => self.compare_key(st, k),
            DuelView::End(st) => self.end_key(st, k),
        }
    }

    fn form_key(&mut self, f: &mut DuelForm, k: KeyEvent) -> bool {
        let (ctrl, alt) = (ctrl_held(&k), alt_held(&k));
        if k.code == KeyCode::Esc {
            return false;
        }
        if f.busy {
            return true;
        }
        let in_task = f.field == DuelField::Task;
        let newline = in_task
            && match k.code {
                KeyCode::Enter => k.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT),
                KeyCode::Char('j') => ctrl,
                _ => false,
            };
        if newline {
            f.insert("\n");
            return true;
        }
        match k.code {
            KeyCode::Enter => self.submit_duel_form(f),
            KeyCode::Tab => f.step_field(1),
            KeyCode::BackTab => f.step_field(-1),
            KeyCode::Up if in_task => {
                if !f.move_line(false) {
                    f.step_field(-1);
                }
            }
            KeyCode::Down if in_task => {
                f.move_line(true);
            }
            KeyCode::Up => f.step_field(-1),
            KeyCode::Down => f.step_field(1),
            KeyCode::Left if in_task => f.move_to(f.cursor.saturating_sub(1)),
            KeyCode::Right if in_task => f.move_to(f.cursor + 1),
            KeyCode::Home if in_task => f.line_start(),
            KeyCode::End if in_task => f.line_end(),
            KeyCode::Backspace if in_task => f.backspace(),
            KeyCode::Delete if in_task => f.delete(),
            KeyCode::Char('a') if in_task && ctrl => f.line_start(),
            KeyCode::Char('e') if in_task && ctrl => f.line_end(),
            KeyCode::Char('u') if in_task && ctrl => {
                f.task.clear();
                f.cursor = 0;
            }
            KeyCode::Char(c) if in_task && !ctrl && !alt => f.insert(c.encode_utf8(&mut [0; 4])),
            // On an agent row ←→ (or h/l, space) step through the agents.
            KeyCode::Left | KeyCode::Char('h') => self.form_cycle(f, -1),
            KeyCode::Right | KeyCode::Char('l' | ' ') => self.form_cycle(f, 1),
            _ => {}
        }
        true
    }

    fn form_cycle(&mut self, f: &mut DuelForm, delta: isize) {
        match f.field {
            DuelField::A => f.cycle(Side::A, delta),
            DuelField::B => f.cycle(Side::B, delta),
            DuelField::Task => {}
        }
    }

    fn compare_key(&mut self, st: &mut CompareState, k: KeyEvent) -> bool {
        let rows = st.report.as_ref().map_or(0, |r| r.side(st.side).files.len());
        let slot = usize::from(st.side == Side::B);
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => return false,
            KeyCode::Left | KeyCode::Char('h') => st.side = Side::A,
            KeyCode::Right | KeyCode::Char('l') => st.side = Side::B,
            KeyCode::Tab | KeyCode::BackTab => st.side = st.side.other(),
            KeyCode::Up | KeyCode::Char('k') => st.scroll[slot] = st.scroll[slot].saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => st.scroll[slot] = (st.scroll[slot] + 1).min(rows.saturating_sub(1)),
            KeyCode::PageUp => st.scroll[slot] = st.scroll[slot].saturating_sub(8),
            KeyCode::PageDown => st.scroll[slot] = (st.scroll[slot] + 8).min(rows.saturating_sub(1)),
            KeyCode::Home => st.scroll[slot] = 0,
            KeyCode::End => st.scroll[slot] = rows.saturating_sub(1),
            KeyCode::Char('c') => self.copy_duel_commands(st),
            KeyCode::Char('r') => {
                st.report = None;
                self.spawn_compare(st.rec.clone());
            }
            _ => {}
        }
        true
    }

    /// Copies the commands that inspect and merge the focused side to the clipboard.
    fn copy_duel_commands(&mut self, st: &CompareState) {
        let text = duel::side_commands(&st.rec, st.side).join("\n");
        self.copy_notice(&text);
    }

    fn end_key(&mut self, st: &mut EndState, k: KeyEvent) -> bool {
        if k.code == KeyCode::Esc || matches!(k.code, KeyCode::Char('n' | 'N')) && st.step != EndStep::Removing {
            return false;
        }
        match (st.step, k.code) {
            (EndStep::Ask, KeyCode::Char('y' | 'Y')) => self.begin_end(st),
            (EndStep::Ask, KeyCode::Up | KeyCode::Char('k')) => st.delete = step_delete(st.delete, -1),
            (EndStep::Ask, KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab) => st.delete = step_delete(st.delete, 1),
            (EndStep::Ask, KeyCode::Char(c @ '1'..='4')) => {
                st.delete = DeleteBranches::ALL[(c as usize) - ('1' as usize)];
            }
            // The only way to discard uncommitted work is this explicit key (not ⏎, not y).
            (EndStep::Dirty(..), KeyCode::Char('f' | 'F')) => {
                st.step = EndStep::Removing;
                let (rec, delete) = (st.rec.clone(), st.delete);
                self.remove_worktrees(&rec, true, delete);
            }
            _ => {}
        }
        true
    }

    pub(super) fn duel_paste(&mut self, text: &str) {
        if let Some(Overlay::Duel(DuelView::Form(f))) = &mut self.overlay
            && f.field == DuelField::Task
            && !f.busy
        {
            f.insert(text);
        }
    }

    pub(super) fn duel_click(&mut self, hit: DuelHit) {
        // A stale target (the overlay changed since the frame): nothing to do, and no other overlay is dropped.
        if !matches!(self.overlay, Some(Overlay::Duel(_))) {
            return;
        }
        let Some(Overlay::Duel(mut view)) = self.overlay.take() else { return };
        let keep = match (&mut view, hit) {
            (_, DuelHit::Close) => false,
            (DuelView::Form(f), DuelHit::Pick(side, i)) if !f.busy => {
                if i < f.agents.len() {
                    match side {
                        Side::A => f.a = i,
                        Side::B => f.b = i,
                    }
                    f.field = if side == Side::A { DuelField::A } else { DuelField::B };
                    f.error = None;
                }
                true
            }
            (DuelView::Form(f), DuelHit::Field(field)) if !f.busy => {
                f.field = field;
                true
            }
            (DuelView::Form(f), DuelHit::Start) => {
                self.submit_duel_form(f);
                true
            }
            (DuelView::Compare(st), DuelHit::Side(side)) => {
                st.side = side;
                true
            }
            (DuelView::Compare(st), DuelHit::Copy) => {
                self.copy_duel_commands(st);
                true
            }
            (DuelView::Compare(st), DuelHit::Refresh) => {
                st.report = None;
                self.spawn_compare(st.rec.clone());
                true
            }
            (DuelView::End(st), DuelHit::Delete(choice)) if st.step == EndStep::Ask => {
                st.delete = choice;
                true
            }
            (DuelView::End(st), DuelHit::Confirm) if st.step == EndStep::Ask => {
                self.begin_end(st);
                true
            }
            (DuelView::End(st), DuelHit::Force) if matches!(st.step, EndStep::Dirty(..)) => {
                st.step = EndStep::Removing;
                let (rec, delete) = (st.rec.clone(), st.delete);
                self.remove_worktrees(&rec, true, delete);
                true
            }
            _ => true,
        };
        if keep && self.overlay.is_none() {
            self.overlay = Some(Overlay::Duel(view));
        }
    }
}

/// The next or previous branch choice, staying within the list.
fn step_delete(current: DeleteBranches, delta: i32) -> DeleteBranches {
    let at = DeleteBranches::ALL.iter().position(|c| *c == current).unwrap_or(0) as i32;
    DeleteBranches::ALL[(at + delta).clamp(0, 3) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(text: &str) -> DuelForm {
        let agent = DuelAgent { name: "claude".into(), command: "claude".into(), agent: "claude" };
        let mut f = DuelForm {
            project: PathBuf::from("/p"),
            name: "p".into(),
            from: String::new(),
            agents: vec![agent],
            a: 0,
            b: 0,
            task: String::new(),
            cursor: 0,
            field: DuelField::Task,
            error: None,
            busy: false,
        };
        f.insert(text);
        f
    }

    #[test]
    fn typing_keeps_line_breaks_and_drops_control_characters() {
        let mut f = form("one\r\ntwo\u{1b}\tthree");
        assert_eq!(f.task, "one\ntwo three");
        assert_eq!(f.cursor, 13);
        f.insert(&"x".repeat(duel::MAX_PROMPT * 2));
        assert_eq!(f.task.chars().count(), duel::MAX_PROMPT, "the task has a limit");
    }

    #[test]
    fn the_cursor_moves_by_lines_and_keeps_its_column() {
        let mut f = form("first line\nab\nthird line");
        // At the end of "third line" (column 10): up lands on the end of the shorter "ab", then on "first line".
        assert!(f.move_line(false));
        assert_eq!(&f.task[..f.byte_at(f.cursor)], "first line\nab");
        assert!(f.move_line(false));
        assert_eq!(f.cursor, 2, "column 2 of the first line");
        assert!(!f.move_line(false), "no line above the first");
        f.line_end();
        assert_eq!(f.cursor, 10);
        f.line_start();
        assert_eq!(f.cursor, 0);
        f.move_to(usize::MAX);
        assert!(!f.move_line(true), "no line below the last");
        f.backspace();
        f.delete();
        assert_eq!(f.task, "first line\nab\nthird lin");
    }

    #[test]
    fn rows_wrap_at_the_width_and_at_line_breaks() {
        let f = form("abcdefgh\n\nxy");
        let rows = f.rows(5);
        let texts: Vec<String> = rows.iter().map(|r| f.task.chars().skip(r.start).take(r.len()).collect()).collect();
        assert_eq!(texts, ["abcde", "fgh", "", "xy"]);
        // The cursor at the end is on the last row; at a wrap boundary it is on the row that starts there.
        assert_eq!(f.cursor_row(&rows), 3);
        let mut g = form("abcdefgh");
        g.move_to(5);
        assert_eq!(g.cursor_row(&g.rows(5)), 1);
        assert_eq!(form("").rows(10), vec![0..0]);
        // A wide character that does not fit moves to the next row whole.
        assert_eq!(form("ab日").rows(3), vec![0..2, 2..3]);
    }

    #[test]
    fn the_branch_choice_steps_within_its_list() {
        assert_eq!(step_delete(DeleteBranches::None, -1), DeleteBranches::None);
        assert_eq!(step_delete(DeleteBranches::None, 1), DeleteBranches::A);
        assert_eq!(step_delete(DeleteBranches::Both, 1), DeleteBranches::Both);
    }
}
