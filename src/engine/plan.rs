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
use omu_dsp::{add_into, db_to_linear, peak, Denoiser, SmoothedGain};
use omu_soundboard::voices::BoardRuntime;
use omu_types::{
    port_number, CompareOp, LogicOp, MathOp, NodeId, PortLimits, RoundMode, TextOp,
    ENGINE_CHANNELS, ENGINE_SAMPLE_RATE, MAX_BRANCHES, MAX_DELAY_MS,
};
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CH: usize = ENGINE_CHANNELS as usize;
/// Time a gain change takes to complete a full 0 -> 1 move (also the fade of a gate).
const GAIN_RAMP_MS: f32 = 10.0;
/// How long a level detector stays on after the sound stops, so that speech is not chopped between words.
const LEVEL_HOLD_MS: u32 = 300;

/// How many cycles of input level a block remembers (ten seconds).
const LEVEL_HISTORY: usize = 1000;

/// What a block sends on a value flow (text, numbers), or its yes/no for a block that makes one.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Value {
    /// Nothing (the block makes no value, or none has arrived).
    #[default]
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(Arc<str>),
}

impl Value {
    /// The value as a number, for a flow that carries one.
    pub fn number(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// The value written out, as a text flow takes it.
    pub fn written(&self) -> String {
        match self {
            Value::None => String::new(),
            Value::Bool(b) => (if *b { "true" } else { "false" }).to_owned(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Text(t) => t.to_string(),
        }
    }
}

/// What a node shows to the status API: the loudest sample since the last reading, the node's yes/no when it has
/// one, the value it makes, and how many actions it sent.
#[derive(Debug)]
pub struct Meter {
    /// The value the block makes now, written out (only set when it changes).
    value: Mutex<Option<String>>,
    /// How loud the sound at the block's input was, cycle after cycle (half decibels above -120), as a ring.
    levels: Vec<AtomicU8>,
    levels_at: AtomicU32,
    /// Highest absolute sample (non-negative `f32` bits order like integers).
    peak: AtomicU32,
    /// 0 = no condition, 1 = false, 2 = true.
    state: AtomicU8,
    events: AtomicU64,
}

impl Default for Meter {
    fn default() -> Self {
        Meter {
            value: Mutex::new(None),
            levels: (0..LEVEL_HISTORY).map(|_| AtomicU8::new(0)).collect(),
            levels_at: AtomicU32::new(0),
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
    /// Writes down how loud the sound at the block's input is this cycle.
    pub fn record_level(&self, samples: &[f32]) {
        let db = 20.0 * f64::from(peak(samples)).max(1e-6).log10();
        let at = self.levels_at.fetch_add(1, Ordering::Relaxed) as usize % LEVEL_HISTORY;
        self.levels[at].store(
            ((db + 120.0) * 2.0).clamp(0.0, 240.0).round() as u8,
            Ordering::Relaxed,
        );
    }
    /// The levels written down, in decibels, the oldest first.
    pub fn levels(&self) -> Vec<f32> {
        let written = self.levels_at.load(Ordering::Relaxed) as usize;
        let (count, first) = if written >= LEVEL_HISTORY {
            (LEVEL_HISTORY, written % LEVEL_HISTORY)
        } else {
            (written, 0)
        };
        (0..count)
            .map(|k| {
                f32::from(self.levels[(first + k) % LEVEL_HISTORY].load(Ordering::Relaxed)) / 2.0
                    - 120.0
            })
            .collect()
    }
    /// Remembers the value the block makes (called when it changes; the graph thread never waits for the lock).
    pub fn set_value(&self, value: &Value) {
        if let Ok(mut v) = self.value.try_lock() {
            *v = match value {
                Value::None => None,
                other => Some(other.written()),
            };
        }
    }
    /// The value the block makes now, written out.
    pub fn value(&self) -> Option<String> {
        self.value.lock().ok().and_then(|v| v.clone())
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

/// An input of a node, by name. The numbered ones belong to the blocks that branch: `When(0)` is `when1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Port {
    In,
    Open,
    Set,
    Clear,
    Toggle,
    A,
    B,
    /// The condition of a branch (`when1`, `when2`...), counted from 0.
    When(u8),
    /// The sound of a branch of a `select` (`in1`, `in2`...), counted from 0.
    Pick(u8),
    /// The sound of a `select` when no condition is true.
    Else,
    /// The action that plays a trigger pad of a soundboard (`play1`, `play2`...), counted from 0.
    Play(u8),
    /// The action that stops a whole soundboard.
    StopAll,
    /// The input that gives a block's setting (its gain, a wait) from a flow instead of by hand.
    Param,
    /// An argument of a `join` block (`arg1`, `arg2`...), counted from 0.
    Arg(u8),
    /// The template of a `join` block, when a flow gives it.
    Template,
    /// The number a `choose` block sends while its condition is true.
    Then,
    /// The flows that give the settings of a network block: `host` (0), `port` (1), `stream` (2), `from` (3).
    Net(u8),
    /// The actions that make a counter go up, go down, go back to its first value.
    Up,
    Down,
    Reset,
}

impl Port {
    pub fn from_name(name: &str) -> Option<Port> {
        let numbered = |base: &str| {
            port_number(base, name)
                .filter(|n| *n <= MAX_BRANCHES)
                .map(|n| (n - 1) as u8)
        };
        Some(match name {
            "in" => Port::In,
            "open" => Port::Open,
            "set" => Port::Set,
            "clear" => Port::Clear,
            "toggle" => Port::Toggle,
            "a" => Port::A,
            "b" => Port::B,
            "else" => Port::Else,
            "host" => Port::Net(0),
            "port" => Port::Net(1),
            "stream" => Port::Net(2),
            "from" => Port::Net(3),
            "template" => Port::Template,
            "when" => Port::When(0),
            "then" => Port::Then,
            "up" => Port::Up,
            "down" => Port::Down,
            "reset" => Port::Reset,
            "stop" => Port::StopAll,
            "db" | "threshold_db" | "ms" | "interval_ms" => Port::Param,
            _ => {
                return numbered("when")
                    .map(Port::When)
                    .or_else(|| numbered("in").map(Port::Pick))
                    .or_else(|| numbered("play").map(Port::Play))
                    .or_else(|| numbered("arg").map(Port::Arg))
            }
        })
    }
}

/// An edge as the node at its end sees it: where it comes from (a position in the plan and which of that node's
/// outputs) and which input it enters.
#[derive(Clone, Copy, Debug)]
pub struct Wire {
    pub from: usize,
    /// Which output of the source node: 0 for a block with one output.
    pub out: usize,
    pub port: Port,
}

/// How many action outputs a node can have (a `change` block has two: it became true, it became false).
const ACTION_OUTPUTS: usize = 2;

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
    /// One sound to several outputs: `gains[k]` fades the output of condition `k`, the last one the `else`.
    Branch {
        gains: Vec<SmoothedGain>,
    },
    /// Several sounds to one output: `gains[k]` fades the sound of condition `k`, the last one the `else` sound.
    Select {
        gains: Vec<SmoothedGain>,
        scratch: Vec<f32>,
    },
    /// A soundboard: what its pads play. Shared with the plan that replaces this one, so that sounds go on playing
    /// when the graph is edited.
    Soundboard(Arc<BoardRuntime>),
    /// Noise reduction. Shared with the plan that replaces this one, so that editing the graph does not make the
    /// network start again (it would let the filter's fade-in through).
    NoiseReduction(Arc<Mutex<Denoiser>>),
    /// The larger noise reduction: the model runs on a thread of its own (see `dsp::deepfilter`); this hands it each
    /// cycle's sound and takes back what it finished. The resource that owns the thread outlives plans.
    DeepFilter(Arc<Mutex<omu_deepfilter::Endpoints>>),
    /// The sound, later: a ring of samples that is read before it is written.
    Delay {
        line: Vec<f32>,
        pos: usize,
    },
    Variable {
        state: bool,
    },
    Level {
        threshold: f32,
        hold_left: u32,
    },
    Logic(LogicOp),
    Not,
    All,
    Any,
    /// True while the input is, and `cycles` cycles after.
    Hold {
        cycles: u32,
        left: u32,
    },
    /// True once the input has been true for `cycles` cycles in a row.
    After {
        cycles: u32,
        seen: u32,
    },
    /// Sends an action when the input becomes true and another when it becomes false (`None`: nothing seen yet, so
    /// the first cycle is not a change).
    Change {
        prev: Option<bool>,
    },
    /// Actions counted by another thread (a keyboard shortcut) and handed over each cycle.
    Trigger(Arc<AtomicU32>),
    Timer {
        interval_frames: u64,
        elapsed: u64,
    },
    /// A value that never changes (a text, a number).
    Const(Value),
    /// Text made from a template and its arguments; written again only when the template or an argument changes.
    Join {
        template: String,
        args: u32,
        last: Vec<Value>,
        last_template: String,
        text: Arc<str>,
    },
    /// Two numbers combined.
    Math(MathOp),
    /// A yes/no from two numbers, from two texts.
    Compare(CompareOp),
    TextCompare(TextOp),
    /// One of two numbers, by a condition.
    Choose,
    /// A number from the text that says it (nothing when the text is not a number).
    ToNumber,
    /// A whole number from a decimal one.
    Round(RoundMode),
    /// 1 or 0 from a yes/no, a yes/no from a number.
    FromCondition,
    ToCondition,
    /// A whole number that actions change; `initial` is where `reset` goes back to.
    Counter {
        count: i64,
        initial: i64,
    },
    /// How loud the sound is, in decibels (rounded to half a decibel).
    Volume,
    /// Tests only: audio that never runs dry.
    #[cfg(test)]
    Constant(f32),
    /// Tests only: a block with a bug.
    #[cfg(test)]
    Panic,
}

impl NodeOp {
    /// How many sounds the node sends out (a buffer for each).
    fn audio_outputs(&self) -> usize {
        match self {
            NodeOp::Source(_)
            | NodeOp::SilentSource
            | NodeOp::Gain(_)
            | NodeOp::Mixer
            | NodeOp::Gate(_)
            | NodeOp::Select { .. }
            | NodeOp::NoiseReduction(_)
            | NodeOp::DeepFilter(_)
            | NodeOp::Soundboard(_)
            | NodeOp::Delay { .. } => 1,
            NodeOp::Branch { gains } => gains.len(),
            #[cfg(test)]
            NodeOp::Constant(_) => 1,
            _ => 0,
        }
    }

    /// Whether the node's yes/no is shown in the status.
    fn has_condition(&self) -> bool {
        matches!(
            self,
            NodeOp::Variable { .. }
                | NodeOp::Logic(_)
                | NodeOp::Level { .. }
                | NodeOp::Gate(_)
                | NodeOp::Not
                | NodeOp::All
                | NodeOp::Any
                | NodeOp::Hold { .. }
                | NodeOp::After { .. }
                | NodeOp::Compare(_)
                | NodeOp::TextCompare(_)
                | NodeOp::ToCondition
        )
    }
}

/// The values that flows give to the settings of a network block, and how long they have been what they are: the
/// control side opens the stream again with them once they have settled.
#[derive(Debug)]
pub struct NetFlows(Mutex<(Vec<Value>, Instant)>);

impl NetFlows {
    pub fn new() -> Arc<NetFlows> {
        Arc::new(NetFlows(Mutex::new((vec![Value::None; 4], Instant::now()))))
    }

    /// Called every cycle by the graph thread, which never waits for the lock.
    pub fn publish(&self, now: [Value; 4]) {
        if let Ok(mut g) = self.0.try_lock() {
            if g.0 != now {
                g.0 = now.to_vec();
                g.1 = Instant::now();
            }
        }
    }

    /// The values, when they have not changed for `stable`.
    pub fn settled(&self, stable: Duration) -> Option<Vec<Value>> {
        let g = self.0.lock().ok()?;
        (g.1.elapsed() >= stable).then(|| g.0.clone())
    }
}

/// The setting of a block that a flow can give (the block's `Port::Param` input).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamKind {
    GainDb,
    ThresholdDb,
    HoldMs,
    AfterMs,
    IntervalMs,
}

pub struct PlanNode {
    pub id: NodeId,
    pub op: NodeOp,
    pub inputs: Vec<Wire>,
    pub meter: Arc<Meter>,
    /// Which setting the block's `Port::Param` input gives, when it has one.
    pub param: Option<ParamKind>,
    /// The flows on the settings of a network block, shared with the control side.
    pub net: Option<Arc<NetFlows>>,
    /// A muted block sends nothing.
    pub muted: bool,
}

/// What a node produced this cycle.
#[derive(Default)]
struct Out {
    /// One buffer per sound output.
    audio: Vec<Vec<f32>>,
    cond: bool,
    /// One count per action output.
    actions: [u32; ACTION_OUTPUTS],
    /// What the block sends on its value output.
    value: Value,
}

/// The sound entering `port` of the node at `me`, or silence when nothing is linked.
fn audio_of<'a>(
    outs: &'a [Out],
    silence: &'a [f32],
    inputs: &[Wire],
    port: Port,
    me: usize,
) -> &'a [f32] {
    inputs
        .iter()
        .find(|w| w.port == port && w.from != me && w.from < outs.len())
        .and_then(|w| outs[w.from].audio.get(w.out))
        .map_or(silence, |a| &a[..])
}

/// The condition entering `port`: `None` when nothing is linked.
fn cond_of(outs: &[Out], inputs: &[Wire], port: Port) -> Option<bool> {
    inputs
        .iter()
        .find(|w| w.port == port)
        .map(|w| outs[w.from].cond)
}

/// The value entering `port`: `None` when nothing is linked. A block that makes a yes/no sends it as a value too.
fn value_of(outs: &[Out], inputs: &[Wire], port: Port) -> Value {
    inputs
        .iter()
        .find(|w| w.port == port && w.from < outs.len())
        .map_or(Value::None, |w| outs[w.from].value.clone())
}

/// Which of the conditions `Port::When(0..branches)` is the first true one (`branches` itself when none is).
fn first_true(outs: &[Out], inputs: &[Wire], branches: usize) -> usize {
    (0..branches)
        .find(|&k| cond_of(outs, inputs, Port::When(k as u8)).unwrap_or(false))
        .unwrap_or(branches)
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
            .map(|n| Out {
                audio: vec![vec![0.0; cycle_samples]; n.op.audio_outputs()],
                ..Default::default()
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

    /// The fades of a block with `branches` conditions: one per condition and one for the `else`, all closed (the
    /// first cycle opens the one that applies).
    pub fn fades(branches: u32) -> Vec<SmoothedGain> {
        (0..=branches.clamp(1, MAX_BRANCHES))
            .map(|_| Plan::gain(-120.0))
            .collect()
    }

    /// A `select` block, with the room it needs to mix one cycle.
    pub fn select(branches: u32, cycle_frames: usize) -> NodeOp {
        NodeOp::Select {
            gains: Plan::fades(branches),
            scratch: vec![0.0; cycle_frames * CH],
        }
    }

    /// A `noise_reduction` block for cycles of `cycle_frames` frames.
    pub fn noise_reduction(cycle_frames: usize) -> NodeOp {
        NodeOp::NoiseReduction(Arc::new(Mutex::new(Denoiser::new(CH, cycle_frames))))
    }

    /// A `delay` block of `ms` milliseconds.
    pub fn delay(ms: u32) -> NodeOp {
        let frames = (ms.min(MAX_DELAY_MS) as u64 * ENGINE_SAMPLE_RATE as u64 / 1000) as usize;
        NodeOp::Delay {
            line: vec![0.0; frames * CH],
            pos: 0,
        }
    }

    /// How many engine cycles last `ms` milliseconds, rounded up (zero for zero).
    pub fn cycles(ms: u32, cycle_frames: usize) -> u32 {
        let frames = ms as u64 * ENGINE_SAMPLE_RATE as u64 / 1000;
        frames
            .div_ceil(cycle_frames.max(1) as u64)
            .min(u32::MAX as u64) as u32
    }

    /// Threshold of a level detector, from decibels.
    pub fn level_threshold(db: f32) -> f32 {
        db_to_linear(db)
    }

    /// Continues the state of the nodes of `old` that exist, of the same kind, here: gains and gates do not jump, a
    /// variable keeps its value, a level detector its hold, a timer its phase, a delay what it was holding back.
    pub fn inherit(&mut self, old: &Plan) {
        for node in &mut self.nodes {
            let Some(before) = old.nodes.iter().find(|n| n.id == node.id) else {
                continue;
            };
            match (&mut node.op, &before.op) {
                (NodeOp::Gain(g), NodeOp::Gain(o)) | (NodeOp::Gate(g), NodeOp::Gate(o)) => {
                    g.start_from(o.current())
                }
                (NodeOp::Branch { gains }, NodeOp::Branch { gains: o })
                | (NodeOp::Select { gains, .. }, NodeOp::Select { gains: o, .. }) => {
                    // the first conditions keep their fade, so adding a condition changes nothing audible
                    for (g, before) in gains.iter_mut().zip(o) {
                        g.start_from(before.current());
                    }
                }
                (NodeOp::NoiseReduction(d), NodeOp::NoiseReduction(o)) => *d = o.clone(),
                (NodeOp::Delay { line, pos }, NodeOp::Delay { line: o, pos: op }) => {
                    if line.len() == o.len() {
                        line.copy_from_slice(o);
                        *pos = *op;
                    }
                }
                (NodeOp::Variable { state }, NodeOp::Variable { state: o }) => *state = *o,
                (NodeOp::Counter { count, .. }, NodeOp::Counter { count: o, .. }) => *count = *o,
                (NodeOp::Level { hold_left, .. }, NodeOp::Level { hold_left: o, .. }) => {
                    *hold_left = *o
                }
                (NodeOp::Hold { left, .. }, NodeOp::Hold { left: o, .. }) => *left = *o,
                (NodeOp::After { seen, .. }, NodeOp::After { seen: o, .. }) => *seen = *o,
                (NodeOp::Change { prev }, NodeOp::Change { prev: o }) => *prev = *o,
                (NodeOp::Timer { elapsed, .. }, NodeOp::Timer { elapsed: o, .. }) => *elapsed = *o,
                _ => {}
            }
        }
    }

    /// Puts into a block the setting that its parameter input gives (the number is limited to what the block accepts).
    fn apply_param(op: &mut NodeOp, kind: ParamKind, v: f64, cycle_frames: usize) {
        // a number that comes in by a flow is cropped to what the setting allows: it never goes past
        let crop = |name: &str| PortLimits::of(name).map_or(v, |l| l.crop(v));
        match (op, kind) {
            (NodeOp::Gain(g), ParamKind::GainDb) => g.set_db(crop("db") as f32),
            (NodeOp::Level { threshold, .. }, ParamKind::ThresholdDb) => {
                *threshold = Plan::level_threshold(crop("threshold_db") as f32)
            }
            (NodeOp::Hold { cycles, .. }, ParamKind::HoldMs)
            | (NodeOp::After { cycles, .. }, ParamKind::AfterMs) => {
                *cycles = Plan::cycles(crop("ms") as u32, cycle_frames)
            }
            (
                NodeOp::Timer {
                    interval_frames, ..
                },
                ParamKind::IntervalMs,
            ) => {
                let ms = crop("interval_ms") as u64;
                *interval_frames = (ms * ENGINE_SAMPLE_RATE as u64 / 1000).max(cycle_frames as u64);
            }
            _ => {}
        }
    }

    /// Runs one cycle.
    pub fn process(&mut self) {
        let hold_cycles =
            (LEVEL_HOLD_MS as u64 * ENGINE_SAMPLE_RATE as u64 / 1000 / self.cycle_frames as u64)
                .max(1) as u32;
        for i in 0..self.nodes.len() {
            let mut audios = std::mem::take(&mut self.outs[i].audio);
            let (mut cond, mut actions) = (false, [0u32; ACTION_OUTPUTS]);
            let mut value = Value::None;
            let node = &mut self.nodes[i];
            let (outs, silence) = (&self.outs[..], &self.silence[..]);
            if let Some(net) = &node.net {
                net.publish([0, 1, 2, 3].map(|k| value_of(outs, &node.inputs, Port::Net(k))));
            }
            // a flow linked to the setting wins over what was typed in the block
            if let Some(kind) = node.param {
                if let Some(v) = value_of(outs, &node.inputs, Port::Param).number() {
                    Plan::apply_param(&mut node.op, kind, v, self.cycle_frames);
                }
            }
            let wire = |port: Port| node.inputs.iter().find(|w| w.port == port).copied();
            let input_audio = |port: Port| audio_of(outs, silence, &node.inputs, port, i);
            match &mut node.op {
                NodeOp::Source(consumer) => match consumer.try_lock() {
                    Ok(mut c) => c.pull(&mut audios[0]),
                    Err(_) => audios[0].fill(0.0),
                },
                NodeOp::SilentSource => audios[0].fill(0.0),
                #[cfg(test)]
                NodeOp::Constant(v) => audios[0].fill(*v),
                #[cfg(test)]
                NodeOp::Panic => panic!("test: a block panicked"),
                NodeOp::Gain(g) => {
                    audios[0].copy_from_slice(input_audio(Port::In));
                    g.apply(&mut audios[0], CH);
                }
                NodeOp::Gate(g) => {
                    let open = wire(Port::Open).is_none_or(|w| outs[w.from].cond);
                    g.set_db(if open { 0.0 } else { -120.0 });
                    audios[0].copy_from_slice(input_audio(Port::In));
                    g.apply(&mut audios[0], CH);
                    cond = open;
                }
                NodeOp::Mixer => {
                    audios[0].fill(0.0);
                    for w in node.inputs.iter().filter(|w| w.port == Port::In) {
                        if let Some(a) = outs[w.from].audio.get(w.out) {
                            add_into(&mut audios[0], a);
                        }
                    }
                }
                NodeOp::Branch { gains } => {
                    let chosen = first_true(outs, &node.inputs, gains.len() - 1);
                    let input = input_audio(Port::In);
                    for (k, (g, out)) in gains.iter_mut().zip(audios.iter_mut()).enumerate() {
                        g.set_db(if k == chosen { 0.0 } else { -120.0 });
                        out.copy_from_slice(input);
                        g.apply(out, CH);
                    }
                }
                NodeOp::Select { gains, scratch } => {
                    let last = gains.len() - 1;
                    let chosen = first_true(outs, &node.inputs, last);
                    audios[0].fill(0.0);
                    for (k, g) in gains.iter_mut().enumerate() {
                        g.set_db(if k == chosen { 0.0 } else { -120.0 });
                        let port = if k == last {
                            Port::Else
                        } else {
                            Port::Pick(k as u8)
                        };
                        scratch.copy_from_slice(input_audio(port));
                        g.apply(scratch, CH);
                        add_into(&mut audios[0], scratch);
                    }
                }
                NodeOp::Soundboard(board) => {
                    board.mix(&mut audios[0]);
                    // actions from the graph: the control side plays the pads (loading a sound is not for this thread)
                    for w in node.inputs.iter() {
                        let fired = outs[w.from].actions[w.out.min(ACTION_OUTPUTS - 1)] > 0;
                        match w.port {
                            Port::Play(k) if fired => board.request_play(k as usize),
                            Port::StopAll if fired => board.request_stop(),
                            _ => {}
                        }
                    }
                }
                NodeOp::DeepFilter(filter) => {
                    let input = input_audio(Port::In);
                    node.meter.record(input);
                    node.meter.record_level(input);
                    match filter.try_lock() {
                        Ok(mut f) => f.process(input, &mut audios[0]),
                        Err(_) => audios[0].copy_from_slice(input),
                    }
                }
                NodeOp::NoiseReduction(denoiser) => {
                    let input = input_audio(Port::In);
                    node.meter.record_level(input);
                    match denoiser.try_lock() {
                        Ok(mut d) => d.process(input, &mut audios[0]),
                        // nobody else uses it while this plan runs; if it happens, the sound goes through
                        Err(_) => audios[0].copy_from_slice(input),
                    }
                }
                NodeOp::Delay { line, pos } => {
                    let input = input_audio(Port::In);
                    if line.is_empty() {
                        audios[0].copy_from_slice(input);
                    } else {
                        for (o, s) in audios[0].iter_mut().zip(input) {
                            *o = std::mem::replace(&mut line[*pos], *s);
                            *pos += 1;
                            if *pos == line.len() {
                                *pos = 0;
                            }
                        }
                    }
                }
                NodeOp::Sink(producer) => {
                    // a muted output plays silence
                    let input = if node.muted {
                        silence
                    } else {
                        input_audio(Port::In)
                    };
                    node.meter.record(input);
                    if let Ok(mut p) = producer.try_lock() {
                        p.push(input);
                    }
                }
                NodeOp::DiscardSink => node.meter.record(input_audio(Port::In)),
                NodeOp::Variable { state } => {
                    let count = |port: Port| -> u32 {
                        node.inputs
                            .iter()
                            .filter(|w| w.port == port)
                            .map(|w| outs[w.from].actions[w.out.min(ACTION_OUTPUTS - 1)])
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
                    let input = input_audio(Port::In);
                    node.meter.record(input);
                    if peak(input) > *threshold {
                        *hold_left = hold_cycles;
                    } else {
                        *hold_left = hold_left.saturating_sub(1);
                    }
                    cond = *hold_left > 0;
                }
                NodeOp::Logic(op) => {
                    let value = |port: Port| cond_of(outs, &node.inputs, port).unwrap_or(false);
                    let (a, b) = (value(Port::A), value(Port::B));
                    cond = match op {
                        LogicOp::And => a && b,
                        LogicOp::Or => a || b,
                        LogicOp::Xor => a != b,
                        LogicOp::Not => !a,
                    };
                }
                NodeOp::Not => cond = !cond_of(outs, &node.inputs, Port::In).unwrap_or(false),
                NodeOp::All => {
                    let mut linked = node.inputs.iter().filter(|w| w.port == Port::In).peekable();
                    cond = linked.peek().is_some() && linked.all(|w| outs[w.from].cond);
                }
                NodeOp::Any => {
                    cond = node
                        .inputs
                        .iter()
                        .any(|w| w.port == Port::In && outs[w.from].cond);
                }
                NodeOp::Hold { cycles, left } => {
                    let on = cond_of(outs, &node.inputs, Port::In).unwrap_or(false);
                    *left = if on { *cycles } else { left.saturating_sub(1) };
                    cond = on || *left > 0;
                }
                NodeOp::After { cycles, seen } => {
                    let on = cond_of(outs, &node.inputs, Port::In).unwrap_or(false);
                    *seen = if on { (*seen + 1).min(*cycles) } else { 0 };
                    cond = on && *seen >= *cycles;
                }
                NodeOp::Change { prev } => {
                    let on = cond_of(outs, &node.inputs, Port::In).unwrap_or(false);
                    if let Some(before) = *prev {
                        actions[0] = (on && !before) as u32;
                        actions[1] = (!on && before) as u32;
                    }
                    *prev = Some(on);
                }
                NodeOp::Const(v) => {
                    value = v.clone();
                    // a yes/no typed in a block is a condition too
                    if let Value::Bool(b) = v {
                        cond = *b;
                    }
                }
                NodeOp::Join {
                    template,
                    args,
                    last,
                    last_template,
                    text,
                } => {
                    let now: Vec<Value> = (0..*args as u8)
                        .map(|k| value_of(outs, &node.inputs, Port::Arg(k)))
                        .collect();
                    // a flow on the template wins over the one typed in the block
                    let flowed = value_of(outs, &node.inputs, Port::Template);
                    let written;
                    let wanted: &str = match &flowed {
                        Value::None => template,
                        Value::Text(t) => t,
                        other => {
                            written = other.written();
                            &written
                        }
                    };
                    // written again only when the template or an argument changed (writing makes text, which is not free)
                    if now != *last || wanted != last_template {
                        let values: Vec<String> = now.iter().map(Value::written).collect();
                        *text = omu_types::template::render(wanted, &values)
                            .unwrap_or_default()
                            .into();
                        *last_template = wanted.to_owned();
                        *last = now;
                    }
                    value = Value::Text(text.clone());
                }
                NodeOp::ToNumber => {
                    let said = value_of(outs, &node.inputs, Port::In);
                    value = match said {
                        Value::None => Value::None,
                        other => other
                            .written()
                            .trim()
                            .parse::<f64>()
                            .ok()
                            .filter(|n| n.is_finite())
                            .map_or(Value::None, Value::Float),
                    };
                }
                NodeOp::Round(mode) => {
                    value = match value_of(outs, &node.inputs, Port::In).number() {
                        None => Value::None,
                        Some(n) => Value::Int(match mode {
                            RoundMode::Nearest => n.round(),
                            RoundMode::Down => n.floor(),
                            RoundMode::Up => n.ceil(),
                        } as i64),
                    };
                }
                NodeOp::FromCondition => {
                    value = Value::Int(i64::from(
                        cond_of(outs, &node.inputs, Port::In).unwrap_or(false),
                    ));
                }
                NodeOp::ToCondition => {
                    cond = value_of(outs, &node.inputs, Port::In)
                        .number()
                        .is_some_and(|n| n != 0.0);
                }
                NodeOp::Math(op) => {
                    let num =
                        |port: Port| value_of(outs, &node.inputs, port).number().unwrap_or(0.0);
                    let (a, b) = (num(Port::A), num(Port::B));
                    value = Value::Float(match op {
                        MathOp::Add => a + b,
                        MathOp::Subtract => a - b,
                        MathOp::Multiply => a * b,
                        MathOp::Divide if b == 0.0 => 0.0,
                        MathOp::Divide => a / b,
                        MathOp::Min => a.min(b),
                        MathOp::Max => a.max(b),
                    });
                }
                NodeOp::Compare(op) => {
                    let num =
                        |port: Port| value_of(outs, &node.inputs, port).number().unwrap_or(0.0);
                    let (a, b) = (num(Port::A), num(Port::B));
                    cond = match op {
                        CompareOp::Greater => a > b,
                        CompareOp::Less => a < b,
                        CompareOp::GreaterOrEqual => a >= b,
                        CompareOp::LessOrEqual => a <= b,
                        CompareOp::Equal => a == b,
                        CompareOp::NotEqual => a != b,
                    };
                }
                NodeOp::TextCompare(op) => {
                    let text = |port: Port| value_of(outs, &node.inputs, port).written();
                    let (a, b) = (text(Port::A), text(Port::B));
                    cond = match op {
                        TextOp::Equals => a == b,
                        TextOp::Contains => a.contains(&b),
                        TextOp::StartsWith => a.starts_with(&b),
                        TextOp::EndsWith => a.ends_with(&b),
                    };
                }
                NodeOp::Choose => {
                    let on = cond_of(outs, &node.inputs, Port::When(0)).unwrap_or(false);
                    let num =
                        |port: Port| value_of(outs, &node.inputs, port).number().unwrap_or(0.0);
                    value = Value::Float(if on { num(Port::Then) } else { num(Port::Else) });
                }
                NodeOp::Counter { count, initial } => {
                    let sum = |port: Port| -> i64 {
                        node.inputs
                            .iter()
                            .filter(|w| w.port == port && w.from < outs.len())
                            .map(|w| outs[w.from].actions[w.out.min(ACTION_OUTPUTS - 1)] as i64)
                            .sum()
                    };
                    if sum(Port::Reset) > 0 {
                        *count = *initial;
                    }
                    *count = (*count + sum(Port::Up) - sum(Port::Down))
                        .clamp(-1_000_000_000, 1_000_000_000);
                    value = Value::Int(*count);
                }
                NodeOp::Volume => {
                    let input = input_audio(Port::In);
                    node.meter.record(input);
                    let db = 20.0 * f64::from(peak(input)).max(1e-6).log10();
                    value = Value::Float(((db * 2.0).round() / 2.0).max(-120.0));
                }
                NodeOp::Trigger(events) => actions[0] = events.swap(0, Ordering::AcqRel),
                NodeOp::Timer {
                    interval_frames,
                    elapsed,
                } => {
                    *elapsed += self.cycle_frames as u64;
                    if *elapsed >= *interval_frames {
                        actions[0] = (*elapsed / *interval_frames).min(u32::MAX as u64) as u32;
                        *elapsed %= *interval_frames;
                    }
                }
            }
            // a muted block sends nothing
            if node.muted {
                audios.iter_mut().for_each(|a| a.fill(0.0));
                cond = false;
                actions = [0; ACTION_OUTPUTS];
                value = Value::None;
            }
            // a block that sends sound is metered by what it sends; one that only listens was metered above
            for a in &audios {
                node.meter.record(a);
            }
            if node.op.has_condition() {
                node.meter.set_condition(cond);
                // a yes/no can also go where a value is expected (it is written out in a text)
                value = Value::Bool(cond);
            }
            node.meter.add_events(actions.iter().sum());
            let out = &mut self.outs[i];
            // the status shows the value when it changes
            let makes_value = matches!(
                node.op,
                NodeOp::Const(_)
                    | NodeOp::Join { .. }
                    | NodeOp::Math(_)
                    | NodeOp::Choose
                    | NodeOp::Counter { .. }
                    | NodeOp::Volume
                    | NodeOp::ToNumber
                    | NodeOp::Round(_)
                    | NodeOp::FromCondition
            );
            if makes_value && out.value != value {
                node.meter.set_value(&value);
            }
            out.audio = audios;
            out.cond = cond;
            out.actions = actions;
            out.value = value;
        }
    }

    /// The first audio output of node `i` after the last cycle (for tests).
    pub fn output(&self, i: usize) -> &[f32] {
        self.output_n(i, 0)
    }

    /// Audio output number `n` of node `i` after the last cycle (for tests).
    pub fn output_n(&self, i: usize, n: usize) -> &[f32] {
        &self.outs[i].audio[n]
    }

    /// The condition output of node `i` after the last cycle.
    pub fn condition(&self, i: usize) -> bool {
        self.outs[i].cond
    }

    /// The value node `i` sent in the last cycle.
    pub fn value(&self, i: usize) -> &Value {
        &self.outs[i].value
    }

    /// The actions node `i` sent in the last cycle, on its first action output.
    pub fn actions(&self, i: usize) -> u32 {
        self.outs[i].actions[0]
    }

    /// The actions node `i` sent in the last cycle, on action output number `n`.
    pub fn actions_n(&self, i: usize, n: usize) -> u32 {
        self.outs[i].actions[n]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{sink_bridge, source_bridge};
    use omu_types::AudioFormat;

    fn wire(from: usize) -> Wire {
        Wire {
            from,
            out: 0,
            port: Port::In,
        }
    }
    fn wire_to(from: usize, port: Port) -> Wire {
        Wire { from, out: 0, port }
    }
    fn node(id: &str, op: NodeOp, inputs: Vec<Wire>) -> PlanNode {
        PlanNode {
            param: None,
            net: None,
            muted: false,
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

    // ---- the blocks that program: branch, select, delay, not, all, any, hold, after, change

    fn variable(state: bool) -> NodeOp {
        NodeOp::Variable { state }
    }
    fn when(from: usize, k: u8) -> Wire {
        wire_to(from, Port::When(k))
    }
    fn set_var(plan: &mut Plan, node: usize, state: bool) {
        plan.nodes[node].op = NodeOp::Variable { state };
    }
    fn first_sample(plan: &Plan, node: usize, out: usize) -> f32 {
        plan.output_n(node, out)[0]
    }
    fn last_sample(plan: &Plan, node: usize, out: usize) -> f32 {
        *plan.output_n(node, out).last().unwrap()
    }
    fn plan_silent(plan: &Plan, node: usize, out: usize) -> bool {
        plan.output_n(node, out).iter().all(|s| s.abs() < 1e-6)
    }

    #[test]
    fn port_names_become_inputs_and_unknown_ones_do_not() {
        assert_eq!(Port::from_name("in"), Some(Port::In));
        assert_eq!(Port::from_name("when1"), Some(Port::When(0)));
        assert_eq!(Port::from_name("when3"), Some(Port::When(2)));
        assert_eq!(Port::from_name("in2"), Some(Port::Pick(1)));
        assert_eq!(Port::from_name("else"), Some(Port::Else));
        assert_eq!(Port::from_name("when0"), None);
        assert_eq!(Port::from_name("when17"), None);
        assert_eq!(Port::from_name("whenever"), None);
    }

    /// src(0.5) -> branch with two conditions, driven by two variables.
    fn branch_plan(a: bool, b: bool) -> Plan {
        Plan::new(
            vec![
                node("src", NodeOp::Constant(0.5), vec![]),
                node("a", variable(a), vec![]),
                node("b", variable(b), vec![]),
                node(
                    "br",
                    NodeOp::Branch {
                        gains: Plan::fades(2),
                    },
                    vec![wire(0), when(1, 0), when(2, 1)],
                ),
            ],
            480,
        )
    }

    #[test]
    fn a_branch_sends_the_sound_to_the_first_true_condition_only() {
        let heard = |plan: &Plan| -> Vec<bool> {
            (0..3)
                .map(|o| last_sample(plan, 3, o).abs() > 0.4)
                .collect()
        };
        // if a: out1
        let mut p = branch_plan(true, true);
        run(&mut p, 3);
        assert_eq!(heard(&p), [true, false, false], "the first true one wins");
        // else if b: out2
        let mut p = branch_plan(false, true);
        run(&mut p, 3);
        assert_eq!(heard(&p), [false, true, false]);
        // else
        let mut p = branch_plan(false, false);
        run(&mut p, 3);
        assert_eq!(heard(&p), [false, false, true]);
        assert!(plan_silent(&p, 3, 0) && plan_silent(&p, 3, 1));
    }

    #[test]
    fn a_branch_that_has_no_condition_linked_sends_everything_to_else() {
        let mut p = Plan::new(
            vec![
                node("src", NodeOp::Constant(0.5), vec![]),
                node(
                    "br",
                    NodeOp::Branch {
                        gains: Plan::fades(1),
                    },
                    vec![wire(0)],
                ),
            ],
            480,
        );
        run(&mut p, 3);
        assert!(plan_silent(&p, 1, 0));
        assert!((last_sample(&p, 1, 1) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn a_branch_switches_with_a_fade_not_a_click() {
        let mut p = branch_plan(false, false);
        run(&mut p, 3); // everything goes to else
        set_var(&mut p, 1, true);
        p.process();
        // out1 opens during this cycle: it starts near silence and ends at the full sound
        assert!(first_sample(&p, 3, 0) < 0.1, "{}", first_sample(&p, 3, 0));
        assert!((last_sample(&p, 3, 0) - 0.5).abs() < 1e-2);
        // else closes the same way
        assert!(first_sample(&p, 3, 2) > 0.4);
        assert!(last_sample(&p, 3, 2) < 1e-2);
    }

    #[test]
    fn a_select_takes_the_sound_of_the_first_true_condition() {
        let plan = |a: bool, b: bool| {
            Plan::new(
                vec![
                    node("s1", NodeOp::Constant(0.2), vec![]),
                    node("s2", NodeOp::Constant(0.6), vec![]),
                    node("s3", NodeOp::Constant(0.9), vec![]),
                    node("a", variable(a), vec![]),
                    node("b", variable(b), vec![]),
                    node(
                        "sel",
                        Plan::select(2, 480),
                        vec![
                            wire_to(0, Port::Pick(0)),
                            wire_to(1, Port::Pick(1)),
                            wire_to(2, Port::Else),
                            when(3, 0),
                            when(4, 1),
                        ],
                    ),
                ],
                480,
            )
        };
        let heard = |a: bool, b: bool| {
            let mut p = plan(a, b);
            run(&mut p, 3);
            last_sample(&p, 5, 0)
        };
        assert!((heard(true, false) - 0.2).abs() < 1e-3);
        assert!((heard(false, true) - 0.6).abs() < 1e-3);
        assert!((heard(false, false) - 0.9).abs() < 1e-3, "else");
        assert!((heard(true, true) - 0.2).abs() < 1e-3, "the first one");
    }

    #[test]
    fn a_delay_holds_the_sound_back_and_a_new_plan_keeps_what_was_in_the_line() {
        // 10 ms is exactly one cycle of 480 frames
        let delayed = |ms| {
            Plan::new(
                vec![
                    node("src", NodeOp::Constant(0.5), vec![]),
                    node("d", Plan::delay(ms), vec![wire(0)]),
                ],
                480,
            )
        };
        let mut p = delayed(10);
        p.process();
        assert!(plan_silent(&p, 1, 0), "nothing has had time to come out");
        p.process();
        assert!((last_sample(&p, 1, 0) - 0.5).abs() < 1e-6);
        // no delay at all: the sound as it is
        let mut p0 = delayed(0);
        p0.process();
        assert!((last_sample(&p0, 1, 0) - 0.5).abs() < 1e-6);
        // an edit of the graph does not empty the line
        let mut next = delayed(10);
        next.inherit(&p);
        next.process();
        assert!((first_sample(&next, 1, 0) - 0.5).abs() < 1e-6);
        // a different length starts afresh
        let mut other = delayed(20);
        other.inherit(&p);
        other.process();
        assert!(plan_silent(&other, 1, 0));
    }

    fn condition_plan(op: NodeOp, inputs: &[Option<bool>]) -> Plan {
        let mut nodes: Vec<PlanNode> = vec![];
        let mut wires = vec![];
        for (i, v) in inputs.iter().enumerate() {
            if let Some(v) = v {
                nodes.push(node(&format!("v{i}"), variable(*v), vec![]));
                wires.push(wire_to(nodes.len() - 1, Port::In));
            }
        }
        nodes.push(node("c", op, wires));
        Plan::new(nodes, 480)
    }
    fn result(op: NodeOp, inputs: &[Option<bool>]) -> bool {
        let mut p = condition_plan(op, inputs);
        run(&mut p, 2);
        p.condition(p.nodes.len() - 1)
    }

    #[test]
    fn not_inverts_and_an_unlinked_input_counts_as_false() {
        assert!(!result(NodeOp::Not, &[Some(true)]));
        assert!(result(NodeOp::Not, &[Some(false)]));
        assert!(
            result(NodeOp::Not, &[None]),
            "nothing linked reads as false"
        );
    }

    #[test]
    fn all_needs_every_link_true_and_any_needs_one() {
        let t = Some(true);
        let f = Some(false);
        assert!(result(NodeOp::All, &[t, t, t]));
        assert!(!result(NodeOp::All, &[t, f, t]));
        assert!(
            !result(NodeOp::All, &[None]),
            "no link: not true by default"
        );
        assert!(result(NodeOp::Any, &[f, t, f]));
        assert!(!result(NodeOp::Any, &[f, f]));
        assert!(!result(NodeOp::Any, &[None]));
        assert!(result(NodeOp::All, &[t]) && result(NodeOp::Any, &[t]));
    }

    #[test]
    fn hold_keeps_a_condition_true_for_a_while_after_it_stops() {
        let mut p = condition_plan(NodeOp::Hold { cycles: 3, left: 0 }, &[Some(true)]);
        run(&mut p, 2);
        assert!(p.condition(1));
        set_var(&mut p, 0, false);
        let mut seen = vec![];
        for _ in 0..5 {
            p.process();
            seen.push(p.condition(1));
        }
        assert_eq!(seen, [true, true, false, false, false][..], "{seen:?}");
        // with a hold of zero it is the input itself
        let mut z = condition_plan(NodeOp::Hold { cycles: 0, left: 0 }, &[Some(true)]);
        run(&mut z, 1);
        assert!(z.condition(1));
        set_var(&mut z, 0, false);
        run(&mut z, 1);
        assert!(!z.condition(1));
    }

    #[test]
    fn after_waits_for_the_input_to_stay_true() {
        let mut p = condition_plan(NodeOp::After { cycles: 3, seen: 0 }, &[Some(true)]);
        let mut seen = vec![];
        for _ in 0..5 {
            p.process();
            seen.push(p.condition(1));
        }
        assert_eq!(seen, [false, false, true, true, true][..], "{seen:?}");
        // a break starts the wait again
        set_var(&mut p, 0, false);
        p.process();
        assert!(!p.condition(1), "false as soon as the input is");
        set_var(&mut p, 0, true);
        p.process();
        p.process();
        assert!(!p.condition(1), "two cycles are not enough");
        p.process();
        assert!(p.condition(1));
    }

    #[test]
    fn change_sends_an_action_when_a_condition_becomes_true_and_another_when_false() {
        let mut p = condition_plan(NodeOp::Change { prev: None }, &[Some(true)]);
        let both = |p: &Plan| (p.actions_n(1, 0), p.actions_n(1, 1));
        p.process();
        assert_eq!(both(&p), (0, 0), "the first look is not a change");
        p.process();
        assert_eq!(both(&p), (0, 0));
        set_var(&mut p, 0, false);
        p.process();
        assert_eq!(both(&p), (0, 1), "fell");
        p.process();
        assert_eq!(both(&p), (0, 0), "an action acts once");
        set_var(&mut p, 0, true);
        p.process();
        assert_eq!(both(&p), (1, 0), "rose");
        assert_eq!(p.nodes[1].meter.events(), 2);
    }

    #[test]
    fn each_action_output_of_a_change_reaches_its_own_input() {
        // v1 --condition--> change --rise--> v2.set ; change --fall--> v2.clear: v2 follows v1
        let mut p = Plan::new(
            vec![
                node("v1", variable(false), vec![]),
                node("ch", NodeOp::Change { prev: None }, vec![wire(0)]),
                node(
                    "v2",
                    variable(false),
                    vec![
                        Wire {
                            from: 1,
                            out: 0,
                            port: Port::Set,
                        },
                        Wire {
                            from: 1,
                            out: 1,
                            port: Port::Clear,
                        },
                    ],
                ),
            ],
            480,
        );
        run(&mut p, 2);
        assert!(!p.condition(2));
        set_var(&mut p, 0, true);
        run(&mut p, 2);
        assert!(p.condition(2), "set by the rise");
        set_var(&mut p, 0, false);
        run(&mut p, 2);
        assert!(!p.condition(2), "cleared by the fall");
    }

    #[test]
    fn a_state_of_the_new_blocks_survives_a_plan_swap() {
        let mut old = condition_plan(NodeOp::Hold { cycles: 5, left: 0 }, &[Some(true)]);
        run(&mut old, 2);
        set_var(&mut old, 0, false);
        run(&mut old, 1); // holding: 4 cycles left
        let mut new = condition_plan(NodeOp::Hold { cycles: 5, left: 0 }, &[Some(false)]);
        new.inherit(&old);
        new.process();
        assert!(new.condition(1), "the hold went on across the swap");
    }

    #[test]
    fn noise_reduction_runs_in_a_plan_and_keeps_its_state_when_the_plan_is_replaced() {
        let nr = || {
            Plan::new(
                vec![
                    node("src", NodeOp::Constant(0.2), vec![]),
                    node("nr", Plan::noise_reduction(480), vec![wire(0)]),
                ],
                480,
            )
        };
        let mut old = nr();
        run(&mut old, 5);
        assert!(plan_silent(&old, 1, 0) || old.output(1).iter().all(|s| s.is_finite()));
        // the first cycle is silent (the fade-in of the network is not let through)
        let mut fresh = nr();
        fresh.process();
        assert!(plan_silent(&fresh, 1, 0));
        // a new plan takes over the same network, so editing the graph does not start it again
        let mut next = nr();
        next.inherit(&old);
        let (NodeOp::NoiseReduction(a), NodeOp::NoiseReduction(b)) =
            (&next.nodes[1].op, &old.nodes[1].op)
        else {
            panic!("not noise reduction blocks");
        };
        assert!(Arc::ptr_eq(a, b));
    }

    fn text(v: &str) -> NodeOp {
        NodeOp::Const(Value::Text(v.into()))
    }

    #[test]
    fn a_join_writes_text_numbers_and_yes_no_together_and_follows_its_arguments() {
        let join = NodeOp::Join {
            template: "$arg1 has $arg2 items".into(),
            args: 2,
            last: vec![],
            last_template: String::new(),
            text: "".into(),
        };
        let mut plan = Plan::new(
            vec![
                node("name", text("Bob"), vec![]),
                node("count", NodeOp::Const(Value::Int(3)), vec![]),
                node(
                    "j",
                    join,
                    vec![wire_to(0, Port::Arg(0)), wire_to(1, Port::Arg(1))],
                ),
            ],
            480,
        );
        plan.process();
        assert_eq!(plan.value(2), &Value::Text("Bob has 3 items".into()));
        // a join with nothing linked writes its template with empty arguments
        let empty = NodeOp::Join {
            template: "[$arg1]".into(),
            args: 1,
            last: vec![],
            last_template: String::new(),
            text: "".into(),
        };
        let mut plan = Plan::new(vec![node("j", empty, vec![])], 480);
        plan.process();
        assert_eq!(plan.value(0), &Value::Text("[]".into()));
    }

    #[test]
    fn math_combines_two_numbers_and_a_division_by_zero_gives_zero() {
        let result = |op: MathOp, a: Value, b: Value| {
            let mut plan = Plan::new(
                vec![
                    node("a", NodeOp::Const(a), vec![]),
                    node("b", NodeOp::Const(b), vec![]),
                    node(
                        "m",
                        NodeOp::Math(op),
                        vec![wire_to(0, Port::A), wire_to(1, Port::B)],
                    ),
                ],
                480,
            );
            plan.process();
            plan.value(2).clone()
        };
        assert_eq!(
            result(MathOp::Add, Value::Int(2), Value::Float(0.5)),
            Value::Float(2.5)
        );
        assert_eq!(
            result(MathOp::Subtract, Value::Int(2), Value::Int(5)),
            Value::Float(-3.0)
        );
        assert_eq!(
            result(MathOp::Multiply, Value::Float(1.5), Value::Int(4)),
            Value::Float(6.0)
        );
        assert_eq!(
            result(MathOp::Divide, Value::Int(9), Value::Int(2)),
            Value::Float(4.5)
        );
        assert_eq!(
            result(MathOp::Divide, Value::Int(9), Value::Int(0)),
            Value::Float(0.0)
        );
        assert_eq!(
            result(MathOp::Min, Value::Int(9), Value::Int(2)),
            Value::Float(2.0)
        );
        assert_eq!(
            result(MathOp::Max, Value::Int(9), Value::Int(2)),
            Value::Float(9.0)
        );
    }

    #[test]
    fn a_flow_on_a_setting_wins_over_what_was_typed_in_the_block() {
        let build = |linked: bool| {
            let mut gain = node(
                "g",
                NodeOp::Gain(Plan::gain(0.0)),
                if linked {
                    vec![wire(0), wire_to(1, Port::Param)]
                } else {
                    vec![wire(0)]
                },
            );
            gain.param = Some(ParamKind::GainDb);
            Plan::new(
                vec![
                    node("s", NodeOp::Constant(1.0), vec![]),
                    node("n", NodeOp::Const(Value::Float(-6.0206)), vec![]),
                    gain,
                ],
                480,
            )
        };
        let mut linked = build(true);
        run(&mut linked, 30);
        assert!(
            (last_sample(&linked, 2, 0) - 0.5).abs() < 0.01,
            "the linked number sets the gain"
        );
        let mut typed = build(false);
        run(&mut typed, 30);
        assert!(
            (last_sample(&typed, 2, 0) - 1.0).abs() < 0.01,
            "without a flow the typed value is used"
        );
    }

    #[test]
    fn a_whole_number_sets_a_wait_and_a_timer_follows_its_flow() {
        // a hold of 0 ms, lengthened to 100 ms by a flow: it stays true for 10 cycles after the input stops
        let mut hold = node(
            "h",
            NodeOp::Hold { cycles: 0, left: 0 },
            vec![wire(0), wire_to(1, Port::Param)],
        );
        hold.param = Some(ParamKind::HoldMs);
        let mut plan = Plan::new(
            vec![
                node("v", variable(true), vec![]),
                node("ms", NodeOp::Const(Value::Int(100)), vec![]),
                hold,
            ],
            480,
        );
        plan.process();
        assert!(plan.condition(2));
        set_var(&mut plan, 0, false);
        run(&mut plan, 5);
        assert!(plan.condition(2), "still true 50 ms after");
        run(&mut plan, 10);
        assert!(!plan.condition(2), "false after 100 ms");
    }

    #[test]
    fn a_yes_no_can_be_written_into_a_text() {
        let join = NodeOp::Join {
            template: "mic: $arg1".into(),
            args: 1,
            last: vec![],
            last_template: String::new(),
            text: "".into(),
        };
        let mut plan = Plan::new(
            vec![
                node("v", variable(true), vec![]),
                node("j", join, vec![wire_to(0, Port::Arg(0))]),
            ],
            480,
        );
        plan.process();
        assert_eq!(plan.value(1), &Value::Text("mic: true".into()));
    }

    fn compared(op: CompareOp, a: Value, b: Value) -> bool {
        let mut plan = Plan::new(
            vec![
                node("a", NodeOp::Const(a), vec![]),
                node("b", NodeOp::Const(b), vec![]),
                node(
                    "c",
                    NodeOp::Compare(op),
                    vec![wire_to(0, Port::A), wire_to(1, Port::B)],
                ),
            ],
            480,
        );
        plan.process();
        plan.condition(2)
    }

    #[test]
    fn a_comparison_of_two_numbers_is_a_condition() {
        use CompareOp::*;
        assert!(compared(Greater, Value::Int(3), Value::Float(2.5)));
        assert!(!compared(Greater, Value::Int(2), Value::Int(2)));
        assert!(compared(GreaterOrEqual, Value::Int(2), Value::Int(2)));
        assert!(compared(Less, Value::Float(-1.0), Value::Int(0)));
        assert!(compared(LessOrEqual, Value::Int(2), Value::Int(2)));
        assert!(compared(Equal, Value::Int(2), Value::Float(2.0)));
        assert!(compared(NotEqual, Value::Int(2), Value::Int(3)));
        // a number that is not linked counts as 0
        let mut plan = Plan::new(
            vec![
                node("a", NodeOp::Const(Value::Int(-2)), vec![]),
                node("c", NodeOp::Compare(Less), vec![wire_to(0, Port::A)]),
            ],
            480,
        );
        plan.process();
        assert!(plan.condition(1));
    }

    #[test]
    fn a_text_comparison_is_a_condition_and_the_case_counts() {
        let said = |op: TextOp, a: &str, b: &str| {
            let mut plan = Plan::new(
                vec![
                    node("a", text(a), vec![]),
                    node("b", text(b), vec![]),
                    node(
                        "c",
                        NodeOp::TextCompare(op),
                        vec![wire_to(0, Port::A), wire_to(1, Port::B)],
                    ),
                ],
                480,
            );
            plan.process();
            plan.condition(2)
        };
        assert!(said(TextOp::Equals, "Bob", "Bob"));
        assert!(!said(TextOp::Equals, "Bob", "bob"));
        assert!(said(TextOp::Contains, "hello Bob!", "Bob"));
        assert!(said(TextOp::StartsWith, "hello", "he"));
        assert!(!said(TextOp::StartsWith, "hello", "lo"));
        assert!(said(TextOp::EndsWith, "hello", "lo"));
    }

    #[test]
    fn choose_sends_one_of_two_numbers_by_a_condition() {
        let chosen = |on: bool| {
            let mut plan = Plan::new(
                vec![
                    node("v", variable(on), vec![]),
                    node("t", NodeOp::Const(Value::Int(10)), vec![]),
                    node("e", NodeOp::Const(Value::Float(0.5)), vec![]),
                    node(
                        "c",
                        NodeOp::Choose,
                        vec![
                            wire_to(0, Port::When(0)),
                            wire_to(1, Port::Then),
                            wire_to(2, Port::Else),
                        ],
                    ),
                ],
                480,
            );
            plan.process();
            plan.value(3).clone()
        };
        assert_eq!(chosen(true), Value::Float(10.0));
        assert_eq!(chosen(false), Value::Float(0.5));
    }

    #[test]
    fn a_counter_goes_up_and_down_and_back_to_where_it_started_and_keeps_its_count_in_a_new_plan() {
        let (up, down, reset) = (
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicU32::new(0)),
        );
        let build = || {
            Plan::new(
                vec![
                    node("up", NodeOp::Trigger(up.clone()), vec![]),
                    node("down", NodeOp::Trigger(down.clone()), vec![]),
                    node("reset", NodeOp::Trigger(reset.clone()), vec![]),
                    node(
                        "n",
                        NodeOp::Counter {
                            count: 5,
                            initial: 5,
                        },
                        vec![
                            wire_to(0, Port::Up),
                            wire_to(1, Port::Down),
                            wire_to(2, Port::Reset),
                        ],
                    ),
                ],
                480,
            )
        };
        let mut plan = build();
        plan.process();
        assert_eq!(plan.value(3), &Value::Int(5));
        up.store(3, Ordering::Release);
        down.store(1, Ordering::Release);
        plan.process();
        assert_eq!(plan.value(3), &Value::Int(7));
        plan.process();
        assert_eq!(plan.value(3), &Value::Int(7), "an action counts once");
        let mut next = build();
        next.inherit(&plan);
        next.process();
        assert_eq!(
            next.value(3),
            &Value::Int(7),
            "a new plan goes on from the count"
        );
        reset.store(1, Ordering::Release);
        next.process();
        assert_eq!(next.value(3), &Value::Int(5));
    }

    #[test]
    fn a_volume_block_tells_how_loud_the_sound_is() {
        let level = |v: f32| {
            let mut plan = Plan::new(
                vec![
                    node("s", NodeOp::Constant(v), vec![]),
                    node("v", NodeOp::Volume, vec![wire(0)]),
                ],
                480,
            );
            plan.process();
            plan.value(1).clone()
        };
        assert_eq!(level(1.0), Value::Float(0.0));
        assert_eq!(level(0.5), Value::Float(-6.0));
        assert_eq!(level(0.1), Value::Float(-20.0));
        assert_eq!(level(0.0), Value::Float(-120.0));
    }

    #[test]
    fn a_meter_remembers_the_levels_of_the_last_ten_seconds_oldest_first() {
        let meter = Meter::default();
        assert!(meter.levels().is_empty());
        meter.record_level(&[0.5, -0.25]);
        meter.record_level(&[0.0]);
        let first = meter.levels();
        assert_eq!(first.len(), 2);
        assert!(
            (first[0] + 6.0).abs() < 0.5,
            "half of full scale is about -6 dB"
        );
        assert_eq!(first[1], -120.0, "silence is -120");
        // past the ten seconds, the oldest are forgotten
        for _ in 0..LEVEL_HISTORY {
            meter.record_level(&[1.0]);
        }
        let all = meter.levels();
        assert_eq!(all.len(), LEVEL_HISTORY);
        assert!(all.iter().all(|db| *db == 0.0));
    }

    #[test]
    fn the_flows_on_a_network_blocks_settings_are_published_and_settle() {
        let flows = NetFlows::new();
        let mut plan = Plan::new(
            vec![
                node("p", NodeOp::Const(Value::Int(7000)), vec![]),
                PlanNode {
                    net: Some(flows.clone()),
                    ..node("n", NodeOp::SilentSource, vec![wire_to(0, Port::Net(1))])
                },
            ],
            480,
        );
        plan.process();
        // just changed: not settled yet
        assert_eq!(flows.settled(Duration::from_secs(60)), None);
        // settled once it has stayed the same for the asked time
        let values = flows.settled(Duration::ZERO).unwrap();
        assert_eq!(values[1], Value::Int(7000));
        assert_eq!(values[0], Value::None);
        assert_eq!(Port::from_name("port"), Some(Port::Net(1)));
        assert_eq!(Port::from_name("host"), Some(Port::Net(0)));
    }

    #[test]
    fn conversions_turn_one_flow_into_another() {
        let through = |op: NodeOp, input: NodeOp| {
            let mut plan = Plan::new(
                vec![
                    node("i", input, vec![]),
                    node("c", op, vec![wire_to(0, Port::In)]),
                ],
                480,
            );
            plan.process();
            (plan.value(1).clone(), plan.condition(1))
        };
        // a text into a number: spaces do not matter, what is not a number gives nothing
        assert_eq!(
            through(NodeOp::ToNumber, text(" 12.5 ")).0,
            Value::Float(12.5)
        );
        assert_eq!(through(NodeOp::ToNumber, text("twelve")).0, Value::None);
        // a decimal number into a whole one, three ways
        let r = |mode, n: f64| through(NodeOp::Round(mode), NodeOp::Const(Value::Float(n))).0;
        assert_eq!(r(RoundMode::Nearest, 2.5), Value::Int(3));
        assert_eq!(r(RoundMode::Down, 2.9), Value::Int(2));
        assert_eq!(r(RoundMode::Up, 2.1), Value::Int(3));
        assert_eq!(r(RoundMode::Down, -2.1), Value::Int(-3));
        // a yes/no into 1 or 0, a number into a yes/no
        assert_eq!(
            through(NodeOp::FromCondition, variable(true)).0,
            Value::Int(1)
        );
        assert_eq!(
            through(NodeOp::FromCondition, variable(false)).0,
            Value::Int(0)
        );
        assert!(through(NodeOp::ToCondition, NodeOp::Const(Value::Float(0.1))).1);
        assert!(!through(NodeOp::ToCondition, NodeOp::Const(Value::Int(0))).1);
    }

    #[test]
    fn a_flow_on_the_template_of_a_join_changes_the_text() {
        let join = NodeOp::Join {
            template: "typed $arg1".into(),
            args: 1,
            last: vec![],
            last_template: String::new(),
            text: "".into(),
        };
        let build = |template: Option<&str>| {
            let mut nodes = vec![
                node("a", text("x"), vec![]),
                node("t", text(template.unwrap_or("")), vec![]),
            ];
            let mut wires = vec![wire_to(0, Port::Arg(0))];
            if template.is_some() {
                wires.push(wire_to(1, Port::Template));
            }
            nodes.push(node(
                "j",
                NodeOp::Join {
                    template: "typed $arg1".into(),
                    args: 1,
                    last: vec![],
                    last_template: String::new(),
                    text: "".into(),
                },
                wires,
            ));
            let mut plan = Plan::new(nodes, 480);
            plan.process();
            plan.value(2).clone()
        };
        drop(join);
        assert_eq!(build(None), Value::Text("typed x".into()));
        assert_eq!(build(Some("flowed [$1]")), Value::Text("flowed [x]".into()));
        // a template that cannot be read gives an empty text, never a crash
        assert_eq!(build(Some("broken ${arg1")), Value::Text("".into()));
    }

    #[test]
    fn a_number_flowing_into_a_setting_is_cropped_to_its_limits() {
        let gain = |db: f64| {
            let mut g = node(
                "g",
                NodeOp::Gain(Plan::gain(0.0)),
                vec![wire(0), wire_to(1, Port::Param)],
            );
            g.param = Some(ParamKind::GainDb);
            let mut plan = Plan::new(
                vec![
                    node("s", NodeOp::Constant(1.0), vec![]),
                    node("n", NodeOp::Const(Value::Float(db)), vec![]),
                    g,
                ],
                480,
            );
            // a gain of +40 dB is a hundred times louder: the smooth gain takes a second to get there
            run(&mut plan, 150);
            last_sample(&plan, 2, 0)
        };
        // +40 dB is the most: a flow asking for +1000 dB gets +40 dB, not more
        let most = db_to_linear(40.0);
        assert!((gain(1000.0) - most).abs() < most * 0.01);
        // and -120 dB the least: silence, not a negative gain
        assert!(gain(-1000.0).abs() < 0.001);
    }

    #[test]
    fn a_muted_block_sends_no_sound_no_condition_and_no_value() {
        let mut gain = node("g", NodeOp::Gain(Plan::gain(0.0)), vec![wire(0)]);
        gain.muted = true;
        let mut var = node("v", variable(true), vec![]);
        var.muted = true;
        let mut value = node("n", NodeOp::Const(Value::Int(3)), vec![]);
        value.muted = true;
        let mut plan = Plan::new(
            vec![node("s", NodeOp::Constant(1.0), vec![]), gain, var, value],
            480,
        );
        run(&mut plan, 20);
        assert!(
            plan_silent(&plan, 1, 0),
            "the sound of a muted block is silence"
        );
        assert!(!plan.condition(2), "a muted variable is false");
        assert_eq!(plan.value(3), &Value::None);
        // what comes after it hears silence, and the source itself is untouched
        assert!(!plan_silent(&plan, 0, 0));
    }

    #[test]
    fn a_muted_output_plays_silence() {
        let (producer, _consumer, _) = sink_bridge(AudioFormat::ENGINE, 100.0);
        let mut sink = node(
            "k",
            NodeOp::Sink(Arc::new(Mutex::new(producer))),
            vec![wire(0)],
        );
        sink.muted = true;
        let mut plan = Plan::new(vec![node("s", NodeOp::Constant(1.0), vec![]), sink], 480);
        run(&mut plan, 5);
        assert_eq!(plan.nodes[1].meter.take(), 0.0, "the output heard nothing");
    }
}
