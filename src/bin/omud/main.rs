//! `omud`: the daemon. Owns the audio engine, serves the control protocol and the web panel.
//! See `omud --help`.

use clap::Parser;
use omu_daemon::instance;
use omu_daemon::launch::{serve, Options, DEFAULT_LISTEN};
use omu_diagnostics::{self as diagnostics, Settings};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = omu_branding::DAEMON_EXECUTABLE, version, about = "Audio routing daemon")]
struct Args {
    /// Address and port of the control server. Loopback by default; anything else exposes the daemon to the
    /// network (the access token then travels unencrypted unless a TLS proxy or an SSH tunnel is used).
    #[arg(long, default_value = DEFAULT_LISTEN)]
    listen: SocketAddr,
    /// Where the daemon keeps its files (default: the user's data directory).
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Allow a browser page served from this origin to call the daemon (a development server of the panel).
    #[arg(long = "allow-origin")]
    allow_origin: Vec<String>,
    /// Do not start again the graph that was running when the daemon last stopped.
    #[arg(long)]
    no_restore: bool,
    /// Use a simulated audio system (fake applications and devices): try the panel without any real audio.
    #[arg(long)]
    demo: bool,
    /// Do not print the logs on the console (they still go to the log file).
    #[arg(long)]
    quiet: bool,
    /// Run without a console window: what a start at logon uses (the console is let go and nothing is printed).
    #[arg(long)]
    background: bool,
    /// Do not show the icon in the notification area, whatever the settings say.
    #[arg(long)]
    no_tray: bool,
    /// Make the system forget the old virtual devices it still remembers, then stop (the daemon does not start). Asks
    /// for administrator rights only when this program does not have them already.
    #[arg(long)]
    clean_devices: bool,
    /// Turns the start of the daemon at logon on or off for the current user (what the installer's option does), then stop: the
    /// daemon does not start.
    #[arg(long, value_name = "ON|OFF")]
    set_start_at_logon: Option<Switch>,
    /// Puts the folder of this program in the PATH of the computer, or takes it out (what the installer does; needs administrator
    /// rights), then stop: the daemon does not start.
    #[arg(long, value_name = "ON|OFF")]
    set_path: Option<Switch>,
    /// Takes this product's devices out of the USB/IP driver's permanent list (what the installer does when it removes the product's data), then stop.
    #[arg(long)]
    forget_permanent_devices: bool,
    /// Asks the daemon that runs for this user to stop and waits for it to be gone (what the installer does before it replaces the
    /// programs), then stop: nothing happens when none runs.
    #[arg(long)]
    stop: bool,
    /// Used by the program itself when it starts again with administrator rights for `--clean-devices`.
    #[arg(long, hide = true)]
    elevated_helper: bool,
    /// Break on purpose once started, to check that crashes are handled (used by the tests).
    #[arg(long, hide = true, value_name = "KIND")]
    crash_test: Option<String>,
}

/// A value that is on or off.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum Switch {
    On,
    Off,
}

/// `--stop`: asks the daemon of this data folder to stop, and waits (a few seconds at most) until it no longer answers.
fn stop_running(data_dir: &std::path::Path) {
    use omu_protocol::methods::{SystemInfoMethod, SystemShutdown};
    let Ok(client) = omu_client::Client::discover_in(data_dir) else {
        return;
    };
    if client.call::<SystemShutdown>(&()).is_err() {
        return;
    }
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if client.call::<SystemInfoMethod>(&()).is_err() {
            return;
        }
    }
}

/// Fails the way the named kind of bug would.
fn crash_on_purpose(kind: &str) {
    match kind {
        // a panic where nothing guards it: fatal
        "panic" => panic!("crash test: panic on the main thread"),
        "thread-panic" => {
            let _ = std::thread::Builder::new()
                .name("crash-test".into())
                .spawn(|| panic!("crash test: panic on a worker thread"))
                .map(|t| t.join());
            // the daemon must be gone before this line is reached
            std::thread::sleep(std::time::Duration::from_secs(30));
        }
        // a panic inside a guarded block: reported, and the daemon goes on
        "contained" => {
            let outcome = diagnostics::contained(|| panic!("crash test: contained panic"));
            tracing::info!(
                survived = outcome.is_err(),
                "the contained panic was handled"
            );
        }
        // a native crash: writing to address 0
        "violation" => {
            // SAFETY: none, this is the point: it must crash with an access violation.
            unsafe { std::ptr::write_volatile(std::ptr::null_mut::<u8>(), 1) }
        }
        other => tracing::error!("unknown crash test {other:?}"),
    }
}

/// `--clean-devices`: the old devices of the saved graph's neighbours, forgotten; no daemon is started, so it runs next
/// to a daemon that is running.
fn clean_devices(data_dir: PathBuf, helper: bool) -> std::process::ExitCode {
    use omu_daemon::store::Store;
    use omu_desktop::elevation::SystemElevator;
    use omu_types::KeptDevices;
    use omu_virtual_devices::clean::{self, Relaunch};
    let _diag = omu_logging::init(Settings {
        program: omu_branding::DAEMON_EXECUTABLE,
        data_dir: data_dir.clone(),
        // a helper started with rights has no window: it must not show an error box of its own
        dialog: false,
        console_log: false,
    });
    let kept = KeptDevices::of_graph(&Store::new(&data_dir).load_state().graph);
    let relaunch = match Relaunch::this_program(&data_dir) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            return std::process::ExitCode::from(clean::EXIT_FAILED as u8);
        }
    };
    let provider = omu_virtual_devices::platform_provider();
    let code = clean::run_helper(
        &*provider,
        &kept,
        &SystemElevator,
        &relaunch,
        helper,
        &mut std::io::stdout(),
    );
    std::process::ExitCode::from(code as u8)
}

fn main() -> std::process::ExitCode {
    let args = Args::parse();
    let data_dir = args.data_dir.unwrap_or_else(omu_discovery::data_dir);
    if let Some(switch) = args.set_start_at_logon {
        return match omu_daemon::setup::set_start_at_logon(&data_dir, matches!(switch, Switch::On))
        {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                std::process::ExitCode::FAILURE
            }
        };
    }
    if let Some(switch) = args.set_path {
        let folder = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf));
        return match folder
            .ok_or_else(|| "cannot tell where this program is".to_string())
            .and_then(|f| omu_desktop::path::set(&f, matches!(switch, Switch::On)))
        {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                std::process::ExitCode::FAILURE
            }
        };
    }
    if args.forget_permanent_devices {
        // no driver, nothing to forget
        if let Err(e) = omu_virtual_devices::forget_permanent(
            &omu_virtual_devices::UsbipExe,
            omu_branding::VIRTUAL_SERVER_PORT,
        ) {
            eprintln!("{e}");
        }
        return std::process::ExitCode::SUCCESS;
    }
    if args.stop {
        stop_running(&data_dir);
        return std::process::ExitCode::SUCCESS;
    }
    if args.clean_devices {
        return clean_devices(data_dir, args.elevated_helper);
    }
    let mut diag = omu_logging::init(Settings {
        program: omu_branding::DAEMON_EXECUTABLE,
        data_dir: data_dir.clone(),
        dialog: true,
        console_log: !(args.quiet || args.background),
    });
    // One daemon at a time: the real audio system is shared by the whole machine, a simulated one by its data folder.
    let scope = if args.demo {
        instance::Scope::Folder(&data_dir)
    } else {
        instance::Scope::Machine
    };
    let _instance = match instance::acquire(scope, &data_dir) {
        Ok(lock) => lock,
        Err(message) => diagnostics::fail(&message, 2),
    };
    let previous = diag.track_session();
    let mut opts = Options::new(data_dir);
    opts.listen = args.listen;
    opts.allowed_origins = args.allow_origin;
    opts.restore = !args.no_restore;
    opts.demo = args.demo;
    opts.previous_session = previous;
    opts.tray = !args.no_tray;
    // a daemon that only simulates audio is not one to start at logon
    opts.autostart = !args.demo;
    let background = args.background;
    #[cfg(windows)]
    if background {
        // let go of the console this process was given: a start at logon must not leave a black window
        let _ = unsafe { windows::Win32::System::Console::FreeConsole() };
    }
    let loopback = opts.listen.ip().is_loopback();
    let crash_test = args.crash_test;

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => diagnostics::fail(&format!("cannot start the async runtime: {e}"), 1),
    };
    let ready = move |info: &omu_daemon::launch::Info| {
        if !background {
            println!(
                "{} daemon listening on http://{}",
                omu_branding::DISPLAY_NAME,
                info.address
            );
            println!("audio backend: {}", info.backend);
            println!("web panel: {}", info.panel_url);
            println!("press Ctrl-C (or close this window) to stop");
        }
        if !loopback {
            tracing::warn!("listening on a non-local address; the access token is sent in clear text over the network");
        }
        if let Some(kind) = &crash_test {
            crash_on_purpose(kind);
        }
    };
    let shutdown = async {
        let why = stop_requested().await;
        tracing::info!("{why}: stopping");
    };
    let outcome = runtime.block_on(serve(opts, ready, shutdown));
    // The blocking tasks still running (a prompt waiting for an answer, a driver that does not answer) are not waited for beyond a few
    // seconds: dropping the runtime would wait for them for ever, and the daemon would never end.
    runtime.shutdown_timeout(std::time::Duration::from_secs(3));
    match outcome {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => diagnostics::fail(&e, 1),
    }
}

/// Resolves when the user or the system asks the daemon to stop, and says why: Ctrl-C, Ctrl-Break, the console window
/// closed, the user logging off, Windows shutting down (on other systems: Ctrl-C or SIGTERM). The daemon then leaves
/// the audio system in order instead of being cut off; Windows gives a few seconds for that after a close.
async fn stop_requested() -> &'static str {
    #[cfg(windows)]
    {
        use tokio::signal::windows as w;
        match (
            w::ctrl_break(),
            w::ctrl_close(),
            w::ctrl_logoff(),
            w::ctrl_shutdown(),
        ) {
            (Ok(mut brk), Ok(mut close), Ok(mut logoff), Ok(mut down)) => tokio::select! {
                _ = tokio::signal::ctrl_c() => "Ctrl-C received",
                _ = brk.recv() => "Ctrl-Break received",
                _ = close.recv() => "the console window was closed",
                _ = logoff.recv() => "the user is logging off",
                _ = down.recv() => "Windows is shutting down",
            },
            _ => {
                let _ = tokio::signal::ctrl_c().await;
                "Ctrl-C received"
            }
        }
    }
    #[cfg(not(windows))]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut term) => tokio::select! {
                _ = tokio::signal::ctrl_c() => "Ctrl-C received",
                _ = term.recv() => "SIGTERM received",
            },
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                "Ctrl-C received"
            }
        }
    }
}
