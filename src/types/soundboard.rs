//! Soundboards as the daemon reports and saves them: the sounds that were imported, and the soundboards that use them.
//! They are not part of the graph (which describes routing): a soundboard block only names the soundboard it plays.

use serde::{Deserialize, Serialize};

/// A sound that was imported (converted once to the format sounds are stored in).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundInfo {
    pub id: String,
    /// What it was called when imported (the file name); a pad can have its own name.
    pub name: String,
    /// 1 or 2.
    pub channels: u8,
    /// Frames at 48 kHz.
    pub frames: u64,
    /// Size of the stored sound.
    pub bytes: u64,
}

impl SoundInfo {
    pub fn seconds(&self) -> f64 {
        self.frames as f64 / 48_000.0
    }
}

/// One button of a soundboard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pad {
    pub id: String,
    pub name: String,
    /// The sound it plays; `None` for a pad that has none yet.
    #[serde(default)]
    pub sound: Option<String>,
    #[serde(default)]
    pub gain_db: f32,
    /// Plays again and again until stopped (not for the long sounds that are read from the disk).
    #[serde(default)]
    pub looped: bool,
    /// A global keyboard shortcut that plays the pad ("Ctrl+Alt+1"). The daemon listens for it while the pad's
    /// soundboard is in the running graph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Board {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub pads: Vec<Pad>,
}

/// Everything saved about soundboards.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SoundLibrary {
    #[serde(default)]
    pub sounds: Vec<SoundInfo>,
    #[serde(default)]
    pub boards: Vec<Board>,
}

/// How the soundboards' memory is used (`soundboards.status`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundStatus {
    /// At least one soundboard is in the running graph: the memory exists.
    pub active: bool,
    /// What the memory may grow to.
    pub cache_limit_bytes: u64,
    /// What exists now (blocks made so far).
    pub cache_allocated_bytes: u64,
    /// What sounds use now.
    pub cache_used_bytes: u64,
    pub sounds_loaded: usize,
    /// Soundboards in the running graph.
    pub active_boards: Vec<String>,
    /// Frames of long sounds the disk was too late for.
    pub late_frames: u64,
    /// Long sounds being read from the disk now.
    pub streams: usize,
    /// Pad shortcuts that could not be registered (another program uses the keys, or the system has no shortcuts).
    #[serde(default)]
    pub hotkey_problems: Vec<HotkeyProblem>,
}

/// A pad whose keyboard shortcut is not working.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyProblem {
    pub board: String,
    pub pad: String,
    pub message: String,
}

/// The settings of the daemon that people change in the panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// The most memory soundboards may use, in megabytes. It applies when the memory is next made (when a soundboard
    /// enters the running graph and none was there). The environment variable replaces it.
    #[serde(default = "default_cache_mb")]
    pub sound_cache_mb: u32,
}

fn default_cache_mb() -> u32 {
    crate::branding::SOUND_CACHE_MB as u32
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            sound_cache_mb: default_cache_mb(),
        }
    }
}
