//! Creates a virtual microphone that carries a quiet tone, for as long as you ask (default 60 s).
//! Nothing is played on your speakers: the tone only comes out of the microphone, so a program (Windows' sound
//! settings, Discord, OBS...) that records from "Microphone (<name>)" will hear it.
//!
//! `cargo run --example virtual_mic -- [--name "Omu Test Tone"] [--seconds 60] [--hz 440]`
//! Needs usbip-win2 (the USB/IP client driver) installed.

#[cfg(windows)]
fn main() {
    use omu_hal::HalError;
    use omu_virtual_devices::platform_provider;
    use std::time::Duration;

    let args: Vec<String> = std::env::args().collect();
    let get = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let name = get("--name").unwrap_or_else(|| "Omu Test Tone".into());
    let seconds: f64 = get("--seconds")
        .and_then(|v| v.parse().ok())
        .unwrap_or(60.0);
    let hz: f64 = get("--hz").and_then(|v| v.parse().ok()).unwrap_or(440.0);

    let provider = platform_provider();
    let stream = match provider.create_microphone(&name) {
        Ok(s) => s,
        Err(HalError::Unsupported(m)) => {
            eprintln!("not available: {m}");
            std::process::exit(2);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    let format = stream.format();
    let step = std::f64::consts::TAU * hz / format.sample_rate as f64;
    let mut phase = 0.0f64;
    let channels = format.channels as usize;
    let running = match stream.start(Box::new(move |out: &mut [f32]| {
        for frame in out.chunks_mut(channels) {
            frame.fill(phase.sin() as f32 * 0.2);
            phase = (phase + step) % std::f64::consts::TAU;
        }
    })) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    println!("virtual microphone {name:?} is up with a {hz} Hz tone for {seconds} s (Ctrl-C to stop; it disappears when this program ends)");
    std::thread::sleep(Duration::from_secs_f64(seconds));
    drop(running);
    println!("stopped");
}

#[cfg(not(windows))]
fn main() {
    eprintln!("virtual microphones through USB/IP are only available on Windows for now");
}
