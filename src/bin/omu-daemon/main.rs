//! `omu-daemon`: the daemon. Owns the audio engine, serves the control protocol and the web panel.
//! See `omu-daemon --help`.

use clap::Parser;
use omu::daemon::instance;
use omu::daemon::launch::{serve, Options, DEFAULT_LISTEN};
use omu::diagnostics::{self, Settings};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = omu::branding::DAEMON_EXECUTABLE, version, about = "Audio routing daemon")]
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
    /// Break on purpose once started, to check that crashes are handled (used by the tests).
    #[arg(long, hide = true, value_name = "KIND")]
    crash_test: Option<String>,
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

fn main() -> std::process::ExitCode {
    let args = Args::parse();
    let data_dir = args.data_dir.unwrap_or_else(omu::discovery::data_dir);
    let mut diag = diagnostics::init(Settings {
        program: omu::branding::DAEMON_EXECUTABLE,
        data_dir: data_dir.clone(),
        dialog: true,
        console_log: !args.quiet,
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
    let loopback = opts.listen.ip().is_loopback();
    let crash_test = args.crash_test;

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => diagnostics::fail(&format!("cannot start the async runtime: {e}"), 1),
    };
    let ready = move |info: &omu::daemon::launch::Info| {
        println!(
            "{} daemon listening on http://{}",
            omu::branding::DISPLAY_NAME,
            info.address
        );
        println!("audio backend: {}", info.backend);
        println!("web panel: {}", info.panel_url);
        if !loopback {
            tracing::warn!("listening on a non-local address; the access token is sent in clear text over the network");
        }
        println!("press Ctrl-C to stop");
        if let Some(kind) = &crash_test {
            crash_on_purpose(kind);
        }
    };
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("Ctrl-C received, stopping");
    };
    match runtime.block_on(serve(opts, ready, shutdown)) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => diagnostics::fail(&e, 1),
    }
}
