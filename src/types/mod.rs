//! Plain data types shared by every crate. No OS dependency, no I/O: this crate compiles and tests anywhere.

mod format;
mod keys;
mod node;
mod status;

pub use format::{AudioFormat, ENGINE_CHANNELS, ENGINE_SAMPLE_RATE};
pub use keys::{Key, KeyCombo, Modifiers};
pub use node::{
    AppTarget, Category, DeviceRef, EdgeSpec, GraphSpec, LogicOp, NodeId, NodeKind, NodeSpec,
    PortDirection, PortRef, PortSpec, PortType,
};
pub use status::{EngineStatus, NodeState, NodeStatus};
