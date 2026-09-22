//! Komut paleti: eylemler, sekmeler, projeler, başlatıcılar, temalar, çalışma alanları.

use std::path::PathBuf;

use super::{App, Overlay, View};
use crate::keys::Action;
use crate::util;

#[derive(Clone, Debug, PartialEq)]
pub enum PaletteCmd {
    Action(Action),
    GoTab(usize),
    OpenProject(PathBuf),
    Launch { launcher: usize, path: PathBuf },
    Theme(&'static str),
    Workspace(usize),
    OpenDir(PathBuf),
}

#[derive(Clone, Debug)]
pub struct PaletteItem {
    pub title: String,
    pub group: &'static str,
    pub hint: String,
    pub cmd: PaletteCmd,
}

pub struct PaletteState {
    pub query: String,
    pub selected: usize,
    pub all: Vec<PaletteItem>,
    pub matches: Vec<usize>,
}

impl PaletteState {
    pub fn refilter(&mut self) {
        let q = self.query.trim();
        if q.is_empty() {
            self.matches = (0..self.all.len()).collect();
        } else {
            let mut scored: Vec<(usize, i32)> = self
                .all
                .iter()
                .enumerate()
                .filter_map(|(i, it)| {
                    let title = util::fuzzy_score(q, &it.title);
                    let tagged = util::fuzzy_score(q, &format!("{} {}", it.group, it.title)).map(|s| s - 4);
                    title.max(tagged).map(|s| (i, s))
                })
                .collect();
            scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            self.matches = scored.into_iter().map(|(i, _)| i).collect();
        }
        self.selected = self.selected.min(self.matches.len().saturating_sub(1));
    }

    pub fn current(&self) -> Option<&PaletteItem> {
        self.all.get(*self.matches.get(self.selected)?)
    }
}

impl App {
    pub fn palette_items(&self) -> Vec<PaletteItem> {
        let mut items = Vec::new();
        let in_term = matches!(self.view, View::Term(_));
        for a in Action::ALL {
            let relevant = match a {
                Action::GoTab(n) => (n as usize) <= self.tabs.len(),
                Action::CloseTab
                | Action::SplitRight
                | Action::SplitDown
                | Action::ClosePane
                | Action::Zoom
                | Action::FocusLeft
                | Action::FocusRight
                | Action::FocusUp
                | Action::FocusDown
                | Action::FocusNext
                | Action::ResizeLeft
                | Action::ResizeRight
                | Action::ResizeUp
                | Action::ResizeDown
                | Action::RenameTab
                | Action::ScrollUp
                | Action::ScrollDown
                | Action::SendPrefix => in_term,
                Action::Palette => false,
                _ => true,
            };
            if !relevant {
                continue;
            }
            // Sekmeler aşağıda adlarıyla listelenir.
            if matches!(a, Action::GoTab(_)) {
                continue;
            }
            items.push(PaletteItem {
                title: a.title(),
                group: a.group(),
                hint: self.keymap.hint(a).unwrap_or_default(),
                cmd: PaletteCmd::Action(a),
            });
        }
        for i in 0..self.tabs.len() {
            items.push(PaletteItem {
                title: format!("Tab {}: {}", i + 1, self.tab_title(i)),
                group: "TAB",
                hint: if i < 9 { format!("alt+{}", i + 1) } else { String::new() },
                cmd: PaletteCmd::GoTab(i + 1),
            });
        }
        if let Some(p) = self.selected_project() {
            for (i, (l, ok)) in self.launchers.iter().enumerate() {
                if *ok {
                    items.push(PaletteItem {
                        title: format!("Launch {} in {}", l.name, p.name),
                        group: "RUN",
                        hint: l.key.clone(),
                        cmd: PaletteCmd::Launch { launcher: i, path: p.path.clone() },
                    });
                }
            }
        }
        for p in self.projects.iter().take(120) {
            items.push(PaletteItem {
                title: format!("Open {}", p.name),
                group: "PROJ",
                hint: p.branch.clone().unwrap_or_default(),
                cmd: PaletteCmd::OpenProject(p.path.clone()),
            });
        }
        for (i, w) in self.workspaces.list.iter().enumerate() {
            items.push(PaletteItem {
                title: format!("Restore workspace: {}", w.name),
                group: "WORK",
                hint: format!("{} tabs", w.tabs.len()),
                cmd: PaletteCmd::Workspace(i),
            });
        }
        for e in self.recent.top(12) {
            let path = PathBuf::from(&e.path);
            if self.projects.iter().any(|p| p.path == path) {
                continue;
            }
            items.push(PaletteItem {
                title: format!("Shell in {}", util::tilde(&path)),
                group: "DIR",
                hint: String::new(),
                cmd: PaletteCmd::OpenDir(path),
            });
        }
        for t in crate::theme::THEMES.iter() {
            items.push(PaletteItem {
                title: format!("Theme: {}", t.label),
                group: "THEME",
                hint: if t.name == self.theme.name { "active".into() } else { String::new() },
                cmd: PaletteCmd::Theme(t.name),
            });
        }
        items
    }

    pub fn open_palette(&mut self) {
        let mut st = PaletteState { query: String::new(), selected: 0, all: self.palette_items(), matches: Vec::new() };
        st.refilter();
        self.overlay = Some(Overlay::Palette(st));
    }

    pub fn run_palette(&mut self, cmd: PaletteCmd) {
        match cmd {
            PaletteCmd::Action(a) => self.run(a),
            PaletteCmd::GoTab(n) => self.go_tab(n),
            PaletteCmd::OpenProject(p) => self.open_project_shell(p),
            PaletteCmd::Launch { launcher, path } => self.launch(launcher, Some(path)),
            PaletteCmd::Theme(name) => self.set_theme(name),
            PaletteCmd::Workspace(i) => self.restore_workspace(i),
            PaletteCmd::OpenDir(p) => {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.new_tab(p, None, Some(name));
            }
        }
    }
}
