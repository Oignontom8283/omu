//! Logs: everything that happens, as it happens.
//!
//! One event goes to up to three places:
//!
//! * a log file in the data folder (`logs/<program>.<date>.log`, one file a day, the last seven kept), written by a
//!   background thread so that logging never waits for the disk;
//! * the console (stderr), when the program has one and the user asked for it;
//! * a ring of the last lines in memory, which crash reports and the `logs.recent` request read.
//!
//! The level is chosen with the `OMU_LOG` environment variable (the prefix comes from the branding), in the usual
//! `tracing` filter syntax: `debug`, `warn,omu::engine=trace`... Without it the file keeps `info` and the details of
//! this program (`debug`), the console shows `info`.

use super::ring;
use std::io;
use std::path::Path;
use std::sync::Mutex;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Layer};

/// Days of log files kept.
const KEEP_FILES: usize = 7;

static FILE_GUARD: Mutex<Option<WorkerGuard>> = Mutex::new(None);

/// A formatted event on its way into the ring: the line is complete when the writer is dropped.
struct RingLine(Vec<u8>);

impl io::Write for RingLine {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for RingLine {
    fn drop(&mut self) {
        let text = String::from_utf8_lossy(&self.0).trim_end().to_owned();
        if !text.is_empty() {
            ring::push(text);
        }
    }
}

struct RingWriter;

impl<'a> MakeWriter<'a> for RingWriter {
    type Writer = RingLine;
    fn make_writer(&'a self) -> RingLine {
        RingLine(Vec::with_capacity(160))
    }
}

fn filter(default: &str) -> EnvFilter {
    EnvFilter::try_from_env(crate::branding::env_var("LOG"))
        .unwrap_or_else(|_| EnvFilter::new(default))
}

/// Starts the logging. Returns what went wrong (the program goes on without a log file in that case).
pub(super) fn init(program: &str, log_dir: &Path, console: bool) -> Result<(), String> {
    // the appender reads the folder to prune old files: it must exist from the first run
    let _ = std::fs::create_dir_all(log_dir);
    let file = match Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix(program)
        .filename_suffix("log")
        .max_log_files(KEEP_FILES)
        .build(log_dir)
    {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            *FILE_GUARD.lock().unwrap_or_else(|e| e.into_inner()) = Some(guard);
            Some(
                fmt::layer()
                    .with_writer(writer)
                    .with_ansi(false)
                    .with_thread_names(true)
                    .with_file(true)
                    .with_line_number(true)
                    .with_filter(filter("info,omu=debug")),
            )
        }
        Err(e) => {
            eprintln!("warning: no log file in {}: {e}", log_dir.display());
            None
        }
    };
    let ring_layer = fmt::layer()
        .with_writer(RingWriter)
        .with_ansi(false)
        .with_thread_names(true)
        .with_filter(filter("info,omu=debug"));
    let console_layer = console.then(|| {
        use io::IsTerminal;
        fmt::layer()
            .with_writer(io::stderr)
            .with_ansi(io::stderr().is_terminal())
            .with_target(false)
            .compact()
            .with_filter(filter("info"))
    });
    tracing_subscriber::registry()
        .with(file)
        .with(ring_layer)
        .with(console_layer)
        .try_init()
        .map_err(|e| e.to_string())
}

/// Waits until the background thread has written everything (up to a second); logging then stops working.
pub(super) fn flush() {
    let guard = FILE_GUARD.lock().unwrap_or_else(|e| e.into_inner()).take();
    drop(guard);
}
