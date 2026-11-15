//! The daemon: the process that owns the audio engine and answers the control protocol.
//!
//! * [`Daemon`] is the transport-independent core: it dispatches JSON-RPC requests to the engine, remembers the
//!   graph and the editors' layout ([`store`]), and publishes events. It is plain synchronous Rust, so it is tested
//!   without any network.
//! * the network part (HTTP, WebSocket, the web panel) lives in the sibling modules and only forwards messages here.

pub mod auth;
pub mod instance;
pub mod launch;
pub mod server;
pub mod store;
pub mod web;

use crate::engine::{Engine, EngineConfig};
use crate::graph::validate;
use crate::hal::Direction;
use crate::hal::{Backend, HalError, VirtualDeviceProvider};
use crate::protocol::methods::{
    self as m, Method, ProviderInfo, SystemInfo, ValidateResult, EVENT_GRAPH, EVENT_STATUS,
};
use crate::protocol::{
    Id, Notification, Request, Response, RpcError, JSONRPC_VERSION, PROTOCOL_VERSION,
};
use crate::types::{DeviceRef, EngineStatus, GraphSpec, KeptDevices, NodeKind};
use crate::virtual_devices::clean;
use serde_json::Value;
use std::sync::{Arc, Mutex, MutexGuard};
use store::{SavedState, Store};

/// Receives the events the daemon publishes.
pub type EventListener = Box<dyn Fn(&Notification) + Send + Sync>;

struct State {
    saved: SavedState,
    layout: crate::protocol::methods::Layout,
    status: EngineStatus,
}

pub struct Daemon {
    engine: Engine,
    backend: Arc<dyn Backend>,
    virtual_devices: Option<Arc<dyn VirtualDeviceProvider>>,
    store: Store,
    state: Mutex<State>,
    listeners: Mutex<Vec<EventListener>>,
    previous: Mutex<crate::diagnostics::PreviousSession>,
    /// Woken when a client asks the daemon to stop (`system.shutdown`).
    stop: Arc<tokio::sync::Notify>,
    /// Administrator rights, for cleaning up old devices.
    elevator: Arc<dyn crate::elevation::Elevator>,
    /// The soundboards: their memory, their sounds and what they play.
    sounds: Arc<crate::soundboard::SoundEngine>,
    /// The keyboard shortcuts that play pads.
    pad_hotkeys: crate::soundboard::hotkeys::PadHotkeys,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn hal_error(e: HalError) -> RpcError {
    match e {
        HalError::Unsupported(_) => RpcError::new(RpcError::UNAVAILABLE, e.to_string()),
        _ => RpcError::internal(e.to_string()),
    }
}

fn sound_error(e: crate::soundboard::SoundError) -> RpcError {
    use crate::soundboard::SoundError as E;
    match &e {
        E::UnknownBoard(_) | E::UnknownPad(_) | E::NoSound | E::UnknownSound(_) => {
            RpcError::invalid_params(e.to_string())
        }
        E::NotInGraph(_) | E::NotActive | E::PoolFull(_) | E::TooManyStreams => {
            RpcError::new(RpcError::UNAVAILABLE, e.to_string())
        }
        E::Io(_) => RpcError::internal(e.to_string()),
    }
}

/// Decodes the parameters of method `M`.
fn params<M: Method>(value: Value) -> Result<M::Params, RpcError> {
    serde_json::from_value(value).map_err(|e| RpcError::invalid_params(format!("{}: {e}", M::NAME)))
}

fn result<T: serde::Serialize>(v: T) -> Result<Value, RpcError> {
    serde_json::to_value(v).map_err(|e| RpcError::internal(e.to_string()))
}

impl Daemon {
    pub fn new(
        backend: Arc<dyn Backend>,
        virtual_devices: Option<Arc<dyn VirtualDeviceProvider>>,
        engine_config: EngineConfig,
        store: Store,
    ) -> Daemon {
        let saved = store.load_state();
        let layout = store.load_layout();
        let sounds = Arc::new(crate::soundboard::SoundEngine::new(store.dir()));
        let mut engine_config = engine_config;
        engine_config.sounds = Some(sounds.clone());
        let engine = Engine::new(backend.clone(), virtual_devices.clone(), engine_config);
        Daemon {
            engine,
            backend,
            virtual_devices,
            store,
            state: Mutex::new(State {
                saved,
                layout,
                status: EngineStatus::default(),
            }),
            listeners: Mutex::new(vec![]),
            previous: Mutex::default(),
            stop: Arc::default(),
            elevator: Arc::new(crate::elevation::SystemElevator),
            sounds,
            pad_hotkeys: Default::default(),
        }
    }

    /// Makes the system's registered pad shortcuts those of the pads whose soundboard is in the running graph. Called
    /// whenever that can change: a graph is applied or stopped, or the pads are edited.
    fn sync_pad_hotkeys(&self) {
        let wanted = {
            let st = lock(&self.state);
            if st.saved.running {
                crate::soundboard::hotkeys::wanted(
                    &self.sounds.library().get().boards,
                    &st.saved.graph,
                )
            } else {
                vec![]
            }
        };
        self.pad_hotkeys
            .sync(self.backend.as_ref(), &self.sounds, &wanted);
    }

    /// The soundboards' engine (the panel's upload route imports through it).
    pub fn sounds(&self) -> &Arc<crate::soundboard::SoundEngine> {
        &self.sounds
    }

    /// The same daemon asking for administrator rights another way (tests).
    pub fn with_elevator(mut self, elevator: Arc<dyn crate::elevation::Elevator>) -> Daemon {
        self.elevator = elevator;
        self
    }

    /// Completes when a client has asked the daemon to stop.
    pub fn stop_signal(&self) -> Arc<tokio::sync::Notify> {
        self.stop.clone()
    }

    /// Tells the daemon what the previous run left behind, for `system.info`.
    pub fn set_previous_session(&self, previous: crate::diagnostics::PreviousSession) {
        *lock(&self.previous) = previous;
    }

    /// Starts again what was running when the daemon last stopped.
    pub fn restore(&self) {
        let saved = lock(&self.state).saved.clone();
        if saved.running {
            match validate(&saved.graph) {
                Ok(valid) => {
                    tracing::info!("restoring the graph that was running at the last stop");
                    self.engine.apply(valid);
                    self.sync_pad_hotkeys();
                }
                Err(e) => {
                    tracing::warn!("the saved graph is no longer valid and was not restored: {e:?}")
                }
            }
        }
    }

    /// Registers a listener for events (called from whichever thread publishes).
    pub fn on_event(&self, listener: EventListener) {
        lock(&self.listeners).push(listener);
    }

    fn publish(&self, method: &str, params: Value) {
        let note = Notification::new(method, params);
        for l in lock(&self.listeners).iter() {
            l(&note);
        }
    }

    /// Reads the engine's status (consuming the level meters, so only this method may do it), caches it for
    /// `engine.status` and publishes it. Called about four times a second by the server's timer.
    pub fn tick(&self) -> EngineStatus {
        let status = self.engine.status();
        lock(&self.state).status = status.clone();
        if let Ok(v) = serde_json::to_value(&status) {
            self.publish(EVENT_STATUS, v);
        }
        status
    }

    /// Answers one request. `None` for a notification (a request without an id).
    pub fn handle(&self, request: Request) -> Option<Response> {
        let id = request.id.clone();
        if request.jsonrpc != JSONRPC_VERSION {
            return id.map(|id| {
                Response::err(
                    Some(id),
                    RpcError::invalid_request("jsonrpc must be \"2.0\""),
                )
            });
        }
        let method = request.method.clone();
        let started = std::time::Instant::now();
        // A bug in one method must answer that request with an error, not take the daemon down.
        let outcome =
            crate::diagnostics::contained(|| self.dispatch(&request.method, request.params))
                .unwrap_or_else(|_| {
                    Err(RpcError::internal(
                        "internal error while handling the request (a crash report was written)",
                    ))
                });
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        match &outcome {
            // the panel asks for the status several times a second: that is not worth a line
            Ok(_) if method == m::EngineStatusMethod::NAME || method == m::LogsRecent::NAME => {
                tracing::trace!(%method, elapsed_ms, "request")
            }
            Ok(_) => tracing::debug!(%method, elapsed_ms, "request"),
            Err(e) => {
                tracing::warn!(%method, code = e.code, elapsed_ms, "request failed: {}", e.message)
            }
        }
        let id = id?;
        Some(match outcome {
            Ok(v) => Response::ok(Some(id), v),
            Err(e) => Response::err(Some(id), e),
        })
    }

    /// Answers a raw JSON text (one request, or a batch array); the reply is JSON text, `None` if nothing is owed.
    pub fn handle_text(&self, text: &str) -> Option<String> {
        let value: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => {
                return serde_json::to_string(&Response::err(None, RpcError::parse(e.to_string())))
                    .ok();
            }
        };
        let one = |v: Value| -> Option<Response> {
            match serde_json::from_value::<Request>(v) {
                Ok(r) => self.handle(r),
                Err(e) => Some(Response::err(
                    None,
                    RpcError::invalid_request(e.to_string()),
                )),
            }
        };
        match value {
            Value::Array(items) if !items.is_empty() => {
                let replies: Vec<Response> = items.into_iter().filter_map(one).collect();
                (!replies.is_empty())
                    .then(|| serde_json::to_string(&replies).ok())
                    .flatten()
            }
            Value::Array(_) => serde_json::to_string(&Response::err(
                None,
                RpcError::invalid_request("empty batch"),
            ))
            .ok(),
            other => one(other).and_then(|r| serde_json::to_string(&r).ok()),
        }
    }

    fn dispatch(&self, method: &str, p: Value) -> Result<Value, RpcError> {
        match method {
            m::SystemInfoMethod::NAME => result(self.system_info()),
            m::DevicesList::NAME => result(self.backend.list_devices().map_err(hal_error)?),
            m::DevicesStale::NAME => {
                let kept = KeptDevices::of_graph(&lock(&self.state).saved.graph);
                result(clean::survey(
                    self.virtual_devices.as_deref(),
                    &kept,
                    &*self.elevator,
                ))
            }
            m::DevicesClean::NAME => {
                let kept = KeptDevices::of_graph(&lock(&self.state).saved.graph);
                let provider = self.virtual_devices.as_deref().ok_or_else(|| {
                    RpcError::new(
                        RpcError::UNAVAILABLE,
                        "there are no virtual devices to clean up",
                    )
                })?;
                let relaunch = clean::Relaunch::this_program(self.store.dir())
                    .map_err(|e| RpcError::internal(e.to_string()))?;
                tracing::info!("cleaning up the old devices");
                let outcome = clean::clean(provider, &kept, &*self.elevator, &relaunch).map_err(
                    |e| match e {
                        clean::CleanError::Unsupported(m) => {
                            RpcError::new(RpcError::UNAVAILABLE, m)
                        }
                        clean::CleanError::Declined => {
                            RpcError::new(RpcError::CANCELLED, e.to_string())
                        }
                        clean::CleanError::Failed(m) => RpcError::internal(m),
                    },
                )?;
                tracing::info!(
                    removed = outcome.removed,
                    remaining = outcome.remaining,
                    "old devices cleaned up"
                );
                result(outcome)
            }
            m::SoundboardsGet::NAME => result(self.sounds.library().get()),
            m::SoundboardsSet::NAME => {
                let boards = params::<m::SoundboardsSet>(p)?;
                self.sounds
                    .library()
                    .set_boards(boards)
                    .map_err(RpcError::invalid_params)?;
                self.sync_pad_hotkeys();
                result(())
            }
            m::SoundsImport::NAME => {
                let p = params::<m::SoundsImport>(p)?;
                tracing::info!(file = %p.path, "importing a sound");
                let info = self
                    .sounds
                    .import(std::path::Path::new(&p.path), p.name.as_deref())
                    .map_err(sound_error)?;
                result(info)
            }
            m::SoundsCompress::NAME => {
                let p = params::<m::SoundsCompress>(p)?;
                result(self.sounds.compress(&p.id).map_err(sound_error)?)
            }
            m::SoundsRemove::NAME => {
                let p = params::<m::SoundsRemove>(p)?;
                result(self.sounds.remove_sound(&p.id).map_err(sound_error)?)
            }
            m::SoundboardPlay::NAME => {
                let p = params::<m::SoundboardPlay>(p)?;
                self.sounds.play(&p.board, &p.pad).map_err(sound_error)?;
                result(())
            }
            m::SoundboardStop::NAME => {
                let p = params::<m::SoundboardStop>(p)?;
                self.sounds
                    .stop(&p.board, p.pad.as_deref())
                    .map_err(sound_error)?;
                result(())
            }
            m::SoundboardPlaying::NAME => {
                let p = params::<m::SoundboardPlaying>(p)?;
                result(self.sounds.playing(&p.board))
            }
            m::SoundboardsStatus::NAME => {
                let mut status = self.sounds.status();
                status.hotkey_problems = self.pad_hotkeys.problems();
                result(status)
            }
            m::SettingsGet::NAME => result(self.sounds.settings()),
            m::SettingsSet::NAME => {
                let settings = params::<m::SettingsSet>(p)?;
                self.sounds
                    .set_settings(settings)
                    .map_err(RpcError::internal)?;
                result(())
            }
            m::AppsList::NAME => result(self.backend.list_apps().map_err(hal_error)?),
            m::GraphGet::NAME => result(lock(&self.state).saved.graph.clone()),
            m::GraphValidate::NAME => {
                let graph = params::<m::GraphValidate>(p)?;
                let errors = self.problems(&graph);
                result(ValidateResult {
                    ok: errors.is_empty(),
                    errors,
                })
            }
            m::GraphSet::NAME => {
                let graph = params::<m::GraphSet>(p)?;
                self.set_graph(graph)?;
                result(())
            }
            m::EngineStatusMethod::NAME => result(lock(&self.state).status.clone()),
            m::EngineStop::NAME => {
                tracing::info!("engine stop requested");
                self.engine.stop();
                let mut st = lock(&self.state);
                st.saved.running = false;
                let _ = self.store.save_state(&st.saved);
                st.status = EngineStatus::default();
                drop(st);
                self.sync_pad_hotkeys();
                result(())
            }
            m::SystemShutdown::NAME => {
                tracing::info!("a client asked the daemon to stop");
                // the permit is kept if nobody waits yet; the reply goes out before the server closes
                self.stop.notify_one();
                result(())
            }
            m::LogsRecent::NAME => {
                let p = params::<m::LogsRecent>(p)?;
                let limit = p
                    .limit
                    .unwrap_or(200)
                    .clamp(1, crate::diagnostics::ring::CAPACITY);
                result(match p.after {
                    Some(seq) => crate::diagnostics::ring::since(seq, limit),
                    None => crate::diagnostics::ring::recent(limit),
                })
            }
            m::LayoutGet::NAME => result(lock(&self.state).layout.clone()),
            m::LayoutSet::NAME => {
                let layout = params::<m::LayoutSet>(p)?;
                let mut st = lock(&self.state);
                self.store
                    .save_layout(&layout)
                    .map_err(|e| RpcError::internal(format!("cannot save the layout: {e}")))?;
                st.layout = layout;
                result(())
            }
            other => Err(RpcError::method_not_found(other)),
        }
    }

    fn system_info(&self) -> SystemInfo {
        let previous = lock(&self.previous).clone();
        SystemInfo {
            previous_run_unclean: previous.unclean,
            crash_report: previous.crash_report.map(|p| p.display().to_string()),
            name: crate::branding::DISPLAY_NAME.into(),
            version: env!("CARGO_PKG_VERSION").into(),
            protocol_version: PROTOCOL_VERSION,
            platform: std::env::consts::OS.into(),
            backend: self.backend.name().into(),
            capabilities: self.backend.capabilities(),
            virtual_microphones: self.virtual_devices.as_ref().map(|p| match p.available() {
                Ok(()) => ProviderInfo {
                    name: p.name().into(),
                    available: true,
                    error: None,
                },
                Err(e) => ProviderInfo {
                    name: p.name().into(),
                    available: false,
                    error: Some(e.to_string()),
                },
            }),
        }
    }

    /// Everything wrong with `graph`: the structural problems, and the daemon's own rules that need to know the
    /// devices (a graph may not record this product's virtual microphones: the program using one would hear its own
    /// output).
    fn problems(&self, graph: &GraphSpec) -> Vec<String> {
        let mut out: Vec<String> = match validate(graph) {
            Ok(_) => vec![],
            Err(errors) => errors.iter().map(|e| e.to_string()).collect(),
        };
        out.extend(self.feedback_problems(graph));
        // a soundboard block must name a soundboard that exists
        for n in &graph.nodes {
            if let NodeKind::Soundboard { board } = &n.kind {
                if self.sounds.library().board(board).is_none() {
                    out.push(format!(
                        "block {:?} plays the soundboard {board:?}, which does not exist",
                        n.id.0
                    ));
                }
            }
        }
        out
    }

    fn feedback_problems(&self, graph: &GraphSpec) -> Vec<String> {
        let provider = self.virtual_devices.as_ref();
        let mut mics: Vec<String> = graph
            .nodes
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::VirtualMic { name } => Some(name.clone()),
                _ => None,
            })
            .collect();
        mics.extend(provider.map(|p| p.microphone_names()).unwrap_or_default());
        let mut speakers: Vec<String> = graph
            .nodes
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::VirtualSpeaker { name } => Some(name.clone()),
                _ => None,
            })
            .collect();
        speakers.extend(provider.map(|p| p.speaker_names()).unwrap_or_default());

        let mut problems = self.feedback_for(
            graph,
            &mics,
            Direction::Capture,
            |k| match k {
                NodeKind::DeviceCapture { device } => Some(device),
                _ => None,
            },
            "records the virtual microphone",
            "whatever is sent to it would be heard again",
        );
        problems.extend(self.feedback_for(
            graph,
            &speakers,
            Direction::Render,
            |k| match k {
                NodeKind::DeviceRender { device } => Some(device),
                _ => None,
            },
            "plays into the virtual speaker",
            "it would come straight back into the graph",
        ));
        problems
    }

    /// The nodes (`device_of` picks the device a node uses, if it uses one in `direction`) that use one of this
    /// product's own virtual devices `names`: a loop.
    fn feedback_for<'g>(
        &self,
        graph: &'g GraphSpec,
        names: &[String],
        direction: Direction,
        device_of: impl Fn(&'g NodeKind) -> Option<&'g DeviceRef>,
        verb: &str,
        consequence: &str,
    ) -> Vec<String> {
        let mut own: Vec<String> = names
            .iter()
            .map(|n| n.trim().to_lowercase())
            .filter(|n| !n.is_empty())
            .collect();
        own.dedup();
        if own.is_empty() {
            return vec![];
        }
        let devices = self.backend.list_devices().unwrap_or_default();
        let listed = || devices.iter().filter(|d| d.direction == direction);
        let mut problems = vec![];
        for node in &graph.nodes {
            let Some(device) = device_of(&node.kind) else {
                continue;
            };
            let candidates: Vec<String> = match device {
                DeviceRef::Name(n) => {
                    let n = n.to_lowercase();
                    std::iter::once(n.clone())
                        .chain(
                            listed()
                                .filter(|d| d.name.to_lowercase().contains(&n))
                                .map(|d| d.name.to_lowercase()),
                        )
                        .collect()
                }
                DeviceRef::Id(id) => listed()
                    .filter(|d| &d.id == id)
                    .map(|d| d.name.to_lowercase())
                    .collect(),
                DeviceRef::Default => listed()
                    .filter(|d| d.is_default)
                    .map(|d| d.name.to_lowercase())
                    .collect(),
            };
            // Windows names an emulated device "Microphone (<name>)" or "Speakers (<name>)": match the name exactly or
            // inside those parentheses (a short name such as "Mic" must not match every device with "mic" in its name).
            let is_ours = |c: &str, m: &str| c == m || c.ends_with(&format!("({m})"));
            if let Some(own) = own
                .iter()
                .find(|m| candidates.iter().any(|c| is_ours(c, m)))
            {
                problems.push(format!(
                    "node {:?} {verb} {own:?}: {consequence} (feedback)",
                    node.id.0
                ));
            }
        }
        problems
    }

    fn set_graph(&self, graph: GraphSpec) -> Result<(), RpcError> {
        let errors = self.problems(&graph);
        if !errors.is_empty() {
            tracing::warn!(?errors, "a client sent an invalid graph; refused");
            return Err(RpcError::new(
                RpcError::VALIDATION,
                format!("the graph is invalid: {}", errors.join("; ")),
            )
            .with_data(serde_json::json!(errors)));
        }
        tracing::info!(
            nodes = graph.nodes.len(),
            edges = graph.edges.len(),
            "graph set by a client"
        );
        crate::diagnostics::note(
            "graph",
            format!("{} nodes, {} edges", graph.nodes.len(), graph.edges.len()),
        );
        // `problems` found none, so validation succeeds
        let valid = validate(&graph)
            .map_err(|_| RpcError::internal("the graph changed under validation"))?;
        self.engine.apply(valid);
        {
            let mut st = lock(&self.state);
            st.saved = SavedState {
                graph: graph.clone(),
                running: true,
            };
            if let Err(e) = self.store.save_state(&st.saved) {
                // The graph is running; only its persistence failed. Report it without undoing anything.
                return Err(RpcError::internal(format!(
                    "the graph runs but could not be saved: {e}"
                )));
            }
        }
        self.sync_pad_hotkeys();
        if let Ok(v) = serde_json::to_value(&graph) {
            self.publish(EVENT_GRAPH, v);
        }
        Ok(())
    }
}

/// Builds a JSON-RPC error response to a message that could not even be parsed as a request.
pub fn parse_error_response(message: &str) -> Response {
    Response::err(None::<Id>, RpcError::parse(message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hal::sim::{SimBackend, SimSource};
    use crate::protocol::methods::NodeLayout;
    use crate::types::{AppTarget, EdgeSpec, NodeKind, NodeSpec, NodeState, PortRef};
    use serde_json::json;
    use std::time::Duration;
    use store::temp_dir;

    fn daemon_in(dir: &std::path::Path, sim: &SimBackend) -> Daemon {
        Daemon::new(
            Arc::new(sim.clone()) as Arc<dyn Backend>,
            Some(Arc::new(sim.clone()) as Arc<dyn VirtualDeviceProvider>),
            EngineConfig::default(),
            Store::new(dir),
        )
    }

    fn call(d: &Daemon, method: &str, params: Value) -> Response {
        d.handle(Request::new(1, method, params))
            .expect("a response")
    }

    fn chain() -> GraphSpec {
        GraphSpec {
            nodes: vec![
                NodeSpec {
                    id: "app".into(),
                    display: None,
                    kind: NodeKind::AppCapture {
                        target: AppTarget::ProcessName("a.exe".into()),
                    },
                },
                NodeSpec {
                    id: "mic".into(),
                    display: None,
                    kind: NodeKind::VirtualMic { name: "Mic".into() },
                },
            ],
            edges: vec![EdgeSpec {
                from: PortRef::new("app", "out"),
                to: PortRef::new("mic", "in"),
            }],
        }
    }

    #[test]
    fn system_info_describes_the_daemon() {
        let sim = SimBackend::new();
        let d = daemon_in(&temp_dir("info"), &sim);
        let r = call(&d, "system.info", Value::Null);
        let info: SystemInfo = serde_json::from_value(r.result.unwrap()).unwrap();
        assert_eq!(info.protocol_version, PROTOCOL_VERSION);
        assert_eq!(info.backend, "simulated");
        assert!(info.virtual_microphones.unwrap().available);
        assert_eq!(info.name, crate::branding::DISPLAY_NAME);
    }

    #[test]
    fn unknown_methods_and_bad_params_get_proper_errors() {
        let d = daemon_in(&temp_dir("errors"), &SimBackend::new());
        assert_eq!(
            call(&d, "nope", Value::Null).error.unwrap().code,
            RpcError::METHOD_NOT_FOUND
        );
        assert_eq!(
            call(&d, "graph.set", json!({"nodes": 3}))
                .error
                .unwrap()
                .code,
            RpcError::INVALID_PARAMS
        );
    }

    #[test]
    fn notifications_get_no_reply_and_wrong_versions_are_refused() {
        let d = daemon_in(&temp_dir("notif"), &SimBackend::new());
        let note = Request {
            jsonrpc: "2.0".into(),
            id: None,
            method: "system.info".into(),
            params: Value::Null,
        };
        assert!(d.handle(note).is_none());
        let bad = Request {
            jsonrpc: "1.0".into(),
            id: Some(Id::Number(1)),
            method: "system.info".into(),
            params: Value::Null,
        };
        assert_eq!(
            d.handle(bad).unwrap().error.unwrap().code,
            RpcError::INVALID_REQUEST
        );
    }

    #[test]
    fn raw_text_handles_garbage_batches_and_single_requests() {
        let d = daemon_in(&temp_dir("text"), &SimBackend::new());
        let parse: Response = serde_json::from_str(&d.handle_text("{ nope").unwrap()).unwrap();
        assert_eq!(parse.error.unwrap().code, RpcError::PARSE);
        let single: Response = serde_json::from_str(
            &d.handle_text(r#"{"jsonrpc":"2.0","id":5,"method":"graph.get"}"#)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(single.id, Some(Id::Number(5)));
        let batch: Vec<Response> = serde_json::from_str(
            &d.handle_text(
                r#"[{"jsonrpc":"2.0","id":1,"method":"graph.get"},{"jsonrpc":"2.0","method":"graph.get"},{"jsonrpc":"2.0","id":2,"method":"nope"}]"#,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            batch.len(),
            2,
            "the notification of the batch gets no answer"
        );
        assert!(d.handle_text("[]").unwrap().contains("empty batch"));
    }

    #[test]
    fn an_invalid_graph_is_rejected_with_every_problem_listed() {
        let d = daemon_in(&temp_dir("invalid"), &SimBackend::new());
        let bad = GraphSpec {
            nodes: vec![
                NodeSpec {
                    id: "a b".into(),
                    display: None,
                    kind: NodeKind::Mixer,
                },
                NodeSpec {
                    id: "a b".into(),
                    display: None,
                    kind: NodeKind::Mixer,
                },
            ],
            edges: vec![],
        };
        let r = call(&d, "graph.set", serde_json::to_value(&bad).unwrap());
        let e = r.error.unwrap();
        assert_eq!(e.code, RpcError::VALIDATION);
        assert!(e.data.unwrap().as_array().unwrap().len() >= 2);
        let v: ValidateResult = serde_json::from_value(
            call(&d, "graph.validate", serde_json::to_value(&bad).unwrap())
                .result
                .unwrap(),
        )
        .unwrap();
        assert!(!v.ok && v.errors.len() >= 2);
        let ok: ValidateResult = serde_json::from_value(
            call(&d, "graph.validate", serde_json::to_value(chain()).unwrap())
                .result
                .unwrap(),
        )
        .unwrap();
        assert!(ok.ok);
    }

    #[test]
    fn applying_a_graph_runs_it_reports_status_and_publishes_events() {
        let sim = SimBackend::new();
        sim.add_app(1, "a.exe", SimSource::tone(1000.0));
        let dir = temp_dir("apply");
        let d = daemon_in(&dir, &sim);
        let events = Arc::new(Mutex::new(vec![]));
        let e2 = events.clone();
        d.on_event(Box::new(move |n| e2.lock().unwrap().push(n.method.clone())));

        assert!(
            call(&d, "graph.set", serde_json::to_value(chain()).unwrap())
                .error
                .is_none()
        );
        std::thread::sleep(Duration::from_millis(700));
        d.tick();
        let st: EngineStatus =
            serde_json::from_value(call(&d, "engine.status", Value::Null).result.unwrap()).unwrap();
        assert!(st.running);
        assert!(
            st.nodes.iter().all(|n| n.state == NodeState::Running),
            "{st:?}"
        );
        assert!(st.nodes.iter().any(|n| n.peak > 0.1), "audio flows: {st:?}");
        let got: GraphSpec =
            serde_json::from_value(call(&d, "graph.get", Value::Null).result.unwrap()).unwrap();
        assert_eq!(got, chain());
        let names = events.lock().unwrap().clone();
        assert!(
            names.contains(&EVENT_GRAPH.to_string()) && names.contains(&EVENT_STATUS.to_string()),
            "{names:?}"
        );

        assert!(call(&d, "engine.stop", Value::Null).error.is_none());
        let st: EngineStatus =
            serde_json::from_value(call(&d, "engine.status", Value::Null).result.unwrap()).unwrap();
        assert!(!st.running);
        drop(d);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_graph_comes_back_after_a_restart_unless_it_was_stopped() {
        let sim = SimBackend::new();
        sim.add_app(1, "a.exe", SimSource::tone(1000.0));
        let dir = temp_dir("restore");
        {
            let d = daemon_in(&dir, &sim);
            assert!(
                call(&d, "graph.set", serde_json::to_value(chain()).unwrap())
                    .error
                    .is_none()
            );
        }
        {
            let d = daemon_in(&dir, &sim);
            d.restore();
            std::thread::sleep(Duration::from_millis(500));
            let st = d.tick();
            assert!(
                st.running && st.nodes.len() == 2,
                "restarted with the saved graph: {st:?}"
            );
            call(&d, "engine.stop", Value::Null);
        }
        {
            let d = daemon_in(&dir, &sim);
            d.restore();
            std::thread::sleep(Duration::from_millis(300));
            assert!(!d.tick().running, "a stopped engine stays stopped");
            let got: GraphSpec =
                serde_json::from_value(call(&d, "graph.get", Value::Null).result.unwrap()).unwrap();
            assert_eq!(got, chain(), "the graph itself is kept");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn playing_into_a_virtual_speaker_that_the_same_graph_records_is_refused() {
        let sim = SimBackend::new();
        // how Windows names the virtual speaker
        sim.add_render_device("Speakers (Spk)", crate::types::AudioFormat::ENGINE, 0.0);
        sim.add_render_device("Real Speakers", crate::types::AudioFormat::ENGINE, 0.0);
        let d = daemon_in(&temp_dir("feedback-speaker"), &sim);
        let graph = |target: &str| GraphSpec {
            nodes: vec![
                NodeSpec {
                    id: "in".into(),
                    display: None,
                    kind: NodeKind::VirtualSpeaker { name: "Spk".into() },
                },
                NodeSpec {
                    id: "out".into(),
                    display: None,
                    kind: NodeKind::DeviceRender {
                        device: DeviceRef::Name(target.into()),
                    },
                },
            ],
            edges: vec![EdgeSpec {
                from: PortRef::new("in", "out"),
                to: PortRef::new("out", "in"),
            }],
        };
        let r = call(
            &d,
            "graph.set",
            serde_json::to_value(graph("Speakers (Spk)")).unwrap(),
        );
        let e = r.error.expect("refused");
        assert_eq!(e.code, RpcError::VALIDATION);
        assert!(
            e.message.contains("feedback") && e.message.contains("\"out\""),
            "{}",
            e.message
        );
        // playing the speaker's sound on an ordinary output is the whole point
        let r = call(
            &d,
            "graph.set",
            serde_json::to_value(graph("Real")).unwrap(),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
    }

    #[test]
    fn recording_a_virtual_microphone_of_the_same_graph_is_refused() {
        let sim = SimBackend::new();
        sim.add_app(1, "a.exe", SimSource::tone(300.0));
        sim.add_capture_device("Microphone (Mic)", SimSource::tone(300.0)); // how Windows names the virtual microphone
        sim.add_capture_device("Real Microphone", SimSource::tone(300.0));
        let d = daemon_in(&temp_dir("feedback"), &sim);
        let mut g = chain(); // app -> virtual mic named "Mic"
        g.nodes.push(NodeSpec {
            id: "rec".into(),
            display: None,
            kind: NodeKind::DeviceCapture {
                device: DeviceRef::Name("Microphone (Mic)".into()),
            },
        });
        g.nodes.push(NodeSpec {
            id: "spk".into(),
            display: None,
            kind: NodeKind::DeviceRender {
                device: DeviceRef::Default,
            },
        });
        g.edges.push(EdgeSpec {
            from: PortRef::new("rec", "out"),
            to: PortRef::new("spk", "in"),
        });
        let r = call(&d, "graph.set", serde_json::to_value(&g).unwrap());
        let e = r.error.expect("refused");
        assert_eq!(e.code, RpcError::VALIDATION);
        assert!(
            e.message.contains("feedback") && e.message.contains("\"rec\""),
            "{}",
            e.message
        );
        // recording an ordinary microphone is fine
        let mut ok = chain();
        ok.nodes.push(NodeSpec {
            id: "rec".into(),
            display: None,
            kind: NodeKind::DeviceCapture {
                device: DeviceRef::Name("Real".into()),
            },
        });
        ok.nodes.push(NodeSpec {
            id: "spk".into(),
            display: None,
            kind: NodeKind::DeviceRender {
                device: DeviceRef::Default,
            },
        });
        ok.edges.push(EdgeSpec {
            from: PortRef::new("rec", "out"),
            to: PortRef::new("spk", "in"),
        });
        let v: ValidateResult = serde_json::from_value(
            call(&d, "graph.validate", serde_json::to_value(&ok).unwrap())
                .result
                .unwrap(),
        )
        .unwrap();
        assert!(v.ok, "{:?}", v.errors);
    }

    #[test]
    fn the_layout_is_saved_and_restored() {
        let dir = temp_dir("layout");
        let sim = SimBackend::new();
        {
            let d = daemon_in(&dir, &sim);
            let mut l = crate::protocol::methods::Layout::new();
            let mut mixer = NodeLayout::at(7.0, 8.0);
            mixer.inputs = Some(4);
            mixer.slots.insert("a.out->mix.in".into(), 3);
            l.insert("mix".into(), mixer);
            l.insert("app".into(), NodeLayout::at(5.0, 6.0));
            assert!(call(&d, "layout.set", serde_json::to_value(&l).unwrap())
                .error
                .is_none());
        }
        let d = daemon_in(&dir, &sim);
        let got: crate::protocol::methods::Layout =
            serde_json::from_value(call(&d, "layout.get", Value::Null).result.unwrap()).unwrap();
        assert_eq!(got["app"], NodeLayout::at(5.0, 6.0));
        assert_eq!(
            got["mix"].inputs,
            Some(4),
            "the input points survive a restart"
        );
        assert_eq!(got["mix"].slots["a.out->mix.in"], 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn devices_and_apps_are_listed() {
        let sim = SimBackend::new();
        sim.add_app(9, "x.exe", SimSource::tone(300.0));
        sim.add_capture_device("mic", SimSource::tone(300.0));
        let d = daemon_in(&temp_dir("lists"), &sim);
        let apps: Vec<crate::hal::AppInfo> =
            serde_json::from_value(call(&d, "apps.list", Value::Null).result.unwrap()).unwrap();
        assert_eq!(apps[0].pid, 9);
        let devs: Vec<crate::hal::DeviceInfo> =
            serde_json::from_value(call(&d, "devices.list", Value::Null).result.unwrap()).unwrap();
        assert!(devs.iter().any(|x| x.name == "mic"));
    }

    // ---- cleaning up the old devices the system remembers

    use crate::elevation::{ElevateError, Elevator};
    use crate::types::{CleanOutcome, StaleDevice, StaleDevices};

    /// A provider that remembers old devices (and forgets them when told to, as an elevated process would).
    struct Remembers {
        stale: Mutex<Vec<StaleDevice>>,
    }

    impl VirtualDeviceProvider for Remembers {
        fn name(&self) -> &str {
            "remembers"
        }
        fn available(&self) -> Result<(), HalError> {
            Ok(())
        }
        fn create_microphone(
            &self,
            _name: &str,
        ) -> Result<Box<dyn crate::hal::RenderStream>, HalError> {
            Err(HalError::Unsupported("test".into()))
        }
        fn stale_devices(&self, kept: &KeptDevices) -> Result<Vec<StaleDevice>, HalError> {
            // what the graph uses is never stale
            Ok(self
                .stale
                .lock()
                .unwrap()
                .iter()
                .filter(|s| !kept.microphones.contains(&s.name))
                .cloned()
                .collect())
        }
        fn forget_stale_devices(&self, kept: &KeptDevices) -> Result<(), HalError> {
            self.stale
                .lock()
                .unwrap()
                .retain(|s| kept.microphones.contains(&s.name));
            Ok(())
        }
    }

    /// Rights that are there or not; a helper started without them is refused or accepted as the test says.
    struct Rights {
        elevated: bool,
        user_accepts: bool,
        helper_runs: Mutex<u32>,
        provider: Arc<Remembers>,
    }

    impl Elevator for Rights {
        fn is_elevated(&self) -> bool {
            self.elevated
        }
        fn run(
            &self,
            _exe: &std::path::Path,
            args: &[String],
            _timeout: std::time::Duration,
        ) -> Result<i32, ElevateError> {
            *self.helper_runs.lock().unwrap() += 1;
            assert!(
                args.contains(&"--clean-devices".to_string())
                    && args.contains(&"--elevated-helper".to_string())
            );
            if !self.user_accepts {
                return Err(ElevateError::Declined);
            }
            self.provider.stale.lock().unwrap().clear();
            Ok(0)
        }
    }

    fn remembering_daemon(
        names: &[&str],
        elevated: bool,
        user_accepts: bool,
    ) -> (Daemon, Arc<Rights>, std::path::PathBuf) {
        let dir = temp_dir("clean");
        let provider = Arc::new(Remembers {
            stale: Mutex::new(
                names
                    .iter()
                    .map(|n| StaleDevice { name: (*n).into() })
                    .collect(),
            ),
        });
        let rights = Arc::new(Rights {
            elevated,
            user_accepts,
            helper_runs: Mutex::new(0),
            provider: provider.clone(),
        });
        let d = Daemon::new(
            Arc::new(SimBackend::new()) as Arc<dyn Backend>,
            Some(provider as Arc<dyn VirtualDeviceProvider>),
            EngineConfig::default(),
            Store::new(&dir),
        )
        .with_elevator(rights.clone());
        (d, rights, dir)
    }

    #[test]
    fn the_old_devices_are_listed_with_whether_cleaning_would_ask_for_rights() {
        let (d, _, dir) = remembering_daemon(&["Old", "Older"], false, true);
        let r: StaleDevices =
            serde_json::from_value(call(&d, "devices.stale", Value::Null).result.unwrap()).unwrap();
        assert!(r.supported && !r.elevated);
        assert_eq!(r.stale.len(), 2);
        let (d, _, dir2) = remembering_daemon(&["Old"], true, true);
        let r: StaleDevices =
            serde_json::from_value(call(&d, "devices.stale", Value::Null).result.unwrap()).unwrap();
        assert!(
            r.elevated,
            "the developer who runs the daemon with rights is never asked"
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
    }

    #[test]
    fn a_system_with_nothing_to_clean_says_so_and_the_panel_shows_nothing() {
        // the simulated system remembers no devices
        let dir = temp_dir("clean-none");
        let d = daemon_in(&dir, &SimBackend::new());
        let r: StaleDevices =
            serde_json::from_value(call(&d, "devices.stale", Value::Null).result.unwrap()).unwrap();
        assert_eq!(r, StaleDevices::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleaning_with_the_rights_asks_for_nothing_and_without_them_asks_once() {
        let (d, rights, dir) = remembering_daemon(&["Old"], true, true);
        let done: CleanOutcome =
            serde_json::from_value(call(&d, "devices.clean", Value::Null).result.unwrap()).unwrap();
        assert_eq!(
            done,
            CleanOutcome {
                removed: 1,
                remaining: 0,
                asked_for_rights: false
            }
        );
        assert_eq!(
            *rights.helper_runs.lock().unwrap(),
            0,
            "no helper, no window"
        );

        let (d, rights, dir2) = remembering_daemon(&["Old", "Older"], false, true);
        let done: CleanOutcome =
            serde_json::from_value(call(&d, "devices.clean", Value::Null).result.unwrap()).unwrap();
        assert_eq!(
            done,
            CleanOutcome {
                removed: 2,
                remaining: 0,
                asked_for_rights: true
            }
        );
        assert_eq!(*rights.helper_runs.lock().unwrap(), 1);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
    }

    #[test]
    fn a_refused_confirmation_cancels_and_removes_nothing() {
        let (d, rights, dir) = remembering_daemon(&["Old"], false, false);
        let reply = call(&d, "devices.clean", Value::Null);
        assert_eq!(reply.error.expect("an error").code, RpcError::CANCELLED);
        assert_eq!(rights.provider.stale.lock().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_the_saved_graph_uses_is_never_cleaned() {
        let (d, _, dir) = remembering_daemon(&["Old", "Mine"], true, true);
        let mut g = GraphSpec::default();
        g.nodes.push(NodeSpec::new(
            "m",
            NodeKind::VirtualMic {
                name: "Mine".into(),
            },
        ));
        assert!(call(&d, "graph.set", serde_json::to_value(&g).unwrap())
            .error
            .is_none());
        let r: StaleDevices =
            serde_json::from_value(call(&d, "devices.stale", Value::Null).result.unwrap()).unwrap();
        assert_eq!(r.stale, vec![StaleDevice { name: "Old".into() }]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- soundboards, through the protocol, the graph and the engine

    /// A 16-bit mono WAV of one second of a 1000 Hz tone at 44.1 kHz (what a person might import).
    fn write_tone_wav(path: &std::path::Path) {
        let samples: Vec<i16> = (0..44_100)
            .map(|i| {
                ((i as f32 * 1000.0 * std::f32::consts::TAU / 44_100.0).sin() * 12_000.0) as i16
            })
            .collect();
        let data = (samples.len() * 2) as u32;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&44_100u32.to_le_bytes());
        b.extend_from_slice(&88_200u32.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data.to_le_bytes());
        for x in samples {
            b.extend_from_slice(&x.to_le_bytes());
        }
        std::fs::write(path, b).unwrap();
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn a_soundboard_is_made_played_through_the_graph_and_its_memory_goes_when_it_leaves_the_graph()
    {
        let dir = temp_dir("soundboard");
        let sim = SimBackend::new();
        let d = daemon_in(&dir, &sim);
        let ok = |r: Response| -> Value {
            assert!(r.error.is_none(), "{:?}", r.error);
            r.result.unwrap_or(Value::Null)
        };

        // import a file that is not in the stored format (44.1 kHz): it is converted once
        std::fs::create_dir_all(&dir).unwrap();
        write_tone_wav(&dir.join("tone.wav"));
        let info: crate::types::SoundInfo = serde_json::from_value(ok(call(
            &d,
            "sounds.import",
            json!({"path": dir.join("tone.wav").display().to_string(), "name": "A tone"}),
        )))
        .unwrap();
        assert_eq!((info.channels, info.name.as_str()), (1, "A tone"));
        assert!(
            (info.frames as i64 - 48_000).abs() <= 8,
            "{} frames",
            info.frames
        );

        // a soundboard with a pad that plays it
        let boards = json!([{ "id": "b1", "name": "Memes", "pads": [{"id": "p1", "name": "Tone", "sound": info.id}] }]);
        ok(call(&d, "soundboards.set", boards));
        let library: crate::types::SoundLibrary =
            serde_json::from_value(ok(call(&d, "soundboards.get", Value::Null))).unwrap();
        assert_eq!((library.sounds.len(), library.boards.len()), (1, 1));
        // a pad cannot play a sound that does not exist
        let bad = call(
            &d,
            "soundboards.set",
            json!([{"id": "b", "name": "x", "pads": [{"id": "p", "name": "p", "sound": "nope"}]}]),
        );
        assert_eq!(bad.error.unwrap().code, RpcError::INVALID_PARAMS);

        // nothing is reserved yet, and a pad cannot play: its soundboard is not in a running graph
        let status = |d: &Daemon| -> crate::types::SoundStatus {
            serde_json::from_value(ok(call(d, "soundboards.status", Value::Null))).unwrap()
        };
        assert!(!status(&d).active && status(&d).cache_allocated_bytes == 0);
        let early = call(&d, "soundboard.play", json!({"board": "b1", "pad": "p1"}));
        assert_eq!(early.error.unwrap().code, RpcError::UNAVAILABLE);

        // a block for a soundboard that does not exist is refused with a clear message
        let unknown = GraphSpec {
            nodes: vec![NodeSpec::new(
                "sb",
                NodeKind::Soundboard {
                    board: "ghost".into(),
                },
            )],
            edges: vec![],
        };
        let refused = call(&d, "graph.set", serde_json::to_value(&unknown).unwrap());
        let refusal = refused.error.expect("refused");
        assert_eq!(refusal.code, RpcError::VALIDATION);
        assert!(
            refusal
                .data
                .expect("problems")
                .to_string()
                .contains("does not exist"),
            "the reason is stated"
        );

        // the real graph: soundboard -> virtual microphone
        let graph = GraphSpec {
            nodes: vec![
                NodeSpec::new("sb", NodeKind::Soundboard { board: "b1".into() }),
                NodeSpec::new("mic", NodeKind::VirtualMic { name: "SB".into() }),
            ],
            edges: vec![EdgeSpec {
                from: PortRef::new("sb", "out"),
                to: PortRef::new("mic", "in"),
            }],
        };
        ok(call(&d, "graph.set", serde_json::to_value(&graph).unwrap()));
        std::thread::sleep(Duration::from_millis(600));
        let s = status(&d);
        assert!(
            s.active && s.active_boards == vec!["b1".to_string()],
            "{s:?}"
        );
        ok(call(
            &d,
            "soundboard.play",
            json!({"board": "b1", "pad": "p1"}),
        ));
        std::thread::sleep(Duration::from_millis(300));
        let playing: Vec<String> =
            serde_json::from_value(ok(call(&d, "soundboard.playing", json!({"board": "b1"}))))
                .unwrap();
        assert_eq!(playing, vec!["p1".to_string()]);
        std::thread::sleep(Duration::from_millis(1700));
        let recorded = sim
            .recording_of("virtual:SB")
            .expect("the virtual microphone")
            .lock()
            .unwrap()
            .clone();
        let left: Vec<f32> = recorded.chunks(2).map(|f| f[0]).collect();
        let loud = left.chunks(4_800).filter(|c| rms(c) > 0.15).count();
        let quiet = left.chunks(4_800).filter(|c| rms(c) < 0.001).count();
        assert!(
            loud >= 6,
            "the tone was heard for about a second: {loud} loud tenths of a second"
        );
        assert!(
            quiet >= 6,
            "and before and after it there is silence: {quiet}"
        );
        let s = status(&d);
        assert!(s.sounds_loaded == 1 && s.cache_used_bytes > 0, "{s:?}");

        // the soundboard leaves the graph: the memory is given back
        ok(call(
            &d,
            "graph.set",
            serde_json::to_value(GraphSpec::default()).unwrap(),
        ));
        std::thread::sleep(Duration::from_millis(400));
        let s = status(&d);
        assert!(
            !s.active && s.cache_allocated_bytes == 0 && s.sounds_loaded == 0,
            "{s:?}"
        );
        // the settings can be read and changed
        let settings: crate::types::Settings =
            serde_json::from_value(ok(call(&d, "settings.get", Value::Null))).unwrap();
        assert_eq!(settings.sound_cache_mb, 256);
        ok(call(&d, "settings.set", json!({"sound_cache_mb": 64})));
        assert_eq!(status(&d).cache_limit_bytes, 64 * 1024 * 1024);
        drop(d);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pad_shortcut_plays_the_pad_while_its_soundboard_runs_and_not_otherwise() {
        let dir = temp_dir("padkeys");
        let sim = SimBackend::new();
        let d = daemon_in(&dir, &sim);
        let ok = |r: Response| -> Value {
            assert!(r.error.is_none(), "{:?}", r.error);
            r.result.unwrap_or(Value::Null)
        };
        std::fs::create_dir_all(&dir).unwrap();
        write_tone_wav(&dir.join("tone.wav"));
        let info: crate::types::SoundInfo = serde_json::from_value(ok(call(
            &d,
            "sounds.import",
            json!({"path": dir.join("tone.wav").display().to_string()}),
        )))
        .unwrap();

        // a shortcut that is not one is refused; a good one is saved the way it is normally written
        let bad = call(
            &d,
            "soundboards.set",
            json!([{"id": "b1", "name": "x", "pads": [{"id": "p1", "name": "p", "sound": info.id, "key": "Ctrl+Nope"}]}]),
        );
        assert_eq!(bad.error.unwrap().code, RpcError::INVALID_PARAMS);
        ok(call(
            &d,
            "soundboards.set",
            json!([{"id": "b1", "name": "x", "pads": [{"id": "p1", "name": "p", "sound": info.id, "key": "ctrl+alt+1"}]}]),
        ));
        let library: crate::types::SoundLibrary =
            serde_json::from_value(ok(call(&d, "soundboards.get", Value::Null))).unwrap();
        assert_eq!(library.boards[0].pads[0].key.as_deref(), Some("Ctrl+Alt+1"));

        // the soundboard is not in a running graph: the keys are not taken from other programs
        assert!(!sim.press("Ctrl+Alt+1"), "nothing listens yet");

        let graph = GraphSpec {
            nodes: vec![
                NodeSpec::new("sb", NodeKind::Soundboard { board: "b1".into() }),
                NodeSpec::new("mic", NodeKind::VirtualMic { name: "K".into() }),
            ],
            edges: vec![EdgeSpec {
                from: PortRef::new("sb", "out"),
                to: PortRef::new("mic", "in"),
            }],
        };
        ok(call(&d, "graph.set", serde_json::to_value(&graph).unwrap()));
        std::thread::sleep(Duration::from_millis(600));
        assert!(sim.press("Ctrl+Alt+1"), "listening now");
        std::thread::sleep(Duration::from_millis(300));
        let playing: Vec<String> =
            serde_json::from_value(ok(call(&d, "soundboard.playing", json!({"board": "b1"}))))
                .unwrap();
        assert_eq!(
            playing,
            vec!["p1".to_string()],
            "the shortcut played the pad"
        );

        // changing the shortcut moves the registration
        ok(call(
            &d,
            "soundboards.set",
            json!([{"id": "b1", "name": "x", "pads": [{"id": "p1", "name": "p", "sound": info.id, "key": "F9"}]}]),
        ));
        assert!(!sim.press("Ctrl+Alt+1"), "the old keys are free again");
        assert!(sim.press("F9"));

        // stopping the engine gives the keys back
        ok(call(&d, "engine.stop", Value::Null));
        assert!(!sim.press("F9"));
        drop(d);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pad_shortcut_another_program_owns_is_reported_not_fatal() {
        let dir = temp_dir("padkeys-busy");
        let sim = SimBackend::new();
        let d = daemon_in(&dir, &sim);
        let ok = |r: Response| -> Value {
            assert!(r.error.is_none(), "{:?}", r.error);
            r.result.unwrap_or(Value::Null)
        };
        // somebody else already owns F7
        let _taken = sim
            .open_hotkey(
                &crate::types::KeyCombo::parse("F7").unwrap(),
                Box::new(|| {}),
            )
            .unwrap();
        ok(call(
            &d,
            "soundboards.set",
            json!([{"id": "b1", "name": "x", "pads": [{"id": "p1", "name": "p", "key": "F7"}]}]),
        ));
        let graph = GraphSpec {
            nodes: vec![NodeSpec::new(
                "sb",
                NodeKind::Soundboard { board: "b1".into() },
            )],
            edges: vec![],
        };
        ok(call(&d, "graph.set", serde_json::to_value(&graph).unwrap()));
        std::thread::sleep(Duration::from_millis(400));
        let status: crate::types::SoundStatus =
            serde_json::from_value(ok(call(&d, "soundboards.status", Value::Null))).unwrap();
        assert_eq!(status.hotkey_problems.len(), 1, "{status:?}");
        assert_eq!(status.hotkey_problems[0].pad, "p1");
        assert!(status.hotkey_problems[0].message.contains("already used"));
        drop(d);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
