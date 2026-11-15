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
    /// Size of the stored sound on the disk.
    pub bytes: u64,
    /// Stored compressed (without loss), which takes less room on the disk; it is read back exactly as it was.
    #[serde(default)]
    pub compressed: bool,
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
    /// Playing it stops every other sound of the soundboard first (a "choke group" of one soundboard).
    #[serde(default)]
    pub exclusive: bool,
    /// How long it takes to fade out when stopped, in milliseconds; 0 is the shortest fade that does not click.
    #[serde(default)]
    pub fade_ms: u32,
    /// Only a part of the sound plays: from `start_ms` to `end_ms` (0 = to the end of the sound).
    #[serde(default)]
    pub start_ms: u32,
    #[serde(default)]
    pub end_ms: u32,
    /// With a shortcut: the pad plays while the key is held and stops when it is let go.
    #[serde(default)]
    pub hold: bool,
    /// The colour of its button in the panel, `#rrggbb`; the panel's own colour when missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
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
    /// A sound that would take more than this many megabytes of plain samples is stored compressed (without loss) when
    /// imported; 0 never compresses. A sound can also be compressed by hand.
    #[serde(default = "default_compress_mb")]
    pub compress_above_mb: u32,
    /// Show the program's icon in the notification area (the daemon carries it).
    #[serde(default = "default_true")]
    pub show_tray_icon: bool,
}

fn default_true() -> bool {
    true
}

fn default_compress_mb() -> u32 {
    20
}

fn default_cache_mb() -> u32 {
    crate::branding::SOUND_CACHE_MB as u32
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            sound_cache_mb: default_cache_mb(),
            compress_above_mb: default_compress_mb(),
            show_tray_icon: true,
        }
    }
}
