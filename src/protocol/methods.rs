//! The methods and events of the control protocol, with their parameter and result types.
//!
//! Each method is a zero-sized type implementing [`Method`], so a client call is typed end to end
//! (`client.call::<GraphSet>(&graph)`) and the daemon dispatches on the same names.

use crate::diagnostics::Line;
use crate::types::{
    AppInfo, Board, Capabilities, CleanOutcome, DeviceInfo, EngineStatus, GraphSpec, ModuleInfo,
    Settings, SoundInfo, SoundLibrary, SoundStatus, StaleDevices,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub trait Method {
    const NAME: &'static str;
    type Params: Serialize + DeserializeOwned;
    type Result: Serialize + DeserializeOwned;
}

macro_rules! method {
    ($(#[$doc:meta])* $ty:ident, $name:literal, $params:ty => $result:ty) => {
        $(#[$doc])*
        pub struct $ty;
        impl Method for $ty {
            const NAME: &'static str = $name;
            type Params = $params;
            type Result = $result;
        }
    };
}

/// Where the virtual-microphone provider stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub name: String,
    pub available: bool,
    /// Why it is not available (component missing, not installed...).
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemInfo {
    /// The product's display name.
    pub name: String,
    pub version: String,
    pub protocol_version: u32,
    pub platform: String,
    /// Name of the audio backend (`WASAPI`, `simulated`...).
    pub backend: String,
    pub capabilities: Capabilities,
    pub virtual_microphones: Option<ProviderInfo>,
    /// The previous run of the daemon did not stop cleanly (crash, kill, power cut).
    #[serde(default)]
    pub previous_run_unclean: bool,
    /// The crash report that run left, when there is one (a path on the daemon's machine).
    #[serde(default)]
    pub crash_report: Option<String>,
}

/// Which log lines `logs.recent` returns.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogsParams {
    /// Only the lines that came after this one (to follow the log): the `seq` of the last line already seen.
    #[serde(default)]
    pub after: Option<u64>,
    /// At most this many lines (default 200, at most 2000): the most recent ones, or the oldest after `after`.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidateResult {
    pub ok: bool,
    /// Human-readable problems, empty when `ok`.
    pub errors: Vec<String>,
}

/// How a node is drawn in an editor. Not part of the audio graph: stored by the daemon for the clients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeLayout {
    pub x: f64,
    pub y: f64,
    /// How many input points an editor shows on a block that sums its inputs (the mixer). The daemon only ever sees
    /// the port; this is purely how the links are laid out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<u32>,
    /// Which of those input points each link into the block is drawn on (the link's id -> the point's number, from 1).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub slots: BTreeMap<String, u32>,
}

impl NodeLayout {
    /// A block at a place, with nothing else to remember.
    pub fn at(x: f64, y: f64) -> Self {
        Self {
            x,
            y,
            inputs: None,
            slots: BTreeMap::new(),
        }
    }
}

pub type Layout = BTreeMap<String, NodeLayout>;

method!(
    /// Name, version, protocol version, backend and capabilities of the daemon.
    SystemInfoMethod, "system.info", () => SystemInfo
);
method!(
    /// The audio devices (microphones and playback devices) the backend can open.
    DevicesList, "devices.list", () => Vec<DeviceInfo>
);
method!(
    /// The virtual devices that no longer exist but that the system still remembers (Windows keeps one record per
    /// name a microphone or a speaker ever had), and whether cleaning them up would ask for administrator rights.
    DevicesStale, "devices.stale", () => StaleDevices
);
method!(
    /// Makes the system forget those old devices. Asks the user for administrator rights, through the system's own
    /// confirmation window, unless the daemon already has them. Fails with `CANCELLED` when the user refuses.
    DevicesClean, "devices.clean", () => CleanOutcome
);
/// Parameters of `sounds.import`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportSound {
    /// A file on the daemon's computer (MP3, FLAC, OGG or WAV).
    pub path: String,
    /// What to call the sound; the file name when missing.
    #[serde(default)]
    pub name: Option<String>,
}

/// A pad of a soundboard (`soundboard.play`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PadRef {
    pub board: String,
    pub pad: String,
}

/// What to stop (`soundboard.stop`): one pad, or the whole soundboard when `pad` is missing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopSound {
    pub board: String,
    #[serde(default)]
    pub pad: Option<String>,
}

/// A soundboard by its id (`soundboard.playing`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardRef {
    pub board: String,
}

/// A sound by its id (`sounds.remove`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundRef {
    pub id: String,
}

method!(
    /// The soundboards and the sounds that were imported.
    SoundboardsGet, "soundboards.get", () => SoundLibrary
);
method!(
    /// Replaces the soundboards (their pads); the imported sounds are left as they are.
    SoundboardsSet, "soundboards.set", Vec<Board> => ()
);
method!(
    /// Imports a sound file: it is converted once to the format sounds are stored in. A long file takes a while.
    SoundsImport, "sounds.import", ImportSound => SoundInfo
);
/// A module by its id (`modules.install`, `modules.remove`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleRef {
    pub id: String,
}

/// Parameters of `modules.enable`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnableModule {
    pub id: String,
    pub enabled: bool,
}

method!(
    /// The optional modules (FFmpeg) and how each stands.
    ModulesList, "modules.list", () => Vec<ModuleInfo>
);
method!(
    /// Switches a module on. When the system has what it needs that is all; otherwise it is downloaded into the
    /// modules folder in the background (follow it with `modules.list`). This is the only thing that reaches the
    /// Internet, and only because it was asked for.
    ModulesInstall, "modules.install", ModuleRef => ()
);
method!(
    /// Stops a download that is running; nothing is installed.
    ModulesCancel, "modules.cancel", ModuleRef => ()
);
method!(
    /// Switches a module on or off (it must be installed to be switched on).
    ModulesEnable, "modules.enable", EnableModule => ()
);
method!(
    /// Switches a module off and deletes its copy in the modules folder.
    ModulesRemove, "modules.remove", ModuleRef => ()
);
method!(
    /// Stores an imported sound compressed, without loss (it plays exactly the same and takes less room on the disk). A
    /// long sound takes a while.
    SoundsCompress, "sounds.compress", SoundRef => SoundInfo
);
method!(
    /// Removes an imported sound for good (pads that played it have none).
    SoundsRemove, "sounds.remove", SoundRef => bool
);
method!(
    /// Plays a pad of a soundboard that is in the running graph.
    SoundboardPlay, "soundboard.play", PadRef => ()
);
method!(
    /// Stops a pad, or a whole soundboard.
    SoundboardStop, "soundboard.stop", StopSound => ()
);
method!(
    /// The ids of the pads of a soundboard that are playing now.
    SoundboardPlaying, "soundboard.playing", BoardRef => Vec<String>
);
method!(
    /// How the soundboards' memory is used.
    SoundboardsStatus, "soundboards.status", () => SoundStatus
);
method!(
    /// The settings people change in the panel.
    SettingsGet, "settings.get", () => Settings
);
method!(
    /// Saves the settings.
    SettingsSet, "settings.set", Settings => ()
);
method!(
    /// The applications known to the audio system, active ones first.
    AppsList, "apps.list", () => Vec<AppInfo>
);
method!(
    /// The graph the daemon is running (or last applied).
    GraphGet, "graph.get", () => GraphSpec
);
method!(
    /// Checks a graph without applying it.
    GraphValidate, "graph.validate", GraphSpec => ValidateResult
);
method!(
    /// Validates and applies a graph, replacing the current one. Fails with `VALIDATION` when it is invalid.
    GraphSet, "graph.set", GraphSpec => ()
);
method!(
    /// The engine's latest status (updated about four times a second).
    EngineStatusMethod, "engine.status", () => EngineStatus
);
method!(
    /// Stops the engine and forgets the running graph (the saved graph is kept).
    EngineStop, "engine.stop", () => ()
);
method!(
    /// Starts again the graph that was last applied (after `engine.stop`). Fails when there is none.
    EngineStart, "engine.start", () => ()
);
method!(
    /// Stops the daemon itself, cleanly: the engine, the streams, the virtual devices and the discovery file go away.
    /// The reply is sent first; clients see the connection close.
    SystemShutdown, "system.shutdown", () => ()
);
method!(
    /// The daemon's latest log lines, to read or to follow (see [`LogsParams`]).
    LogsRecent, "logs.recent", LogsParams => Vec<Line>
);
method!(
    /// The editor layout saved by the clients.
    LayoutGet, "layout.get", () => Layout
);
method!(
    /// Saves the editor layout.
    LayoutSet, "layout.set", Layout => ()
);

/// Event pushed to WebSocket clients about four times a second: the payload is an [`EngineStatus`].
pub const EVENT_STATUS: &str = "engine.status";
/// Event pushed when a client applied a new graph: the payload is the [`GraphSpec`].
pub const EVENT_GRAPH: &str = "graph.changed";

/// Names of every method, for the daemon's dispatcher and for documentation.
pub const ALL_METHODS: &[&str] = &[
    SystemInfoMethod::NAME,
    DevicesList::NAME,
    DevicesStale::NAME,
    DevicesClean::NAME,
    SoundboardsGet::NAME,
    SoundboardsSet::NAME,
    SoundsImport::NAME,
    ModulesList::NAME,
    ModulesInstall::NAME,
    ModulesEnable::NAME,
    ModulesCancel::NAME,
    ModulesRemove::NAME,
    SoundsCompress::NAME,
    SoundsRemove::NAME,
    SoundboardPlay::NAME,
    SoundboardStop::NAME,
    SoundboardPlaying::NAME,
    SoundboardsStatus::NAME,
    SettingsGet::NAME,
    SettingsSet::NAME,
    AppsList::NAME,
    GraphGet::NAME,
    GraphValidate::NAME,
    GraphSet::NAME,
    EngineStatusMethod::NAME,
    EngineStop::NAME,
    EngineStart::NAME,
    SystemShutdown::NAME,
    LogsRecent::NAME,
    LayoutGet::NAME,
    LayoutSet::NAME,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_names_are_unique() {
        let mut names: Vec<&str> = ALL_METHODS.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), ALL_METHODS.len());
    }

    #[test]
    fn unit_params_accept_null() {
        assert!(
            serde_json::from_value::<<GraphGet as Method>::Params>(serde_json::Value::Null).is_ok()
        );
    }

    #[test]
    fn layout_round_trips() {
        let mut l = Layout::new();
        l.insert("app".into(), NodeLayout::at(10.0, -4.5));
        let mut mixer = NodeLayout::at(1.0, 2.0);
        mixer.inputs = Some(3);
        mixer.slots.insert("a.out->mix.in".into(), 2);
        l.insert("mix".into(), mixer);
        let back: Layout = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
        assert_eq!(l, back);
    }

    #[test]
    fn a_layout_without_input_points_still_loads() {
        // what older versions saved, and what a block without summing inputs needs
        let old: Layout = serde_json::from_str(r#"{"app":{"x":10.0,"y":-4.5}}"#).unwrap();
        assert_eq!(old["app"], NodeLayout::at(10.0, -4.5));
        let plain = serde_json::to_string(&old).unwrap();
        assert!(
            !plain.contains("inputs") && !plain.contains("slots"),
            "{plain}"
        );
    }
}
