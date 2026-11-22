//! The daemon: the process that owns the audio engine and answers the control protocol.
//!
//! * [`Daemon`] is the transport-independent core: it dispatches JSON-RPC requests to the engine, remembers the
//!   graph and the editors' layout ([`store`]), and publishes events. It is plain synchronous Rust, so it is tested
//!   without any network.
//! * the network part (HTTP, WebSocket, the web panel) lives in the sibling modules and only forwards messages here.

pub mod auth;
pub mod instance;
pub mod launch;
pub mod open;
pub mod server;
pub mod store;
#[cfg(feature = "tray")]
pub mod tray_host;
pub mod web;

use omu_engine::{Engine, EngineConfig};
use omu_graph::validate;
use omu_hal::Direction;
use omu_hal::{Backend, HalError, VirtualDeviceProvider};
use omu_protocol::methods::{
    self as m, Method, ProviderInfo, SystemInfo, ValidateResult, EVENT_GRAPH, EVENT_STATUS,
};
use omu_protocol::{
    Id, Notification, Request, Response, RpcError, JSONRPC_VERSION, PROTOCOL_VERSION,
};
use omu_types::{DeviceRef, Diagnostic, EngineStatus, GraphSpec, KeptDevices, NodeKind, Severity};
use omu_virtual_devices::clean;
use serde_json::Value;
use std::sync::{Arc, Mutex, MutexGuard};
use store::{SavedState, Store};

/// Receives the events the daemon publishes.
pub type EventListener = Box<dyn Fn(&Notification) + Send + Sync>;

struct State {
    saved: SavedState,
    layout: omu_protocol::methods::Layout,
    status: EngineStatus,
}

pub struct Daemon {
    engine: Engine,
    backend: Arc<dyn Backend>,
    virtual_devices: Option<Arc<dyn VirtualDeviceProvider>>,
    store: Store,
    state: Mutex<State>,
    listeners: Mutex<Vec<EventListener>>,
    previous: Mutex<omu_diagnostics::PreviousSession>,
    /// Woken when a client asks the daemon to stop (`system.shutdown`).
    stop: Arc<tokio::sync::Notify>,
    /// Administrator rights, for cleaning up old devices.
    elevator: Arc<dyn omu_desktop::elevation::Elevator>,
    /// The soundboards: their memory, their sounds and what they play.
    sounds: Arc<omu_soundboard::SoundEngine>,
    /// The names of the web inputs and outputs, and the connections to them.
    web: omu_net::web::Hub,
    /// What changes the running graph (apply, start, stop, restore) takes one at a time, from the engine to the file and the shortcuts:
    /// the engine, the saved state and the pad shortcuts then always describe the same graph. Taken before `state`, never after.
    ops: Mutex<()>,
    /// When the saved graph was started again at launch, and how long the daemon must then live for that to count as having worked.
    restoring: Mutex<Option<std::time::Instant>>,
    restore_settles: std::time::Duration,
    /// Saving the layout: one at a time, so that what is on the disk is what the memory holds.
    layout_gate: Mutex<()>,
    /// What the secure server says about itself (written by the task that runs it).
    remote: Mutex<omu_types::RemoteInfo>,
    /// The key a page on another device carries: it only allows the web sound routes.
    guest_token: Mutex<String>,
    /// The keyboard shortcuts that play pads.
    pad_hotkeys: omu_soundboard::hotkeys::PadHotkeys,
    /// What is told when the settings are saved (the notification-area icon shows or hides itself).
    settings_listeners: Mutex<Vec<SettingsListener>>,
    /// Whether saving the settings also updates the startup list (a real daemon does; tests and demos do not).
    manages_autostart: std::sync::atomic::AtomicBool,
}

/// A function told when the settings are saved.
type SettingsListener = Box<dyn Fn(&omu_types::Settings) + Send + Sync>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn hal_error(e: HalError) -> RpcError {
    match e {
        HalError::Unsupported(_) => RpcError::new(RpcError::UNAVAILABLE, e.to_string()),
        _ => RpcError::internal(e.to_string()),
    }
}

pub(crate) fn sound_error(e: omu_soundboard::SoundError) -> RpcError {
    use omu_soundboard::SoundError as E;
    match &e {
        E::UnknownBoard(_) | E::UnknownPad(_) | E::NoSound | E::UnknownSound(_) => {
            RpcError::invalid_params(e.to_string())
        }
        E::NotInGraph(_) | E::NotActive | E::PoolFull(_) | E::TooManyStreams => {
            RpcError::new(RpcError::UNAVAILABLE, e.to_string())
        }
        E::ModuleNeeded(_) => RpcError::new(RpcError::MODULE_NEEDED, e.to_string())
            .with_data(serde_json::json!("ffmpeg")),
        E::Io(_) => RpcError::internal(e.to_string()),
    }
}

/// Decodes the parameters of method `M`.
fn params<M: Method>(value: Value) -> Result<M::Params, RpcError> {
    serde_json::from_value(value).map_err(|e| RpcError::invalid_params(format!("{}: {e}", M::NAME)))
}

/// The file that says the saved graph is being started again (see `Daemon::restore`).
const RESTORING_MARKER: &str = "restoring";

/// The event that tells the clients about something that went wrong without being the failure of a request.
pub const EVENT_WARNING: &str = "daemon.warning";

fn result<T: serde::Serialize>(v: T) -> Result<Value, RpcError> {
    serde_json::to_value(v).map_err(|e| RpcError::internal(e.to_string()))
}

impl Drop for Daemon {
    /// A daemon that ends properly was not brought down by the graph it restored: the next start may restore it again.
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.store.dir().join(RESTORING_MARKER));
    }
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
        let sounds = Arc::new(omu_soundboard::SoundEngine::new(store.dir()));
        sounds.modules().set_trust(omu_tls::Trust::from_settings(
            &sounds.settings(),
            store.dir(),
        ));
        let mut engine_config = engine_config;
        engine_config.sounds = Some(sounds.clone());
        let web = omu_net::web::Hub::new();
        engine_config.web = Some(web.clone());
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
            elevator: Arc::new(omu_desktop::elevation::SystemElevator),
            sounds,
            web,
            ops: Mutex::new(()),
            restoring: Mutex::new(None),
            restore_settles: std::time::Duration::from_secs(30),
            layout_gate: Mutex::new(()),
            remote: Mutex::default(),
            guest_token: Mutex::default(),
            pad_hotkeys: Default::default(),
            settings_listeners: Mutex::new(vec![]),
            manages_autostart: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Makes the system's registered pad shortcuts those of the pads whose soundboard is in the running graph. Called
    /// whenever that can change: a graph is applied or stopped, or the pads are edited.
    fn sync_pad_hotkeys(&self) {
        let wanted = {
            let st = lock(&self.state);
            if st.saved.running {
                omu_soundboard::hotkeys::wanted(
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

    /// Whose certificates the program believes, from the settings.
    fn trust(&self) -> omu_tls::Trust {
        omu_tls::Trust::from_settings(&self.sounds.settings(), self.store.dir())
    }

    /// The soundboards' engine (the panel's upload route imports through it).
    /// The program's data folder.
    pub fn data_dir(&self) -> &std::path::Path {
        self.store.dir()
    }

    /// Records what the secure server is doing (called by the task that runs it).
    pub fn set_remote(&self, info: omu_types::RemoteInfo) {
        *lock(&self.remote) = info;
    }

    pub fn set_guest_token(&self, token: String) {
        *lock(&self.guest_token) = token;
    }

    /// What the secure server is doing, with the pages to open on other devices.
    fn remote_info(&self) -> omu_types::RemoteInfo {
        let mut info = lock(&self.remote).clone();
        if info.enabled {
            let key = lock(&self.guest_token).clone();
            info.urls = omu_tls::local_names(&[])
                .into_iter()
                .filter(|n| {
                    n.parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_ipv4() && !ip.is_loopback())
                })
                // a network joined since the certificate was made: its address is not in it, and a browser would refuse the page
                .filter(|n| info.names.is_empty() || info.names.contains(n))
                .map(|ip| format!("https://{ip}:{}/remote.html#token={key}", info.port))
                .collect();
            if info.urls.is_empty() {
                // no network but this computer: a browser here can still use the page
                info.urls.push(format!(
                    "https://localhost:{}/remote.html#token={key}",
                    info.port
                ));
            }
        }
        info
    }

    /// The web inputs and outputs (the HTTP side hands the connection requests to it).
    pub fn web(&self) -> &omu_net::web::Hub {
        &self.web
    }

    pub fn sounds(&self) -> &Arc<omu_soundboard::SoundEngine> {
        &self.sounds
    }

    /// The same daemon asking for administrator rights another way (tests).
    pub fn with_elevator(mut self, elevator: Arc<dyn omu_desktop::elevation::Elevator>) -> Daemon {
        self.elevator = elevator;
        self
    }

    /// Completes when a client has asked the daemon to stop.
    pub fn stop_signal(&self) -> Arc<tokio::sync::Notify> {
        self.stop.clone()
    }

    /// Tells the daemon what the previous run left behind, for `system.info`.
    pub fn set_previous_session(&self, previous: omu_diagnostics::PreviousSession) {
        *lock(&self.previous) = previous;
    }

    /// Starts again what was running when the daemon last stopped.
    pub fn restore(&self) {
        let _ops = lock(&self.ops);
        let saved = lock(&self.state).saved.clone();
        // A graph that brings the daemon down as soon as it runs would bring it down again at every start (and at every logon, with the
        // startup entry). The file `restoring` is written before the graph is started again and removed once the daemon has lived long
        // enough, or ends properly: finding it means the last restore did not survive, so this time the graph is kept but not started.
        let marker = self.store.dir().join(RESTORING_MARKER);
        if saved.running && marker.exists() {
            let _ = std::fs::remove_file(&marker);
            tracing::warn!("the daemon did not survive starting the saved graph last time: it is kept, but not started again by itself");
            self.warn("The last start with the saved graph did not go well, so the graph was not started again by itself. Start it when you are ready.".into());
            return;
        }
        if saved.running {
            let _ = std::fs::create_dir_all(self.store.dir());
            let _ = std::fs::write(&marker, b"");
            *lock(&self.restoring) = Some(std::time::Instant::now());
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

    /// From now on the startup list follows the setting "start at logon" (and is made to match it now).
    pub fn manage_autostart(&self) {
        // a daemon with another data folder (a test, a second set of settings) must not rewrite or remove the entry of the real one
        let same = |a: &std::path::Path, b: &std::path::Path| match (
            std::fs::canonicalize(a),
            std::fs::canonicalize(b),
        ) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        };
        if !same(self.store.dir(), &omu_discovery::data_dir()) {
            tracing::info!(
                "another data folder than the usual one: the startup list is left as it is"
            );
            return;
        }
        self.manages_autostart
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let enabled = self.sounds.settings().start_at_logon;
        if let Err(e) = self.apply_autostart(enabled) {
            tracing::warn!("the startup list: {e}");
        }
    }

    fn apply_autostart(&self, enabled: bool) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        omu_desktop::autostart::apply(omu_desktop::autostart::ENTRY, enabled, &exe)
    }

    /// Registers a function called each time the settings are saved.
    pub fn on_settings(&self, listener: impl Fn(&omu_types::Settings) + Send + Sync + 'static) {
        lock(&self.settings_listeners).push(Box::new(listener));
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
        // the graph started again at launch has been running long enough: that start worked
        {
            let mut restoring = lock(&self.restoring);
            if restoring.is_some_and(|at| at.elapsed() >= self.restore_settles) {
                *restoring = None;
                let _ = std::fs::remove_file(self.store.dir().join(RESTORING_MARKER));
            }
        }
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
        let outcome = omu_diagnostics::contained(|| self.dispatch(&request.method, request.params))
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
            m::ModulesList::NAME => result(self.sounds.modules().list()),
            m::ModulesInstall::NAME => {
                let p = params::<m::ModulesInstall>(p)?;
                tracing::info!(module = %p.id, "installing a module");
                self.sounds
                    .modules()
                    .install(&p.id)
                    .map_err(RpcError::invalid_params)?;
                result(())
            }
            m::ModulesCancel::NAME => {
                let p = params::<m::ModulesCancel>(p)?;
                self.sounds
                    .modules()
                    .cancel(&p.id)
                    .map_err(RpcError::invalid_params)?;
                result(())
            }
            m::ModulesEnable::NAME => {
                let p = params::<m::ModulesEnable>(p)?;
                self.sounds
                    .modules()
                    .set_enabled(&p.id, p.enabled)
                    .map_err(RpcError::invalid_params)?;
                result(())
            }
            m::ModulesRemove::NAME => {
                let p = params::<m::ModulesRemove>(p)?;
                self.sounds
                    .modules()
                    .remove(&p.id)
                    .map_err(RpcError::invalid_params)?;
                result(())
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
            m::CertificatesList::NAME => result(omu_tls::report(&self.trust())),
            m::RemoteInfoMethod::NAME => result(self.remote_info()),
            m::SettingsSet::NAME => {
                let settings = params::<m::SettingsSet>(p)?;
                // saved first: the startup list then follows what is saved, never the other way round
                self.sounds
                    .set_settings(settings.clone())
                    .map_err(RpcError::internal)?;
                if self
                    .manages_autostart
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    self.apply_autostart(settings.start_at_logon)
                        .map_err(RpcError::internal)?;
                }
                self.sounds.modules().set_trust(self.trust());
                for l in lock(&self.settings_listeners).iter() {
                    l(&settings);
                }
                result(())
            }
            m::AppsList::NAME => result(self.backend.list_apps().map_err(hal_error)?),
            m::GraphGet::NAME => result(lock(&self.state).saved.graph.clone()),
            m::GraphValidate::NAME => {
                let graph = params::<m::GraphValidate>(p)?;
                let diagnostics = self.problems(&graph);
                result(ValidateResult {
                    ok: diagnostics.iter().all(|d| d.severity != Severity::Error),
                    errors: diagnostics.iter().map(|d| d.message.clone()).collect(),
                    diagnostics,
                })
            }
            m::GraphSet::NAME => {
                let graph = params::<m::GraphSet>(p)?;
                self.set_graph(graph)?;
                result(())
            }
            m::EngineStatusMethod::NAME => result(lock(&self.state).status.clone()),
            m::EngineLevels::NAME => {
                let p = params::<m::EngineLevels>(p)?;
                match self.engine.levels(&omu_types::NodeId(p.node.clone())) {
                    Some(db) => result(omu_types::Levels { cycle_ms: 10, db }),
                    None => Err(RpcError::new(
                        RpcError::UNAVAILABLE,
                        "this block is not running: apply the graph and start the engine",
                    )),
                }
            }
            m::HotkeysPauseMethod::NAME => {
                let p = params::<m::HotkeysPauseMethod>(p)?;
                self.engine.pause_hotkeys(p.paused);
                result(())
            }
            m::NodeControlMethod::NAME => {
                let p = params::<m::NodeControlMethod>(p)?;
                self.control_node(p)?;
                result(())
            }
            m::EngineStart::NAME => {
                let graph = lock(&self.state).saved.graph.clone();
                if graph.nodes.is_empty() {
                    return Err(RpcError::new(
                        RpcError::UNAVAILABLE,
                        "there is no graph to start: build one and apply it",
                    ));
                }
                tracing::info!("engine start requested");
                self.set_graph(graph)?;
                result(())
            }
            m::EngineStop::NAME => {
                tracing::info!("engine stop requested");
                let _ops = lock(&self.ops);
                self.engine.stop();
                let saved = {
                    let mut st = lock(&self.state);
                    st.saved.running = false;
                    st.status = EngineStatus::default();
                    st.saved.clone()
                };
                // the disk is written without holding what the status tick needs
                if let Err(e) = self.store.save_state(&saved) {
                    self.warn(format!(
                        "the engine stopped but that could not be saved: {e}"
                    ));
                }
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
                    .clamp(1, omu_diagnostics::ring::CAPACITY);
                result(match p.after {
                    Some(seq) => omu_diagnostics::ring::since(seq, limit),
                    None => omu_diagnostics::ring::recent(limit),
                })
            }
            m::LogsOpenFolder::NAME => {
                let dir = self.store.dir().join(omu_diagnostics::LOG_DIR);
                if !dir.is_dir() {
                    return Err(RpcError::internal("there is no folder of logs yet"));
                }
                open::open_target(&dir.display().to_string()).map_err(RpcError::internal)?;
                result(())
            }
            m::LayoutGet::NAME => result(lock(&self.state).layout.clone()),
            m::LayoutSet::NAME => {
                let layout = params::<m::LayoutSet>(p)?;
                let _gate = lock(&self.layout_gate);
                self.store
                    .save_layout(&layout)
                    .map_err(|e| RpcError::internal(format!("cannot save the layout: {e}")))?;
                lock(&self.state).layout = layout;
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
            name: omu_branding::DISPLAY_NAME.into(),
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
    fn problems(&self, graph: &GraphSpec) -> Vec<Diagnostic> {
        let mut out = omu_graph::diagnose(graph);
        // what is inside the groups counts too
        let graph = &omu_graph::expand(graph).unwrap_or_else(|_| graph.clone());
        out.extend(self.feedback_problems(graph));
        // a soundboard block must name a soundboard that exists
        for n in &graph.nodes {
            if let NodeKind::Soundboard { board, .. } = &n.kind {
                if self.sounds.library().board(board).is_none() {
                    out.push(
                        Diagnostic::error(
                            "missing_soundboard",
                            format!(
                                "block {:?} plays the soundboard {board:?}, which does not exist",
                                n.id.0
                            ),
                        )
                        .at(&n.id)
                        .hint(
                            "choose a soundboard that exists, or make one in the Soundboards page",
                        ),
                    );
                }
            }
        }
        out
    }

    fn feedback_problems(&self, graph: &GraphSpec) -> Vec<Diagnostic> {
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
    ) -> Vec<Diagnostic> {
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
                problems.push(
                    Diagnostic::error(
                        "feedback",
                        format!(
                            "node {:?} {verb} {own:?}: {consequence} (feedback)",
                            node.id.0
                        ),
                    )
                    .at(&node.id)
                    .hint("choose another device: this one is a virtual device of this program"),
                );
            }
        }
        problems
    }

    /// Tells the clients something that went wrong without being a failure of what they asked (the graph runs, but its file could not
    /// be written...).
    fn warn(&self, message: String) {
        tracing::warn!("{message}");
        self.publish(EVENT_WARNING, serde_json::json!({ "message": message }));
    }

    /// A button of the panel is pressed, a switch or a slider is set: the running graph hears it at its next cycle, and the new place
    /// of a switch or a slider is kept with the graph when `persist` says so (the graph is not applied again for that).
    fn control_node(&self, p: m::NodeControl) -> Result<(), RpcError> {
        use omu_types::NodeKind;
        let id = omu_types::NodeId(p.node.clone());
        let _ops = lock(&self.ops);
        let unavailable = || {
            RpcError::new(
                RpcError::UNAVAILABLE,
                "this block is not in a running graph: apply the graph and start the engine",
            )
        };
        let wrong = |what: &str| RpcError::invalid_params(format!("this block is not {what}"));
        match p.action {
            m::ControlAction::Press => {
                if !matches!(self.kind_of(&id), Some(NodeKind::Button)) {
                    return Err(wrong("a button"));
                }
                if !self.engine.press(&id) {
                    return Err(unavailable());
                }
            }
            m::ControlAction::Switch { on } => {
                if !matches!(self.kind_of(&id), Some(NodeKind::Switch { .. })) {
                    return Err(wrong("a switch"));
                }
                // the running graph hears it when there is one; the graph that is kept holds it either way
                self.engine.set_switch(&id, on);
                self.keep(&id, p.persist, |kind| {
                    if let NodeKind::Switch { on: held } = kind {
                        *held = on;
                    }
                });
            }
            m::ControlAction::Slide { value } => {
                if !value.is_finite() {
                    return Err(RpcError::invalid_params("the value is not a number"));
                }
                if !matches!(self.kind_of(&id), Some(NodeKind::Slider { .. })) {
                    return Err(wrong("a slider"));
                }
                self.engine.set_slider(&id, value);
                self.keep(&id, p.persist, |kind| {
                    if let NodeKind::Slider {
                        min,
                        max,
                        value: held,
                    } = kind
                    {
                        *held = value.clamp(*min, *max);
                    }
                });
            }
        }
        Ok(())
    }

    /// The kind of the block `id` in the graph that is kept.
    fn kind_of(&self, id: &omu_types::NodeId) -> Option<omu_types::NodeKind> {
        lock(&self.state)
            .saved
            .graph
            .nodes
            .iter()
            .find(|n| &n.id == id)
            .map(|n| n.kind.clone())
    }

    /// Changes the kind of the block `id` in the graph that is kept, and writes it to the disk when `persist`.
    fn keep(
        &self,
        id: &omu_types::NodeId,
        persist: bool,
        change: impl FnOnce(&mut omu_types::NodeKind),
    ) {
        let saved = {
            let mut st = lock(&self.state);
            if let Some(node) = st.saved.graph.nodes.iter_mut().find(|n| &n.id == id) {
                change(&mut node.kind);
            }
            st.saved.clone()
        };
        if persist {
            if let Err(e) = self.store.save_state(&saved) {
                self.warn(format!(
                    "the place of a switch or a slider could not be saved: {e}"
                ));
            }
        }
    }

    fn set_graph(&self, graph: GraphSpec) -> Result<(), RpcError> {
        let _ops = lock(&self.ops);
        self.set_graph_locked(graph)
    }

    fn set_graph_locked(&self, graph: GraphSpec) -> Result<(), RpcError> {
        let diagnostics = self.problems(&graph);
        let errors: Vec<&Diagnostic> = diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        if !errors.is_empty() {
            tracing::warn!(?errors, "a client sent an invalid graph; refused");
            let said: Vec<&str> = errors.iter().map(|d| d.message.as_str()).collect();
            return Err(RpcError::new(
                RpcError::VALIDATION,
                format!("the graph is invalid: {}", said.join("; ")),
            )
            .with_data(serde_json::json!(errors)));
        }
        tracing::info!(
            nodes = graph.nodes.len(),
            edges = graph.edges.len(),
            "graph set by a client"
        );
        omu_diagnostics::note(
            "graph",
            format!("{} nodes, {} edges", graph.nodes.len(), graph.edges.len()),
        );
        // `problems` found none, so validation succeeds
        let valid = validate(&graph)
            .map_err(|_| RpcError::internal("the graph changed under validation"))?;
        self.engine.apply(valid);
        let saved = SavedState {
            graph: graph.clone(),
            running: true,
        };
        lock(&self.state).saved = saved.clone();
        let written = self.store.save_state(&saved);
        // the graph runs whether or not its file could be written: its shortcuts and the news of it do not wait for the disk's answer
        self.sync_pad_hotkeys();
        if let Ok(v) = serde_json::to_value(&graph) {
            self.publish(EVENT_GRAPH, v);
        }
        if let Err(e) = written {
            self.warn(format!("the graph runs but could not be saved: {e}"));
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
    use omu_hal::sim::{SimBackend, SimSource};
    use omu_protocol::methods::NodeLayout;
    use omu_types::{AppTarget, EdgeSpec, NodeKind, NodeSpec, NodeState, PortRef};
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
                    inputs: Default::default(),
                    muted: false,
                    kind: NodeKind::AppCapture {
                        target: AppTarget::ProcessName("a.exe".into()),
                    },
                },
                NodeSpec {
                    id: "mic".into(),
                    display: None,
                    inputs: Default::default(),
                    muted: false,
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
        assert_eq!(info.name, omu_branding::DISPLAY_NAME);
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
    fn the_page_can_let_go_of_the_global_shortcuts_and_take_them_back() {
        let d = daemon_in(&temp_dir("hotkeys-pause"), &SimBackend::new());
        assert!(call(&d, "hotkeys.pause", json!({"paused": true}))
            .error
            .is_none());
        assert!(call(&d, "hotkeys.pause", json!({"paused": false}))
            .error
            .is_none());
        assert_eq!(
            call(&d, "hotkeys.pause", json!({"paused": "yes"}))
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
                    inputs: Default::default(),
                    muted: false,
                    kind: NodeKind::Mixer,
                },
                NodeSpec {
                    id: "a b".into(),
                    display: None,
                    inputs: Default::default(),
                    muted: false,
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

    /// Many clients at once (the panel, the command line, the notification-area menu): whatever the order, the engine, the saved state
    /// and the graph the daemon reports describe the same thing at the end, and nothing waits for ever.
    #[test]
    fn applying_starting_and_stopping_from_many_threads_leaves_the_engine_and_the_saved_state_in_agreement(
    ) {
        let sim = SimBackend::new();
        sim.add_app(1, "a.exe", SimSource::tone(1000.0));
        let dir = temp_dir("concurrent");
        let d = Arc::new(daemon_in(&dir, &sim));
        let a = chain();
        let mut b = chain();
        b.nodes.retain(|n| n.id.0 != "mic"); // a second, different graph: only the application
        b.edges.clear();
        let (graph_a, graph_b) = (
            serde_json::to_value(&a).unwrap(),
            serde_json::to_value(&b).unwrap(),
        );

        let (done_tx, done_rx) = std::sync::mpsc::channel();
        for t in 0..4u32 {
            let (d, done_tx) = (d.clone(), done_tx.clone());
            let (graph_a, graph_b) = (graph_a.clone(), graph_b.clone());
            std::thread::spawn(move || {
                for i in 0..25u32 {
                    // a mix that differs from thread to thread, without a random number generator
                    match (t * 7 + i * 3) % 6 {
                        0 => drop(call(&d, "graph.set", graph_a.clone())),
                        1 => drop(call(&d, "graph.set", graph_b.clone())),
                        2 => drop(call(&d, "engine.stop", Value::Null)),
                        3 => drop(call(&d, "engine.start", Value::Null)),
                        4 => drop(call(&d, "layout.set", json!({}))),
                        _ => drop((call(&d, "graph.get", Value::Null), d.tick())),
                    }
                }
                let _ = done_tx.send(());
            });
        }
        drop(done_tx);
        for _ in 0..4 {
            done_rx
                .recv_timeout(Duration::from_secs(60))
                .expect("no deadlock: every thread finishes");
        }

        let status = d.tick();
        let saved = Store::new(&dir).load_state();
        let reported: GraphSpec =
            serde_json::from_value(call(&d, "graph.get", Value::Null).result.unwrap()).unwrap();
        assert_eq!(
            saved.graph, reported,
            "the file holds the graph the daemon reports"
        );
        assert_eq!(
            saved.running, status.running,
            "the file says what the engine does"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The graph runs even when its file cannot be written: the answer is a success, the shortcuts and the news of the graph follow, and
    /// the clients are warned.
    #[test]
    fn a_graph_that_runs_but_cannot_be_saved_is_still_applied_and_announced_with_a_warning() {
        let sim = SimBackend::new();
        sim.add_app(1, "a.exe", SimSource::tone(1000.0));
        let dir = temp_dir("unsaveable");
        std::fs::create_dir_all(dir.join("state.json")).unwrap(); // a folder where the file must go: the save cannot succeed
        let d = daemon_in(&dir, &sim);
        let events = Arc::new(Mutex::new(vec![]));
        let e2 = events.clone();
        d.on_event(Box::new(move |n| {
            e2.lock()
                .unwrap()
                .push((n.method.clone(), n.params.clone()))
        }));

        let r = call(&d, "graph.set", serde_json::to_value(chain()).unwrap());
        assert!(r.error.is_none(), "{:?}", r.error);
        std::thread::sleep(Duration::from_millis(500));
        let status = d.tick();
        assert!(status.running, "the graph runs");
        let seen = events.lock().unwrap().clone();
        assert!(
            seen.iter().any(|(m, _)| m == EVENT_GRAPH),
            "the clients are told the graph changed"
        );
        let warning = seen
            .iter()
            .find(|(m, _)| m == EVENT_WARNING)
            .expect("and warned that it was not saved");
        assert!(
            warning.1.to_string().contains("could not be saved"),
            "{warning:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_button_a_switch_and_a_slider_are_pressed_and_set_while_the_graph_runs_and_kept_with_it() {
        let sim = SimBackend::new();
        let dir = temp_dir("panel-blocks");
        let ok = |r: Response| r.result.expect("a result");
        let d = daemon_in(&dir, &sim);
        let node = |id: &str, kind: NodeKind| NodeSpec {
            id: id.into(),
            display: None,
            inputs: Default::default(),
            muted: false,
            kind,
        };
        let graph = GraphSpec {
            nodes: vec![
                node("button", NodeKind::Button),
                node("switch", NodeKind::Switch { on: false }),
                node(
                    "slider",
                    NodeKind::Slider {
                        min: 0.0,
                        max: 10.0,
                        value: 0.0,
                    },
                ),
                node("mixer", NodeKind::Mixer),
            ],
            edges: vec![],
        };
        let control = |node: &str, action: Value| {
            call(
                &d,
                "node.control",
                json!({"node": node, "action": action, "persist": true}),
            )
        };
        // nothing runs yet: a press has nowhere to go
        assert!(control("button", json!({"type": "press"})).error.is_some());
        ok(call(&d, "graph.set", serde_json::to_value(&graph).unwrap()));
        assert!(control("button", json!({"type": "press"})).error.is_none());
        assert!(control("switch", json!({"type": "switch", "on": true}))
            .error
            .is_none());
        assert!(control("slider", json!({"type": "slide", "value": 99.0}))
            .error
            .is_none());
        // the wrong sort of block, and what is not a block
        assert!(control("switch", json!({"type": "press"})).error.is_some());
        assert!(control("button", json!({"type": "slide", "value": 1.0}))
            .error
            .is_some());
        assert!(control("nothing", json!({"type": "press"})).error.is_some());
        // the graph that is kept holds where they were set (the slider kept inside its limits)
        let got: GraphSpec =
            serde_json::from_value(ok(call(&d, "graph.get", Value::Null))).unwrap();
        let kind = |id: &str| {
            got.nodes
                .iter()
                .find(|n| n.id == id.into())
                .unwrap()
                .kind
                .clone()
        };
        assert_eq!(kind("switch"), NodeKind::Switch { on: true });
        assert_eq!(
            kind("slider"),
            NodeKind::Slider {
                min: 0.0,
                max: 10.0,
                value: 10.0
            }
        );
        drop(d);
        let _ = std::fs::remove_dir_all(&dir);
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
    fn a_graph_that_took_the_daemon_down_at_its_last_start_is_kept_but_not_started_again_by_itself_once(
    ) {
        let sim = SimBackend::new();
        sim.add_app(1, "a.exe", SimSource::tone(1000.0));
        let dir = temp_dir("restoring");
        {
            let d = daemon_in(&dir, &sim);
            assert!(
                call(&d, "graph.set", serde_json::to_value(chain()).unwrap())
                    .error
                    .is_none()
            );
        }
        // a start that ended properly leaves nothing behind: the graph comes back
        {
            let d = daemon_in(&dir, &sim);
            d.restore();
            std::thread::sleep(Duration::from_millis(300));
            assert!(d.tick().running);
            assert!(
                dir.join("restoring").exists(),
                "while it has not lived long enough, the restore is marked"
            );
        }
        assert!(
            !dir.join("restoring").exists(),
            "ending properly takes the mark away"
        );
        // a start that was cut short (crash, kill) leaves the mark: the next one keeps the graph and does not start it
        std::fs::write(dir.join("restoring"), b"").unwrap();
        {
            let d = daemon_in(&dir, &sim);
            d.restore();
            std::thread::sleep(Duration::from_millis(200));
            assert!(!d.tick().running, "not started again by itself");
            let got: GraphSpec =
                serde_json::from_value(call(&d, "graph.get", Value::Null).result.unwrap()).unwrap();
            assert_eq!(got, chain(), "but kept");
        }
        // and only once: the one after that restores again
        {
            let d = daemon_in(&dir, &sim);
            d.restore();
            std::thread::sleep(Duration::from_millis(300));
            assert!(d.tick().running);
        }
        // living long enough counts as having worked
        {
            let mut d = daemon_in(&dir, &sim);
            d.restore_settles = Duration::ZERO;
            d.restore();
            d.tick();
            assert!(!dir.join("restoring").exists());
        }
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
        sim.add_render_device("Speakers (Spk)", omu_types::AudioFormat::ENGINE, 0.0);
        sim.add_render_device("Real Speakers", omu_types::AudioFormat::ENGINE, 0.0);
        let d = daemon_in(&temp_dir("feedback-speaker"), &sim);
        let graph = |target: &str| GraphSpec {
            nodes: vec![
                NodeSpec {
                    id: "in".into(),
                    display: None,
                    inputs: Default::default(),
                    muted: false,
                    kind: NodeKind::VirtualSpeaker { name: "Spk".into() },
                },
                NodeSpec {
                    id: "out".into(),
                    display: None,
                    inputs: Default::default(),
                    muted: false,
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
            inputs: Default::default(),
            muted: false,
            kind: NodeKind::DeviceCapture {
                device: DeviceRef::Name("Microphone (Mic)".into()),
            },
        });
        g.nodes.push(NodeSpec {
            id: "spk".into(),
            display: None,
            inputs: Default::default(),
            muted: false,
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
            inputs: Default::default(),
            muted: false,
            kind: NodeKind::DeviceCapture {
                device: DeviceRef::Name("Real".into()),
            },
        });
        ok.nodes.push(NodeSpec {
            id: "spk".into(),
            display: None,
            inputs: Default::default(),
            muted: false,
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
            let mut l = omu_protocol::methods::Layout::new();
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
        let got: omu_protocol::methods::Layout =
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
        let apps: Vec<omu_hal::AppInfo> =
            serde_json::from_value(call(&d, "apps.list", Value::Null).result.unwrap()).unwrap();
        assert_eq!(apps[0].pid, 9);
        let devs: Vec<omu_hal::DeviceInfo> =
            serde_json::from_value(call(&d, "devices.list", Value::Null).result.unwrap()).unwrap();
        assert!(devs.iter().any(|x| x.name == "mic"));
    }

    // ---- cleaning up the old devices the system remembers

    use omu_desktop::elevation::{ElevateError, Elevator};
    use omu_types::{CleanOutcome, StaleDevice, StaleDevices};

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
        ) -> Result<Box<dyn omu_hal::RenderStream>, HalError> {
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
        let info: omu_types::SoundInfo = serde_json::from_value(ok(call(
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
        let library: omu_types::SoundLibrary =
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
        let status = |d: &Daemon| -> omu_types::SoundStatus {
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
                    triggers: vec![],
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
                NodeSpec::new(
                    "sb",
                    NodeKind::Soundboard {
                        board: "b1".into(),
                        triggers: vec![],
                    },
                ),
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
        let settings: omu_types::Settings =
            serde_json::from_value(ok(call(&d, "settings.get", Value::Null))).unwrap();
        assert_eq!(settings.sound_cache_mb, 256);
        ok(call(&d, "settings.set", json!({"sound_cache_mb": 64})));
        assert_eq!(status(&d).cache_limit_bytes, 64 * 1024 * 1024);
        drop(d);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_certificates_the_program_trusts_follow_the_settings_and_the_folder() {
        let dir = temp_dir("trust");
        let sim = SimBackend::new();
        let d = daemon_in(&dir, &sim);
        let report = |d: &Daemon| -> omu_types::TrustReport {
            let r = call(d, "certificates.list", Value::Null);
            assert!(r.error.is_none(), "{:?}", r.error);
            serde_json::from_value(r.result.unwrap()).unwrap()
        };
        // by default: the system's certificates, and the folder is in the data folder
        let first = report(&d);
        assert_eq!(first.mode, omu_types::TrustMode::System);
        assert!(
            first.folder.ends_with("certificates")
                && first.certificates.is_empty()
                && first.problems.is_empty()
        );

        // trusting only one's own certificates with none in the folder is reported as the problem it is
        let mut settings: omu_types::Settings =
            serde_json::from_value(call(&d, "settings.get", Value::Null).result.unwrap()).unwrap();
        settings.trust = omu_types::TrustMode::Own;
        let r = call(&d, "settings.set", serde_json::to_value(&settings).unwrap());
        assert!(r.error.is_none(), "{:?}", r.error);
        let own = report(&d);
        assert_eq!(own.mode, omu_types::TrustMode::Own);
        assert!(
            own.problems.iter().any(|p| p.code == "nothing_trusted"),
            "{own:?}"
        );

        // the choice is kept, and a folder of the user's own is the one that is read
        let mine = dir.join("my-certs");
        std::fs::create_dir_all(&mine).unwrap();
        settings.certificates_dir = Some(mine.display().to_string());
        assert!(
            call(&d, "settings.set", serde_json::to_value(&settings).unwrap())
                .error
                .is_none()
        );
        let moved = report(&d);
        assert_eq!(moved.folder, mine.display().to_string());
        assert!(moved.folder_exists);
        let again: omu_types::Settings =
            serde_json::from_value(call(&d, "settings.get", Value::Null).result.unwrap()).unwrap();
        assert_eq!(
            (again.trust, again.certificates_dir),
            (omu_types::TrustMode::Own, Some(mine.display().to_string()))
        );
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
        let info: omu_types::SoundInfo = serde_json::from_value(ok(call(
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
        let library: omu_types::SoundLibrary =
            serde_json::from_value(ok(call(&d, "soundboards.get", Value::Null))).unwrap();
        assert_eq!(library.boards[0].pads[0].key.as_deref(), Some("Ctrl+Alt+1"));

        // the soundboard is not in a running graph: the keys are not taken from other programs
        assert!(!sim.press("Ctrl+Alt+1"), "nothing listens yet");

        let graph = GraphSpec {
            nodes: vec![
                NodeSpec::new(
                    "sb",
                    NodeKind::Soundboard {
                        board: "b1".into(),
                        triggers: vec![],
                    },
                ),
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
            .open_hotkey(&omu_types::KeyCombo::parse("F7").unwrap(), Box::new(|| {}))
            .unwrap();
        ok(call(
            &d,
            "soundboards.set",
            json!([{"id": "b1", "name": "x", "pads": [{"id": "p1", "name": "p", "key": "F7"}]}]),
        ));
        let graph = GraphSpec {
            nodes: vec![NodeSpec::new(
                "sb",
                NodeKind::Soundboard {
                    board: "b1".into(),
                    triggers: vec![],
                },
            )],
            edges: vec![],
        };
        ok(call(&d, "graph.set", serde_json::to_value(&graph).unwrap()));
        std::thread::sleep(Duration::from_millis(400));
        let status: omu_types::SoundStatus =
            serde_json::from_value(ok(call(&d, "soundboards.status", Value::Null))).unwrap();
        assert_eq!(status.hotkey_problems.len(), 1, "{status:?}");
        assert_eq!(status.hotkey_problems[0].pad, "p1");
        assert!(status.hotkey_problems[0].message.contains("already used"));
        drop(d);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pad_that_plays_while_its_key_is_held_stops_when_the_key_is_let_go() {
        let dir = temp_dir("padhold");
        let sim = SimBackend::new();
        let d = daemon_in(&dir, &sim);
        let ok = |r: Response| -> Value {
            assert!(r.error.is_none(), "{:?}", r.error);
            r.result.unwrap_or(Value::Null)
        };
        std::fs::create_dir_all(&dir).unwrap();
        write_tone_wav(&dir.join("tone.wav"));
        let info: omu_types::SoundInfo = serde_json::from_value(ok(call(
            &d,
            "sounds.import",
            json!({"path": dir.join("tone.wav").display().to_string()}),
        )))
        .unwrap();
        ok(call(
            &d,
            "soundboards.set",
            json!([{"id": "b1", "name": "x", "pads": [
                {"id": "held", "name": "held", "sound": info.id, "key": "F8", "hold": true, "looped": true},
                {"id": "tap", "name": "tap", "sound": info.id, "key": "F9", "looped": true}
            ]}]),
        ));
        let graph = GraphSpec {
            nodes: vec![NodeSpec::new(
                "sb",
                NodeKind::Soundboard {
                    board: "b1".into(),
                    triggers: vec![],
                },
            )],
            edges: vec![],
        };
        ok(call(&d, "graph.set", serde_json::to_value(&graph).unwrap()));
        std::thread::sleep(Duration::from_millis(600));
        let playing = |d: &Daemon| -> Vec<String> {
            serde_json::from_value(ok(call(d, "soundboard.playing", json!({"board": "b1"}))))
                .unwrap()
        };

        assert!(sim.press("F8"));
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(playing(&d), vec!["held".to_string()], "playing while held");
        assert!(sim.release("F8"));
        std::thread::sleep(Duration::from_millis(200));
        assert!(playing(&d).is_empty(), "stopped when let go");

        // a pad without the option goes on playing when the key is let go (there is no release to hear)
        assert!(sim.press("F9"));
        std::thread::sleep(Duration::from_millis(200));
        assert!(!sim.release("F9"), "no release is registered for it");
        assert_eq!(playing(&d), vec!["tap".to_string()]);
        drop(d);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_graph_can_play_and_stop_the_pads_of_a_soundboard_with_actions() {
        let dir = temp_dir("padtrigger");
        let sim = SimBackend::new();
        let d = daemon_in(&dir, &sim);
        let ok = |r: Response| -> Value {
            assert!(r.error.is_none(), "{:?}", r.error);
            r.result.unwrap_or(Value::Null)
        };
        std::fs::create_dir_all(&dir).unwrap();
        write_tone_wav(&dir.join("tone.wav"));
        let info: omu_types::SoundInfo = serde_json::from_value(ok(call(
            &d,
            "sounds.import",
            json!({"path": dir.join("tone.wav").display().to_string()}),
        )))
        .unwrap();
        ok(call(
            &d,
            "soundboards.set",
            json!([{"id": "b1", "name": "x", "pads": [
                {"id": "p1", "name": "one", "sound": info.id, "looped": true},
                {"id": "p2", "name": "two", "sound": info.id, "looped": true}
            ]}]),
        ));
        let graph = json!({
            "nodes": [
                {"id": "sb", "type": "soundboard", "board": "b1", "triggers": ["p2", "p1"]},
                {"id": "k1", "type": "hotkey", "keys": "F5"},
                {"id": "k2", "type": "hotkey", "keys": "F6"},
                {"id": "k3", "type": "hotkey", "keys": "F7"}
            ],
            "edges": [
                {"from": {"node": "k1", "port": "pressed"}, "to": {"node": "sb", "port": "play1"}},
                {"from": {"node": "k2", "port": "pressed"}, "to": {"node": "sb", "port": "play2"}},
                {"from": {"node": "k3", "port": "pressed"}, "to": {"node": "sb", "port": "stop"}}
            ]
        });
        ok(call(&d, "graph.set", graph));
        std::thread::sleep(Duration::from_millis(700));
        let playing = |d: &Daemon| -> Vec<String> {
            let mut v: Vec<String> =
                serde_json::from_value(ok(call(d, "soundboard.playing", json!({"board": "b1"}))))
                    .unwrap();
            v.sort();
            v
        };
        assert!(playing(&d).is_empty());

        // play1 is the first trigger: pad p2
        assert!(sim.press("F5"));
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(playing(&d), vec!["p2".to_string()]);
        // play2 is p1
        assert!(sim.press("F6"));
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(playing(&d), vec!["p1".to_string(), "p2".to_string()]);
        // stop stops the whole soundboard
        assert!(sim.press("F7"));
        std::thread::sleep(Duration::from_millis(300));
        assert!(playing(&d).is_empty());

        // a link to a trigger that the block does not have is refused
        let bad = json!({
            "nodes": [
                {"id": "sb", "type": "soundboard", "board": "b1", "triggers": ["p1"]},
                {"id": "k", "type": "hotkey", "keys": "F5"}
            ],
            "edges": [{"from": {"node": "k", "port": "pressed"}, "to": {"node": "sb", "port": "play2"}}]
        });
        assert_eq!(
            call(&d, "graph.set", bad).error.unwrap().code,
            RpcError::VALIDATION
        );
        drop(d);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saving_the_settings_tells_whoever_listens_and_the_tray_icon_is_on_by_default() {
        let dir = temp_dir("settings-listener");
        let sim = SimBackend::new();
        let d = daemon_in(&dir, &sim);
        let seen = Arc::new(Mutex::new(vec![]));
        let s2 = seen.clone();
        d.on_settings(move |s| s2.lock().unwrap().push(s.show_tray_icon));
        let current: omu_types::Settings =
            serde_json::from_value(call(&d, "settings.get", Value::Null).result.unwrap()).unwrap();
        assert!(current.show_tray_icon, "shown unless the user says no");
        let off = omu_types::Settings {
            show_tray_icon: false,
            ..current
        };
        assert!(
            call(&d, "settings.set", serde_json::to_value(&off).unwrap())
                .error
                .is_none()
        );
        assert_eq!(*seen.lock().unwrap(), vec![false]);
        // settings saved by an older version have no such field: the icon is shown
        let old: omu_types::Settings =
            serde_json::from_value(json!({"sound_cache_mb": 64})).unwrap();
        assert!(old.show_tray_icon);
        drop(d);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sound_goes_through_the_network_blocks_and_bad_settings_are_refused() {
        let dir = temp_dir("network");
        let sim = SimBackend::new();
        sim.add_app(77, "tone.exe", SimSource::tone(440.0));
        let d = daemon_in(&dir, &sim);
        // a free UDP port: ask for one, then let it go
        let port = std::net::UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let graph = json!({
            "nodes": [
                {"id": "app", "type": "app_capture", "target": {"process_name": "tone.exe"}},
                {"id": "send", "type": "network_out", "host": "127.0.0.1", "port": port, "stream": "T"},
                {"id": "recv", "type": "network_in", "port": port, "stream": "T", "from": "127.0.0.1"},
                {"id": "mic", "type": "virtual_mic", "name": "Net"}
            ],
            "edges": [
                {"from": {"node": "app", "port": "out"}, "to": {"node": "send", "port": "in"}},
                {"from": {"node": "recv", "port": "out"}, "to": {"node": "mic", "port": "in"}}
            ]
        });
        let applied = call(&d, "graph.set", graph);
        assert!(applied.error.is_none(), "{:?}", applied.error);
        std::thread::sleep(Duration::from_millis(1800));
        let recorded = sim
            .recording_of("virtual:Net")
            .expect("the virtual microphone")
            .lock()
            .unwrap()
            .clone();
        let left: Vec<f32> = recorded.chunks(2).map(|f| f[0]).collect();
        // after the first moments, the tone is heard (440 Hz, about the level the source has)
        let tail = &left[left.len().saturating_sub(24_000)..];
        let level = rms(tail);
        assert!(
            level > 0.1,
            "the sound came back through the network: rms {level}"
        );
        let rising = tail
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count() as f32
            * 48_000.0
            / tail.len() as f32;
        assert!((rising - 440.0).abs() < 40.0, "pitch {rising} Hz");

        // refused: a port of 0, two blocks on one port, a bad name, an address that is not one
        for bad in [
            json!({"nodes": [{"id": "r", "type": "network_in", "port": 0}], "edges": []}),
            json!({"nodes": [{"id": "a", "type": "network_in", "port": 7000}, {"id": "b", "type": "network_in", "port": 7000}], "edges": []}),
            json!({"nodes": [{"id": "r", "type": "network_in", "port": 7001, "stream": "a name that is far too long"}], "edges": []}),
            json!({"nodes": [{"id": "r", "type": "network_in", "port": 7002, "from": "not an address"}], "edges": []}),
            json!({"nodes": [{"id": "s", "type": "network_out", "host": " ", "port": 6980}], "edges": []}),
        ] {
            assert_eq!(
                call(&d, "graph.set", bad.clone()).error.unwrap().code,
                RpcError::VALIDATION,
                "{bad}"
            );
        }
        drop(d);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// app (a tone) -> deep noise reduction -> virtual microphone; returns the recording of the microphone.
    fn deep_filter_run(d: &Daemon, sim: &SimBackend) -> (Vec<f32>, Value) {
        let graph = json!({
            "nodes": [
                {"id": "app", "type": "app_capture", "target": {"process_name": "tone.exe"}},
                {"id": "df", "type": "deep_filter"},
                {"id": "mic", "type": "virtual_mic", "name": "Clean"}
            ],
            "edges": [
                {"from": {"node": "app", "port": "out"}, "to": {"node": "df", "port": "in"}},
                {"from": {"node": "df", "port": "out"}, "to": {"node": "mic", "port": "in"}}
            ]
        });
        let applied = call(d, "graph.set", graph);
        assert!(applied.error.is_none(), "{:?}", applied.error);
        std::thread::sleep(Duration::from_millis(2500));
        d.tick();
        let status = call(d, "engine.status", Value::Null).result.unwrap();
        let df = status["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == "df")
            .cloned()
            .unwrap_or(Value::Null);
        let recorded = sim
            .recording_of("virtual:Clean")
            .expect("the virtual microphone")
            .lock()
            .unwrap()
            .clone();
        (recorded.chunks(2).map(|f| f[0]).collect(), df)
    }

    #[test]
    fn the_deep_noise_block_lets_the_sound_through_and_says_why_while_its_module_is_missing() {
        let dir = temp_dir("deepfilter-missing");
        let sim = SimBackend::new();
        sim.add_app(77, "tone.exe", SimSource::tone(440.0));
        let d = daemon_in(&dir, &sim);
        let (left, df) = deep_filter_run(&d, &sim);
        let tail = &left[left.len().saturating_sub(24_000)..];
        assert!(
            rms(tail) > 0.1,
            "the sound goes through as it is: rms {}",
            rms(tail)
        );
        assert_eq!(df["state"], "failed", "{df}");
        assert!(
            df["message"].as_str().unwrap().contains("Modules panel"),
            "{df}"
        );
        drop(d);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With the real model (set OMU_TEST_DF_MODEL to the DeepFilterNet3_onnx.tar.gz): a tone is not a voice, so the model
    /// takes it away.
    #[test]
    #[cfg(feature = "deepfilter")]
    fn the_deep_noise_block_with_the_real_model_takes_away_what_is_not_a_voice() {
        let Some(model) = std::env::var_os("OMU_TEST_DF_MODEL") else {
            eprintln!("OMU_TEST_DF_MODEL is not set: skipped");
            return;
        };
        let dir = temp_dir("deepfilter-real");
        let sim = SimBackend::new();
        sim.add_app(77, "tone.exe", SimSource::tone(440.0));
        let d = daemon_in(&dir, &sim);
        let folder = dir.join("modules").join("deepfilter");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::copy(&model, folder.join(omu_modules::deepfilter::MODEL_FILE)).unwrap();
        assert!(call(
            &d,
            "modules.enable",
            json!({"id": "deepfilter", "enabled": true})
        )
        .error
        .is_none());
        let (left, df) = deep_filter_run(&d, &sim);
        assert_eq!(df["state"], "running", "{df}");
        let tail = &left[left.len().saturating_sub(24_000)..];
        assert!(
            rms(tail) < 0.05,
            "the tone is taken away: rms {}",
            rms(tail)
        );
        drop(d);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stopped_engine_starts_again_with_the_last_graph_and_has_nothing_to_start_without_one() {
        let dir = temp_dir("engine-start");
        let sim = SimBackend::new();
        let d = daemon_in(&dir, &sim);
        let running = |d: &Daemon| -> bool {
            d.tick();
            call(d, "engine.status", Value::Null).result.unwrap()["running"] == true
        };
        // nothing was ever applied: there is nothing to start, and it says so
        let none = call(&d, "engine.start", Value::Null);
        assert_eq!(none.error.unwrap().code, RpcError::UNAVAILABLE);

        assert!(
            call(&d, "graph.set", serde_json::to_value(chain()).unwrap())
                .error
                .is_none()
        );
        std::thread::sleep(Duration::from_millis(500));
        assert!(running(&d));
        assert!(call(&d, "engine.stop", Value::Null).error.is_none());
        assert!(!running(&d));
        // the graph is still the saved one: starting runs it again
        assert!(call(&d, "engine.start", Value::Null).error.is_none());
        std::thread::sleep(Duration::from_millis(500));
        assert!(running(&d));
        // and it is what a restart would bring back too
        assert!(lock(&d.state).saved.running);
        drop(d);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
