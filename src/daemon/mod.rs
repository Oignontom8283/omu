//! The daemon: the process that owns the audio engine and answers the control protocol.
//!
//! * [`Daemon`] is the transport-independent core: it dispatches JSON-RPC requests to the engine, remembers the
//!   graph and the editors' layout ([`store`]), and publishes events. It is plain synchronous Rust, so it is tested
//!   without any network.
//! * the network part (HTTP, WebSocket, the web panel) lives in the sibling modules and only forwards messages here.

pub mod store;

use crate::engine::{Engine, EngineConfig};
use crate::graph::validate;
use crate::hal::{Backend, HalError, VirtualDeviceProvider};
use crate::protocol::methods::{
    self as m, Method, ProviderInfo, SystemInfo, ValidateResult, EVENT_GRAPH, EVENT_STATUS,
};
use crate::protocol::{
    Id, Notification, Request, Response, RpcError, JSONRPC_VERSION, PROTOCOL_VERSION,
};
use crate::types::{EngineStatus, GraphSpec};
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
        }
    }

    /// Starts again what was running when the daemon last stopped.
    pub fn restore(&self) {
        let saved = lock(&self.state).saved.clone();
        if saved.running {
            if let Ok(valid) = validate(&saved.graph) {
                self.engine.apply(valid);
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
        let outcome = self.dispatch(&request.method, request.params);
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
            m::AppsList::NAME => result(self.backend.list_apps().map_err(hal_error)?),
            m::GraphGet::NAME => result(lock(&self.state).saved.graph.clone()),
            m::GraphValidate::NAME => {
                let graph = params::<m::GraphValidate>(p)?;
                result(match validate(&graph) {
                    Ok(_) => ValidateResult {
                        ok: true,
                        errors: vec![],
                    },
                    Err(errors) => ValidateResult {
                        ok: false,
                        errors: errors.iter().map(|e| e.to_string()).collect(),
                    },
                })
            }
            m::GraphSet::NAME => {
                let graph = params::<m::GraphSet>(p)?;
                self.set_graph(graph)?;
                result(())
            }
            m::EngineStatusMethod::NAME => result(lock(&self.state).status.clone()),
            m::EngineStop::NAME => {
                self.engine.stop();
                let mut st = lock(&self.state);
                st.saved.running = false;
                let _ = self.store.save_state(&st.saved);
                st.status = EngineStatus::default();
                result(())
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
        SystemInfo {
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

    fn set_graph(&self, graph: GraphSpec) -> Result<(), RpcError> {
        let valid = validate(&graph).map_err(|errors| {
            let list: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
            RpcError::new(
                RpcError::VALIDATION,
                format!("the graph is invalid: {}", list.join("; ")),
            )
            .with_data(serde_json::json!(list))
        })?;
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
    use crate::protocol::methods::Position;
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
                    kind: NodeKind::AppCapture {
                        target: AppTarget::ProcessName("a.exe".into()),
                    },
                },
                NodeSpec {
                    id: "mic".into(),
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
                    kind: NodeKind::Mixer,
                },
                NodeSpec {
                    id: "a b".into(),
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
    fn the_layout_is_saved_and_restored() {
        let dir = temp_dir("layout");
        let sim = SimBackend::new();
        {
            let d = daemon_in(&dir, &sim);
            let mut l = crate::protocol::methods::Layout::new();
            l.insert("app".into(), Position { x: 5.0, y: 6.0 });
            assert!(call(&d, "layout.set", serde_json::to_value(&l).unwrap())
                .error
                .is_none());
        }
        let d = daemon_in(&dir, &sim);
        let got: crate::protocol::methods::Layout =
            serde_json::from_value(call(&d, "layout.get", Value::Null).result.unwrap()).unwrap();
        assert_eq!(got["app"], Position { x: 5.0, y: 6.0 });
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
}
