//! A simulated backend: sine-wave sources, recording sinks, optional clock drift. It lets the engine be tested
//! end to end on any machine, without audio hardware. Streams run on real time (10 ms chunks), so tests that use it
//! last a few real seconds.

use super::{
    AppInfo, Backend, Capabilities, CaptureCallback, CaptureStream, DeviceInfo, Direction,
    HalError, RenderCallback, RenderStream, Running, StreamHealth, VirtualDeviceProvider,
};
use crate::types::{AppTarget, AudioFormat, DeviceRef};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How a simulated source sounds.
#[derive(Debug, Clone, Copy)]
pub struct SimSource {
    pub format: AudioFormat,
    pub freq_hz: f64,
    pub amplitude: f32,
    /// Clock error in parts per million: +100 means the device runs 0.01 % fast.
    pub ppm: f64,
}

impl SimSource {
    pub fn tone(freq_hz: f64) -> Self {
        SimSource {
            format: AudioFormat::ENGINE,
            freq_hz,
            amplitude: 0.5,
            ppm: 0.0,
        }
    }
}

/// What a simulated sink recorded (interleaved samples) and how it runs.
#[derive(Debug, Clone)]
pub struct SimSink {
    pub format: AudioFormat,
    pub ppm: f64,
    pub recording: Arc<Mutex<Vec<f32>>>,
}

#[derive(Default)]
struct Shared {
    sources: HashMap<String, SimSource>,
    apps: HashMap<u32, (String, SimSource)>,
    sinks: HashMap<String, SimSink>,
    /// Health of every stream opened so far (to simulate device failures).
    healths: Vec<Arc<StreamHealth>>,
    /// When false, sinks throw away what they receive instead of recording it (long-running demos).
    discard: bool,
}

/// The simulated backend. Cloning shares the same world.
#[derive(Clone, Default)]
pub struct SimBackend {
    shared: Arc<Mutex<Shared>>,
}

impl SimBackend {
    pub fn new() -> Self {
        Self::default()
    }

    /// A capture device called `name` playing `source`.
    pub fn add_capture_device(&self, name: &str, source: SimSource) {
        self.shared
            .lock()
            .unwrap()
            .sources
            .insert(name.to_owned(), source);
    }

    /// An application with process id `pid`, playing `source`.
    pub fn add_app(&self, pid: u32, exe: &str, source: SimSource) {
        self.shared
            .lock()
            .unwrap()
            .apps
            .insert(pid, (exe.to_owned(), source));
    }

    /// A playback device called `name`; returns the buffer receiving everything played to it.
    pub fn add_render_device(
        &self,
        name: &str,
        format: AudioFormat,
        ppm: f64,
    ) -> Arc<Mutex<Vec<f32>>> {
        let recording = Arc::new(Mutex::new(vec![]));
        self.shared.lock().unwrap().sinks.insert(
            name.to_owned(),
            SimSink {
                format,
                ppm,
                recording: recording.clone(),
            },
        );
        recording
    }

    /// Makes the sinks opened from now on discard their audio instead of recording it (for demos that run for
    /// hours: a recording would grow without bound).
    pub fn discard_audio(&self) {
        self.shared.lock().unwrap().discard = true;
    }

    /// Makes every stream opened so far fail, like a device that was unplugged: they stop delivering audio.
    pub fn fail_all_streams(&self, message: &str) {
        for h in &self.shared.lock().unwrap().healths {
            h.fail(message);
        }
    }

    /// The buffer recording what was played to the sink (device or `virtual:<name>`), if it exists.
    pub fn recording_of(&self, sink: &str) -> Option<Arc<Mutex<Vec<f32>>>> {
        self.shared
            .lock()
            .unwrap()
            .sinks
            .get(sink)
            .map(|s| s.recording.clone())
    }

    fn track(&self, health: &Arc<StreamHealth>) {
        self.shared.lock().unwrap().healths.push(health.clone());
    }

    fn find<T: Clone>(map: &HashMap<String, T>, device: &DeviceRef) -> Option<(String, T)> {
        match device {
            DeviceRef::Default => map
                .iter()
                .min_by_key(|(k, _)| (*k).clone())
                .map(|(k, v)| (k.clone(), v.clone())),
            DeviceRef::Id(id) => map.get(id).map(|v| (id.clone(), v.clone())),
            DeviceRef::Name(part) => {
                let part = part.to_lowercase();
                map.iter()
                    .find(|(k, _)| k.to_lowercase().contains(&part))
                    .map(|(k, v)| (k.clone(), v.clone()))
            }
        }
    }
}

struct Pacer {
    started: Instant,
    frames_done: u64,
    rate: f64,
}

impl Pacer {
    fn new(rate: u32, ppm: f64) -> Self {
        Pacer {
            started: Instant::now(),
            frames_done: 0,
            rate: rate as f64 * (1.0 + ppm * 1e-6),
        }
    }
    /// Sleeps until `frames` more frames are due, then books them.
    fn wait(&mut self, frames: u64, stop: &AtomicBool) {
        self.frames_done += frames;
        let due = self.started + Duration::from_secs_f64(self.frames_done as f64 / self.rate);
        while Instant::now() < due {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_micros(500).min(due - Instant::now()));
        }
    }
}

struct Guard {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Running for Guard {}

struct SimCapture {
    source: SimSource,
    health: Arc<StreamHealth>,
}

impl CaptureStream for SimCapture {
    fn format(&self) -> AudioFormat {
        self.source.format
    }
    fn health(&self) -> Arc<StreamHealth> {
        self.health.clone()
    }
    fn start(self: Box<Self>, mut callback: CaptureCallback) -> Result<Box<dyn Running>, HalError> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let (s, health) = (self.source, self.health.clone());
        let thread = std::thread::spawn(move || {
            let chunk = (s.format.sample_rate / 100) as usize;
            let ch = s.format.channels as usize;
            let mut buf = vec![0.0f32; chunk * ch];
            let mut pacer = Pacer::new(s.format.sample_rate, s.ppm);
            let mut n = 0u64;
            while !stop2.load(Ordering::Relaxed) && !health.has_failed() {
                for f in 0..chunk {
                    let v = ((n + f as u64) as f64 * s.freq_hz * std::f64::consts::TAU
                        / s.format.sample_rate as f64)
                        .sin() as f32
                        * s.amplitude;
                    buf[f * ch..(f + 1) * ch].fill(v);
                }
                n += chunk as u64;
                pacer.wait(chunk as u64, &stop2);
                callback(&buf);
                health.add_frames(chunk as u64);
            }
        });
        Ok(Box::new(Guard {
            stop,
            thread: Some(thread),
        }))
    }
}

struct SimRender {
    sink: SimSink,
    health: Arc<StreamHealth>,
    discard: bool,
}

impl RenderStream for SimRender {
    fn format(&self) -> AudioFormat {
        self.sink.format
    }
    fn health(&self) -> Arc<StreamHealth> {
        self.health.clone()
    }
    fn start(self: Box<Self>, mut callback: RenderCallback) -> Result<Box<dyn Running>, HalError> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let (sink, health, discard) = (self.sink, self.health.clone(), self.discard);
        let thread = std::thread::spawn(move || {
            let chunk = (sink.format.sample_rate / 100) as usize;
            let mut buf = vec![0.0f32; chunk * sink.format.channels as usize];
            let mut pacer = Pacer::new(sink.format.sample_rate, sink.ppm);
            while !stop2.load(Ordering::Relaxed) && !health.has_failed() {
                callback(&mut buf);
                if !discard {
                    sink.recording.lock().unwrap().extend_from_slice(&buf);
                }
                health.add_frames(chunk as u64);
                pacer.wait(chunk as u64, &stop2);
            }
        });
        Ok(Box::new(Guard {
            stop,
            thread: Some(thread),
        }))
    }
}

impl Backend for SimBackend {
    fn name(&self) -> &str {
        "simulated"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            devices: true,
            app_capture: true,
            virtual_devices: true,
        }
    }

    fn list_devices(&self) -> Result<Vec<DeviceInfo>, HalError> {
        let s = self.shared.lock().unwrap();
        let mut out: Vec<DeviceInfo> = s
            .sources
            .iter()
            .map(|(n, src)| (n, Direction::Capture, src.format))
            .chain(
                s.sinks
                    .iter()
                    .map(|(n, sink)| (n, Direction::Render, sink.format)),
            )
            .map(|(n, d, f)| DeviceInfo {
                id: n.clone(),
                name: n.clone(),
                direction: d,
                is_default: false,
                format: Some(f),
            })
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    fn open_capture(&self, device: &DeviceRef) -> Result<Box<dyn CaptureStream>, HalError> {
        let s = self.shared.lock().unwrap();
        let (_, source) = Self::find(&s.sources, device)
            .ok_or_else(|| HalError::NotFound(format!("{device:?}")))?;
        let health = StreamHealth::new();
        drop(s);
        self.track(&health);
        Ok(Box::new(SimCapture { source, health }))
    }

    fn open_render(&self, device: &DeviceRef) -> Result<Box<dyn RenderStream>, HalError> {
        let s = self.shared.lock().unwrap();
        let (_, sink) = Self::find(&s.sinks, device)
            .ok_or_else(|| HalError::NotFound(format!("{device:?}")))?;
        let (health, discard) = (StreamHealth::new(), s.discard);
        drop(s);
        self.track(&health);
        Ok(Box::new(SimRender {
            sink,
            health,
            discard,
        }))
    }

    fn list_apps(&self) -> Result<Vec<AppInfo>, HalError> {
        let s = self.shared.lock().unwrap();
        let mut apps: Vec<AppInfo> = s
            .apps
            .iter()
            .map(|(pid, (name, _))| AppInfo {
                pid: *pid,
                name: name.clone(),
                title: None,
                active: true,
            })
            .collect();
        apps.sort_by_key(|a| a.pid);
        Ok(apps)
    }

    fn open_app_capture(&self, target: &AppTarget) -> Result<Box<dyn CaptureStream>, HalError> {
        let s = self.shared.lock().unwrap();
        let found = match target {
            AppTarget::Pid(pid) => s.apps.get(pid).map(|(_, src)| *src),
            AppTarget::ProcessName(n) => s
                .apps
                .values()
                .find(|(name, _)| name.eq_ignore_ascii_case(n))
                .map(|(_, src)| *src),
        };
        let source = found.ok_or_else(|| HalError::NotFound(format!("{target:?}")))?;
        let health = StreamHealth::new();
        drop(s);
        self.track(&health);
        Ok(Box::new(SimCapture { source, health }))
    }
}

impl VirtualDeviceProvider for SimBackend {
    fn name(&self) -> &str {
        "simulated virtual devices"
    }

    fn available(&self) -> Result<(), HalError> {
        Ok(())
    }

    fn microphone_names(&self) -> Vec<String> {
        let s = self.shared.lock().unwrap();
        s.sinks
            .keys()
            .filter_map(|k| k.strip_prefix("virtual:"))
            .map(str::to_owned)
            .collect()
    }

    fn create_microphone(&self, name: &str) -> Result<Box<dyn RenderStream>, HalError> {
        let key = format!("virtual:{name}");
        let mut s = self.shared.lock().unwrap();
        let sink = s
            .sinks
            .entry(key)
            .or_insert_with(|| SimSink {
                format: AudioFormat::ENGINE,
                ppm: 0.0,
                recording: Arc::default(),
            })
            .clone();
        let discard = s.discard;
        drop(s);
        let health = StreamHealth::new();
        self.track(&health);
        Ok(Box::new(SimRender {
            sink,
            health,
            discard,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_delivers_a_tone_at_real_time_pace() {
        let sim = SimBackend::new();
        sim.add_capture_device("mic", SimSource::tone(1000.0));
        let got = Arc::new(Mutex::new(vec![]));
        let g2 = got.clone();
        let stream = sim.open_capture(&DeviceRef::Name("mic".into())).unwrap();
        let running = stream
            .start(Box::new(move |b| g2.lock().unwrap().extend_from_slice(b)))
            .unwrap();
        std::thread::sleep(Duration::from_millis(500));
        drop(running);
        let frames = got.lock().unwrap().len() / 2;
        assert!(
            (20_000..=26_500).contains(&frames),
            "{frames} frames in 0.5 s"
        );
    }

    #[test]
    fn render_records_what_the_callback_writes_and_stops_on_drop() {
        let sim = SimBackend::new();
        let rec = sim.add_render_device("out", AudioFormat::ENGINE, 0.0);
        let stream = sim.open_render(&DeviceRef::Default).unwrap();
        let running = stream.start(Box::new(|b| b.fill(0.25))).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        drop(running);
        let len = rec.lock().unwrap().len();
        assert!(len > 0 && rec.lock().unwrap().iter().all(|s| *s == 0.25));
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            rec.lock().unwrap().len(),
            len,
            "no more audio after the stream is dropped"
        );
    }

    #[test]
    fn apps_are_found_by_pid_and_by_name() {
        let sim = SimBackend::new();
        sim.add_app(42, "Chrome.exe", SimSource::tone(440.0));
        assert_eq!(sim.list_apps().unwrap()[0].pid, 42);
        assert!(sim.open_app_capture(&AppTarget::Pid(42)).is_ok());
        assert!(sim
            .open_app_capture(&AppTarget::ProcessName("chrome.EXE".into()))
            .is_ok());
        assert!(matches!(
            sim.open_app_capture(&AppTarget::Pid(1)),
            Err(HalError::NotFound(_))
        ));
    }

    #[test]
    fn failed_streams_stop_delivering() {
        let sim = SimBackend::new();
        sim.add_capture_device("mic", SimSource::tone(440.0));
        let count = Arc::new(Mutex::new(0usize));
        let c2 = count.clone();
        let stream = sim.open_capture(&DeviceRef::Default).unwrap();
        let health = stream.health();
        let running = stream
            .start(Box::new(move |b| *c2.lock().unwrap() += b.len()))
            .unwrap();
        std::thread::sleep(Duration::from_millis(100));
        sim.fail_all_streams("unplugged");
        std::thread::sleep(Duration::from_millis(60));
        let frozen = *count.lock().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(*count.lock().unwrap(), frozen);
        assert!(health.has_failed() && health.error().as_deref() == Some("unplugged"));
        drop(running);
    }

    #[test]
    fn unknown_devices_are_reported_not_panicked_on() {
        let sim = SimBackend::new();
        assert!(matches!(
            sim.open_capture(&DeviceRef::Id("x".into())),
            Err(HalError::NotFound(_))
        ));
        assert!(matches!(
            sim.open_render(&DeviceRef::Default),
            Err(HalError::NotFound(_))
        ));
    }

    #[test]
    fn virtual_microphones_are_recordable_sinks() {
        let sim = SimBackend::new();
        sim.available().unwrap();
        let mic = sim.create_microphone("Stream").unwrap();
        let running = mic.start(Box::new(|b| b.fill(0.1))).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        drop(running);
        assert!(sim
            .list_devices()
            .unwrap()
            .iter()
            .any(|d| d.id == "virtual:Stream"));
    }
}
