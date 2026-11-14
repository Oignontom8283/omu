//! Starting a daemon: choosing the audio backend, restoring the saved graph, opening the control port, publishing
//! the discovery file. Shared by the `omu-daemon` binary and by the tests that exercise the whole HTTP stack.

use super::server::{self, ServerConfig};
use super::store::Store;
use super::{auth, Daemon};
use crate::discovery::Discovery;
use crate::engine::EngineConfig;
use crate::hal::sim::{SimBackend, SimSource};
use crate::hal::{Backend, VirtualDeviceProvider};
use crate::types::AudioFormat;
use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

/// The default control address: loopback only.
pub const DEFAULT_LISTEN: &str = "127.0.0.1:7650";

#[derive(Debug, Clone)]
pub struct Options {
    pub listen: SocketAddr,
    pub data_dir: PathBuf,
    pub allowed_origins: Vec<String>,
    /// Start again what was running when the daemon last stopped.
    pub restore: bool,
    /// Use a simulated audio system (fake applications and devices) instead of the real one: to try the panel and
    /// the protocol on a machine without audio, or without touching the real audio.
    pub demo: bool,
}

impl Options {
    pub fn new(data_dir: PathBuf) -> Options {
        Options {
            listen: DEFAULT_LISTEN
                .parse()
                .expect("the default address is valid"),
            data_dir,
            allowed_origins: vec![],
            restore: true,
            demo: false,
        }
    }
}

/// What a started daemon tells its starter.
#[derive(Debug, Clone)]
pub struct Info {
    pub address: SocketAddr,
    pub token: String,
    pub backend: String,
    pub panel_url: String,
}

/// A fake audio system: a few applications playing tones, a microphone, speakers. Nothing is recorded.
fn demo_backend() -> SimBackend {
    let sim = SimBackend::new();
    sim.discard_audio();
    sim.add_app(1001, "chrome.exe", SimSource::tone(440.0));
    sim.add_app(1002, "spotify.exe", SimSource::tone(660.0));
    sim.add_app(1003, "discord.exe", SimSource::tone(880.0));
    sim.add_capture_device("Demo Microphone", SimSource::tone(220.0));
    sim.add_render_device("Demo Speakers", AudioFormat::ENGINE, 0.0);
    sim
}

/// The real audio system of this platform, with the virtual-microphone provider when there is one.
#[allow(clippy::type_complexity)]
fn real_backend() -> Result<
    (
        Arc<dyn Backend>,
        Option<Arc<dyn VirtualDeviceProvider>>,
        EngineConfig,
    ),
    String,
> {
    #[cfg(windows)]
    {
        let backend = Arc::new(crate::hal::wasapi::WasapiBackend::new());
        let config = EngineConfig {
            thread_init: Some(crate::hal::wasapi::graph_thread_init()),
            ..Default::default()
        };
        Ok((
            backend,
            Some(crate::virtual_devices::platform_provider()),
            config,
        ))
    }
    #[cfg(not(windows))]
    {
        Err(
            "there is no audio backend for this platform yet; start with --demo to try the panel"
                .into(),
        )
    }
}

/// Runs a daemon until `shutdown` completes. `ready` is called once the port is open.
pub async fn serve(
    opts: Options,
    ready: impl FnOnce(&Info),
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), String> {
    let (backend, virtual_devices, engine_config): (
        Arc<dyn Backend>,
        Option<Arc<dyn VirtualDeviceProvider>>,
        EngineConfig,
    ) = if opts.demo {
        let sim = demo_backend();
        (
            Arc::new(sim.clone()),
            Some(Arc::new(sim) as Arc<dyn VirtualDeviceProvider>),
            EngineConfig::default(),
        )
    } else {
        real_backend()?
    };
    let backend_name = backend.name().to_string();
    let daemon = Arc::new(Daemon::new(
        backend,
        virtual_devices,
        engine_config,
        Store::new(&opts.data_dir),
    ));

    let token = auth::new_token();
    let (listener, router) = server::bind(
        daemon.clone(),
        ServerConfig {
            listen: opts.listen,
            token: token.clone(),
            allowed_origins: opts.allowed_origins.clone(),
        },
    )
    .await
    .map_err(|e| match e.kind() {
        std::io::ErrorKind::AddrInUse => {
            format!(
                "{} is already in use: another daemon is probably running (see {})",
                opts.listen,
                opts.data_dir.display()
            )
        }
        _ => format!("cannot listen on {}: {e}", opts.listen),
    })?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;

    if opts.restore {
        let d = daemon.clone();
        let _ = tokio::task::spawn_blocking(move || d.restore()).await;
    }

    let discovery = Discovery {
        address: address.to_string(),
        token: token.clone(),
        pid: std::process::id(),
    };
    discovery.write(&opts.data_dir).map_err(|e| {
        format!(
            "cannot write {}: {e}",
            Discovery::path_in(&opts.data_dir).display()
        )
    })?;
    ready(&Info {
        address,
        token,
        backend: backend_name,
        panel_url: discovery.panel_url(),
    });

    let result = server::run(daemon.clone(), listener, router, shutdown).await;
    Discovery::remove_if_mine(&opts.data_dir);
    // Dropping the daemon stops the engine and every stream.
    drop(daemon);
    result.map_err(|e| e.to_string())
}

/// A daemon running on its own thread (for tests and for embedding).
pub struct Running {
    pub info: Info,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Running {
    /// Starts a daemon and waits until its port is open.
    pub fn start(opts: Options) -> Result<Running, String> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<Info, String>>();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("daemon".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e.to_string()));
                        return;
                    }
                };
                let tx = ready_tx.clone();
                let outcome = runtime.block_on(serve(
                    opts,
                    move |info| {
                        let _ = tx.send(Ok(info.clone()));
                    },
                    async {
                        let _ = stop_rx.await;
                    },
                ));
                if let Err(e) = outcome {
                    let _ = ready_tx.send(Err(e));
                }
            })
            .map_err(|e| e.to_string())?;
        match ready_rx.recv_timeout(std::time::Duration::from_secs(15)) {
            Ok(Ok(info)) => Ok(Running {
                info,
                stop: Some(stop_tx),
                thread: Some(thread),
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("the daemon did not start within 15 seconds".into()),
        }
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.shutdown();
    }
}
