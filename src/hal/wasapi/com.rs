//! COM apartment handling and small conversions shared by the WASAPI modules.

use crate::hal::HalError;
use windows::core::Error;
use windows::Win32::Foundation::{RPC_E_CHANGED_MODE, S_FALSE, S_OK};
use windows::Win32::Media::Audio::AUDCLNT_E_DEVICE_INVALIDATED;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

/// Joins the multithreaded COM apartment for the lifetime of the guard (every thread that touches WASAPI keeps one).
pub struct ComGuard {
    uninit: bool,
}

impl ComGuard {
    pub fn new() -> ComGuard {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        // S_OK and S_FALSE both need a matching CoUninitialize; a thread already in another mode keeps its own.
        ComGuard {
            uninit: hr == S_OK || hr == S_FALSE,
        }
        .also_check(hr)
    }

    fn also_check(self, hr: windows::core::HRESULT) -> ComGuard {
        debug_assert!(hr.is_ok() || hr == RPC_E_CHANGED_MODE);
        self
    }
}

impl Default for ComGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.uninit {
            unsafe { CoUninitialize() };
        }
    }
}

/// True when the error says the device was removed or reconfigured under the stream.
pub fn is_invalidated(e: &Error) -> bool {
    e.code() == AUDCLNT_E_DEVICE_INVALIDATED
}

/// A readable message for a failed call.
pub fn describe(call: &str, e: &Error) -> String {
    if is_invalidated(e) {
        format!(
            "{call}: the device was removed or changed ({})",
            e.message().trim()
        )
    } else {
        format!(
            "{call}: {} (0x{:08x})",
            e.message().trim(),
            e.code().0 as u32
        )
    }
}

pub fn backend_error(call: &str, e: Error) -> HalError {
    HalError::Backend(describe(call, &e))
}
