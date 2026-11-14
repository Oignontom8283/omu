//! The command-line client against a real (demo) daemon.

use omu::cli::execute;
use omu::daemon::launch::{Options, Running};
use std::time::{Duration, Instant};

fn run(running: &Running, extra: &[&str]) -> Result<String, String> {
    let address = running.info.address.to_string();
    let mut args: Vec<&str> = vec![
        "omu-cli",
        "--address",
        &address,
        "--token",
        &running.info.token,
    ];
    args.extend_from_slice(extra);
    let mut out = vec![];
    execute(args, &mut out)
        .map(|_| String::from_utf8(out).unwrap())
        .map_err(|e| e.to_string())
}

#[test]
fn the_commands_drive_a_daemon() {
    let dir = std::env::temp_dir().join(format!("omu-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut opts = Options::new(dir.clone());
    opts.listen = "127.0.0.1:0".parse().unwrap();
    opts.demo = true;
    let running = Running::start(opts).unwrap();

    let status = run(&running, &["status"]).unwrap();
    assert!(
        status.contains("simulated") && status.contains("engine: stopped"),
        "{status}"
    );
    assert!(run(&running, &["apps"]).unwrap().contains("chrome.exe"));
    assert!(run(&running, &["devices"])
        .unwrap()
        .contains("Demo Microphone"));

    let routed = run(
        &running,
        &[
            "route",
            "--app",
            "chrome.exe",
            "--mic",
            "Stream",
            "--gain-db",
            "-6",
        ],
    )
    .unwrap();
    assert!(routed.contains("virtual microphone"), "{routed}");
    let start = Instant::now();
    loop {
        let s = run(&running, &["status"]).unwrap();
        if s.contains("engine: running")
            && s.contains("out")
            && !s.contains("FAILED")
            && s.lines().any(|l| l.contains("app") && !l.contains("-inf"))
        {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "never flowed: {s}"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
    let json = run(&running, &["--json", "graph", "show"]).unwrap();
    assert!(
        json.contains("\"gain\"") && json.contains("virtual_mic"),
        "{json}"
    );

    // a bad graph file is reported with its problems
    let bad = dir.join("bad.json");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &bad,
        r#"{"nodes":[{"id":"a","type":"mixer"},{"id":"a","type":"mixer"}]}"#,
    )
    .unwrap();
    let err = run(&running, &["graph", "validate", bad.to_str().unwrap()]).unwrap_err();
    assert!(
        err.contains("invalid") && err.contains("used twice"),
        "{err}"
    );
    assert!(run(&running, &["graph", "set", bad.to_str().unwrap()])
        .unwrap_err()
        .contains("used twice"));

    assert!(run(&running, &["graph", "clear"]).is_ok());
    assert!(run(&running, &["stop"]).unwrap().contains("stopped"));
    assert!(run(&running, &["panel"]).unwrap().contains("#token="));
    running.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn without_a_daemon_the_error_says_how_to_start_one() {
    let mut out = vec![];
    // nothing listens on this port
    let err = execute(
        [
            "omu-cli",
            "--address",
            "127.0.0.1:1",
            "--token",
            "x",
            "status",
        ],
        &mut out,
    )
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("cannot reach the daemon") && err.contains(omu::branding::DAEMON_EXECUTABLE),
        "{err}"
    );
}
