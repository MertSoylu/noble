//! Pure binary split tree: pane placement, splitting, closing, resizing and
//! directional focus moves. It knows nothing about PTYs or the UI; fully testable.

use ratatui::layout::Rect;
use serde::{Deserialize, Serialize};

pub type PaneId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dir {
    /// Side by side (vertical divider).
    Row,
    /// Stacked (horizontal divider).
    Col,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Leaf(PaneId),
    Split { dir: Dir, ratio: f32, a: Box<Node>, b: Box<Node> },
}

/// A mouse-draggable divider: its path in the tree and the area the split covers.
#[derive(Clone, Debug, PartialEq)]
pub struct Divider {
    pub path: Vec<bool>,
    pub dir: Dir,
    /// Area of the split (sum of both children); the ratio is computed against it.
    pub area: Rect,
    /// The thin clickable strip.
    pub hit: Rect,
}

pub const MIN_W: u16 = 12;
pub const MIN_H: u16 = 4;

impl Node {
    pub fn leaves(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect(&self, out: &mut Vec<PaneId>) {
        match self {
            Node::Leaf(id) => out.push(*id),
            Node::Split { a, b, .. } => {
                a.collect(out);
                b.collect(out);
            }
        }
    }

    pub fn contains(&self, id: PaneId) -> bool {
        match self {
            Node::Leaf(x) => *x == id,
            Node::Split { a, b, .. } => a.contains(id) || b.contains(id),
        }
    }

    /// Splits the `target` leaf in two; the new pane goes to side `b`.
    pub fn split(&mut self, target: PaneId, new: PaneId, dir: Dir) -> bool {
        match self {
            Node::Leaf(id) if *id == target => {
                *self = Node::Split { dir, ratio: 0.5, a: Box::new(Node::Leaf(target)), b: Box::new(Node::Leaf(new)) };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.split(target, new, dir) || b.split(target, new, dir),
        }
    }

    /// Removes the `target` leaf and puts its sibling in the parent's place.
    /// The root leaf cannot be removed (the caller should close the tab); returns `false`.
    pub fn remove(&mut self, target: PaneId) -> bool {
        match self {
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => {
                if matches!(**a, Node::Leaf(id) if id == target) {
                    let sibling = std::mem::replace(&mut **b, Node::Leaf(0));
                    *self = sibling;
                    return true;
                }
                if matches!(**b, Node::Leaf(id) if id == target) {
                    let sibling = std::mem::replace(&mut **a, Node::Leaf(0));
                    *self = sibling;
                    return true;
                }
                a.remove(target) || b.remove(target)
            }
        }
    }

    /// Computes every pane's rectangle and the dividers.
    pub fn layout(&self, area: Rect) -> (Vec<(PaneId, Rect)>, Vec<Divider>) {
        let mut panes = Vec::new();
        let mut dividers = Vec::new();
        self.layout_into(area, &mut Vec::new(), &mut panes, &mut dividers);
        (panes, dividers)
    }

    fn layout_into(
        &self,
        area: Rect,
        path: &mut Vec<bool>,
        panes: &mut Vec<(PaneId, Rect)>,
        dividers: &mut Vec<Divider>,
    ) {
        match self {
            Node::Leaf(id) => panes.push((*id, area)),
            Node::Split { dir, ratio, a, b } => {
                let (ra, rb) = split_rect(area, *dir, *ratio);
                let hit = match dir {
                    // Divider: the two columns/rows where the pane frames meet.
                    Dir::Row => Rect::new(ra.right().saturating_sub(1), area.y, 2.min(area.width), area.height),
                    // Only the top pane's bottom edge, so the lower pane's title buttons stay clickable.
                    Dir::Col => Rect::new(area.x, ra.bottom().saturating_sub(1), area.width, 1.min(area.height)),
                };
                dividers.push(Divider { path: path.clone(), dir: *dir, area, hit });
                path.push(false);
                a.layout_into(ra, path, panes, dividers);
                path.pop();
                path.push(true);
                b.layout_into(rb, path, panes, dividers);
                path.pop();
            }
        }
    }

    fn at_path_mut(&mut self, path: &[bool]) -> Option<&mut Node> {
        match path.split_first() {
            None => Some(self),
            Some((first, rest)) => match self {
                Node::Leaf(_) => None,
                Node::Split { a, b, .. } => {
                    if *first {
                        b.at_path_mut(rest)
                    } else {
                        a.at_path_mut(rest)
                    }
                }
            },
        }
    }

    pub fn set_ratio(&mut self, path: &[bool], new_ratio: f32) {
        if let Some(Node::Split { ratio, .. }) = self.at_path_mut(path) {
            *ratio = new_ratio.clamp(0.1, 0.9);
        }
    }

    /// Moves the divider of the deepest split surrounding the target pane along
    /// the given axis by `delta`. `false` when there is no suitable split.
    pub fn nudge(&mut self, target: PaneId, dir: Dir, delta: f32) -> bool {
        match self {
            Node::Leaf(_) => false,
            Node::Split { dir: d, ratio, a, b } => {
                let inner = if a.contains(target) {
                    a.nudge(target, dir, delta)
                } else if b.contains(target) {
                    b.nudge(target, dir, delta)
                } else {
                    return false;
                };
                if inner {
                    return true;
                }
                if *d == dir {
                    *ratio = (*ratio + delta).clamp(0.1, 0.9);
                    return true;
                }
                false
            }
        }
    }
}

/// Splits an area by ratio; each side gets at least 1 cell.
pub fn split_rect(area: Rect, dir: Dir, ratio: f32) -> (Rect, Rect) {
    match dir {
        Dir::Row => {
            let total = area.width;
            let wa = ((total as f32 * ratio).round() as u16).clamp(1.min(total), total.saturating_sub(1).max(1));
            let wa = wa.min(total);
            (Rect::new(area.x, area.y, wa, area.height), Rect::new(area.x + wa, area.y, total - wa, area.height))
        }
        Dir::Col => {
            let total = area.height;
            let ha = ((total as f32 * ratio).round() as u16).clamp(1.min(total), total.saturating_sub(1).max(1));
            let ha = ha.min(total);
            (Rect::new(area.x, area.y, area.width, ha), Rect::new(area.x, area.y + ha, area.width, total - ha))
        }
    }
}

/// Computes the new ratio from the mouse position while a divider is dragged.
pub fn ratio_from_point(div: &Divider, x: u16, y: u16) -> f32 {
    match div.dir {
        Dir::Row => {
            let rel = x.saturating_sub(div.area.x) as f32 + 1.0;
            rel / div.area.width.max(1) as f32
        }
        Dir::Col => {
            let rel = y.saturating_sub(div.area.y) as f32 + 1.0;
            rel / div.area.height.max(1) as f32
        }
    }
}

/// Focus move: the nearest pane in the given direction that overlaps on the perpendicular axis.
pub fn neighbor(rects: &[(PaneId, Rect)], from: PaneId, dir: Direction) -> Option<PaneId> {
    let cur = rects.iter().find(|(id, _)| *id == from)?.1;
    let overlap = |a0: u16, a1: u16, b0: u16, b1: u16| a1.min(b1) as i32 - a0.max(b0) as i32;
    rects
        .iter()
        .filter(|(id, _)| *id != from)
        .filter_map(|(id, r)| {
            let (dist, ov) = match dir {
                Direction::Right if r.x >= cur.right() => {
                    (r.x as i32 - cur.right() as i32, overlap(cur.y, cur.bottom(), r.y, r.bottom()))
                }
                Direction::Left if r.right() <= cur.x => {
                    (cur.x as i32 - r.right() as i32, overlap(cur.y, cur.bottom(), r.y, r.bottom()))
                }
                Direction::Down if r.y >= cur.bottom() => {
                    (r.y as i32 - cur.bottom() as i32, overlap(cur.x, cur.right(), r.x, r.right()))
                }
                Direction::Up if r.bottom() <= cur.y => {
                    (cur.y as i32 - r.bottom() as i32, overlap(cur.x, cur.right(), r.x, r.right()))
                }
                _ => return None,
            };
            (ov > 0).then_some((*id, dist, ov))
        })
        .min_by_key(|(_, dist, ov)| (*dist, -*ov))
        .map(|(id, _, _)| id)
}

/// Persistable (serializable) tree: leaves carry launch info instead of a pane.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SavedNode {
    Leaf {
        cwd: String,
        /// Quick-launch command this pane was opened with (e.g. `claude`); run again on restore.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        launch: Option<String>,
    },
    Split {
        dir: Dir,
        ratio: f32,
        a: Box<SavedNode>,
        b: Box<SavedNode>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rect::new(0, 0, 100, 40)
    }

    #[test]
    fn split_and_layout() {
        let mut n = Node::Leaf(1);
        assert!(n.split(1, 2, Dir::Row));
        assert!(n.split(2, 3, Dir::Col));
        assert_eq!(n.leaves(), vec![1, 2, 3]);
        let (panes, dividers) = n.layout(area());
        assert_eq!(panes.len(), 3);
        assert_eq!(dividers.len(), 2);
        let r1 = panes[0].1;
        let r2 = panes[1].1;
        let r3 = panes[2].1;
        assert_eq!(r1, Rect::new(0, 0, 50, 40));
        assert_eq!(r2, Rect::new(50, 0, 50, 20));
        assert_eq!(r3, Rect::new(50, 20, 50, 20));
        // The area is fully covered.
        let total: u32 = panes.iter().map(|(_, r)| r.width as u32 * r.height as u32).sum();
        assert_eq!(total, 100 * 40);
    }

    #[test]
    fn remove_collapses_parent() {
        let mut n = Node::Leaf(1);
        n.split(1, 2, Dir::Row);
        n.split(2, 3, Dir::Col);
        assert!(n.remove(2));
        assert_eq!(n.leaves(), vec![1, 3]);
        assert!(n.remove(1));
        assert_eq!(n, Node::Leaf(3));
        assert!(!n.remove(3));
    }

    #[test]
    fn focus_navigation() {
        let mut n = Node::Leaf(1);
        n.split(1, 2, Dir::Row);
        n.split(2, 3, Dir::Col);
        let (panes, _) = n.layout(area());
        assert_eq!(neighbor(&panes, 1, Direction::Right), Some(2));
        assert_eq!(neighbor(&panes, 3, Direction::Up), Some(2));
        assert_eq!(neighbor(&panes, 3, Direction::Left), Some(1));
        assert_eq!(neighbor(&panes, 1, Direction::Left), None);
        assert_eq!(neighbor(&panes, 2, Direction::Down), Some(3));
    }

    #[test]
    fn nudge_and_drag() {
        let mut n = Node::Leaf(1);
        n.split(1, 2, Dir::Row);
        assert!(n.nudge(1, Dir::Row, 0.1));
        assert!(!n.nudge(1, Dir::Col, 0.1));
        if let Node::Split { ratio, .. } = &n {
            assert!((ratio - 0.6).abs() < 1e-6);
        }
        let (_, dividers) = n.layout(area());
        let r = ratio_from_point(&dividers[0], 24, 0);
        n.set_ratio(&dividers[0].path, r);
        let (panes, _) = n.layout(area());
        assert_eq!(panes[0].1.width, 25);
        n.set_ratio(&[], 5.0);
        if let Node::Split { ratio, .. } = &n {
            assert!((ratio - 0.9).abs() < 1e-6);
        }
    }

    #[test]
    fn tiny_areas_do_not_panic() {
        let mut n = Node::Leaf(1);
        n.split(1, 2, Dir::Row);
        n.split(2, 3, Dir::Row);
        for w in 0..5 {
            let (panes, _) = n.layout(Rect::new(0, 0, w, 3));
            assert_eq!(panes.len(), 3);
        }
    }
}
