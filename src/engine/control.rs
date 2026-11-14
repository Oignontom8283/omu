//! The control side of the engine: turns a validated graph into running streams and an executable plan, keeps
//! them alive (retrying failed devices with a growing delay), and reports status.
//!
//! Applying a new graph reuses every stream whose node did not change, so editing a gain or adding a branch
//! does not interrupt the audio of the other nodes.

use super::bridge::{sink_bridge, source_bridge, BridgeStats, SinkProducer, SourceConsumer};
use super::plan::{Meter, NodeOp, Plan, PlanNode};
use super::runtime::{GraphThread, RuntimeStats, ThreadInit};
use crate::graph::ValidGraph;
use crate::hal::{
    Backend, CaptureStream, RenderStream, Running, StreamHealth, VirtualDeviceProvider,
};
use crate::types::{NodeId, NodeKind};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
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
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            cycle_frames: 480,
            target_ms: 25.0,
            thread_init: None,
            retry_initial: Duration::from_secs(1),
            retry_max: Duration::from_secs(10),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", content = "message", rename_all = "snake_case")]
pub enum NodeState {
    Running,
    /// The node's stream could not be opened or broke; the engine retries. The node is silent meanwhile.
    Failed(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeStatus {
    pub id: NodeId,
    #[serde(flatten)]
    pub state: NodeState,
    /// Highest level (linear, 1.0 = full scale) since the previous status call.
    pub peak: f32,
    pub underruns: u64,
    pub overruns: u64,
    pub fill_ms: f32,
    pub frames: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineStatus {
    pub running: bool,
    pub cycles: u64,
    pub late_cycles: u64,
    pub nodes: Vec<NodeStatus>,
}

enum Io {
    Source(Arc<Mutex<SourceConsumer>>),
    Sink(Arc<Mutex<SinkProducer>>),
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
                while !stop2.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(250));
                    let Some(inner) = weak.upgrade() else { return };
                    if inner.needs_attention() {
                        inner.reconcile();
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
        self.inner.lock().desired = Some(graph);
        self.inner.reconcile();
    }

    /// Stops everything and forgets the graph.
    pub fn stop(&self) {
        let mut st = self.inner.lock();
        st.desired = None;
        st.graph_thread = None;
        st.resources.clear();
        st.failures.clear();
        st.meters.clear();
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
                        (None, None) if n.kind.is_source() || n.kind.is_sink() => {
                            (NodeState::Failed("starting".into()), 0, 0, 0.0, 0)
                        }
                        _ => (NodeState::Running, 0, 0, 0.0, 0),
                    };
                NodeStatus {
                    id: n.id.clone(),
                    state,
                    peak,
                    underruns,
                    overruns,
                    fill_ms,
                    frames,
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
        self.stop();
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
            if !(node.kind.is_source() || node.kind.is_sink())
                || st.resources.contains_key(&node.id)
            {
                continue;
            }
            if st.failures.get(&node.id).is_some_and(|f| f.retry_at > now) {
                continue;
            }
            match self.open_node(&node.kind) {
                Ok(resource) => {
                    st.failures.remove(&node.id);
                    st.resources.insert(node.id.clone(), resource);
                }
                Err(message) => {
                    let backoff = st
                        .failures
                        .get(&node.id)
                        .map_or(self.cfg.retry_initial, |f| {
                            (f.backoff * 2).min(self.cfg.retry_max)
                        });
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

        // 3. Build the plan and hand it to the graph thread.
        let (plan, meters) = self.build_plan(&desired, &st.resources);
        st.meters = meters;
        if st.graph_thread.as_ref().is_some_and(|t| t.is_finished()) {
            st.graph_thread = None;
        }
        let cfg = &self.cfg;
        let thread = st
            .graph_thread
            .get_or_insert_with(|| GraphThread::spawn(cfg.cycle_frames, cfg.thread_init.clone()));
        thread.collect_garbage();
        if thread.send(Box::new(plan)).is_err() {
            // The thread is stalled; the supervisor will try again.
            st.failures.insert(
                NodeId("engine".into()),
                Failure {
                    message: "the graph thread is not consuming plans".into(),
                    retry_at: now + Duration::from_secs(1),
                    backoff: Duration::from_secs(1),
                },
            );
        } else {
            st.failures.remove(&NodeId("engine".into()));
        }
    }

    fn build_plan(
        &self,
        graph: &ValidGraph,
        resources: &HashMap<NodeId, Resource>,
    ) -> (Plan, HashMap<NodeId, Arc<Meter>>) {
        let mut position = vec![0usize; graph.spec.nodes.len()];
        for (pos, &idx) in graph.order.iter().enumerate() {
            position[idx] = pos;
        }
        let mut meters = HashMap::new();
        let nodes: Vec<PlanNode> = graph
            .order
            .iter()
            .map(|&idx| {
                let spec = &graph.spec.nodes[idx];
                let op = match (&spec.kind, resources.get(&spec.id).map(|r| &r.io)) {
                    (_, Some(Io::Source(c))) => NodeOp::Source(c.clone()),
                    (_, Some(Io::Sink(p))) => NodeOp::Sink(p.clone()),
                    (k, None) if k.is_source() => NodeOp::SilentSource,
                    (k, None) if k.is_sink() => NodeOp::DiscardSink,
                    (NodeKind::Gain { db }, _) => NodeOp::Gain(Plan::gain(*db)),
                    _ => NodeOp::Mixer,
                };
                let inputs = graph
                    .incoming(idx)
                    .map(|e| position[graph.edge_nodes[e].0])
                    .collect();
                let meter: Arc<Meter> = Arc::default();
                meters.insert(spec.id.clone(), meter.clone());
                PlanNode {
                    id: spec.id.clone(),
                    op,
                    inputs,
                    meter,
                }
            })
            .collect();
        (Plan::new(nodes, self.cfg.cycle_frames), meters)
    }

    fn open_node(&self, kind: &NodeKind) -> Result<Resource, String> {
        let describe = |e: crate::hal::HalError| e.to_string();
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
            NodeKind::VirtualMic { name } => {
                let provider = self.virtual_devices.as_ref().ok_or_else(|| {
                    "no virtual device provider is available on this system".to_string()
                })?;
                provider.available().map_err(describe)?;
                self.start_sink(kind, provider.create_microphone(name).map_err(describe)?)
            }
            NodeKind::Gain { .. } | NodeKind::Mixer => Err("not a device node".into()),
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
