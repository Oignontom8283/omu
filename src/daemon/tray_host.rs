//! The daemon's side of the notification-area icon: what its click and its menu do, and showing or hiding it when the
//! setting changes.

use super::Daemon;
use omu_desktop::tray::{Actions, Tray};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};

pub struct TrayHost {
    daemon: Weak<Daemon>,
    /// The address of the web panel, with the access token.
    panel_url: String,
    tray: Mutex<Option<Tray>>,
}

impl TrayHost {
    pub fn new(daemon: &Arc<Daemon>, panel_url: String) -> Arc<TrayHost> {
        Arc::new(TrayHost {
            daemon: Arc::downgrade(daemon),
            panel_url,
            tray: Mutex::new(None),
        })
    }

    /// Shows the icon or takes it away. An icon that cannot be made is logged and the daemon goes on without it.
    pub fn set_visible(&self, show: bool) {
        let mut tray = self.tray.lock().unwrap_or_else(|e| e.into_inner());
        if !show {
            *tray = None;
            return;
        }
        if tray.is_some() {
            return;
        }
        let (open_url, stop_daemon, start_daemon, quit_daemon) = (
            self.panel_url.clone(),
            self.daemon.clone(),
            self.daemon.clone(),
            self.daemon.clone(),
        );
        let actions = Actions {
            open: Box::new(move || {
                let url = open_url.clone();
                std::thread::spawn(move || {
                    if let Err(e) = open_interface(&url) {
                        tracing::warn!("cannot open the interface: {e}");
                    }
                });
            }),
            start_engine: Box::new(move || {
                if let Some(d) = start_daemon.upgrade() {
                    std::thread::spawn(move || {
                        let _ =
                            d.handle_text(r#"{"jsonrpc":"2.0","id":1,"method":"engine.start"}"#);
                    });
                }
            }),
            stop_engine: Box::new(move || {
                if let Some(d) = stop_daemon.upgrade() {
                    std::thread::spawn(move || {
                        let _ = d.handle_text(r#"{"jsonrpc":"2.0","id":1,"method":"engine.stop"}"#);
                    });
                }
            }),
            quit: Box::new(move || {
                if let Some(d) = quit_daemon.upgrade() {
                    d.stop_signal().notify_one();
                }
            }),
        };
        match Tray::start(actions) {
            Ok(t) => {
                tracing::info!("the notification-area icon is shown");
                *tray = Some(t);
            }
            Err(e) => tracing::warn!("no notification-area icon: {e}"),
        }
    }
}

/// The desktop window when it is installed next to the daemon, else the web panel in the browser.
pub fn open_interface(panel_url: &str) -> Result<(), String> {
    if let Some(window) = window_executable() {
        return spawn_detached(std::process::Command::new(window));
    }
    open_target(panel_url)
}

/// Opens an address, a file or a folder the way the system does for the user (the browser, the file manager...).
pub fn open_target(target: &str) -> Result<(), String> {
    #[cfg(windows)]
    let command = {
        // not `cmd /C start`: it would read an `&` in the address as another command
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", target]);
        c
    };
    #[cfg(target_os = "macos")]
    let command = {
        let mut c = std::process::Command::new("open");
        c.arg(target);
        c
    };
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let command = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(target);
        c
    };
    spawn_detached(command)
}

/// The desktop window's executable, when it sits next to this one.
fn window_executable() -> Option<PathBuf> {
    let me = std::env::current_exe().ok()?;
    let name = format!(
        "{}{}",
        omu_branding::GUI_EXECUTABLE,
        std::env::consts::EXE_SUFFIX
    );
    let path = me.with_file_name(name);
    path.is_file().then_some(path)
}

fn spawn_detached(mut command: std::process::Command) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // no console window
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("cannot start {:?}: {e}", command.get_program()))
}
