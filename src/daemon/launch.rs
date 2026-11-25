//! Starting a daemon: choosing the audio backend, restoring the saved graph, opening the control port, publishing
//! the discovery file. Shared by the `omud` binary and by the tests that exercise the whole HTTP stack.

use super::server::{self, ServerConfig};
use super::store::Store;
use super::{auth, Daemon};
use omu_discovery::Discovery;
use omu_engine::EngineConfig;
use omu_hal::sim::{SimBackend, SimSource};
use omu_hal::{Backend, VirtualDeviceProvider};
use omu_types::AudioFormat;
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
    /// Show the notification-area icon (when the settings allow it and the program has one).
    pub tray: bool,
    /// Keep the user's startup list in step with the "start at logon" setting.
    pub autostart: bool,
    /// What the previous run left behind (reported by `system.info`).
    pub previous_session: omu_diagnostics::PreviousSession,
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
            tray: false,
            autostart: false,
            previous_session: Default::default(),
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
        let backend = Arc::new(omu_hal::wasapi::WasapiBackend::new());
        let config = EngineConfig {
            thread_init: Some(omu_hal::wasapi::graph_thread_init()),
            ..Default::default()
        };
        Ok((
            backend,
            Some(omu_virtual_devices::platform_provider()),
            config,
        ))
    }
    #[cfg(not(windows))]
    {
        Err(
            "sound is not supported on this system yet; start with --demo to try the panel without sound"
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
    // the system may make one of our virtual devices its default: a block on "the default device" must not use it
    let backend: Arc<dyn Backend> = match &virtual_devices {
        Some(provider) => Arc::new(omu_hal::OwnDevicesAside::new(backend, provider.clone())),
        None => backend,
    };
    let backend_name = backend.name().to_string();
    tracing::info!(
        backend = %backend_name,
        listen = %opts.listen,
        data_dir = %opts.data_dir.display(),
        demo = opts.demo,
        "starting the daemon"
    );
    omu_diagnostics::note("backend", backend_name.clone());
    omu_diagnostics::note("listen", opts.listen.to_string());
    let daemon = Arc::new(Daemon::new(
        backend,
        virtual_devices,
        engine_config,
        Store::new(&opts.data_dir),
    ));
    daemon.set_previous_session(opts.previous_session.clone());

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
    tracing::info!(%address, "control port open");

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
    // If the daemon dies without going through the end of this function, the file would send clients to nobody.
    let cleanup_dir = opts.data_dir.clone();
    let _discovery_cleanup = omu_diagnostics::on_crash_cleanup("discovery file", move || {
        Discovery::remove_if_mine(&cleanup_dir)
    });
    ready(&Info {
        address,
        token,
        backend: backend_name,
        panel_url: discovery.panel_url(),
    });

    if opts.autostart {
        daemon.manage_autostart();
    }

    // the icon in the notification area: it follows the setting for as long as the daemon runs
    #[cfg(feature = "tray")]
    let tray = opts.tray.then(|| {
        let host = super::tray_host::TrayHost::new(&daemon, discovery.panel_url());
        let h = host.clone();
        daemon.on_settings(move |s| h.set_visible(s.show_tray_icon));
        host.set_visible(daemon.sounds().settings().show_tray_icon);
        host
    });

    // stop on the caller's signal (Ctrl-C...) or when a client asks for it
    let asked = daemon.stop_signal();
    let shutdown = async move {
        tokio::select! {
            () = shutdown => {}
            () = asked.notified() => {}
        }
    };
    // the secure server follows the setting for as long as the daemon runs
    let (wanted_tx, wanted_rx) =
        tokio::sync::watch::channel(daemon.sounds().settings().remote_access);
    daemon.on_settings(move |s| {
        wanted_tx.send_if_modified(|on| std::mem::replace(on, s.remote_access) != s.remote_access);
    });
    let (https_stop, https_stopped) = tokio::sync::oneshot::channel();
    let https = tokio::spawn(server::serve_https(
        daemon.clone(),
        router.clone(),
        wanted_rx,
        https_stopped,
    ));
    let result = server::run(daemon.clone(), listener, router, shutdown).await;
    // the secure server is closed properly (it holds the daemon through its router), and not waited for beyond a moment
    let _ = https_stop.send(());
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), https).await;
    tracing::info!("shutting down");
    Discovery::remove_if_mine(&opts.data_dir);
    #[cfg(feature = "tray")]
    drop(tray);
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
