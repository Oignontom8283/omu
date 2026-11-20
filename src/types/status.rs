//! Live status of the engine, as reported to clients (serialisable both ways: the daemon writes it, clients read it).

use super::NodeId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", content = "message", rename_all = "snake_case")]
pub enum NodeState {
    Running,
    /// The node's stream could not be opened or broke; the engine retries. The node is silent meanwhile.
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeStatus {
    pub id: NodeId,
    #[serde(flatten)]
    pub state: NodeState,
    /// Highest level (linear, 1.0 = full scale) since the previous reading.
    pub peak: f32,
    pub underruns: u64,
    pub overruns: u64,
    pub fill_ms: f32,
    pub frames: u64,
    /// For a block that holds a yes/no (a variable, a detector, a gate, logic): its value now.
    #[serde(default)]
    pub active: Option<bool>,
    /// Actions a block has sent (a trigger) since the graph started.
    #[serde(default)]
    pub events: u64,
    /// For a block that makes a value (text, a number): what it is now, written out.
    #[serde(default)]
    pub value: Option<String>,
}

/// How loud the sound at the input of a block has been, cycle after cycle: what a calibration listens to.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Levels {
    /// How long one value lasts, in milliseconds.
    pub cycle_ms: u32,
    /// The levels in decibels (0 is full scale, -120 silence), the oldest first; the most recent ten seconds at most.
    pub db: Vec<f32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EngineStatus {
    pub running: bool,
    pub cycles: u64,
    pub late_cycles: u64,
    pub nodes: Vec<NodeStatus>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trips_through_json() {
        let status = EngineStatus {
            running: true,
            cycles: 10,
            late_cycles: 1,
            nodes: vec![
                NodeStatus {
                    id: "app".into(),
                    state: NodeState::Running,
                    peak: 0.5,
                    underruns: 0,
                    overruns: 0,
                    fill_ms: 24.5,
                    frames: 480,
                    active: Some(true),
                    events: 3,
                    value: None,
                },
                NodeStatus {
                    id: "mic".into(),
                    state: NodeState::Failed("no device".into()),
                    peak: 0.0,
                    underruns: 1,
                    overruns: 2,
                    fill_ms: 0.0,
                    frames: 0,
                    active: None,
                    events: 0,
                    value: None,
                },
            ],
        };
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(serde_json::from_str::<EngineStatus>(&json).unwrap(), status);
    }

    #[test]
    fn json_shape_is_flat() {
        let node = NodeStatus {
            id: "x".into(),
            state: NodeState::Failed("boom".into()),
            peak: 0.0,
            underruns: 0,
            overruns: 0,
            fill_ms: 0.0,
            frames: 0,
            active: None,
            events: 0,
            value: None,
        };
        let v = serde_json::to_value(&node).unwrap();
        assert_eq!(v["state"], "failed");
        assert_eq!(v["message"], "boom");
        assert_eq!(v["id"], "x");
    }
}
