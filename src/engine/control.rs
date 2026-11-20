//! The control side of the engine: turns a validated graph into running streams and an executable plan, keeps
//! them alive (retrying failed devices with a growing delay), and reports status.
//!
//! Applying a new graph reuses every stream whose node did not change, so editing a gain or adding a branch
//! does not interrupt the audio of the other nodes.

use super::bridge::{sink_bridge, source_bridge, BridgeStats, SinkProducer, SourceConsumer};
use super::plan::{Meter, NodeOp, ParamKind, Plan, PlanNode, Port, Value, Wire};
use super::runtime::{GraphThread, RuntimeStats, ThreadInit};
use omu_graph::ValidGraph;
use omu_hal::{Backend, CaptureStream, RenderStream, Running, StreamHealth, VirtualDeviceProvider};
use omu_types::{
    EngineStatus, KeyCombo, NodeId, NodeKind, NodeState, NodeStatus, PortType, ENGINE_SAMPLE_RATE,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct EngineConfig {
    /// Frames per graph cycle (480 = 10 ms at 48 kHz).
    pub cycle_frames: usize,
    /// Audio kept buffered in each device bridge (latency vs. robustness).
    pub target_ms: f32,
    /// Runs on the graph thread when it starts (raise its priority).
    pub thread_init: Option<ThreadInit>,
    /// First delay before reopening a failed device; doubles up to `retry_max`.
    pub retry_initial: Duration,
    pub retry_max: Duration,
    /// The soundboards' memory and sounds; without it a soundboard block is silent.
    pub sounds: Option<Arc<omu_soundboard::SoundEngine>>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            cycle_frames: 480,
            target_ms: 25.0,
            thread_init: None,
            retry_initial: Duration::from_secs(1),
            retry_max: Duration::from_secs(10),
            sounds: None,
        }
    }
}

enum Io {
    Source(Arc<Mutex<SourceConsumer>>),
    Sink(Arc<Mutex<SinkProducer>>),
    /// Actions counted by the system's keyboard shortcut callback.
    Trigger(Arc<AtomicU32>),
    /// The larger noise reduction, running on its own thread.
    Filter(Arc<Mutex<omu_deepfilter::Endpoints>>),
}

struct Resource {
    kind: NodeKind,
    io: Io,
    health: Arc<StreamHealth>,
    stats: Arc<BridgeStats>,
    /// Dropping it stops the stream.
    _running: Box<dyn Running>,
}

struct Failure {
    message: String,
    retry_at: Instant,
    backoff: Duration,
}

#[derive(Default)]
struct State {
    desired: Option<ValidGraph>,
    resources: HashMap<NodeId, Resource>,
    failures: HashMap<NodeId, Failure>,
    graph_thread: Option<GraphThread>,
    /// Panics of the graph thread already answered with a new plan.
    panics_handled: u64,
    meters: HashMap<NodeId, Arc<Meter>>,
}

struct Inner {
    backend: Arc<dyn Backend>,
    virtual_devices: Option<Arc<dyn VirtualDeviceProvider>>,
    cfg: EngineConfig,
    state: Mutex<State>,
}

pub struct Engine {
    inner: Arc<Inner>,
    stop: Arc<AtomicBool>,
    supervisor: Option<JoinHandle<()>>,
}

impl Engine {
    pub fn new(
        backend: Arc<dyn Backend>,
        virtual_devices: Option<Arc<dyn VirtualDeviceProvider>>,
        cfg: EngineConfig,
    ) -> Engine {
        let inner = Arc::new(Inner {
            backend,
            virtual_devices,
            cfg,
            state: Mutex::default(),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let (weak, stop2) = (Arc::downgrade(&inner), stop.clone());
        let supervisor = std::thread::Builder::new()
            .name("engine-supervisor".into())
            .spawn(move || {
                // Late cycles are reported at most every few seconds: they come in bursts.
                let (mut late_seen, mut late_logged_at) = (0u64, Instant::now());
                while !stop2.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(250));
                    let Some(inner) = weak.upgrade() else { return };
                    if inner.needs_attention() {
                        inner.reconcile();
                    }
                    let late = inner
                        .lock()
                        .graph_thread
                        .as_ref()
                        .map(|t| t.stats.late_cycles.load(Ordering::Relaxed));
                    if let Some(late) = late {
                        if late > late_seen && late_logged_at.elapsed() > Duration::from_secs(5) {
                            tracing::warn!(
                                new = late - late_seen,
                                total = late,
                                "the graph thread was late: the system did not schedule it in time (audio may glitch)"
                            );
                            late_seen = late;
                            late_logged_at = Instant::now();
                        }
                    }
                }
            })
            .expect("cannot spawn the supervisor thread");
        Engine {
            inner,
            stop,
            supervisor: Some(supervisor),
        }
    }

    /// Makes `graph` the running graph. Devices that cannot be opened do not make this fail: their node reports
    /// [`NodeState::Failed`], stays silent, and is retried.
    pub fn apply(&self, graph: ValidGraph) {
        tracing::info!(
            nodes = graph.spec.nodes.len(),
            edges = graph.spec.edges.len(),
            "applying graph"
        );
        self.inner.lock().desired = Some(graph);
        self.inner.reconcile();
    }

    /// Stops everything and forgets the graph.
    /// Stops the audio. The virtual devices stay (and send silence): other programs keep their microphone or speaker
    /// while the graph is stopped, and the next graph that still has the block reuses the device.
    pub fn stop(&self) {
        let mut st = self.inner.lock();
        if st.desired.is_some() || !st.resources.is_empty() {
            tracing::info!(streams = st.resources.len(), "stopping the engine");
        }
        st.desired = None;
        st.graph_thread = None;
        st.resources.retain(|_, r| {
            matches!(
                r.kind,
                NodeKind::VirtualMic { .. } | NodeKind::VirtualSpeaker { .. }
            )
        });
        st.failures.clear();
        st.meters.clear();
        // no graph runs: the soundboards' memory goes back to the system
        if let Some(sounds) = &self.inner.cfg.sounds {
            sounds.sync(&[]);
        }
    }

    /// Stops everything, virtual devices included (the program is ending).
    fn shutdown(&self) {
        self.stop();
        self.inner.lock().resources.clear();
    }

    pub fn status(&self) -> EngineStatus {
        let st = self.inner.lock();
        let (running, cycles, late) = match &st.graph_thread {
            Some(t) => (
                true,
                t.stats.cycles.load(Ordering::Relaxed),
                t.stats.late_cycles.load(Ordering::Relaxed),
            ),
            None => (false, 0, 0),
        };
        let nodes = st
            .desired
            .iter()
            .flat_map(|g| g.spec.nodes.iter())
            .map(|n| {
                let peak = st.meters.get(&n.id).map_or(0.0, |m| m.take());
                let (state, underruns, overruns, fill_ms, frames) =
                    match (st.resources.get(&n.id), st.failures.get(&n.id)) {
                        (Some(r), _) if !r.health.has_failed() => (
                            NodeState::Running,
                            r.stats.underruns.load(Ordering::Relaxed),
                            r.stats.overruns.load(Ordering::Relaxed),
                            r.stats.fill_ms(),
                            r.health.frames(),
                        ),
                        (_, Some(f)) => (NodeState::Failed(f.message.clone()), 0, 0, 0.0, 0),
                        (Some(r), None) => (
                            NodeState::Failed(r.health.error().unwrap_or_default()),
                            0,
                            0,
                            0.0,
                            0,
                        ),
                        (None, None) if n.kind.needs_stream() => {
                            (NodeState::Failed("starting".into()), 0, 0, 0.0, 0)
                        }
                        _ => (NodeState::Running, 0, 0, 0.0, 0),
                    };
                let meter = st.meters.get(&n.id);
                NodeStatus {
                    id: n.id.clone(),
                    state,
                    peak,
                    underruns,
                    overruns,
                    fill_ms,
                    frames,
                    active: meter.and_then(|m| m.condition()),
                    events: meter.map_or(0, |m| m.events()),
                    value: meter.and_then(|m| m.value()),
                }
            })
            .collect();
        EngineStatus {
            running,
            cycles,
            late_cycles: late,
            nodes,
        }
    }

    /// Runtime counters of the graph thread, if it runs.
    pub fn runtime_stats(&self) -> Option<Arc<RuntimeStats>> {
        self.inner
            .lock()
            .graph_thread
            .as_ref()
            .map(|t| t.stats.clone())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.supervisor.take() {
            let _ = t.join();
        }
        self.shutdown();
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while holding the lock must not make the whole engine unusable.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn needs_attention(&self) -> bool {
        let st = self.lock();
        let Some(desired) = &st.desired else {
            return false;
        };
        let now = Instant::now();
        st.resources.values().any(|r| r.health.has_failed())
            || st.failures.values().any(|f| f.retry_at <= now)
            || st.graph_thread.as_ref().is_some_and(|t| t.is_finished())
            || st
                .graph_thread
                .as_ref()
                .is_some_and(|t| t.stats.panics.load(Ordering::Relaxed) > st.panics_handled)
            || (!desired.spec.nodes.is_empty() && st.graph_thread.is_none())
    }

    /// Brings the running streams and the plan in line with `desired`.
    fn reconcile(&self) {
        let mut st = self.lock();
        let Some(desired) = st.desired.clone() else {
            return;
        };
        let now = Instant::now();

        // 1. Drop streams that are no longer wanted, changed, or broke.
        let wanted: HashMap<&NodeId, &NodeKind> = desired
            .spec
            .nodes
            .iter()
            .map(|n| (&n.id, &n.kind))
            .collect();
        let mut broken = vec![];
        st.resources.retain(|id, r| {
            let keep = wanted.get(id).is_some_and(|k| **k == r.kind);
            if keep && r.health.has_failed() {
                broken.push((
                    id.clone(),
                    r.health
                        .error()
                        .unwrap_or_else(|| "the device stopped".into()),
                ));
                return false;
            }
            keep
        });
        for (id, message) in broken {
            tracing::warn!(node = %id.0, "stream broke: {message}; will reopen it");
            let backoff = self.cfg.retry_initial;
            st.failures.insert(
                id,
                Failure {
                    message,
                    retry_at: now + backoff,
                    backoff,
                },
            );
        }
        st.failures.retain(|id, _| wanted.contains_key(id));

        // 2. Open the streams that are missing (and due).
        for node in &desired.spec.nodes {
            if !node.kind.needs_stream() || st.resources.contains_key(&node.id) {
                continue;
            }
            if st.failures.get(&node.id).is_some_and(|f| f.retry_at > now) {
                continue;
            }
            match self.open_node(&node.kind) {
                Ok(resource) => {
                    if st.failures.remove(&node.id).is_some() {
                        tracing::info!(node = %node.id.0, "stream recovered");
                    } else {
                        tracing::info!(node = %node.id.0, name = node.display.as_deref().unwrap_or(""), kind = ?node.kind, "stream started");
                    }
                    st.resources.insert(node.id.clone(), resource);
                }
                Err(message) => {
                    let known = st.failures.get(&node.id);
                    let backoff = known.map_or(self.cfg.retry_initial, |f| {
                        (f.backoff * 2).min(self.cfg.retry_max)
                    });
                    // the first failure (or a different one) is a warning; the retries of the same one are not news
                    if known.is_none_or(|f| f.message != message) {
                        tracing::warn!(node = %node.id.0, retry_in_ms = backoff.as_millis() as u64, "cannot open the stream: {message}");
                    } else {
                        tracing::debug!(node = %node.id.0, "still cannot open the stream: {message}");
                    }
                    st.failures.insert(
                        node.id.clone(),
                        Failure {
                            message,
                            retry_at: now + backoff,
                            backoff,
                        },
                    );
                }
            }
        }

        // 3. Build the plan and hand it to the graph thread. The soundboards of the graph get their memory first.
        if let Some(sounds) = &self.cfg.sounds {
            let boards: Vec<String> = desired
                .spec
                .nodes
                .iter()
                .filter_map(|n| match &n.kind {
                    NodeKind::Soundboard { board, .. } => Some(board.clone()),
                    _ => None,
                })
                .collect();
            sounds.sync(&boards);
        }
        let (plan, meters) = self.build_plan(&desired, &st.resources);
        st.meters = meters;
        if st.graph_thread.as_ref().is_some_and(|t| t.is_finished()) {
            tracing::error!("the graph thread ended unexpectedly; starting a new one");
            st.graph_thread = None;
            st.panics_handled = 0;
        }
        let cfg = &self.cfg;
        let handled = st.panics_handled;
        let thread = st
            .graph_thread
            .get_or_insert_with(|| GraphThread::spawn(cfg.cycle_frames, cfg.thread_init.clone()));
        thread.collect_garbage();
        let panics = thread.stats.panics.load(Ordering::Relaxed);
        let sent = thread.send(Box::new(plan)).is_ok();
        if panics > handled {
            tracing::error!(
                panics,
                "a block panicked on the graph thread (crash report written); running a fresh plan"
            );
            st.panics_handled = panics;
        }
        if sent {
            st.failures.remove(&NodeId("engine".into()));
        } else {
            tracing::warn!("the graph thread is not consuming plans; retrying");
            // The thread is stalled; the supervisor will try again.
            st.failures.insert(
                NodeId("engine".into()),
                Failure {
                    message: "the graph thread is not consuming plans".into(),
                    retry_at: now + Duration::from_secs(1),
                    backoff: Duration::from_secs(1),
                },
            );
        }
        // the virtual devices that exist now are the ones the system should bring back by itself at the next start
        if let Some(provider) = &self.virtual_devices {
            provider.persist();
        }
    }

    fn build_plan(
        &self,
        graph: &ValidGraph,
        resources: &HashMap<NodeId, Resource>,
    ) -> (Plan, HashMap<NodeId, Arc<Meter>>) {
        // where each block of the graph is in the plan (a block can be preceded by a mixer the engine adds)
        let mut position = vec![0usize; graph.spec.nodes.len()];
        let mut meters = HashMap::new();
        let mut nodes: Vec<PlanNode> = Vec::with_capacity(graph.order.len());
        for &idx in &graph.order {
            {
                let spec = &graph.spec.nodes[idx];
                let cycle_frames = self.cfg.cycle_frames as u64;
                let op = match (&spec.kind, resources.get(&spec.id).map(|r| &r.io)) {
                    (_, Some(Io::Source(c))) => NodeOp::Source(c.clone()),
                    (_, Some(Io::Sink(p))) => NodeOp::Sink(p.clone()),
                    (_, Some(Io::Trigger(t))) => NodeOp::Trigger(t.clone()),
                    (_, Some(Io::Filter(f))) => NodeOp::DeepFilter(f.clone()),
                    // the model is not there (not installed, not switched on): the sound goes through as it is
                    (NodeKind::DeepFilter { .. }, None) => NodeOp::Gain(Plan::gain(0.0)),
                    // a soundboard needs nothing from the system: it plays what its pads are told to play
                    (NodeKind::Soundboard { board, triggers }, _) => {
                        match self.cfg.sounds.as_ref().and_then(|s| s.runtime(board)) {
                            Some(runtime) => {
                                runtime.set_triggers(triggers.clone());
                                NodeOp::Soundboard(runtime)
                            }
                            None => NodeOp::SilentSource,
                        }
                    }
                    (k, None) if k.is_source() => NodeOp::SilentSource,
                    (k, None) if k.is_sink() => NodeOp::DiscardSink,
                    // a shortcut that could not be registered never fires
                    (NodeKind::Hotkey { .. }, None) => NodeOp::Trigger(Arc::new(AtomicU32::new(0))),
                    (NodeKind::Gain { db }, _) => NodeOp::Gain(Plan::gain(*db)),
                    (NodeKind::Gate, _) => NodeOp::Gate(Plan::gate()),
                    (NodeKind::Variable { initial }, _) => NodeOp::Variable { state: *initial },
                    (NodeKind::Level { threshold_db }, _) => NodeOp::Level {
                        threshold: Plan::level_threshold(*threshold_db),
                        hold_left: 0,
                    },
                    (NodeKind::Logic { op }, _) => NodeOp::Logic(*op),
                    (NodeKind::Timer { interval_ms }, _) => NodeOp::Timer {
                        interval_frames: (*interval_ms as u64 * ENGINE_SAMPLE_RATE as u64 / 1000)
                            .max(cycle_frames),
                        elapsed: 0,
                    },
                    (NodeKind::Branch { branches }, _) => NodeOp::Branch {
                        gains: Plan::fades(*branches),
                    },
                    (NodeKind::Select { branches }, _) => {
                        Plan::select(*branches, self.cfg.cycle_frames)
                    }
                    (NodeKind::Delay { ms }, _) => Plan::delay(*ms),
                    (NodeKind::NoiseReduction, _) => Plan::noise_reduction(self.cfg.cycle_frames),
                    (NodeKind::Not, _) => NodeOp::Not,
                    (NodeKind::All, _) => NodeOp::All,
                    (NodeKind::Any, _) => NodeOp::Any,
                    (NodeKind::Hold { ms }, _) => NodeOp::Hold {
                        cycles: Plan::cycles(*ms, self.cfg.cycle_frames),
                        left: 0,
                    },
                    (NodeKind::After { ms }, _) => NodeOp::After {
                        cycles: Plan::cycles(*ms, self.cfg.cycle_frames),
                        seen: 0,
                    },
                    (NodeKind::Change, _) => NodeOp::Change { prev: None },
                    (NodeKind::Text { value }, _) => {
                        NodeOp::Const(Value::Text(value.as_str().into()))
                    }
                    (NodeKind::Integer { value }, _) => NodeOp::Const(Value::Int(*value)),
                    (NodeKind::Number { value }, _) => NodeOp::Const(Value::Float(*value)),
                    (NodeKind::Join { template, args }, _) => NodeOp::Join {
                        template: template.clone(),
                        args: (*args).clamp(1, omu_types::MAX_JOIN_ARGS),
                        last: Vec::new(),
                        text: "".into(),
                    },
                    (NodeKind::Math { op }, _) => NodeOp::Math(*op),
                    (NodeKind::Compare { op }, _) => NodeOp::Compare(*op),
                    (NodeKind::TextCompare { op }, _) => NodeOp::TextCompare(*op),
                    (NodeKind::Choose, _) => NodeOp::Choose,
                    (NodeKind::Counter { initial }, _) => NodeOp::Counter {
                        count: *initial,
                        initial: *initial,
                    },
                    (NodeKind::Volume, _) => NodeOp::Volume,
                    // every other kind has been handled above (streams, triggers and the blocks); what is left is the mixer
                    _ => NodeOp::Mixer,
                };
                let wires: Vec<(Wire, bool)> = graph
                    .incoming(idx)
                    .filter_map(|e| {
                        let edge = &graph.spec.edges[e];
                        let source = graph.edge_nodes[e].0;
                        let audio = spec
                            .kind
                            .port(&edge.to.port)
                            .is_some_and(|p| p.ty == PortType::Audio);
                        Port::from_name(&edge.to.port).map(|port| {
                            (
                                Wire {
                                    from: position[source],
                                    out: graph.spec.nodes[source]
                                        .kind
                                        .output_index(&edge.from.port)
                                        .unwrap_or(0),
                                    port,
                                },
                                audio,
                            )
                        })
                    })
                    .collect();
                let inputs = sum_sounds(&mut nodes, &spec.id, matches!(op, NodeOp::Mixer), &wires);
                let meter: Arc<Meter> = Arc::default();
                meters.insert(spec.id.clone(), meter.clone());
                position[idx] = nodes.len();
                // the setting a flow can give to this block, when it has one
                let param = match &spec.kind {
                    NodeKind::Gain { .. } => Some(ParamKind::GainDb),
                    NodeKind::Level { .. } => Some(ParamKind::ThresholdDb),
                    NodeKind::Hold { .. } => Some(ParamKind::HoldMs),
                    NodeKind::After { .. } => Some(ParamKind::AfterMs),
                    NodeKind::Timer { .. } => Some(ParamKind::IntervalMs),
                    _ => None,
                };
                nodes.push(PlanNode {
                    id: spec.id.clone(),
                    op,
                    inputs,
                    meter,
                    param,
                });
            }
        }
        (Plan::new(nodes, self.cfg.cycle_frames), meters)
    }

    fn open_node(&self, kind: &NodeKind) -> Result<Resource, String> {
        let describe = |e: omu_hal::HalError| e.to_string();
        match kind {
            NodeKind::AppCapture { target } => self.start_source(
                kind,
                self.backend.open_app_capture(target).map_err(describe)?,
            ),
            NodeKind::DeviceCapture { device } => {
                self.start_source(kind, self.backend.open_capture(device).map_err(describe)?)
            }
            NodeKind::DeviceRender { device } => {
                self.start_sink(kind, self.backend.open_render(device).map_err(describe)?)
            }
            NodeKind::VirtualSpeaker { name } => {
                let provider = self.virtual_devices.as_ref().ok_or_else(|| {
                    "no virtual device provider is available on this system".to_string()
                })?;
                provider.available().map_err(describe)?;
                self.start_source(kind, provider.create_speaker(name).map_err(describe)?)
            }
            NodeKind::DeepFilter { limit_db } => {
                let model = self
                    .cfg
                    .sounds
                    .as_ref()
                    .and_then(|s| s.modules().deepfilter_model())
                    .ok_or_else(|| {
                        "the DeepFilterNet module is not installed or not switched on (see the Modules panel)".to_string()
                    })?;
                let (endpoints, worker) = omu_deepfilter::start(&model, *limit_db)?;
                Ok(Resource {
                    kind: kind.clone(),
                    io: Io::Filter(endpoints),
                    health: StreamHealth::new(),
                    stats: Arc::default(),
                    _running: Box::new(worker),
                })
            }
            NodeKind::NetworkIn { port, stream, from } => {
                let capture =
                    omu_net::vban::VbanCapture::open(*port, stream, from).map_err(describe)?;
                self.start_source(kind, Box::new(capture))
            }
            NodeKind::NetworkOut { host, port, stream } => {
                let render =
                    omu_net::vban::VbanRender::open(host, *port, stream).map_err(describe)?;
                self.start_sink(kind, Box::new(render))
            }
            NodeKind::VirtualMic { name } => {
                let provider = self.virtual_devices.as_ref().ok_or_else(|| {
                    "no virtual device provider is available on this system".to_string()
                })?;
                provider.available().map_err(describe)?;
                self.start_sink(kind, provider.create_microphone(name).map_err(describe)?)
            }
            NodeKind::Hotkey { keys } => {
                let combo = KeyCombo::parse(keys)?;
                let counter = Arc::new(AtomicU32::new(0));
                let c2 = counter.clone();
                let running = self
                    .backend
                    .open_hotkey(
                        &combo,
                        Box::new(move || {
                            c2.fetch_add(1, Ordering::AcqRel);
                        }),
                    )
                    .map_err(describe)?;
                Ok(Resource {
                    kind: kind.clone(),
                    io: Io::Trigger(counter),
                    health: StreamHealth::new(),
                    stats: Arc::default(),
                    _running: running,
                })
            }
            _ => Err("this block needs nothing from the system".into()),
        }
    }

    fn start_source(
        &self,
        kind: &NodeKind,
        stream: Box<dyn CaptureStream>,
    ) -> Result<Resource, String> {
        let (format, health) = (stream.format(), stream.health());
        let (mut producer, consumer, stats) = source_bridge(format, self.cfg.target_ms);
        let running = stream
            .start(Box::new(move |data| producer.push(data)))
            .map_err(|e| e.to_string())?;
        Ok(Resource {
            kind: kind.clone(),
            io: Io::Source(Arc::new(Mutex::new(consumer))),
            health,
            stats,
            _running: running,
        })
    }

    fn start_sink(
        &self,
        kind: &NodeKind,
        stream: Box<dyn RenderStream>,
    ) -> Result<Resource, String> {
        let (format, health) = (stream.format(), stream.health());
        let (producer, mut consumer, stats) = sink_bridge(format, self.cfg.target_ms);
        let running = stream
            .start(Box::new(move |out| consumer.pull(out)))
            .map_err(|e| e.to_string())?;
        Ok(Resource {
            kind: kind.clone(),
            io: Io::Sink(Arc::new(Mutex::new(producer))),
            health,
            stats,
            _running: running,
        })
    }
}

/// Several sounds entering one input are added together: the engine puts a mixer of its own just before the block (a
/// block that is itself a mixer adds its inputs on its own). Returns the wires of the block.
fn sum_sounds(
    nodes: &mut Vec<PlanNode>,
    owner: &NodeId,
    adds_itself: bool,
    wires: &[(Wire, bool)],
) -> Vec<Wire> {
    let mut out = Vec::with_capacity(wires.len());
    let mut summed: Vec<Port> = vec![];
    for (wire, audio) in wires {
        if adds_itself || !audio {
            out.push(*wire);
            continue;
        }
        if summed.contains(&wire.port) {
            continue;
        }
        let same: Vec<Wire> = wires
            .iter()
            .filter(|(w, a)| *a && w.port == wire.port)
            .map(|(w, _)| *w)
            .collect();
        if same.len() == 1 {
            out.push(*wire);
            continue;
        }
        summed.push(wire.port);
        nodes.push(PlanNode {
            id: NodeId(format!("{}~sum{}", owner.0, nodes.len())),
            op: NodeOp::Mixer,
            inputs: same
                .iter()
                .map(|w| Wire {
                    port: Port::In,
                    ..*w
                })
                .collect(),
            meter: Arc::default(),
            param: None,
        });
        out.push(Wire {
            from: nodes.len() - 1,
            out: 0,
            port: wire.port,
        });
    }
    out
}
