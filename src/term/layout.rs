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

impl Direction {
    pub const ALL: [Direction; 4] = [Direction::Left, Direction::Right, Direction::Up, Direction::Down];

    /// The split that puts something on this side: side by side for left and right, stacked otherwise.
    pub fn axis(self) -> Dir {
        match self {
            Direction::Left | Direction::Right => Dir::Row,
            Direction::Up | Direction::Down => Dir::Col,
        }
    }

    /// The side comes first in its split (`a`): left or top.
    fn first(self) -> bool {
        matches!(self, Direction::Left | Direction::Up)
    }

    /// The side's name for the UI ("left", "top" …).
    pub fn edge_name(self) -> &'static str {
        match self {
            Direction::Left => "left",
            Direction::Right => "right",
            Direction::Up => "top",
            Direction::Down => "bottom",
        }
    }
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

    /// Exchanges the places of two panes in the tree; the sizes stay where they are.
    /// `false` (and unchanged) unless both are leaves of this tree.
    pub fn swap(&mut self, x: PaneId, y: PaneId) -> bool {
        if x == y || !self.contains(x) || !self.contains(y) {
            return false;
        }
        self.swap_ids(x, y);
        true
    }

    fn swap_ids(&mut self, x: PaneId, y: PaneId) {
        match self {
            Node::Leaf(id) if *id == x => *id = y,
            Node::Leaf(id) if *id == y => *id = x,
            Node::Leaf(_) => {}
            Node::Split { a, b, .. } => {
                a.swap_ids(x, y);
                b.swap_ids(x, y);
            }
        }
    }

    /// Puts the new leaf `new` on side `side` of the `target` leaf, which gives up `share` (0..1) of its size to it.
    pub fn insert_beside(&mut self, target: PaneId, new: PaneId, side: Direction, share: f32) -> bool {
        match self {
            Node::Leaf(id) if *id == target => {
                let old = std::mem::replace(self, Node::Leaf(0));
                *self = Node::wrapped(old, new, side, share);
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => {
                a.insert_beside(target, new, side, share) || b.insert_beside(target, new, side, share)
            }
        }
    }

    /// `node` with the leaf `new` added on side `side` across its whole length, taking `share` of it.
    fn wrapped(node: Node, new: PaneId, side: Direction, share: f32) -> Node {
        let share = share.clamp(0.05, 0.95);
        let leaf = Box::new(Node::Leaf(new));
        let (ratio, a, b) =
            if side.first() { (share, leaf, Box::new(node)) } else { (1.0 - share, Box::new(node), leaf) };
        Node::Split { dir: side.axis(), ratio, a, b }
    }

    /// Moves pane `id` next to pane `target`, on its side `side`; the target gives up half its size. The tree is
    /// unchanged (`false`) unless both are different leaves of it.
    pub fn move_beside(&mut self, id: PaneId, target: PaneId, side: Direction) -> bool {
        if id == target || !self.contains(id) || !self.contains(target) {
            return false;
        }
        let mut next = self.clone();
        if !(next.remove(id) && next.insert_beside(target, id, side, 0.5)) {
            return false;
        }
        *self = next;
        true
    }

    /// Moves pane `id` to side `side` of the whole tree, across its full height (left/right) or width (top/bottom),
    /// with an even share among the parts on that axis. `false` (unchanged) for a lone pane or one not in the tree.
    pub fn move_to_edge(&mut self, id: PaneId, side: Direction) -> bool {
        let mut rest = self.clone();
        if !rest.remove(id) {
            return false;
        }
        rest.add_at_edge(id, side);
        *self = rest;
        true
    }

    /// Adds the new leaf `new` on side `side` of the whole tree (see `move_to_edge`).
    pub fn add_at_edge(&mut self, new: PaneId, side: Direction) {
        let parts = self.weight(side.axis()) + 1;
        let old = std::mem::replace(self, Node::Leaf(0));
        *self = Node::wrapped(old, new, side, 1.0 / parts as f32);
    }

    /// Moves every pane one place along the pane order (the last to the first with `forward`); the sizes stay.
    pub fn rotate(&mut self, forward: bool) -> bool {
        let mut ids = self.leaves();
        if ids.len() < 2 {
            return false;
        }
        if forward {
            ids.rotate_right(1);
        } else {
            ids.rotate_left(1);
        }
        self.relabel(&mut ids.into_iter());
        true
    }

    fn relabel(&mut self, ids: &mut impl Iterator<Item = PaneId>) {
        match self {
            Node::Leaf(id) => *id = ids.next().unwrap_or(*id),
            Node::Split { a, b, .. } => {
                a.relabel(ids);
                b.relabel(ids);
            }
        }
    }

    /// Turns the split that holds pane `id` the other way: side by side becomes stacked and back. The order and the
    /// ratio stay (left becomes top). `false` for a lone pane.
    pub fn flip(&mut self, id: PaneId) -> bool {
        match self {
            Node::Leaf(_) => false,
            Node::Split { dir, a, b, .. } => {
                if a.contains(id) && a.flip(id) || b.contains(id) && b.flip(id) {
                    return true;
                }
                if !matches!(**a, Node::Leaf(x) if x == id) && !matches!(**b, Node::Leaf(x) if x == id) {
                    return false;
                }
                *dir = match dir {
                    Dir::Row => Dir::Col,
                    Dir::Col => Dir::Row,
                };
                true
            }
        }
    }

    /// Same panes in the same places, ratios aside: a move that would change nothing visible but the sizes.
    pub fn same_shape(&self, other: &Node) -> bool {
        match (self, other) {
            (Node::Leaf(x), Node::Leaf(y)) => x == y,
            (Node::Split { dir: d1, a: a1, b: b1, .. }, Node::Split { dir: d2, a: a2, b: b2, .. }) => {
                d1 == d2 && a1.same_shape(a2) && b1.same_shape(b2)
            }
            _ => false,
        }
    }

    /// Gives every pane the same share of its axis: a run of splits in one direction (`a | (b | c)`)
    /// divides the space evenly between its parts, the way tmux's even layouts do. The ratios are not
    /// clamped like `set_ratio` and `nudge`: both sides weigh at least one part, so each ratio stays
    /// strictly inside 0..1, and `split_rect` keeps every side at least one cell.
    pub fn equalize(&mut self) {
        if let Node::Split { dir, ratio, a, b } = self {
            let (wa, wb) = (a.weight(*dir), b.weight(*dir));
            *ratio = wa as f32 / (wa + wb) as f32;
            a.equalize();
            b.equalize();
        }
    }

    /// How many parts this node adds to a run of splits in direction `dir`.
    fn weight(&self, dir: Dir) -> usize {
        match self {
            Node::Split { dir: d, a, b, .. } if *d == dir => a.weight(dir) + b.weight(dir),
            _ => 1,
        }
    }

    /// A copy in which pane `id` gets only `share` (0..1) of its size within its split: a new pane
    /// unfolding from the divider. Unchanged when `id` is not directly in a split.
    pub fn growing(&self, id: PaneId, share: f32) -> Node {
        let mut node = self.clone();
        node.grow_into(id, share.clamp(0.0, 1.0));
        node
    }

    fn grow_into(&mut self, id: PaneId, share: f32) -> bool {
        let Node::Split { ratio, a, b, .. } = self else { return false };
        if matches!(**b, Node::Leaf(x) if x == id) {
            *ratio = 1.0 - (1.0 - *ratio) * share;
            return true;
        }
        if matches!(**a, Node::Leaf(x) if x == id) {
            *ratio *= share;
            return true;
        }
        a.grow_into(id, share) || b.grow_into(id, share)
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
                // Divider: the border line the two sides share (see `frame_rect`). For a
                // horizontal split that row is also the lower pane's title row; the title
                // text and buttons are pushed after the dividers and stay clickable.
                let hit = match dir {
                    Dir::Row => Rect::new(ra.right().saturating_sub(1), area.y, 1.min(area.width), area.height),
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

/// The rect a pane draws its frame in. A tile with a neighbour on its left or top reaches one
/// cell into it, so two neighbours share a single border line instead of drawing two. `area`
/// is the whole tab area the tiles were laid out in.
pub fn frame_rect(tile: Rect, area: Rect) -> Rect {
    let dx = u16::from(tile.x > area.x);
    let dy = u16::from(tile.y > area.y);
    Rect::new(tile.x - dx, tile.y - dy, tile.width + dx, tile.height + dy)
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
        /// The agent conversation the pane was running (its session id); restored by adding the agent's
        /// resume arguments to `launch`. Files written before this existed have none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resume: Option<crate::hooks::resume::Resume>,
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

    /// A session file from before `resume` existed loads with none, and writing one without a resume adds no key.
    #[test]
    fn saved_leaf_without_resume_loads_and_writes_as_before() {
        let old: SavedNode = serde_json::from_str(r#"{"type":"leaf","cwd":"C:\\x","launch":"claude"}"#).unwrap();
        assert_eq!(old, SavedNode::Leaf { cwd: "C:\\x".into(), launch: Some("claude".into()), resume: None });
        let text = serde_json::to_string(&old).unwrap();
        assert!(!text.contains("resume"), "no resume key without one: {text}");
    }

    fn area() -> Rect {
        Rect::new(0, 0, 100, 40)
    }

    #[test]
    fn growing_pane_unfolds() {
        let mut root = Node::Leaf(1);
        root.split(1, 2, Dir::Row);
        let area = Rect::new(0, 0, 100, 10);
        let width = |n: &Node, id| n.layout(area).0.into_iter().find(|(p, _)| *p == id).map(|(_, r)| r.width);
        assert_eq!(width(&root.growing(2, 1.0), 2), width(&root, 2));
        assert!(width(&root.growing(2, 0.2), 2) < Some(15));
        assert!(width(&root.growing(1, 0.2), 1) < Some(15));
        // Not a pane of the tree: unchanged.
        assert_eq!(root.growing(9, 0.0), root);
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
    fn neighbours_share_a_border() {
        let mut n = Node::Leaf(1);
        n.split(1, 2, Dir::Row);
        n.split(2, 3, Dir::Col);
        let (panes, dividers) = n.layout(area());
        let frames: Vec<Rect> = panes.iter().map(|(_, t)| frame_rect(*t, area())).collect();
        assert_eq!(frames[0], Rect::new(0, 0, 50, 40));
        // The right panes start on the left pane's right border; the lower one on the upper's bottom border.
        assert_eq!(frames[1], Rect::new(49, 0, 51, 20));
        assert_eq!(frames[2], Rect::new(49, 19, 51, 21));
        // The dividers are exactly those shared lines.
        assert_eq!(dividers[0].hit, Rect::new(49, 0, 1, 40));
        assert_eq!(dividers[1].hit, Rect::new(50, 19, 50, 1));
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
    fn swap_exchanges_places_and_keeps_sizes() {
        let mut n = Node::Leaf(1);
        n.split(1, 2, Dir::Row);
        n.split(2, 3, Dir::Col);
        n.set_ratio(&[], 0.3);
        let before = n.layout(area()).0;
        assert!(n.swap(1, 3));
        assert_eq!(n.leaves(), vec![3, 2, 1]);
        let after = n.layout(area()).0;
        assert_eq!(after[0], (3, before[0].1));
        assert_eq!(after[2], (1, before[2].1));
        assert!(!n.swap(1, 1));
        assert!(!n.swap(1, 9));
        assert_eq!(n.leaves(), vec![3, 2, 1]);
    }

    /// A 2×2 grid: (1 / 2) | (3 / 4) — 1 top left, 2 bottom left, 3 top right, 4 bottom right.
    fn grid() -> Node {
        let mut n = Node::Leaf(1);
        n.split(1, 3, Dir::Row);
        n.split(1, 2, Dir::Col);
        n.split(3, 4, Dir::Col);
        n
    }

    fn rect_of(n: &Node, id: PaneId) -> Rect {
        n.layout(area()).0.into_iter().find(|(p, _)| *p == id).map(|(_, r)| r).unwrap()
    }

    #[test]
    fn move_beside_puts_a_pane_on_each_side_of_another() {
        // Bottom left to the right of the top right pane: the right column's top row is split in two.
        let mut n = grid();
        assert!(n.move_beside(2, 3, Direction::Right));
        assert_eq!(rect_of(&n, 1), Rect::new(0, 0, 50, 40), "1 takes the whole left column");
        assert_eq!(rect_of(&n, 2), Rect::new(75, 0, 25, 20));
        assert_eq!(rect_of(&n, 3), Rect::new(50, 0, 25, 20));
        for (side, expect) in [
            (Direction::Left, Rect::new(50, 0, 25, 20)),
            (Direction::Up, Rect::new(50, 0, 50, 10)),
            (Direction::Down, Rect::new(50, 10, 50, 10)),
        ] {
            let mut n = grid();
            assert!(n.move_beside(2, 3, side));
            assert_eq!(rect_of(&n, 2), expect, "{side:?}");
        }
        // Top and bottom of a two-pane stack swap places by moving one below the other.
        let mut n = Node::Leaf(1);
        n.split(1, 2, Dir::Col);
        assert!(n.move_beside(1, 2, Direction::Down));
        assert_eq!(n.leaves(), vec![2, 1]);
        // Nothing for a pane onto itself or an unknown pane.
        let mut n = grid();
        assert!(!n.move_beside(2, 2, Direction::Left));
        assert!(!n.move_beside(2, 9, Direction::Left));
        assert!(!n.move_beside(9, 2, Direction::Left));
        assert_eq!(n, grid());
    }

    #[test]
    fn move_to_edge_spans_the_whole_side() {
        for (side, expect) in [
            (Direction::Left, Rect::new(0, 0, 33, 40)),
            (Direction::Right, Rect::new(67, 0, 33, 40)),
            (Direction::Up, Rect::new(0, 0, 100, 20)),
            (Direction::Down, Rect::new(0, 20, 100, 20)),
        ] {
            let mut n = grid();
            assert!(n.move_to_edge(2, side));
            assert_eq!(rect_of(&n, 2), expect, "{side:?}");
            assert_eq!(n.leaves().len(), 4);
        }
        // A lone pane has no edge to move to.
        let mut leaf = Node::Leaf(1);
        assert!(!leaf.move_to_edge(1, Direction::Left));
        assert!(!grid().move_to_edge(9, Direction::Left));
        // Already there: the shape stays.
        let mut n = Node::Leaf(1);
        n.split(1, 2, Dir::Row);
        let before = n.clone();
        assert!(n.move_to_edge(1, Direction::Left));
        assert!(n.same_shape(&before));
        assert!(!n.same_shape(&grid()));
    }

    #[test]
    fn rotate_and_flip() {
        let mut n = grid();
        let sizes = n.layout(area()).0.iter().map(|(_, r)| *r).collect::<Vec<_>>();
        assert!(n.rotate(true));
        assert_eq!(n.leaves(), vec![4, 1, 2, 3]);
        assert!(n.rotate(false));
        assert_eq!(n, grid());
        assert_eq!(n.layout(area()).0.iter().map(|(_, r)| *r).collect::<Vec<_>>(), sizes);
        assert!(!Node::Leaf(1).rotate(true));
        // The left column (1 / 2) turns into a row: 1 and 2 side by side in the left half.
        let mut n = grid();
        assert!(n.flip(2));
        assert_eq!(rect_of(&n, 1), Rect::new(0, 0, 25, 40));
        assert_eq!(rect_of(&n, 2), Rect::new(25, 0, 25, 40));
        assert_eq!(rect_of(&n, 3), Rect::new(50, 0, 50, 20), "the other column stays");
        assert!(n.flip(2));
        assert_eq!(n, grid());
        assert!(!Node::Leaf(1).flip(1));
        assert!(!grid().flip(9));
    }

    #[test]
    fn equalize_evens_out_runs_of_one_direction() {
        // 1 | (2 | 3) with a lopsided divider: three equal columns after.
        let mut n = Node::Leaf(1);
        n.split(1, 2, Dir::Row);
        n.split(2, 3, Dir::Row);
        n.set_ratio(&[], 0.8);
        n.equalize();
        let widths: Vec<u16> = n.layout(Rect::new(0, 0, 90, 10)).0.iter().map(|(_, r)| r.width).collect();
        assert_eq!(widths, vec![30, 30, 30]);
        // A split in the other direction counts as one part: 1 | (2 / 3) halves the width.
        let mut n = Node::Leaf(1);
        n.split(1, 2, Dir::Row);
        n.split(2, 3, Dir::Col);
        n.set_ratio(&[], 0.2);
        n.set_ratio(&[true], 0.9);
        n.equalize();
        let rects: Vec<Rect> = n.layout(area()).0.iter().map(|(_, r)| *r).collect();
        assert_eq!(rects, vec![Rect::new(0, 0, 50, 40), Rect::new(50, 0, 50, 20), Rect::new(50, 20, 50, 20)]);
        // A single pane is left alone.
        let mut leaf = Node::Leaf(1);
        leaf.equalize();
        assert_eq!(leaf, Node::Leaf(1));
        // Long runs stay even: 16 panes in a row get 1/16 each, not a clamped 0.1 for the outer divider.
        let mut row = Node::Leaf(1);
        for id in 2..=16 {
            row.split(id - 1, id, Dir::Row);
        }
        row.equalize();
        let widths: Vec<u16> = row.layout(Rect::new(0, 0, 160, 10)).0.iter().map(|(_, r)| r.width).collect();
        assert_eq!(widths, vec![10; 16]);
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
