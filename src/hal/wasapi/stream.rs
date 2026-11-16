//! Event-driven shared-mode WASAPI streams (capture and render) and their real-time threads.
//!
//! Every stream is `f32`. Device streams use the device's own mix format; the process-loopback stream (which has
//! no mix format) is asked for a fixed format and converted by the audio engine of Windows.
//! A stream that fails (device removed, driver error, no data for two seconds) flags its [`StreamHealth`],
//! stops its thread and goes silent; nothing here panics or aborts.

use super::com::{describe, ComGuard};
use crate::{
    CaptureCallback, CaptureStream, HalError, RenderCallback, RenderStream, Running, StreamHealth,
};
use omu_types::AudioFormat;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::core::{w, GUID};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Media::Audio::{
    IAudioCaptureClient, IAudioClient, IAudioRenderClient, AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
    WAVEFORMATEXTENSIBLE_0,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW, SetEvent,
    WaitForSingleObject,
};

const IEEE_FLOAT: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
const WAVE_FORMAT_EXTENSIBLE_TAG: u16 = 0xFFFE;
const WAVE_FORMAT_IEEE_FLOAT_TAG: u16 = 3;
/// A stream that receives no event for this long is declared dead (capture of an application is exempt: a silent
/// application produces no packets).
const STALL: Duration = Duration::from_secs(2);

/// An audio client that may move to the stream's thread.
///
/// SAFETY: every thread that touches WASAPI joins the multithreaded COM apartment (see `ComGuard`), where these
/// objects may be used from any thread; each client is only used by one thread at a time.
pub struct Client(pub IAudioClient);

unsafe impl Send for Client {}

impl std::ops::Deref for Client {
    type Target = IAudioClient;
    fn deref(&self) -> &IAudioClient {
        &self.0
    }
}

/// A Win32 event handle that closes itself.
pub struct Event(HANDLE);

// SAFETY: an event handle can be waited on and signalled from any thread.
unsafe impl Send for Event {}
unsafe impl Sync for Event {}

impl Event {
    pub fn new() -> Result<Event, HalError> {
        unsafe { CreateEventW(None, false, false, None) }
            .map(Event)
            .map_err(|e| HalError::Backend(describe("CreateEventW", &e)))
    }
    pub fn handle(&self) -> HANDLE {
        self.0
    }
    pub fn signal(&self) {
        let _ = unsafe { SetEvent(self.0) };
    }
    /// Waits for the event; true if it was signalled.
    pub fn wait(&self, timeout: Duration) -> bool {
        // WAIT_OBJECT_0 is 0
        let result = unsafe { WaitForSingleObject(self.0, timeout.as_millis() as u32) };
        result.0 == 0
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Raises the calling thread to the multimedia scheduler's "Pro Audio" class (low-latency audio scheduling).
/// Returns a guard that reverts it. Failure is ignored: the stream still works at normal priority.
pub struct Boost(Option<HANDLE>);

impl Boost {
    pub fn new() -> Boost {
        let mut index = 0u32;
        Boost(unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut index) }.ok())
    }
}

impl Default for Boost {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Boost {
    fn drop(&mut self) {
        if let Some(h) = self.0.take() {
            let _ = unsafe { AvRevertMmThreadCharacteristics(h) };
        }
    }
}

struct Guard {
    stop: Arc<AtomicBool>,
    event: Arc<Event>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.event.signal();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Running for Guard {}

fn float_format(format: AudioFormat) -> WAVEFORMATEXTENSIBLE {
    let ch = format.channels;
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_EXTENSIBLE_TAG,
            nChannels: ch,
            nSamplesPerSec: format.sample_rate,
            nAvgBytesPerSec: format.sample_rate * 4 * ch as u32,
            nBlockAlign: 4 * ch,
            wBitsPerSample: 32,
            cbSize: 22,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 {
            wValidBitsPerSample: 32,
        },
        dwChannelMask: match ch {
            1 => 0x4,
            2 => 0x3,
            _ => 0,
        },
        SubFormat: IEEE_FLOAT,
    }
}

/// Initialises `client` for event-driven shared mode in `f32`. With `fixed` the stream is asked for that format
/// (and Windows converts); otherwise the device's mix format is used. `loopback` adds the loopback flag.
pub fn initialize(
    client: &IAudioClient,
    fixed: Option<AudioFormat>,
    loopback: bool,
) -> Result<AudioFormat, HalError> {
    let mut flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
    if loopback {
        flags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
    }
    let err = |call: &str, e: windows::core::Error| HalError::Backend(describe(call, &e));
    if let Some(format) = fixed {
        let wf = float_format(format);
        flags |= AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
        unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                flags,
                0,
                0,
                &wf as *const WAVEFORMATEXTENSIBLE as *const WAVEFORMATEX,
                None,
            )
        }
        .map_err(|e| err("IAudioClient::Initialize", e))?;
        return Ok(format);
    }
    let mix = unsafe { client.GetMixFormat() }.map_err(|e| err("IAudioClient::GetMixFormat", e))?;
    let m = unsafe { mix.read_unaligned() };
    let is_float = m.wBitsPerSample == 32
        && (m.wFormatTag == WAVE_FORMAT_IEEE_FLOAT_TAG
            || (m.wFormatTag == WAVE_FORMAT_EXTENSIBLE_TAG
                && unsafe {
                    std::ptr::addr_of!((*(mix as *const WAVEFORMATEXTENSIBLE)).SubFormat)
                        .read_unaligned()
                } == IEEE_FLOAT));
    let format = AudioFormat {
        sample_rate: m.nSamplesPerSec,
        channels: m.nChannels,
    };
    let result = if is_float {
        unsafe { client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 0, 0, mix, None) }
    } else {
        // Not float (unusual): ask for float at the same rate and channels, Windows converts.
        let wf = float_format(format);
        flags |= AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
        unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                flags,
                0,
                0,
                &wf as *const WAVEFORMATEXTENSIBLE as *const WAVEFORMATEX,
                None,
            )
        }
    };
    unsafe { CoTaskMemFree(Some(mix as *const _)) };
    result.map_err(|e| err("IAudioClient::Initialize", e))?;
    Ok(format)
}

// ---------------------------------------------------------------------------------------------------------
// capture

pub struct WasapiCapture {
    pub client: Client,
    pub format: AudioFormat,
    pub health: Arc<StreamHealth>,
    /// Application capture: fill the timeline with silence while the application is silent.
    pub silence_fill: bool,
    pub label: String,
}

impl CaptureStream for WasapiCapture {
    fn format(&self) -> AudioFormat {
        self.format
    }
    fn health(&self) -> Arc<StreamHealth> {
        self.health.clone()
    }
    fn start(self: Box<Self>, callback: CaptureCallback) -> Result<Box<dyn Running>, HalError> {
        let event = Arc::new(Event::new()?);
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (stop2, event2) = (stop.clone(), event.clone());
        let name = format!("wasapi-capture {}", self.label);
        let thread = std::thread::Builder::new()
            .name(name)
            .spawn(move || capture_thread(*self, callback, stop2, event2, tx))
            .map_err(|e| HalError::Backend(format!("cannot spawn the capture thread: {e}")))?;
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Box::new(Guard {
                stop,
                event,
                thread: Some(thread),
            })),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
                event.signal();
                Err(HalError::Backend("the capture thread did not start".into()))
            }
        }
    }
}

fn capture_thread(
    s: WasapiCapture,
    mut callback: CaptureCallback,
    stop: Arc<AtomicBool>,
    event: Arc<Event>,
    started: mpsc::Sender<Result<(), HalError>>,
) {
    let _com = ComGuard::new();
    let _boost = Boost::new();
    let health = s.health.clone();
    let setup = (|| -> Result<IAudioCaptureClient, HalError> {
        unsafe { s.client.SetEventHandle(event.handle()) }
            .map_err(|e| HalError::Backend(describe("SetEventHandle", &e)))?;
        let cap: IAudioCaptureClient = unsafe { s.client.GetService() }
            .map_err(|e| HalError::Backend(describe("GetService(IAudioCaptureClient)", &e)))?;
        unsafe { s.client.Start() }
            .map_err(|e| HalError::Backend(describe("IAudioClient::Start", &e)))?;
        Ok(cap)
    })();
    let cap = match setup {
        Ok(c) => {
            let _ = started.send(Ok(()));
            c
        }
        Err(e) => {
            health.fail(e.to_string());
            let _ = started.send(Err(e));
            return;
        }
    };
    let ch = s.format.channels as usize;
    let rate = s.format.sample_rate as f64;
    let mut zeros = vec![0.0f32; 16384 * ch];
    let mut last_data = Instant::now();
    let mut last_event = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let signalled = event.wait(Duration::from_millis(20));
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if signalled {
            last_event = Instant::now();
            loop {
                let frames = match unsafe { cap.GetNextPacketSize() } {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) => {
                        health.fail(describe("GetNextPacketSize", &e));
                        return finish(&s);
                    }
                };
                let (mut data, mut got, mut flags) = (std::ptr::null_mut(), 0u32, 0u32);
                if let Err(e) =
                    unsafe { cap.GetBuffer(&mut data, &mut got, &mut flags, None, None) }
                {
                    health.fail(describe("GetBuffer", &e));
                    return finish(&s);
                }
                let _ = frames;
                let n = got as usize * ch;
                if flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0 {
                    health.add_xrun();
                }
                if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                    if zeros.len() < n {
                        zeros.resize(n, 0.0);
                    }
                    callback(&zeros[..n]);
                } else {
                    // SAFETY: WASAPI guarantees `got` frames of the negotiated float format at `data` until
                    // ReleaseBuffer.
                    callback(unsafe { std::slice::from_raw_parts(data as *const f32, n) });
                }
                health.add_frames(got as u64);
                last_data = Instant::now();
                if let Err(e) = unsafe { cap.ReleaseBuffer(got) } {
                    health.fail(describe("ReleaseBuffer", &e));
                    return finish(&s);
                }
            }
        } else if s.silence_fill {
            // A silent application sends no packets: keep the timeline moving with silence.
            let idle = last_data.elapsed();
            if idle >= Duration::from_millis(10) {
                let frames = ((idle.as_secs_f64() * rate) as usize).min(zeros.len() / ch);
                if frames > 0 {
                    callback(&zeros[..frames * ch]);
                    health.add_frames(frames as u64);
                    last_data = Instant::now();
                }
            }
        } else if last_event.elapsed() > STALL {
            health.fail("no audio from the device for 2 seconds");
            return finish(&s);
        }
    }
    finish(&s);
}

fn finish(s: &WasapiCapture) {
    let _ = unsafe { s.client.Stop() };
}

// ---------------------------------------------------------------------------------------------------------
// render

pub struct WasapiRender {
    pub client: Client,
    pub format: AudioFormat,
    pub health: Arc<StreamHealth>,
    pub label: String,
}

impl RenderStream for WasapiRender {
    fn format(&self) -> AudioFormat {
        self.format
    }
    fn health(&self) -> Arc<StreamHealth> {
        self.health.clone()
    }
    fn start(self: Box<Self>, callback: RenderCallback) -> Result<Box<dyn Running>, HalError> {
        let event = Arc::new(Event::new()?);
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (stop2, event2) = (stop.clone(), event.clone());
        let name = format!("wasapi-render {}", self.label);
        let thread = std::thread::Builder::new()
            .name(name)
            .spawn(move || render_thread(*self, callback, stop2, event2, tx))
            .map_err(|e| HalError::Backend(format!("cannot spawn the render thread: {e}")))?;
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Box::new(Guard {
                stop,
                event,
                thread: Some(thread),
            })),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
                event.signal();
                Err(HalError::Backend("the render thread did not start".into()))
            }
        }
    }
}

fn render_thread(
    s: WasapiRender,
    mut callback: RenderCallback,
    stop: Arc<AtomicBool>,
    event: Arc<Event>,
    started: mpsc::Sender<Result<(), HalError>>,
) {
    let _com = ComGuard::new();
    let _boost = Boost::new();
    let health = s.health.clone();
    let ch = s.format.channels as usize;
    let setup = (|| -> Result<(IAudioRenderClient, u32), HalError> {
        let render: IAudioRenderClient = unsafe { s.client.GetService() }
            .map_err(|e| HalError::Backend(describe("GetService(IAudioRenderClient)", &e)))?;
        let buffer = unsafe { s.client.GetBufferSize() }
            .map_err(|e| HalError::Backend(describe("GetBufferSize", &e)))?;
        unsafe { s.client.SetEventHandle(event.handle()) }
            .map_err(|e| HalError::Backend(describe("SetEventHandle", &e)))?;
        // Start with a buffer of silence.
        unsafe { render.GetBuffer(buffer) }
            .map_err(|e| HalError::Backend(describe("GetBuffer(prefill)", &e)))?;
        unsafe { render.ReleaseBuffer(buffer, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) }
            .map_err(|e| HalError::Backend(describe("ReleaseBuffer(prefill)", &e)))?;
        unsafe { s.client.Start() }
            .map_err(|e| HalError::Backend(describe("IAudioClient::Start", &e)))?;
        Ok((render, buffer))
    })();
    let (render, buffer) = match setup {
        Ok(r) => {
            let _ = started.send(Ok(()));
            r
        }
        Err(e) => {
            health.fail(e.to_string());
            let _ = started.send(Err(e));
            return;
        }
    };
    let mut last_event = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        if !event.wait(Duration::from_millis(100)) {
            if last_event.elapsed() > STALL {
                health.fail("the device stopped asking for audio for 2 seconds");
                break;
            }
            continue;
        }
        last_event = Instant::now();
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let padding = match unsafe { s.client.GetCurrentPadding() } {
            Ok(p) => p,
            Err(e) => {
                health.fail(describe("GetCurrentPadding", &e));
                break;
            }
        };
        let available = buffer.saturating_sub(padding);
        if available == 0 {
            continue;
        }
        match unsafe { render.GetBuffer(available) } {
            Ok(data) => {
                // SAFETY: WASAPI gives `available` frames of the negotiated float format to fill.
                let out = unsafe {
                    std::slice::from_raw_parts_mut(data as *mut f32, available as usize * ch)
                };
                callback(out);
                health.add_frames(available as u64);
                if let Err(e) = unsafe { render.ReleaseBuffer(available, 0) } {
                    health.fail(describe("ReleaseBuffer", &e));
                    break;
                }
            }
            Err(e) => {
                health.fail(describe("GetBuffer", &e));
                break;
            }
        }
    }
    let _ = unsafe { s.client.Stop() };
}
