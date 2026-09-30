//! Terminal engine: PTY panes, split tree and input encoding.

pub mod input;
pub mod integration;
pub mod layout;
pub mod link;
pub mod pane;

use std::sync::atomic::{AtomicU64, Ordering};

use layout::{Node, PaneId};

/// Stable identity of a tab for the whole run: unlike an index it survives tabs closing or moving,
/// and unlike a pane id it survives the tab's panes closing.
pub type TabId = u64;

static NEXT_TAB_ID: AtomicU64 = AtomicU64::new(1);

/// A terminal tab: split tree + focus + maximized state.
#[derive(Clone, Debug)]
pub struct Tab {
    pub id: TabId,
    pub root: Node,
    pub focus: PaneId,
    pub zoomed: bool,
    /// User-given name; derived from the focused pane when absent.
    pub name: Option<String>,
    /// Project/directory name the tab was opened in (for the automatic title).
    pub origin: String,
    /// Opened by a launcher (quick launch, `git pull`, an editor …): `origin` ends in " · <launcher>".
    /// Kept on the tab so the title still drops that part after the launcher's pane is closed.
    pub launched: bool,
    /// Did output arrive in the background (dot on the tab strip).
    pub activity: bool,
    /// What happened in the background that the user has not seen yet (the marker after the tab title).
    pub alert: Option<TabAlert>,
}

/// Kinds of background event a tab marker reports. The order is the priority when several happen
/// before the tab is visited: a later variant replaces an earlier one, never the other way round.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TabAlert {
    /// A long command finished successfully (✓).
    Done,
    /// A background command ended with a non-zero exit code (✗).
    Failed,
    /// The app sent a notification, the bell rang or an agent needs attention (◆).
    Notice,
}

impl Tab {
    pub fn new(pane: PaneId, origin: String) -> Tab {
        Tab {
            id: NEXT_TAB_ID.fetch_add(1, Ordering::Relaxed),
            root: Node::Leaf(pane),
            focus: pane,
            zoomed: false,
            name: None,
            origin,
            launched: false,
            activity: false,
            alert: None,
        }
    }

    pub fn panes(&self) -> Vec<PaneId> {
        self.root.leaves()
    }

    /// Records a background event; a more urgent kind that is already there stays.
    pub fn raise(&mut self, kind: TabAlert) {
        self.alert = self.alert.max(Some(kind));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_ids_are_unique() {
        let (a, b) = (Tab::new(1, "a".into()), Tab::new(1, "a".into()));
        assert_ne!(a.id, b.id, "two tabs got the same id");
    }
}
