//! Arka plan iş parçacıklarından ana döngüye gelen olaylar.

use std::path::PathBuf;

use crate::ai::ProviderState;
use crate::projects::{GitInfo, Project};
use crate::sensors::{SensorSample, StaticInfo};
use crate::term::layout::PaneId;

pub enum AppEvent {
    Input(crossterm::event::Event),
    /// Bir veya daha fazla pane'e çıktı geldi (pane başına `dirty` bayrağı tutulur).
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
}

pub type Tx = std::sync::mpsc::Sender<AppEvent>;
