//! Asking the real daemon to stop (`system.shutdown`, what the panel's button and `omu-cli shutdown` use): it must
//! answer, then leave cleanly even though a panel keeps its WebSocket open, and leave nothing behind.

use omu_client::Client;
use omu_protocol::methods::{SystemInfoMethod, SystemShutdown};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("omu-shutdown-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn start(dir: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_omud"))
        .args([
            "--demo",
            "--quiet",
            "--no-restore",
            "--listen",
            "127.0.0.1:0",
        ])
        .arg("--data-dir")
        .arg(dir)
        .env(omu_branding::env_var("NO_DIALOG"), "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the daemon starts")
}

fn wait_for<T>(what: &str, limit: Duration, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + limit;
    loop {
        if let Some(v) = check() {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_shutdown_request_stops_the_daemon_cleanly_even_with_a_panel_connected() {
    let dir = temp("clean");
    let mut daemon = start(&dir);
    let client = wait_for("the daemon", Duration::from_secs(30), || {
        let c = Client::discover_in(&dir).ok()?;
        c.call::<SystemInfoMethod>(&()).ok()?;
        Some(c)
    });
    assert!(
        dir.join("omud.session").exists(),
        "a running daemon keeps its session marker"
    );

    // a panel that stays connected, like a browser tab
    let token = Client::discover_in(&dir).unwrap();
    let url = format!(
        "ws://{}/ws?token={}",
        token.address(),
        omu_discovery::Discovery::read(&dir).unwrap().token
    );
    let (_panel, _) = tungstenite::connect(url).expect("the panel connects");

    client
        .call::<SystemShutdown>(&())
        .expect("the daemon answers before it stops");
    let status = wait_for("the daemon to exit", Duration::from_secs(15), || {
        daemon.try_wait().unwrap()
    });
    assert_eq!(status.code(), Some(0), "a requested stop is a clean exit");
    assert!(
        !dir.join("daemon.json").exists(),
        "the discovery file is removed"
    );
    assert!(
        !dir.join("omud.session").exists(),
        "the next start must not think this one crashed"
    );
    assert!(
        Client::discover_in(&dir).is_err(),
        "nobody is left to talk to"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
