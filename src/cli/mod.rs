//! The command-line client: parses the arguments, talks to the daemon through [`crate::client`], prints the answer.
//! The binary (`src/bin/omu-cli`) only calls [`execute`]; keeping the logic here makes it testable against a real daemon.

use crate::client::{Client, ClientError};
use crate::protocol::methods::*;
use crate::types::{
    AppTarget, DeviceRef, EdgeSpec, GraphSpec, NodeKind, NodeSpec, NodeState, PortRef,
};
use clap::{Args, Parser, Subcommand};
use std::io::Write;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = crate::branding::CLI_EXECUTABLE, version, about = "Control the audio daemon")]
pub struct Cli {
    /// Address of the daemon (`host:port`), for a daemon that is not the local one found automatically.
    #[arg(long, global = true, env = "OMU_ADDRESS", requires = "token")]
    pub address: Option<String>,
    /// Access token of that daemon.
    #[arg(long, global = true, env = "OMU_TOKEN", requires = "address")]
    pub token: Option<String>,
    /// Print the raw JSON the daemon answered instead of a table.
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Show the daemon's version and backend, and the state of every node of the running graph.
    Status,
    /// List the audio devices.
    Devices,
    /// List the applications that have an audio session (the ones whose sound can be captured).
    Apps,
    /// Capture an application's sound and send it to a virtual microphone or a playback device.
    Route(RouteArgs),
    /// Show, check, apply or clear the whole graph.
    Graph {
        #[command(subcommand)]
        action: GraphAction,
    },
    /// Stop the engine (the graph is kept and starts again with `route` or `graph set`).
    Stop,
    /// Stop the daemon itself (not only its engine): everything it created goes away.
    Shutdown,
    /// Show the daemon's latest log lines (and follow them as they come with --follow).
    Logs(LogsArgs),
    /// Print the address of the web panel (and open it with --open).
    Panel {
        #[arg(long)]
        open: bool,
    },
}

#[derive(Args, Debug)]
pub struct LogsArgs {
    /// How many of the latest lines to show.
    #[arg(short = 'n', long, default_value_t = 100)]
    pub lines: usize,
    /// Keep running and print new lines as they are logged (stop with Ctrl-C).
    #[arg(short, long)]
    pub follow: bool,
}

#[derive(Args, Debug)]
pub struct RouteArgs {
    /// The application: a process id, or an executable name such as `chrome.exe`.
    #[arg(long)]
    pub app: String,
    /// Send to a virtual microphone with this name (other programs pick it as their microphone).
    #[arg(long, conflicts_with = "device", required_unless_present = "device")]
    pub mic: Option<String>,
    /// Send to this playback device (a part of its name is enough).
    #[arg(long)]
    pub device: Option<String>,
    /// Gain in decibels applied on the way (negative = quieter).
    #[arg(long, allow_hyphen_values = true)]
    pub gain_db: Option<f32>,
}

#[derive(Subcommand, Debug)]
pub enum GraphAction {
    /// Print the current graph as JSON.
    Show,
    /// Check a graph file without applying it.
    Validate { file: PathBuf },
    /// Apply a graph file.
    Set { file: PathBuf },
    /// Apply the empty graph (everything goes silent).
    Clear,
}

#[derive(Debug)]
pub enum CliError {
    Client(ClientError),
    Other(String),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::Client(ClientError::Rpc(e)) => {
                write!(f, "{}", e.message)?;
                if let Some(list) = e.data.as_ref().and_then(|d| d.as_array()) {
                    for item in list.iter().filter_map(|i| i.as_str()) {
                        write!(f, "\n  - {item}")?;
                    }
                }
                Ok(())
            }
            CliError::Client(e) => write!(f, "{e}"),
            CliError::Other(m) => f.write_str(m),
        }
    }
}

impl From<ClientError> for CliError {
    fn from(e: ClientError) -> Self {
        CliError::Client(e)
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError::Other(e.to_string())
    }
}

fn client_for(cli: &Cli) -> Result<Client, CliError> {
    Ok(match (&cli.address, &cli.token) {
        (Some(a), Some(t)) => Client::new(a.clone(), t.clone()),
        _ => Client::discover()?,
    })
}

/// The graph `route` builds: application -> (gain) -> destination.
pub fn route_graph(args: &RouteArgs) -> GraphSpec {
    let target = match args.app.parse::<u32>() {
        Ok(pid) => AppTarget::Pid(pid),
        Err(_) => AppTarget::ProcessName(args.app.clone()),
    };
    let destination = match (&args.mic, &args.device) {
        (Some(name), _) => NodeKind::VirtualMic { name: name.clone() },
        (None, Some(device)) => NodeKind::DeviceRender {
            device: DeviceRef::Name(device.clone()),
        },
        (None, None) => NodeKind::VirtualMic {
            name: crate::branding::DISPLAY_NAME.into(),
        },
    };
    let mut nodes = vec![NodeSpec {
        id: "app".into(),
        display: None,
        kind: NodeKind::AppCapture { target },
    }];
    let mut edges = vec![];
    let mut last = "app";
    if let Some(db) = args.gain_db {
        nodes.push(NodeSpec {
            id: "gain".into(),
            display: None,
            kind: NodeKind::Gain { db },
        });
        edges.push(EdgeSpec {
            from: PortRef::new(last, "out"),
            to: PortRef::new("gain", "in"),
        });
        last = "gain";
    }
    nodes.push(NodeSpec {
        id: "out".into(),
        display: None,
        kind: destination,
    });
    edges.push(EdgeSpec {
        from: PortRef::new(last, "out"),
        to: PortRef::new("out", "in"),
    });
    GraphSpec { nodes, edges }
}

fn db(peak: f32) -> String {
    if peak <= 0.00001 {
        "-inf".into()
    } else {
        format!("{:.0}", 20.0 * peak.log10())
    }
}

fn print_json<T: serde::Serialize>(out: &mut dyn Write, v: &T) -> Result<(), CliError> {
    writeln!(
        out,
        "{}",
        serde_json::to_string_pretty(v).map_err(|e| CliError::Other(e.to_string()))?
    )?;
    Ok(())
}

/// Parses `args` (the first one is the program name) and runs the command, writing to `out`.
pub fn execute<I, T>(args: I, out: &mut dyn Write) -> Result<(), CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(c) => c,
        Err(e) => {
            // --help and --version are "errors" for clap: print them and succeed; real mistakes fail.
            let text = e.to_string();
            let ok = !e.use_stderr();
            writeln!(out, "{}", text.trim_end())?;
            return if ok {
                Ok(())
            } else {
                Err(CliError::Other("invalid arguments".into()))
            };
        }
    };
    let client = client_for(&cli)?;
    match &cli.command {
        Command::Status => {
            let info = client.call::<SystemInfoMethod>(&())?;
            let status = client.call::<EngineStatusMethod>(&())?;
            if cli.json {
                return print_json(out, &serde_json::json!({ "info": info, "status": status }));
            }
            writeln!(
                out,
                "{} {} (protocol {}) on {}, audio backend: {}",
                info.name, info.version, info.protocol_version, info.platform, info.backend
            )?;
            match &info.virtual_microphones {
                Some(p) if p.available => writeln!(out, "virtual microphones: {} (ready)", p.name)?,
                Some(p) => writeln!(
                    out,
                    "virtual microphones: {} (not available: {})",
                    p.name,
                    p.error.as_deref().unwrap_or("unknown")
                )?,
                None => writeln!(out, "virtual microphones: none on this system")?,
            }
            if info.previous_run_unclean {
                writeln!(
                    out,
                    "warning: the previous run of the daemon did not stop cleanly{}",
                    info.crash_report
                        .as_deref()
                        .map(|p| format!(" (crash report: {p})"))
                        .unwrap_or_default()
                )?;
            }
            writeln!(
                out,
                "engine: {}",
                if status.running { "running" } else { "stopped" }
            )?;
            for n in &status.nodes {
                let state = match &n.state {
                    NodeState::Running => "running".to_string(),
                    NodeState::Failed(m) => format!("FAILED: {m}"),
                };
                writeln!(
                    out,
                    "  {:<12} {:<10} level {:>5} dB  buffer {:>5.1} ms  underruns {}  overruns {}",
                    n.id.0,
                    state,
                    db(n.peak),
                    n.fill_ms,
                    n.underruns,
                    n.overruns
                )?;
            }
        }
        Command::Devices => {
            let devices = client.call::<DevicesList>(&())?;
            if cli.json {
                return print_json(out, &devices);
            }
            for d in devices {
                let fmt = d
                    .format
                    .map(|f| format!("{} Hz, {} ch", f.sample_rate, f.channels))
                    .unwrap_or_default();
                writeln!(
                    out,
                    "{:<8} {}{}  [{}]  id {}",
                    format!("{:?}", d.direction).to_lowercase(),
                    d.name,
                    if d.is_default { "  (default)" } else { "" },
                    fmt,
                    d.id
                )?;
            }
        }
        Command::Apps => {
            let apps = client.call::<AppsList>(&())?;
            if cli.json {
                return print_json(out, &apps);
            }
            for a in apps {
                writeln!(
                    out,
                    "{:>7}  {:<24} {}{}",
                    a.pid,
                    a.name,
                    if a.active { "playing" } else { "idle   " },
                    a.title.map(|t| format!("  \"{t}\"")).unwrap_or_default()
                )?;
            }
        }
        Command::Route(args) => {
            let graph = route_graph(args);
            client.call::<GraphSet>(&graph)?;
            writeln!(
                out,
                "routing {} -> {}",
                args.app,
                args.mic
                    .as_ref()
                    .map(|m| format!("virtual microphone \"{m}\""))
                    .or_else(|| args.device.clone())
                    .unwrap_or_default()
            )?;
        }
        Command::Graph { action } => match action {
            GraphAction::Show => print_json(out, &client.call::<GraphGet>(&())?)?,
            GraphAction::Validate { file } => {
                let graph = read_graph(file)?;
                let r = client.call::<GraphValidate>(&graph)?;
                if cli.json {
                    return print_json(out, &r);
                }
                if r.ok {
                    writeln!(out, "the graph is valid")?;
                } else {
                    return Err(CliError::Other(format!(
                        "the graph is invalid:\n  - {}",
                        r.errors.join("\n  - ")
                    )));
                }
            }
            GraphAction::Set { file } => {
                client.call::<GraphSet>(&read_graph(file)?)?;
                writeln!(out, "graph applied")?;
            }
            GraphAction::Clear => {
                client.call::<GraphSet>(&GraphSpec::default())?;
                writeln!(out, "graph cleared")?;
            }
        },
        Command::Stop => {
            client.call::<EngineStop>(&())?;
            writeln!(out, "engine stopped")?;
        }
        Command::Shutdown => {
            client.call::<SystemShutdown>(&())?;
            writeln!(out, "the daemon is stopping")?;
        }
        Command::Logs(args) => {
            let mut lines = client.call::<LogsRecent>(&LogsParams {
                after: None,
                limit: Some(args.lines),
            })?;
            let mut last = 0;
            loop {
                for line in &lines {
                    if cli.json {
                        writeln!(out, "{}", serde_json::to_string(line).unwrap_or_default())?;
                    } else {
                        writeln!(out, "{}", line.text)?;
                    }
                    last = last.max(line.seq);
                }
                out.flush()?;
                if !args.follow {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
                lines = client.call::<LogsRecent>(&LogsParams {
                    after: Some(last),
                    limit: Some(500),
                })?;
            }
        }
        Command::Panel { open } => {
            let url = client.panel_url();
            writeln!(out, "{url}")?;
            if *open {
                open_in_browser(&url)?;
            }
        }
    }
    Ok(())
}

fn read_graph(file: &std::path::Path) -> Result<GraphSpec, CliError> {
    let text = std::fs::read_to_string(file)
        .map_err(|e| CliError::Other(format!("{}: {e}", file.display())))?;
    serde_json::from_str(&text)
        .map_err(|e| CliError::Other(format!("{}: not a graph: {e}", file.display())))
}

fn open_in_browser(url: &str) -> Result<(), CliError> {
    #[cfg(windows)]
    let status = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .status();
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").arg(url).status();
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let status = std::process::Command::new("xdg-open").arg(url).status();
    status
        .map(|_| ())
        .map_err(|e| CliError::Other(format!("cannot open the browser: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(app: &str, mic: Option<&str>, device: Option<&str>, gain: Option<f32>) -> RouteArgs {
        RouteArgs {
            app: app.into(),
            mic: mic.map(Into::into),
            device: device.map(Into::into),
            gain_db: gain,
        }
    }

    #[test]
    fn route_builds_app_to_virtual_mic() {
        let g = route_graph(&args("chrome.exe", Some("Stream"), None, None));
        assert_eq!(g.nodes.len(), 2);
        assert!(
            matches!(&g.nodes[0].kind, NodeKind::AppCapture { target: AppTarget::ProcessName(n) } if n == "chrome.exe")
        );
        assert!(matches!(&g.nodes[1].kind, NodeKind::VirtualMic { name } if name == "Stream"));
        assert!(crate::graph::validate(&g).is_ok());
    }

    #[test]
    fn a_number_is_a_process_id_and_a_gain_adds_a_node() {
        let g = route_graph(&args("4242", None, Some("Speakers"), Some(-6.0)));
        assert!(matches!(
            &g.nodes[0].kind,
            NodeKind::AppCapture {
                target: AppTarget::Pid(4242)
            }
        ));
        assert!(matches!(&g.nodes[1].kind, NodeKind::Gain { db } if *db == -6.0));
        assert!(matches!(&g.nodes[2].kind, NodeKind::DeviceRender { .. }));
        assert_eq!(g.edges.len(), 2);
        assert!(crate::graph::validate(&g).is_ok());
    }

    #[test]
    fn decibel_display() {
        assert_eq!(db(0.0), "-inf");
        assert_eq!(db(1.0), "0");
        assert_eq!(db(0.5), "-6");
    }

    #[test]
    fn help_succeeds_and_mistakes_fail_without_needing_a_daemon() {
        let mut out = vec![];
        assert!(execute(["omu-cli", "--help"], &mut out).is_ok());
        assert!(String::from_utf8(out).unwrap().contains("route"));
        let mut out = vec![];
        assert!(
            execute(["omu-cli", "route"], &mut out).is_err(),
            "route needs --app"
        );
        let mut out = vec![];
        assert!(
            execute(
                ["omu-cli", "route", "--app", "x", "--mic", "a", "--device", "b"],
                &mut out
            )
            .is_err(),
            "mic and device exclude each other"
        );
    }
}
