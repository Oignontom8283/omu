//! Live streams through FFmpeg: RTMP, RTSP, SRT, RTP, UDP, HTTP (Icecast, HLS...).
//!
//! Those protocols each take a library to speak; FFmpeg already speaks them all (and the module that downloads it is already
//! there), so a stream block starts FFmpeg and talks to it through a pipe: raw sound goes out of it for an input, into it for an
//! output. FFmpeg reconnects nothing by itself: when it stops, the stream is reported as failed and the engine opens it again
//! after a delay, as it does for a device that went away.
//!
//! Only the protocols that carry sound over a network are allowed, and FFmpeg is told so (`-protocol_whitelist`): a stream address
//! can never make it read a file of this computer.

use omu_hal::{
    CaptureCallback, CaptureStream, HalError, RenderCallback, RenderStream, Running, StreamHealth,
};
use omu_types::AudioFormat;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Frames per pipe write: 10 ms at 48 kHz.
const FRAMES: usize = 480;
/// What FFmpeg may open for an input (the protocols a stream address can use, and the ones they use inside).
const WHITELIST: &str = "rtmp,rtmps,rtsp,rtsps,srt,rtp,udp,tcp,tls,http,https,crypto,httpproxy";

fn command(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

/// The arguments that make FFmpeg read the stream at `url` and write raw 48 kHz stereo floats on its standard output.
pub fn input_args(url: &str, tls: &[String]) -> Vec<String> {
    let mut args: Vec<String> = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-protocol_whitelist",
        WHITELIST,
    ]
    .map(String::from)
    .into();
    args.extend(tls.iter().cloned());
    let scheme = omu_types::stream_scheme(url);
    if matches!(scheme.as_deref(), Some("rtsp" | "rtsps")) {
        // over TCP: nothing is lost on the way, and it passes through firewalls
        args.extend(["-rtsp_transport", "tcp"].map(String::from));
    }
    if matches!(
        scheme.as_deref(),
        Some("rtmp" | "rtmps" | "rtsp" | "rtsps" | "srt" | "udp" | "rtp")
    ) {
        // a live stream: no waiting to fill a buffer before the first sound
        args.extend(["-fflags", "nobuffer", "-flags", "low_delay"].map(String::from));
    }
    args.extend(
        [
            "-i", url, "-vn", "-sn", "-dn", "-f", "f32le", "-ar", "48000", "-ac", "2", "pipe:1",
        ]
        .map(String::from),
    );
    args
}

/// The arguments that make FFmpeg take raw 48 kHz stereo floats on its standard input and send them to `url`, in the format
/// that kind of address expects.
pub fn output_args(url: &str, tls: &[String]) -> Vec<String> {
    let mut args: Vec<String> = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-f",
        "f32le",
        "-ar",
        "48000",
        "-ac",
        "2",
        "-i",
        "pipe:0",
    ]
    .map(String::from)
    .into();
    args.extend(tls.iter().cloned());
    let format: &[&str] = match omu_types::stream_scheme(url).as_deref() {
        Some("rtmp" | "rtmps") => &["-c:a", "aac", "-b:a", "160k", "-f", "flv"],
        Some("rtsp") => &[
            "-c:a",
            "aac",
            "-b:a",
            "160k",
            "-f",
            "rtsp",
            "-rtsp_transport",
            "tcp",
        ],
        Some("srt" | "udp") => &["-c:a", "aac", "-b:a", "160k", "-f", "mpegts"],
        Some("rtp") => &["-c:a", "libopus", "-b:a", "96k", "-f", "rtp"],
        Some("icecast") => &[
            "-c:a",
            "libmp3lame",
            "-b:a",
            "128k",
            "-f",
            "mp3",
            "-content_type",
            "audio/mpeg",
        ],
        _ => &["-c:a", "aac", "-b:a", "160k", "-f", "mpegts"],
    };
    args.extend(format.iter().map(|s| (*s).to_owned()));
    args.push(url.to_owned());
    args
}

/// The last line FFmpeg said on its error output: what to tell the user when it stops.
type LastError = Arc<Mutex<String>>;

fn follow_errors(child: &mut Child) -> (LastError, Option<JoinHandle<()>>) {
    let last: LastError = Arc::default();
    let handle = child.stderr.take().map(|mut err| {
        let last = last.clone();
        std::thread::spawn(move || {
            let mut text = String::new();
            let mut buf = [0u8; 1024];
            while let Ok(n) = err.read(&mut buf) {
                if n == 0 {
                    break;
                }
                text.push_str(&String::from_utf8_lossy(&buf[..n]));
                // only the end matters, and a program that says a lot must not fill the memory
                if text.len() > 4096 {
                    text.drain(..text.len() - 2048);
                }
                if let Some(line) = text.lines().rev().find(|l| !l.trim().is_empty()) {
                    *last.lock().unwrap_or_else(|e| e.into_inner()) = line.trim().to_owned();
                }
            }
        })
    });
    (last, handle)
}

fn reason(last: &LastError, what: &str) -> String {
    let said = last.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if said.is_empty() {
        what.to_owned()
    } else {
        said
    }
}

fn spawn(ffmpeg: &Path, args: &[String], stdin: bool, stdout: bool) -> Result<Child, HalError> {
    command(ffmpeg)
        .args(args)
        .stdin(if stdin { Stdio::piped() } else { Stdio::null() })
        .stdout(if stdout {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| HalError::Backend(format!("cannot start FFmpeg: {e}")))
}

struct Guard {
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Child>>,
    threads: Vec<JoinHandle<()>>,
}

impl Running for Guard {}

impl Drop for Guard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        {
            let mut child = self.child.lock().unwrap_or_else(|e| e.into_inner());
            let _ = child.kill();
            let _ = child.wait();
        }
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

/// A stream read through FFmpeg, as a capture stream.
pub struct StreamCapture {
    child: Child,
    health: Arc<StreamHealth>,
}

impl StreamCapture {
    /// Starts FFmpeg on `url` (the check of the address is the caller's: see [`omu_types::stream_problem`]).
    pub fn open(ffmpeg: &Path, url: &str, tls: &[String]) -> Result<StreamCapture, HalError> {
        Ok(StreamCapture {
            child: spawn(ffmpeg, &input_args(url, tls), false, true)?,
            health: StreamHealth::new(),
        })
    }
}

impl CaptureStream for StreamCapture {
    fn format(&self) -> AudioFormat {
        AudioFormat::ENGINE
    }

    fn health(&self) -> Arc<StreamHealth> {
        self.health.clone()
    }

    fn start(self: Box<Self>, mut callback: CaptureCallback) -> Result<Box<dyn Running>, HalError> {
        let StreamCapture { mut child, health } = *self;
        let mut out = child
            .stdout
            .take()
            .ok_or_else(|| HalError::Backend("FFmpeg has no output".into()))?;
        let (last, errors) = follow_errors(&mut child);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let child = Arc::new(Mutex::new(child));
        let reader = std::thread::Builder::new()
            .name("stream-in".into())
            .spawn(move || {
                let mut bytes = vec![0u8; FRAMES * 2 * 4];
                let mut samples = vec![0f32; FRAMES * 2];
                while !flag.load(Ordering::Relaxed) {
                    if out.read_exact(&mut bytes).is_err() {
                        if !flag.load(Ordering::Relaxed) {
                            health.fail(reason(&last, "the stream ended"));
                        }
                        return;
                    }
                    let (floats, _) = bytes.as_chunks::<4>();
                    for (s, b) in samples.iter_mut().zip(floats) {
                        *s = f32::from_le_bytes(*b);
                    }
                    health.add_frames(FRAMES as u64);
                    callback(&samples);
                }
            })
            .map_err(|e| HalError::Backend(format!("cannot start the stream input: {e}")))?;
        let mut threads = vec![reader];
        threads.extend(errors);
        Ok(Box::new(Guard {
            stop,
            child,
            threads,
        }))
    }
}

/// The sound of the graph sent as a live stream through FFmpeg, as a render stream.
pub struct StreamRender {
    child: Child,
    health: Arc<StreamHealth>,
}

impl StreamRender {
    pub fn open(ffmpeg: &Path, url: &str, tls: &[String]) -> Result<StreamRender, HalError> {
        Ok(StreamRender {
            child: spawn(ffmpeg, &output_args(url, tls), true, false)?,
            health: StreamHealth::new(),
        })
    }
}

impl RenderStream for StreamRender {
    fn format(&self) -> AudioFormat {
        AudioFormat::ENGINE
    }

    fn health(&self) -> Arc<StreamHealth> {
        self.health.clone()
    }

    fn start(self: Box<Self>, mut callback: RenderCallback) -> Result<Box<dyn Running>, HalError> {
        let StreamRender { mut child, health } = *self;
        let mut input: ChildStdin = child
            .stdin
            .take()
            .ok_or_else(|| HalError::Backend("FFmpeg has no input".into()))?;
        let (last, errors) = follow_errors(&mut child);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let child = Arc::new(Mutex::new(child));
        let watched = child.clone();
        let writer = std::thread::Builder::new()
            .name("stream-out".into())
            .spawn(move || {
                let mut frame = vec![0f32; FRAMES * 2];
                let mut bytes = Vec::with_capacity(FRAMES * 2 * 4);
                let mut next = Instant::now();
                while !flag.load(Ordering::Relaxed) {
                    callback(&mut frame);
                    bytes.clear();
                    bytes.extend(frame.iter().flat_map(|s| s.to_le_bytes()));
                    if input.write_all(&bytes).is_err() {
                        if !flag.load(Ordering::Relaxed) {
                            health.fail(reason(&last, "the stream was closed"));
                        }
                        return;
                    }
                    health.add_frames(FRAMES as u64);
                    // FFmpeg stopped (the server refused the connection...) without our pipe noticing yet
                    if let Ok(Some(status)) =
                        watched.lock().unwrap_or_else(|e| e.into_inner()).try_wait()
                    {
                        if !flag.load(Ordering::Relaxed) {
                            health.fail(reason(&last, &format!("FFmpeg stopped ({status})")));
                        }
                        return;
                    }
                    next += Duration::from_millis(10);
                    let now = Instant::now();
                    if next > now {
                        std::thread::sleep(next - now);
                    } else if now - next > Duration::from_millis(200) {
                        next = now;
                    }
                }
            })
            .map_err(|e| HalError::Backend(format!("cannot start the stream output: {e}")))?;
        let mut threads = vec![writer];
        threads.extend(errors);
        Ok(Box::new(Guard {
            stop,
            child,
            threads,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has(args: &[String], pair: &[&str]) -> bool {
        args.windows(pair.len())
            .any(|w| w.iter().map(String::as_str).eq(pair.iter().copied()))
    }

    #[test]
    fn an_input_reads_the_stream_as_raw_sound_and_cannot_open_a_file() {
        let args = input_args("rtmp://example.com/live/key", &[]);
        assert!(has(&args, &["-i", "rtmp://example.com/live/key"]));
        assert!(has(
            &args,
            &["-f", "f32le", "-ar", "48000", "-ac", "2", "pipe:1"]
        ));
        let whitelist = args
            .iter()
            .position(|a| a == "-protocol_whitelist")
            .map(|i| args[i + 1].clone())
            .unwrap();
        assert!(
            !whitelist
                .split(',')
                .any(|p| p == "file" || p == "pipe" || p == "concat"),
            "{whitelist}"
        );
        assert!(has(
            &input_args("rtsp://cam/stream", &[]),
            &["-rtsp_transport", "tcp"]
        ));
        assert!(has(
            &input_args("https://radio/live.m3u8", &[]),
            &["-i", "https://radio/live.m3u8"]
        ));
    }

    #[test]
    fn an_output_gets_the_format_its_address_expects() {
        let of = |url: &str| output_args(url, &[]);
        assert!(
            has(&of("rtmp://live.example/app/key"), &["-c:a", "aac"])
                && has(&of("rtmp://live.example/app/key"), &["-f", "flv"])
        );
        assert!(has(&of("srt://host:9000"), &["-f", "mpegts"]));
        assert!(has(&of("udp://127.0.0.1:5000"), &["-f", "mpegts"]));
        assert!(
            has(&of("rtp://host:5004"), &["-c:a", "libopus"])
                && has(&of("rtp://host:5004"), &["-f", "rtp"])
        );
        assert!(
            has(
                &of("icecast://source:pw@host:8000/mount"),
                &["-c:a", "libmp3lame"]
            ) && has(&of("icecast://source:pw@host:8000/mount"), &["-f", "mp3"])
        );
        assert_eq!(
            of("rtmp://live.example/app/key").last().unwrap(),
            "rtmp://live.example/app/key"
        );
        // the settings of the TLS the user chose come before the address
        let tls = vec!["-tls_verify".to_owned(), "1".to_owned()];
        assert!(has(
            &output_args("rtmps://x/y", &tls),
            &["-tls_verify", "1"]
        ));
    }
}
