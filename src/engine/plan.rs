//! The executable form of a graph: nodes in evaluation order, each with a pre-allocated output. The graph thread runs
//! [`Plan::process`] once per cycle; nothing in it allocates or blocks.
//!
//! Three kinds of flow travel along the edges, all evaluated in the same cycle, in the order of the graph:
//!
//! * **audio**: a buffer of frames (a node's `audio`);
//! * **condition**: a yes/no that holds until the node says otherwise (a node's `cond`);
//! * **action**: how many times something happened since the previous cycle (a node's `actions`): a key was pressed,
//!   a timer fired. An action is consumed by the nodes it reaches (a variable changes) and then forgotten.

use super::bridge::{SinkProducer, SourceConsumer};
use crate::dsp::{add_into, db_to_linear, peak, SmoothedGain};
use crate::types::{LogicOp, NodeId, ENGINE_CHANNELS, ENGINE_SAMPLE_RATE};
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

const CH: usize = ENGINE_CHANNELS as usize;
/// Time a gain change takes to complete a full 0 -> 1 move (also the fade of a gate).
const GAIN_RAMP_MS: f32 = 10.0;
/// How long a level detector stays on after the sound stops, so that speech is not chopped between words.
const LEVEL_HOLD_MS: u32 = 300;

/// What a node shows to the status API: the loudest sample since the last reading, the node's yes/no when it has
/// one, and how many actions it sent.
#[derive(Debug)]
pub struct Meter {
    /// Highest absolute sample (non-negative `f32` bits order like integers).
    peak: AtomicU32,
    /// 0 = no condition, 1 = false, 2 = true.
    state: AtomicU8,
    events: AtomicU64,
}

impl Default for Meter {
    fn default() -> Self {
        Meter {
            peak: AtomicU32::new(0),
            state: AtomicU8::new(0),
            events: AtomicU64::new(0),
        }
    }
}

impl Meter {
    pub fn record(&self, samples: &[f32]) {
        self.peak
            .fetch_max(peak(samples).to_bits(), Ordering::Relaxed);
    }
    /// Returns the peak since the previous call and resets it.
    pub fn take(&self) -> f32 {
        f32::from_bits(self.peak.swap(0, Ordering::Relaxed))
    }
    pub fn set_condition(&self, value: bool) {
        self.state
            .store(if value { 2 } else { 1 }, Ordering::Relaxed);
    }
    /// The node's condition now, for a node that has one.
    pub fn condition(&self) -> Option<bool> {
        match self.state.load(Ordering::Relaxed) {
            0 => None,
            s => Some(s == 2),
        }
    }
    pub fn add_events(&self, n: u32) {
        if n > 0 {
            self.events.fetch_add(n as u64, Ordering::Relaxed);
        }
    }
    /// Actions the node has sent since the plan started.
    pub fn events(&self) -> u64 {
        self.events.load(Ordering::Relaxed)
    }
}

/// An input of a node, by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Port {
    In,
    Open,
    Set,
    Clear,
    Toggle,
    A,
    B,
}

impl Port {
    pub fn from_name(name: &str) -> Option<Port> {
        Some(match name {
            "in" => Port::In,
            "open" => Port::Open,
            "set" => Port::Set,
            "clear" => Port::Clear,
            "toggle" => Port::Toggle,
            "a" => Port::A,
            "b" => Port::B,
            _ => return None,
        })
    }
}

/// An edge as the node at its end sees it: where it comes from (a position in the plan) and which input it enters.
#[derive(Clone, Copy, Debug)]
pub struct Wire {
    pub from: usize,
    pub port: Port,
}

pub enum NodeOp {
    /// Audio from a device (the lock is only ever taken by the graph thread).
    Source(Arc<Mutex<SourceConsumer>>),
    /// A source whose stream is not running: silence.
    SilentSource,
    /// Audio to a device.
    Sink(Arc<Mutex<SinkProducer>>),
    /// A sink whose stream is not running: the audio is discarded.
    DiscardSink,
    Gain(SmoothedGain),
    Mixer,
    /// Audio gated by the condition at `open` (always open without one).
    Gate(SmoothedGain),
    Variable {
        state: bool,
    },
    Level {
        threshold: f32,
        hold_left: u32,
    },
    Logic(LogicOp),
    /// Actions counted by another thread (a keyboard shortcut) and handed over each cycle.
    Trigger(Arc<AtomicU32>),
    Timer {
        interval_frames: u64,
        elapsed: u64,
    },
    /// Tests only: audio that never runs dry.
    #[cfg(test)]
    Constant(f32),
}

pub struct PlanNode {
    pub id: NodeId,
    pub op: NodeOp,
    pub inputs: Vec<Wire>,
    pub meter: Arc<Meter>,
}

/// What a node produced this cycle.
#[derive(Default)]
struct Out {
    audio: Vec<f32>,
    cond: bool,
    actions: u32,
}

pub struct Plan {
    pub nodes: Vec<PlanNode>,
    outs: Vec<Out>,
    silence: Vec<f32>,
    cycle_frames: usize,
}

impl Plan {
    pub fn new(nodes: Vec<PlanNode>, cycle_frames: usize) -> Plan {
        let cycle_samples = cycle_frames * CH;
        let outs = nodes
            .iter()
            .map(|n| match n.op {
                NodeOp::Source(_)
                | NodeOp::SilentSource
                | NodeOp::Gain(_)
                | NodeOp::Mixer
                | NodeOp::Gate(_) => Out {
                    audio: vec![0.0; cycle_samples],
                    ..Default::default()
                },
                #[cfg(test)]
                NodeOp::Constant(_) => Out {
                    audio: vec![0.0; cycle_samples],
                    ..Default::default()
                },
                _ => Out::default(),
            })
            .collect();
        Plan {
            outs,
            nodes,
            silence: vec![0.0; cycle_samples],
            cycle_frames,
        }
    }

    pub fn cycle_samples(&self) -> usize {
        self.cycle_frames * CH
    }

    /// A gain node of this plan, built for the engine format, with `db` as target.
    pub fn gain(db: f32) -> SmoothedGain {
        SmoothedGain::new(db, ENGINE_SAMPLE_RATE, GAIN_RAMP_MS)
    }

    /// A gate: closed or open as the condition says, fading over a few milliseconds.
    pub fn gate() -> SmoothedGain {
        Plan::gain(0.0)
    }

    /// Threshold of a level detector, from decibels.
    pub fn level_threshold(db: f32) -> f32 {
        db_to_linear(db)
    }

    /// Continues the state of the nodes of `old` that exist, of the same kind, here: gains and gates do not jump, a
    /// variable keeps its value, a level detector its hold, a timer its phase.
    pub fn inherit(&mut self, old: &Plan) {
        for node in &mut self.nodes {
            let Some(before) = old.nodes.iter().find(|n| n.id == node.id) else {
                continue;
            };
            match (&mut node.op, &before.op) {
                (NodeOp::Gain(g), NodeOp::Gain(o)) | (NodeOp::Gate(g), NodeOp::Gate(o)) => {
                    g.start_from(o.current())
                }
                (NodeOp::Variable { state }, NodeOp::Variable { state: o }) => *state = *o,
                (NodeOp::Level { hold_left, .. }, NodeOp::Level { hold_left: o, .. }) => {
                    *hold_left = *o
                }
                (NodeOp::Timer { elapsed, .. }, NodeOp::Timer { elapsed: o, .. }) => *elapsed = *o,
                _ => {}
            }
        }
    }

    /// Runs one cycle.
    pub fn process(&mut self) {
        let hold_cycles =
            (LEVEL_HOLD_MS as u64 * ENGINE_SAMPLE_RATE as u64 / 1000 / self.cycle_frames as u64)
                .max(1) as u32;
        for i in 0..self.nodes.len() {
            let mut audio = std::mem::take(&mut self.outs[i].audio);
            let (mut cond, mut actions) = (false, 0u32);
            let node = &mut self.nodes[i];
            let wire = |port: Port| node.inputs.iter().find(|w| w.port == port).copied();
            let audio_in = |outs: &'_ [Out], port: Port| -> Option<usize> {
                wire(port)
                    .map(|w| w.from)
                    .filter(|&j| j != i && j < outs.len())
            };
            match &mut node.op {
                NodeOp::Source(consumer) => match consumer.try_lock() {
                    Ok(mut c) => c.pull(&mut audio),
                    Err(_) => audio.fill(0.0),
                },
                NodeOp::SilentSource => audio.fill(0.0),
                #[cfg(test)]
                NodeOp::Constant(v) => audio.fill(*v),
                NodeOp::Gain(g) => {
                    let input = audio_in(&self.outs, Port::In)
                        .map_or(&self.silence[..], |j| &self.outs[j].audio[..]);
                    audio.copy_from_slice(input);
                    g.apply(&mut audio, CH);
                }
                NodeOp::Gate(g) => {
                    let open = wire(Port::Open).is_none_or(|w| self.outs[w.from].cond);
                    g.set_db(if open { 0.0 } else { -120.0 });
                    let input = audio_in(&self.outs, Port::In)
                        .map_or(&self.silence[..], |j| &self.outs[j].audio[..]);
                    audio.copy_from_slice(input);
                    g.apply(&mut audio, CH);
                    cond = open;
                }
                NodeOp::Mixer => {
                    audio.fill(0.0);
                    for w in node.inputs.iter().filter(|w| w.port == Port::In) {
                        add_into(&mut audio, &self.outs[w.from].audio);
                    }
                }
                NodeOp::Sink(producer) => {
                    let input = audio_in(&self.outs, Port::In)
                        .map_or(&self.silence[..], |j| &self.outs[j].audio[..]);
                    node.meter.record(input);
                    if let Ok(mut p) = producer.try_lock() {
                        p.push(input);
                    }
                }
                NodeOp::DiscardSink => {
                    let input = audio_in(&self.outs, Port::In)
                        .map_or(&self.silence[..], |j| &self.outs[j].audio[..]);
                    node.meter.record(input);
                }
                NodeOp::Variable { state } => {
                    let count = |port: Port| -> u32 {
                        node.inputs
                            .iter()
                            .filter(|w| w.port == port)
                            .map(|w| self.outs[w.from].actions)
                            .sum()
                    };
                    if count(Port::Set) > 0 {
                        *state = true;
                    }
                    if count(Port::Clear) > 0 {
                        *state = false;
                    }
                    if count(Port::Toggle) % 2 == 1 {
                        *state = !*state;
                    }
                    cond = *state;
                }
                NodeOp::Level {
                    threshold,
                    hold_left,
                } => {
                    let input = audio_in(&self.outs, Port::In)
                        .map_or(&self.silence[..], |j| &self.outs[j].audio[..]);
                    node.meter.record(input);
                    if peak(input) > *threshold {
                        *hold_left = hold_cycles;
                    } else {
                        *hold_left = hold_left.saturating_sub(1);
                    }
                    cond = *hold_left > 0;
                }
                NodeOp::Logic(op) => {
                    let value = |port: Port| wire(port).is_some_and(|w| self.outs[w.from].cond);
                    let (a, b) = (value(Port::A), value(Port::B));
                    cond = match op {
                        LogicOp::And => a && b,
                        LogicOp::Or => a || b,
                        LogicOp::Xor => a != b,
                        LogicOp::Not => !a,
                    };
                }
                NodeOp::Trigger(events) => actions = events.swap(0, Ordering::AcqRel),
                NodeOp::Timer {
                    interval_frames,
                    elapsed,
                } => {
                    *elapsed += self.cycle_frames as u64;
                    if *elapsed >= *interval_frames {
                        actions = (*elapsed / *interval_frames).min(u32::MAX as u64) as u32;
                        *elapsed %= *interval_frames;
                    }
                }
            }
            match node.op {
                NodeOp::Sink(_) | NodeOp::DiscardSink | NodeOp::Level { .. } => {}
                NodeOp::Variable { .. }
                | NodeOp::Logic(_)
                | NodeOp::Trigger(_)
                | NodeOp::Timer { .. } => {}
                _ => node.meter.record(&audio),
            }
            if matches!(
                node.op,
                NodeOp::Variable { .. } | NodeOp::Logic(_) | NodeOp::Level { .. } | NodeOp::Gate(_)
            ) {
                node.meter.set_condition(cond);
            }
            node.meter.add_events(actions);
            let out = &mut self.outs[i];
            out.audio = audio;
            out.cond = cond;
            out.actions = actions;
        }
    }

    /// The audio output of node `i` after the last cycle (for tests).
    pub fn output(&self, i: usize) -> &[f32] {
        &self.outs[i].audio
    }

    /// The condition output of node `i` after the last cycle.
    pub fn condition(&self, i: usize) -> bool {
        self.outs[i].cond
    }

    /// The actions node `i` sent in the last cycle.
    pub fn actions(&self, i: usize) -> u32 {
        self.outs[i].actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::bridge::{sink_bridge, source_bridge};
    use crate::types::AudioFormat;

    fn wire(from: usize) -> Wire {
        Wire {
            from,
            port: Port::In,
        }
    }
    fn wire_to(from: usize, port: Port) -> Wire {
        Wire { from, port }
    }
    fn node(id: &str, op: NodeOp, inputs: Vec<Wire>) -> PlanNode {
        PlanNode {
            id: id.into(),
            op,
            inputs,
            meter: Arc::default(),
        }
    }

    /// A source node playing a constant, built from a bridge fed with enough audio for `cycles` cycles.
    fn constant_source(value: f32, cycles: usize) -> NodeOp {
        let (mut p, c, _) = source_bridge(AudioFormat::ENGINE, 100.0);
        for _ in 0..cycles + 12 {
            p.push(&vec![value; 960]);
        }
        NodeOp::Source(Arc::new(Mutex::new(c)))
    }

    #[test]
    fn gain_node_scales_its_input_and_meters_it() {
        let mut plan = Plan::new(
            vec![
                node("src", constant_source(0.5, 10), vec![]),
                node("g", NodeOp::Gain(Plan::gain(-6.0206)), vec![wire(0)]),
            ],
            480,
        );
        for _ in 0..10 {
            plan.process();
        }
        let tail = plan.output(1);
        assert!(tail.iter().all(|s| (*s - 0.25).abs() < 1e-2), "{}", tail[0]);
        assert!(plan.nodes[1].meter.take() > 0.2);
        assert_eq!(plan.nodes[1].meter.take(), 0.0, "take() resets the peak");
    }

    #[test]
    fn mixer_sums_and_unconnected_gain_is_silent() {
        let mut plan = Plan::new(
            vec![
                node("a", NodeOp::SilentSource, vec![]),
                node("b", NodeOp::SilentSource, vec![]),
                node("m", NodeOp::Mixer, vec![wire(0), wire(1)]),
                node("g", NodeOp::Gain(Plan::gain(0.0)), vec![]),
            ],
            480,
        );
        plan.process();
        assert!(plan.output(2).iter().all(|s| *s == 0.0));
        assert!(plan.output(3).iter().all(|s| *s == 0.0));
    }

    #[test]
    fn sink_pushes_its_input_into_the_bridge() {
        let (sp, mut sc, _) = sink_bridge(AudioFormat::ENGINE, 10.0);
        let mut plan = Plan::new(
            vec![
                node("src", constant_source(0.3, 6), vec![]),
                node("out", NodeOp::Sink(Arc::new(Mutex::new(sp))), vec![wire(0)]),
            ],
            480,
        );
        for _ in 0..6 {
            plan.process();
        }
        let mut out = vec![0.0f32; 960];
        sc.pull(&mut out);
        assert!(out.iter().any(|s| (*s - 0.3).abs() < 1e-2));
    }

    #[test]
    fn inherit_continues_the_gain_instead_of_jumping() {
        let mut old = Plan::new(
            vec![node("g", NodeOp::Gain(Plan::gain(-20.0)), vec![])],
            480,
        );
        old.process();
        let current = |p: &Plan| match &p.nodes[0].op {
            NodeOp::Gain(g) => g.current(),
            _ => unreachable!(),
        };
        let before = current(&old);
        let mut new = Plan::new(vec![node("g", NodeOp::Gain(Plan::gain(0.0)), vec![])], 480);
        new.inherit(&old);
        assert_eq!(before, current(&new));
    }

    // ---- conditions and actions

    /// hotkey -> variable.toggle -> gate.open on audio from a constant source
    fn gated_chain(initial: bool) -> (Plan, Arc<AtomicU32>) {
        let events = Arc::new(AtomicU32::new(0));
        let plan = Plan::new(
            vec![
                node("src", NodeOp::Constant(0.5), vec![]),
                node("key", NodeOp::Trigger(events.clone()), vec![]),
                node(
                    "var",
                    NodeOp::Variable { state: initial },
                    vec![wire_to(1, Port::Toggle)],
                ),
                node(
                    "gate",
                    NodeOp::Gate(Plan::gate()),
                    vec![wire(0), wire_to(2, Port::Open)],
                ),
            ],
            480,
        );
        (plan, events)
    }

    fn run(plan: &mut Plan, cycles: usize) {
        for _ in 0..cycles {
            plan.process();
        }
    }

    #[test]
    fn a_key_press_toggles_a_variable_that_opens_and_closes_a_gate() {
        let (mut plan, key) = gated_chain(true);
        run(&mut plan, 5);
        assert!(
            plan.condition(2) && plan.output(3).iter().all(|s| (s - 0.5).abs() < 1e-3),
            "starts open: audio passes"
        );

        key.fetch_add(1, Ordering::AcqRel); // the user presses the key
        run(&mut plan, 5);
        assert!(!plan.condition(2), "the press toggled the variable off");
        assert!(
            plan.output(3).iter().all(|s| s.abs() < 1e-3),
            "the gate is closed: silence"
        );
        assert_eq!(
            plan.nodes[1].meter.events(),
            1,
            "the trigger reports its action"
        );
        assert_eq!(plan.nodes[2].meter.condition(), Some(false));

        key.fetch_add(1, Ordering::AcqRel);
        run(&mut plan, 5);
        assert!(
            plan.condition(2) && plan.output(3).iter().all(|s| (s - 0.5).abs() < 1e-3),
            "pressed again: open"
        );
    }

    #[test]
    fn an_action_acts_once() {
        let (mut plan, key) = gated_chain(false);
        key.fetch_add(1, Ordering::AcqRel);
        run(&mut plan, 1);
        assert_eq!(plan.actions(1), 1);
        run(&mut plan, 1);
        assert_eq!(plan.actions(1), 0, "the event is gone next cycle");
        assert!(plan.condition(2), "but the variable remembers");
        run(&mut plan, 20);
        assert!(
            plan.condition(2),
            "and keeps its value without further actions"
        );
    }

    #[test]
    fn two_presses_in_one_cycle_cancel_out() {
        let (mut plan, key) = gated_chain(false);
        key.fetch_add(2, Ordering::AcqRel);
        run(&mut plan, 1);
        assert!(!plan.condition(2));
    }

    #[test]
    fn set_and_clear_override_and_toggle_applies_after() {
        let a = Arc::new(AtomicU32::new(0));
        let b = Arc::new(AtomicU32::new(0));
        let mut plan = Plan::new(
            vec![
                node("set", NodeOp::Trigger(a.clone()), vec![]),
                node("clear", NodeOp::Trigger(b.clone()), vec![]),
                node(
                    "var",
                    NodeOp::Variable { state: false },
                    vec![wire_to(0, Port::Set), wire_to(1, Port::Clear)],
                ),
            ],
            480,
        );
        a.store(1, Ordering::Release);
        run(&mut plan, 1);
        assert!(plan.condition(2), "set makes it true");
        b.store(1, Ordering::Release);
        run(&mut plan, 1);
        assert!(!plan.condition(2), "clear makes it false");
    }

    #[test]
    fn a_gate_without_a_condition_is_open_and_without_audio_is_silent() {
        let mut plan = Plan::new(
            vec![
                node("src", NodeOp::Constant(0.4), vec![]),
                node("gate", NodeOp::Gate(Plan::gate()), vec![wire(0)]),
            ],
            480,
        );
        run(&mut plan, 5);
        assert!(plan.output(1).iter().all(|s| (s - 0.4).abs() < 1e-3));
        let mut plan = Plan::new(vec![node("gate", NodeOp::Gate(Plan::gate()), vec![])], 480);
        run(&mut plan, 3);
        assert!(plan.output(0).iter().all(|s| *s == 0.0));
    }

    #[test]
    fn the_gate_fades_instead_of_clicking() {
        let (mut plan, key) = gated_chain(true);
        run(&mut plan, 3);
        key.fetch_add(1, Ordering::AcqRel);
        plan.process(); // the cycle where it closes
        let out = plan.output(3);
        let steps = out.chunks(2).map(|f| f[0]).collect::<Vec<_>>();
        assert!(
            steps.windows(2).all(|w| (w[0] - w[1]).abs() < 0.02),
            "no jump larger than a small step"
        );
        assert!(
            steps.iter().any(|s| *s > 0.01 && *s < 0.49),
            "passes through intermediate levels"
        );
    }

    #[test]
    fn a_level_detector_opens_on_sound_holds_then_closes() {
        let mut plan = Plan::new(
            vec![
                node("src", NodeOp::Constant(0.5), vec![]),
                node(
                    "lvl",
                    NodeOp::Level {
                        threshold: Plan::level_threshold(-30.0),
                        hold_left: 0,
                    },
                    vec![wire(0)],
                ),
            ],
            480,
        );
        run(&mut plan, 3);
        assert!(plan.condition(1), "loud audio: detected");
        plan.nodes[0].op = NodeOp::Constant(0.0); // the sound stops: the detector holds, then lets go
        run(&mut plan, 5);
        assert!(plan.condition(1), "still on during the hold (300 ms)");
        run(&mut plan, 60);
        assert!(!plan.condition(1), "silence for longer than the hold: off");
    }

    #[test]
    fn logic_combines_conditions() {
        let t = |op, a: bool, b: bool| {
            let mut plan = Plan::new(
                vec![
                    node("va", NodeOp::Variable { state: a }, vec![]),
                    node("vb", NodeOp::Variable { state: b }, vec![]),
                    node(
                        "l",
                        NodeOp::Logic(op),
                        vec![wire_to(0, Port::A), wire_to(1, Port::B)],
                    ),
                ],
                480,
            );
            plan.process();
            plan.condition(2)
        };
        assert!(t(LogicOp::And, true, true) && !t(LogicOp::And, true, false));
        assert!(t(LogicOp::Or, false, true) && !t(LogicOp::Or, false, false));
        assert!(t(LogicOp::Xor, true, false) && !t(LogicOp::Xor, true, true));
        assert!(
            t(LogicOp::Not, false, true) && !t(LogicOp::Not, true, false),
            "not ignores b"
        );
    }

    #[test]
    fn a_timer_fires_on_schedule() {
        // 100 ms interval at 10 ms cycles: one action every 10 cycles
        let mut plan = Plan::new(
            vec![node(
                "t",
                NodeOp::Timer {
                    interval_frames: 4800,
                    elapsed: 0,
                },
                vec![],
            )],
            480,
        );
        let mut fired = 0;
        for _ in 0..100 {
            plan.process();
            fired += plan.actions(0);
        }
        assert_eq!(fired, 10);
        assert_eq!(plan.nodes[0].meter.events(), 10);
    }

    #[test]
    fn state_survives_a_plan_swap() {
        let (mut old, key) = gated_chain(false);
        key.fetch_add(1, Ordering::AcqRel);
        run(&mut old, 2);
        assert!(old.condition(2));
        let (mut new, _) = gated_chain(false); // the new plan starts from "false"...
        new.inherit(&old);
        run(&mut new, 1);
        assert!(new.condition(2), "...but the variable kept its value");
    }
}
