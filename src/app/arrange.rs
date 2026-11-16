//! Rearranging the panes of a tab: swaps (with the next or previous pane, or the neighbour on a side), moves next to
//! another pane or to an edge of the tab, rotation, turning a split the other way, and moving a pane into another tab.
//! A drag of a pane's title does the same with the mouse (`Drag::Pane`): `drop_at` says what a release there would
//! do, `drop_preview` where the pane would land, `apply_drop` does it. The shells keep running wherever they go.

use ratatui::layout::Rect;

use super::{App, Hit, View};
use crate::term::layout::{Direction, Node, PaneId, frame_rect, neighbor};
use crate::term::{Tab, TabId};

/// What a release of a dragged pane does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneDrop {
    /// Trade places with this pane.
    Swap(PaneId),
    /// Go next to this pane, on that side of it (it gives up half its size).
    Beside(PaneId, Direction),
    /// Go along that whole side of the tab.
    Edge(Direction),
    /// Join this tab (along its right side).
    Tab(TabId),
    /// Open a tab of its own (the tab strip's +).
    NewTab,
}

/// Share of a pane's width or height, from each side, where a drop goes next to it rather than in its place.
const SIDE_ZONE: f32 = 0.25;

impl App {
    /// Runs `change` on a copy of the visible tab's tree for its focused pane and keeps the result when it changed
    /// what is shown; otherwise `why` explains (a toast) and nothing moves. The pane keeps the focus and lights up.
    fn arrange(&mut self, why: &str, change: impl FnOnce(&mut Node, PaneId, &[(PaneId, Rect)]) -> bool) {
        let body = self.body();
        let Some(tab) = self.current_tab() else {
            return self.no_op("no pane here — open a terminal tab first");
        };
        if tab.panes().len() < 2 {
            return self.no_op("only one pane — nothing to arrange");
        }
        let pane = tab.focus;
        let rects = tab.root.layout(body).0;
        let mut next = tab.root.clone();
        if !change(&mut next, pane, &rects) || next.same_shape(&tab.root) {
            return self.no_op(why);
        }
        self.set_tree(pane, next);
    }

    /// Puts the new tree in place in the tab of `pane`, which gets the focus; a zoomed tab shows its layout again.
    fn set_tree(&mut self, pane: PaneId, root: Node) {
        let Some(ti) = self.tab_of(pane) else { return };
        let tab = &mut self.tabs[ti];
        tab.root = root;
        tab.focus = pane;
        tab.zoomed = false;
        // A zoom animation still running would draw its pane over the tiled layout.
        self.zoom_anim = None;
        self.fx.start(super::fx::Fx::Focus(pane));
    }

    /// Swaps the focused pane with the one `step` places along the pane order (wrapping around).
    pub fn swap_step(&mut self, step: isize) {
        self.arrange("only one pane — nothing to swap", |root, pane, _| {
            let panes = root.leaves();
            let i = panes.iter().position(|p| *p == pane).unwrap_or(0) as isize;
            let other = panes[(i + step).rem_euclid(panes.len() as isize) as usize];
            root.swap(pane, other)
        });
    }

    /// Swaps the focused pane with its neighbour on side `d` (the one focus would move to).
    pub fn swap_dir(&mut self, d: Direction) {
        let why = format!("no pane {} this one", side_words(d));
        self.arrange(&why, |root, pane, rects| neighbor(rects, pane, d).is_some_and(|n| root.swap(pane, n)));
    }

    /// Moves the focused pane along the whole side `d` of the tab.
    pub fn move_to_edge(&mut self, d: Direction) {
        let why = format!("the pane already spans the {} edge", d.edge_name());
        self.arrange(&why, |root, pane, _| root.move_to_edge(pane, d));
    }

    /// Moves every pane of the tab one place along the pane order; the focus stays with its pane.
    pub fn rotate_panes(&mut self, forward: bool) {
        self.arrange("only one pane — nothing to rotate", |root, _, _| root.rotate(forward));
    }

    /// Turns the split that holds the focused pane the other way (side by side ↔ stacked).
    pub fn flip_split(&mut self) {
        self.arrange("only one pane — nothing to flip", |root, pane, _| root.flip(pane));
    }

    /// Moves pane `id` into tab `dest` (along its right side) and shows it there. A pane that is alone in its tab takes
    /// the tab along: the two tabs become one.
    pub fn move_pane_into_tab(&mut self, id: PaneId, dest: TabId) {
        let (Some(src), Some(mut to)) = (self.tab_of(id), self.tab_index(dest)) else { return };
        if src == to {
            return self.no_op("the pane is already in that tab");
        }
        if self.tabs[src].panes().len() > 1 {
            self.detach_pane(src, id);
        } else {
            self.take_tab_out(src);
            if src < to {
                to -= 1;
            }
        }
        let tab = &mut self.tabs[to];
        tab.root.add_at_edge(id, Direction::Right);
        tab.focus = id;
        tab.zoomed = false;
        tab.activity = false;
        tab.alert = None;
        self.zoom_anim = None;
        self.view = View::Term(to);
        self.fx.start(super::fx::Fx::Focus(id));
    }

    /// Takes tab `i` out of the list without ending its panes (they go to another tab); the visible tab stays.
    fn take_tab_out(&mut self, i: usize) -> Tab {
        let tab = self.tabs.remove(i);
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
        tab
    }

    /// What releasing the dragged `pane` at (x, y) would do; `None` when it would change nothing. Over the tab strip:
    /// another tab, or + for a tab of its own. Over the panes: the outermost cells of the tab are its edges, the outer
    /// quarter of a pane puts the dragged one next to it on that side, and the middle swaps the two.
    pub fn drop_at(&self, pane: PaneId, x: u16, y: u16) -> Option<PaneDrop> {
        let ti = self.tab_of(pane)?;
        match self.hit_at(x, y) {
            Some(Hit::Tab(i)) => return (i != ti).then(|| PaneDrop::Tab(self.tabs[i].id)),
            Some(Hit::NewTab) => return (self.tabs[ti].panes().len() > 1).then_some(PaneDrop::NewTab),
            _ => {}
        }
        if self.view != View::Term(ti) || self.tabs[ti].zoomed {
            return None;
        }
        let body = self.body();
        if !body.contains(ratatui::layout::Position { x, y }) {
            return None;
        }
        let edge_w = if body.width >= 60 { 2 } else { 1 };
        let edge = if x < body.x + edge_w {
            Some(Direction::Left)
        } else if x >= body.right().saturating_sub(edge_w) {
            Some(Direction::Right)
        } else if y == body.y {
            Some(Direction::Up)
        } else if y + 1 >= body.bottom() {
            Some(Direction::Down)
        } else {
            None
        };
        let drop = match edge {
            Some(d) => PaneDrop::Edge(d),
            None => {
                let rects = self.tabs[ti].root.layout(body).0;
                let (target, r) = rects.into_iter().find(|(_, r)| r.contains(ratatui::layout::Position { x, y }))?;
                if target == pane {
                    return None;
                }
                let fx = (f32::from(x - r.x) + 0.5) / f32::from(r.width.max(1));
                let fy = (f32::from(y - r.y) + 0.5) / f32::from(r.height.max(1));
                let sides = [
                    (Direction::Left, fx),
                    (Direction::Right, 1.0 - fx),
                    (Direction::Up, fy),
                    (Direction::Down, 1.0 - fy),
                ];
                let (side, near) = sides.into_iter().min_by(|a, b| a.1.total_cmp(&b.1))?;
                if near < SIDE_ZONE { PaneDrop::Beside(target, side) } else { PaneDrop::Swap(target) }
            }
        };
        self.dropped_tree(pane, drop).map(|_| drop)
    }

    /// The tree of the dragged pane's tab after `drop` (one inside the tab); `None` when it would look the same.
    fn dropped_tree(&self, pane: PaneId, drop: PaneDrop) -> Option<Node> {
        let tab = &self.tabs[self.tab_of(pane)?];
        let mut next = tab.root.clone();
        let changed = match drop {
            PaneDrop::Swap(other) => next.swap(pane, other),
            PaneDrop::Beside(target, side) => next.move_beside(pane, target, side),
            PaneDrop::Edge(side) => next.move_to_edge(pane, side),
            PaneDrop::Tab(_) | PaneDrop::NewTab => return None,
        };
        (changed && !next.same_shape(&tab.root)).then_some(next)
    }

    /// Where the dragged pane's frame would be after `drop` (one inside the tab), for the preview.
    pub fn drop_preview(&self, pane: PaneId, drop: PaneDrop) -> Option<Rect> {
        let body = self.body();
        let tree = self.dropped_tree(pane, drop)?;
        tree.layout(body).0.into_iter().find(|(id, _)| *id == pane).map(|(_, r)| frame_rect(r, body))
    }

    /// What a release would do, for the status bar ("swap with pwsh", "to the left edge" …).
    pub fn drop_label(&self, drop: PaneDrop) -> String {
        let name = |p: PaneId| self.panes.get(&p).map(|p| p.label()).unwrap_or_default();
        match drop {
            PaneDrop::Swap(p) => format!("swap with {}", name(p)),
            PaneDrop::Beside(p, d) => format!("{} {}", side_words(d), name(p)),
            PaneDrop::Edge(d) => format!("along the {} edge", d.edge_name()),
            PaneDrop::Tab(t) => match self.tab_index(t) {
                Some(i) => format!("into tab {}", self.tab_title(i)),
                None => "into the tab".into(),
            },
            PaneDrop::NewTab => "into a new tab".into(),
        }
    }

    /// Releases the dragged `pane` with `drop`.
    pub fn apply_drop(&mut self, pane: PaneId, drop: PaneDrop) {
        match drop {
            PaneDrop::Tab(t) => self.move_pane_into_tab(pane, t),
            PaneDrop::NewTab => self.move_pane_to_tab(pane),
            _ => {
                if let Some(tree) = self.dropped_tree(pane, drop) {
                    self.set_tree(pane, tree);
                }
            }
        }
    }
}

/// "left of", "right of", "above", "below": where something is next to a pane.
fn side_words(d: Direction) -> &'static str {
    match d {
        Direction::Left => "left of",
        Direction::Right => "right of",
        Direction::Up => "above",
        Direction::Down => "below",
    }
}
