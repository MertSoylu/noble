//! Eylemler: sekme/pane işlemleri, başlatıcılar, oturum ve çalışma alanları.

use std::path::{Path, PathBuf};

use super::{App, Confirm, ConfirmAction, Overlay, Prompt, PromptPurpose, ToastLevel, View};
use crate::keys::Action;
use crate::sensors::SensorRequest;
use crate::store::{SavedTab, Workspace};
use crate::term::Tab;
use crate::term::layout::{Dir, Direction, MIN_H, MIN_W, Node, PaneId, SavedNode, neighbor};
use crate::term::pane::{Pane, SpawnSpec};
use crate::theme;

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn dir_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

impl App {
    pub fn run(&mut self, action: Action) {
        match action {
            Action::Bridge => self.view = View::Bridge,
            Action::System => self.view = View::System,
            Action::Settings => self.view = View::Settings,
            Action::NewTab => {
                let cwd = self.current_cwd().unwrap_or_else(home);
                self.new_tab(cwd, None, None);
            }
            Action::CloseTab => {
                if let View::Term(i) = self.view {
                    self.remove_tab(i);
                }
            }
            Action::NextTab => self.cycle_tab(1),
            Action::PrevTab => self.cycle_tab(-1),
            Action::GoTab(n) => self.go_tab(n as usize),
            Action::SplitRight => self.split(Dir::Row),
            Action::SplitDown => self.split(Dir::Col),
            Action::ClosePane => {
                if let Some(id) = self.focused_pane() {
                    self.close_pane(id);
                }
            }
            Action::Zoom => {
                if let (View::Term(ti), Some(id)) = (self.view, self.focused_pane()) {
                    self.toggle_zoom(ti, id);
                }
            }
            Action::FocusLeft => self.focus_dir(Direction::Left),
            Action::FocusRight => self.focus_dir(Direction::Right),
            Action::FocusUp => self.focus_dir(Direction::Up),
            Action::FocusDown => self.focus_dir(Direction::Down),
            Action::FocusNext => {
                if let Some(t) = self.current_tab_mut() {
                    let panes = t.panes();
                    let i = panes.iter().position(|p| *p == t.focus).unwrap_or(0);
                    t.focus = panes[(i + 1) % panes.len()];
                }
            }
            Action::ResizeLeft => self.nudge(Dir::Row, -0.05),
            Action::ResizeRight => self.nudge(Dir::Row, 0.05),
            Action::ResizeUp => self.nudge(Dir::Col, -0.05),
            Action::ResizeDown => self.nudge(Dir::Col, 0.05),
            Action::Palette => self.open_palette(),
            Action::Help => self.overlay = Some(Overlay::Help { scroll: 0 }),
            Action::Quit => self.request_quit(),
            Action::ReloadConfig => self.reload_config(true),
            Action::OpenConfig => self.open_config(),
            Action::CycleTheme => {
                let next = theme::next_theme(self.theme.name);
                self.set_theme(next);
            }
            Action::RefreshAi => self.refresh_ai(),
            Action::RescanProjects => self.rescan_projects(),
            Action::RenameTab => {
                if let View::Term(i) = self.view {
                    let value = self.tabs.get(i).and_then(|t| t.name.clone()).unwrap_or_default();
                    self.overlay = Some(Overlay::Prompt(Prompt {
                        title: "RENAME TAB".into(),
                        value,
                        purpose: PromptPurpose::RenameTab(i),
                    }));
                }
            }
            Action::SaveWorkspace => {
                if self.tabs.is_empty() {
                    self.toast(ToastLevel::Warn, "no terminal tabs to save");
                } else {
                    let value = self.tabs.iter().map(|t| t.origin.clone()).collect::<Vec<_>>().join(" + ");
                    self.overlay = Some(Overlay::Prompt(Prompt {
                        title: "SAVE WORKSPACE AS".into(),
                        value: crate::util::truncate(&value, 32),
                        purpose: PromptPurpose::SaveWorkspace,
                    }));
                }
            }
            Action::ScrollUp => self.scroll_page(1),
            Action::ScrollDown => self.scroll_page(-1),
            Action::Search => self.open_search(),
            Action::AddProjectFolder => {
                self.overlay = Some(Overlay::Prompt(Prompt {
                    title: "ADD PROJECT FOLDER".into(),
                    value: String::new(),
                    purpose: PromptPurpose::AddRoot,
                }));
            }
            Action::SendPrefix => {
                let bytes = crate::term::input::encode_key(
                    &crossterm::event::KeyEvent::new(self.keymap.prefix.code, self.keymap.prefix.mods),
                    false,
                );
                if let Some(p) = self.focused_pane().and_then(|id| self.panes.get(&id)) {
                    p.write(&bytes);
                }
            }
        }
    }

    // ─── Sekmeler ───────────────────────────────────────────────────────────

    pub fn current_tab(&self) -> Option<&Tab> {
        match self.view {
            View::Term(i) => self.tabs.get(i),
            _ => None,
        }
    }

    pub fn current_tab_mut(&mut self) -> Option<&mut Tab> {
        match self.view {
            View::Term(i) => self.tabs.get_mut(i),
            _ => None,
        }
    }

    pub fn focused_pane(&self) -> Option<PaneId> {
        self.current_tab().map(|t| t.focus)
    }

    fn current_cwd(&self) -> Option<PathBuf> {
        self.focused_pane().and_then(|id| self.panes.get(&id)).map(|p| p.cwd())
    }

    pub fn tab_title(&self, i: usize) -> String {
        let Some(t) = self.tabs.get(i) else { return String::new() };
        if let Some(n) = &t.name {
            return n.clone();
        }
        let label = self.panes.get(&t.focus).map(|p| (p.label(), p.shell_label.clone()));
        match label {
            // Başlatıcı sekmeleri ("proje · claude") zaten ne çalıştığını söyler.
            _ if t.origin.contains(" · ") => t.origin.clone(),
            Some((l, shell)) if !l.eq_ignore_ascii_case(&shell) && !l.is_empty() => {
                format!("{} · {}", t.origin, crate::util::truncate(&l, 16))
            }
            _ => t.origin.clone(),
        }
    }

    pub fn go_tab(&mut self, n: usize) {
        if n >= 1 && n <= self.tabs.len() {
            self.view = View::Term(n - 1);
            self.tabs[n - 1].activity = false;
        }
    }

    fn cycle_tab(&mut self, delta: i32) {
        if self.tabs.is_empty() {
            return;
        }
        let len = self.tabs.len() as i32;
        let next = match self.view {
            View::Term(i) => (i as i32 + delta).rem_euclid(len),
            _ => {
                if delta > 0 {
                    0
                } else {
                    len - 1
                }
            }
        };
        self.go_tab(next as usize + 1);
    }

    /// Terminal gövdesinde yeni bir pane'in alacağı yaklaşık boyut.
    fn fresh_size(&self) -> (u16, u16) {
        let b = self.body();
        (b.height.saturating_sub(2).max(4), b.width.saturating_sub(2).max(10))
    }

    fn spawn_pane(&mut self, cwd: &Path, command: Option<&str>, rows: u16, cols: u16) -> Option<PaneId> {
        let id = self.next_id;
        self.next_id += 1;
        let spec = SpawnSpec {
            cwd,
            command,
            shell: &self.shell,
            rows,
            cols,
            scrollback: self.cfg.terminal.scrollback.clamp(100, 100_000),
        };
        match Pane::spawn(id, spec, self.tx.clone()) {
            Ok(p) => {
                self.panes.insert(id, p);
                Some(id)
            }
            Err(e) => {
                self.toast(ToastLevel::Error, format!("{e:#}"));
                None
            }
        }
    }

    /// Yeni terminal sekmesi açar; `command` verilirse shell önce onu çalıştırır.
    pub fn new_tab(&mut self, cwd: PathBuf, command: Option<&str>, origin: Option<String>) {
        let (rows, cols) = self.fresh_size();
        if let Some(id) = self.spawn_pane(&cwd, command, rows, cols) {
            let origin = origin.unwrap_or_else(|| dir_name(&cwd));
            self.tabs.push(Tab::new(id, origin));
            self.view = View::Term(self.tabs.len() - 1);
            self.recent.record(&cwd);
        }
    }

    pub fn remove_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(i);
        for id in tab.panes() {
            self.panes.remove(&id);
            if self.agent_hooks.remove(&id).is_some() {
                crate::hooks::remove_record(&self.paths.data, std::process::id(), id);
            }
        }
        self.view = match self.view {
            View::Term(v) if v == i => {
                if self.tabs.is_empty() {
                    View::Bridge
                } else {
                    View::Term(i.min(self.tabs.len() - 1))
                }
            }
            View::Term(v) if v > i => View::Term(v - 1),
            other => other,
        };
        if let View::Term(v) = self.view {
            self.tabs[v].activity = false;
        }
    }

    pub(super) fn split(&mut self, dir: Dir) {
        let body = self.body();
        let Some(tab) = self.current_tab() else {
            self.toast(ToastLevel::Info, "open a terminal tab first");
            return;
        };
        let (rects, _) = tab.root.layout(body);
        let focus = tab.focus;
        let Some(rect) = rects.iter().find(|(id, _)| *id == focus).map(|(_, r)| *r) else { return };
        let fits = match dir {
            Dir::Row => rect.width / 2 >= MIN_W,
            Dir::Col => rect.height / 2 > MIN_H,
        };
        if !fits {
            self.toast(ToastLevel::Warn, "not enough room to split");
            return;
        }
        let (rows, cols) = match dir {
            Dir::Row => (rect.height.saturating_sub(2), (rect.width / 2).saturating_sub(2)),
            Dir::Col => ((rect.height / 2).saturating_sub(2), rect.width.saturating_sub(2)),
        };
        let cwd = self.panes.get(&focus).map(|p| p.cwd()).unwrap_or_else(home);
        if let Some(new) = self.spawn_pane(&cwd, None, rows, cols)
            && let Some(tab) = self.current_tab_mut()
        {
            tab.root.split(focus, new, dir);
            tab.focus = new;
            tab.zoomed = false;
        }
    }

    pub fn close_pane(&mut self, id: PaneId) {
        self.panes.remove(&id);
        if self.agent_hooks.remove(&id).is_some() {
            crate::hooks::remove_record(&self.paths.data, std::process::id(), id);
        }
        let Some(ti) = self.tabs.iter().position(|t| t.root.contains(id)) else { return };
        if self.tabs[ti].panes().len() <= 1 {
            self.remove_tab(ti);
            return;
        }
        let body = self.body();
        let tab = &mut self.tabs[ti];
        let (rects, _) = tab.root.layout(body);
        let fallback = [Direction::Left, Direction::Up, Direction::Right, Direction::Down]
            .iter()
            .find_map(|d| neighbor(&rects, id, *d));
        tab.root.remove(id);
        tab.zoomed = false;
        if tab.focus == id {
            tab.focus = fallback.filter(|f| tab.root.contains(*f)).unwrap_or_else(|| tab.root.leaves()[0]);
        }
    }

    /// Pane'i tam ekrana alır ya da geri döndürür; yerinden büyüyerek/küçülerek.
    pub fn toggle_zoom(&mut self, ti: usize, pane: PaneId) {
        let body = self.body();
        let Some(tab) = self.tabs.get_mut(ti) else { return };
        if tab.panes().len() < 2 {
            return;
        }
        tab.focus = pane;
        let tile = tab.root.layout(body).0.into_iter().find(|(id, _)| *id == pane).map(|(_, r)| r);
        tab.zoomed = !tab.zoomed;
        let zoomed = tab.zoomed;
        if let Some(tile) = tile
            && self.cfg.general.animations
        {
            let (from, to) = if zoomed { (tile, body) } else { (body, tile) };
            self.zoom_anim = Some(super::ZoomAnim { pane, from, to, started: std::time::Instant::now() });
        }
    }

    fn focus_dir(&mut self, d: Direction) {
        let body = self.body();
        if let Some(tab) = self.current_tab_mut() {
            tab.zoomed = false;
            let (rects, _) = tab.root.layout(body);
            if let Some(n) = neighbor(&rects, tab.focus, d) {
                tab.focus = n;
            }
        }
    }

    fn nudge(&mut self, dir: Dir, delta: f32) {
        if let Some(tab) = self.current_tab_mut() {
            let focus = tab.focus;
            tab.root.nudge(focus, dir, delta);
        }
    }

    fn scroll_page(&mut self, dir: i32) {
        if let Some(p) = self.focused_pane().and_then(|id| self.panes.get(&id)) {
            let page = p.size.0.saturating_sub(2).max(1) as i32;
            p.scroll(dir * page);
        }
    }

    /// Görünen sekmedeki pane'leri çizimden önce yeni boyutlarına getirir.
    pub fn sync_layout(&mut self) {
        let body = self.body();
        let Some(tab) = self.current_tab() else { return };
        let rects: Vec<(PaneId, ratatui::layout::Rect)> =
            if tab.zoomed { vec![(tab.focus, body)] } else { tab.root.layout(body).0 };
        for (id, r) in rects {
            if let Some(p) = self.panes.get_mut(&id) {
                p.resize(r.height.saturating_sub(2), r.width.saturating_sub(2));
            }
        }
    }

    // ─── Başlatıcılar ───────────────────────────────────────────────────────

    /// Seçili projede (ya da verilen dizinde) başlatıcı komutunu yeni sekmede çalıştırır.
    pub fn launch(&mut self, idx: usize, dir: Option<PathBuf>) {
        let Some((launcher, available)) = self.launchers.get(idx).cloned() else { return };
        let target = dir.or_else(|| self.selected_project().map(|p| p.path.clone()));
        let Some(path) = target else {
            self.toast(ToastLevel::Warn, "select a project first");
            return;
        };
        if !available {
            self.toast(ToastLevel::Warn, format!("'{}' not found on PATH — running anyway", launcher.command));
        }
        let origin = format!("{} · {}", dir_name(&path), launcher.name);
        let command = self.shell.invocation(&launcher.command);
        self.new_tab(path, Some(&command), Some(origin));
    }

    pub fn open_project_shell(&mut self, path: PathBuf) {
        let name = dir_name(&path);
        self.new_tab(path, None, Some(name));
    }

    pub fn open_in_explorer(&mut self, path: &Path) {
        let program = if cfg!(windows) {
            "explorer"
        } else if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        match std::process::Command::new(program).arg(path).spawn() {
            Ok(_) => self.toast(ToastLevel::Info, format!("opened {}", crate::util::tilde(path))),
            Err(_) => self.toast(ToastLevel::Error, "could not open file manager"),
        }
    }

    // ─── Tema / config ──────────────────────────────────────────────────────

    pub fn set_theme(&mut self, name: &str) {
        self.cfg.general.theme = name.to_string();
        self.theme = crate::theme::Theme::by_name(name, self.cfg.general.transparent);
        if self.services.is_some()
            && let Ok(text) = std::fs::read_to_string(&self.paths.config)
        {
            let updated = crate::config::set_value(&text, "general", "theme", &format!("\"{name}\""));
            if std::fs::write(&self.paths.config, updated).is_ok() {
                self.cfg_mtime = crate::config::mtime_of(&self.paths.config);
            }
        }
        self.toast(ToastLevel::Ok, format!("theme · {}", self.theme.label));
    }

    fn open_config(&mut self) {
        let path = self.paths.config.clone();
        let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).ok().filter(|e| !e.trim().is_empty());
        if let Some(editor) = editor {
            let quoted = format!("{editor} \"{}\"", path.display());
            let dir = path.parent().map(Path::to_path_buf).unwrap_or_else(home);
            self.new_tab(dir, Some(&quoted), Some("config".into()));
            return;
        }
        let result = if cfg!(windows) {
            std::process::Command::new("notepad").arg(&path).spawn()
        } else if cfg!(target_os = "macos") {
            std::process::Command::new("open").arg("-t").arg(&path).spawn()
        } else {
            std::process::Command::new("xdg-open").arg(&path).spawn()
        };
        match result {
            Ok(_) => self
                .toast(ToastLevel::Info, format!("editing {} — saved changes apply live", crate::util::tilde(&path))),
            Err(_) => self.toast(ToastLevel::Error, format!("config: {}", path.display())),
        }
    }

    pub fn refresh_ai(&mut self) {
        if !self.cfg.ai.enabled {
            self.toast(ToastLevel::Info, "AI link disabled in config");
            return;
        }
        if let Some(s) = &self.services {
            let _ = s.ai_refresh.send(crate::ai::AiReq::Refresh);
        }
        self.toast(ToastLevel::Info, "refreshing AI quotas…");
    }

    pub fn rescan_projects(&mut self) {
        if let Some(s) = &self.services {
            let _ = s.rescan.send(crate::projects::ProjectReq::Rescan);
        }
        self.toast(ToastLevel::Info, "scanning projects…");
    }

    pub fn request_kill(&mut self, pid: u32, name: String) {
        self.overlay = Some(Overlay::Confirm(Confirm {
            title: "TERMINATE PROCESS".into(),
            body: format!("{name} · pid {pid}"),
            action: ConfirmAction::Kill { pid, name },
        }));
    }

    pub fn request_quit(&mut self) {
        let n = self.pane_count();
        if n == 0 {
            self.quit = true;
            return;
        }
        let saved = if self.cfg.terminal.restore_session { " — session will be restored" } else { "" };
        self.overlay = Some(Overlay::Confirm(Confirm {
            title: "QUIT NOBLE".into(),
            body: format!("{n} shell{} running{saved}", if n == 1 { "" } else { "s" }),
            action: ConfirmAction::Quit,
        }));
    }

    pub fn confirm(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::Quit => self.quit = true,
            ConfirmAction::Kill { pid, name } => match &self.services {
                Some(s) => {
                    let _ = s.sensor_req.send(SensorRequest::Kill { pid });
                }
                None => self.toast(ToastLevel::Warn, format!("cannot terminate {name}")),
            },
        }
    }

    pub fn submit_prompt(&mut self, prompt: Prompt) {
        let value = prompt.value.trim().to_string();
        match prompt.purpose {
            PromptPurpose::RenameTab(i) => {
                if let Some(t) = self.tabs.get_mut(i) {
                    t.name = (!value.is_empty()).then_some(value);
                }
            }
            PromptPurpose::AddRoot => self.add_project_root(&value),
            PromptPurpose::SaveWorkspace => {
                if value.is_empty() {
                    return;
                }
                let ws = self.snapshot(&value);
                let panes = ws.pane_count();
                self.workspaces.upsert(ws);
                self.toast(ToastLevel::Ok, format!("workspace '{value}' saved · {panes} panes"));
            }
        }
    }

    // ─── Oturum / çalışma alanı ─────────────────────────────────────────────

    fn save_node(&self, n: &Node) -> SavedNode {
        match n {
            Node::Leaf(id) => {
                SavedNode::Leaf { cwd: self.panes.get(id).map(|p| p.cwd()).unwrap_or_else(home).display().to_string() }
            }
            Node::Split { dir, ratio, a, b } => SavedNode::Split {
                dir: *dir,
                ratio: *ratio,
                a: Box::new(self.save_node(a)),
                b: Box::new(self.save_node(b)),
            },
        }
    }

    pub fn snapshot(&self, name: &str) -> Workspace {
        Workspace {
            name: name.to_string(),
            saved_at: chrono::Utc::now().timestamp(),
            tabs: self
                .tabs
                .iter()
                .map(|t| SavedTab {
                    name: t.name.clone(),
                    origin: t.origin.clone(),
                    layout: self.save_node(&t.root),
                    focus: t.panes().iter().position(|p| *p == t.focus).unwrap_or(0),
                })
                .collect(),
        }
    }

    fn build_node(&mut self, n: &SavedNode, rows: u16, cols: u16) -> Option<Node> {
        match n {
            SavedNode::Leaf { cwd } => {
                let path = PathBuf::from(cwd);
                let path = if path.is_dir() { path } else { home() };
                self.spawn_pane(&path, None, rows, cols).map(Node::Leaf)
            }
            SavedNode::Split { dir, ratio, a, b } => {
                let a = self.build_node(a, rows, cols);
                let b = self.build_node(b, rows, cols);
                match (a, b) {
                    (Some(a), Some(b)) => {
                        Some(Node::Split { dir: *dir, ratio: *ratio, a: Box::new(a), b: Box::new(b) })
                    }
                    (Some(x), None) | (None, Some(x)) => Some(x),
                    (None, None) => None,
                }
            }
        }
    }

    /// Kayıtlı sekmeleri açar; açılan sekme sayısını döndürür.
    pub fn open_workspace(&mut self, ws: &Workspace) -> usize {
        let (rows, cols) = self.fresh_size();
        let mut opened = 0;
        for t in &ws.tabs {
            if let Some(root) = self.build_node(&t.layout, rows, cols) {
                let leaves = root.leaves();
                let focus = leaves.get(t.focus).copied().unwrap_or(leaves[0]);
                let mut tab = Tab::new(focus, t.origin.clone());
                tab.root = root;
                tab.name = t.name.clone();
                self.tabs.push(tab);
                opened += 1;
            }
        }
        opened
    }

    pub fn restore_workspace(&mut self, idx: usize) {
        let Some(ws) = self.workspaces.list.get(idx).cloned() else { return };
        let before = self.tabs.len();
        let opened = self.open_workspace(&ws);
        if opened > 0 {
            self.view = View::Term(before);
            self.toast(ToastLevel::Ok, format!("workspace '{}' · {opened} tabs", ws.name));
        }
    }
}
