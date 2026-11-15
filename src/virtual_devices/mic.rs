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
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// Whether the server listens on the product's fixed port: only then can the driver attach the devices again by
    /// itself (it remembers where the server is).
    pub fixed_port: bool,
    /// The bus id of each device, by kind and name: the same name is always the same bus id, so the driver finds a
    /// permanent device again after a restart.
    busids: Mutex<HashMap<String, String>>,
    /// The port number the driver gave to each attached device, by bus id.
    ports: Mutex<BTreeMap<String, u32>>,
    /// The ports last made permanent, to ask the driver only when the set changes.
    persisted: Mutex<Option<BTreeSet<u32>>>,
    /// Said once: other permanent devices are in the way, so ours are left alone.
    foreign_said: AtomicBool,
}

impl Shared {
    fn new(server: Server, fixed_port: bool) -> Shared {
        Shared {
            server: Arc::new(server),
            fixed_port,
            busids: Mutex::new(HashMap::new()),
            ports: Mutex::new(BTreeMap::new()),
            persisted: Mutex::new(None),
            foreign_said: AtomicBool::new(false),
        }
    }

    /// The bus id of the device called `name` on bus `bus` (1 for microphones, 2 for speakers), made from the name
    /// (`1-418`) and kept: nothing else takes it while this program runs.
    pub(super) fn busid_for(&self, bus: u32, name: &str) -> String {
        let key = format!("{bus}:{}", name.trim().to_lowercase());
        let mut ids = lock(&self.busids);
        if let Some(id) = ids.get(&key) {
            return id.clone();
        }
        let mut n = name_hash(&key) % 900 + 1;
        let id = loop {
            let candidate = format!("{bus}-{n}");
            if !ids.values().any(|v| *v == candidate) {
                break candidate;
            }
            n = n % 999 + 1;
        };
        ids.insert(key, id.clone());
        id
    }
}

/// The provider of virtual microphones through USB/IP.
pub struct UsbMicProvider {
    tool: Arc<dyn UsbipTool>,
    /// The TCP port of the USB/IP server: the product's fixed one (0 would be any free one).
    server_port: u16,
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
            server_port: crate::branding::VIRTUAL_SERVER_PORT,
            vendor_id: crate::branding::VIRTUAL_VENDOR_ID,
            product_id: crate::branding::VIRTUAL_PRODUCT_ID,
            manufacturer: crate::branding::VIRTUAL_MANUFACTURER.into(),
            shared: Mutex::new(None),
            names: Mutex::new(vec![]),
            speaker_names: Mutex::new(vec![]),
        }
    }

    /// What the system remembers of devices of ours that are gone and not in `kept`, with the records to forget.
    fn stale_records(
        &self,
        kept: &crate::types::KeptDevices,
    ) -> Result<Vec<(crate::types::StaleDevice, Vec<String>)>, HalError> {
        if !cfg!(windows) {
            return Err(HalError::Unsupported(
                "this system does not remember devices the way Windows does".into(),
            ));
        }
        let all =
            super::stale::remembered(self.vendor_id, self.product_id).map_err(|f| match f {
                super::stale::Failure::NeedsAdministrator => {
                    HalError::NeedsAdministrator("listing the devices".into())
                }
                super::stale::Failure::Other(m) => HalError::Backend(m),
            })?;
        Ok(super::stale::classify(
            self.vendor_id,
            self.product_id,
            &all,
            kept,
        ))
    }

    /// The same provider with its server on another port (tests, which must not take the product's own).
    pub fn with_server_port(mut self, port: u16) -> UsbMicProvider {
        self.server_port = port;
        self
    }

    /// The server is started on the first microphone and kept for the others.
    fn server(&self) -> Result<Arc<Shared>, HalError> {
        let mut shared = lock(&self.shared);
        if let Some(s) = shared.as_ref() {
            return Ok(s.clone());
        }
        // The fixed port is what lets the driver attach the permanent devices by itself; if something else holds it,
        // work anyway on another one (the devices then only live as long as this run).
        let wanted = self.server_port;
        let (server, fixed) = match Server::start(wanted, Arc::new(MonotonicClock::new())) {
            Ok(server) => (server, wanted != 0),
            Err(e) => {
                tracing::warn!(
                    port = wanted,
                    "the USB/IP port is not free ({e}): using another one; the virtual devices will not come back \
                     by themselves after a restart"
                );
                let server = Server::start(0, Arc::new(MonotonicClock::new())).map_err(|e| {
                    HalError::Backend(format!("cannot start the USB/IP server: {e}"))
                })?;
                (server, false)
            }
        };
        self.detach_leftovers(server.port());
        let s = Arc::new(Shared::new(server, fixed));
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
            busid: shared.busid_for(1, name),
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
            busid: shared.busid_for(2, name),
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

    fn stale_devices(
        &self,
        kept: &crate::types::KeptDevices,
    ) -> Result<Vec<crate::types::StaleDevice>, HalError> {
        Ok(self
            .stale_records(kept)?
            .into_iter()
            .map(|(stale, _)| stale)
            .collect())
    }

    fn forget_stale_devices(&self, kept: &crate::types::KeptDevices) -> Result<(), HalError> {
        let ids: Vec<String> = self
            .stale_records(kept)?
            .into_iter()
            .flat_map(|(_, ids)| ids)
            .collect();
        if ids.is_empty() {
            return Ok(());
        }
        super::stale::forget(&ids).map(|_| ()).map_err(|f| match f {
            super::stale::Failure::NeedsAdministrator => {
                HalError::NeedsAdministrator("forgetting the old devices".into())
            }
            super::stale::Failure::Other(m) => HalError::Backend(m),
        })
    }

    fn persist(&self) {
        let Some(shared) = lock(&self.shared).clone() else {
            return;
        };
        if !shared.fixed_port {
            return; // the driver could not find the server again on its own
        }
        let wanted: BTreeSet<u32> = lock(&shared.ports).values().copied().collect();
        if lock(&shared.persisted).as_ref() == Some(&wanted) {
            return;
        }
        // The driver keeps ONE list for every program: asking replaces it. Leave it alone when it holds devices that
        // are not ours (we could not put them back).
        let ours = shared.server.port().to_string();
        match self.tool.stashed() {
            Ok(lines) if lines.iter().any(|l| !l.contains(&ours)) => {
                if !shared.foreign_said.swap(true, Ordering::Relaxed) {
                    tracing::info!(
                        "other permanent USB/IP devices are set up on this computer: ours are not made permanent"
                    );
                }
                return;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("cannot read the permanent USB/IP devices: {e}");
                return;
            }
        }
        let ports: Vec<u32> = wanted.iter().copied().collect();
        match self.tool.stash(&ports) {
            Ok(()) => tracing::info!(
                ?ports,
                "the virtual devices are permanent: the driver attaches them again at every start"
            ),
            Err(e) => tracing::warn!("cannot make the virtual devices permanent: {e}"),
        }
        *lock(&shared.persisted) = Some(wanted);
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
    let (port, adopted) = match a.tool.attach(server.port(), a.busid) {
        Ok(p) => (p, false),
        Err(e) => match already_attached(a, &server) {
            // the driver attached its permanent device by itself, a moment before us
            Some(p) => (p, true),
            None => {
                server.remove(a.busid);
                return Err(HalError::Busy(format!(
                    "cannot attach the {} {:?}: {e}",
                    a.what, a.name
                )));
            }
        },
    };
    lock(&a.shared.ports).insert(a.busid.to_owned(), port);
    tracing::info!(name = %a.name, busid = %a.busid, port, adopted, "{} attached", a.what);
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
            .spawn(move || watch(&server, &busid, &health, &stop, adopted))
            .ok()
    };
    Ok(Box::new(UsbRunning {
        shared: a.shared.clone(),
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

/// The port the driver gave to a device of ours that is already attached (the driver attached its permanent device
/// again by itself).
fn already_attached(a: &Attach<'_>, server: &Server) -> Option<u32> {
    a.tool
        .imported()
        .ok()?
        .into_iter()
        .find(|d| d.server_port == Some(server.port()) && d.busid.as_deref() == Some(a.busid))
        .map(|d| d.port)
}

/// Flags the device as failed when the system detaches it (sleep, driver restart...): the engine then recreates it.
/// A device the driver attached by itself (`adopted`) must show up on our server within a few seconds, or it is a
/// leftover of a connection that is gone: it is flagged too, and recreated.
fn watch(server: &Server, busid: &str, health: &StreamHealth, stop: &AtomicBool, adopted: bool) {
    let mut seen_attached = false;
    let mut gone_for = 0;
    let mut waited = 0;
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(500));
        if server.is_attached(busid) {
            seen_attached = true;
            gone_for = 0;
        } else if adopted && !seen_attached {
            waited += 1;
            if waited >= 20 {
                tracing::warn!(
                    busid,
                    "the permanent device is not connected to this server"
                );
                health.fail("the permanent device is not connected to this server");
                return;
            }
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
    shared: Arc<Shared>,
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
        lock(&self.shared.ports).remove(&self.busid);
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

    /// A provider whose server is on any free port (tests run side by side and must not fight over the real one).
    fn provider(tool: &Arc<FakeTool>) -> UsbMicProvider {
        UsbMicProvider::new(tool.clone() as Arc<dyn UsbipTool>).with_server_port(0)
    }

    /// A provider whose server is on a port that is free now and stays "the fixed port" for this provider.
    fn fixed_provider(tool: &Arc<FakeTool>) -> UsbMicProvider {
        let free = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        UsbMicProvider::new(tool.clone() as Arc<dyn UsbipTool>).with_server_port(free)
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
            assert_eq!(
                attached[0].1,
                p.server().unwrap().busid_for(1, "Stream"),
                "the bus id of its name"
            );
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
        let shared = p.server().unwrap();
        assert_eq!(
            ids,
            vec![shared.busid_for(1, "Alpha"), shared.busid_for(1, "Beta")]
        );
        assert_ne!(ids[0], ids[1]);
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
                busid: None,
            },
            Imported {
                port: 6,
                vendor_id: 0x046d,
                product_id: 0xc52b,
                server_port: Some(40001),
                busid: None,
            },
            Imported {
                port: 7,
                vendor_id: crate::branding::VIRTUAL_VENDOR_ID,
                product_id: crate::branding::VIRTUAL_PRODUCT_ID,
                server_port: None,
                busid: None,
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

    // ---- through the whole engine: what the user does in the panel (rename a block, remove it)

    fn engine_with(tool: &Arc<FakeTool>) -> crate::engine::Engine {
        engine_over(provider(tool))
    }

    fn engine_over(provider: UsbMicProvider) -> crate::engine::Engine {
        use crate::engine::{Engine, EngineConfig};
        use crate::hal::sim::SimBackend;
        use crate::hal::{Backend, VirtualDeviceProvider};
        let sim = SimBackend::new();
        Engine::new(
            Arc::new(sim) as Arc<dyn Backend>,
            Some(Arc::new(provider) as Arc<dyn VirtualDeviceProvider>),
            EngineConfig::default(),
        )
    }

    fn mic_graph(names: &[&str]) -> crate::graph::ValidGraph {
        use crate::types::{GraphSpec, NodeKind, NodeSpec};
        let spec = GraphSpec {
            nodes: names
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    NodeSpec::new(
                        &format!("mic{i}"),
                        NodeKind::VirtualMic { name: (*n).into() },
                    )
                })
                .collect(),
            edges: vec![],
        };
        crate::graph::validate(&spec).unwrap()
    }

    fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
        let start = std::time::Instant::now();
        while !cond() {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "timed out: {what}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn renaming_a_microphone_block_replaces_the_device_and_removing_it_removes_the_device() {
        let tool = Arc::new(FakeTool::default());
        let engine = engine_with(&tool);
        let attached = || tool.attached.lock().unwrap().len();
        let detached = || tool.detached.lock().unwrap().clone();

        engine.apply(mic_graph(&["Alpha"]));
        wait_until("the first device", || attached() == 1);
        assert!(detached().is_empty());

        // the same block, another name: the old device goes away, a new one is attached
        engine.apply(mic_graph(&["Beta"]));
        wait_until("the renamed device", || attached() == 2);
        wait_until("the old device removed", || detached() == vec![1]);

        // a graph without the block: its device goes away
        engine.apply(mic_graph(&[]));
        wait_until("the device of the removed block", || {
            detached() == vec![1, 2]
        });

        // nothing else was touched along the way
        assert_eq!(attached(), 2);
    }

    #[test]
    fn an_edit_that_does_not_touch_the_name_keeps_the_device() {
        let tool = Arc::new(FakeTool::default());
        let engine = engine_with(&tool);
        engine.apply(mic_graph(&["Alpha"]));
        wait_until("the device", || tool.attached.lock().unwrap().len() == 1);
        // another block is added: the first microphone is not detached and attached again
        engine.apply(mic_graph(&["Alpha", "Gamma"]));
        wait_until("the second device", || {
            tool.attached.lock().unwrap().len() == 2
        });
        std::thread::sleep(Duration::from_millis(300));
        assert!(tool.detached.lock().unwrap().is_empty());
    }

    #[test]
    fn naming_a_block_does_not_touch_its_device() {
        let tool = Arc::new(FakeTool::default());
        let engine = engine_with(&tool);
        engine.apply(mic_graph(&["Alpha"]));
        wait_until("the device", || tool.attached.lock().unwrap().len() == 1);
        // the same block and the same device name, but the block gets a display name
        let mut named = mic_graph(&["Alpha"]).spec;
        named.nodes[0].display = Some("Mon micro".into());
        engine.apply(crate::graph::validate(&named).unwrap());
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(tool.attached.lock().unwrap().len(), 1, "not attached again");
        assert!(tool.detached.lock().unwrap().is_empty(), "not detached");
    }

    // ---- permanent devices

    #[test]
    fn bus_ids_come_from_the_name_and_stay_the_same() {
        let tool = Arc::new(FakeTool::default());
        let shared = provider(&tool).server().unwrap();
        let a = shared.busid_for(1, "Alpha");
        assert_eq!(a, shared.busid_for(1, "  alpha "), "same name, same bus id");
        assert_ne!(a, shared.busid_for(1, "Beta"));
        assert_ne!(
            a,
            shared.busid_for(2, "Alpha"),
            "a speaker is not the microphone of the same name"
        );
        for id in [&a, &shared.busid_for(2, "Alpha")] {
            let (bus, dev) = id.split_once('-').expect("bus-device");
            assert!(
                bus.parse::<u32>().is_ok() && dev.parse::<u32>().is_ok(),
                "{id}"
            );
            assert!(id.len() < 32, "USB/IP bus ids are at most 31 bytes");
        }
        // the same answer in another run: that is what lets the driver find a permanent device again
        let again = provider(&tool).server().unwrap();
        assert_eq!(a, again.busid_for(1, "Alpha"));
    }

    #[test]
    fn bus_ids_never_collide_even_when_names_hash_alike() {
        let tool = Arc::new(FakeTool::default());
        let shared = provider(&tool).server().unwrap();
        let ids: std::collections::HashSet<String> = (0..600)
            .map(|i| shared.busid_for(1, &format!("device number {i}")))
            .collect();
        assert_eq!(ids.len(), 600, "600 names, 600 different bus ids");
    }

    #[test]
    fn the_devices_are_made_permanent_when_the_set_changes_and_only_then() {
        let tool = Arc::new(FakeTool::default());
        let p = fixed_provider(&tool);
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
        p.persist();
        assert_eq!(
            *tool.stashes.lock().unwrap(),
            vec![vec![1, 2]],
            "both, by their driver ports"
        );
        p.persist();
        assert_eq!(
            tool.stashes.lock().unwrap().len(),
            1,
            "nothing changed, nothing asked"
        );
        // a device that is gone is no longer permanent (the list is replaced, not added to)
        drop(a);
        p.persist();
        assert_eq!(tool.stashes.lock().unwrap().last().unwrap(), &vec![2]);
        drop(b);
        p.persist();
        assert_eq!(
            tool.stashes.lock().unwrap().last().unwrap(),
            &Vec::<u32>::new()
        );
    }

    #[test]
    fn the_permanent_devices_of_other_programs_are_left_alone() {
        let tool = Arc::new(FakeTool::default());
        *tool.stashed_lines.lock().unwrap() = vec!["host=192.168.1.5;port=3240;busid=3-2".into()];
        let p = fixed_provider(&tool);
        let _a = p
            .create_microphone("Alpha")
            .unwrap()
            .start(Box::new(|_| {}))
            .unwrap();
        p.persist();
        assert!(
            tool.stashes.lock().unwrap().is_empty(),
            "asking would have erased theirs"
        );
        // ours alone in the list are replaced as usual
        let tool = Arc::new(FakeTool::default());
        let p = fixed_provider(&tool);
        let port = p.server().unwrap().server.port();
        *tool.stashed_lines.lock().unwrap() = vec![format!("host=127.0.0.1;port={port};busid=1-5")];
        let _a = p
            .create_microphone("Alpha")
            .unwrap()
            .start(Box::new(|_| {}))
            .unwrap();
        p.persist();
        assert_eq!(tool.stashes.lock().unwrap().len(), 1);
    }

    #[test]
    fn without_the_fixed_port_nothing_is_made_permanent() {
        // something else holds the port: the provider works on another one, but the driver could not find it again
        let busy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = busy.local_addr().unwrap().port();
        let tool = Arc::new(FakeTool::default());
        let p = UsbMicProvider::new(tool.clone() as Arc<dyn UsbipTool>).with_server_port(port);
        let _a = p
            .create_microphone("Alpha")
            .unwrap()
            .start(Box::new(|_| {}))
            .unwrap();
        assert_eq!(tool.attached.lock().unwrap().len(), 1, "it still works");
        p.persist();
        assert!(tool.stashes.lock().unwrap().is_empty());
        drop(busy);
    }

    #[test]
    fn a_device_the_driver_already_attached_by_itself_is_taken_over() {
        use crate::virtual_devices::tool::Imported;
        let tool = Arc::new(FakeTool::default());
        let p = fixed_provider(&tool);
        let shared = p.server().unwrap();
        // the driver brought the permanent device back a moment before we asked: our own attach is refused
        *tool.attach_error.lock().unwrap() = Some("already attached".into());
        tool.imported.lock().unwrap().push(Imported {
            port: 9,
            vendor_id: crate::branding::VIRTUAL_VENDOR_ID,
            product_id: crate::branding::VIRTUAL_PRODUCT_ID,
            server_port: Some(shared.server.port()),
            busid: Some(shared.busid_for(1, "Alpha")),
        });
        let running = p
            .create_microphone("Alpha")
            .unwrap()
            .start(Box::new(|_| {}))
            .unwrap();
        // it is ours now: dropping it detaches that port
        drop(running);
        assert_eq!(*tool.detached.lock().unwrap(), vec![9]);
        // a refused attach with nothing of ours attached is still an error
        let err = p
            .create_microphone("Beta")
            .unwrap()
            .start(Box::new(|_| {}))
            .err()
            .unwrap();
        assert!(matches!(err, HalError::Busy(_)), "{err}");
    }

    #[test]
    fn stopping_the_engine_keeps_the_virtual_devices_and_ending_the_program_removes_them() {
        let tool = Arc::new(FakeTool::default());
        let engine = engine_with(&tool);
        engine.apply(mic_graph(&["Alpha"]));
        wait_until("the device", || tool.attached.lock().unwrap().len() == 1);
        engine.stop();
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            tool.detached.lock().unwrap().is_empty(),
            "the microphone stays while the engine is stopped"
        );
        // the same graph again reuses it
        engine.apply(mic_graph(&["Alpha"]));
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(
            tool.attached.lock().unwrap().len(),
            1,
            "not attached a second time"
        );
        assert!(tool.detached.lock().unwrap().is_empty());
        // a graph without it removes it, even after a stop
        engine.stop();
        engine.apply(mic_graph(&[]));
        wait_until("the device of the removed block", || {
            *tool.detached.lock().unwrap() == vec![1]
        });
        // and when the program ends nothing is left attached
        engine.apply(mic_graph(&["Gamma"]));
        wait_until("a new device", || tool.attached.lock().unwrap().len() == 2);
        drop(engine);
        assert_eq!(*tool.detached.lock().unwrap(), vec![1, 2]);
    }

    #[test]
    fn the_engine_makes_the_devices_permanent_and_updates_the_list_as_blocks_change() {
        let tool = Arc::new(FakeTool::default());
        let engine = engine_over(fixed_provider(&tool));
        let last = || tool.stashes.lock().unwrap().last().cloned();
        engine.apply(mic_graph(&["Alpha"]));
        wait_until("permanent", || last() == Some(vec![1]));
        engine.apply(mic_graph(&["Beta"])); // renamed: a new device, on a new driver port
        wait_until("the renamed device", || last() == Some(vec![2]));
        engine.apply(mic_graph(&[])); // removed
        wait_until("nothing permanent", || last() == Some(vec![]));
        // ending the program does not empty the list: the devices of the last graph come back at the next start
        engine.apply(mic_graph(&["Gamma"]));
        wait_until("permanent again", || last() == Some(vec![3]));
        let asked = tool.stashes.lock().unwrap().len();
        drop(engine);
        assert_eq!(
            tool.stashes.lock().unwrap().len(),
            asked,
            "the program ending asks nothing"
        );
    }
}
