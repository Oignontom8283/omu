//! Diagnostics: logs, crash reports and a clean exit when things go wrong.
//!
//! A program calls [`init`] first thing in `main` and keeps the returned [`Guard`] until it exits:
//!
//! * **logs** (feature `logging`): file, console and an in-memory ring, see [`logging`](self);
//! * **crash handling**: panics and (on Windows) native exceptions write a report with backtrace, recent log lines
//!   and state notes, undo what must be undone ([`shutdown`]) and tell the user, see [`crash`];
//! * **session marker**: tells the next start whether this run ended cleanly, see [`session`].
//!
//! Code deep in the program uses [`tracing`]'s macros (`info!`, `warn!`...) directly; nothing is printed unless a
//! program has called [`init`] with logging. The audio thread never logs (it counts, and the supervisor logs).

pub mod clock;
pub mod crash;
#[cfg(feature = "logging")]
mod logging;
mod platform;
pub mod ring;
pub mod session;
pub mod shutdown;

pub use crash::{contained, fail, fatal, note};
pub use ring::Line;
pub use session::PreviousSession;
pub use shutdown::{register as on_crash_cleanup, Registration};

use std::path::PathBuf;
use std::time::Instant;

/// Folder (in the data folder) of the log files.
pub const LOG_DIR: &str = "logs";
/// Folder (in the data folder) of crash reports and minidumps.
pub const CRASH_DIR: &str = "crashes";

/// How a program wants its diagnostics.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Name of the executable, for file names and messages.
    pub program: &'static str,
    /// Where the `logs` and `crashes` folders live.
    pub data_dir: PathBuf,
    /// Show a dialog when the program has to stop and there is no console to read the message in.
    pub dialog: bool,
    /// Also print the logs on stderr.
    pub console_log: bool,
    /// Keep a session marker to notice a crash at the next start (for long-running programs).
    pub track_session: bool,
}

/// Lives as long as the program: dropping it marks a clean exit and flushes the logs.
#[derive(Debug)]
pub struct Guard {
    /// What the previous run of this program left behind.
    pub previous: PreviousSession,
    session: bool,
}

/// Sets diagnostics up. Never fails: whatever cannot be set up (no log file...) is reported on stderr and skipped.
pub fn init(settings: Settings) -> Guard {
    #[cfg(feature = "logging")]
    if let Err(e) = logging::init(
        settings.program,
        &settings.data_dir.join(LOG_DIR),
        settings.console_log,
    ) {
        eprintln!("warning: logging could not start: {e}");
    }
    let previous = if settings.track_session {
        session::begin(&settings.data_dir, settings.program)
    } else {
        PreviousSession::default()
    };
    crash::install(crash::Context {
        program: settings.program,
        crash_dir: settings.data_dir.join(CRASH_DIR),
        dialog: settings.dialog,
        started: Instant::now(),
    });
    tracing::info!(
        program = settings.program,
        version = env!("CARGO_PKG_VERSION"),
        pid = std::process::id(),
        os = std::env::consts::OS,
        exe = ?std::env::current_exe().ok(),
        "starting"
    );
    if previous.unclean {
        tracing::warn!(
            report = ?previous.crash_report,
            "the previous run did not stop cleanly (crash, kill or power cut)"
        );
    }
    Guard {
        previous,
        session: settings.track_session,
    }
}

/// Writes out the logs still waiting in memory. After a crash or at exit; logging stops afterwards.
pub fn flush_logs() {
    #[cfg(feature = "logging")]
    logging::flush();
}

impl Drop for Guard {
    fn drop(&mut self) {
        tracing::info!("stopped cleanly");
        if self.session {
            session::end_current();
        }
        flush_logs();
    }
}
