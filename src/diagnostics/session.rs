//! Tells a clean stop from a crash, kill or power cut: a small file exists for as long as the program runs. Finding
//! it at the next start, with nobody alive behind it, means the last run ended unexpectedly.

use super::clock;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What the previous run left behind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreviousSession {
    /// The previous run did not stop cleanly.
    pub unclean: bool,
    /// The crash report written during that run, when there is one (a kill or power cut leaves none).
    pub crash_report: Option<PathBuf>,
}

/// The marker file of the current run (removed by [`end_current`]).
static CURRENT: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

fn marker_path(dir: &Path, program: &str) -> PathBuf {
    dir.join(format!("{program}.session"))
}

/// Looks at what the previous run left, then marks this run as started.
pub fn begin(data_dir: &Path, program: &str) -> PreviousSession {
    let path = marker_path(data_dir, program);
    let previous = inspect(&path, &data_dir.join(super::CRASH_DIR));
    let _ = std::fs::create_dir_all(data_dir);
    let _ = std::fs::write(
        &path,
        format!(
            "pid={}\nstarted={}\n",
            std::process::id(),
            clock::iso(SystemTime::now())
        ),
    );
    *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(path);
    previous
}

fn inspect(marker: &Path, crash_dir: &Path) -> PreviousSession {
    let Ok(text) = std::fs::read_to_string(marker) else {
        return PreviousSession::default();
    };
    let pid = text
        .lines()
        .find_map(|l| l.strip_prefix("pid="))
        .and_then(|p| p.trim().parse::<u32>().ok());
    // another copy still running: this is not a crash (the port or the lock will tell the user)
    if pid.is_some_and(|p| p != std::process::id() && pid_alive(p)) {
        return PreviousSession::default();
    }
    let since = std::fs::metadata(marker).and_then(|m| m.modified()).ok();
    PreviousSession {
        unclean: true,
        crash_report: newest_report_since(crash_dir, since),
    }
}

fn newest_report_since(dir: &Path, since: Option<SystemTime>) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "txt"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .filter(|(modified, _)| since.is_none_or(|s| *modified >= s))
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, p)| p)
}

/// Marks the current run as ended cleanly (also called when the console is closed or the user logs off).
pub fn end_current() {
    let path = CURRENT.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(path) = path {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(all(windows, feature = "logging"))]
pub(crate) fn pid_alive(pid: u32) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    const STILL_ACTIVE: u32 = 259;
    // SAFETY: plain Win32 calls; the handle is closed before returning.
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut code = 0u32;
        let alive = GetExitCodeProcess(h, &mut code).is_ok() && code == STILL_ACTIVE;
        let _ = CloseHandle(h);
        alive
    }
}

#[cfg(not(all(windows, feature = "logging")))]
pub(crate) fn pid_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("omu-session-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_clean_stop_leaves_nothing_and_a_crash_is_noticed() {
        let dir = temp("a");
        let first = begin(&dir, "t");
        assert!(!first.unclean, "nothing before the first run");
        end_current();
        let second = begin(&dir, "t");
        assert!(!second.unclean, "the first run stopped cleanly");
        // the run "crashes": the marker is left behind by a process that is gone (pid 0 never exists)
        std::fs::write(marker_path(&dir, "t"), "pid=0\nstarted=x\n").unwrap();
        let crash_dir = dir.join(super::super::CRASH_DIR);
        std::fs::create_dir_all(&crash_dir).unwrap();
        std::fs::write(crash_dir.join("crash-1.txt"), "report").unwrap();
        let third = begin(&dir, "t");
        end_current();
        assert!(third.unclean);
        assert_eq!(third.crash_report, Some(crash_dir.join("crash-1.txt")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
