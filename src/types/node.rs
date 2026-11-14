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

/// The five families of blocks. Every block belongs to one; its family tells what kind of flows it sends and
/// receives, and an editor offers a drop-down of the blocks of a family ("what does this block do?").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// Brings audio into the graph (a program's sound, a microphone).
    Input,
    /// Takes audio out of the graph (speakers, a virtual microphone).
    Output,
    /// Works on audio (gain, mixing, gating...).
    Processing,
    /// Makes a continuous yes/no value (a variable, a voice detector, logic). Shown in red.
    Condition,
    /// Produces actions when something happens (a key is pressed, a timer fires).
    Trigger,
}

/// How a [`NodeKind::Logic`] block combines its conditions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogicOp {
    And,
    Or,
    Xor,
    /// Inverts `a` (`b` is ignored).
    Not,
}

/// What a node does. Ports are fixed per kind ([`NodeKind::ports`]).
///
/// Three kinds of flow connect them: continuous **audio**, a continuous **condition** (true or false, evaluated
/// every cycle) and discrete **actions** (an event that happens once: a key press). A trigger sends actions; an
/// action can set a variable; the variable's condition can open a gate on the audio.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeKind {
    // ---- inputs
    /// Input: the sound played by an application and its child processes (process loopback).
    AppCapture { target: AppTarget },
    /// Input: a capture device (microphone, line in, or a virtual device's output).
    DeviceCapture { device: DeviceRef },
    // ---- outputs
    /// Output: a playback device (speakers, headphones, or the input side of a third-party virtual cable).
    DeviceRender { device: DeviceRef },
    /// Output: a virtual microphone created and owned by this product; other applications record from it.
    VirtualMic { name: String },
    // ---- processing
    /// Processing: gain in decibels.
    Gain { db: f32 },
    /// Processing: sum of all its incoming audio edges.
    Mixer,
    /// Processing: lets the audio through while its `open` condition is true (silence otherwise, with a short fade
    /// so that nothing clicks). Without a condition connected it is always open.
    Gate,
    // ---- conditions
    /// Condition: a remembered yes/no. Actions change it: `set` makes it true, `clear` false, `toggle` flips it.
    Variable { initial: bool },
    /// Condition: true while the audio at its input is louder than a threshold (a simple voice detector); it stays
    /// true for a short time after the sound stops so that speech is not chopped.
    Level { threshold_db: f32 },
    /// Condition: combines conditions (and, or, xor, not).
    Logic { op: LogicOp },
    // ---- triggers
    /// Trigger: sends an action each time the keyboard shortcut is pressed (works while another program has focus).
    Hotkey { keys: String },
    /// Trigger: sends an action every `interval_ms` milliseconds.
    Timer { interval_ms: u32 },
}

const fn port(name: &'static str, direction: PortDirection, ty: PortType, multi: bool) -> PortSpec {
    PortSpec {
        name,
        direction,
        ty,
        multi,
    }
}
use PortDirection::{Input, Output};
use PortType::{Action, Audio, Condition};

const IN_AUDIO: PortSpec = port("in", Input, Audio, false);
const IN_AUDIO_MULTI: PortSpec = port("in", Input, Audio, true);
const OUT_AUDIO: PortSpec = port("out", Output, Audio, false);
const IN_OPEN: PortSpec = port("open", Input, Condition, false);
const IN_SET: PortSpec = port("set", Input, Action, true);
const IN_CLEAR: PortSpec = port("clear", Input, Action, true);
const IN_TOGGLE: PortSpec = port("toggle", Input, Action, true);
const OUT_VALUE: PortSpec = port("value", Output, Condition, false);
const OUT_ACTIVE: PortSpec = port("active", Output, Condition, false);
const IN_A: PortSpec = port("a", Input, Condition, false);
const IN_B: PortSpec = port("b", Input, Condition, false);
const OUT_RESULT: PortSpec = port("out", Output, Condition, false);
const OUT_PRESSED: PortSpec = port("pressed", Output, Action, false);
const OUT_TICK: PortSpec = port("tick", Output, Action, false);

impl NodeKind {
    /// The ports of this kind of node.
    pub fn ports(&self) -> &'static [PortSpec] {
        match self {
            NodeKind::AppCapture { .. } | NodeKind::DeviceCapture { .. } => &[OUT_AUDIO],
            NodeKind::DeviceRender { .. } | NodeKind::VirtualMic { .. } => &[IN_AUDIO],
            NodeKind::Gain { .. } => &[IN_AUDIO, OUT_AUDIO],
            NodeKind::Mixer => &[IN_AUDIO_MULTI, OUT_AUDIO],
            NodeKind::Gate => &[IN_AUDIO, IN_OPEN, OUT_AUDIO],
            NodeKind::Variable { .. } => &[IN_SET, IN_CLEAR, IN_TOGGLE, OUT_VALUE],
            NodeKind::Level { .. } => &[IN_AUDIO, OUT_ACTIVE],
            NodeKind::Logic { .. } => &[IN_A, IN_B, OUT_RESULT],
            NodeKind::Hotkey { .. } => &[OUT_PRESSED],
            NodeKind::Timer { .. } => &[OUT_TICK],
        }
    }

    pub fn port(&self, name: &str) -> Option<&'static PortSpec> {
        self.ports().iter().find(|p| p.name == name)
    }

    /// The family of this block.
    pub fn category(&self) -> Category {
        match self {
            NodeKind::AppCapture { .. } | NodeKind::DeviceCapture { .. } => Category::Input,
            NodeKind::DeviceRender { .. } | NodeKind::VirtualMic { .. } => Category::Output,
            NodeKind::Gain { .. } | NodeKind::Mixer | NodeKind::Gate => Category::Processing,
            NodeKind::Variable { .. } | NodeKind::Level { .. } | NodeKind::Logic { .. } => {
                Category::Condition
            }
            NodeKind::Hotkey { .. } | NodeKind::Timer { .. } => Category::Trigger,
        }
    }

    /// True for nodes that bring audio into the graph from outside.
    pub fn is_source(&self) -> bool {
        self.category() == Category::Input
    }

    /// True for nodes that send audio out of the graph.
    pub fn is_sink(&self) -> bool {
        self.category() == Category::Output
    }

    /// True for nodes that need something from the system when the graph runs: a device, an application, a keyboard
    /// shortcut. The engine opens it, and the node reports an error (and stays silent) if that fails.
    pub fn needs_stream(&self) -> bool {
        matches!(self.category(), Category::Input | Category::Output)
            || matches!(self, NodeKind::Hotkey { .. })
    }

    /// The type of the flow this node sends, if it sends one (every kind has at most one output port).
    pub fn output_type(&self) -> Option<PortType> {
        self.ports()
            .iter()
            .find(|p| p.direction == Output)
            .map(|p| p.ty)
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

    fn every_kind() -> Vec<NodeKind> {
        vec![
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
            NodeKind::Gain { db: 0.0 },
            NodeKind::Mixer,
            NodeKind::Gate,
            NodeKind::Variable { initial: false },
            NodeKind::Level {
                threshold_db: -40.0,
            },
            NodeKind::Logic { op: LogicOp::And },
            NodeKind::Hotkey { keys: "F8".into() },
            NodeKind::Timer { interval_ms: 1000 },
        ]
    }

    #[test]
    fn every_kind_survives_json_and_belongs_to_a_family() {
        for k in every_kind() {
            let spec = NodeSpec {
                id: "x".into(),
                kind: k.clone(),
            };
            let back: NodeSpec =
                serde_json::from_value(serde_json::to_value(&spec).unwrap()).unwrap();
            assert_eq!(back, spec);
            let _ = k.category();
        }
    }

    #[test]
    fn families_match_the_flows_they_send_and_receive() {
        for k in every_kind() {
            match k.category() {
                Category::Input => assert_eq!(k.output_type(), Some(PortType::Audio)),
                Category::Output => assert_eq!(k.output_type(), None),
                Category::Trigger => {
                    assert_eq!(k.output_type(), Some(PortType::Action));
                    assert!(
                        k.ports()
                            .iter()
                            .all(|p| p.direction == PortDirection::Output),
                        "a trigger receives nothing"
                    );
                }
                Category::Condition => assert_eq!(k.output_type(), Some(PortType::Condition)),
                Category::Processing => assert_eq!(k.output_type(), Some(PortType::Audio)),
            }
        }
    }

    #[test]
    fn the_example_chain_is_typed_end_to_end() {
        // hotkey --action--> variable.toggle ; variable.value --condition--> gate.open ; audio through the gate
        let hotkey = NodeKind::Hotkey { keys: "F8".into() };
        let variable = NodeKind::Variable { initial: true };
        let gate = NodeKind::Gate;
        assert_eq!(hotkey.port("pressed").unwrap().ty, PortType::Action);
        assert_eq!(variable.port("toggle").unwrap().ty, PortType::Action);
        assert!(
            variable.port("toggle").unwrap().multi,
            "several triggers may toggle one variable"
        );
        assert_eq!(variable.port("value").unwrap().ty, PortType::Condition);
        assert_eq!(gate.port("open").unwrap().ty, PortType::Condition);
        assert!(!gate.port("open").unwrap().multi, "one condition decides");
    }

    #[test]
    fn only_streams_nodes_need_the_system() {
        let needs: Vec<bool> = every_kind().iter().map(|k| k.needs_stream()).collect();
        assert_eq!(
            needs,
            vec![true, true, true, true, false, false, false, false, false, false, true, false]
        );
    }
}
