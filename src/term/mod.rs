//! Terminal engine: PTY panes, split tree and input encoding.

pub mod input;
pub mod layout;
pub mod link;
pub mod pane;

use layout::{Node, PaneId};

/// A terminal tab: split tree + focus + maximized state.
#[derive(Clone, Debug)]
pub struct Tab {
    pub root: Node,
    pub focus: PaneId,
    pub zoomed: bool,
    /// User-given name; derived from the focused pane when absent.
    pub name: Option<String>,
    /// Project/directory name the tab was opened in (for the automatic title).
    pub origin: String,
    /// Did output arrive in the background (dot on the tab strip).
    pub activity: bool,
    /// Needs attention: a long command finished, the bell rang or the app sent a notification.
    pub alert: bool,
}

impl Tab {
    pub fn new(pane: PaneId, origin: String) -> Tab {
        Tab { root: Node::Leaf(pane), focus: pane, zoomed: false, name: None, origin, activity: false, alert: false }
    }

    pub fn panes(&self) -> Vec<PaneId> {
        self.root.leaves()
    }
}
