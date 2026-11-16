//! The control protocol: JSON-RPC 2.0 messages and the typed list of methods the daemon offers.
//!
//! Every client (command line, web panel, Tauri window, a script sending raw JSON) speaks exactly this; there is no
//! second API. Requests go to the daemon over HTTP (`POST /rpc`) or a WebSocket (`/ws`, which also pushes events).
//! The protocol is versioned ([`PROTOCOL_VERSION`]); `system.info` reports the daemon's.

mod envelope;
pub mod methods;

pub use envelope::{Id, Notification, Request, Response, RpcError, JSONRPC_VERSION};

/// Bumped on every incompatible change of a method or event.
pub const PROTOCOL_VERSION: u32 = 1;
