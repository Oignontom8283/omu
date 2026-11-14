//! `omu-daemon`: the daemon. Owns the audio engine, serves the control protocol and the web panel.
//! See `omu-daemon --help`.

use clap::Parser;
use omu::daemon::launch::{serve, Options, DEFAULT_LISTEN};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

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
}

fn main() -> ExitCode {
    let args = Args::parse();
    let mut opts = Options::new(args.data_dir.unwrap_or_else(omu::discovery::data_dir));
    opts.listen = args.listen;
    opts.allowed_origins = args.allow_origin;
    opts.restore = !args.no_restore;
    opts.demo = args.demo;
    let loopback = opts.listen.ip().is_loopback();

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: cannot start the runtime: {e}");
            return ExitCode::FAILURE;
        }
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
            eprintln!("warning: listening on a non-local address; the access token is sent in clear text over the network");
        }
        println!("press Ctrl-C to stop");
    };
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
        println!("stopping");
    };
    match runtime.block_on(serve(opts, ready, shutdown)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
