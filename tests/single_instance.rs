//! Only one daemon at a time, checked with the real program: a second one in the same data folder refuses to start,
//! says why and leaves the first one alone; a demo daemon elsewhere is not in its way; and once the first is gone (even
//! killed, with no chance to clean up) the next one starts.

use omu::client::Client;
use omu::protocol::methods::SystemInfoMethod;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("omu-single-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn start(dir: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_omu-daemon"))
        .args([
            "--demo",
            "--quiet",
            "--no-restore",
            "--listen",
            "127.0.0.1:0",
        ])
        .arg("--data-dir")
        .arg(dir)
        .env(omu::branding::env_var("NO_DIALOG"), "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the daemon starts")
}

fn wait_for<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(v) = check() {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn serving(dir: &Path) -> Option<omu::protocol::methods::SystemInfo> {
    Client::discover_in(dir)
        .ok()?
        .call::<SystemInfoMethod>(&())
        .ok()
}

#[test]
fn a_second_daemon_stops_with_a_clear_message_and_the_first_keeps_running() {
    let dir = temp("same");
    let mut first = start(&dir);
    wait_for("the first daemon", || serving(&dir));
    let first_pid = first.id();

    let mut second = start(&dir);
    let status = wait_for("the second daemon to give up", || {
        second.try_wait().unwrap()
    });
    assert_eq!(
        status.code(),
        Some(2),
        "a refused start has its own exit code"
    );
    let mut message = String::new();
    std::io::Read::read_to_string(&mut second.stderr.take().unwrap(), &mut message).unwrap();
    assert!(
        message.contains("already running") && message.contains(&format!("process {first_pid}")),
        "the message names the running daemon:\n{message}"
    );

    // the first one was not disturbed: still there, still the one clients are sent to
    assert!(first.try_wait().unwrap().is_none());
    assert!(serving(&dir).is_some());
    let announced = omu::discovery::Discovery::read(&dir).expect("discovery file");
    assert_eq!(announced.pid, first_pid);

    // killed from outside (no cleanup at all): the next one can start
    first.kill().unwrap();
    let _ = first.wait();
    let mut third = start(&dir);
    wait_for("the third daemon", || serving(&dir));
    assert!(third.try_wait().unwrap().is_none());
    third.kill().unwrap();
    let _ = third.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn demo_daemons_of_different_folders_do_not_get_in_each_others_way() {
    let (a, b) = (temp("a"), temp("b"));
    let mut one = start(&a);
    let mut two = start(&b);
    wait_for("the first demo", || serving(&a));
    wait_for("the second demo", || serving(&b));
    assert!(one.try_wait().unwrap().is_none() && two.try_wait().unwrap().is_none());
    for c in [&mut one, &mut two] {
        c.kill().unwrap();
        let _ = c.wait();
    }
    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);
}
