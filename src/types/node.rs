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

/// The kinds of flows of the graph. An input takes the flows of its own type, and a few others that convert without
/// loss of meaning ([`PortType::accepts`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortType {
    /// Interleaved `f32` audio in the engine format.
    Audio,
    /// A yes/no that follows something, evaluated every cycle.
    Condition,
    /// A discrete event: something that happens once.
    Action,
    /// A piece of text.
    Text,
    /// A whole number.
    Int,
    /// A number with a decimal part.
    Float,
}

impl PortType {
    /// Whether an input of this type takes a flow of type `from`: the same type; a whole number where a decimal one is
    /// wanted; and text takes numbers and yes/no (they are written out).
    pub fn accepts(self, from: PortType) -> bool {
        use PortType::*;
        self == from
            || matches!(
                (self, from),
                (Float, Int) | (Text, Int) | (Text, Float) | (Text, Condition)
            )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortDirection {
    Input,
    Output,
}

/// A port declared by a node kind. Most kinds have fixed ports; the blocks that branch (`branch`, `select`) have as
/// many as their `branches` setting says, so a port's name is owned text (`when2`, `out3`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortSpec {
    pub name: String,
    pub direction: PortDirection,
    pub ty: PortType,
    /// An input that accepts several incoming edges. Sounds are added together (every sound input does); a block that
    /// combines conditions (`all`, `any`) combines them; a variable collects the actions.
    pub multi: bool,
}

/// The longest name of a network stream (what VBAN allows).
pub const MAX_STREAM_NAME: usize = 16;
/// The longest name of a web input or output.
pub const MAX_WEB_NAME: usize = 32;
/// The longest address of a live stream.
pub const MAX_STREAM_URL: usize = 2048;

/// The protocol of a stream address, in lower case (`rtmp` for `rtmp://host/app`).
pub fn stream_scheme(url: &str) -> Option<String> {
    let (scheme, rest) = url.trim().split_once("://")?;
    (!scheme.is_empty()
        && !rest.is_empty()
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.'))
    .then(|| scheme.to_ascii_lowercase())
}

/// What is wrong with the address of a live stream, if anything: only protocols that carry sound over a network are accepted
/// (so a stream block can never be made to read a file of this computer).
pub fn stream_problem(url: &str, output: bool) -> Option<String> {
    const INPUTS: [&str; 9] = [
        "rtmp", "rtmps", "rtsp", "rtsps", "srt", "rtp", "udp", "http", "https",
    ];
    const OUTPUTS: [&str; 7] = ["rtmp", "rtmps", "rtsp", "srt", "rtp", "udp", "icecast"];
    let allowed: &[&str] = if output { &OUTPUTS } else { &INPUTS };
    if url.chars().count() > MAX_STREAM_URL {
        return Some(format!(
            "the address is at most {MAX_STREAM_URL} characters"
        ));
    }
    if url.chars().any(char::is_control) {
        return Some("the address holds a character that cannot be in an address".into());
    }
    match stream_scheme(url) {
        Some(s) if allowed.contains(&s.as_str()) => None,
        Some(s) => Some(format!(
            "{s}:// cannot be used here: use {}",
            allowed
                .iter()
                .map(|a| format!("{a}://"))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        None => Some(format!(
            "the address starts with its protocol, like {}://host/name",
            allowed[0]
        )),
    }
}

/// A name that fits in an address, as a web input or output needs: letters, digits, `-` and `_`.
pub fn is_web_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_WEB_NAME
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}
/// The most conditions a `branch` or `select` block can have.
pub const MAX_BRANCHES: u32 = 16;
/// The most arguments a `join` block can have.
pub const MAX_JOIN_ARGS: u32 = 16;
/// The longest text a block can hold.
pub const MAX_TEXT: usize = 4096;
/// The longest delay of a `delay` block, in milliseconds.
pub const MAX_DELAY_MS: u32 = 5_000;
/// The longest time a `hold` or `after` block can wait, in milliseconds.
pub const MAX_WAIT_MS: u32 = 60_000;

fn no_limit() -> f32 {
    100.0
}

fn one() -> u32 {
    1
}

/// The ports of an expression block: the expression itself (so that a flow can give it), its inputs `a`, `b`, `c`... (numbers, or
/// texts for the test of texts) and its result.
fn expression_ports(args: u32, input: PortType, result: PortSpec) -> Vec<PortSpec> {
    let inputs = crate::expression::MAX_EXPRESSION_ARGS;
    [port("expression", Input, TextFlow, false)]
        .into_iter()
        .chain((0..args.clamp(1, inputs) as usize).map(|k| {
            port(
                crate::expression::variable_name(k).to_string(),
                Input,
                input,
                false,
            )
        }))
        .chain([result])
        .collect()
}

fn one_step() -> i64 {
    1
}

fn slider_max() -> f64 {
    100.0
}

fn two() -> u32 {
    2
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn nearest() -> RoundMode {
    RoundMode::Nearest
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
    /// Makes and combines numbers (whole or decimal).
    Number,
    /// Makes and combines text.
    Text,
    /// Changes one kind of flow into another (a sound into a number, a text into a number, a number into a yes/no...).
    Conversion,
    /// Several blocks made into one.
    Group,
}

/// A connection point of a block inside a group that the group shows as its own: `name` is what the group calls it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exposed {
    pub name: String,
    pub node: NodeId,
    pub port: String,
}

/// How a [`NodeKind::Round`] block makes a whole number from a decimal one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoundMode {
    /// To the nearest whole number.
    Nearest,
    Down,
    Up,
}

/// A number, a yes/no or a text typed in a block for an input that nothing is linked to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InputValue {
    Bool(bool),
    Number(f64),
    Text(String),
}

/// How far a number input can go: `min` and `max` are never passed (a number that comes in by a flow is cropped to
/// them), `soft_min` and `soft_max` are the range a slider offers.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PortLimits {
    pub min: f64,
    pub max: f64,
    pub soft_min: f64,
    pub soft_max: f64,
}

impl PortLimits {
    /// `value` brought back into `min..=max` (a number that is not a number becomes `min`).
    pub fn crop(&self, value: f64) -> f64 {
        if value.is_nan() {
            self.min
        } else {
            value.clamp(self.min, self.max)
        }
    }

    /// The limits of the setting `name`, for the settings of the blocks that have some.
    pub fn of(name: &str) -> Option<PortLimits> {
        let l = |min, max, soft_min, soft_max| PortLimits {
            min,
            max,
            soft_min,
            soft_max,
        };
        Some(match name {
            "db" => l(-120.0, 40.0, -60.0, 24.0),
            "threshold_db" => l(-120.0, 0.0, -80.0, 0.0),
            "ms" => l(0.0, 60_000.0, 0.0, 5_000.0),
            "interval_ms" => l(10.0, 3_600_000.0, 10.0, 5_000.0),
            "port" => l(1.0, 65_535.0, 1.0, 65_535.0),
            _ => return None,
        })
    }
}

/// How a [`NodeKind::Compare`] block compares its two numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareOp {
    Greater,
    Less,
    GreaterOrEqual,
    LessOrEqual,
    Equal,
    NotEqual,
}

/// How a [`NodeKind::TextCompare`] block compares its two texts (the case counts).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextOp {
    Equals,
    Contains,
    StartsWith,
    EndsWith,
}

/// What a [`NodeKind::Math`] block does with its two numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MathOp {
    Add,
    Subtract,
    Multiply,
    /// A division by zero gives 0.
    Divide,
    Min,
    Max,
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

/// What a node does. Its ports follow from its kind and settings ([`NodeKind::ports`]).
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
    /// Input: a virtual speaker created and owned by this product. Programs choose it as their output device; what
    /// they play into it is the sound of this block (useful for programs that cannot be captured one by one, or to
    /// gather several of them on one device).
    VirtualSpeaker { name: String },
    /// Input: sound sent by another computer or program, in VBAN (UDP). Listens on `port`; `stream` is the stream name it
    /// accepts (empty: any) and `from` the address of the one computer it accepts sound from (empty: any).
    NetworkIn {
        port: u16,
        #[serde(default)]
        stream: String,
        #[serde(default)]
        from: String,
    },
    /// Input: sound sent through the web (WebRTC): a browser (a phone's microphone, the page `/remote`) or a program
    /// (OBS, FFmpeg...) connects with WHIP to the address of `name` and sends. The name is open only while the graph runs.
    WebIn { name: String },
    /// Input: a live stream read through FFmpeg (needs the FFmpeg module): RTMP, RTSP, SRT, RTP, UDP, or HTTP (Icecast, HLS...).
    StreamIn { url: String },
    /// Input: a soundboard: the sound of its pads, as many as are playing at once. The soundboards themselves (their
    /// pads and sounds) are saved by the daemon, not in the graph: this block only names one.
    ///
    /// `triggers` are pads the graph can play: each one is an action input (`play1`, `play2`...) that plays that pad
    /// every time an action arrives, and `stop` stops the whole soundboard.
    Soundboard {
        board: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        triggers: Vec<String>,
    },
    // ---- outputs
    /// Output: a playback device (speakers, headphones, or the input side of a third-party virtual cable).
    DeviceRender { device: DeviceRef },
    /// Output: a virtual microphone created and owned by this product; other applications record from it.
    VirtualMic { name: String },
    /// Output: sends the sound to another computer or program, in VBAN (UDP), to `host`:`port` under the stream name
    /// `stream`.
    NetworkOut {
        host: String,
        port: u16,
        #[serde(default)]
        stream: String,
    },
    /// Output: sound for whoever listens through the web (WebRTC): a browser or a program connects with WHEP to the address
    /// of `name` and hears the graph.
    WebOut { name: String },
    /// Output: the sound sent as a live stream through FFmpeg (needs the FFmpeg module): RTMP, RTSP, SRT, RTP, UDP or Icecast.
    StreamOut { url: String },
    // ---- processing
    /// Processing: gain in decibels.
    Gain { db: f32 },
    /// Processing: sum of all its incoming audio edges.
    Mixer,
    /// Processing: lets the audio through while its `open` condition is true (silence otherwise, with a short fade
    /// so that nothing clicks). Without a condition connected it is always open.
    Gate,
    /// Processing: "if / else if / else" for sound. One sound goes in; it comes out of the output of the first
    /// condition that is true (`out1` for `when1`, `out2` for `when2`...), or out of `else` when none is. The other
    /// outputs are silent. `branches` is how many conditions there are (the `if` and the `else if`s).
    Branch {
        #[serde(default = "one")]
        branches: u32,
    },
    /// Processing: the other way round: each condition has its own sound (`in1` for `when1`...), and the output is
    /// the sound of the first condition that is true, or the `else` sound when none is.
    Select {
        #[serde(default = "one")]
        branches: u32,
    },
    /// Processing: the sound, `ms` milliseconds later (to line a sound up with another, e.g. a camera).
    Delay { ms: u32 },
    /// Processing: takes the background noise out of a voice (a fan, a keyboard, hiss) with RNNoise, a small neural
    /// network that tells voice from noise. Adds 10 ms of delay.
    NoiseReduction,
    /// Processing: the larger noise reduction (DeepFilterNet). It needs the DeepFilterNet module; `limit_db` is the most
    /// the noise is reduced by (100: no limit).
    DeepFilter {
        #[serde(default = "no_limit")]
        limit_db: f32,
    },
    // ---- conditions
    /// Condition: a remembered yes/no. Actions change it: `set` makes it true, `clear` false, `toggle` flips it.
    Variable { initial: bool },
    /// Condition: true while the audio at its input is louder than a threshold (a simple voice detector); it stays
    /// true for a short time after the sound stops so that speech is not chopped.
    Level { threshold_db: f32 },
    /// Condition: combines conditions (and, or, xor, not). Kept for graphs saved before the blocks below existed.
    Logic { op: LogicOp },
    /// Condition: the opposite of its input.
    Not,
    /// Condition: true while every condition linked to its input is true (false when none is linked).
    All,
    /// Condition: true while at least one condition linked to its input is true.
    Any,
    /// Condition: true while its input is true, and for `ms` milliseconds after it stops being true.
    Hold { ms: u32 },
    /// Condition: true once its input has been true for `ms` milliseconds without a break (false again as soon as the
    /// input is).
    After { ms: u32 },
    // ---- triggers
    /// Trigger: sends an action each time the keyboard shortcut is pressed (works while another program has focus).
    Hotkey { keys: String },
    /// Trigger: sends an action every `interval_ms` milliseconds.
    Timer { interval_ms: u32 },
    /// Trigger: watches a condition and sends an action when it becomes true (`rise`) and another when it becomes
    /// false (`fall`).
    Change,
    // ---- values
    /// Value: a piece of text.
    Text { value: String },
    /// Value: a whole number.
    Integer { value: i64 },
    /// Value: a number with a decimal part.
    Number { value: f64 },
    /// Value: text made from a template, where `$1`, `$2`... (or `${1}`) stand for the arguments `arg1`, `arg2`...
    /// An argument can be text, a number or a yes/no.
    Join {
        template: String,
        #[serde(default = "two")]
        args: u32,
    },
    /// Value: a mathematical expression of its inputs (`a`, `b`, `c`... as many as `args`): `a + b`, `c * log(a)`. See
    /// [`crate::expression`] for what can be written.
    MathExpr {
        expression: String,
        #[serde(default = "two")]
        args: u32,
    },
    /// Condition: true while a mathematical expression of its inputs is not 0: `a = b`, `a > 3 and b < 10` (a comparison is 1 when it is
    /// true and 0 when it is not).
    CondExpr {
        expression: String,
        #[serde(default = "two")]
        args: u32,
    },
    /// Condition: a test of its text inputs (`a`, `b`, `c`... as many as `args`): `a == b`, `a.contains(b)`, `a.matches("^[0-9]+$")`. See
    /// [`crate::text_expression`] for what can be written.
    TextExpr {
        expression: String,
        #[serde(default = "two")]
        args: u32,
    },
    /// Value: two numbers combined (`a` and `b`).
    Math { op: MathOp },
    /// Condition: true while `a` compares to `b` the way `op` says (a number not linked counts as 0).
    Compare { op: CompareOp },
    /// Condition: true while the text `a` compares to the text `b` the way `op` says.
    TextCompare { op: TextOp },
    /// Value: the number at `then` while the condition `when` is true, the number at `else` otherwise.
    Choose,
    /// Value: a whole number that actions change: `up` adds `up_by`, `down` takes `down_by` off, `reset` goes back to `initial`.
    Counter {
        #[serde(default)]
        initial: i64,
        #[serde(default = "one_step")]
        up_by: i64,
        #[serde(default = "one_step")]
        down_by: i64,
    },
    /// Action: a button of the panel; the action goes out each time it is pressed there. Nothing else is linked to it.
    Button,
    /// Condition: a switch of the panel; the condition is what it is set to there (`on` is where it is now).
    Switch {
        #[serde(default)]
        on: bool,
    },
    /// Value: a slider of the panel, between `min` and `max`; the number is where it is set there (`value`).
    Slider {
        #[serde(default)]
        min: f64,
        #[serde(default = "slider_max")]
        max: f64,
        #[serde(default)]
        value: f64,
    },
    /// Conversion: how loud the sound at its input is, in decibels (0 is full scale; silence is -120).
    Volume,
    /// Conversion: the number written in a text (nothing when it is not a number).
    ToNumber,
    /// Conversion: a whole number from a decimal one.
    Round {
        #[serde(default = "nearest")]
        mode: RoundMode,
    },
    /// Conversion: 1 while a condition is true, 0 while it is not.
    FromCondition,
    /// Conversion: true while a number is not 0.
    ToCondition,
    // ---- groups
    /// Group: several blocks made into one, with the connection points of its inner blocks that `exposed` names. The engine never
    /// sees a group: the graph is expanded into its blocks before it is checked and run (see `omu_graph::expand`).
    Group {
        graph: Box<GraphSpec>,
        #[serde(default)]
        exposed: Vec<Exposed>,
        /// Where the inner blocks were drawn, by identifier, for an editor that takes the group apart again. Not read by the daemon.
        #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
        layout: std::collections::BTreeMap<String, [f64; 2]>,
    },
}

use PortDirection::{Input, Output};
use PortType::Text as TextFlow;
use PortType::{Action, Audio, Condition, Float, Int};

fn port(name: impl Into<String>, direction: PortDirection, ty: PortType, multi: bool) -> PortSpec {
    PortSpec {
        name: name.into(),
        direction,
        ty,
        multi,
    }
}

/// The ports of a block that has `branches` conditions: the numbered names are `{base}1`, `{base}2`...
fn numbered(
    base: &str,
    branches: u32,
    direction: PortDirection,
    ty: PortType,
) -> impl Iterator<Item = PortSpec> + '_ {
    // a sound input takes any number of links (they are added together), and so does an input of actions
    let multi = direction == Input && matches!(ty, Audio | Action);
    (1..=branches.clamp(1, MAX_BRANCHES))
        .map(move |n| port(format!("{base}{n}"), direction, ty, multi))
}

/// The number the name of a numbered port ends with, when it is `base` followed by a number: `("when", "when3")`
/// is `Some(3)`. Numbers start at 1.
pub fn port_number(base: &str, name: &str) -> Option<u32> {
    let n: u32 = name.strip_prefix(base)?.parse().ok()?;
    (n >= 1 && name == format!("{base}{n}")).then_some(n)
}

impl NodeKind {
    /// The ports of this kind of node: the inputs first, then the outputs, in the order a block shows them.
    pub fn ports(&self) -> Vec<PortSpec> {
        // a sound input takes any number of links: the engine adds the sounds together
        let in_audio = || port("in", Input, Audio, true);
        let out_audio = || port("out", Output, Audio, false);
        let in_cond = |multi| port("in", Input, Condition, multi);
        let out_cond = || port("out", Output, Condition, false);
        match self {
            NodeKind::AppCapture { .. }
            | NodeKind::DeviceCapture { .. }
            | NodeKind::VirtualSpeaker { .. } => vec![out_audio()],
            NodeKind::NetworkIn { .. } => vec![
                port("port", Input, Int, false),
                port("stream", Input, TextFlow, false),
                port("from", Input, TextFlow, false),
                out_audio(),
            ],
            NodeKind::WebIn { .. } => vec![port("name", Input, TextFlow, false), out_audio()],
            NodeKind::StreamIn { .. } => vec![port("url", Input, TextFlow, false), out_audio()],
            NodeKind::Soundboard { triggers, .. } => {
                let count = (triggers.len() as u32).min(MAX_BRANCHES);
                let plays = numbered("play", count, Input, Action);
                if triggers.is_empty() {
                    vec![out_audio()]
                } else {
                    [out_audio()]
                        .into_iter()
                        .chain(plays)
                        .chain([port("stop", Input, Action, true)])
                        .collect()
                }
            }
            NodeKind::DeviceRender { .. } | NodeKind::VirtualMic { .. } => vec![in_audio()],
            NodeKind::NetworkOut { .. } => vec![
                in_audio(),
                port("host", Input, TextFlow, false),
                port("port", Input, Int, false),
                port("stream", Input, TextFlow, false),
            ],
            NodeKind::WebOut { .. } => vec![in_audio(), port("name", Input, TextFlow, false)],
            NodeKind::StreamOut { .. } => vec![in_audio(), port("url", Input, TextFlow, false)],
            NodeKind::Gain { .. } => vec![in_audio(), port("db", Input, Float, false), out_audio()],
            NodeKind::Delay { .. } | NodeKind::NoiseReduction | NodeKind::DeepFilter { .. } => {
                vec![in_audio(), out_audio()]
            }
            NodeKind::Mixer => vec![in_audio(), out_audio()],
            NodeKind::Gate => vec![
                in_audio(),
                port("open", Input, Condition, false),
                out_audio(),
            ],
            NodeKind::Branch { branches } => [in_audio()]
                .into_iter()
                .chain(numbered("when", *branches, Input, Condition))
                .chain(numbered("out", *branches, Output, Audio))
                .chain([port("else", Output, Audio, false)])
                .collect(),
            NodeKind::Select { branches } => numbered("in", *branches, Input, Audio)
                .chain(numbered("when", *branches, Input, Condition))
                .chain([port("else", Input, Audio, true), out_audio()])
                .collect(),
            NodeKind::Variable { .. } => vec![
                port("set", Input, Action, true),
                port("clear", Input, Action, true),
                port("toggle", Input, Action, true),
                port("value", Output, Condition, false),
            ],
            NodeKind::Level { .. } => vec![
                in_audio(),
                port("threshold_db", Input, Float, false),
                port("active", Output, Condition, false),
            ],
            NodeKind::Logic { .. } => vec![
                port("a", Input, Condition, false),
                port("b", Input, Condition, false),
                out_cond(),
            ],
            NodeKind::Not => vec![in_cond(false), out_cond()],
            NodeKind::Hold { .. } | NodeKind::After { .. } => {
                vec![in_cond(false), port("ms", Input, Int, false), out_cond()]
            }
            NodeKind::All | NodeKind::Any => vec![in_cond(true), out_cond()],
            NodeKind::Hotkey { .. } | NodeKind::Button => {
                vec![port("pressed", Output, Action, false)]
            }
            NodeKind::Switch { .. } => vec![out_cond()],
            NodeKind::Slider { .. } => vec![port("out", Output, Float, false)],
            NodeKind::Timer { .. } => vec![
                port("interval_ms", Input, Int, false),
                port("tick", Output, Action, false),
            ],
            NodeKind::Change => vec![
                in_cond(false),
                port("rise", Output, Action, false),
                port("fall", Output, Action, false),
            ],
            NodeKind::Text { .. } => vec![port("out", Output, TextFlow, false)],
            NodeKind::Integer { .. } => vec![port("out", Output, Int, false)],
            NodeKind::Number { .. } => vec![port("out", Output, Float, false)],
            NodeKind::Join { args, .. } => [port("template", Input, TextFlow, false)]
                .into_iter()
                .chain(
                    (1..=(*args).clamp(1, MAX_JOIN_ARGS))
                        .map(|n| port(format!("arg{n}"), Input, TextFlow, false)),
                )
                .chain([port("out", Output, TextFlow, false)])
                .collect(),
            NodeKind::MathExpr { args, .. } => {
                expression_ports(*args, Float, port("out", Output, Float, false))
            }
            NodeKind::CondExpr { args, .. } => expression_ports(*args, Float, out_cond()),
            NodeKind::TextExpr { args, .. } => expression_ports(*args, TextFlow, out_cond()),
            NodeKind::Math { .. } => vec![
                port("a", Input, Float, false),
                port("b", Input, Float, false),
                port("out", Output, Float, false),
            ],
            NodeKind::Compare { .. } => vec![
                port("a", Input, Float, false),
                port("b", Input, Float, false),
                out_cond(),
            ],
            NodeKind::TextCompare { .. } => vec![
                port("a", Input, TextFlow, false),
                port("b", Input, TextFlow, false),
                out_cond(),
            ],
            NodeKind::Choose => vec![
                port("when", Input, Condition, false),
                port("then", Input, Float, false),
                port("else", Input, Float, false),
                port("out", Output, Float, false),
            ],
            NodeKind::Counter { .. } => vec![
                port("up", Input, Action, true),
                port("down", Input, Action, true),
                port("reset", Input, Action, true),
                port("out", Output, Int, false),
            ],
            NodeKind::Volume => vec![in_audio(), port("out", Output, Float, false)],
            NodeKind::ToNumber => vec![
                port("in", Input, TextFlow, false),
                port("out", Output, Float, false),
            ],
            NodeKind::Round { .. } => vec![
                port("in", Input, Float, false),
                port("out", Output, Int, false),
            ],
            NodeKind::FromCondition => vec![in_cond(false), port("out", Output, Int, false)],
            NodeKind::ToCondition => vec![port("in", Input, Float, false), out_cond()],
            // the group shows the connection points of its inner blocks that it exposes, with their direction and type
            NodeKind::Group { graph, exposed, .. } => exposed
                .iter()
                .filter_map(|e| {
                    let inner = graph
                        .nodes
                        .iter()
                        .find(|n| n.id == e.node)?
                        .kind
                        .port(&e.port)?;
                    Some(PortSpec {
                        name: e.name.clone(),
                        ..inner
                    })
                })
                .collect(),
        }
    }

    /// The inputs that give a setting of this block from a flow (each is named after its setting).
    pub fn param_ports(&self) -> &'static [&'static str] {
        match self {
            NodeKind::Join { .. } => &["template"],
            NodeKind::MathExpr { .. } | NodeKind::CondExpr { .. } | NodeKind::TextExpr { .. } => {
                &["expression"]
            }
            NodeKind::Gain { .. } => &["db"],
            NodeKind::Level { .. } => &["threshold_db"],
            NodeKind::Hold { .. } | NodeKind::After { .. } => &["ms"],
            NodeKind::Timer { .. } => &["interval_ms"],
            NodeKind::NetworkIn { .. } => &["port", "stream", "from"],
            NodeKind::NetworkOut { .. } => &["host", "port", "stream"],
            NodeKind::WebIn { .. } | NodeKind::WebOut { .. } => &["name"],
            NodeKind::StreamIn { .. } | NodeKind::StreamOut { .. } => &["url"],
            _ => &[],
        }
    }

    pub fn port(&self, name: &str) -> Option<PortSpec> {
        self.ports().into_iter().find(|p| p.name == name)
    }

    /// Where the output port `name` comes after the other outputs (0 for the first output). A block keeps one
    /// result per output, and the engine finds the one an edge starts at by this number.
    pub fn output_index(&self, name: &str) -> Option<usize> {
        self.ports()
            .into_iter()
            .filter(|p| p.direction == Output)
            .position(|p| p.name == name)
    }

    /// The family of this block.
    pub fn category(&self) -> Category {
        match self {
            NodeKind::AppCapture { .. }
            | NodeKind::DeviceCapture { .. }
            | NodeKind::NetworkIn { .. }
            | NodeKind::WebIn { .. }
            | NodeKind::StreamIn { .. }
            | NodeKind::VirtualSpeaker { .. }
            | NodeKind::Soundboard { .. } => Category::Input,
            NodeKind::DeviceRender { .. }
            | NodeKind::VirtualMic { .. }
            | NodeKind::NetworkOut { .. }
            | NodeKind::WebOut { .. }
            | NodeKind::StreamOut { .. } => Category::Output,
            NodeKind::Gain { .. }
            | NodeKind::Mixer
            | NodeKind::Gate
            | NodeKind::Branch { .. }
            | NodeKind::Select { .. }
            | NodeKind::Delay { .. }
            | NodeKind::NoiseReduction
            | NodeKind::DeepFilter { .. } => Category::Processing,
            NodeKind::Variable { .. }
            | NodeKind::Level { .. }
            | NodeKind::Logic { .. }
            | NodeKind::Not
            | NodeKind::All
            | NodeKind::Any
            | NodeKind::Hold { .. }
            | NodeKind::After { .. }
            | NodeKind::Compare { .. }
            | NodeKind::CondExpr { .. }
            | NodeKind::TextExpr { .. }
            | NodeKind::Switch { .. }
            | NodeKind::TextCompare { .. }
            | NodeKind::ToCondition => Category::Condition,
            NodeKind::Hotkey { .. }
            | NodeKind::Timer { .. }
            | NodeKind::Change
            | NodeKind::Button => Category::Trigger,
            NodeKind::Integer { .. }
            | NodeKind::Number { .. }
            | NodeKind::Math { .. }
            | NodeKind::MathExpr { .. }
            | NodeKind::Choose
            | NodeKind::Slider { .. }
            | NodeKind::Counter { .. } => Category::Number,
            NodeKind::Text { .. } | NodeKind::Join { .. } => Category::Text,
            NodeKind::Volume
            | NodeKind::ToNumber
            | NodeKind::Round { .. }
            | NodeKind::FromCondition => Category::Conversion,
            NodeKind::Group { .. } => Category::Group,
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
        (matches!(self.category(), Category::Input | Category::Output)
            && !matches!(self, NodeKind::Soundboard { .. }))
            || matches!(self, NodeKind::Hotkey { .. } | NodeKind::DeepFilter { .. })
    }

    /// The types of the flows this node sends, one per output port (empty for a block that only receives).
    pub fn output_types(&self) -> Vec<PortType> {
        self.ports()
            .iter()
            .filter(|p| p.direction == Output)
            .map(|p| p.ty)
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeSpec {
    /// What the graph's edges call the block: any text that is not used by another block (an editor makes a random
    /// one, a hand-written graph can use `"mic"`). It never changes when the block is renamed.
    pub id: NodeId,
    /// A name for people, as free as they want. Nothing in the daemon reads it: it is kept with the graph so that
    /// every editor shows the same names. Missing means no name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    /// What is typed in the block for its inputs that nothing is linked to (by port name): a number, a text. A flow linked
    /// to the input wins over it. The settings that belong to the block's kind (a gain, a wait) are kept in the kind.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub inputs: std::collections::BTreeMap<String, InputValue>,
    /// A muted block sends nothing (silence, no condition, no action): to listen to the rest of the graph without it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub muted: bool,
    #[serde(flatten)]
    pub kind: NodeKind,
}

impl NodeSpec {
    /// A block with no display name.
    pub fn new(id: &str, kind: NodeKind) -> Self {
        NodeSpec {
            id: NodeId::from(id),
            display: None,
            inputs: Default::default(),
            muted: false,
            kind,
        }
    }
}

/// The whole editable graph.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphSpec {
    #[serde(default)]
    pub nodes: Vec<NodeSpec>,
    #[serde(default)]
    pub edges: Vec<EdgeSpec>,
}

impl GraphSpec {
    /// Every block that does something, the ones inside groups (at any depth) included, the groups themselves left out: what the engine
    /// really runs. Whoever asks "is there a soundboard (a virtual microphone...) in this graph" must look here, not at `nodes`, or
    /// what is inside a group is not seen.
    pub fn leaf_nodes(&self) -> Vec<&NodeSpec> {
        let mut out = vec![];
        for node in &self.nodes {
            match &node.kind {
                NodeKind::Group { graph, .. } => out.extend(graph.leaf_nodes()),
                _ => out.push(node),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_blocks_inside_groups_are_found_at_any_depth() {
        let node = |id: &str, kind: NodeKind| NodeSpec {
            id: id.into(),
            display: None,
            inputs: Default::default(),
            muted: false,
            kind,
        };
        let inner = GraphSpec {
            nodes: vec![node("mic", NodeKind::VirtualMic { name: "M".into() })],
            edges: vec![],
        };
        let group = |id: &str, graph: GraphSpec| {
            node(
                id,
                NodeKind::Group {
                    graph: Box::new(graph),
                    exposed: vec![],
                    layout: Default::default(),
                },
            )
        };
        let outer = GraphSpec {
            nodes: vec![
                node(
                    "board",
                    NodeKind::Soundboard {
                        board: "b".into(),
                        triggers: vec![],
                    },
                ),
                group(
                    "g1",
                    GraphSpec {
                        nodes: vec![group("g2", inner)],
                        edges: vec![],
                    },
                ),
            ],
            edges: vec![],
        };
        let ids: Vec<&str> = outer.leaf_nodes().iter().map(|n| n.id.0.as_str()).collect();
        assert_eq!(
            ids,
            ["board", "mic"],
            "the groups are not blocks, what is in them is"
        );
    }

    #[test]
    fn graph_round_trips_through_json() {
        let spec = GraphSpec {
            nodes: vec![
                NodeSpec {
                    id: "app".into(),
                    display: None,
                    inputs: Default::default(),
                    muted: false,
                    kind: NodeKind::AppCapture {
                        target: AppTarget::ProcessName("chrome.exe".into()),
                    },
                },
                NodeSpec {
                    id: "vol".into(),
                    display: None,
                    inputs: Default::default(),
                    muted: false,
                    kind: NodeKind::Gain { db: -6.0 },
                },
                NodeSpec {
                    id: "mic".into(),
                    display: None,
                    inputs: Default::default(),
                    muted: false,
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
            display: None,
            inputs: Default::default(),
            muted: false,
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
    fn every_sound_input_takes_several_links_and_a_condition_input_only_when_it_combines() {
        for k in [
            NodeKind::Mixer,
            NodeKind::Gain { db: 0.0 },
            NodeKind::VirtualMic { name: "x".into() },
            NodeKind::Gate,
            NodeKind::Delay { ms: 1 },
            NodeKind::Branch { branches: 2 },
            NodeKind::Select { branches: 2 },
            NodeKind::Level {
                threshold_db: -40.0,
            },
        ] {
            for p in k.ports() {
                if p.direction == PortDirection::Input && p.ty == PortType::Audio {
                    assert!(p.multi, "{k:?}: {} adds the sounds", p.name);
                }
            }
        }
        // a condition input takes one link, except the ones that combine them
        let combining = |k: &NodeKind| {
            k.ports()
                .iter()
                .filter(|p| p.ty == PortType::Condition && p.multi)
                .count()
        };
        assert_eq!(combining(&NodeKind::All), 1);
        assert_eq!(combining(&NodeKind::Any), 1);
        for k in [
            NodeKind::Not,
            NodeKind::Gate,
            NodeKind::Hold { ms: 1 },
            NodeKind::After { ms: 1 },
            NodeKind::Change,
            NodeKind::Branch { branches: 3 },
            NodeKind::Logic { op: LogicOp::And },
        ] {
            assert_eq!(
                combining(&k),
                0,
                "{k:?}: a condition decides, it is not combined"
            );
        }
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
            NodeKind::DeepFilter { limit_db: 100.0 },
            NodeKind::NetworkIn {
                port: 6980,
                stream: "s".into(),
                from: String::new(),
            },
            NodeKind::NetworkOut {
                host: "192.168.1.20".into(),
                port: 6980,
                stream: "s".into(),
            },
            NodeKind::WebIn {
                name: "phone".into(),
            },
            NodeKind::WebOut {
                name: "listen".into(),
            },
            NodeKind::StreamIn {
                url: "rtmp://localhost/live/in".into(),
            },
            NodeKind::StreamOut {
                url: "rtmp://localhost/live/out".into(),
            },
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
            NodeKind::Branch { branches: 3 },
            NodeKind::Select { branches: 2 },
            NodeKind::Delay { ms: 40 },
            NodeKind::NoiseReduction,
            NodeKind::Not,
            NodeKind::All,
            NodeKind::Any,
            NodeKind::Hold { ms: 500 },
            NodeKind::After { ms: 200 },
            NodeKind::Change,
            NodeKind::Text { value: "hi".into() },
            NodeKind::Integer { value: 3 },
            NodeKind::Number { value: 1.5 },
            NodeKind::Join {
                template: "$1-$2".into(),
                args: 2,
            },
            NodeKind::MathExpr {
                expression: "a + b".into(),
                args: 2,
            },
            NodeKind::CondExpr {
                expression: "a = b".into(),
                args: 2,
            },
            NodeKind::TextExpr {
                expression: "a = b".into(),
                args: 2,
            },
            NodeKind::Math { op: MathOp::Add },
            NodeKind::Compare {
                op: CompareOp::Greater,
            },
            NodeKind::TextCompare {
                op: TextOp::Contains,
            },
            NodeKind::Choose,
            NodeKind::Counter {
                initial: 0,
                up_by: 1,
                down_by: 1,
            },
            NodeKind::Button,
            NodeKind::Switch { on: false },
            NodeKind::Slider {
                min: 0.0,
                max: 100.0,
                value: 0.0,
            },
            NodeKind::Volume,
            NodeKind::ToNumber,
            NodeKind::Round {
                mode: RoundMode::Nearest,
            },
            NodeKind::FromCondition,
            NodeKind::ToCondition,
            NodeKind::Soundboard {
                board: "b".into(),
                triggers: vec!["p".into()],
            },
        ]
    }

    #[test]
    fn every_kind_survives_json_and_belongs_to_a_family() {
        for k in every_kind() {
            let spec = NodeSpec {
                id: "x".into(),
                display: None,
                inputs: Default::default(),
                muted: false,
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
            let outputs = k.output_types();
            let inputs: Vec<PortType> = k
                .ports()
                .iter()
                .filter(|p| {
                    p.direction == PortDirection::Input
                        && !k.param_ports().contains(&p.name.as_str())
                })
                .map(|p| p.ty)
                .collect();
            match k.category() {
                Category::Input => {
                    assert!(!outputs.is_empty() && outputs.iter().all(|t| *t == PortType::Audio));
                    // a source receives no sound; the soundboard alone takes actions (to play and stop its pads)
                    assert!(
                        inputs.iter().all(|t| *t == PortType::Action),
                        "a source receives no sound or condition"
                    );
                }
                Category::Output => assert!(outputs.is_empty()),
                Category::Trigger => {
                    assert!(!outputs.is_empty() && outputs.iter().all(|t| *t == PortType::Action));
                    assert!(
                        inputs.iter().all(|t| *t == PortType::Condition),
                        "a trigger starts from nothing or from a condition that changes"
                    );
                }
                Category::Condition => {
                    assert!(
                        !outputs.is_empty() && outputs.iter().all(|t| *t == PortType::Condition)
                    )
                }
                Category::Processing => {
                    assert!(!outputs.is_empty() && outputs.iter().all(|t| *t == PortType::Audio))
                }
                Category::Number => assert!(
                    !outputs.is_empty()
                        && outputs
                            .iter()
                            .all(|t| matches!(t, PortType::Int | PortType::Float))
                ),
                Category::Text => {
                    assert!(!outputs.is_empty() && outputs.iter().all(|t| *t == PortType::Text))
                }
                Category::Group => {}
                Category::Conversion => assert!(
                    !outputs.is_empty()
                        && outputs
                            .iter()
                            .all(|t| !matches!(t, PortType::Audio | PortType::Action))
                ),
            }
        }
    }

    #[test]
    fn a_branch_has_one_when_and_one_output_per_condition_plus_an_else() {
        let names = |k: &NodeKind, d: PortDirection| -> Vec<String> {
            k.ports()
                .into_iter()
                .filter(|p| p.direction == d)
                .map(|p| p.name)
                .collect()
        };
        let one = NodeKind::Branch { branches: 1 };
        assert_eq!(names(&one, Input), ["in", "when1"]);
        assert_eq!(names(&one, Output), ["out1", "else"]);
        let three = NodeKind::Branch { branches: 3 };
        assert_eq!(names(&three, Input), ["in", "when1", "when2", "when3"]);
        assert_eq!(names(&three, Output), ["out1", "out2", "out3", "else"]);
        assert_eq!(three.output_index("out1"), Some(0));
        assert_eq!(three.output_index("else"), Some(3));
        assert_eq!(
            three.output_index("in"),
            None,
            "an input has no output number"
        );
        // a select is the other way round: a sound per condition, one output
        let select = NodeKind::Select { branches: 2 };
        assert_eq!(
            names(&select, Input),
            ["in1", "in2", "when1", "when2", "else"]
        );
        assert_eq!(names(&select, Output), ["out"]);
        assert!(select.port("when2").is_some() && select.port("when3").is_none());
    }

    #[test]
    fn the_number_of_branches_is_always_sensible() {
        // none or far too many cannot give a block with no ports or a thousand of them
        assert_eq!(NodeKind::Branch { branches: 0 }.ports().len(), 4);
        assert_eq!(
            NodeKind::Select { branches: 9999 }.ports().len(),
            2 * MAX_BRANCHES as usize + 2
        );
        // the number is optional in saved graphs: one condition
        let k: NodeKind = serde_json::from_str(r#"{"type":"branch"}"#).unwrap();
        assert_eq!(k, NodeKind::Branch { branches: 1 });
    }

    #[test]
    fn numbered_ports_are_read_strictly() {
        assert_eq!(port_number("when", "when1"), Some(1));
        assert_eq!(port_number("when", "when12"), Some(12));
        assert_eq!(port_number("when", "when0"), None);
        assert_eq!(port_number("when", "when01"), None, "no leading zero");
        assert_eq!(port_number("when", "when"), None);
        assert_eq!(port_number("when", "whenx"), None);
        assert_eq!(port_number("when", "in1"), None);
        assert_eq!(port_number("in", "in2"), Some(2));
    }

    #[test]
    fn a_change_block_turns_a_condition_into_two_actions() {
        let c = NodeKind::Change;
        assert_eq!(c.port("in").unwrap().ty, PortType::Condition);
        assert_eq!(c.port("rise").unwrap().ty, PortType::Action);
        assert_eq!(c.port("fall").unwrap().ty, PortType::Action);
        assert_eq!(c.output_index("fall"), Some(1));
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
        // the first six kinds (devices, microphones, network) and the keyboard shortcut need the system; no other does
        let expected: Vec<bool> = every_kind()
            .iter()
            .map(|k| {
                matches!(
                    k,
                    NodeKind::AppCapture { .. }
                        | NodeKind::DeviceCapture { .. }
                        | NodeKind::DeviceRender { .. }
                        | NodeKind::VirtualMic { .. }
                        | NodeKind::NetworkIn { .. }
                        | NodeKind::NetworkOut { .. }
                        | NodeKind::WebIn { .. }
                        | NodeKind::WebOut { .. }
                        | NodeKind::StreamIn { .. }
                        | NodeKind::StreamOut { .. }
                        | NodeKind::Hotkey { .. }
                        | NodeKind::DeepFilter { .. }
                )
            })
            .collect();
        assert_eq!(needs, expected);
        assert_eq!(needs.iter().filter(|n| **n).count(), 12);
    }

    #[test]
    fn the_display_name_is_optional_free_text_next_to_the_id() {
        // a graph written by hand has none, and means none
        let old: NodeSpec =
            serde_json::from_str(r#"{"id":"mic","type":"virtual_mic","name":"Stream"}"#).unwrap();
        assert_eq!(old.display, None);
        // an editor adds one, with any text, and the id stays what it was
        let named: NodeSpec = serde_json::from_str(
            r#"{"id":"3f2c0a52-9d4e-4c8e-8f3e-0a7d5a2b9c11","display":"Micro du stream (principal) é","type":"gain","db":-3.0}"#,
        )
        .unwrap();
        assert_eq!(
            named.display.as_deref(),
            Some("Micro du stream (principal) é")
        );
        assert_eq!(named.kind, NodeKind::Gain { db: -3.0 });
        let back: NodeSpec = serde_json::from_value(serde_json::to_value(&named).unwrap()).unwrap();
        assert_eq!(back, named);
        // without one, nothing is written (the files of people who never use it do not change)
        let json = serde_json::to_value(&old).unwrap();
        assert!(json.get("display").is_none(), "{json}");
        // it never gets mixed up with the settings of a block, whatever they are called
        let mic = NodeSpec {
            id: "m".into(),
            display: Some("Display".into()),
            inputs: Default::default(),
            muted: false,
            kind: NodeKind::VirtualMic {
                name: "Device".into(),
            },
        };
        let v = serde_json::to_value(&mic).unwrap();
        assert_eq!(v["display"], "Display");
        assert_eq!(v["name"], "Device");
    }
}

#[cfg(test)]
mod inline_tests {
    use super::*;

    #[test]
    fn a_number_is_cropped_to_the_limits_of_its_setting() {
        let gain = PortLimits::of("db").unwrap();
        assert_eq!(gain.crop(1000.0), 40.0);
        assert_eq!(gain.crop(-1000.0), -120.0);
        assert_eq!(gain.crop(3.0), 3.0);
        assert_eq!(gain.crop(f64::NAN), -120.0);
        assert!(
            gain.soft_min >= gain.min && gain.soft_max <= gain.max,
            "the slider stays inside the limits"
        );
        assert_eq!(PortLimits::of("port").unwrap().max, 65_535.0);
        assert_eq!(PortLimits::of("template"), None);
        // every setting that a flow can give and that is a number has limits
        for kind in [
            NodeKind::Gain { db: 0.0 },
            NodeKind::Level {
                threshold_db: -40.0,
            },
            NodeKind::Hold { ms: 1 },
            NodeKind::After { ms: 1 },
            NodeKind::Timer { interval_ms: 20 },
        ] {
            for name in kind.param_ports() {
                assert!(PortLimits::of(name).is_some(), "{name}");
            }
        }
    }

    #[test]
    fn values_typed_in_a_block_are_a_flat_map_that_keeps_its_types() {
        let json = r#"{"id":"m","type":"math","op":"add","inputs":{"a":2,"b":0.5}}"#;
        let spec: NodeSpec = serde_json::from_str(json).unwrap();
        assert_eq!(spec.inputs["a"], InputValue::Number(2.0));
        let spec: NodeSpec = serde_json::from_str(r#"{"id":"t","type":"join","template":"$arg1","args":1,"inputs":{"arg1":"hi","x":true}}"#).unwrap();
        assert_eq!(spec.inputs["arg1"], InputValue::Text("hi".into()));
        assert_eq!(spec.inputs["x"], InputValue::Bool(true));
        // nothing typed: nothing written
        let plain = NodeSpec::new("g", NodeKind::Mixer);
        assert!(!serde_json::to_string(&plain).unwrap().contains("inputs"));
        assert_eq!(
            serde_json::from_str::<NodeSpec>(&serde_json::to_string(&spec).unwrap()).unwrap(),
            spec
        );
    }

    #[test]
    fn the_new_families_hold_what_their_names_say() {
        assert_eq!(
            NodeKind::Math { op: MathOp::Add }.category(),
            Category::Number
        );
        assert_eq!(
            NodeKind::Join {
                template: String::new(),
                args: 1
            }
            .category(),
            Category::Text
        );
        assert_eq!(NodeKind::Volume.category(), Category::Conversion);
        assert_eq!(NodeKind::ToNumber.category(), Category::Conversion);
        assert_eq!(NodeKind::ToCondition.category(), Category::Condition);
        // a join has the template as its first input, then one per argument
        let names: Vec<String> = NodeKind::Join {
            template: String::new(),
            args: 2,
        }
        .ports()
        .into_iter()
        .map(|p| p.name)
        .collect();
        assert_eq!(names, ["template", "arg1", "arg2", "out"]);
    }
}

#[cfg(test)]
mod muted_tests {
    use super::*;

    #[test]
    fn muted_is_written_only_when_it_is_true() {
        let mut spec = NodeSpec::new("g", NodeKind::Mixer);
        assert!(!serde_json::to_string(&spec).unwrap().contains("muted"));
        spec.muted = true;
        let json = serde_json::to_string(&spec).unwrap();
        assert!(json.contains("\"muted\":true"), "{json}");
        assert_eq!(serde_json::from_str::<NodeSpec>(&json).unwrap(), spec);
        assert!(
            !serde_json::from_str::<NodeSpec>(r#"{"id":"m","type":"mixer"}"#)
                .unwrap()
                .muted,
            "a graph written before has nothing muted"
        );
    }
}
