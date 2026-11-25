//! What the real daemon does when it breaks: it is started as a program, made to fail on purpose (the hidden
//! `--crash-test` option) and observed from outside, like a user or the operating system would.
//!
//! Each failure must leave: a crash report naming the cause and the last log lines, the log file written to the end,
//! the discovery file removed (clients must not be sent to a dead daemon), and a clear exit code.

use omu_client::Client;
use omu_protocol::methods::{LogsParams, LogsRecent, SystemInfoMethod};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("omu-crash-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn start(dir: &Path, crash_test: Option<&str>) -> Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_omud"));
    command
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
        .stderr(Stdio::null());
    if let Some(kind) = crash_test {
        command.args(["--crash-test", kind]);
    }
    command.spawn().expect("the daemon starts")
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

fn exit_code(child: &mut Child) -> i32 {
    wait_for("the daemon to exit", || child.try_wait().unwrap())
        .code()
        .expect("exited with a code")
}

fn files_with(dir: &Path, sub: &str, extension: &str) -> Vec<PathBuf> {
    std::fs::read_dir(dir.join(sub))
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == extension))
                .collect()
        })
        .unwrap_or_default()
}

fn read_all(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| std::fs::read_to_string(p).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Checks everything a fatal failure must leave behind; returns the crash report text.
fn assert_clean_death(dir: &Path, cause: &str) -> String {
    let reports = files_with(dir, "crashes", "txt");
    assert_eq!(reports.len(), 1, "one crash report expected");
    let report = std::fs::read_to_string(&reports[0]).unwrap();
    assert!(
        report.contains(cause),
        "the report names the cause:\n{report}"
    );
    assert!(
        report.contains("last ") && report.contains("control port open"),
        "the report carries the last log lines:\n{report}"
    );
    assert!(
        report.contains("cleanups still pending: discovery file"),
        "the report says what had to be undone:\n{report}"
    );
    assert!(
        !dir.join("run").join("daemon.json").exists(),
        "the discovery file must be removed by the crash cleanup"
    );
    let log = read_all(&files_with(dir, "logs", "log"));
    assert!(
        log.contains("control port open") && log.contains(cause),
        "the log file is complete up to the crash:\n{log}"
    );
    report
}

#[test]
fn a_panic_with_no_guard_stops_the_daemon_cleanly() {
    let dir = temp("panic");
    let mut child = start(&dir, Some("panic"));
    assert_eq!(exit_code(&mut child), 101);
    let report = assert_clean_death(&dir, "crash test: panic on the main thread");
    assert!(report.contains("panic (fatal)"));
    assert!(report.contains("graph: ") || report.contains("backend: simulated"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_panic_on_a_worker_thread_also_stops_it_instead_of_leaving_it_half_dead() {
    let dir = temp("thread");
    let mut child = start(&dir, Some("thread-panic"));
    assert_eq!(exit_code(&mut child), 101);
    let report = assert_clean_death(&dir, "crash test: panic on a worker thread");
    assert!(
        report.contains("crash-test"),
        "the thread is named:\n{report}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(windows)]
#[test]
fn a_native_crash_leaves_a_report_and_a_minidump() {
    let dir = temp("violation");
    let mut child = start(&dir, Some("violation"));
    assert_eq!(exit_code(&mut child), 0xC000_0005_u32 as i32);
    let report = assert_clean_death(&dir, "access violation writing address 0x0");
    assert!(report.contains("native exception (fatal)"));
    let dumps = files_with(&dir, "crashes", "dmp");
    assert_eq!(dumps.len(), 1, "a minidump is written");
    assert!(std::fs::metadata(&dumps[0]).unwrap().len() > 1000);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_guarded_panic_is_reported_but_the_daemon_keeps_serving_and_a_kill_is_noticed_next_time() {
    let dir = temp("contained");
    let mut child = start(&dir, Some("contained"));
    let client = wait_for("the daemon to answer", || {
        let c = Client::discover_in(&dir).ok()?;
        c.call::<SystemInfoMethod>(&()).ok()?;
        Some(c)
    });
    // the report exists, the daemon is alive, and its logs can be read through the protocol
    let reports = wait_for("the crash report", || {
        let r = files_with(&dir, "crashes", "txt");
        (!r.is_empty()).then_some(r)
    });
    assert!(read_all(&reports).contains("panic (contained)"));
    assert!(child.try_wait().unwrap().is_none(), "still running");
    let lines = client
        .call::<LogsRecent>(&LogsParams::default())
        .expect("logs.recent");
    let text: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
    assert!(
        text.iter()
            .any(|l| l.contains("the contained panic was handled")),
        "live logs: {text:#?}"
    );
    // following: nothing new after the last line, then something new after a request is logged
    let last = lines.last().unwrap().seq;
    let none = client
        .call::<LogsRecent>(&LogsParams {
            after: Some(last),
            limit: None,
        })
        .unwrap();
    assert!(none.iter().all(|l| l.seq > last));

    // killed from outside (no chance to clean up): the next start notices
    child.kill().unwrap();
    let _ = child.wait();
    let _ = std::fs::remove_file(dir.join("run").join("daemon.json"));
    let mut again = start(&dir, None);
    let info = wait_for("the second daemon to answer", || {
        Client::discover_in(&dir)
            .ok()?
            .call::<SystemInfoMethod>(&())
            .ok()
    });
    assert!(info.previous_run_unclean, "the kill was noticed");
    assert!(
        info.crash_report.is_some(),
        "and the report of that run is pointed to"
    );
    again.kill().unwrap();
    let _ = again.wait();
    let _ = std::fs::remove_dir_all(&dir);
}
