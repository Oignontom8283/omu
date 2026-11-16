//! Records a capture device for a few seconds and prints its level and dominant tone: a quick check that a
//! microphone (real or virtual) carries what you expect. Silent: it only listens.
//!
//! `cargo run --example probe_mic -- "Omu Test Tone" [--seconds 4]`

#[cfg(windows)]
fn main() {
    use omu_hal::wasapi::WasapiBackend;
    use omu_hal::Backend;
    use omu_types::DeviceRef;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    let args: Vec<String> = std::env::args().collect();
    let Some(name) = args.get(1).filter(|a| !a.starts_with("--")) else {
        eprintln!("usage: probe_mic <part of the device name> [--seconds 4]");
        std::process::exit(2);
    };
    let seconds: f64 = args
        .iter()
        .position(|a| a == "--seconds")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(4.0);
    let stream = match WasapiBackend::new().open_capture(&DeviceRef::Name(name.clone())) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot open {name:?}: {e}");
            std::process::exit(1);
        }
    };
    let format = stream.format();
    let data = Arc::new(Mutex::new(Vec::<f32>::new()));
    let sink = data.clone();
    let running = stream
        .start(Box::new(move |b| sink.lock().unwrap().extend_from_slice(b)))
        .expect("start");
    std::thread::sleep(Duration::from_secs_f64(seconds));
    drop(running);
    let ch = format.channels as usize;
    let left: Vec<f32> = data.lock().unwrap().chunks(ch).map(|f| f[0]).collect();
    let skip = (format.sample_rate as usize / 2).min(left.len() / 2);
    let s = &left[skip..];
    let rms = (s.iter().map(|x| x * x).sum::<f32>() / s.len().max(1) as f32).sqrt();
    let hz = s.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count() as f32
        / (s.len().max(1) as f32 / format.sample_rate as f32);
    println!(
        "{name:?}: {} frames at {} Hz, level {rms:.4} rms ({:.0} dB), about {hz:.0} Hz",
        left.len(),
        format.sample_rate,
        20.0 * rms.max(1e-6).log10()
    );
}

#[cfg(not(windows))]
fn main() {
    eprintln!("this example needs the Windows audio backend");
}
