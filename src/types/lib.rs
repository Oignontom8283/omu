//! Plain data types shared by every crate. No OS dependency, no I/O: this crate compiles and tests anywhere.

mod devices;
mod diagnostic;
pub mod expression;
mod format;
mod keys;
mod modules;
mod node;
mod soundboard;
mod status;
pub mod template;
pub mod text_expression;
mod trust;

pub use devices::{
    AppInfo, Capabilities, CleanOutcome, DeviceInfo, Direction, KeptDevices, StaleDevice,
    StaleDevices,
};
pub use diagnostic::{Diagnostic, Severity};
pub use format::{AudioFormat, ENGINE_CHANNELS, ENGINE_SAMPLE_RATE};
pub use keys::{Extra, Key, KeyCombo, Modifiers};
pub use modules::{ModuleInfo, Progress};
pub use node::{is_web_name, stream_problem, stream_scheme, MAX_STREAM_URL};
pub use node::{
    port_number, AppTarget, Category, CompareOp, DeviceRef, EdgeSpec, Exposed, GraphSpec,
    InputValue, LogicOp, MathOp, NodeId, NodeKind, NodeSpec, PortDirection, PortLimits, PortRef,
    PortSpec, PortType, RoundMode, TextOp, MAX_BRANCHES, MAX_DELAY_MS, MAX_JOIN_ARGS,
    MAX_STREAM_NAME, MAX_TEXT, MAX_WAIT_MS, MAX_WEB_NAME,
};
pub use node::{NoiseColor, Wave, GENERATED_MAX_DB, TONE_MAX_HZ, TONE_MIN_HZ};
pub use soundboard::{
    Board, HotkeyProblem, ImportJob, ImportStep, Pad, PadProgress, Settings, SoundInfo,
    SoundLibrary, SoundStatus,
};
pub use status::{EngineStatus, Levels, NodeState, NodeStatus};
pub use template::TemplateError;
pub use trust::{CertificateInfo, RemoteInfo, TrustMode, TrustReport};
