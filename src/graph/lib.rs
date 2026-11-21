//! Validation of a [`GraphSpec`] and its topological order. Pure logic: no OS dependency.
//!
//! Rules enforced here (the daemon adds rules that need to know the devices, such as "a capture node may not
//! record this product's own virtual microphone"):
//!
//! * node ids are non-empty, made of letters, digits, `-`, `_`, `.`, and unique;
//! * an edge goes from an existing output port to an existing input port of the same type;
//! * an input port takes one edge, except a `multi` input: every sound input (the sounds are added), the `all` and
//!   `any` conditions and the actions of a variable;
//! * no duplicate edge, no cycle (a feedback loop needs an explicit delay block, which does not exist yet);
//! * two sinks may not use the same device, and two virtual microphones may not share a name.

use omu_types::{
    DeviceRef, Diagnostic, EdgeSpec, GraphSpec, KeyCombo, NodeId, NodeKind, PortDirection,
    MAX_BRANCHES, MAX_DELAY_MS, MAX_JOIN_ARGS, MAX_TEXT, MAX_WAIT_MS,
};
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
    #[error("input {0}.{1} receives several edges but does not accept it (to combine conditions use an 'all' or 'any' block)")]
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
    #[error("shortcut of node {0:?} is not valid: {1}")]
    BadShortcut(NodeId, String),
    #[error("nodes {0:?} and {1:?} use the same shortcut ({2}): it can only trigger one place")]
    SameShortcut(NodeId, NodeId, String),
    #[error("timer {0:?} fires every {1} ms: use at least 10 ms")]
    TimerTooFast(NodeId, u32),
    #[error("block {0:?} has {1} conditions: use between 1 and {max}", max = MAX_BRANCHES)]
    BadBranches(NodeId, u32),
    #[error("delay {0:?} is {1} ms: the longest is {max} ms", max = MAX_DELAY_MS)]
    DelayTooLong(NodeId, u32),
    #[error("block {0:?} waits {1} ms: the longest is {max} ms", max = MAX_WAIT_MS)]
    WaitTooLong(NodeId, u32),
    #[error("network block {0:?}: {1}")]
    BadNetwork(NodeId, String),
    #[error("network blocks {0:?} and {1:?} both listen on UDP port {2}")]
    SamePort(NodeId, NodeId, u16),
    #[error("block {0:?}: {1}")]
    BadValue(NodeId, String),
}

impl GraphError {
    /// The error as data: a stable code, the block and the connection point it is about, what to do.
    pub fn diagnostic(&self) -> Diagnostic {
        let node_of = |port: &str| -> (NodeId, String) {
            // "node.port", as the messages name a connection point
            match port.rsplit_once('.') {
                Some((n, p)) => (NodeId::from(n), p.to_owned()),
                None => (NodeId::from(port), String::new()),
            }
        };
        let d = |code: &str| Diagnostic::error(code, self.to_string());
        match self {
            GraphError::BadNodeId(_) => d("bad_node_id"),
            GraphError::DuplicateNode(n) => d("duplicate_node").at(n),
            GraphError::UnknownNode(n) => d("unknown_node").at(n),
            GraphError::UnknownPort { node, port } => d("unknown_port").at_port(node, port),
            GraphError::FromInput(n, p) => d("link_from_input")
                .at_port(n, p)
                .hint("a link starts at an output"),
            GraphError::ToOutput(n, p) => d("link_to_output")
                .at_port(n, p)
                .hint("a link ends at an input"),
            GraphError::TypeMismatch { to, .. } => {
                let (n, p) = node_of(to);
                d("type_mismatch").at_port(&n, &p).hint("a sound goes to a sound, a number to a number; numbers and yes/no can go into a text")
            }
            GraphError::FanIn(n, p) => d("fan_in")
                .at_port(n, p)
                .hint("add an input to the block, or use a mixer, an 'all' or an 'any' block"),
            GraphError::DuplicateEdge(_, to) => {
                let (n, p) = node_of(to);
                d("duplicate_link").at_port(&n, &p)
            }
            GraphError::Cycle(n) => d("cycle")
                .at(n)
                .hint("a block cannot feed itself, even through other blocks"),
            GraphError::SameDevice(a, _) => d("same_device").at(a),
            GraphError::SameVirtualName(a, _) => d("same_virtual_name").at(a),
            GraphError::EmptyVirtualName(n) => d("empty_name").at(n),
            GraphError::BadShortcut(n, _) => d("bad_shortcut").at(n),
            GraphError::SameShortcut(a, _, _) => d("same_shortcut").at(a),
            GraphError::TimerTooFast(n, _) => d("timer_too_fast").at(n).hint("use at least 10 ms"),
            GraphError::BadBranches(n, _) => d("bad_branches").at(n),
            GraphError::DelayTooLong(n, _) => d("delay_too_long").at(n),
            GraphError::WaitTooLong(n, _) => d("wait_too_long").at(n),
            GraphError::BadNetwork(n, _) => d("bad_network").at(n),
            GraphError::SamePort(a, _, _) => d("same_port").at(a),
            GraphError::BadValue(n, _) => d("bad_value").at(n),
        }
    }
}

/// Everything wrong with `spec` as data (empty when the graph is valid).
pub fn diagnose(spec: &GraphSpec) -> Vec<Diagnostic> {
    match validate(spec) {
        Ok(_) => vec![],
        Err(errors) => errors.iter().map(GraphError::diagnostic).collect(),
    }
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
    let mut fan_in: HashMap<(usize, String), usize> = HashMap::new();
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
        if !tp.ty.accepts(fp.ty) {
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
        let count = fan_in.entry((to, tp.name.clone())).or_insert(0);
        *count += 1;
        if *count == 2 && !tp.multi {
            errors.push(GraphError::FanIn(
                edge.to.node.clone(),
                edge.to.port.clone(),
            ));
        }
        edge_nodes.push((from, to));
    }

    check_typed_inputs(spec, &mut errors);
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

/// What is typed in a block for an input has to be for an input that takes it, with the right sort of value.
fn check_typed_inputs(spec: &GraphSpec, errors: &mut Vec<GraphError>) {
    use omu_types::{InputValue, PortType};
    for node in &spec.nodes {
        for (name, value) in &node.inputs {
            let Some(port) = node
                .kind
                .port(name)
                .filter(|p| p.direction == PortDirection::Input)
            else {
                errors.push(GraphError::BadValue(
                    node.id.clone(),
                    format!("there is no input {name:?} to type a value in"),
                ));
                continue;
            };
            let fits = match (port.ty, value) {
                (PortType::Float | PortType::Int, InputValue::Number(n)) => n.is_finite(),
                (PortType::Text, InputValue::Text(t)) => t.chars().count() <= MAX_TEXT,
                (PortType::Text, InputValue::Number(n)) => n.is_finite(),
                (PortType::Condition, InputValue::Bool(_)) => true,
                _ => false,
            };
            if !fits {
                errors.push(GraphError::BadValue(
                    node.id.clone(),
                    format!("the value typed for {name:?} does not fit this input"),
                ));
            }
        }
    }
}

fn check_conflicts(spec: &GraphSpec, errors: &mut Vec<GraphError>) {
    let mut devices: Vec<(&NodeId, &DeviceRef)> = vec![];
    let mut virtual_names: HashMap<String, &NodeId> = HashMap::new();
    let mut speaker_names: HashMap<String, &NodeId> = HashMap::new();
    let mut shortcuts: HashMap<String, &NodeId> = HashMap::new();
    let mut listen_ports: HashMap<u16, &NodeId> = HashMap::new();
    for node in &spec.nodes {
        match &node.kind {
            NodeKind::DeviceRender { device } => {
                if let Some((other, _)) = devices.iter().find(|(_, d)| *d == device) {
                    errors.push(GraphError::SameDevice((*other).clone(), node.id.clone()));
                }
                devices.push((&node.id, device));
            }
            NodeKind::Hotkey { keys } => match KeyCombo::parse(keys) {
                Err(e) => errors.push(GraphError::BadShortcut(node.id.clone(), e)),
                Ok(combo) => {
                    let normal = combo.to_string();
                    if let Some(other) = shortcuts.insert(normal.clone(), &node.id) {
                        errors.push(GraphError::SameShortcut(
                            other.clone(),
                            node.id.clone(),
                            normal,
                        ));
                    }
                }
            },
            NodeKind::Timer { interval_ms } if *interval_ms < 10 => {
                errors.push(GraphError::TimerTooFast(node.id.clone(), *interval_ms));
            }
            NodeKind::Branch { branches } | NodeKind::Select { branches }
                if !(1..=MAX_BRANCHES).contains(branches) =>
            {
                errors.push(GraphError::BadBranches(node.id.clone(), *branches));
            }
            NodeKind::Text { value } if value.chars().count() > MAX_TEXT => {
                errors.push(GraphError::BadValue(
                    node.id.clone(),
                    format!("the text is longer than {MAX_TEXT} characters"),
                ));
            }
            NodeKind::Number { value } if !value.is_finite() => {
                errors.push(GraphError::BadValue(
                    node.id.clone(),
                    "the number is not a finite number".into(),
                ));
            }
            NodeKind::Join { template, args } => {
                if !(1..=MAX_JOIN_ARGS).contains(args) {
                    errors.push(GraphError::BadValue(
                        node.id.clone(),
                        format!("it has {args} arguments: use between 1 and {MAX_JOIN_ARGS}"),
                    ));
                }
                if template.chars().count() > MAX_TEXT {
                    errors.push(GraphError::BadValue(
                        node.id.clone(),
                        format!("the template is longer than {MAX_TEXT} characters"),
                    ));
                } else {
                    // the template has to be written the way the substitution library reads it, and name values that exist
                    let values = vec![String::new(); (*args).clamp(1, MAX_JOIN_ARGS) as usize];
                    if let Err(e) = omu_types::template::render(template, &values) {
                        errors.push(GraphError::BadValue(
                            node.id.clone(),
                            format!("the text template is not valid: {e}"),
                        ));
                    }
                }
            }
            NodeKind::Delay { ms } if *ms > MAX_DELAY_MS => {
                errors.push(GraphError::DelayTooLong(node.id.clone(), *ms));
            }
            NodeKind::Hold { ms } | NodeKind::After { ms } if *ms > MAX_WAIT_MS => {
                errors.push(GraphError::WaitTooLong(node.id.clone(), *ms));
            }
            NodeKind::VirtualSpeaker { name } => {
                if name.trim().is_empty() {
                    errors.push(GraphError::EmptyVirtualName(node.id.clone()));
                } else if let Some(other) =
                    speaker_names.insert(name.trim().to_lowercase(), &node.id)
                {
                    errors.push(GraphError::SameVirtualName(other.clone(), node.id.clone()));
                }
            }
            NodeKind::NetworkIn { port, stream, from } => {
                if *port == 0 {
                    errors.push(GraphError::BadNetwork(
                        node.id.clone(),
                        "the port must be between 1 and 65535".into(),
                    ));
                } else if let Some(other) = listen_ports.insert(*port, &node.id) {
                    errors.push(GraphError::SamePort(other.clone(), node.id.clone(), *port));
                }
                if stream.chars().count() > omu_types::MAX_STREAM_NAME || !stream.is_ascii() {
                    errors.push(GraphError::BadNetwork(
                        node.id.clone(),
                        format!(
                            "the stream name is at most {} plain characters",
                            omu_types::MAX_STREAM_NAME
                        ),
                    ));
                }
                if !from.trim().is_empty() && from.trim().parse::<std::net::IpAddr>().is_err() {
                    errors.push(GraphError::BadNetwork(
                        node.id.clone(),
                        format!("{from:?} is not an address (like 192.168.1.20)"),
                    ));
                }
            }
            NodeKind::NetworkOut { host, port, stream } => {
                if host.trim().is_empty() {
                    errors.push(GraphError::BadNetwork(
                        node.id.clone(),
                        "there is no computer to send the sound to".into(),
                    ));
                }
                if *port == 0 {
                    errors.push(GraphError::BadNetwork(
                        node.id.clone(),
                        "the port must be between 1 and 65535".into(),
                    ));
                }
                if stream.chars().count() > omu_types::MAX_STREAM_NAME || !stream.is_ascii() {
                    errors.push(GraphError::BadNetwork(
                        node.id.clone(),
                        format!(
                            "the stream name is at most {} plain characters",
                            omu_types::MAX_STREAM_NAME
                        ),
                    ));
                }
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
    use omu_types::{AppTarget, NodeSpec, PortRef};

    fn node(id: &str, kind: NodeKind) -> NodeSpec {
        NodeSpec {
            id: id.into(),
            display: None,
            inputs: Default::default(),
            muted: false,
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
    fn flows_connect_when_the_input_takes_the_type_of_the_output() {
        let check = |from: NodeKind, fp: &str, to: NodeKind, tp: &str| {
            let spec = GraphSpec {
                nodes: vec![node("a", from), node("b", to)],
                edges: vec![edge("a", fp, "b", tp)],
            };
            validate(&spec).is_ok()
        };
        let int = || NodeKind::Integer { value: 3 };
        let float = || NodeKind::Number { value: 1.5 };
        let text = || NodeKind::Text { value: "x".into() };
        let join = || NodeKind::Join {
            template: "$1".into(),
            args: 1,
        };
        let math = || NodeKind::Math {
            op: omu_types::MathOp::Add,
        };
        // a whole number goes where a decimal one is wanted, and numbers go into text
        assert!(check(int(), "out", math(), "a"));
        assert!(check(float(), "out", math(), "b"));
        assert!(check(int(), "out", join(), "arg1"));
        assert!(check(float(), "out", join(), "arg1"));
        assert!(check(text(), "out", join(), "arg1"));
        assert!(check(
            NodeKind::Variable { initial: true },
            "value",
            join(),
            "arg1"
        ));
        // not the other way round, and not across kinds that do not convert
        assert!(!check(float(), "out", NodeKind::Hold { ms: 10 }, "ms"));
        assert!(!check(text(), "out", math(), "a"));
        assert!(!check(text(), "out", gain(), "db"));
        assert!(!check(app(), "out", join(), "arg1"));
        // a flow can give the setting of a block
        assert!(check(int(), "out", NodeKind::Hold { ms: 10 }, "ms"));
        assert!(check(float(), "out", gain(), "db"));
        assert!(check(
            int(),
            "out",
            NodeKind::Timer { interval_ms: 1000 },
            "interval_ms"
        ));
    }

    #[test]
    fn a_setting_takes_one_flow_only() {
        let spec = GraphSpec {
            nodes: vec![
                node("a", NodeKind::Number { value: 1.0 }),
                node("b", NodeKind::Number { value: 2.0 }),
                node("g", gain()),
            ],
            edges: vec![edge("a", "out", "g", "db"), edge("b", "out", "g", "db")],
        };
        assert!(validate(&spec)
            .unwrap_err()
            .iter()
            .any(|e| matches!(e, GraphError::FanIn(..))));
    }

    #[test]
    fn a_text_template_has_to_be_readable_and_name_values_that_exist() {
        let errors = |template: &str| {
            validate(&GraphSpec {
                nodes: vec![node(
                    "j",
                    NodeKind::Join {
                        template: template.into(),
                        args: 2,
                    },
                )],
                edges: vec![],
            })
            .err()
            .unwrap_or_default()
        };
        assert!(errors("$arg1 and ${arg2:none} cost \\$5").is_empty());
        for bad in ["$arg3", "${arg1", "price: $", "\\q"] {
            assert!(
                errors(bad)
                    .iter()
                    .any(|e| matches!(e, GraphError::BadValue(..))),
                "{bad}"
            );
        }
    }

    #[test]
    fn what_is_typed_in_a_block_has_to_fit_an_input_of_that_block() {
        use omu_types::{InputValue, MathOp};
        let with = |name: &str, value: InputValue| {
            let mut m = node("m", NodeKind::Math { op: MathOp::Add });
            m.inputs.insert(name.into(), value);
            validate(&GraphSpec {
                nodes: vec![m],
                edges: vec![],
            })
        };
        assert!(with("a", InputValue::Number(2.5)).is_ok());
        assert!(
            with("a", InputValue::Text("x".into())).is_err(),
            "text where a number goes"
        );
        assert!(with("a", InputValue::Number(f64::INFINITY)).is_err());
        assert!(
            with("nope", InputValue::Number(1.0)).is_err(),
            "an input that does not exist"
        );
        assert!(
            with("out", InputValue::Number(1.0)).is_err(),
            "an output is not an input"
        );
    }

    #[test]
    fn every_problem_is_data_with_a_code_the_block_and_the_connection_point() {
        // a sound output into a number input, a loop, two links into one input
        let spec = GraphSpec {
            nodes: vec![node("a", app()), node("g", gain()), node("h", gain())],
            edges: vec![
                edge("a", "out", "g", "db"),
                edge("g", "out", "h", "in"),
                edge("h", "out", "g", "in"),
            ],
        };
        let found = diagnose(&spec);
        let mismatch = found
            .iter()
            .find(|d| d.code == "type_mismatch")
            .expect("a sound does not go into a number");
        assert_eq!(mismatch.node, Some("g".into()));
        assert_eq!(mismatch.port.as_deref(), Some("db"));
        assert!(mismatch.hint.is_some(), "it says what to do");
        assert!(found.iter().any(|d| d.code == "cycle"));
        assert!(found
            .iter()
            .all(|d| d.severity == omu_types::Severity::Error));
        // the text is still the error's own
        assert!(found.iter().all(|d| !d.message.is_empty()));
        assert!(diagnose(&GraphSpec::default()).is_empty());
    }

    #[test]
    fn values_are_checked() {
        let bad = |kind: NodeKind| {
            validate(&GraphSpec {
                nodes: vec![node("x", kind)],
                edges: vec![],
            })
            .unwrap_err()
        };
        assert!(bad(NodeKind::Join {
            template: "$1".into(),
            args: 0
        })
        .iter()
        .any(|e| matches!(e, GraphError::BadValue(..))));
        assert!(bad(NodeKind::Join {
            template: "$1".into(),
            args: 99
        })
        .iter()
        .any(|e| matches!(e, GraphError::BadValue(..))));
        assert!(bad(NodeKind::Number { value: f64::NAN })
            .iter()
            .any(|e| matches!(e, GraphError::BadValue(..))));
        assert!(bad(NodeKind::Text {
            value: "x".repeat(5000)
        })
        .iter()
        .any(|e| matches!(e, GraphError::BadValue(..))));
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
    fn several_sounds_can_enter_one_sound_input_and_are_added() {
        let two_sources = |sink: NodeKind| GraphSpec {
            nodes: vec![node("a", app()), node("b", app()), node("s", sink)],
            edges: vec![edge("a", "out", "s", "in"), edge("b", "out", "s", "in")],
        };
        for sink in [
            NodeKind::Mixer,
            gain(),
            mic("M"),
            NodeKind::Gate,
            NodeKind::Delay { ms: 10 },
        ] {
            assert!(validate(&two_sources(sink.clone())).is_ok(), "{sink:?}");
        }
        // the same output twice into one input would only double the sound: that is a mistake
        let twice = GraphSpec {
            nodes: vec![node("a", app()), node("g", gain())],
            edges: vec![edge("a", "out", "g", "in"), edge("a", "out", "g", "in")],
        };
        assert!(validate(&twice)
            .unwrap_err()
            .iter()
            .any(|e| matches!(e, GraphError::DuplicateEdge(..))));
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
    fn virtual_speakers_need_distinct_names_but_may_share_one_with_a_microphone() {
        let speaker = |name: &str| NodeKind::VirtualSpeaker { name: name.into() };
        let spec = GraphSpec {
            nodes: vec![
                node("s1", speaker("Game")),
                node("s2", speaker(" game")),
                node("s3", speaker("")),
                // a microphone and a speaker of the same name are two different devices
                node("m1", mic("Game")),
            ],
            edges: vec![],
        };
        let e = validate(&spec).unwrap_err();
        assert!(e.contains(&GraphError::SameVirtualName("s1".into(), "s2".into())));
        assert!(e.contains(&GraphError::EmptyVirtualName("s3".into())));
        assert!(!e
            .iter()
            .any(|x| matches!(x, GraphError::SameVirtualName(a, b) if a.0 == "m1" || b.0 == "m1")));
    }

    #[test]
    fn a_virtual_speaker_is_an_input_that_sends_audio() {
        let k = NodeKind::VirtualSpeaker { name: "x".into() };
        assert_eq!(k.category(), omu_types::Category::Input);
        assert!(k.is_source() && k.needs_stream());
        assert_eq!(k.output_types(), vec![omu_types::PortType::Audio]);
        assert!(k.port("out").is_some() && k.port("in").is_none());
    }

    #[test]
    fn the_example_chain_of_a_key_a_variable_and_a_gate_is_valid() {
        let spec = GraphSpec {
            nodes: vec![
                node("key", NodeKind::Hotkey { keys: "F8".into() }),
                node("mute", NodeKind::Variable { initial: true }),
                node("app", app()),
                node("gate", NodeKind::Gate),
                node("mic", mic("M")),
            ],
            edges: vec![
                edge("key", "pressed", "mute", "toggle"),
                edge("mute", "value", "gate", "open"),
                edge("app", "out", "gate", "in"),
                edge("gate", "out", "mic", "in"),
            ],
        };
        let v = validate(&spec).unwrap();
        let pos = |name: &str| {
            v.order
                .iter()
                .position(|&i| v.spec.nodes[i].id.0 == name)
                .unwrap()
        };
        assert!(pos("key") < pos("mute") && pos("mute") < pos("gate") && pos("gate") < pos("mic"));
    }

    #[test]
    fn flows_of_different_types_do_not_connect() {
        let spec = GraphSpec {
            nodes: vec![
                node("key", NodeKind::Hotkey { keys: "F8".into() }),
                node("gate", NodeKind::Gate),
                node("app", app()),
                node("var", NodeKind::Variable { initial: false }),
            ],
            edges: vec![
                edge("key", "pressed", "gate", "open"), // an action into a condition input
                edge("app", "out", "var", "toggle"),    // audio into an action input
            ],
        };
        let e = validate(&spec).unwrap_err();
        assert_eq!(
            e.iter()
                .filter(|x| matches!(x, GraphError::TypeMismatch { .. }))
                .count(),
            2,
            "{e:?}"
        );
    }

    #[test]
    fn several_triggers_may_toggle_one_variable_but_one_condition_drives_a_gate() {
        let triggers = |n: usize| GraphSpec {
            nodes: (0..n)
                .map(|i| {
                    node(
                        &format!("k{i}"),
                        NodeKind::Hotkey {
                            keys: format!("F{}", i + 1),
                        },
                    )
                })
                .chain([node("var", NodeKind::Variable { initial: false })])
                .collect(),
            edges: (0..n)
                .map(|i| edge(&format!("k{i}"), "pressed", "var", "toggle"))
                .collect(),
        };
        assert!(validate(&triggers(3)).is_ok());
        let two_conditions = GraphSpec {
            nodes: vec![
                node("a", NodeKind::Variable { initial: true }),
                node("b", NodeKind::Variable { initial: true }),
                node("gate", NodeKind::Gate),
            ],
            edges: vec![
                edge("a", "value", "gate", "open"),
                edge("b", "value", "gate", "open"),
            ],
        };
        let e = validate(&two_conditions).unwrap_err();
        assert!(e.contains(&GraphError::FanIn("gate".into(), "open".into())));
    }

    #[test]
    fn shortcuts_and_timers_are_checked() {
        let spec = GraphSpec {
            nodes: vec![
                node(
                    "a",
                    NodeKind::Hotkey {
                        keys: "ctrl+f8".into(),
                    },
                ),
                node(
                    "b",
                    NodeKind::Hotkey {
                        keys: "Ctrl+F8".into(),
                    },
                ),
                node(
                    "c",
                    NodeKind::Hotkey {
                        keys: "Ctrl".into(),
                    },
                ),
                node("t", NodeKind::Timer { interval_ms: 2 }),
            ],
            edges: vec![],
        };
        let e = validate(&spec).unwrap_err();
        assert!(
            e.iter().any(
                |x| matches!(x, GraphError::SameShortcut(a, b, _) if a.0 == "a" && b.0 == "b")
            ),
            "{e:?}"
        );
        assert!(e
            .iter()
            .any(|x| matches!(x, GraphError::BadShortcut(n, _) if n.0 == "c")));
        assert!(e.iter().any(|x| matches!(x, GraphError::TimerTooFast(..))));
    }

    #[test]
    fn a_condition_loop_is_a_cycle_too() {
        use omu_types::LogicOp;
        let spec = GraphSpec {
            nodes: vec![
                node("a", NodeKind::Logic { op: LogicOp::Not }),
                node("b", NodeKind::Logic { op: LogicOp::Not }),
            ],
            edges: vec![edge("a", "out", "b", "a"), edge("b", "out", "a", "a")],
        };
        assert!(validate(&spec)
            .unwrap_err()
            .iter()
            .any(|x| matches!(x, GraphError::Cycle(_))));
    }

    #[test]
    fn a_branch_routes_a_sound_by_conditions_and_every_output_can_be_used() {
        let spec = GraphSpec {
            nodes: vec![
                node("app", app()),
                node("v", NodeKind::Variable { initial: true }),
                node("w", NodeKind::Variable { initial: false }),
                node("br", NodeKind::Branch { branches: 2 }),
                node("m1", mic("One")),
                node("m2", mic("Two")),
                node("m3", mic("Else")),
            ],
            edges: vec![
                edge("app", "out", "br", "in"),
                edge("v", "value", "br", "when1"),
                edge("w", "value", "br", "when2"),
                edge("br", "out1", "m1", "in"),
                edge("br", "out2", "m2", "in"),
                edge("br", "else", "m3", "in"),
            ],
        };
        assert!(validate(&spec).is_ok());
        // a third condition does not exist on a block with two
        let mut bad = spec.clone();
        bad.edges.push(edge("v", "value", "br", "when3"));
        assert!(validate(&bad)
            .unwrap_err()
            .iter()
            .any(|e| matches!(e, GraphError::UnknownPort { port, .. } if port == "when3")));
        // a sound cannot be wired where a condition is expected
        let mut wrong = spec.clone();
        wrong.edges.push(edge("app", "out", "br", "when2"));
        assert!(validate(&wrong)
            .unwrap_err()
            .iter()
            .any(|e| matches!(e, GraphError::TypeMismatch { .. })));
    }

    #[test]
    fn conditions_that_combine_take_many_links_and_not_takes_one() {
        let vars = |n: usize| -> Vec<NodeSpec> {
            (0..n)
                .map(|i| node(&format!("v{i}"), NodeKind::Variable { initial: false }))
                .collect()
        };
        let into = |kind: NodeKind, links: usize| {
            let mut nodes = vars(links);
            nodes.push(node("c", kind));
            GraphSpec {
                edges: (0..links)
                    .map(|i| edge(&format!("v{i}"), "value", "c", "in"))
                    .collect(),
                nodes,
            }
        };
        assert!(validate(&into(NodeKind::All, 3)).is_ok());
        assert!(validate(&into(NodeKind::Any, 3)).is_ok());
        assert!(validate(&into(NodeKind::Not, 1)).is_ok());
        assert!(validate(&into(NodeKind::Not, 2))
            .unwrap_err()
            .contains(&GraphError::FanIn("c".into(), "in".into())));
    }

    #[test]
    fn the_settings_of_the_new_blocks_have_limits() {
        let spec = GraphSpec {
            nodes: vec![
                node("b0", NodeKind::Branch { branches: 0 }),
                node("s99", NodeKind::Select { branches: 99 }),
                node("d", NodeKind::Delay { ms: 60_000 }),
                node("h", NodeKind::Hold { ms: 10_000_000 }),
                node("ok1", NodeKind::Branch { branches: 16 }),
                node("ok2", NodeKind::Delay { ms: 5_000 }),
                node("ok3", NodeKind::After { ms: 0 }),
            ],
            edges: vec![],
        };
        let e = validate(&spec).unwrap_err();
        assert_eq!(e.len(), 4, "{e:?}");
        assert!(e.contains(&GraphError::BadBranches("b0".into(), 0)));
        assert!(e.contains(&GraphError::BadBranches("s99".into(), 99)));
        assert!(e.contains(&GraphError::DelayTooLong("d".into(), 60_000)));
        assert!(e.contains(&GraphError::WaitTooLong("h".into(), 10_000_000)));
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
