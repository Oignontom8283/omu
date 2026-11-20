//! The virtual microphone on the REAL system: an emulated USB audio device attached through usbip-win2, recorded
//! through WASAPI like any program would. Silent: nothing is played on any speaker.
//!
//! It attaches a USB device (named "Omu Live Test") for a few seconds, so it is ignored by default and needs
//! usbip-win2 installed: `cargo test --test virtual_mic_live -- --ignored --nocapture`.
#![cfg(windows)]

use omu_engine::{Engine, EngineConfig, NodeState};
use omu_graph::validate;
use omu_hal::sim::{SimBackend, SimSource};
use omu_hal::wasapi::WasapiBackend;
use omu_hal::{Backend, Direction};
use omu_types::{AppTarget, DeviceRef, EdgeSpec, GraphSpec, NodeKind, NodeSpec, PortRef};
use omu_virtual_devices::{UsbMicProvider, UsbipExe, UsbipTool};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const NAME: &str = "Omu Live Test";

fn mic_present(wasapi: &WasapiBackend) -> bool {
    wasapi
        .list_devices()
        .map(|d| {
            d.iter()
                .any(|d| d.direction == Direction::Capture && d.name.contains(NAME))
        })
        .unwrap_or(false)
}

fn wait_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    false
}

#[test]
#[ignore = "attaches a USB device through usbip-win2 for a few seconds"]
fn the_engines_audio_comes_out_of_a_virtual_microphone() {
    UsbipExe
        .available()
        .expect("usbip-win2 must be installed for this test");
    let wasapi = Arc::new(WasapiBackend::new());
    assert!(
        !mic_present(&wasapi),
        "a microphone called {NAME:?} already exists: stop the other program first"
    );

    // the application is simulated (silent), the virtual microphone is real
    let sim = SimBackend::new();
    sim.add_app(1, "tone.exe", SimSource::tone(1000.0));
    let engine = Engine::new(
        Arc::new(sim) as Arc<dyn Backend>,
        // not on the product's fixed port and not permanent: a test must leave nothing behind for the next boot
        Some(Arc::new(
            UsbMicProvider::new(Arc::new(UsbipExe)).with_server_port(0),
        )),
        EngineConfig::default(),
    );
    let spec = GraphSpec {
        nodes: vec![
            NodeSpec {
                id: "app".into(),
                display: None,
                inputs: Default::default(),
                kind: NodeKind::AppCapture {
                    target: AppTarget::ProcessName("tone.exe".into()),
                },
            },
            NodeSpec {
                id: "vol".into(),
                display: None,
                inputs: Default::default(),
                kind: NodeKind::Gain { db: -12.0 },
            },
            NodeSpec {
                id: "mic".into(),
                display: None,
                inputs: Default::default(),
                kind: NodeKind::VirtualMic { name: NAME.into() },
            },
        ],
        edges: vec![
            EdgeSpec {
                from: PortRef::new("app", "out"),
                to: PortRef::new("vol", "in"),
            },
            EdgeSpec {
                from: PortRef::new("vol", "out"),
                to: PortRef::new("mic", "in"),
            },
        ],
    };
    engine.apply(validate(&spec).unwrap());
    let status = engine.status();
    for n in &status.nodes {
        assert_eq!(n.state, NodeState::Running, "{}: {:?}", n.id, n.state);
    }

    assert!(
        wait_until(Duration::from_secs(20), || mic_present(&wasapi)),
        "Windows never showed a microphone called {NAME:?}"
    );
    println!("the microphone appeared");

    // record it like Discord would
    let capture = wasapi
        .open_capture(&DeviceRef::Name(NAME.into()))
        .expect("open the virtual microphone");
    let format = capture.format();
    let recorded = Arc::new(Mutex::new(Vec::<f32>::new()));
    let sink = recorded.clone();
    let running = capture
        .start(Box::new(move |b| sink.lock().unwrap().extend_from_slice(b)))
        .unwrap();
    for second in 1..=6 {
        std::thread::sleep(Duration::from_millis(1000));
        println!(
            "after {second} s: {} frames recorded, mic present: {}",
            recorded.lock().unwrap().len() / format.channels as usize,
            mic_present(&wasapi)
        );
    }
    let status = engine.status();
    drop(running);

    let ch = format.channels as usize;
    let left: Vec<f32> = recorded.lock().unwrap().chunks(ch).map(|f| f[0]).collect();
    assert!(
        left.len() as f32 > format.sample_rate as f32 * 4.0,
        "only {} frames recorded",
        left.len()
    );
    let steady = &left[format.sample_rate as usize..];
    let rms = (steady.iter().map(|s| s * s).sum::<f32>() / steady.len() as f32).sqrt();
    let hz = steady
        .windows(2)
        .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
        .count() as f32
        / (steady.len() as f32 / format.sample_rate as f32);
    println!(
        "recorded {} frames at {} Hz: rms {rms:.4} (expected about 0.0887), {hz:.0} Hz",
        left.len(),
        format.sample_rate
    );
    println!("{status:#?}");
    assert!(
        (rms - 0.5 * 0.2512 / 2f32.sqrt()).abs() < 0.03,
        "wrong level: {rms}"
    );
    assert!((hz - 1000.0).abs() < 40.0, "wrong tone: {hz} Hz");

    // stopping the engine removes the device from the system
    drop(engine);
    assert!(
        wait_until(Duration::from_secs(20), || !mic_present(&wasapi)),
        "the microphone is still there after the engine stopped"
    );
    println!("the microphone went away");
}
