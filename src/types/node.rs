//! The editable graph, as stored, sent over the control protocol and saved to disk.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Identifier of a node, chosen by the client (`"discord-in"`, `"mic"`): human readable in JSON.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub String);

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for NodeId {
    fn from(s: &str) -> Self {
        NodeId(s.to_owned())
    }
}

/// The three kinds of flows of the graph. Only the same type connects to the same type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortType {
    /// Interleaved `f32` audio in the engine format.
    Audio,
    /// A boolean/scalar level (not implemented yet).
    Condition,
    /// A discrete event (not implemented yet).
    Action,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortDirection {
    Input,
    Output,
}

/// A port declared by a node kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortSpec {
    pub name: &'static str,
    pub direction: PortDirection,
    pub ty: PortType,
    /// An input that accepts several incoming edges (summed by the node itself, e.g. a mixer).
    pub multi: bool,
}

/// One end of an edge: `{"node": "app", "port": "out"}`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PortRef {
    pub node: NodeId,
    pub port: String,
}

impl PortRef {
    pub fn new(node: &str, port: &str) -> Self {
        PortRef {
            node: NodeId::from(node),
            port: port.to_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EdgeSpec {
    pub from: PortRef,
    pub to: PortRef,
}

/// Which application's sound a capture node takes.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppTarget {
    /// A process id (and its child processes).
    Pid(u32),
    /// An executable name such as `"chrome.exe"`, matched case-insensitively when the graph starts.
    ProcessName(String),
}

/// Which audio device a node uses.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceRef {
    /// The system default for the node's direction.
    Default,
    /// A stable endpoint id (as listed by the `devices.list` method).
    Id(String),
    /// A case-insensitive part of the device's name.
    Name(String),
}

/// What a node does. Ports are fixed per kind ([`NodeKind::ports`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeKind {
    /// Source: the sound played by an application and its child processes (process loopback).
    AppCapture { target: AppTarget },
    /// Source: a capture device (microphone, line in, or a virtual device's output).
    DeviceCapture { device: DeviceRef },
    /// Sink: a playback device (speakers, headphones, or the input side of a third-party virtual cable).
    DeviceRender { device: DeviceRef },
    /// Sink: a virtual microphone created and owned by this product; other applications record from it.
    VirtualMic { name: String },
    /// Processing: gain in decibels.
    Gain { db: f32 },
    /// Processing: sum of all its incoming audio edges.
    Mixer,
}

const IN_AUDIO: PortSpec = PortSpec {
    name: "in",
    direction: PortDirection::Input,
    ty: PortType::Audio,
    multi: false,
};
const IN_AUDIO_MULTI: PortSpec = PortSpec {
    name: "in",
    direction: PortDirection::Input,
    ty: PortType::Audio,
    multi: true,
};
const OUT_AUDIO: PortSpec = PortSpec {
    name: "out",
    direction: PortDirection::Output,
    ty: PortType::Audio,
    multi: false,
};

impl NodeKind {
    /// The ports of this kind of node.
    pub fn ports(&self) -> &'static [PortSpec] {
        match self {
            NodeKind::AppCapture { .. } | NodeKind::DeviceCapture { .. } => &[OUT_AUDIO],
            NodeKind::DeviceRender { .. } | NodeKind::VirtualMic { .. } => &[IN_AUDIO],
            NodeKind::Gain { .. } => &[IN_AUDIO, OUT_AUDIO],
            NodeKind::Mixer => &[IN_AUDIO_MULTI, OUT_AUDIO],
        }
    }

    pub fn port(&self, name: &str) -> Option<&'static PortSpec> {
        self.ports().iter().find(|p| p.name == name)
    }

    /// True for nodes that bring audio into the graph from outside.
    pub fn is_source(&self) -> bool {
        matches!(
            self,
            NodeKind::AppCapture { .. } | NodeKind::DeviceCapture { .. }
        )
    }

    /// True for nodes that send audio out of the graph.
    pub fn is_sink(&self) -> bool {
        matches!(
            self,
            NodeKind::DeviceRender { .. } | NodeKind::VirtualMic { .. }
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeSpec {
    pub id: NodeId,
    #[serde(flatten)]
    pub kind: NodeKind,
}

/// The whole editable graph.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphSpec {
    #[serde(default)]
    pub nodes: Vec<NodeSpec>,
    #[serde(default)]
    pub edges: Vec<EdgeSpec>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_round_trips_through_json() {
        let spec = GraphSpec {
            nodes: vec![
                NodeSpec {
                    id: "app".into(),
                    kind: NodeKind::AppCapture {
                        target: AppTarget::ProcessName("chrome.exe".into()),
                    },
                },
                NodeSpec {
                    id: "vol".into(),
                    kind: NodeKind::Gain { db: -6.0 },
                },
                NodeSpec {
                    id: "mic".into(),
                    kind: NodeKind::VirtualMic {
                        name: "Stream".into(),
                    },
                },
            ],
            edges: vec![
                EdgeSpec {
                    from: PortRef::new("app", "out"),
                    to: PortRef::new("vol", "in"),
                },
                EdgeSpec {
                    from: PortRef::new("vol", "out"),
                    to: PortRef::new("mic", "in"),
                },
            ],
        };
        let json = serde_json::to_string(&spec).unwrap();
        let back: GraphSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(spec, back);
    }

    #[test]
    fn json_shape_is_flat_and_readable() {
        let node = NodeSpec {
            id: "g".into(),
            kind: NodeKind::Gain { db: 3.5 },
        };
        let v: serde_json::Value = serde_json::to_value(&node).unwrap();
        assert_eq!(v, serde_json::json!({"id": "g", "type": "gain", "db": 3.5}));
    }

    #[test]
    fn missing_lists_default_to_empty() {
        let spec: GraphSpec = serde_json::from_str("{}").unwrap();
        assert!(spec.nodes.is_empty() && spec.edges.is_empty());
    }

    #[test]
    fn only_the_mixer_accepts_several_inputs() {
        let kinds = [
            NodeKind::Mixer,
            NodeKind::Gain { db: 0.0 },
            NodeKind::VirtualMic { name: "x".into() },
        ];
        let multi = kinds
            .iter()
            .filter(|k| k.ports().iter().any(|p| p.multi))
            .count();
        assert_eq!(multi, 1);
    }

    #[test]
    fn sources_have_only_outputs_and_sinks_only_inputs() {
        let kinds = [
            NodeKind::AppCapture {
                target: AppTarget::Pid(1),
            },
            NodeKind::DeviceCapture {
                device: DeviceRef::Default,
            },
            NodeKind::DeviceRender {
                device: DeviceRef::Default,
            },
            NodeKind::VirtualMic { name: "m".into() },
        ];
        for k in kinds {
            if k.is_source() {
                assert!(k
                    .ports()
                    .iter()
                    .all(|p| p.direction == PortDirection::Output));
            }
            if k.is_sink() {
                assert!(k
                    .ports()
                    .iter()
                    .all(|p| p.direction == PortDirection::Input));
            }
        }
    }
}
