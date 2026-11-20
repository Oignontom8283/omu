//! Plain data types shared by every crate. No OS dependency, no I/O: this crate compiles and tests anywhere.

mod devices;
mod format;
mod keys;
mod modules;
mod node;
mod soundboard;
mod status;

pub use devices::{
    AppInfo, Capabilities, CleanOutcome, DeviceInfo, Direction, KeptDevices, StaleDevice,
    StaleDevices,
};
pub use format::{AudioFormat, ENGINE_CHANNELS, ENGINE_SAMPLE_RATE};
pub use keys::{Key, KeyCombo, Modifiers};
pub use modules::{ModuleInfo, Progress};
pub use node::{
    port_number, AppTarget, Category, CompareOp, DeviceRef, EdgeSpec, GraphSpec, LogicOp, MathOp,
    NodeId, NodeKind, NodeSpec, PortDirection, PortRef, PortSpec, PortType, TextOp, MAX_BRANCHES,
    MAX_DELAY_MS, MAX_JOIN_ARGS, MAX_STREAM_NAME, MAX_TEXT, MAX_WAIT_MS,
};
pub use soundboard::{Board, HotkeyProblem, Pad, Settings, SoundInfo, SoundLibrary, SoundStatus};
pub use status::{EngineStatus, Levels, NodeState, NodeStatus};
