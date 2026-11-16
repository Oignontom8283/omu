//! Real-time audio engine.
//!
//! * [`bridge`]: lock-free rings that join a device clock to the graph clock (with drift compensation);
//! * [`plan`]: the executable form of a graph;
//! * [`runtime`]: the graph thread that runs a plan every 10 ms;
//! * [`control`]: opens the devices of a graph, builds plans, supervises and retries failed streams.
//!
//! The engine never panics into its caller and never stops the process: a device that cannot be opened or breaks
//! makes its node silent and is retried; the rest of the graph keeps running.

pub mod bridge;
pub mod control;
pub mod plan;
pub mod runtime;

pub use control::{Engine, EngineConfig};
pub use omu_types::{EngineStatus, NodeState, NodeStatus};
