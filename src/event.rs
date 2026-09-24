//! Events arriving from the background threads into the main loop.

use std::path::PathBuf;

use crate::ai::ProviderState;
use crate::projects::{GitInfo, Project};
use crate::sensors::{SensorSample, StaticInfo};
use crate::term::layout::PaneId;

pub enum AppEvent {
    Input(crossterm::event::Event),
    /// Output arrived in one or more panes (a per-pane `dirty` flag is kept).
    PtyOutput,
    PtyExit(PaneId),
    SensorStatic(Box<StaticInfo>),
    Sensors(Box<SensorSample>),
    KillResult {
        pid: u32,
        name: String,
        ok: bool,
    },
    Projects(Vec<Project>),
    Git(PathBuf, GitInfo),
    Ai(Box<ProviderState>),
    /// Update check: the latest published release or an error.
    Update(Result<String, String>),
}

pub type Tx = std::sync::mpsc::Sender<AppEvent>;
