//! Command palette: actions, tabs, projects, launchers, themes, workspaces.

use std::path::PathBuf;

use super::{App, Overlay, SettingItem, ToastLevel, ToastTarget, View};
use crate::keys::Action;
use crate::util;

/// Palette commands remembered for the empty query (`UiStateData::palette_recent`).
const RECENT_LIMIT: usize = 10;

#[derive(Clone, Debug, PartialEq)]
pub enum PaletteCmd {
    Action(Action),
    GoTab(usize),
    OpenProject(PathBuf),
    Launch {
        launcher: usize,
        path: PathBuf,
    },
    /// A launcher in a new split of the focused pane.
    LaunchSplit(usize),
    /// A quick task of a project (`Project::tasks`) in a new tab in its folder.
    RunTask {
        path: PathBuf,
        command: String,
    },
    Theme(&'static str),
    Workspace(usize),
    DeleteWorkspace(String),
    OpenDir(PathBuf),
    /// A past notification (`notifications`): Enter follows its target, as a click on the toast does, or copies
    /// its full text when it has none; Ctrl+Y always copies the text.
    Notice {
        text: String,
        target: Option<ToastTarget>,
    },
    /// A Settings row: running it does what ⏎ does on that row (`App::enter_setting`).
    Setting(SettingItem),
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
    /// For each match: the char positions of its title that the query matched (drawn in bold accent).
    pub positions: Vec<Vec<usize>>,
    /// Indices into `all` of the recently run commands, most recent first (listed first on an empty query).
    pub recent: Vec<usize>,
}

impl PaletteState {
    pub fn refilter(&mut self) {
        let q = self.query.trim();
        if q.is_empty() {
            // Recent commands first, then the rest in the usual order; no item twice.
            let mut seen = vec![false; self.all.len()];
            let order = self.recent.iter().copied().chain(0..self.all.len());
            let mut matches = Vec::with_capacity(self.all.len());
            for i in order {
                if i < seen.len() && !seen[i] {
                    seen[i] = true;
                    matches.push(i);
                }
            }
            self.positions = vec![Vec::new(); matches.len()];
            self.matches = matches;
        } else {
            let mut scored: Vec<(usize, i32, Vec<usize>)> = self
                .all
                .iter()
                .enumerate()
                .filter_map(|(i, it)| {
                    let title = util::fuzzy_match(q, &it.title);
                    // The group is matched too (`THEME`), and wins ties; only title letters are marked.
                    let tagged = util::fuzzy_match(q, &format!("{} {}", it.group, it.title));
                    let offset = it.group.chars().count() + 1;
                    let (score, marks) = match (title, tagged) {
                        (None, None) => return None,
                        (Some(t), None) => t,
                        (None, Some((s, p))) => (s - 4, p.into_iter().filter_map(|x| x.checked_sub(offset)).collect()),
                        (Some(t), Some((s, p))) if s - 4 >= t.0 => {
                            (s - 4, p.into_iter().filter_map(|x| x.checked_sub(offset)).collect())
                        }
                        (Some(t), Some(_)) => t,
                    };
                    Some((i, score, marks))
                })
                .collect();
            scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            self.matches = scored.iter().map(|(i, _, _)| *i).collect();
            self.positions = scored.into_iter().map(|(_, _, marks)| marks).collect();
        }
        self.selected = self.selected.min(self.matches.len().saturating_sub(1));
    }

    pub fn current(&self) -> Option<&PaletteItem> {
        self.all.get(*self.matches.get(self.selected)?)
    }
}

/// The single key Home handles itself for an action that has no keymap binding (`input.rs` `bridge_key`).
fn home_key(a: Action) -> Option<&'static str> {
    match a {
        Action::RescanProjects => Some("r"),
        Action::RefreshAi => Some("R"),
        Action::AddProjectFolder => Some("a"),
        _ => None,
    }
}

/// The quick tasks of a project as palette commands: "Run: npm run dev" in the TASK group.
fn task_items(p: &crate::projects::Project) -> Vec<PaletteItem> {
    p.tasks
        .iter()
        .map(|command| PaletteItem {
            title: format!("Run: {command}"),
            group: "TASK",
            hint: String::new(),
            cmd: PaletteCmd::RunTask { path: p.path.clone(), command: command.clone() },
        })
        .collect()
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
                | Action::EqualizePanes
                | Action::SwapPane
                | Action::MovePaneToTab
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
                | Action::SendPrefix
                | Action::MoveTabLeft
                | Action::MoveTabRight
                | Action::PaneMenu
                | Action::OpenLink
                | Action::Search
                | Action::FocusMode
                | Action::PrevCommand
                | Action::NextCommand
                | Action::CopyLastOutput
                | Action::SendLastOutput
                | Action::Passthrough => in_term,
                Action::Update | Action::DismissUpdate => self.update_notice().is_some(),
                Action::ReopenTab => !self.closed_tabs.is_empty(),
                Action::EnableHooks | Action::DismissHooksOffer => self.hooks_offer_notice(),
                Action::Palette => false,
                _ => true,
            };
            if !relevant {
                continue;
            }
            // Tabs are listed below by name.
            if matches!(a, Action::GoTab(_)) {
                continue;
            }
            // In a terminal the hint is a key that reaches NOBLE there (not a shell key, none while the pane's
            // keys are locked); on Home the single keys that Home handles itself are the fallback.
            let hint = if in_term {
                self.keymap.term_hint(a, self.focused_locked())
            } else {
                let home = if self.view == View::Bridge { home_key(a) } else { None };
                self.keymap.hint(a).or_else(|| home.map(String::from))
            };
            items.push(PaletteItem {
                title: a.title(),
                group: a.group(),
                hint: hint.unwrap_or_default(),
                cmd: PaletteCmd::Action(a),
            });
        }
        for i in 0..self.tabs.len() {
            items.push(PaletteItem {
                title: format!("Tab {}: {}", i + 1, self.tab_title(i)),
                group: "TAB",
                // The key actually bound to the tab (rebinds count), none for tabs past the ninth.
                hint: u8::try_from(i + 1)
                    .ok()
                    .filter(|n| *n <= 9)
                    .and_then(|n| self.keymap.hint(Action::GoTab(n)))
                    .unwrap_or_default(),
                cmd: PaletteCmd::GoTab(i + 1),
            });
        }
        if let Some(cwd) = self.focused_cwd().filter(|_| in_term) {
            // In a terminal the launchers act on the focused pane's directory, not on Home's selected project.
            for (i, l) in self.quick_launchers() {
                items.push(PaletteItem {
                    title: format!("Launch {} here", l.name),
                    group: "RUN",
                    hint: String::new(),
                    cmd: PaletteCmd::Launch { launcher: i, path: cwd.clone() },
                });
                items.push(PaletteItem {
                    title: format!("Launch {} in split", l.name),
                    group: "RUN",
                    hint: String::new(),
                    cmd: PaletteCmd::LaunchSplit(i),
                });
            }
        } else if let Some(p) = self.selected_project().filter(|_| !in_term) {
            for (i, l) in self.quick_launchers() {
                items.push(PaletteItem {
                    title: format!("Launch {} in {}", l.name, p.name),
                    group: "RUN",
                    hint: l.key.clone(),
                    cmd: PaletteCmd::Launch { launcher: i, path: p.path.clone() },
                });
            }
            items.extend(task_items(p));
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
            items.push(PaletteItem {
                title: format!("Delete Workspace: {}", w.name),
                group: "WORK",
                hint: String::new(),
                cmd: PaletteCmd::DeleteWorkspace(w.name.clone()),
            });
        }
        for e in self.recent.top(12) {
            let path = PathBuf::from(&e.path);
            if self.projects.iter().any(|p| util::same_path(&p.path, &path)) {
                continue;
            }
            items.push(PaletteItem {
                title: format!("Shell in {}", util::tilde(&path)),
                group: "DIR",
                hint: String::new(),
                cmd: PaletteCmd::OpenDir(path),
            });
        }
        // Every Settings row, with its value; running one does what ⏎ does on that row.
        for item in self.all_settings_items() {
            if let SettingItem::Setting(key) = item {
                items.push(PaletteItem {
                    title: format!("Setting: {} ({})", key.label(), self.setting_value(key)),
                    group: "SET",
                    hint: String::new(),
                    cmd: PaletteCmd::Setting(item),
                });
            }
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

    /// The notification history, newest first (the `notifications` action); title = time + text.
    pub fn open_notifications(&mut self) {
        if self.notices.is_empty() {
            self.toast(ToastLevel::Info, "no notifications yet");
            return;
        }
        // The list holds every message, so the ones still on screen go: the only keyboard way to clear a
        // sticky error toast.
        self.toasts.clear();
        self.dirty = true;
        let items = self
            .notices
            .iter()
            .rev()
            .map(|n| PaletteItem {
                title: format!("{}  {}", n.at.format("%H:%M:%S"), n.text),
                group: "NOTE",
                hint: match n.level {
                    ToastLevel::Info => "info",
                    ToastLevel::Ok => "ok",
                    ToastLevel::Warn => "warn",
                    ToastLevel::Error => "error",
                }
                .into(),
                cmd: PaletteCmd::Notice { text: n.text.clone(), target: n.target.clone() },
            })
            .collect();
        self.open_palette_on(items);
    }

    pub fn open_palette(&mut self) {
        let all = self.palette_items();
        self.open_palette_on(all);
    }

    /// The palette listing only the selected project's tasks: Home's `T`. Nothing opens without tasks.
    pub fn open_task_list(&mut self) {
        let Some(p) = self.selected_project() else { return };
        let tasks = task_items(p);
        if !tasks.is_empty() {
            self.open_palette_on(tasks);
        }
    }

    /// The palette listing only the tabs: the tab bar's "+N" chip, for the tabs the bar has no room for.
    pub fn open_tab_list(&mut self) {
        let tabs = self.palette_items().into_iter().filter(|i| matches!(i.cmd, PaletteCmd::GoTab(_))).collect();
        self.open_palette_on(tabs);
    }

    fn open_palette_on(&mut self, all: Vec<PaletteItem>) {
        // The recent commands that are in this list, as indices into it (most recent first).
        let keys: Vec<Option<String>> = all.iter().map(|i| self.palette_key(&i.cmd)).collect();
        let recent = self
            .ui_state
            .data
            .palette_recent
            .iter()
            .filter_map(|k| keys.iter().position(|key| key.as_deref() == Some(k.as_str())))
            .collect();
        let mut st =
            PaletteState { query: String::new(), selected: 0, all, matches: Vec::new(), positions: Vec::new(), recent };
        st.refilter();
        self.overlay = Some(Overlay::Palette(st));
    }

    /// A stable key for a command, as remembered in `UiStateData::palette_recent`: the Action id, or
    /// the kind and the value ("theme:amber", "project:/path"). `None` for commands that cannot be
    /// repeated later: tabs (listed by position) and notices (listed by their text).
    pub fn palette_key(&self, cmd: &PaletteCmd) -> Option<String> {
        Some(match cmd {
            PaletteCmd::Action(a) => a.id(),
            PaletteCmd::GoTab(_) | PaletteCmd::Notice { .. } => return None,
            PaletteCmd::OpenProject(p) => format!("project:{}", p.display()),
            PaletteCmd::Launch { launcher, path } => {
                let name = self.launchers.get(*launcher).map_or(launcher.to_string(), |(l, _)| l.name.clone());
                format!("launch:{name}:{}", path.display())
            }
            PaletteCmd::LaunchSplit(i) => {
                let name = self.launchers.get(*i).map_or(i.to_string(), |(l, _)| l.name.clone());
                format!("launch_split:{name}")
            }
            PaletteCmd::RunTask { path, command } => format!("task:{}:{command}", path.display()),
            PaletteCmd::Theme(name) => format!("theme:{name}"),
            PaletteCmd::Workspace(i) => format!("workspace:{}", self.workspaces.list.get(*i)?.name),
            PaletteCmd::DeleteWorkspace(name) => format!("delete_workspace:{name}"),
            PaletteCmd::OpenDir(p) => format!("dir:{}", p.display()),
            PaletteCmd::Setting(SettingItem::Setting(key)) => format!("setting:{key:?}"),
            PaletteCmd::Setting(_) => return None,
        })
    }

    /// Puts a command first in the recent list (kept to `RECENT_LIMIT`, written to `state.json`).
    fn remember_palette_cmd(&mut self, cmd: &PaletteCmd) {
        let Some(key) = self.palette_key(cmd) else { return };
        self.edit_ui_state(|d| {
            d.palette_recent.retain(|k| *k != key);
            d.palette_recent.insert(0, key);
            d.palette_recent.truncate(RECENT_LIMIT);
        });
    }

    pub fn run_palette(&mut self, cmd: PaletteCmd) {
        self.remember_palette_cmd(&cmd);
        match cmd {
            PaletteCmd::Setting(item) => self.enter_setting(item),
            PaletteCmd::Action(a) => self.run(a),
            PaletteCmd::GoTab(n) => self.go_tab(n),
            PaletteCmd::OpenProject(p) => self.open_project_shell(p),
            PaletteCmd::Launch { launcher, path } => self.launch(launcher, Some(path)),
            PaletteCmd::LaunchSplit(i) => self.launch_in_split(i),
            PaletteCmd::RunTask { path, command } => self.run_task(path, &command),
            PaletteCmd::Theme(name) => self.set_theme(name),
            PaletteCmd::Workspace(i) => self.restore_workspace(i),
            PaletteCmd::DeleteWorkspace(name) => self.request_delete_workspace(name),
            PaletteCmd::Notice { text, target } => match target {
                Some(target) if self.toast_target_open(&target) => self.follow_toast(target),
                // No target, or its pane has closed: Enter copies.
                _ => self.copy_notice(&text),
            },
            PaletteCmd::OpenDir(p) => {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.new_tab(p, None, Some(name));
            }
        }
    }

    /// Copies a notification's text to the clipboard and says so.
    pub(super) fn copy_notice(&mut self, text: &str) {
        if self.clipboard.set(text).is_some() {
            self.toast(ToastLevel::Ok, "copied");
        } else {
            self.toast(ToastLevel::Warn, "clipboard unavailable");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(group: &'static str, title: &str) -> PaletteItem {
        PaletteItem { title: title.into(), group, hint: String::new(), cmd: PaletteCmd::GoTab(1) }
    }

    fn state(query: &str, all: Vec<PaletteItem>, recent: Vec<usize>) -> PaletteState {
        let mut st =
            PaletteState { query: query.into(), selected: 0, all, matches: Vec::new(), positions: Vec::new(), recent };
        st.refilter();
        st
    }

    #[test]
    fn empty_query_lists_recent_first_without_duplicates() {
        let all = vec![item("A", "one"), item("A", "two"), item("A", "three"), item("A", "four")];
        // Index 2 and 0 were run last (newest first); 9 does not exist and is skipped.
        let st = state("", all, vec![2, 0, 9]);
        assert_eq!(st.matches, vec![2, 0, 1, 3]);
        assert_eq!(st.positions.len(), 4);
    }

    #[test]
    fn matches_carry_title_positions() {
        let all = vec![item("TAB", "Split Pane Right"), item("TAB", "New Tab")];
        let st = state("spr", all, Vec::new());
        assert_eq!(st.matches, vec![0]);
        assert_eq!(st.positions[0], vec![0, 1, 11]);
        // A match only in the group name marks no title letters.
        let st = state("tab", vec![item("TAB", "Open Folder")], Vec::new());
        assert_eq!(st.matches, vec![0]);
        assert!(st.positions[0].is_empty());
    }
}
