//! Administrator rights, for the few things that need them (forgetting the devices Windows remembers).
//!
//! Nothing here asks for rights unless it must: a program that already has them (a developer who runs the daemon from an
//! elevated terminal) does the work itself, and everyone else gets the system's own confirmation window, once, for one
//! run of a helper (this same program started again with a flag). Other platforms plug their own way in here later
//! (`pkexec` on Linux): the rest of the product only talks to [`Elevator`].

use std::path::Path;
use std::time::Duration;

/// Why a run with administrator rights did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElevateError {
    /// The user said no to the system's confirmation window.
    Declined,
    /// The helper did not finish in time (it may still be running).
    TimedOut,
    /// This platform has no way to ask.
    Unsupported,
    /// Anything else, in words.
    Failed(String),
}

impl std::fmt::Display for ElevateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ElevateError::Declined => {
                f.write_str("the request for administrator rights was declined")
            }
            ElevateError::TimedOut => {
                f.write_str("the helper with administrator rights did not finish in time")
            }
            ElevateError::Unsupported => {
                f.write_str("this system has no way to ask for administrator rights")
            }
            ElevateError::Failed(e) => write!(f, "cannot get administrator rights: {e}"),
        }
    }
}

/// Whether this process has administrator rights, and a way to run another one that has them.
pub trait Elevator: Send + Sync {
    fn is_elevated(&self) -> bool;
    /// Runs `exe args...` with administrator rights (the system asks the user), waits and returns its exit code.
    fn run(&self, exe: &Path, args: &[String], timeout: Duration) -> Result<i32, ElevateError>;
}

/// The real system.
pub struct SystemElevator;

impl Elevator for SystemElevator {
    fn is_elevated(&self) -> bool {
        imp::is_elevated()
    }

    fn run(&self, exe: &Path, args: &[String], timeout: Duration) -> Result<i32, ElevateError> {
        imp::run_elevated(exe, args, timeout)
    }
}

/// One argument of a command line, quoted the way Windows programs read it back.
pub fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_owned();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                // the backslashes before a quote are doubled, and the quote itself is escaped
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            c => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                backslashes = 0;
                out.push(c);
            }
        }
    }
    // the backslashes before the closing quote are doubled too
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

#[cfg(windows)]
mod imp {
    use super::ElevateError;
    use std::path::Path;
    use std::time::Duration;
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetExitCodeProcess, OpenProcessToken, WaitForSingleObject,
    };
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

    pub fn is_elevated() -> bool {
        // SAFETY: plain Win32 calls on the current process; the token handle is closed before returning.
        unsafe {
            let mut token = HANDLE::default();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
                return false;
            }
            let mut elevation = TOKEN_ELEVATION::default();
            let mut returned = 0u32;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                Some(&mut elevation as *mut _ as *mut _),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut returned,
            )
            .is_ok();
            let _ = CloseHandle(token);
            ok && elevation.TokenIsElevated != 0
        }
    }

    pub fn run_elevated(
        exe: &Path,
        args: &[String],
        timeout: Duration,
    ) -> Result<i32, ElevateError> {
        let file = HSTRING::from(exe.as_os_str());
        let parameters = HSTRING::from(
            args.iter()
                .map(|a| super::quote_arg(a))
                .collect::<Vec<_>>()
                .join(" "),
        );
        let verb = HSTRING::from("runas");
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            lpVerb: PCWSTR(verb.as_ptr()),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR(parameters.as_ptr()),
            nShow: SW_HIDE.0,
            ..Default::default()
        };
        // SAFETY: `info` and the strings it points to live until the call returns; the process handle it hands back
        // is closed below.
        unsafe {
            if let Err(e) = ShellExecuteExW(&mut info) {
                // ERROR_CANCELLED: the user closed or refused the confirmation window
                return Err(if e.code().0 as u32 == 0x8007_04C7 {
                    ElevateError::Declined
                } else {
                    ElevateError::Failed(e.message())
                });
            }
            let process = info.hProcess;
            if process.is_invalid() {
                return Err(ElevateError::Failed("the system started no process".into()));
            }
            let wait = WaitForSingleObject(
                process,
                timeout.as_millis().min(u32::MAX as u128 - 1) as u32,
            );
            let result = if wait == WAIT_OBJECT_0 {
                let mut code = 0u32;
                match GetExitCodeProcess(process, &mut code) {
                    Ok(()) => Ok(code as i32),
                    Err(e) => Err(ElevateError::Failed(e.message())),
                }
            } else if wait == WAIT_TIMEOUT {
                Err(ElevateError::TimedOut)
            } else {
                Err(ElevateError::Failed("waiting for the helper failed".into()))
            };
            let _ = CloseHandle(process);
            result
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::ElevateError;
    use std::path::Path;
    use std::time::Duration;

    pub fn is_elevated() -> bool {
        false
    }

    pub fn run_elevated(
        _exe: &Path,
        _args: &[String],
        _timeout: Duration,
    ) -> Result<i32, ElevateError> {
        Err(ElevateError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_quoted_so_that_windows_reads_them_back() {
        assert_eq!(quote_arg("plain"), "plain");
        assert_eq!(quote_arg("with space"), "\"with space\"");
        assert_eq!(quote_arg(""), "\"\"");
        assert_eq!(quote_arg("a\"b"), "\"a\\\"b\"");
        // a path with spaces that ends in a backslash must not swallow the closing quote
        assert_eq!(quote_arg("C:\\My Folder\\"), "\"C:\\My Folder\\\\\"");
        // backslashes not followed by a quote are left alone
        assert_eq!(quote_arg("C:\\a b\\c"), "\"C:\\a b\\c\"");
        assert_eq!(quote_arg("x\\\"y z"), "\"x\\\\\\\"y z\"");
    }

    #[test]
    fn asking_whether_we_are_elevated_does_not_fail() {
        let _ = SystemElevator.is_elevated();
    }
}
