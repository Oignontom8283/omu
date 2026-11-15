//! The desktop window: the web panel in a native window.
//!
//! The panel does not open a network connection itself here. It calls the commands below, which use the same
//! [`crate::client`] as the command line to reach the daemon found through its discovery file. So the window works
//! with the daemon of this machine without any browser-origin or token handling in the page, and the page and the
//! command line cannot drift apart in how they talk to the daemon.

use crate::client::{Client, ClientError};
use crate::discovery::data_dir;
use crate::protocol::Request;
use serde::Serialize;
use serde_json::Value;
use std::sync::atomic::{AtomicI64, Ordering};

/// An error as the page receives it (same shape as a JSON-RPC error).
#[derive(Debug, Serialize, PartialEq)]
pub struct IpcError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
}

/// Code of "no daemon running" errors, which are not JSON-RPC errors (the page shows a Start button for them).
pub const NO_DAEMON: i32 = -1;

fn from_client(e: ClientError) -> IpcError {
    match e {
        ClientError::Rpc(e) => IpcError {
            code: e.code,
            message: e.message,
            data: e.data,
        },
        ClientError::NoDaemon(m) => IpcError {
            code: NO_DAEMON,
            message: m,
            data: None,
        },
        ClientError::Transport(m) => IpcError {
            code: NO_DAEMON,
            message: m,
            data: None,
        },
    }
}

/// Calls a daemon method on behalf of the page. The discovery file is read every time, so a daemon that was
/// restarted (new token, new port) is found again without restarting the window.
pub fn call(method: &str, params: Value) -> Result<Value, IpcError> {
    static IDS: AtomicI64 = AtomicI64::new(1);
    let client = Client::discover_in(&data_dir()).map_err(|e| {
        tracing::debug!(%method, "no daemon found: {e:?}");
        from_client(e)
    })?;
    let response = client
        .send(&Request::new(
            IDS.fetch_add(1, Ordering::Relaxed),
            method,
            params,
        ))
        .map_err(from_client)?;
    match response.error {
        Some(e) => Err(from_client(ClientError::Rpc(e))),
        None => Ok(response.result.unwrap_or(Value::Null)),
    }
}

/// Starts the daemon executable that sits next to this one, detached (it keeps running when the window closes).
pub fn start_daemon() -> Result<(), String> {
    // one daemon at a time: if one runs, it is the one the window will talk to
    if let Some(d) = crate::discovery::Discovery::read(&data_dir()) {
        if crate::diagnostics::session::pid_alive(d.pid) {
            tracing::info!(
                pid = d.pid,
                "a daemon is already running; not starting another"
            );
            return Ok(());
        }
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    start_daemon_from(exe.parent().ok_or("this program has no folder")?)
}

/// Starts the daemon executable from folder `dir`.
pub fn start_daemon_from(dir: &std::path::Path) -> Result<(), String> {
    let daemon = dir.join(format!(
        "{}{}",
        crate::branding::DAEMON_EXECUTABLE,
        std::env::consts::EXE_SUFFIX
    ));
    if !daemon.exists() {
        return Err(format!(
            "{} not found next to this program",
            daemon.display()
        ));
    }
    let mut command = std::process::Command::new(daemon);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
    }
    tracing::info!("starting the daemon");
    command.spawn().map(|_| ()).map_err(|e| {
        tracing::error!("cannot start the daemon: {e}");
        e.to_string()
    })
}

#[tauri::command]
async fn rpc(method: String, params: Value) -> Result<Value, IpcError> {
    tauri::async_runtime::spawn_blocking(move || call(&method, params))
        .await
        .map_err(|e| IpcError {
            code: -2,
            message: e.to_string(),
            data: None,
        })?
}

/// The file extensions the sound dialog offers first: what the soundboards can import.
const SOUND_EXTENSIONS: &[&str] = &[
    "wav", "mp3", "flac", "ogg", "oga", "aac", "m4a", "mp4", "m4v", "mov", "mkv", "webm",
];

/// Shows the system's dialog to choose sound or video files, and returns the paths (none when it is dismissed).
#[tauri::command]
async fn pick_sound_files() -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .add_filter("Sound and video files", SOUND_EXTENSIONS)
            .add_filter("All files", &["*"])
            .pick_files()
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.display().to_string())
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn launch_daemon() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(start_daemon)
        .await
        .map_err(|e| e.to_string())?
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            rpc,
            launch_daemon,
            pick_sound_files
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| crate::diagnostics::fail(&format!("the window could not run: {e}"), 1));
}

#[cfg(all(test, feature = "daemon"))]
mod tests {
    use super::*;
    use crate::daemon::launch::{Options, Running};

    #[test]
    fn calls_reach_the_daemon_and_errors_keep_their_shape() {
        let dir = std::env::temp_dir().join(format!("omu-gui-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut opts = Options::new(dir.clone());
        opts.listen = "127.0.0.1:0".parse().unwrap();
        opts.demo = true;
        let running = Running::start(opts).unwrap();
        // `call` finds the daemon through the data directory, like the real window does
        std::env::set_var(crate::branding::env_var("DATA_DIR"), &dir);

        let info = call("system.info", Value::Null).unwrap();
        assert_eq!(info["backend"], "simulated");
        let err = call("graph.set", serde_json::json!({"nodes": 3})).unwrap_err();
        assert_eq!(err.code, crate::protocol::RpcError::INVALID_PARAMS);
        let missing = call("nope", Value::Null).unwrap_err();
        assert_eq!(missing.code, crate::protocol::RpcError::METHOD_NOT_FOUND);

        running.stop();
        let gone = call("system.info", Value::Null).unwrap_err();
        assert_eq!(gone.code, NO_DAEMON, "no daemon: {gone:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_daemon_executable_is_looked_for_in_the_given_folder() {
        // an empty folder has no daemon: the error says where it looked (and nothing is started)
        let dir = std::env::temp_dir().join(format!("omu-gui-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let e = start_daemon_from(&dir).unwrap_err();
        assert!(
            e.contains(crate::branding::DAEMON_EXECUTABLE) && e.contains("not found"),
            "{e}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
