//! Plain data types shared by every crate. No OS dependency, no I/O: this crate compiles and tests anywhere.

mod devices;
mod diagnostic;
mod format;
mod keys;
mod modules;
mod node;
mod soundboard;
mod status;
pub mod template;
mod trust;

pub use devices::{
    AppInfo, Capabilities, CleanOutcome, DeviceInfo, Direction, KeptDevices, StaleDevice,
    StaleDevices,
};
pub use diagnostic::{Diagnostic, Severity};
pub use format::{AudioFormat, ENGINE_CHANNELS, ENGINE_SAMPLE_RATE};
pub use keys::{Key, KeyCombo, Modifiers};
pub use modules::{ModuleInfo, Progress};
pub use node::is_web_name;
pub use node::{
    port_number, AppTarget, Category, CompareOp, DeviceRef, EdgeSpec, Exposed, GraphSpec,
    InputValue, LogicOp, MathOp, NodeId, NodeKind, NodeSpec, PortDirection, PortLimits, PortRef,
    PortSpec, PortType, RoundMode, TextOp, MAX_BRANCHES, MAX_DELAY_MS, MAX_JOIN_ARGS,
    MAX_STREAM_NAME, MAX_TEXT, MAX_WAIT_MS, MAX_WEB_NAME,
};
pub use soundboard::{Board, HotkeyProblem, Pad, Settings, SoundInfo, SoundLibrary, SoundStatus};
pub use status::{EngineStatus, Levels, NodeState, NodeStatus};
pub use template::TemplateError;
pub use trust::{CertificateInfo, TrustMode, TrustReport};
