//! The operating-system side of crash handling (Windows): the unhandled-exception filter with a minidump, the
//! console and logoff events, and the error dialog. On other
//! systems these do nothing and the panic hook alone applies.

#[cfg(windows)]
pub use imp::{install_handlers, show_error};

#[cfg(not(windows))]
pub fn install_handlers() {}

#[cfg(not(windows))]
pub fn show_error(_title: &str, _text: &str) {}

#[cfg(windows)]
mod imp {
    use crate::crash::{self, Report};
    use crate::{clock, session, shutdown};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, SystemTime};
    use windows::core::BOOL;
    use windows::core::HSTRING;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Console::{GetConsoleWindow, SetConsoleCtrlHandler};
    use windows::Win32::System::Diagnostics::Debug::{
        MiniDumpNormal, MiniDumpWithThreadInfo, MiniDumpWithUnloadedModules, MiniDumpWriteDump,
        SetUnhandledExceptionFilter, EXCEPTION_POINTERS, MINIDUMP_EXCEPTION_INFORMATION,
        MINIDUMP_TYPE,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
    };

    pub fn install_handlers() {
        // SAFETY: both handlers are plain functions that live for the whole program.
        unsafe {
            SetUnhandledExceptionFilter(Some(exception_filter));
            let _ = SetConsoleCtrlHandler(Some(console_event), true);
        }
    }

    /// Names the exceptions a program is likely to die of.
    fn describe(code: u32, info: &[usize]) -> String {
        let name = match code {
            0xC000_0005 => {
                let what = match info.first() {
                    Some(0) => "reading",
                    Some(1) => "writing",
                    Some(8) => "executing",
                    _ => "accessing",
                };
                let at = info.get(1).copied().unwrap_or(0);
                return format!("access violation {what} address {at:#x} (0xC0000005)");
            }
            0xC000_00FD => "stack overflow",
            0xC000_0374 => "heap corruption",
            0xC000_0094 => "integer division by zero",
            0xC000_001D => "illegal instruction",
            0xC000_0096 => "privileged instruction",
            0xC000_0409 => "stack buffer overrun (fail-fast)",
            0x8000_0003 => "breakpoint",
            _ => "unhandled native exception",
        };
        format!("{name} ({code:#010X})")
    }

    unsafe extern "system" fn exception_filter(pointers: *const EXCEPTION_POINTERS) -> i32 {
        static BUSY: AtomicBool = AtomicBool::new(false);
        if BUSY.swap(true, Ordering::SeqCst) || pointers.is_null() {
            return 1; // EXCEPTION_EXECUTE_HANDLER: a crash while crashing ends the process at once
        }
        // SAFETY: Windows passes valid pointers to the exception being dispatched.
        let (code, address, info) = unsafe {
            let rec = &*(*pointers).ExceptionRecord;
            let n = (rec.NumberParameters as usize).min(rec.ExceptionInformation.len());
            (
                rec.ExceptionCode.0 as u32,
                rec.ExceptionAddress as usize,
                rec.ExceptionInformation[..n].to_vec(),
            )
        };
        let summary = describe(code, &info);
        let thread = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_owned();
        tracing::error!(target: "omu::crash", thread = %thread, address = format_args!("{address:#x}"), "native crash: {summary}");
        let backtrace =
            crash::trim_backtrace(&std::backtrace::Backtrace::force_capture().to_string());
        let dump = write_minidump(pointers);
        let details = format!(
            "{backtrace}\n\nminidump: {}",
            dump.as_deref()
                .map_or("not written".to_owned(), |p| p.display().to_string())
        );
        let report = crash::write_report(&Report {
            kind: "native exception (fatal)",
            summary: &summary,
            place: &format!("instruction at {address:#x}"),
            thread: &thread,
            details: &details,
        });
        crash::fatal(&summary, report.as_deref(), code as i32)
    }

    fn write_minidump(pointers: *const EXCEPTION_POINTERS) -> Option<std::path::PathBuf> {
        use std::os::windows::io::AsRawHandle;
        let context = crash::context()?;
        let path = context.crash_dir.join(format!(
            "crash-{}-{}.dmp",
            clock::stamp(SystemTime::now()),
            std::process::id()
        ));
        std::fs::create_dir_all(&context.crash_dir).ok()?;
        let file = std::fs::File::create(&path).ok()?;
        let exception = MINIDUMP_EXCEPTION_INFORMATION {
            // SAFETY: plain query of the calling thread's id.
            ThreadId: unsafe { GetCurrentThreadId() },
            ExceptionPointers: pointers as *mut _,
            ClientPointers: false.into(),
        };
        let kind = MINIDUMP_TYPE(
            MiniDumpNormal.0 | MiniDumpWithThreadInfo.0 | MiniDumpWithUnloadedModules.0,
        );
        // SAFETY: the file handle stays open for the call; `exception` outlives it.
        let written = unsafe {
            MiniDumpWriteDump(
                GetCurrentProcess(),
                GetCurrentProcessId(),
                HANDLE(file.as_raw_handle()),
                kind,
                Some(&exception),
                None,
                None,
            )
        };
        written.ok().map(|()| path)
    }

    /// The console window was closed, the user logged off or Windows is shutting down: there are a few seconds to
    /// leave things in order. Ctrl-C and Ctrl-Break are left to the program (the daemon stops on them cleanly).
    unsafe extern "system" fn console_event(kind: u32) -> BOOL {
        const CLOSE: u32 = 2;
        const LOGOFF: u32 = 5;
        const SHUTDOWN: u32 = 6;
        if matches!(kind, CLOSE | LOGOFF | SHUTDOWN) {
            tracing::warn!("system event {kind} (close, logoff or shutdown): cleaning up");
            let n = shutdown::run_all();
            tracing::info!("{n} cleanup(s) run");
            session::end_current();
            crate::flush_logs();
        }
        false.into() // let the next handler (the default one) end the process
    }

    /// Shows `text` in a message box, but only when there is no console to read it in (the window or a detached
    /// daemon); waits for the user at most a minute so that an unattended machine does not hang on a dialog.
    pub fn show_error(title: &str, text: &str) {
        // SAFETY: plain query.
        if !unsafe { GetConsoleWindow() }.0.is_null() {
            return;
        }
        let (title, text) = (HSTRING::from(title), HSTRING::from(text));
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("error-dialog".into())
            .spawn(move || {
                // SAFETY: the strings live for the call.
                unsafe {
                    MessageBoxW(
                        None,
                        &text,
                        &title,
                        MB_OK | MB_ICONERROR | MB_SETFOREGROUND | MB_TOPMOST,
                    );
                }
                let _ = tx.send(());
            });
        if spawned.is_ok() {
            let _ = rx.recv_timeout(Duration::from_secs(60));
        }
    }
}
