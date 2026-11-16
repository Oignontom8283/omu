//! VBAN audio: the packet format, a receiver (a capture stream) and a sender (a render stream).
//!
//! A packet is a 28-byte header then the samples:
//!
//! ```text
//! "VBAN"  rate+protocol(1)  samples-1(1)  channels-1(1)  format+codec(1)  stream name(16)  counter(4, little-endian)
//! ```
//! The low 5 bits of the second byte are the sample rate (an index in [`RATES`]) and the high 3 the sub-protocol (0 =
//! audio). The low 3 bits of the fourth byte are the sample format and its high nibble the codec (0 = plain PCM). The
//! data may hold up to [`MAX_DATA`] bytes.

use omu_dsp::resample::StreamResampler;
use omu_hal::{
    CaptureCallback, CaptureStream, HalError, RenderCallback, RenderStream, Running, StreamHealth,
};
use omu_types::AudioFormat;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const HEADER: usize = 28;
/// The most sample data a packet may carry.
pub const MAX_DATA: usize = 1436;
/// The sample rates a VBAN header can name, by index.
pub const RATES: [u32; 21] = [
    6000, 12000, 24000, 48000, 96000, 192000, 384000, 8000, 16000, 32000, 64000, 128000, 256000,
    512000, 11025, 22050, 44100, 88200, 176400, 352800, 705600,
];
/// Frames per packet when sending: 256 stereo 16-bit frames are 1024 bytes, 5.3 ms of sound.
const SEND_FRAMES: usize = 256;
/// The longest stream name.
pub const NAME_LEN: usize = omu_types::MAX_STREAM_NAME;

/// How the samples of a packet are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    U8,
    I16,
    I24,
    I32,
    F32,
}

impl Format {
    fn from_bits(bits: u8) -> Option<Format> {
        Some(match bits {
            0 => Format::U8,
            1 => Format::I16,
            2 => Format::I24,
            3 => Format::I32,
            4 => Format::F32,
            _ => return None,
        })
    }

    fn bytes(self) -> usize {
        match self {
            Format::U8 => 1,
            Format::I16 => 2,
            Format::I24 => 3,
            Format::I32 | Format::F32 => 4,
        }
    }
}

/// A packet of audio, read.
#[derive(Debug, PartialEq)]
pub struct Packet<'a> {
    pub rate: u32,
    pub channels: usize,
    pub format: Format,
    pub stream: String,
    pub counter: u32,
    /// How many frames (a sample for each channel) the header says.
    pub frames: usize,
    pub data: &'a [u8],
}

/// Reads a packet; `None` for anything that is not plain-PCM VBAN audio this program understands (other sub-protocols,
/// other codecs, a damaged or truncated packet).
pub fn parse(buf: &[u8]) -> Option<Packet<'_>> {
    if buf.len() < HEADER || &buf[0..4] != b"VBAN" {
        return None;
    }
    if buf[4] >> 5 != 0 {
        return None; // text, serial and the other sub-protocols
    }
    let rate = *RATES.get((buf[4] & 0x1f) as usize)?;
    let frames = buf[5] as usize + 1;
    let channels = buf[6] as usize + 1;
    if buf[7] >> 4 != 0 {
        return None; // a codec: only PCM is read
    }
    let format = Format::from_bits(buf[7] & 0x07)?;
    let stream = String::from_utf8_lossy(&buf[8..24])
        .trim_end_matches('\0')
        .to_owned();
    let counter = u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]);
    let data = &buf[HEADER..];
    // the data must be what the header says (a packet cut short is dropped, extra bytes are ignored)
    let wanted = frames * channels * format.bytes();
    if data.len() < wanted || wanted == 0 {
        return None;
    }
    Some(Packet {
        rate,
        channels,
        format,
        stream,
        counter,
        frames,
        data: &data[..wanted],
    })
}

/// Writes the header of an audio packet of 16-bit samples at `rate_index`.
pub fn header(
    stream: &str,
    counter: u32,
    frames: usize,
    channels: usize,
    rate_index: u8,
) -> [u8; HEADER] {
    let mut h = [0u8; HEADER];
    h[0..4].copy_from_slice(b"VBAN");
    h[4] = rate_index & 0x1f;
    h[5] = (frames - 1) as u8;
    h[6] = (channels - 1) as u8;
    h[7] = 1; // 16-bit integers, plain PCM
    let name = stream.as_bytes();
    let n = name.len().min(NAME_LEN);
    h[8..8 + n].copy_from_slice(&name[..n]);
    h[24..28].copy_from_slice(&counter.to_le_bytes());
    h
}

/// The samples of a packet as floats in -1..1, interleaved as they came.
pub fn samples(p: &Packet<'_>, out: &mut Vec<f32>) {
    out.clear();
    match p.format {
        Format::U8 => out.extend(p.data.iter().map(|b| (*b as f32 - 128.0) / 128.0)),
        Format::I16 => out.extend(
            p.data
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| i16::from_le_bytes(*b) as f32 / 32768.0),
        ),
        Format::I24 => out.extend(p.data.as_chunks::<3>().0.iter().map(|b| {
            // sign-extend the 24-bit value through the top of an i32
            (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0
        })),
        Format::I32 => out.extend(
            p.data
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| i32::from_le_bytes(*b) as f32 / 2_147_483_648.0),
        ),
        Format::F32 => out.extend(
            p.data
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b)),
        ),
    }
}

/// Whether a stream name filter lets a packet through (an empty filter lets everything through).
fn name_matches(wanted: &str, got: &str) -> bool {
    wanted.is_empty() || wanted == got
}

/// Sound received from the network, as a capture stream in the engine's format (48 kHz stereo): the receiver converts
/// whatever arrives (any rate, mono or more channels, any of the PCM formats).
pub struct VbanCapture {
    socket: UdpSocket,
    stream: String,
    /// Only this computer's packets are accepted, when set.
    from: Option<IpAddr>,
    health: Arc<StreamHealth>,
}

impl VbanCapture {
    /// Opens the port (all addresses of this computer, so that other computers can reach it). `stream` empty accepts any
    /// stream name; `from` empty accepts any sender.
    pub fn open(port: u16, stream: &str, from: &str) -> Result<VbanCapture, HalError> {
        let from = if from.trim().is_empty() {
            None
        } else {
            Some(from.trim().parse::<IpAddr>().map_err(|_| {
                HalError::Backend(format!("{from:?} is not an address (like 192.168.1.20)"))
            })?)
        };
        // Listening on every address of the computer is what lets other computers reach it, and it is what makes the
        // firewall ask the user. When the one computer accepted is this one (a loopback address) there is no need for
        // it: listen on the loopback only, and nothing outside can reach the port nor trigger a question.
        let bind = match from {
            Some(a) if a.is_loopback() => "127.0.0.1",
            _ => "0.0.0.0",
        };
        let socket = UdpSocket::bind((bind, port))
            .map_err(|e| HalError::Busy(format!("cannot listen on UDP port {port}: {e}")))?;
        socket
            .set_read_timeout(Some(Duration::from_millis(100)))
            .map_err(|e| HalError::Backend(e.to_string()))?;
        Ok(VbanCapture {
            socket,
            stream: stream.to_owned(),
            from,
            health: StreamHealth::new(),
        })
    }

    /// The port that was opened (useful when 0 was asked for).
    pub fn port(&self) -> u16 {
        self.socket.local_addr().map_or(0, |a| a.port())
    }
}

struct Guard {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Running for Guard {}

impl Drop for Guard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl CaptureStream for VbanCapture {
    fn format(&self) -> AudioFormat {
        AudioFormat::ENGINE
    }

    fn health(&self) -> Arc<StreamHealth> {
        self.health.clone()
    }

    fn start(self: Box<Self>, mut callback: CaptureCallback) -> Result<Box<dyn Running>, HalError> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let VbanCapture {
            socket,
            stream,
            from,
            health,
        } = *self;
        let thread = std::thread::Builder::new()
            .name("vban-receive".into())
            .spawn(move || {
                let mut buf = vec![0u8; 2048];
                let mut decoded: Vec<f32> = Vec::new();
                let mut pending: Vec<f32> = Vec::new();
                let mut stereo: Vec<f32> = Vec::new();
                let mut out = vec![0.0f32; 4096 * 2];
                let mut resampler = StreamResampler::new(2);
                let mut last_rate = 0u32;
                let mut last_counter: Option<u32> = None;
                while !flag.load(Ordering::Relaxed) {
                    let (n, sender) = match socket.recv_from(&mut buf) {
                        Ok(r) => r,
                        Err(e)
                            if matches!(
                                e.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            continue
                        }
                        Err(e) => {
                            health.fail(format!("the network socket failed: {e}"));
                            return;
                        }
                    };
                    if from.is_some_and(|a| a != sender.ip()) {
                        continue;
                    }
                    let Some(p) = parse(&buf[..n]) else { continue };
                    if !name_matches(&stream, &p.stream) {
                        continue;
                    }
                    // a packet that arrives late (a lower counter, not a sender that started again) is dropped
                    if let Some(last) = last_counter {
                        let behind = last.wrapping_sub(p.counter);
                        if behind > 0 && behind < 1000 {
                            continue;
                        }
                    }
                    last_counter = Some(p.counter);
                    samples(&p, &mut decoded);
                    // the first two channels, as stereo (one channel is both sides)
                    stereo.clear();
                    for frame in decoded.chunks_exact(p.channels) {
                        let (l, r) = (frame[0], if p.channels > 1 { frame[1] } else { frame[0] });
                        stereo.push(l);
                        stereo.push(r);
                    }
                    if p.rate != last_rate {
                        // another rate: the old interpolation state means nothing now
                        resampler.reset();
                        pending.clear();
                        last_rate = p.rate;
                    }
                    if p.rate == 48_000 {
                        callback(&stereo);
                        health.add_frames((stereo.len() / 2) as u64);
                        continue;
                    }
                    pending.extend_from_slice(&stereo);
                    let step = p.rate as f64 / 48_000.0;
                    loop {
                        let (used, made) = resampler.process(&pending, &mut out, step);
                        if made > 0 {
                            callback(&out[..made * 2]);
                            health.add_frames(made as u64);
                        }
                        pending.drain(..used * 2);
                        if made < out.len() / 2 {
                            break;
                        }
                    }
                }
            })
            .map_err(|e| HalError::Backend(format!("cannot start the receiver: {e}")))?;
        Ok(Box::new(Guard {
            stop,
            thread: Some(thread),
        }))
    }
}

/// Sound sent to the network, as a render stream in the engine's format.
pub struct VbanRender {
    socket: UdpSocket,
    target: SocketAddr,
    stream: String,
    health: Arc<StreamHealth>,
}

impl VbanRender {
    /// `host` is a name or an address; the sound goes to its UDP `port`.
    pub fn open(host: &str, port: u16, stream: &str) -> Result<VbanRender, HalError> {
        use std::net::ToSocketAddrs;
        let host = host.trim();
        if host.is_empty() {
            return Err(HalError::Backend("no computer to send the sound to".into()));
        }
        let target = (host, port)
            .to_socket_addrs()
            .map_err(|e| HalError::NotFound(format!("cannot find {host}: {e}")))?
            .next()
            .ok_or_else(|| HalError::NotFound(format!("cannot find {host}")))?;
        // sound for this very computer is sent from the loopback, which never leaves it (no firewall question either)
        let socket = UdpSocket::bind(match (target.ip().is_loopback(), target.is_ipv4()) {
            (true, true) => "127.0.0.1:0",
            (true, false) => "[::1]:0",
            (false, true) => "0.0.0.0:0",
            (false, false) => "[::]:0",
        })
        .map_err(|e| HalError::Backend(format!("cannot open a network socket: {e}")))?;
        Ok(VbanRender {
            socket,
            target,
            stream: stream.chars().take(NAME_LEN).collect(),
            health: StreamHealth::new(),
        })
    }
}

impl RenderStream for VbanRender {
    fn format(&self) -> AudioFormat {
        AudioFormat::ENGINE
    }

    fn health(&self) -> Arc<StreamHealth> {
        self.health.clone()
    }

    fn start(self: Box<Self>, mut callback: RenderCallback) -> Result<Box<dyn Running>, HalError> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let VbanRender {
            socket,
            target,
            stream,
            health,
        } = *self;
        let thread = std::thread::Builder::new()
            .name("vban-send".into())
            .spawn(move || {
                let mut audio = vec![0.0f32; SEND_FRAMES * 2];
                let mut packet = vec![0u8; HEADER + SEND_FRAMES * 4];
                let mut counter = 0u32;
                let started = Instant::now();
                let mut sent_frames = 0u64;
                while !flag.load(Ordering::Relaxed) {
                    // how many frames the clock says are due: a sleep that runs long only sends a few packets together
                    let due = (started.elapsed().as_secs_f64() * 48_000.0) as u64;
                    if due < sent_frames + SEND_FRAMES as u64 {
                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    callback(&mut audio);
                    packet[..HEADER].copy_from_slice(&header(&stream, counter, SEND_FRAMES, 2, 3));
                    for (i, s) in audio.iter().enumerate() {
                        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
                        packet[HEADER + 2 * i..HEADER + 2 * i + 2]
                            .copy_from_slice(&v.to_le_bytes());
                    }
                    if let Err(e) = socket.send_to(&packet[..HEADER + SEND_FRAMES * 4], target) {
                        // a computer that is not there yet is not a failure: UDP says nothing, and an unreachable
                        // address is reported by the system as an error on the next send; keep trying
                        if !matches!(
                            e.kind(),
                            std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::WouldBlock
                        ) {
                            health.fail(format!("cannot send sound to {target}: {e}"));
                            return;
                        }
                    }
                    counter = counter.wrapping_add(1);
                    sent_frames += SEND_FRAMES as u64;
                    health.add_frames(SEND_FRAMES as u64);
                }
            })
            .map_err(|e| HalError::Backend(format!("cannot start the sender: {e}")))?;
        Ok(Box::new(Guard {
            stop,
            thread: Some(thread),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn packet(
        stream: &str,
        counter: u32,
        rate_index: u8,
        channels: usize,
        format_bits: u8,
        data: &[u8],
    ) -> Vec<u8> {
        let bytes = [1usize, 2, 3, 4, 4][format_bits.min(4) as usize];
        let frames = data.len() / (channels * bytes);
        let mut p = header(stream, counter, frames, channels, rate_index).to_vec();
        p[7] = format_bits;
        p.extend_from_slice(data);
        p
    }

    #[test]
    fn a_packet_is_read_back_as_it_was_written_and_what_is_not_audio_is_ignored() {
        let data: Vec<u8> = (0..16u8).collect();
        let bytes = packet("Stream1", 7, 3, 2, 1, &data);
        let p = parse(&bytes).expect("a packet");
        assert_eq!(
            (p.rate, p.channels, p.format, p.counter, p.frames),
            (48_000, 2, Format::I16, 7, 4)
        );
        assert_eq!(p.stream, "Stream1");
        assert_eq!(p.data, &data[..]);
        // not VBAN, too short, text sub-protocol, a codec, an unknown format, data shorter than the header says
        assert!(parse(b"not a packet at all, not at all, no").is_none());
        assert!(parse(&bytes[..20]).is_none());
        let mut text = bytes.clone();
        text[4] |= 0x40;
        assert!(parse(&text).is_none());
        let mut codec = bytes.clone();
        codec[7] |= 0x10;
        assert!(parse(&codec).is_none());
        let mut odd = bytes.clone();
        odd[7] = 7;
        assert!(parse(&odd).is_none());
        assert!(parse(&bytes[..bytes.len() - 2]).is_none(), "cut short");
    }

    #[test]
    fn every_pcm_format_reads_to_the_same_level() {
        let half = |format: Format, data: Vec<u8>| {
            let p = Packet {
                rate: 48_000,
                channels: 1,
                format,
                stream: String::new(),
                counter: 0,
                frames: 1,
                data: &data,
            };
            let mut out = vec![];
            samples(&p, &mut out);
            out[0]
        };
        assert!((half(Format::U8, vec![192]) - 0.5).abs() < 0.01);
        assert!((half(Format::I16, 16384i16.to_le_bytes().to_vec()) - 0.5).abs() < 0.001);
        assert!((half(Format::I24, vec![0, 0, 0x40]) - 0.5).abs() < 0.001);
        assert!((half(Format::I32, 0x4000_0000i32.to_le_bytes().to_vec()) - 0.5).abs() < 0.001);
        assert!((half(Format::F32, 0.5f32.to_le_bytes().to_vec()) - 0.5).abs() < 0.001);
        // negative values keep their sign
        assert!((half(Format::I24, vec![0, 0, 0xC0]) + 0.5).abs() < 0.001);
    }

    #[test]
    fn the_name_filter_lets_through_what_it_names_or_everything_when_empty() {
        assert!(name_matches("", "anything"));
        assert!(name_matches("Mic", "Mic"));
        assert!(!name_matches("Mic", "Stream1"));
    }

    /// Collects what a receiver hands out.
    fn collect(capture: VbanCapture) -> (Box<dyn Running>, Arc<Mutex<Vec<f32>>>, u16) {
        let port = capture.port();
        let heard = Arc::new(Mutex::new(Vec::new()));
        let h2 = heard.clone();
        let running = Box::new(capture)
            .start(Box::new(move |frames| {
                h2.lock().unwrap().extend_from_slice(frames)
            }))
            .unwrap();
        (running, heard, port)
    }

    fn send(port: u16, bytes: &[u8]) {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.send_to(bytes, ("127.0.0.1", port)).unwrap();
    }

    #[test]
    fn what_the_sender_sends_the_receiver_hears_at_the_same_pitch_and_level() {
        let capture = VbanCapture::open(0, "Voice", "127.0.0.1").unwrap();
        let (running, heard, port) = collect(capture);
        // a stereo tone of 1000 Hz on the left and 500 Hz on the right, played for a while
        let mut phase = 0u64;
        let render = VbanRender::open("127.0.0.1", port, "Voice").unwrap();
        let sender = Box::new(render)
            .start(Box::new(move |out| {
                for f in out.as_chunks_mut::<2>().0.iter_mut() {
                    let t = phase as f32 / 48_000.0;
                    f[0] = (t * 1000.0 * std::f32::consts::TAU).sin() * 0.5;
                    f[1] = (t * 500.0 * std::f32::consts::TAU).sin() * 0.5;
                    phase += 1;
                }
            }))
            .unwrap();
        std::thread::sleep(Duration::from_millis(700));
        drop(sender);
        std::thread::sleep(Duration::from_millis(100));
        drop(running);
        let heard = heard.lock().unwrap().clone();
        assert!(
            heard.len() > 2 * 48_000 / 4,
            "about half a second arrived: {} frames",
            heard.len() / 2
        );
        let left: Vec<f32> = heard.iter().step_by(2).copied().collect();
        let right: Vec<f32> = heard.iter().skip(1).step_by(2).copied().collect();
        let rising = |x: &[f32]| {
            x.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count() as f32 * 48_000.0
                / x.len() as f32
        };
        assert!(
            (rising(&left) - 1000.0).abs() < 30.0,
            "left {} Hz",
            rising(&left)
        );
        assert!(
            (rising(&right) - 500.0).abs() < 30.0,
            "right {} Hz",
            rising(&right)
        );
        let peak = left.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((peak - 0.5).abs() < 0.02, "level {peak}");
    }

    #[test]
    fn other_rates_and_mono_are_converted_and_other_streams_and_senders_are_ignored() {
        // only the stream "Wanted", only from 127.0.0.1
        let capture = VbanCapture::open(0, "Wanted", "127.0.0.1").unwrap();
        let (running, heard, port) = collect(capture);
        // 44.1 kHz mono 16-bit: a 441 Hz tone, 100 packets of 100 frames (0.23 s)
        let tone: Vec<i16> = (0..10_000)
            .map(|i| ((i as f32 / 44_100.0 * 441.0 * std::f32::consts::TAU).sin() * 16000.0) as i16)
            .collect();
        for (n, chunk) in tone.chunks(100).enumerate() {
            let data: Vec<u8> = chunk.iter().flat_map(|s| s.to_le_bytes()).collect();
            send(port, &packet("Wanted", n as u32, 16, 1, 1, &data));
            // the same sound under another name must not be heard
            send(port, &packet("Other", n as u32, 16, 1, 1, &data));
            std::thread::sleep(Duration::from_millis(2));
        }
        std::thread::sleep(Duration::from_millis(200));
        drop(running);
        let heard = heard.lock().unwrap().clone();
        let frames = heard.len() / 2;
        // 10 000 frames at 44.1 kHz are 10 884 at 48 kHz (once, not twice: the other stream was ignored)
        assert!((frames as i64 - 10_884).abs() < 400, "{frames} frames");
        // mono: both sides are the same
        assert!(heard.as_chunks::<2>().0.iter().all(|f| f[0] == f[1]));
        let left: Vec<f32> = heard.iter().step_by(2).copied().collect();
        let rising = left
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count() as f32
            * 48_000.0
            / left.len() as f32;
        assert!(
            (rising - 441.0).abs() < 25.0,
            "{rising} Hz, the pitch is kept"
        );
    }

    #[test]
    fn a_sender_that_is_not_the_wanted_one_is_not_heard() {
        // listens on the loopback only, and accepts only ::1 (a sender this computer does not use): nothing gets in
        let capture = VbanCapture::open(0, "", "127.0.0.1").unwrap();
        let (running, heard, port) = collect(capture);
        let data = vec![0u8; 400];
        for n in 0..20 {
            send(port, &packet("x", n, 3, 2, 1, &data));
        }
        std::thread::sleep(Duration::from_millis(250));
        let from_here = heard.lock().unwrap().len();
        drop(running);
        assert!(from_here > 0, "the accepted sender is heard");
        // another sender than the accepted one: the filter drops it (the accepted address is not this computer's)
        let (other, heard2, port2) = collect(VbanCapture::open(0, "", "127.0.0.2").unwrap());
        for n in 0..20 {
            send(port2, &packet("x", n, 3, 2, 1, &data));
        }
        std::thread::sleep(Duration::from_millis(250));
        drop(other);
        assert!(
            heard2.lock().unwrap().is_empty(),
            "127.0.0.1 is not the accepted 127.0.0.2"
        );
        assert!(VbanCapture::open(0, "", "not an address").is_err());
    }

    #[test]
    fn a_port_that_is_taken_and_a_host_that_is_not_found_are_said_in_words() {
        let first = VbanCapture::open(0, "", "127.0.0.1").unwrap();
        let taken = VbanCapture::open(first.port(), "", "127.0.0.1");
        let e = taken.err().expect("the port is in use").to_string();
        assert!(e.contains("cannot listen on UDP port"), "{e}");
        assert!(VbanRender::open("", 6980, "x").is_err());
    }
}
