//! Cleaning up the devices the system remembers although they are gone (see [`super::stale`]).
//!
//! Forgetting them needs administrator rights. This asks for them only when it must: a program that already has them
//! (the developer who runs the daemon from an elevated terminal) does the work itself, without any confirmation
//! window; any other starts this same program again with the rights, which asks the user once for the whole run.

use omu_desktop::elevation::{ElevateError, Elevator};
use omu_hal::{HalError, VirtualDeviceProvider};
use omu_types::{CleanOutcome, KeptDevices, StaleDevice, StaleDevices};
use std::path::PathBuf;
use std::time::Duration;

/// The exit code of the helper that was started without the rights it needed.
pub const EXIT_NEEDS_RIGHTS: i32 = 5;
/// The exit code of the helper when something else went wrong.
pub const EXIT_FAILED: i32 = 6;

/// How long the user has to answer the system's confirmation window, and the helper to work.
const HELPER_TIMEOUT: Duration = Duration::from_secs(180);

/// Why cleaning did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanError {
    /// Nothing to clean on this system.
    Unsupported(String),
    /// The user refused the confirmation window.
    Declined,
    Failed(String),
}

impl std::fmt::Display for CleanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CleanError::Unsupported(m) | CleanError::Failed(m) => f.write_str(m),
            CleanError::Declined => f.write_str(
                "cleaning was cancelled: administrator rights were refused (nothing was removed)",
            ),
        }
    }
}

/// How to start this program again with administrator rights, with the same data folder.
#[derive(Debug, Clone)]
pub struct Relaunch {
    pub exe: PathBuf,
    pub args: Vec<String>,
}

impl Relaunch {
    /// This program started again as the cleaning helper (`--clean-devices --elevated-helper`).
    pub fn this_program(data_dir: &std::path::Path) -> Result<Relaunch, CleanError> {
        let exe = std::env::current_exe()
            .map_err(|e| CleanError::Failed(format!("cannot find this program: {e}")))?;
        Ok(Relaunch {
            exe,
            args: vec![
                "--clean-devices".into(),
                "--elevated-helper".into(),
                "--data-dir".into(),
                data_dir.display().to_string(),
            ],
        })
    }
}

/// What could be cleaned now (read only, no rights needed).
pub fn survey(
    provider: Option<&dyn VirtualDeviceProvider>,
    kept: &KeptDevices,
    elevator: &dyn Elevator,
) -> StaleDevices {
    let found = provider.map(|p| p.stale_devices(kept));
    match found {
        Some(Ok(stale)) => StaleDevices {
            supported: true,
            elevated: elevator.is_elevated(),
            stale,
        },
        _ => StaleDevices::default(),
    }
}

fn stale_of(
    provider: &dyn VirtualDeviceProvider,
    kept: &KeptDevices,
) -> Result<Vec<StaleDevice>, CleanError> {
    provider.stale_devices(kept).map_err(|e| match e {
        HalError::Unsupported(m) => CleanError::Unsupported(m),
        other => CleanError::Failed(other.to_string()),
    })
}

/// Forgets the old devices: in this process when it has the rights, else through a helper that asks for them.
pub fn clean(
    provider: &dyn VirtualDeviceProvider,
    kept: &KeptDevices,
    elevator: &dyn Elevator,
    relaunch: &Relaunch,
) -> Result<CleanOutcome, CleanError> {
    let before = stale_of(provider, kept)?.len();
    if before == 0 {
        return Ok(CleanOutcome::default());
    }
    let mut asked_for_rights = false;
    if elevator.is_elevated() {
        provider.forget_stale_devices(kept).map_err(|e| match e {
            HalError::Unsupported(m) => CleanError::Unsupported(m),
            other => CleanError::Failed(other.to_string()),
        })?;
    } else {
        asked_for_rights = true;
        match elevator.run(&relaunch.exe, &relaunch.args, HELPER_TIMEOUT) {
            Ok(0) => {}
            Ok(EXIT_NEEDS_RIGHTS) => {
                return Err(CleanError::Failed(
                    "the helper did not get administrator rights".into(),
                ))
            }
            Ok(code) => {
                return Err(CleanError::Failed(format!(
                    "the cleaning helper failed (exit code {code}); see the log"
                )))
            }
            Err(ElevateError::Declined) => return Err(CleanError::Declined),
            Err(e) => return Err(CleanError::Failed(e.to_string())),
        }
    }
    let remaining = stale_of(provider, kept)?.len();
    Ok(CleanOutcome {
        removed: before.saturating_sub(remaining),
        remaining,
        asked_for_rights,
    })
}

/// What the helper (and `omud --clean-devices`) does when it runs: returns the exit code.
pub fn run_helper(
    provider: &dyn VirtualDeviceProvider,
    kept: &KeptDevices,
    elevator: &dyn Elevator,
    relaunch: &Relaunch,
    is_helper: bool,
    out: &mut dyn std::io::Write,
) -> i32 {
    let stale = match stale_of(provider, kept) {
        Ok(s) => s,
        Err(e) => {
            let _ = writeln!(out, "{e}");
            return EXIT_FAILED;
        }
    };
    if stale.is_empty() {
        let _ = writeln!(out, "no old device to clean up");
        return 0;
    }
    for s in &stale {
        let _ = writeln!(out, "old device: {}", s.name);
    }
    // the helper must never ask again (it would start itself for ever)
    if is_helper && !elevator.is_elevated() {
        return EXIT_NEEDS_RIGHTS;
    }
    match clean(provider, kept, elevator, relaunch) {
        Ok(outcome) => {
            let _ = writeln!(
                out,
                "{} old device(s) forgotten, {} left{}",
                outcome.removed,
                outcome.remaining,
                if outcome.asked_for_rights {
                    " (asked for administrator rights)"
                } else {
                    ""
                }
            );
            if outcome.remaining == 0 {
                0
            } else {
                EXIT_FAILED
            }
        }
        Err(CleanError::Declined) => {
            let _ = writeln!(out, "{}", CleanError::Declined);
            EXIT_NEEDS_RIGHTS
        }
        Err(e) => {
            let _ = writeln!(out, "{e}");
            EXIT_FAILED
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omu_desktop::elevation::Elevator;
    use std::path::Path;
    use std::sync::Mutex;

    /// A provider with a list of old devices that disappears when it is told to forget them.
    struct Fake {
        stale: Mutex<Vec<StaleDevice>>,
        forgotten: Mutex<u32>,
        refuse: bool,
    }

    impl Fake {
        fn with(names: &[&str]) -> Fake {
            Fake {
                stale: Mutex::new(
                    names
                        .iter()
                        .map(|n| StaleDevice { name: (*n).into() })
                        .collect(),
                ),
                forgotten: Mutex::new(0),
                refuse: false,
            }
        }
    }

    impl VirtualDeviceProvider for Fake {
        fn name(&self) -> &str {
            "fake"
        }
        fn available(&self) -> Result<(), HalError> {
            Ok(())
        }
        fn create_microphone(
            &self,
            _name: &str,
        ) -> Result<Box<dyn omu_hal::RenderStream>, HalError> {
            Err(HalError::Unsupported("test".into()))
        }
        fn stale_devices(&self, _kept: &KeptDevices) -> Result<Vec<StaleDevice>, HalError> {
            Ok(self.stale.lock().unwrap().clone())
        }
        fn forget_stale_devices(&self, _kept: &KeptDevices) -> Result<(), HalError> {
            if self.refuse {
                return Err(HalError::NeedsAdministrator("test".into()));
            }
            *self.forgotten.lock().unwrap() += 1;
            self.stale.lock().unwrap().clear();
            Ok(())
        }
    }

    /// An elevator that is, or is not, elevated, and records how it was asked; its helper "cleans" through `fake`.
    struct Rights<'a> {
        elevated: bool,
        answer: Mutex<Option<Result<i32, ElevateError>>>,
        runs: Mutex<Vec<Vec<String>>>,
        fake: &'a Fake,
    }

    impl Elevator for Rights<'_> {
        fn is_elevated(&self) -> bool {
            self.elevated
        }
        fn run(&self, _exe: &Path, args: &[String], _t: Duration) -> Result<i32, ElevateError> {
            self.runs.lock().unwrap().push(args.to_vec());
            let answer = self.answer.lock().unwrap().take().unwrap_or(Ok(0));
            if matches!(answer, Ok(0)) {
                self.fake.stale.lock().unwrap().clear(); // what the elevated helper does
            }
            answer
        }
    }

    fn relaunch() -> Relaunch {
        Relaunch {
            exe: "omud".into(),
            args: vec!["--clean-devices".into(), "--elevated-helper".into()],
        }
    }

    fn rights<'a>(
        fake: &'a Fake,
        elevated: bool,
        answer: Option<Result<i32, ElevateError>>,
    ) -> Rights<'a> {
        Rights {
            elevated,
            answer: Mutex::new(answer),
            runs: Mutex::new(vec![]),
            fake,
        }
    }

    #[test]
    fn a_program_that_already_has_the_rights_cleans_without_asking() {
        let fake = Fake::with(&["Old", "Older"]);
        let r = rights(&fake, true, None);
        let out = clean(&fake, &KeptDevices::default(), &r, &relaunch()).unwrap();
        assert_eq!(
            out,
            CleanOutcome {
                removed: 2,
                remaining: 0,
                asked_for_rights: false
            }
        );
        assert!(
            r.runs.lock().unwrap().is_empty(),
            "no confirmation window, no helper"
        );
        assert_eq!(*fake.forgotten.lock().unwrap(), 1);
    }

    #[test]
    fn a_program_without_the_rights_starts_the_helper_once_and_reports_what_is_left() {
        let fake = Fake::with(&["Old"]);
        let r = rights(&fake, false, None);
        let out = clean(&fake, &KeptDevices::default(), &r, &relaunch()).unwrap();
        assert_eq!(
            out,
            CleanOutcome {
                removed: 1,
                remaining: 0,
                asked_for_rights: true
            }
        );
        assert_eq!(r.runs.lock().unwrap().len(), 1);
        assert_eq!(
            *fake.forgotten.lock().unwrap(),
            0,
            "this process did not do it itself"
        );
    }

    #[test]
    fn nothing_to_clean_asks_for_nothing() {
        let fake = Fake::with(&[]);
        let r = rights(&fake, false, None);
        let out = clean(&fake, &KeptDevices::default(), &r, &relaunch()).unwrap();
        assert_eq!(out, CleanOutcome::default());
        assert!(r.runs.lock().unwrap().is_empty(), "no window for nothing");
    }

    #[test]
    fn a_refused_confirmation_is_a_plain_cancel_and_nothing_is_removed() {
        let fake = Fake::with(&["Old"]);
        let r = rights(&fake, false, Some(Err(ElevateError::Declined)));
        let err = clean(&fake, &KeptDevices::default(), &r, &relaunch()).unwrap_err();
        assert_eq!(err, CleanError::Declined);
        assert_eq!(fake.stale.lock().unwrap().len(), 1);
        // a helper that failed is reported as such
        let fake = Fake::with(&["Old"]);
        let r = rights(&fake, false, Some(Ok(EXIT_FAILED)));
        assert!(matches!(
            clean(&fake, &KeptDevices::default(), &r, &relaunch()),
            Err(CleanError::Failed(_))
        ));
    }

    #[test]
    fn the_survey_says_whether_the_rights_are_there_and_what_would_go() {
        let fake = Fake::with(&["Old"]);
        let s = survey(
            Some(&fake),
            &KeptDevices::default(),
            &rights(&fake, true, None),
        );
        assert!(s.supported && s.elevated && s.stale.len() == 1);
        let s = survey(
            Some(&fake),
            &KeptDevices::default(),
            &rights(&fake, false, None),
        );
        assert!(!s.elevated);
        // no provider, or one that has nothing to clean: not supported, and the panel shows nothing
        assert_eq!(
            survey(None, &KeptDevices::default(), &rights(&fake, true, None)),
            StaleDevices::default()
        );
    }

    #[test]
    fn the_helper_never_asks_for_rights_itself() {
        let fake = Fake::with(&["Old"]);
        let r = rights(&fake, false, None);
        let mut out = vec![];
        let code = run_helper(
            &fake,
            &KeptDevices::default(),
            &r,
            &relaunch(),
            true,
            &mut out,
        );
        assert_eq!(code, EXIT_NEEDS_RIGHTS);
        assert!(
            r.runs.lock().unwrap().is_empty(),
            "it would start itself for ever"
        );
        // started by hand with the rights: it works and says what it did
        let r = rights(&fake, true, None);
        let mut out = vec![];
        let code = run_helper(
            &fake,
            &KeptDevices::default(),
            &r,
            &relaunch(),
            true,
            &mut out,
        );
        assert_eq!(code, 0);
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("old device: Old") && text.contains("1 old device(s) forgotten"),
            "{text}"
        );
    }
}
