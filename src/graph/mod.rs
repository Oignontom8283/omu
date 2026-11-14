//! Validation of a [`GraphSpec`] and its topological order. Pure logic: no OS dependency.
//!
//! Rules enforced here (the daemon adds rules that need to know the devices, such as "a capture node may not
//! record this product's own virtual microphone"):
//!
//! * node ids are non-empty, made of letters, digits, `-`, `_`, `.`, and unique;
//! * an edge goes from an existing output port to an existing input port of the same type;
//! * an input port takes one edge, except a `multi` input (the mixer's);
//! * no duplicate edge, no cycle (a feedback loop needs an explicit delay block, which does not exist yet);
//! * two sinks may not use the same device, and two virtual microphones may not share a name.

use crate::types::{DeviceRef, EdgeSpec, GraphSpec, NodeId, NodeKind, PortDirection};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GraphError {
    #[error("node id {0:?} is empty or contains characters other than letters, digits, '-', '_' and '.'")]
    BadNodeId(String),
    #[error("node id {0:?} is used twice")]
    DuplicateNode(NodeId),
    #[error("edge refers to the unknown node {0:?}")]
    UnknownNode(NodeId),
    #[error("node {node:?} has no port {port:?}")]
    UnknownPort { node: NodeId, port: String },
    #[error("edge starts at the input port {0}.{1} (it must start at an output)")]
    FromInput(NodeId, String),
    #[error("edge ends at the output port {0}.{1} (it must end at an input)")]
    ToOutput(NodeId, String),
    #[error("edge connects ports of different types ({from} -> {to})")]
    TypeMismatch { from: String, to: String },
    #[error("input {0}.{1} receives several edges but does not accept it (use a mixer)")]
    FanIn(NodeId, String),
    #[error("duplicate edge {0} -> {1}")]
    DuplicateEdge(String, String),
    #[error("the graph contains a cycle through node {0:?}")]
    Cycle(NodeId),
    #[error("nodes {0:?} and {1:?} use the same device")]
    SameDevice(NodeId, NodeId),
    #[error("nodes {0:?} and {1:?} are virtual microphones with the same name")]
    SameVirtualName(NodeId, NodeId),
    #[error("virtual microphone {0:?} has an empty name")]
    EmptyVirtualName(NodeId),
}

/// A validated graph: node indices follow `spec.nodes`, `order` is a valid evaluation order.
#[derive(Debug, Clone)]
pub struct ValidGraph {
    pub spec: GraphSpec,
    /// Node indices, every node after all the nodes feeding it.
    pub order: Vec<usize>,
    /// For each edge of `spec.edges`: (index of the source node, index of the destination node).
    pub edge_nodes: Vec<(usize, usize)>,
}

impl ValidGraph {
    /// Indices of the edges that end at node `node`, in the order of `spec.edges`.
    pub fn incoming(&self, node: usize) -> impl Iterator<Item = usize> + '_ {
        self.edge_nodes
            .iter()
            .enumerate()
            .filter(move |(_, (_, to))| *to == node)
            .map(|(i, _)| i)
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn port_name(node: &NodeId, port: &str) -> String {
    format!("{node}.{port}")
}

/// Validates `spec`; returns every problem found (not only the first), so a client can show them all.
pub fn validate(spec: &GraphSpec) -> Result<ValidGraph, Vec<GraphError>> {
    let mut errors = vec![];
    let mut index: HashMap<&NodeId, usize> = HashMap::new();
    for (i, node) in spec.nodes.iter().enumerate() {
        if !valid_id(&node.id.0) {
            errors.push(GraphError::BadNodeId(node.id.0.clone()));
        }
        if index.insert(&node.id, i).is_some() {
            errors.push(GraphError::DuplicateNode(node.id.clone()));
        }
    }

    let mut edge_nodes = vec![];
    let mut seen_edges: HashSet<&EdgeSpec> = HashSet::new();
    let mut fan_in: HashMap<(usize, &str), usize> = HashMap::new();
    for edge in &spec.edges {
        let (Some(&from), Some(&to)) = (index.get(&edge.from.node), index.get(&edge.to.node))
        else {
            for end in [&edge.from, &edge.to] {
                if !index.contains_key(&end.node) {
                    errors.push(GraphError::UnknownNode(end.node.clone()));
                }
            }
            continue;
        };
        let (from_kind, to_kind) = (&spec.nodes[from].kind, &spec.nodes[to].kind);
        let from_port = from_kind.port(&edge.from.port);
        let to_port = to_kind.port(&edge.to.port);
        if from_port.is_none() {
            errors.push(GraphError::UnknownPort {
                node: edge.from.node.clone(),
                port: edge.from.port.clone(),
            });
        }
        if to_port.is_none() {
            errors.push(GraphError::UnknownPort {
                node: edge.to.node.clone(),
                port: edge.to.port.clone(),
            });
        }
        let (Some(fp), Some(tp)) = (from_port, to_port) else {
            continue;
        };
        if fp.direction != PortDirection::Output {
            errors.push(GraphError::FromInput(
                edge.from.node.clone(),
                edge.from.port.clone(),
            ));
            continue;
        }
        if tp.direction != PortDirection::Input {
            errors.push(GraphError::ToOutput(
                edge.to.node.clone(),
                edge.to.port.clone(),
            ));
            continue;
        }
        if fp.ty != tp.ty {
            errors.push(GraphError::TypeMismatch {
                from: port_name(&edge.from.node, &edge.from.port),
                to: port_name(&edge.to.node, &edge.to.port),
            });
            continue;
        }
        if !seen_edges.insert(edge) {
            errors.push(GraphError::DuplicateEdge(
                port_name(&edge.from.node, &edge.from.port),
                port_name(&edge.to.node, &edge.to.port),
            ));
            continue;
        }
        let count = fan_in.entry((to, tp.name)).or_insert(0);
        *count += 1;
        if *count == 2 && !tp.multi {
            errors.push(GraphError::FanIn(
                edge.to.node.clone(),
                edge.to.port.clone(),
            ));
        }
        edge_nodes.push((from, to));
    }

    check_conflicts(spec, &mut errors);

    let order = topological_order(spec, &edge_nodes).unwrap_or_else(|node| {
        errors.push(GraphError::Cycle(spec.nodes[node].id.clone()));
        vec![]
    });

    if errors.is_empty() {
        Ok(ValidGraph {
            spec: spec.clone(),
            order,
            edge_nodes,
        })
    } else {
        Err(errors)
    }
}

fn check_conflicts(spec: &GraphSpec, errors: &mut Vec<GraphError>) {
    let mut devices: Vec<(&NodeId, &DeviceRef)> = vec![];
    let mut virtual_names: HashMap<String, &NodeId> = HashMap::new();
    for node in &spec.nodes {
        match &node.kind {
            NodeKind::DeviceRender { device } => {
                if let Some((other, _)) = devices.iter().find(|(_, d)| *d == device) {
                    errors.push(GraphError::SameDevice((*other).clone(), node.id.clone()));
                }
                devices.push((&node.id, device));
            }
            NodeKind::VirtualMic { name } => {
                if name.trim().is_empty() {
                    errors.push(GraphError::EmptyVirtualName(node.id.clone()));
                } else if let Some(other) =
                    virtual_names.insert(name.trim().to_lowercase(), &node.id)
                {
                    errors.push(GraphError::SameVirtualName(other.clone(), node.id.clone()));
                }
            }
            _ => {}
        }
    }
}

/// Kahn's algorithm; on a cycle, returns the index of a node that belongs to it.
fn topological_order(spec: &GraphSpec, edges: &[(usize, usize)]) -> Result<Vec<usize>, usize> {
    let n = spec.nodes.len();
    let mut indegree = vec![0usize; n];
    let mut successors: Vec<Vec<usize>> = vec![vec![]; n];
    for &(from, to) in edges {
        indegree[to] += 1;
        successors[from].push(to);
    }
    let mut ready: Vec<usize> = (0..n).filter(|&i| indegree[i] == 0).collect();
    let mut order = Vec::with_capacity(n);
    while let Some(i) = ready.pop() {
        order.push(i);
        for &next in &successors[i] {
            indegree[next] -= 1;
            if indegree[next] == 0 {
                ready.push(next);
            }
        }
    }
    if order.len() == n {
        Ok(order)
    } else {
        Err((0..n)
            .find(|&i| indegree[i] > 0)
            .expect("a node remains when the order is incomplete"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AppTarget, NodeSpec, PortRef};

    fn node(id: &str, kind: NodeKind) -> NodeSpec {
        NodeSpec {
            id: id.into(),
            kind,
        }
    }
    fn edge(a: &str, ap: &str, b: &str, bp: &str) -> EdgeSpec {
        EdgeSpec {
            from: PortRef::new(a, ap),
            to: PortRef::new(b, bp),
        }
    }
    fn app() -> NodeKind {
        NodeKind::AppCapture {
            target: AppTarget::Pid(4),
        }
    }
    fn mic(name: &str) -> NodeKind {
        NodeKind::VirtualMic { name: name.into() }
    }
    fn gain() -> NodeKind {
        NodeKind::Gain { db: 0.0 }
    }

    #[test]
    fn accepts_a_simple_chain_and_orders_it() {
        let spec = GraphSpec {
            nodes: vec![node("mic", mic("M")), node("g", gain()), node("app", app())],
            edges: vec![edge("app", "out", "g", "in"), edge("g", "out", "mic", "in")],
        };
        let v = validate(&spec).unwrap();
        let pos = |name: &str| {
            v.order
                .iter()
                .position(|&i| v.spec.nodes[i].id.0 == name)
                .unwrap()
        };
        assert!(pos("app") < pos("g") && pos("g") < pos("mic"));
        assert_eq!(v.order.len(), 3);
    }

    #[test]
    fn empty_graph_is_valid() {
        assert!(validate(&GraphSpec::default()).unwrap().order.is_empty());
    }

    #[test]
    fn rejects_bad_and_duplicate_ids() {
        let spec = GraphSpec {
            nodes: vec![node("a b", gain()), node("x", gain()), node("x", gain())],
            edges: vec![],
        };
        let e = validate(&spec).unwrap_err();
        assert!(e.contains(&GraphError::BadNodeId("a b".into())));
        assert!(e.contains(&GraphError::DuplicateNode("x".into())));
    }

    #[test]
    fn rejects_unknown_nodes_and_ports() {
        let spec = GraphSpec {
            nodes: vec![node("app", app())],
            edges: vec![
                edge("app", "out", "ghost", "in"),
                edge("app", "nope", "app", "in"),
            ],
        };
        let e = validate(&spec).unwrap_err();
        assert!(e.contains(&GraphError::UnknownNode("ghost".into())));
        assert!(e
            .iter()
            .any(|x| matches!(x, GraphError::UnknownPort { port, .. } if port == "nope")));
    }

    #[test]
    fn rejects_wrong_direction() {
        let spec = GraphSpec {
            nodes: vec![node("app", app()), node("g", gain()), node("m", mic("M"))],
            edges: vec![edge("g", "in", "m", "in"), edge("app", "out", "g", "out")],
        };
        let e = validate(&spec).unwrap_err();
        assert!(e.contains(&GraphError::FromInput("g".into(), "in".into())));
        assert!(e.contains(&GraphError::ToOutput("g".into(), "out".into())));
    }

    #[test]
    fn fan_in_needs_a_mixer() {
        let two_sources = |sink: NodeKind| GraphSpec {
            nodes: vec![node("a", app()), node("b", app()), node("s", sink)],
            edges: vec![edge("a", "out", "s", "in"), edge("b", "out", "s", "in")],
        };
        let e = validate(&two_sources(gain())).unwrap_err();
        assert!(e.contains(&GraphError::FanIn("s".into(), "in".into())));
        assert!(validate(&two_sources(NodeKind::Mixer)).is_ok());
    }

    #[test]
    fn fan_out_is_allowed() {
        let spec = GraphSpec {
            nodes: vec![
                node("a", app()),
                node("m1", mic("one")),
                node("m2", mic("two")),
            ],
            edges: vec![edge("a", "out", "m1", "in"), edge("a", "out", "m2", "in")],
        };
        assert!(validate(&spec).is_ok());
    }

    #[test]
    fn rejects_duplicate_edges() {
        let spec = GraphSpec {
            nodes: vec![node("a", app()), node("m", NodeKind::Mixer)],
            edges: vec![edge("a", "out", "m", "in"), edge("a", "out", "m", "in")],
        };
        let e = validate(&spec).unwrap_err();
        assert!(e.iter().any(|x| matches!(x, GraphError::DuplicateEdge(..))));
    }

    #[test]
    fn rejects_cycles_including_self_loops() {
        let cyc = GraphSpec {
            nodes: vec![node("a", gain()), node("b", gain())],
            edges: vec![edge("a", "out", "b", "in"), edge("b", "out", "a", "in")],
        };
        assert!(validate(&cyc)
            .unwrap_err()
            .iter()
            .any(|x| matches!(x, GraphError::Cycle(_))));
        let selfloop = GraphSpec {
            nodes: vec![node("a", gain())],
            edges: vec![edge("a", "out", "a", "in")],
        };
        assert!(validate(&selfloop)
            .unwrap_err()
            .iter()
            .any(|x| matches!(x, GraphError::Cycle(_))));
    }

    #[test]
    fn rejects_two_sinks_on_one_device_and_duplicate_virtual_names() {
        let spec = GraphSpec {
            nodes: vec![
                node(
                    "r1",
                    NodeKind::DeviceRender {
                        device: DeviceRef::Default,
                    },
                ),
                node(
                    "r2",
                    NodeKind::DeviceRender {
                        device: DeviceRef::Default,
                    },
                ),
                node("v1", mic("Stream")),
                node("v2", mic("stream ")),
                node("v3", mic("  ")),
            ],
            edges: vec![],
        };
        let e = validate(&spec).unwrap_err();
        assert!(e.contains(&GraphError::SameDevice("r1".into(), "r2".into())));
        assert!(e.contains(&GraphError::SameVirtualName("v1".into(), "v2".into())));
        assert!(e.contains(&GraphError::EmptyVirtualName("v3".into())));
    }

    #[test]
    fn reports_every_problem_not_only_the_first() {
        let spec = GraphSpec {
            nodes: vec![node("a", gain()), node("a", gain())],
            edges: vec![edge("a", "out", "zzz", "in")],
        };
        assert!(validate(&spec).unwrap_err().len() >= 2);
    }

    #[test]
    fn incoming_lists_the_edges_of_a_node() {
        let spec = GraphSpec {
            nodes: vec![
                node("a", app()),
                node("b", app()),
                node("m", NodeKind::Mixer),
            ],
            edges: vec![edge("a", "out", "m", "in"), edge("b", "out", "m", "in")],
        };
        let v = validate(&spec).unwrap();
        assert_eq!(v.incoming(2).collect::<Vec<_>>(), vec![0, 1]);
        assert_eq!(v.incoming(0).count(), 0);
    }
}
