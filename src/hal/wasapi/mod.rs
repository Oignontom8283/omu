//! Windows audio backend (WASAPI). Only compiled on Windows.
//!
//! * devices: shared-mode, event-driven, `f32`, in the device's own format;
//! * applications: list the programs that have an audio session; capture the sound of one program and its child
//!   processes before the mix (process loopback, Windows 10 build 20348 / Windows 11), never this program's own;
//! * threads: every stream thread asks the multimedia scheduler for "Pro Audio" priority.
//!
//! Nothing here panics into the caller; a broken stream flags its health and goes silent.

mod apps;
mod com;
mod devices;
mod hotkey;
mod loopback;
mod stream;

use crate::hal::{
    AppInfo, Backend, Capabilities, CaptureStream, DeviceInfo, Direction, HalError, RenderStream,
    StreamHealth,
};
use crate::types::{AppTarget, AudioFormat, DeviceRef, ENGINE_CHANNELS, ENGINE_SAMPLE_RATE};
use com::{backend_error, ComGuard};
use std::sync::Arc;
use stream::{Client, WasapiCapture, WasapiRender};
use windows::Win32::Media::Audio::IAudioClient;
use windows::Win32::System::Com::CLSCTX_ALL;

pub use apps::{processes, resolve_target, ProcessEntry};

/// A function for [`vl_engine`]'s graph thread: moves it into the multimedia scheduler's "Pro Audio" class.
pub fn graph_thread_init() -> Arc<dyn Fn() + Send + Sync> {
    Arc::new(|| {
        // The class applies until the thread ends: the guard is deliberately not dropped.
        std::mem::forget(stream::Boost::new());
    })
}

#[derive(Default)]
pub struct WasapiBackend;

impl WasapiBackend {
    pub fn new() -> Self {
        WasapiBackend
    }
}

fn activate_device(
    device: &windows::Win32::Media::Audio::IMMDevice,
) -> Result<IAudioClient, HalError> {
    unsafe { device.Activate(CLSCTX_ALL, None) }
        .map_err(|e| backend_error("IMMDevice::Activate(IAudioClient)", e))
}

impl Backend for WasapiBackend {
    fn name(&self) -> &str {
        "WASAPI"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            devices: true,
            app_capture: true,
            virtual_devices: false,
            hotkeys: true,
        }
    }

    fn list_devices(&self) -> Result<Vec<DeviceInfo>, HalError> {
        let mut all: Vec<DeviceInfo> = vec![];
        for d in [Direction::Capture, Direction::Render] {
            all.extend(devices::list_direction(d)?.into_iter().map(|(_, i)| i));
        }
        Ok(all)
    }

    fn open_capture(&self, device: &DeviceRef) -> Result<Box<dyn CaptureStream>, HalError> {
        let _com = ComGuard::new();
        let (dev, info) = devices::resolve(Direction::Capture, device)?;
        let client = activate_device(&dev)?;
        let format = stream::initialize(&client, None, false)?;
        Ok(Box::new(WasapiCapture {
            client: Client(client),
            format,
            health: StreamHealth::new(),
            silence_fill: false,
            label: info.name,
        }))
    }

    fn open_render(&self, device: &DeviceRef) -> Result<Box<dyn RenderStream>, HalError> {
        let _com = ComGuard::new();
        let (dev, info) = devices::resolve(Direction::Render, device)?;
        let client = activate_device(&dev)?;
        let format = stream::initialize(&client, None, false)?;
        Ok(Box::new(WasapiRender {
            client: Client(client),
            format,
            health: StreamHealth::new(),
            label: info.name,
        }))
    }

    fn open_hotkey(
        &self,
        combo: &crate::types::KeyCombo,
        on_press: crate::hal::HotkeyCallback,
    ) -> Result<Box<dyn crate::hal::Running>, HalError> {
        hotkey::register(combo, on_press)
    }

    fn open_hotkey_hold(
        &self,
        combo: &crate::types::KeyCombo,
        on_press: crate::hal::HotkeyCallback,
        on_release: crate::hal::HotkeyCallback,
    ) -> Result<Box<dyn crate::hal::Running>, HalError> {
        hotkey::register_hold(combo, on_press, on_release)
    }

    fn list_apps(&self) -> Result<Vec<AppInfo>, HalError> {
        apps::list_apps()
    }

    fn open_app_capture(&self, target: &AppTarget) -> Result<Box<dyn CaptureStream>, HalError> {
        let _com = ComGuard::new();
        let pid = apps::resolve_target(target)?;
        let client = loopback::activate(pid)?;
        let format = stream::initialize(
            &client,
            Some(AudioFormat {
                sample_rate: ENGINE_SAMPLE_RATE,
                channels: ENGINE_CHANNELS,
            }),
            true,
        )?;
        Ok(Box::new(WasapiCapture {
            client: Client(client),
            format,
            health: StreamHealth::new(),
            silence_fill: true,
            label: format!("pid {pid}"),
        }))
    }
}
