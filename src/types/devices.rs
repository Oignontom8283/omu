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
pub struct Capabilities {
    pub devices: bool,
    pub app_capture: bool,
    pub virtual_devices: bool,
    /// Global keyboard shortcuts (the `Hotkey` trigger).
    pub hotkeys: bool,
}
