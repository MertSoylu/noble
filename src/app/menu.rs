//! Right-click menus (tab, pane title, project) and mouse project actions.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent};

use super::{App, Overlay, Prompt, PromptPurpose, ToastLevel, View};
use crate::term::TabId;
use crate::term::layout::{Dir, PaneId};

#[derive(Clone, Debug, PartialEq)]
pub enum MenuCmd {
    Paste(PaneId),
    Split(PaneId, Dir),
    /// Launcher index run in a new split next to the pane.
    LaunchSplit(PaneId, usize),
    /// Opens the list of launchers that can run in a split next to the pane (`LaunchSplit`).
    LaunchMenu(PaneId),
    Zoom(PaneId),
    Search(PaneId),
    Passthrough(PaneId),
    ClosePane(PaneId),
    CopyText(String),
    OpenFolder(PathBuf),
    OpenCode(PathBuf),
    // Tabs are named by their stable id, not by index or pane: a tab can close (its shell exits)
    // while the menu is open, which would shift every index after it, and in a split tab the pane
    // the menu was opened from can close while the tab lives on.
    RenameTab(TabId),
    CloseTab(TabId),
    MoveTab(TabId, i32),
    OpenProject(PathBuf),
    Launch(usize, PathBuf),
    /// Opens the list of launchers that start in the project's folder (`Launch`), where the project menu is.
    StartMenu(PathBuf),
    /// The start form of an agent duel in this project.
    StartDuel(PathBuf),
    GitPull(PathBuf),
    TogglePin(PathBuf),
    RemoveProject(PathBuf),
    OpenLink(crate::term::link::Link),
    /// An action run on a pane (the pane is focused first): the command output actions of its menu.
    PaneAction(PaneId, crate::keys::Action),
    /// Opens the list of ways to rearrange the panes around this one (swaps, edges, rotation, flip).
    ArrangeMenu(PaneId),
    /// Opens the list of tabs the pane can move into (and a new one).
    MoveToTabMenu(PaneId),
    /// Moves the pane into the tab with this id (`App::move_pane_into_tab`).
    MoveToTab(PaneId, TabId),
}

#[derive(Clone, Debug)]
pub struct MenuItem {
    pub label: String,
    pub hint: String,
    pub cmd: MenuCmd,
    pub enabled: bool,
    /// A thin rule is drawn above the item (a rule above the menu's first item is not drawn).
    pub sep: bool,
    /// The label is drawn in the critical color: the command closes or removes something.
    pub danger: bool,
}

impl MenuItem {
    /// Draws a rule above this item.
    pub fn sep(mut self) -> Self {
        self.sep = true;
        self
    }

    /// Draws the label in the critical color.
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
}

/// An open right-click menu, drawn next to the clicked point.
pub struct Menu {
    pub title: String,
    pub x: u16,
    pub y: u16,
    pub items: Vec<MenuItem>,
    pub selected: usize,
}

/// Quick actions shown when a project row is hovered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectAct {
    Pin,
    More,
}

fn item(label: &str, hint: &str, cmd: MenuCmd) -> MenuItem {
    MenuItem { label: label.into(), hint: hint.into(), cmd, enabled: true, sep: false, danger: false }
}

/// The project menu lists this many quick launchers inline; more of them go behind one "Start agent ›" row.
const INLINE_LAUNCHERS: usize = 3;

impl App {
    pub(super) fn open_menu(&mut self, title: String, x: u16, y: u16, items: Vec<MenuItem>) {
        self.overlay = Some(Overlay::Menu(Menu { title, x, y, items, selected: 0 }));
    }

    pub fn open_tab_menu(&mut self, i: usize, x: u16, y: u16) {
        if i >= self.tabs.len() {
            return;
        }
        let hint = |a| self.keymap.hint(a).unwrap_or_default();
        let key = self.tabs[i].id;
        let mut items = vec![item("Rename…", &hint(crate::keys::Action::RenameTab), MenuCmd::RenameTab(key))];
        if i > 0 {
            items.push(item("Move left", &hint(crate::keys::Action::MoveTabLeft), MenuCmd::MoveTab(key, -1)));
        }
        if i + 1 < self.tabs.len() {
            items.push(item("Move right", &hint(crate::keys::Action::MoveTabRight), MenuCmd::MoveTab(key, 1)));
        }
        if let Some(cwd) = self.panes.get(&self.tabs[i].focus).map(|p| p.cwd()) {
            items.push(item("Copy path", "", MenuCmd::CopyText(cwd.display().to_string())));
            items.push(item("Open folder", "", MenuCmd::OpenFolder(cwd)));
        }
        // A tab of an agent duel: compare the two sides, or end the duel.
        if self.duel_of_tab(i).is_some() {
            let pane = self.tabs[i].focus;
            let act = |label: &str, a| item(label, &hint(a), MenuCmd::PaneAction(pane, a));
            items.push(act("Compare duel", crate::keys::Action::CompareDuel).sep());
            items.push(act("End duel…", crate::keys::Action::EndDuel));
        }
        items.push(item("Close tab", &hint(crate::keys::Action::CloseTab), MenuCmd::CloseTab(key)).sep().danger());
        self.open_menu(self.tab_title(i), x, y, items);
    }

    pub fn open_pane_menu(&mut self, pane: PaneId, x: u16, y: u16) {
        let Some(p) = self.panes.get(&pane) else { return };
        let cwd = p.cwd();
        let label = p.label();
        let pass_label = match (self.pass_once(), p.passthrough) {
            (true, _) => "Send next key to the app",
            (false, false) => "Pass keys to the app",
            (false, true) => "Unlock keys",
        };
        let hint = |a| self.keymap.hint(a).unwrap_or_default();
        use crate::keys::Action as A;
        let mut items = Vec::new();
        // Copy is offered while there is a selection (without copy_on_select there is no other way).
        if let Some(text) = p.selected_text() {
            items.push(item("Copy", "", MenuCmd::CopyText(text)));
        }
        items.extend([
            item("Paste", "right-click", MenuCmd::Paste(pane)),
            item("Search scrollback", &hint(A::Search), MenuCmd::Search(pane)),
            item("Split right", &hint(A::SplitRight), MenuCmd::Split(pane, Dir::Row)),
            item("Split down", &hint(A::SplitDown), MenuCmd::Split(pane, Dir::Col)),
            item("Zoom / restore", &hint(A::Zoom), MenuCmd::Zoom(pane)),
        ]);
        // Arranging needs a second pane in the tab; moving into another tab needs another tab or a second pane.
        let multi = self.tabs.iter().any(|t| t.root.contains(pane) && t.panes().len() > 1);
        if multi {
            items.extend([
                item("Equalize splits", &hint(A::EqualizePanes), MenuCmd::PaneAction(pane, A::EqualizePanes)),
                item("Arrange panes ›", "drag title", MenuCmd::ArrangeMenu(pane)),
            ]);
        }
        if multi || self.tabs.len() > 1 {
            items.push(item("Move to tab ›", "", MenuCmd::MoveToTabMenu(pane)));
        }
        // The commands the shell marked in this pane: jumps, copy, and the send to an agent (see `app/blocks.rs`).
        if p.last_output_block().is_some() {
            items.extend([
                item("Jump to previous command", &hint(A::PrevCommand), MenuCmd::PaneAction(pane, A::PrevCommand)),
                item("Jump to next command", &hint(A::NextCommand), MenuCmd::PaneAction(pane, A::NextCommand)),
                item("Copy last output", &hint(A::CopyLastOutput), MenuCmd::PaneAction(pane, A::CopyLastOutput)),
            ]);
            if self.agent_target(pane).is_some() {
                let send = MenuCmd::PaneAction(pane, A::SendLastOutput);
                items.push(item("Send last output to agent", &hint(A::SendLastOutput), send));
            }
        }
        // Quick launchers run next to this pane, in its directory: one row opens their list.
        if self.quick_launchers().next().is_some() {
            items.push(item("Launch in split ›", "", MenuCmd::LaunchMenu(pane)));
        }
        // A pane of an agent duel: compare the two sides, or end the duel.
        if self.tab_of(pane).is_some_and(|ti| self.duel_of_tab(ti).is_some()) {
            items.push(item("Compare duel", &hint(A::CompareDuel), MenuCmd::PaneAction(pane, A::CompareDuel)));
            items.push(item("End duel…", &hint(A::EndDuel), MenuCmd::PaneAction(pane, A::EndDuel)));
        }
        items.extend([
            item(pass_label, &hint(A::Passthrough), MenuCmd::Passthrough(pane)),
            item("Copy path", "", MenuCmd::CopyText(cwd.display().to_string())),
            item("Open folder", "", MenuCmd::OpenFolder(cwd.clone())),
            item("Open in VS Code", "", MenuCmd::OpenCode(cwd)),
            item("Close pane", &hint(A::ClosePane), MenuCmd::ClosePane(pane)).sep().danger(),
        ]);
        self.open_menu(label, x, y, items);
    }

    /// The ways to rearrange the panes around `pane`, at the same place: a swap with each neighbour it has, with the next
    /// and previous pane, a move along each edge of the tab, rotation and turning its split the other way.
    pub fn open_arrange_menu(&mut self, pane: PaneId, x: u16, y: u16) {
        use crate::keys::Action as A;
        use crate::term::layout::{Direction, neighbor};
        let body = self.body();
        let Some(ti) = self.tab_of(pane) else { return };
        let rects = self.tabs[ti].root.layout(body).0;
        let hint = |a| self.keymap.hint(a).unwrap_or_default();
        let act = |label: &str, a: A| item(label, &hint(a), MenuCmd::PaneAction(pane, a));
        let mut items = Vec::new();
        for (d, label, a) in [
            (Direction::Left, "Swap with pane on the left", A::SwapLeft),
            (Direction::Right, "Swap with pane on the right", A::SwapRight),
            (Direction::Up, "Swap with pane above", A::SwapUp),
            (Direction::Down, "Swap with pane below", A::SwapDown),
        ] {
            if neighbor(&rects, pane, d).is_some() {
                items.push(act(label, a));
            }
        }
        items.extend([act("Swap with next pane", A::SwapPane), act("Swap with previous pane", A::SwapPanePrev)]);
        items.extend([
            act("Move to left edge", A::MoveEdgeLeft).sep(),
            act("Move to right edge", A::MoveEdgeRight),
            act("Move to top edge", A::MoveEdgeUp),
            act("Move to bottom edge", A::MoveEdgeDown),
            act("Rotate panes forward", A::RotatePanes).sep(),
            act("Rotate panes backward", A::RotatePanesBack),
            act("Flip split (side by side ↔ stacked)", A::FlipSplit),
        ]);
        self.open_menu("Arrange panes".into(), x, y, items);
    }

    /// The tabs `pane` can move into, at the same place: a new tab of its own (when its tab keeps another pane), then
    /// every other tab by its title.
    pub fn open_move_to_tab_menu(&mut self, pane: PaneId, x: u16, y: u16) {
        let Some(ti) = self.tab_of(pane) else { return };
        let mut items = Vec::new();
        if self.tabs[ti].panes().len() > 1 {
            let hint = self.keymap.hint(crate::keys::Action::MovePaneToTab).unwrap_or_default();
            items.push(item("New tab", &hint, MenuCmd::PaneAction(pane, crate::keys::Action::MovePaneToTab)));
        }
        // A rule between the new tab and the existing ones.
        let rule = !items.is_empty();
        for (n, (i, t)) in self.tabs.iter().enumerate().filter(|(i, _)| *i != ti).enumerate() {
            let it = item(&format!("{}  {}", i + 1, self.tab_title(i)), "", MenuCmd::MoveToTab(pane, t.id));
            items.push(if n == 0 && rule { it.sep() } else { it });
        }
        self.open_menu("Move to tab".into(), x, y, items);
    }

    /// The launchers a pane menu offers to run in a split next to the pane, at the same place.
    pub fn open_launch_menu(&mut self, pane: PaneId, x: u16, y: u16) {
        if !self.panes.contains_key(&pane) {
            return;
        }
        let items: Vec<MenuItem> =
            self.quick_launchers().map(|(i, l)| item(&l.name, "", MenuCmd::LaunchSplit(pane, i))).collect();
        self.open_menu("Launch in split".into(), x, y, items);
    }

    /// The launchers a project menu offers to start in the project's folder, at the same place: a row each
    /// (`Launch`), named by the launcher.
    pub fn open_start_menu(&mut self, path: PathBuf, x: u16, y: u16) {
        let items: Vec<MenuItem> =
            self.quick_launchers().map(|(i, l)| item(&l.name, &l.key, MenuCmd::Launch(i, path.clone()))).collect();
        self.open_menu("Start agent".into(), x, y, items);
    }

    /// Project rows: open or start in the folder, then the folder's own actions, Git pull for repos, and the
    /// pin and the removal from the list (see `MenuCmd::RemoveProject`).
    pub fn open_project_menu(&mut self, path: PathBuf, x: u16, y: u16) {
        let Some(p) = self.projects.iter().find(|p| p.path == path) else { return };
        let (name, repo) = (p.name.clone(), p.repo);
        let mut items = vec![item("Open terminal", "⏎", MenuCmd::OpenProject(path.clone()))];
        if self.quick_launchers().count() > INLINE_LAUNCHERS {
            items.push(item("Start agent ›", "", MenuCmd::StartMenu(path.clone())));
        } else {
            for (i, l) in self.quick_launchers() {
                items.push(item(&format!("Start {}", l.name), &l.key, MenuCmd::Launch(i, path.clone())));
            }
        }
        items.push(item("Open in VS Code", "", MenuCmd::OpenCode(path.clone())).sep());
        items.push(item("Open folder", "o", MenuCmd::OpenFolder(path.clone())));
        items.push(item("Copy path", "", MenuCmd::CopyText(path.display().to_string())));
        if repo {
            items.push(item("Git pull", "", MenuCmd::GitPull(path.clone())).sep());
            // Two agents, one task, two worktrees (`app/duel.rs`); off while no installed agent can take a task.
            let mut duel = item("Start agent duel…", "", MenuCmd::StartDuel(path.clone()));
            duel.enabled = !self.duel_agents().is_empty();
            items.push(duel);
        }
        let pin = if self.ui_state.is_pinned(&path) { "Unpin" } else { "Pin to top" };
        items.push(item(pin, "", MenuCmd::TogglePin(path.clone())).sep());
        // Hides it from the list for good (the folder is not touched); a adds it back.
        items.push(item("Remove from list", "", MenuCmd::RemoveProject(path)).danger());
        self.open_menu(name, x, y, items);
    }

    pub fn run_menu(&mut self, cmd: MenuCmd) {
        // A sub-list opens where its menu was.
        let at = match &self.overlay {
            Some(Overlay::Menu(m)) => (m.x, m.y),
            _ => (0, 0),
        };
        self.overlay = None;
        match cmd {
            MenuCmd::Paste(pane) => self.paste_clipboard(pane),
            MenuCmd::Split(pane, dir) => {
                if self.focus_pane(pane) {
                    self.split(dir);
                }
            }
            MenuCmd::LaunchSplit(pane, i) => {
                if self.focus_pane(pane) {
                    self.launch_in_split(i);
                }
            }
            MenuCmd::LaunchMenu(pane) => self.open_launch_menu(pane, at.0, at.1),
            MenuCmd::ArrangeMenu(pane) => self.open_arrange_menu(pane, at.0, at.1),
            MenuCmd::MoveToTabMenu(pane) => self.open_move_to_tab_menu(pane, at.0, at.1),
            MenuCmd::MoveToTab(pane, tab) => self.move_pane_into_tab(pane, tab),
            MenuCmd::Zoom(pane) => {
                if let Some(ti) = self.tabs.iter().position(|t| t.root.contains(pane)) {
                    self.toggle_zoom(ti, pane);
                }
            }
            MenuCmd::Search(pane) => {
                if self.focus_pane(pane) {
                    self.open_search();
                }
            }
            MenuCmd::Passthrough(pane) => {
                if self.focus_pane(pane) {
                    self.run(crate::keys::Action::Passthrough);
                }
            }
            MenuCmd::ClosePane(pane) => self.request_close_pane(pane),
            MenuCmd::CopyText(text) => self.set_clipboard(&text, true),
            MenuCmd::OpenFolder(path) => self.open_in_file_manager(&path),
            MenuCmd::OpenCode(path) => self.open_in_code(&path),
            // The tab may be gone by now (its shell exited while the menu was open): nothing to do then.
            MenuCmd::RenameTab(tab) => {
                if let Some(i) = self.tab_index(tab) {
                    self.rename_tab_prompt(i);
                }
            }
            MenuCmd::CloseTab(tab) => {
                if let Some(i) = self.tab_index(tab) {
                    self.request_close_tab(i);
                }
            }
            MenuCmd::MoveTab(tab, d) => {
                if let Some(i) = self.tab_index(tab) {
                    let to = i.saturating_add_signed(d as isize).min(self.tabs.len() - 1);
                    self.move_tab(i, to);
                }
            }
            MenuCmd::OpenProject(path) => self.open_project_shell(path),
            MenuCmd::Launch(i, path) => self.launch(i, Some(path)),
            MenuCmd::StartMenu(path) => self.open_start_menu(path, at.0, at.1),
            MenuCmd::StartDuel(path) => self.open_duel_form(Some(path)),
            MenuCmd::GitPull(path) => self.git_pull(&path),
            MenuCmd::TogglePin(path) => self.toggle_pin(&path),
            MenuCmd::RemoveProject(path) => self.request_remove_project(path),
            MenuCmd::OpenLink(target) => self.open_link(target),
            MenuCmd::PaneAction(pane, action) => {
                if self.focus_pane(pane) {
                    self.run(action);
                }
            }
        }
    }

    pub(super) fn menu_key(&mut self, k: KeyEvent) {
        let Some(Overlay::Menu(m)) = &mut self.overlay else { return };
        let n = m.items.len();
        match k.code {
            KeyCode::Esc => self.overlay = None,
            KeyCode::Up | KeyCode::Char('k') => m.selected = (m.selected + n.max(1) - 1) % n.max(1),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => m.selected = (m.selected + 1) % n.max(1),
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(it) = m.items.get(m.selected).filter(|it| it.enabled).cloned() {
                    self.run_menu(it.cmd);
                }
            }
            // An item numbered in its label (the links menu) runs by its digit.
            KeyCode::Char(c @ '1'..='9') => {
                let prefix = format!("{c} ");
                if let Some(it) = m.items.iter().find(|it| it.enabled && it.label.starts_with(&prefix)).cloned() {
                    self.run_menu(it.cmd);
                }
            }
            _ => {}
        }
    }

    /// Switches to the pane's tab and focuses it.
    pub(super) fn focus_pane(&mut self, pane: PaneId) -> bool {
        let Some(ti) = self.tabs.iter().position(|t| t.root.contains(pane)) else { return false };
        self.tabs[ti].focus = pane;
        self.view = View::Term(ti);
        true
    }

    pub fn rename_tab_prompt(&mut self, i: usize) {
        let Some(tab) = self.tabs.get(i) else { return };
        let (value, key) = (tab.name.clone().unwrap_or_default(), tab.id);
        self.overlay = Some(Overlay::Prompt(Prompt::new("RENAME TAB", value, PromptPurpose::RenameTab(key))));
    }

    /// Moves a tab from order `from` to order `to`; the visible tab follows.
    pub fn move_tab(&mut self, from: usize, to: usize) {
        if from >= self.tabs.len() || to >= self.tabs.len() || from == to {
            return;
        }
        let current = match self.view {
            View::Term(i) => Some(i),
            _ => None,
        };
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        if let Some(c) = current {
            let moved = if c == from {
                to
            } else if from < c && c <= to {
                c - 1
            } else if to <= c && c < from {
                c + 1
            } else {
                c
            };
            self.view = View::Term(moved);
        }
    }

    pub fn open_in_code(&mut self, path: &Path) {
        let Some(code) = crate::util::which("code") else {
            self.toast(ToastLevel::Warn, "VS Code (code) is not on PATH");
            return;
        };
        let mut cmd = crate::util::command_for(&code);
        cmd.arg(path);
        match crate::util::spawn_detached(&mut cmd) {
            Ok(_) => self.toast(ToastLevel::Info, format!("opening {} in VS Code", crate::util::tilde(path))),
            Err(_) => self.toast(ToastLevel::Error, "could not start VS Code"),
        }
    }

    /// Runs `git pull` in a new tab (so the output stays visible).
    pub fn git_pull(&mut self, path: &Path) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.new_tab(path.to_path_buf(), Some("git pull"), Some(format!("{name} · pull")));
    }

    pub fn toggle_pin(&mut self, path: &Path) {
        let pinned = self.ui_state.toggle_pin(path);
        // The write read back other windows' changes, hidden/added projects included.
        self.sync_manual_projects();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.sort_pinned();
        self.toast(ToastLevel::Ok, if pinned { format!("pinned {name}") } else { format!("unpinned {name}") });
    }

    /// Moves pinned projects (keeping their own order) to the top; the selection is kept.
    pub(super) fn sort_pinned(&mut self) {
        let selected = self.selected_project().map(|p| p.path.clone());
        let pins: Vec<bool> = self.projects.iter().map(|p| self.ui_state.is_pinned(&p.path)).collect();
        let mut indexed: Vec<(bool, crate::projects::Project)> =
            pins.into_iter().zip(self.projects.drain(..)).collect();
        indexed.sort_by_key(|(pinned, _)| !*pinned);
        self.projects = indexed.into_iter().map(|(_, p)| p).collect();
        if let Some(path) = selected
            && let Some(pos) = self.visible_projects().iter().position(|i| self.projects[*i].path == path)
        {
            self.bridge.proj_sel = pos;
        }
    }

    /// A quick-action button on a project row.
    pub fn project_action(&mut self, row: usize, act: ProjectAct, x: u16, y: u16) {
        let Some(path) = self.visible_projects().get(row).map(|i| self.projects[*i].path.clone()) else { return };
        self.bridge.proj_sel = row;
        match act {
            ProjectAct::Pin => self.toggle_pin(&path),
            ProjectAct::More => self.open_project_menu(path, x, y),
        }
    }
}
