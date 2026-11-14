//! Virtual microphones made of emulated USB audio devices.
//!
//! Each microphone is a USB Audio Class 2 device (a stereo capture stream, 48 kHz, 24 bits) served by our USB/IP
//! server and attached to Windows by the `usbip-win2` client. Windows binds its own audio driver to it and shows it as
//! "Microphone (<name>)": Discord, OBS or a browser can pick it. The audio we play into the node comes out of
//! that microphone.
//!
//! The device is identified to Windows by a vendor/product id (from the branding) and a serial number derived from the
//! microphone's name: the same name is the same device after a restart, so Windows keeps its settings (default
//! device, volume) instead of showing a new microphone each time.

use super::speaker::{serial_of as speaker_serial, SpeakerStream};
use super::tool::UsbipTool;
use super::usb::uac2::{Uac2Config, Uac2Device, Uac2Streams};
use super::usb::{MonotonicClock, Server, UsbDevice};
use crate::hal::{
    CaptureStream, HalError, RenderCallback, RenderStream, Running, StreamHealth,
    VirtualDeviceProvider,
};
use crate::types::{AudioFormat, ENGINE_CHANNELS, ENGINE_SAMPLE_RATE};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

pub(super) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Samples are 24 bits left-justified in 32-bit slots.
pub(super) const FULL_SCALE_24: f32 = 8_388_607.0;

/// Where the audio of a microphone comes from: the engine's sink callback, asked for as many frames as the USB
/// host reads.
struct MicFeed {
    callback: Mutex<RenderCallback>,
    scratch: Mutex<Vec<f32>>,
}

impl Uac2Streams for MicFeed {
    // This device has no playback stream.
    fn playback(&self, _frame: u64, _samples: &[i32]) {}

    fn capture(&self, _frame: u64, samples: &mut [i32]) {
        let mut scratch = lock(&self.scratch);
        if scratch.len() < samples.len() {
            scratch.resize(samples.len(), 0.0);
        }
        let floats = &mut scratch[..samples.len()];
        floats.fill(0.0);
        (lock(&self.callback))(floats);
        for (out, &x) in samples.iter_mut().zip(floats.iter()) {
            *out = ((x.clamp(-1.0, 1.0) * FULL_SCALE_24) as i32) << 8;
        }
    }
}

pub(super) struct Shared {
    pub server: Arc<Server>,
    pub next_bus: AtomicU32,
}

/// The provider of virtual microphones through USB/IP.
pub struct UsbMicProvider {
    tool: Arc<dyn UsbipTool>,
    vendor_id: u16,
    product_id: u16,
    manufacturer: String,
    shared: Mutex<Option<Arc<Shared>>>,
    names: Mutex<Vec<String>>,
    speaker_names: Mutex<Vec<String>>,
}

impl UsbMicProvider {
    /// A provider using `tool` to attach, with the vendor/product id and manufacturer name of the branding.
    pub fn new(tool: Arc<dyn UsbipTool>) -> UsbMicProvider {
        UsbMicProvider {
            tool,
            vendor_id: crate::branding::VIRTUAL_VENDOR_ID,
            product_id: crate::branding::VIRTUAL_PRODUCT_ID,
            manufacturer: crate::branding::VIRTUAL_MANUFACTURER.into(),
            shared: Mutex::new(None),
            names: Mutex::new(vec![]),
            speaker_names: Mutex::new(vec![]),
        }
    }

    /// The server is started on the first microphone and kept for the others.
    fn server(&self) -> Result<Arc<Shared>, HalError> {
        let mut shared = lock(&self.shared);
        if let Some(s) = shared.as_ref() {
            return Ok(s.clone());
        }
        let server = Server::start(0, Arc::new(MonotonicClock::new()))
            .map_err(|e| HalError::Backend(format!("cannot start the USB/IP server: {e}")))?;
        self.detach_leftovers(server.port());
        let s = Arc::new(Shared {
            server: Arc::new(server),
            next_bus: AtomicU32::new(1),
        });
        *shared = Some(s.clone());
        Ok(s)
    }
}

impl UsbMicProvider {
    /// A previous run that crashed leaves its devices attached to a server that is gone: remove those (only ours, by
    /// vendor/product id and a server on this machine that is not the one just started).
    fn detach_leftovers(&self, live_server_port: u16) {
        let Ok(list) = self.tool.imported() else {
            return;
        };
        for d in list {
            let ours = d.vendor_id == self.vendor_id && d.product_id == self.product_id;
            if ours && d.server_port.is_some_and(|p| p != live_server_port) {
                let _ = self.tool.detach(d.port);
            }
        }
    }
}

/// A stable identifier of a name (FNV-1a), for the device's serial number.
pub(super) fn name_hash(name: &str) -> u64 {
    name.trim()
        .to_lowercase()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325, |h, b| {
            (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
        })
}

impl VirtualDeviceProvider for UsbMicProvider {
    fn name(&self) -> &str {
        "USB/IP virtual devices"
    }

    fn available(&self) -> Result<(), HalError> {
        self.tool.available().map_err(HalError::Unsupported)
    }

    fn create_microphone(&self, name: &str) -> Result<Box<dyn RenderStream>, HalError> {
        self.available()?;
        let name = name.trim();
        if name.is_empty() {
            return Err(HalError::Backend(
                "a virtual microphone needs a name".into(),
            ));
        }
        let shared = self.server()?;
        let mut names = lock(&self.names);
        if !names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
            names.push(name.to_owned());
        }
        Ok(Box::new(MicStream {
            name: name.to_owned(),
            busid: format!("1-{}", shared.next_bus.fetch_add(1, Ordering::Relaxed)),
            config: Uac2Config {
                vendor_id: self.vendor_id,
                product_id: self.product_id,
                bcd_device: 0x0100,
                manufacturer: self.manufacturer.clone(),
                product: name.to_owned(),
                serial: format!("{:016x}", name_hash(name)),
                sample_rate: ENGINE_SAMPLE_RATE,
                playback_channels: 0,
                capture_channels: ENGINE_CHANNELS,
                playback_name: None,
                capture_name: None,
                capture_connector: false,
            },
            shared,
            tool: self.tool.clone(),
            health: StreamHealth::new(),
        }))
    }

    fn create_speaker(&self, name: &str) -> Result<Box<dyn CaptureStream>, HalError> {
        self.available()?;
        let name = name.trim();
        if name.is_empty() {
            return Err(HalError::Backend("a virtual speaker needs a name".into()));
        }
        let shared = self.server()?;
        let mut names = lock(&self.speaker_names);
        if !names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
            names.push(name.to_owned());
        }
        Ok(Box::new(SpeakerStream {
            name: name.to_owned(),
            busid: format!("1-{}", shared.next_bus.fetch_add(1, Ordering::Relaxed)),
            config: Uac2Config {
                vendor_id: self.vendor_id,
                product_id: self.product_id,
                bcd_device: 0x0100,
                manufacturer: self.manufacturer.clone(),
                product: name.to_owned(),
                serial: speaker_serial(name),
                sample_rate: ENGINE_SAMPLE_RATE,
                playback_channels: ENGINE_CHANNELS,
                capture_channels: 0,
                playback_name: None,
                capture_name: None,
                capture_connector: false,
            },
            shared,
            tool: self.tool.clone(),
            health: StreamHealth::new(),
        }))
    }

    fn speaker_names(&self) -> Vec<String> {
        lock(&self.speaker_names).clone()
    }

    fn microphone_names(&self) -> Vec<String> {
        lock(&self.names).clone()
    }
}

struct MicStream {
    name: String,
    busid: String,
    config: Uac2Config,
    shared: Arc<Shared>,
    tool: Arc<dyn UsbipTool>,
    health: Arc<StreamHealth>,
}

impl RenderStream for MicStream {
    fn format(&self) -> AudioFormat {
        AudioFormat::ENGINE
    }

    fn health(&self) -> Arc<StreamHealth> {
        self.health.clone()
    }

    fn start(self: Box<Self>, callback: RenderCallback) -> Result<Box<dyn Running>, HalError> {
        let feed = Arc::new(MicFeed {
            callback: Mutex::new(callback),
            scratch: Mutex::new(vec![]),
        });
        let device = Uac2Device::new(self.config.clone(), feed).map_err(HalError::Backend)?;
        attach(
            &Attach {
                what: "virtual microphone",
                name: &self.name,
                busid: &self.busid,
                shared: &self.shared,
                tool: &self.tool,
                health: &self.health,
            },
            Arc::new(device),
        )
    }
}

/// What [`attach`] needs to know about a virtual device (a microphone or a speaker).
pub(super) struct Attach<'a> {
    /// "virtual microphone" or "virtual speaker", for messages.
    pub what: &'static str,
    pub name: &'a str,
    pub busid: &'a str,
    pub shared: &'a Arc<Shared>,
    pub tool: &'a Arc<dyn UsbipTool>,
    pub health: &'a Arc<StreamHealth>,
}

/// Serves `device` on the USB/IP server and has Windows attach it. The returned guard detaches and removes it when
/// dropped, a monitor flags the stream as failed if the system detaches the device (sleep, driver restart), and a
/// crash cleanup detaches it if the program dies before the guard is dropped.
pub(super) fn attach(
    a: &Attach<'_>,
    device: Arc<dyn UsbDevice>,
) -> Result<Box<dyn Running>, HalError> {
    let server = a.shared.server.clone();
    server.add(a.busid, device);
    let port = match a.tool.attach(server.port(), a.busid) {
        Ok(p) => p,
        Err(e) => {
            server.remove(a.busid);
            return Err(HalError::Busy(format!(
                "cannot attach the {} {:?}: {e}",
                a.what, a.name
            )));
        }
    };
    tracing::info!(name = %a.name, busid = %a.busid, port, "{} attached", a.what);
    // If the program dies before this stream is dropped, Windows would keep a ghost device: detach it anyway.
    let cleanup = {
        let (tool, server, busid) = (a.tool.clone(), server.clone(), a.busid.to_owned());
        crate::diagnostics::on_crash_cleanup(a.what, move || {
            let _ = tool.detach(port);
            server.remove(&busid);
        })
    };
    let stop = Arc::new(AtomicBool::new(false));
    let monitor = {
        let (server, busid, health, stop) = (
            server.clone(),
            a.busid.to_owned(),
            a.health.clone(),
            stop.clone(),
        );
        std::thread::Builder::new()
            .name("virtual-device-monitor".into())
            .spawn(move || watch(&server, &busid, &health, &stop))
            .ok()
    };
    Ok(Box::new(UsbRunning {
        what: a.what,
        tool: a.tool.clone(),
        server,
        busid: a.busid.to_owned(),
        port,
        stop,
        monitor,
        _cleanup: cleanup,
    }))
}

/// Flags the device as failed when the system detaches it (sleep, driver restart...): the engine then recreates it.
fn watch(server: &Server, busid: &str, health: &StreamHealth, stop: &AtomicBool) {
    let mut seen_attached = false;
    let mut gone_for = 0;
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(500));
        if server.is_attached(busid) {
            seen_attached = true;
            gone_for = 0;
        } else if seen_attached {
            gone_for += 1;
            if gone_for >= 4 {
                tracing::warn!(busid, "the virtual device was detached by the system");
                health.fail("the virtual device was detached by the system");
                return;
            }
        }
    }
}

struct UsbRunning {
    what: &'static str,
    tool: Arc<dyn UsbipTool>,
    server: Arc<Server>,
    busid: String,
    port: u32,
    stop: Arc<AtomicBool>,
    monitor: Option<JoinHandle<()>>,
    /// Forgets the crash cleanup once this stream has detached the device itself.
    _cleanup: crate::diagnostics::Registration,
}

impl Running for UsbRunning {}

impl Drop for UsbRunning {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(m) = self.monitor.take() {
            let _ = m.join();
        }
        // Detach first (Windows removes the device), then stop serving it.
        tracing::info!(busid = %self.busid, port = self.port, "detaching the {}", self.what);
        if let Err(e) = self.tool.detach(self.port) {
            tracing::warn!(busid = %self.busid, "cannot detach the {}: {e}", self.what);
        }
        self.server.remove(&self.busid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::virtual_devices::tool::fake::FakeTool;
    use crate::virtual_devices::usb::{IsoTiming, UsbDevice};

    fn provider(tool: &Arc<FakeTool>) -> UsbMicProvider {
        UsbMicProvider::new(tool.clone() as Arc<dyn UsbipTool>)
    }

    #[test]
    fn the_feed_converts_floats_to_24_bit_left_justified_samples() {
        let feed = MicFeed {
            callback: Mutex::new(Box::new(|out: &mut [f32]| {
                for (i, s) in out.iter_mut().enumerate() {
                    *s = [0.0, 0.5, -0.5, 2.0, -2.0][i % 5];
                }
            })),
            scratch: Mutex::new(vec![]),
        };
        let mut samples = vec![0i32; 10];
        feed.capture(0, &mut samples);
        assert_eq!(samples[0], 0);
        assert_eq!(samples[1], (4_194_303) << 8);
        assert_eq!(samples[2], (-4_194_303) << 8);
        assert_eq!(samples[3], 8_388_607 << 8, "clipped to full scale");
        assert_eq!(samples[4], -8_388_607 << 8);
        assert!(
            samples.iter().all(|s| s & 0xff == 0),
            "the low byte is unused"
        );
    }

    #[test]
    fn a_usb_host_reading_the_microphone_gets_the_engines_audio() {
        // build the same device the provider would, and read it like Windows does
        let feed = Arc::new(MicFeed {
            callback: Mutex::new(Box::new(|out: &mut [f32]| out.fill(0.25))),
            scratch: Mutex::new(vec![]),
        });
        let config = Uac2Config {
            vendor_id: 0x1209,
            product_id: 0x0001,
            bcd_device: 0x0100,
            manufacturer: "t".into(),
            product: "t".into(),
            serial: "1".into(),
            sample_rate: 48_000,
            playback_channels: 0,
            capture_channels: 2,
            playback_name: None,
            capture_name: None,
            capture_connector: false,
        };
        let device = Uac2Device::new(config, feed).unwrap();
        let ep = 0x80 | 2; // the capture endpoint (the only streaming interface of this device)
        let capacity = vec![1024usize; 8];
        let mut data = vec![];
        let lengths = device.iso_in(
            ep,
            IsoTiming {
                start_ns: 0,
                packet_ns: 125_000,
            },
            &capacity,
            &mut data,
        );
        assert_eq!(lengths.len(), 8);
        assert!(
            lengths.iter().all(|&l| l == 6 * 2 * 4),
            "6 frames of 2 channels of 4 bytes per microframe: {lengths:?}"
        );
        let first = i32::from_le_bytes(data[..4].try_into().unwrap());
        assert_eq!(first, ((0.25f32 * FULL_SCALE_24) as i32) << 8);
    }

    #[test]
    fn availability_follows_the_tool() {
        let tool = Arc::new(FakeTool::default());
        let p = provider(&tool);
        assert!(p.available().is_ok());
        *tool.unavailable.lock().unwrap() = Some("usbip-win2 is not installed".into());
        assert!(
            matches!(p.available(), Err(HalError::Unsupported(m)) if m.contains("not installed"))
        );
        assert!(p.create_microphone("Mic").is_err());
    }

    #[test]
    fn starting_attaches_and_dropping_detaches() {
        let tool = Arc::new(FakeTool::default());
        let p = provider(&tool);
        let stream = p.create_microphone("Stream").unwrap();
        assert_eq!(stream.format(), AudioFormat::ENGINE);
        let running = stream.start(Box::new(|out| out.fill(0.1))).unwrap();
        {
            let attached = tool.attached.lock().unwrap();
            assert_eq!(attached.len(), 1);
            assert_eq!(attached[0].1, "1-1");
        }
        assert!(tool.detached.lock().unwrap().is_empty());
        drop(running);
        assert_eq!(
            *tool.detached.lock().unwrap(),
            vec![1],
            "exactly the port we attached"
        );
        assert_eq!(p.microphone_names(), vec!["Stream".to_string()]);
    }

    #[test]
    fn a_failed_attach_is_an_error_and_leaves_nothing_exported() {
        let tool = Arc::new(FakeTool::default());
        *tool.attach_error.lock().unwrap() = Some("driver refused".into());
        let p = provider(&tool);
        let err = p
            .create_microphone("Stream")
            .unwrap()
            .start(Box::new(|_| {}))
            .err()
            .unwrap();
        assert!(
            matches!(&err, HalError::Busy(m) if m.contains("driver refused")),
            "{err}"
        );
        assert!(
            tool.detached.lock().unwrap().is_empty(),
            "nothing was attached, nothing to detach"
        );
    }

    #[test]
    fn microphones_get_distinct_bus_ids_and_stable_serials() {
        let tool = Arc::new(FakeTool::default());
        let p = provider(&tool);
        let a = p
            .create_microphone("Alpha")
            .unwrap()
            .start(Box::new(|_| {}))
            .unwrap();
        let b = p
            .create_microphone("Beta")
            .unwrap()
            .start(Box::new(|_| {}))
            .unwrap();
        let ids: Vec<String> = tool
            .attached
            .lock()
            .unwrap()
            .iter()
            .map(|(_, b)| b.clone())
            .collect();
        assert_eq!(ids, vec!["1-1", "1-2"]);
        assert_eq!(
            name_hash("Alpha"),
            name_hash("  alpha "),
            "same name, same device"
        );
        assert_ne!(name_hash("Alpha"), name_hash("Beta"));
        drop((a, b));
    }

    #[test]
    fn devices_left_by_a_crashed_run_are_detached_but_others_are_not() {
        use crate::virtual_devices::tool::Imported;
        let tool = Arc::new(FakeTool::default());
        *tool.imported.lock().unwrap() = vec![
            Imported {
                port: 5,
                vendor_id: crate::branding::VIRTUAL_VENDOR_ID,
                product_id: crate::branding::VIRTUAL_PRODUCT_ID,
                server_port: Some(40000),
            },
            Imported {
                port: 6,
                vendor_id: 0x046d,
                product_id: 0xc52b,
                server_port: Some(40001),
            },
            Imported {
                port: 7,
                vendor_id: crate::branding::VIRTUAL_VENDOR_ID,
                product_id: crate::branding::VIRTUAL_PRODUCT_ID,
                server_port: None,
            },
        ];
        let p = provider(&tool);
        let _ = p.create_microphone("Mic").unwrap(); // starts the server, which cleans up first
        assert_eq!(
            *tool.detached.lock().unwrap(),
            vec![5],
            "ours and local only"
        );
    }

    #[test]
    fn an_empty_name_is_refused() {
        let tool = Arc::new(FakeTool::default());
        assert!(provider(&tool).create_microphone("  ").is_err());
    }
}
