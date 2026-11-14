//! What happens when the program breaks.
//!
//! * A panic is always logged and written to a crash report (message, place, thread, backtrace with file and line,
//!   the last log lines, the notes the program keeps about its state).
//! * A panic inside a [`contained`] block (the audio thread's guard, a request handler run by tokio) is only that: the
//!   caller handles it and the program goes on.
//! * Any other panic is **fatal**: [`fatal`] undoes what must be undone ([`super::shutdown::run_all`]), flushes the
//!   logs, tells the user where the report is, and exits with a clear code, instead of limping on with a dead thread.
//!
//! Native crashes (access violation, stack overflow...) take the same road from the Windows exception filter.

use super::{clock, platform, ring, shutdown};
use std::any::Any;
use std::backtrace::Backtrace;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe, PanicHookInfo};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

/// Exit code of a fatal panic (the one Rust itself uses).
pub const EXIT_PANIC: i32 = 101;
/// A fatal problem must not hang the program: after this long it exits whatever the cleanups are doing.
const FATAL_TIME_LIMIT: Duration = Duration::from_secs(8);
/// Crash reports written per run at most (a panic in a loop must not fill the disk).
const MAX_REPORTS: usize = 5;

pub(super) struct Context {
    pub program: &'static str,
    pub crash_dir: PathBuf,
    pub dialog: bool,
    pub started: Instant,
}

static CONTEXT: OnceLock<Context> = OnceLock::new();
static NOTES: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());
static REPORTS: AtomicUsize = AtomicUsize::new(0);
static FATAL: AtomicBool = AtomicBool::new(false);

thread_local! {
    static CONTAINED: Cell<u32> = const { Cell::new(0) };
}

/// Remembers a fact about the program's state for crash reports (the running graph, the audio backend...).
pub fn note(key: &str, value: impl Into<String>) {
    NOTES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.to_owned(), value.into());
}

/// Runs `f`; a panic inside it is logged and reported like any other, but returned to the caller instead of ending
/// the program.
pub fn contained<R>(f: impl FnOnce() -> R) -> Result<R, Box<dyn Any + Send>> {
    CONTAINED.with(|c| c.set(c.get() + 1));
    let result = catch_unwind(AssertUnwindSafe(f));
    CONTAINED.with(|c| c.set(c.get() - 1));
    result
}

/// tokio catches the panics of its tasks and keeps serving: they are contained too (its threads are named
/// `tokio-rt-worker`, or `tokio-runtime-worker` in older versions).
fn is_contained() -> bool {
    CONTAINED.with(|c| c.get() > 0)
        || std::thread::current()
            .name()
            .is_some_and(|n| n.starts_with("tokio-"))
}

pub(super) fn install(context: Context) {
    if CONTEXT.set(context).is_err() {
        return; // already installed
    }
    std::panic::set_hook(Box::new(on_panic));
    platform::install_handlers();
}

/// Used by the Windows exception filter (to find the crash folder).
#[cfg_attr(not(all(windows, feature = "logging")), allow(dead_code))]
pub(super) fn context() -> Option<&'static Context> {
    CONTEXT.get()
}

fn payload_text(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "(a panic with no text)".to_owned()
    }
}

fn on_panic(info: &PanicHookInfo<'_>) {
    let message = payload_text(info.payload());
    let place = info
        .location()
        .map_or_else(|| "unknown place".to_owned(), |l| l.to_string());
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("unnamed").to_owned();
    let contained = is_contained();
    tracing::error!(
        target: "omu::crash",
        thread = %thread_name,
        place = %place,
        contained,
        "panic: {message}"
    );
    let backtrace = trim_backtrace(&Backtrace::force_capture().to_string());
    let report = write_report(&Report {
        kind: if contained {
            "panic (contained)"
        } else {
            "panic (fatal)"
        },
        summary: &message,
        place: &place,
        thread: &thread_name,
        details: &backtrace,
    });
    if !contained {
        fatal(
            &format!("{message}\n(at {place}, thread {thread_name})"),
            report.as_deref(),
            EXIT_PANIC,
        );
    } else if let Some(path) = &report {
        tracing::warn!(target: "omu::crash", "the program goes on; report: {}", path.display());
    }
}

/// Drops the frames of the panic machinery (the hook, `panic_fmt`...) or of Windows' exception dispatch from the top
/// of a backtrace: the first line a reader needs is the one that failed.
pub(super) fn trim_backtrace(backtrace: &str) -> String {
    let is_frame = |line: &str| {
        let t = line.trim_start();
        t.split_once(':')
            .is_some_and(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    };
    let lines: Vec<&str> = backtrace.lines().collect();
    // the last panic-machinery frame among the first few
    let skip_to = lines
        .iter()
        .enumerate()
        .take(40)
        .filter(|(_, l)| {
            is_frame(l)
                && (l.contains("core::panicking::")
                    || l.contains("rust_begin_unwind")
                    || l.contains("std::panicking::begin_panic")
                    || l.contains("KiUserExceptionDispatcher"))
        })
        .map(|(i, _)| i)
        .next_back();
    let Some(start) = skip_to else {
        return backtrace.to_owned();
    };
    // go on to the next frame start (the "at file:line" lines of the skipped frame come first)
    let first = lines[start + 1..]
        .iter()
        .position(|l| is_frame(l))
        .map_or(lines.len(), |p| start + 1 + p);
    lines[first..].join("\n")
}

/// The text of a crash report.
pub(super) struct Report<'a> {
    pub kind: &'a str,
    pub summary: &'a str,
    pub place: &'a str,
    pub thread: &'a str,
    /// Backtrace, exception record...
    pub details: &'a str,
}

/// Writes a report in the crash folder; returns its path. Never panics.
pub(super) fn write_report(report: &Report<'_>) -> Option<PathBuf> {
    let context = CONTEXT.get()?;
    if REPORTS.fetch_add(1, Ordering::SeqCst) >= MAX_REPORTS {
        return None;
    }
    let now = SystemTime::now();
    let path = context.crash_dir.join(format!(
        "crash-{}-{}.txt",
        clock::stamp(now),
        std::process::id()
    ));
    let text = render(context, report, now);
    std::fs::create_dir_all(&context.crash_dir).ok()?;
    std::fs::write(&path, text).ok()?;
    Some(path)
}

fn render(context: &Context, report: &Report<'_>, now: SystemTime) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "{} crash report", crate::branding::DISPLAY_NAME);
    let _ = writeln!(out, "=======================");
    let _ = writeln!(out, "kind:      {}", report.kind);
    let _ = writeln!(out, "message:   {}", report.summary);
    let _ = writeln!(out, "where:     {}", report.place);
    let _ = writeln!(out, "thread:    {}", report.thread);
    let _ = writeln!(
        out,
        "program:   {} {}",
        context.program,
        env!("CARGO_PKG_VERSION")
    );
    let _ = writeln!(out, "time:      {}", clock::iso(now));
    let _ = writeln!(
        out,
        "uptime:    {:.1} s",
        context.started.elapsed().as_secs_f64()
    );
    let _ = writeln!(
        out,
        "system:    {} {}, process {}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::process::id()
    );
    let pending = shutdown::pending();
    let _ = writeln!(
        out,
        "cleanups still pending: {}",
        if pending.is_empty() {
            "none".to_owned()
        } else {
            pending.join(", ")
        }
    );
    let notes = NOTES.try_lock().map(|n| n.clone()).unwrap_or_default();
    if !notes.is_empty() {
        let _ = writeln!(out, "\nstate");
        for (k, v) in notes {
            let _ = writeln!(out, "  {k}: {v}");
        }
    }
    let _ = writeln!(out, "\ndetails\n-------\n{}", report.details.trim_end());
    let lines = ring::recent(200);
    let _ = writeln!(out, "\nlast {} log lines\n-----------------", lines.len());
    for l in lines {
        let _ = writeln!(out, "{}", l.text.trim_end());
    }
    out
}

/// The program cannot go on. Undoes what must be undone, flushes the logs, shows the error to the user and exits.
/// Never returns, never waits longer than a few seconds, and is safe to call twice (the second caller waits for
/// the first to finish the exit).
pub fn fatal(summary: &str, report: Option<&std::path::Path>, exit_code: i32) -> ! {
    if FATAL.swap(true, Ordering::SeqCst) {
        // another thread is already shutting down: do not run the cleanups twice
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    // The watchdog first, so that nothing below can keep the process alive: the cleanups get FATAL_TIME_LIMIT, the
    // error dialog a minute (an unattended machine must not hang on it), and whatever hangs is cut off after both.
    let _ = std::thread::Builder::new()
        .name("fatal-watchdog".into())
        .spawn(move || {
            std::thread::sleep(FATAL_TIME_LIMIT + Duration::from_secs(60));
            std::process::exit(exit_code);
        });
    let cleaned = std::thread::Builder::new()
        .name("fatal-cleanup".into())
        .spawn(|| {
            let n = shutdown::run_all();
            tracing::error!(target: "omu::crash", "fatal error: {n} cleanup(s) run, shutting down");
            super::flush_logs();
        });
    if let Ok(handle) = cleaned {
        let deadline = Instant::now() + FATAL_TIME_LIMIT;
        while !handle.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let program = CONTEXT.get().map_or("the program", |c| c.program);
    let mut text = format!("{program} has to stop because of an internal error.\n\n{summary}\n");
    match report {
        Some(path) => text.push_str(&format!("\nDetails were saved in:\n{}\n", path.display())),
        None => text.push_str("\nNo report could be written.\n"),
    }
    tell_user_and_exit(&text, exit_code)
}

/// The program cannot go on for a reason that is not a bug (the port is taken, the data folder is unusable...):
/// logs it, undoes what must be undone, tells the user (on stderr, and in a dialog when there is no console) and
/// exits with `exit_code`. This is a clean stop: the next start does not report a crash.
pub fn fail(message: &str, exit_code: i32) -> ! {
    tracing::error!("{message}");
    shutdown::run_all();
    super::session::end_current();
    super::flush_logs();
    let program = CONTEXT.get().map_or("the program", |c| c.program);
    tell_user_and_exit(
        &format!("{program} cannot continue.\n\n{message}\n"),
        exit_code,
    )
}

fn tell_user_and_exit(text: &str, exit_code: i32) -> ! {
    eprintln!("\nerror: {text}");
    if CONTEXT.get().is_some_and(|c| c.dialog)
        && std::env::var_os(crate::branding::env_var("NO_DIALOG")).is_none()
    {
        platform::show_error(crate::branding::DISPLAY_NAME, text);
    }
    std::process::exit(exit_code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contained_returns_the_panic_instead_of_ending_the_program() {
        // no hook is installed in unit tests: this checks the guard itself and its counter
        let r = contained(|| -> u32 { panic!("inside") });
        let payload = r.unwrap_err();
        assert_eq!(payload_text(payload.as_ref()), "inside");
        assert!(!is_contained(), "the guard is released after the panic");
        assert_eq!(contained(|| 7).unwrap(), 7);
    }

    #[test]
    fn panics_on_tokio_threads_count_as_contained_because_tokio_catches_them() {
        let named = |name: &str| {
            std::thread::Builder::new()
                .name(name.into())
                .spawn(is_contained)
                .unwrap()
                .join()
                .unwrap()
        };
        assert!(named("tokio-rt-worker"));
        assert!(named("tokio-runtime-worker"));
        assert!(!named("graph"));
    }

    #[test]
    fn a_backtrace_starts_at_the_line_that_panicked() {
        let raw = "   0: std::backtrace::Backtrace::force_capture\n             at lib.rs:1\n   1: omu::crash::on_panic\n             at crash.rs:2\n   2: core::panicking::panic_fmt\n             at core.rs:3\n   3: omu::engine::process\n             at engine.rs:4\n   4: main\n             at main.rs:5";
        let trimmed = trim_backtrace(raw);
        assert!(
            trimmed.starts_with("   3: omu::engine::process"),
            "{trimmed}"
        );
        assert!(trimmed.contains("engine.rs:4") && trimmed.contains("main.rs:5"));
        // nothing to trim: unchanged
        assert_eq!(trim_backtrace("   0: a\n   1: b"), "   0: a\n   1: b");
    }

    #[test]
    fn a_report_names_the_cause_the_place_the_state_and_the_recent_logs() {
        note("test-state", "3 nodes");
        ring::push("an interesting log line".into());
        let context = Context {
            program: "omu-test",
            crash_dir: std::env::temp_dir(),
            dialog: false,
            started: Instant::now(),
        };
        let text = render(
            &context,
            &Report {
                kind: "panic (fatal)",
                summary: "it broke",
                place: "src/x.rs:1:2",
                thread: "graph",
                details: "frame 0\nframe 1\n",
            },
            SystemTime::UNIX_EPOCH,
        );
        for needle in [
            "kind:      panic (fatal)",
            "message:   it broke",
            "where:     src/x.rs:1:2",
            "thread:    graph",
            "omu-test",
            "1970-01-01T00:00:00Z",
            "test-state: 3 nodes",
            "frame 1",
            "an interesting log line",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
        }
    }
}
