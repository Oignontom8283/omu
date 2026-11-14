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

pub mod sim;
#[cfg(all(windows, feature = "engine"))]
pub mod wasapi;

use crate::types::{AppTarget, AudioFormat, DeviceRef};
use serde::{Deserialize, Serialize};
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
    #[error("{0}")]
    Backend(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Microphones and other recording endpoints.
    Capture,
    /// Speakers and other playback endpoints.
    Render,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Stable id, usable in [`DeviceRef::Id`].
    pub id: String,
    pub name: String,
    pub direction: Direction,
    pub is_default: bool,
    /// The device's native format in shared mode, when known.
    pub format: Option<AudioFormat>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInfo {
    pub pid: u32,
    /// Executable file name, e.g. `chrome.exe`.
    pub name: String,
    /// Title of its audio session, when it has one.
    pub title: Option<String>,
    /// True while it is producing sound.
    pub active: bool,
}

/// What a backend can do.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub devices: bool,
    pub app_capture: bool,
    pub virtual_devices: bool,
}

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
}
