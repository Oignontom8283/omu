//! One daemon at a time.
//!
//! Two daemons on one machine would fight over the same audio devices, create the same virtual devices twice and
//! overwrite each other's discovery file. A daemon that uses the real audio system therefore takes a lock that is
//! machine-wide; a daemon that only simulates audio (`--demo`) takes one per data folder, so that trying the panel
//! never gets in the way of the real one.
//!
//! The lock is a named mutex of the operating system: it is released by the system whatever happens to the process
//! (crash, kill), so it can never be left behind. The discovery file is looked at too, which also catches a daemon
//! of an older version that does not take the lock.

use crate::discovery::Discovery;
use std::path::Path;

/// How far the lock reaches.
#[derive(Debug, Clone, Copy)]
pub enum Scope<'a> {
    /// Every daemon of this computer (the real audio system).
    Machine,
    /// Daemons that share this data folder (the simulated audio system).
    Folder(&'a Path),
}

/// Held for as long as this daemon runs.
#[derive(Debug)]
pub struct Instance {
    #[cfg(windows)]
    handle: isize,
}

#[cfg(windows)]
impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by CreateMutexW and is closed once.
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(windows::Win32::Foundation::HANDLE(
                self.handle as *mut _,
            ));
        }
    }
}

/// A stable short text of a folder (FNV-1a of its lower-cased path).
fn folder_key(dir: &Path) -> String {
    let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let h = dir
        .to_string_lossy()
        .to_lowercase()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
        });
    format!("{h:016x}")
}

fn lock_name(scope: Scope<'_>) -> String {
    let base = crate::branding::SINGLE_INSTANCE_NAME;
    match scope {
        Scope::Machine => format!("Global\\{base}"),
        Scope::Folder(dir) => format!("Global\\{base}-{}", folder_key(dir)),
    }
}

fn already_running(pid: Option<u32>, address: Option<&str>) -> String {
    let who = match (pid, address) {
        (Some(p), Some(a)) => format!(" (process {p}, http://{a})"),
        (Some(p), None) => format!(" (process {p})"),
        _ => String::new(),
    };
    format!(
        "{} is already running on this computer{who}. Two audio managers at the same time would fight over the same \
         devices, so this one stops. Use the one that runs, or stop it first.",
        crate::branding::DISPLAY_NAME
    )
}

/// Takes the lock of `scope`, or says (in words for the user) why the daemon must not start. `data_dir` is where
/// the discovery file of a running daemon would be.
pub fn acquire(scope: Scope<'_>, data_dir: &Path) -> Result<Instance, String> {
    // A daemon that is running announces itself there (older versions do not take the lock but do write it).
    if let Some(d) = Discovery::read(data_dir) {
        if d.pid != std::process::id() && crate::diagnostics::session::pid_alive(d.pid) {
            return Err(already_running(Some(d.pid), Some(&d.address)));
        }
    }
    take(&lock_name(scope), data_dir)
}

#[cfg(windows)]
fn take(name: &str, data_dir: &Path) -> Result<Instance, String> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS,
    };
    use windows::Win32::System::Threading::CreateMutexW;
    // SAFETY: plain Win32 calls; the handle is closed here or kept in the returned value.
    unsafe {
        match CreateMutexW(None, false, &HSTRING::from(name)) {
            Ok(handle) => {
                if GetLastError() == ERROR_ALREADY_EXISTS {
                    let _ = CloseHandle(handle);
                    let owner = Discovery::read(data_dir);
                    return Err(already_running(
                        owner.as_ref().map(|d| d.pid),
                        owner.as_ref().map(|d| d.address.as_str()),
                    ));
                }
                Ok(Instance {
                    handle: handle.0 as isize,
                })
            }
            // the mutex exists and belongs to another user or an elevated process: someone else runs a daemon
            Err(e) if e.code() == ERROR_ACCESS_DENIED.to_hresult() => {
                Err(already_running(None, None))
            }
            // anything else is not a reason to keep the user from working: go on without the lock
            Err(e) => {
                tracing::warn!("cannot take the single-daemon lock ({e}); going on without it");
                Ok(Instance { handle: 0 })
            }
        }
    }
}

#[cfg(not(windows))]
fn take(_name: &str, _data_dir: &Path) -> Result<Instance, String> {
    // only the discovery file (checked by `acquire`) guards other systems for now
    Ok(Instance {})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("omu-instance-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn names_differ_by_scope_and_by_folder() {
        let (a, b) = (temp("a"), temp("b"));
        assert_ne!(lock_name(Scope::Machine), lock_name(Scope::Folder(&a)));
        assert_ne!(lock_name(Scope::Folder(&a)), lock_name(Scope::Folder(&b)));
        assert_eq!(lock_name(Scope::Folder(&a)), lock_name(Scope::Folder(&a)));
        assert!(lock_name(Scope::Machine).contains(crate::branding::SINGLE_INSTANCE_NAME));
    }

    #[cfg(windows)]
    #[test]
    fn a_second_daemon_in_the_same_folder_is_refused_until_the_first_is_gone() {
        let dir = temp("lock");
        let first = acquire(Scope::Folder(&dir), &dir).expect("the first one runs");
        let refused = acquire(Scope::Folder(&dir), &dir).unwrap_err();
        assert!(refused.contains("already running"), "{refused}");
        // another folder is another world (a demo next to a real daemon)
        let other = temp("lock-other");
        let _elsewhere = acquire(Scope::Folder(&other), &other).expect("another folder is free");
        drop(first);
        let _again = acquire(Scope::Folder(&dir), &dir).expect("free again once the first stopped");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&other);
    }

    #[test]
    fn a_daemon_announced_in_the_discovery_file_and_still_alive_is_refused() {
        let dir = temp("discovery");
        // a live process that is not us: the parent of this test (cargo or the shell)
        let parent = std::process::Command::new("cmd")
            .args(["/c", "ping -n 20 127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn();
        let Ok(mut child) = parent else { return }; // not Windows: nothing to start
        Discovery {
            address: "127.0.0.1:7650".into(),
            token: "SECRET-TOKEN-VALUE".into(),
            pid: child.id(),
        }
        .write(&dir)
        .unwrap();
        let refused = acquire(Scope::Folder(&dir), &dir).unwrap_err();
        assert!(
            refused.contains(&format!("process {}", child.id()))
                && refused.contains("127.0.0.1:7650"),
            "{refused}"
        );
        assert!(
            !refused.contains("SECRET-TOKEN-VALUE"),
            "no secret in the message"
        );
        // once that process is gone, the stale file does not block anybody
        let _ = child.kill();
        let _ = child.wait();
        assert!(acquire(Scope::Folder(&dir), &dir).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
