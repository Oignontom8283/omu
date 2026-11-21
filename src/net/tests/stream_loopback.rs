//! A live stream through the real FFmpeg, on this computer: a tone leaves as an MPEG-TS stream over UDP (what an output block
//! sends) and comes back through an input block that reads it. Skipped where FFmpeg is not installed.

use omu_hal::{CaptureStream, RenderStream};
use omu_net::stream::{StreamCapture, StreamRender};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn ffmpeg() -> Option<PathBuf> {
    let ok = std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .ok()?
        .status
        .success();
    ok.then(|| PathBuf::from("ffmpeg"))
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

#[test]
fn a_tone_goes_out_as_a_stream_and_comes_back_in() {
    let Some(ffmpeg) = ffmpeg() else {
        eprintln!("skipped: FFmpeg is not installed on this computer");
        return;
    };
    let port = std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("udp://127.0.0.1:{port}");

    // the listener first, so that nothing the sender says is lost
    let heard = Arc::new(Mutex::new(Vec::<f32>::new()));
    let sink = heard.clone();
    let input = Box::new(StreamCapture::open(&ffmpeg, &url, &[]).expect("FFmpeg starts"))
        .start(Box::new(move |samples: &[f32]| {
            sink.lock().unwrap().extend_from_slice(samples)
        }))
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));

    let mut n = 0usize;
    let output = Box::new(StreamRender::open(&ffmpeg, &url, &[]).expect("FFmpeg starts"))
        .start(Box::new(move |out: &mut [f32]| {
            for frame in out.as_chunks_mut::<2>().0 {
                let s = 0.5 * (2.0 * std::f32::consts::PI * 440.0 * n as f32 / 48_000.0).sin();
                frame.copy_from_slice(&[s, s]);
                n += 1;
            }
        }))
        .unwrap();

    // two seconds of the tone arrive (the start of a stream takes a moment)
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        let len = heard.lock().unwrap().len();
        if len >= 4 * 48_000 {
            break;
        }
        assert!(
            Instant::now() < end,
            "the stream never arrived ({len} samples)"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let all = heard.lock().unwrap().clone();
    let level = rms(&all[all.len() - 48_000..]);
    // AAC is not exact, but the tone keeps about its level (0.5 / sqrt 2)
    assert!(
        (level - 0.3535).abs() < 0.08,
        "the tone keeps its level through the stream: rms {level}"
    );
    drop(output);
    drop(input);
}
