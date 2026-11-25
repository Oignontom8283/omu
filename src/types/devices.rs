//! Descriptions of audio devices and applications, as the daemon reports them to its clients. They live here (not in
//! `hal`, which needs the audio engine) so that the command line and the window can read them without it.

use super::AudioFormat;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Microphones and other recording endpoints.
    Capture,
    /// Speakers and other playback endpoints.
    Render,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Stable id, usable in [`super::DeviceRef::Id`].
    pub id: String,
    pub name: String,
    pub direction: Direction,
    pub is_default: bool,
    /// The device's native format in shared mode, when known.
    pub format: Option<AudioFormat>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInfo {
    pub pid: u32,
    /// Executable file name, e.g. `chrome.exe`.
    pub name: String,
    /// Title of its audio session, when it has one.
    pub title: Option<String>,
    /// True while it is producing sound.
    pub active: bool,
}

/// What a backend can do.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Capabilities {
    pub devices: bool,
    pub app_capture: bool,
    pub virtual_devices: bool,
    /// Global keyboard shortcuts (the `Hotkey` trigger).
    pub hotkeys: bool,
}

/// The names of the virtual microphones and speakers a graph uses: what must never be forgotten when old devices are
/// cleaned up.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptDevices {
    pub microphones: Vec<String>,
    pub speakers: Vec<String>,
}

impl KeptDevices {
    /// The virtual devices of `graph`.
    pub fn of_graph(graph: &super::GraphSpec) -> KeptDevices {
        let mut kept = KeptDevices::default();
        // the blocks inside groups count too: a virtual microphone in a group is kept, not "old"
        for node in graph.leaf_nodes() {
            match &node.kind {
                super::NodeKind::VirtualMic { name } => kept.microphones.push(name.clone()),
                super::NodeKind::VirtualSpeaker { name } => kept.speakers.push(name.clone()),
                _ => {}
            }
        }
        kept
    }
}

/// A device the system still remembers although it is gone and no block of the graph uses its name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaleDevice {
    /// The name the user gave it.
    pub name: String,
}

/// The answer of `devices.stale`: what could be cleaned up, and whether doing it would ask for rights.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaleDevices {
    /// False when this system has nothing to clean (no remembered devices, or no virtual devices).
    pub supported: bool,
    /// The daemon already has administrator rights: cleaning will not ask for anything.
    pub elevated: bool,
    pub stale: Vec<StaleDevice>,
}

/// The answer of `devices.clean`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanOutcome {
    /// How many old devices were forgotten.
    pub removed: usize,
    /// How many are still there (some may be refused).
    pub remaining: usize,
    /// The system asked the user for administrator rights for this.
    pub asked_for_rights: bool,
}
