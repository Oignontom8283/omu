//! Hardware abstraction layer.
//!
//! A [`Backend`] knows how to list and open the audio devices of a platform (WASAPI, PipeWire, CoreAudio, ...) and,
//! when the platform allows it, to capture the sound of one application. A [`VirtualDeviceProvider`] creates the
//! virtual microphones other applications record from (an optional capability: on some platforms it needs a
//! third-party component). Backends declare what they can do in [`Capabilities`], so a missing feature is greyed
//! out instead of failing.
//!
//! Streams follow one pattern: *open* (device resolved, format known, nothing running), then *start* with a
//! callback that runs on the backend's real-time thread, then drop the returned guard to stop. Callbacks receive
//! and produce interleaved `f32` frames in the stream's own format; the engine converts to its own format.
//!
//! Golden rule: nothing here may take the process (or the system) down. A failing stream flags its
//! [`StreamHealth`] and goes silent; the engine decides what to do next.

pub mod own_devices;
pub mod sim;
#[cfg(windows)]
pub mod wasapi;

use omu_types::{AppTarget, AudioFormat, DeviceRef, KeyCombo};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use thiserror::Error;

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum HalError {
    #[error("no matching device or application: {0}")]
    NotFound(String),
    #[error("not supported on this system: {0}")]
    Unsupported(String),
    #[error("the device is in use or refused the request: {0}")]
    Busy(String),
    #[error("administrator rights are needed: {0}")]
    NeedsAdministrator(String),
    #[error("{0}")]
    Backend(String),
}

pub use omu_types::{AppInfo, Capabilities, DeviceInfo, Direction};
pub use own_devices::{is_own_device, OwnDevicesAside};

/// Live status of a stream, shared between its real-time thread and the supervisor.
#[derive(Debug, Default)]
pub struct StreamHealth {
    failed: AtomicBool,
    /// Buffers that could not be delivered or produced in time.
    xruns: AtomicU64,
    /// Frames moved so far.
    frames: AtomicU64,
    error: Mutex<Option<String>>,
}

impl StreamHealth {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    /// Marks the stream as failed (device removed, driver error...). Callable from any thread.
    pub fn fail(&self, message: impl Into<String>) {
        if let Ok(mut e) = self.error.lock() {
            e.get_or_insert_with(|| message.into());
        }
        self.failed.store(true, Ordering::Release);
    }
    pub fn has_failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }
    pub fn error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|e| e.clone())
    }
    pub fn add_xrun(&self) {
        self.xruns.fetch_add(1, Ordering::Relaxed);
    }
    pub fn xruns(&self) -> u64 {
        self.xruns.load(Ordering::Relaxed)
    }
    pub fn add_frames(&self, n: u64) {
        self.frames.fetch_add(n, Ordering::Relaxed);
    }
    pub fn frames(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }
}

/// Receives captured frames (interleaved, in the stream's format). Runs on a real-time thread.
pub type CaptureCallback = Box<dyn FnMut(&[f32]) + Send>;
/// Fills the frames to play (interleaved, in the stream's format). Runs on a real-time thread.
pub type RenderCallback = Box<dyn FnMut(&mut [f32]) + Send>;
/// Called each time a global keyboard shortcut is pressed.
pub type HotkeyCallback = Box<dyn Fn() + Send + Sync>;

/// A running stream. Dropping it stops the stream and joins its thread.
pub trait Running: Send {}

/// A capture stream that is open but not yet running.
pub trait CaptureStream: Send {
    fn format(&self) -> AudioFormat;
    fn health(&self) -> Arc<StreamHealth>;
    fn start(self: Box<Self>, callback: CaptureCallback) -> Result<Box<dyn Running>, HalError>;
}

/// A playback stream that is open but not yet running.
pub trait RenderStream: Send {
    fn format(&self) -> AudioFormat;
    fn health(&self) -> Arc<StreamHealth>;
    fn start(self: Box<Self>, callback: RenderCallback) -> Result<Box<dyn Running>, HalError>;
}

/// The audio services of one platform.
pub trait Backend: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> Capabilities;
    fn list_devices(&self) -> Result<Vec<DeviceInfo>, HalError>;
    fn open_capture(&self, device: &DeviceRef) -> Result<Box<dyn CaptureStream>, HalError>;
    fn open_render(&self, device: &DeviceRef) -> Result<Box<dyn RenderStream>, HalError>;
    /// Applications currently known to the audio system (with or without sound right now).
    fn list_apps(&self) -> Result<Vec<AppInfo>, HalError> {
        Err(HalError::Unsupported(
            "capturing an application's sound".into(),
        ))
    }
    /// Registers a global keyboard shortcut: `on_press` is called (from the backend's own thread) each time it is
    /// pressed, whichever program has the focus. The shortcut is released when the returned guard is dropped. Fails
    /// when another program already owns the shortcut.
    fn open_hotkey(
        &self,
        _combo: &KeyCombo,
        _on_press: HotkeyCallback,
    ) -> Result<Box<dyn Running>, HalError> {
        Err(HalError::Unsupported("global keyboard shortcuts".into()))
    }
    /// Like [`Backend::open_hotkey`], and `on_release` is called when the key is let go (a sound that plays while the
    /// key is held).
    fn open_hotkey_hold(
        &self,
        _combo: &KeyCombo,
        _on_press: HotkeyCallback,
        _on_release: HotkeyCallback,
    ) -> Result<Box<dyn Running>, HalError> {
        Err(HalError::Unsupported(
            "global keyboard shortcuts that notice the key being let go".into(),
        ))
    }
    /// Lets go of every global shortcut for a while (`true`), or takes them back (`false`): a registered shortcut is kept from the program
    /// that has the focus, so the field that records a new shortcut could never hear the keys that are already in use.
    fn pause_hotkeys(&self, _paused: bool) {}
    /// Captures the sound of an application (and its child processes) before it reaches any device.
    fn open_app_capture(&self, _target: &AppTarget) -> Result<Box<dyn CaptureStream>, HalError> {
        Err(HalError::Unsupported(
            "capturing an application's sound".into(),
        ))
    }
}

/// Creates virtual microphones: devices other applications can record from, fed by the audio we play into them.
pub trait VirtualDeviceProvider: Send + Sync {
    fn name(&self) -> &str;
    /// Whether the provider can work right now (component installed, permissions...).
    fn available(&self) -> Result<(), HalError>;
    /// Creates (or reuses) a virtual microphone called `name`. The returned stream is what *we* play into it.
    fn create_microphone(&self, name: &str) -> Result<Box<dyn RenderStream>, HalError>;
    /// Creates (or reuses) a virtual speaker called `name`: an output device other programs can play into. The
    /// returned stream is what *we* record from it. Providers that cannot make speakers say so (the default).
    fn create_speaker(&self, _name: &str) -> Result<Box<dyn CaptureStream>, HalError> {
        Err(HalError::Unsupported(
            "this provider cannot create virtual speakers".into(),
        ))
    }
    /// The names of the speakers this provider has created so far: the daemon refuses to play into them from the
    /// graph that records them (feedback).
    fn speaker_names(&self) -> Vec<String> {
        vec![]
    }
    /// The names of the microphones this provider has created so far: the daemon refuses to record from them (a
    /// program must never hear its own output through its own virtual microphone).
    fn microphone_names(&self) -> Vec<String> {
        vec![]
    }
    /// Makes the devices that exist now permanent, when the system can bring them back by itself (at every boot):
    /// asked after every change of the graph, it does something only when the set of devices changed. It is not asked
    /// when the program stops, so that the devices of the last graph stay permanent.
    fn persist(&self) {}
    /// The devices the system still remembers although they are gone and `kept` (the names the graph uses) does not
    /// have them: read only, no rights needed. Systems that remember nothing say so (the default).
    fn stale_devices(
        &self,
        _kept: &omu_types::KeptDevices,
    ) -> Result<Vec<omu_types::StaleDevice>, HalError> {
        Err(HalError::Unsupported(
            "this provider has no old devices to clean up".into(),
        ))
    }
    /// Makes the system forget those devices, in this process: it fails with [`HalError::NeedsAdministrator`] when the
    /// system wants rights this process does not have.
    fn forget_stale_devices(&self, _kept: &omu_types::KeptDevices) -> Result<(), HalError> {
        Err(HalError::Unsupported(
            "this provider has no old devices to clean up".into(),
        ))
    }
}
