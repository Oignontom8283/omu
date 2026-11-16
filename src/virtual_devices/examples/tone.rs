//! Plays a sine tone (a stand-in "application" for tests and demos).
//!
//! `cargo run --example tone -- --device "Voicemeeter In 1" [--seconds 8] [--hz 1000] [--amplitude 0.1]`
//!
//! It never plays on the user's speakers by surprise: a device must be named (`--device`, a part of its name,
//! ideally a virtual one that nobody hears), or `--audible` must be given to use the default playback device.

#[cfg(windows)]
fn main() {
    use omu_hal::wasapi::WasapiBackend;
    use omu_hal::Backend;
    use omu_types::DeviceRef;
    use std::time::Duration;

    let args: Vec<String> = std::env::args().collect();
    let get = |name: &str, default: f64| -> f64 {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let (seconds, hz, amplitude) = (
        get("--seconds", 8.0),
        get("--hz", 1000.0),
        get("--amplitude", 0.1) as f32,
    );
    let named = args
        .iter()
        .position(|a| a == "--device")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let device = match (named, args.iter().any(|a| a == "--audible")) {
        (Some(name), _) => DeviceRef::Name(name),
        (None, true) => DeviceRef::Default,
        (None, false) => {
            eprintln!("refusing to play on the default device unannounced: pass --device NAME, or --audible");
            std::process::exit(2);
        }
    };

    let backend = WasapiBackend::new();
    let stream = match backend.open_render(&device) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot open the default playback device: {e}");
            std::process::exit(1);
        }
    };
    let format = stream.format();
    let mut phase = 0.0f64;
    let step = std::f64::consts::TAU * hz / format.sample_rate as f64;
    let channels = format.channels as usize;
    let running = stream
        .start(Box::new(move |out: &mut [f32]| {
            for frame in out.chunks_mut(channels) {
                let v = phase.sin() as f32 * amplitude;
                frame.fill(v);
                phase = (phase + step) % std::f64::consts::TAU;
            }
        }))
        .unwrap_or_else(|e| {
            eprintln!("cannot start playback: {e}");
            std::process::exit(1);
        });
    println!(
        "playing {hz} Hz at {amplitude} for {seconds} s (pid {})",
        std::process::id()
    );
    std::thread::sleep(Duration::from_secs_f64(seconds));
    drop(running);
}

#[cfg(not(windows))]
fn main() {
    eprintln!("this example needs the Windows audio backend");
}
