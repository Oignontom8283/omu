//! The desktop window: the web panel in a native window.
//!
//! The panel does not open a network connection itself here. It calls the commands below, which use the same
//! [`omu::client`] as the command line to reach the daemon found through its discovery file. So the window works
//! with the daemon of this machine without any browser-origin or token handling in the page, and the page and the
//! command line cannot drift apart in how they talk to the daemon.

use omu::client::{Client, ClientError};
use omu::discovery::data_dir;
use omu::protocol::Request;
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
    let client = Client::discover_in(&data_dir()).map_err(from_client)?;
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
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    start_daemon_from(exe.parent().ok_or("this program has no folder")?)
}

/// Starts the daemon executable from folder `dir`.
pub fn start_daemon_from(dir: &std::path::Path) -> Result<(), String> {
    let daemon = dir.join(format!(
        "{}{}",
        omu::branding::DAEMON_EXECUTABLE,
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
    command.spawn().map(|_| ()).map_err(|e| e.to_string())
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

#[tauri::command]
async fn launch_daemon() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(start_daemon)
        .await
        .map_err(|e| e.to_string())?
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![rpc, launch_daemon])
        .run(tauri::generate_context!())
        .expect("error while running the desktop window");
}

#[cfg(test)]
mod tests {
    use super::*;
    use omu::daemon::launch::{Options, Running};

    #[test]
    fn calls_reach_the_daemon_and_errors_keep_their_shape() {
        let dir = std::env::temp_dir().join(format!("omu-gui-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut opts = Options::new(dir.clone());
        opts.listen = "127.0.0.1:0".parse().unwrap();
        opts.demo = true;
        let running = Running::start(opts).unwrap();
        // `call` finds the daemon through the data directory, like the real window does
        std::env::set_var(omu::branding::env_var("DATA_DIR"), &dir);

        let info = call("system.info", Value::Null).unwrap();
        assert_eq!(info["backend"], "simulated");
        let err = call("graph.set", serde_json::json!({"nodes": 3})).unwrap_err();
        assert_eq!(err.code, omu::protocol::RpcError::INVALID_PARAMS);
        let missing = call("nope", Value::Null).unwrap_err();
        assert_eq!(missing.code, omu::protocol::RpcError::METHOD_NOT_FOUND);

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
            e.contains(omu::branding::DAEMON_EXECUTABLE) && e.contains("not found"),
            "{e}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
