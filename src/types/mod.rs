//! Plain data types shared by every crate. No OS dependency, no I/O: this crate compiles and tests anywhere.

mod format;
mod node;

pub use format::{AudioFormat, ENGINE_CHANNELS, ENGINE_SAMPLE_RATE};
pub use node::{
    AppTarget, DeviceRef, EdgeSpec, GraphSpec, NodeId, NodeKind, NodeSpec, PortDirection, PortRef,
    PortSpec, PortType,
};
