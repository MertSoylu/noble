//! Right-click menus (tab, pane title, project) and mouse project actions.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent};

use super::{App, Overlay, Prompt, PromptPurpose, ToastLevel, View};
use crate::term::layout::{Dir, PaneId};

#[derive(Clone, Debug, PartialEq)]
pub enum MenuCmd {
    Paste(PaneId),
    Split(PaneId, Dir),
    Zoom(PaneId),
    Search(PaneId),
    Passthrough(PaneId),
    ClosePane(PaneId),
    CopyText(String),
    OpenFolder(PathBuf),
    OpenCode(PathBuf),
    RenameTab(usize),
    CloseTab(usize),
    MoveTab(usize, i32),
    OpenProject(PathBuf),
    Launch(usize, PathBuf),
    GitPull(PathBuf),
    TogglePin(PathBuf),
}

#[derive(Clone, Debug)]
pub struct MenuItem {
    pub label: String,
    pub hint: String,
    pub cmd: MenuCmd,
    pub enabled: bool,
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
    MenuItem { label: label.into(), hint: hint.into(), cmd, enabled: true }
}

impl App {
    fn open_menu(&mut self, title: String, x: u16, y: u16, items: Vec<MenuItem>) {
        self.overlay = Some(Overlay::Menu(Menu { title, x, y, items, selected: 0 }));
    }

    pub fn open_tab_menu(&mut self, i: usize, x: u16, y: u16) {
        if i >= self.tabs.len() {
            return;
        }
        let hint = |a| self.keymap.hint(a).unwrap_or_default();
        let mut items = vec![item("Rename…", &hint(crate::keys::Action::RenameTab), MenuCmd::RenameTab(i))];
        if i > 0 {
            items.push(item("Move left", &hint(crate::keys::Action::MoveTabLeft), MenuCmd::MoveTab(i, -1)));
        }
        if i + 1 < self.tabs.len() {
            items.push(item("Move right", &hint(crate::keys::Action::MoveTabRight), MenuCmd::MoveTab(i, 1)));
        }
        if let Some(cwd) = self.panes.get(&self.tabs[i].focus).map(|p| p.cwd()) {
            items.push(item("Copy path", "", MenuCmd::CopyText(cwd.display().to_string())));
            items.push(item("Open folder", "", MenuCmd::OpenFolder(cwd)));
        }
        items.push(item("Close tab", &hint(crate::keys::Action::CloseTab), MenuCmd::CloseTab(i)));
        self.open_menu(self.tab_title(i), x, y, items);
    }

    pub fn open_pane_menu(&mut self, pane: PaneId, x: u16, y: u16) {
        let Some(p) = self.panes.get(&pane) else { return };
        let cwd = p.cwd();
        let label = p.label();
        let pass_label = match (self.pass_once(), p.passthrough) {
            (true, _) => "Send next key to the app",
            (false, false) => "Lock keys to the app",
            (false, true) => "Unlock keys",
        };
        let hint = |a| self.keymap.hint(a).unwrap_or_default();
        use crate::keys::Action as A;
        let items = vec![
            item("Paste", "right-click", MenuCmd::Paste(pane)),
            item("Search scrollback", &hint(A::Search), MenuCmd::Search(pane)),
            item("Split right", &hint(A::SplitRight), MenuCmd::Split(pane, Dir::Row)),
            item("Split down", &hint(A::SplitDown), MenuCmd::Split(pane, Dir::Col)),
            item("Zoom / restore", &hint(A::Zoom), MenuCmd::Zoom(pane)),
            item(pass_label, &hint(A::Passthrough), MenuCmd::Passthrough(pane)),
            item("Copy path", "", MenuCmd::CopyText(cwd.display().to_string())),
            item("Open folder", "", MenuCmd::OpenFolder(cwd.clone())),
            item("Open in VS Code", "", MenuCmd::OpenCode(cwd)),
            item("Close pane", &hint(A::ClosePane), MenuCmd::ClosePane(pane)),
        ];
        self.open_menu(label, x, y, items);
    }

    pub fn open_project_menu(&mut self, path: PathBuf, x: u16, y: u16) {
        let Some(p) = self.projects.iter().find(|p| p.path == path) else { return };
        let name = p.name.clone();
        let mut items = vec![item("Open terminal", "⏎", MenuCmd::OpenProject(path.clone()))];
        for (i, l) in self.quick_launchers() {
            items.push(item(&format!("Start {}", l.name), &l.key, MenuCmd::Launch(i, path.clone())));
        }
        items.push(item("Open in VS Code", "", MenuCmd::OpenCode(path.clone())));
        items.push(item("Open folder", "o", MenuCmd::OpenFolder(path.clone())));
        items.push(item("Git pull", "", MenuCmd::GitPull(path.clone())));
        items.push(item("Copy path", "", MenuCmd::CopyText(path.display().to_string())));
        let pin = if self.ui_state.is_pinned(&path) { "Unpin" } else { "Pin to top" };
        items.push(item(pin, "", MenuCmd::TogglePin(path)));
        self.open_menu(name, x, y, items);
    }

    pub fn run_menu(&mut self, cmd: MenuCmd) {
        self.overlay = None;
        match cmd {
            MenuCmd::Paste(pane) => self.paste_clipboard(pane),
            MenuCmd::Split(pane, dir) => {
                if self.focus_pane(pane) {
                    self.split(dir);
                }
            }
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
            MenuCmd::OpenFolder(path) => self.open_in_explorer(&path),
            MenuCmd::OpenCode(path) => self.open_in_code(&path),
            MenuCmd::RenameTab(i) => self.rename_tab_prompt(i),
            MenuCmd::CloseTab(i) => self.request_close_tab(i),
            MenuCmd::MoveTab(i, d) => {
                let to = (i as i32 + d).clamp(0, self.tabs.len() as i32 - 1) as usize;
                self.move_tab(i, to);
            }
            MenuCmd::OpenProject(path) => self.open_project_shell(path),
            MenuCmd::Launch(i, path) => self.launch(i, Some(path)),
            MenuCmd::GitPull(path) => self.git_pull(&path),
            MenuCmd::TogglePin(path) => self.toggle_pin(&path),
        }
    }

    pub(super) fn menu_key(&mut self, k: KeyEvent) {
        let Some(Overlay::Menu(m)) = &mut self.overlay else { return };
        let n = m.items.len();
        match k.code {
            KeyCode::Esc => self.overlay = None,
            KeyCode::Up | KeyCode::Char('k') => m.selected = (m.selected + n - 1) % n.max(1),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => m.selected = (m.selected + 1) % n.max(1),
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(it) = m.items.get(m.selected).filter(|it| it.enabled).cloned() {
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
        let value = self.tabs.get(i).and_then(|t| t.name.clone()).unwrap_or_default();
        self.overlay =
            Some(Overlay::Prompt(Prompt { title: "RENAME TAB".into(), value, purpose: PromptPurpose::RenameTab(i) }));
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
        cmd.arg(path).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        match cmd.spawn() {
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
