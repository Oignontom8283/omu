//! The methods and events of the control protocol, with their parameter and result types.
//!
//! Each method is a zero-sized type implementing [`Method`], so a client call is typed end to end
//! (`client.call::<GraphSet>(&graph)`) and the daemon dispatches on the same names.

use crate::hal::{AppInfo, Capabilities, DeviceInfo};
use crate::types::{EngineStatus, GraphSpec};
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidateResult {
    pub ok: bool,
    /// Human-readable problems, empty when `ok`.
    pub errors: Vec<String>,
}

/// Where a node is drawn in an editor. Not part of the audio graph: stored by the daemon for the clients.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

pub type Layout = BTreeMap<String, Position>;

method!(
    /// Name, version, protocol version, backend and capabilities of the daemon.
    SystemInfoMethod, "system.info", () => SystemInfo
);
method!(
    /// The audio devices (microphones and playback devices) the backend can open.
    DevicesList, "devices.list", () => Vec<DeviceInfo>
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
    AppsList::NAME,
    GraphGet::NAME,
    GraphValidate::NAME,
    GraphSet::NAME,
    EngineStatusMethod::NAME,
    EngineStop::NAME,
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
        l.insert("app".into(), Position { x: 10.0, y: -4.5 });
        let back: Layout = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
        assert_eq!(l, back);
    }
}
