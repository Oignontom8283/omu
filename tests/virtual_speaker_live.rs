//! The virtual speaker on the REAL system: an emulated USB audio device attached through usbip-win2. A tone is played
//! into it through WASAPI like any program would, and must come out of the graph's "virtual speaker" block. Silent: the
//! virtual speaker has no physical output, and the test refuses to play on any device that is not the one it created.
//!
//! It attaches a USB device (named "Omu Speaker Live Test") for a few seconds, so it is ignored by default and needs
//! usbip-win2 installed: `cargo test --test virtual_speaker_live -- --ignored --nocapture`.
#![cfg(windows)]

use omu::engine::{Engine, EngineConfig, NodeState};
use omu::graph::validate;
use omu::hal::sim::SimBackend;
use omu::hal::wasapi::WasapiBackend;
use omu::hal::{Backend, Direction};
use omu::types::{AudioFormat, DeviceRef, EdgeSpec, GraphSpec, NodeKind, NodeSpec, PortRef};
use omu::virtual_devices::{platform_provider, UsbipExe, UsbipTool};
use std::sync::Arc;
use std::time::{Duration, Instant};

const NAME: &str = "Omu Speaker Live Test";

fn speaker_name(wasapi: &WasapiBackend) -> Option<String> {
    wasapi
        .list_devices()
        .ok()?
        .into_iter()
        .find_map(|d| (d.direction == Direction::Render && d.name.contains(NAME)).then_some(d.name))
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
fn what_a_program_plays_into_a_virtual_speaker_comes_out_of_the_graph() {
    UsbipExe
        .available()
        .expect("usbip-win2 must be installed for this test");
    let wasapi = Arc::new(WasapiBackend::new());
    assert!(
        speaker_name(&wasapi).is_none(),
        "a speaker called {NAME:?} already exists: stop the other program first"
    );

    // the virtual speaker is real; where its sound goes is simulated (and recorded)
    let sim = SimBackend::new();
    let recording = sim.add_render_device("Sink", AudioFormat::ENGINE, 0.0);
    let engine = Engine::new(
        Arc::new(sim) as Arc<dyn Backend>,
        Some(platform_provider()),
        EngineConfig::default(),
    );
    let spec = GraphSpec {
        nodes: vec![
            NodeSpec {
                id: "spk".into(),
                display: None,
                kind: NodeKind::VirtualSpeaker { name: NAME.into() },
            },
            NodeSpec {
                id: "out".into(),
                display: None,
                kind: NodeKind::DeviceRender {
                    device: DeviceRef::Name("Sink".into()),
                },
            },
        ],
        edges: vec![EdgeSpec {
            from: PortRef::new("spk", "out"),
            to: PortRef::new("out", "in"),
        }],
    };
    engine.apply(validate(&spec).unwrap());
    for n in &engine.status().nodes {
        assert_eq!(n.state, NodeState::Running, "{}: {:?}", n.id, n.state);
    }

    assert!(
        wait_until(Duration::from_secs(20), || speaker_name(&wasapi).is_some()),
        "Windows never showed a speaker called {NAME:?}"
    );
    let name = speaker_name(&wasapi).unwrap();
    println!("the speaker appeared as {name:?}");
    assert!(name.contains(NAME), "never play on another device");

    // play a tone into it, like a program would
    let render = wasapi
        .open_render(&DeviceRef::Name(name.clone()))
        .expect("open the virtual speaker");
    let format = render.format();
    let (rate, channels) = (format.sample_rate as f64, format.channels as usize);
    let mut n = 0u64;
    let running = render
        .start(Box::new(move |out| {
            for frame in out.chunks_mut(channels) {
                let v = (n as f64 * 1000.0 * std::f64::consts::TAU / rate).sin() as f32 * 0.5;
                frame.fill(v);
                n += 1;
            }
        }))
        .expect("play into the virtual speaker");
    for second in 1..=6 {
        std::thread::sleep(Duration::from_millis(1000));
        println!(
            "after {second} s: {} frames reached the graph's output",
            recording.lock().unwrap().len() / 2
        );
    }
    let status = engine.status();
    drop(running);

    let left: Vec<f32> = recording.lock().unwrap().chunks(2).map(|f| f[0]).collect();
    assert!(
        left.len() > 48_000 * 4,
        "only {} frames arrived",
        left.len()
    );
    // the recording starts when the engine does; the tone plays during its last seconds
    let steady = &left[left.len() - 4 * 48_000..left.len() - 48_000];
    let rms = (steady.iter().map(|s| s * s).sum::<f32>() / steady.len() as f32).sqrt();
    let hz = steady
        .windows(2)
        .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
        .count() as f32
        / (steady.len() as f32 / 48_000.0);
    println!(
        "{} frames, rms {rms:.4} (a 0.5 tone is 0.3536), {hz:.0} Hz",
        left.len()
    );
    println!("{status:#?}");
    assert!((rms - 0.5 / 2f32.sqrt()).abs() < 0.06, "wrong level: {rms}");
    assert!((hz - 1000.0).abs() < 40.0, "wrong tone: {hz} Hz");

    // stopping the engine removes the device from the system
    drop(engine);
    assert!(
        wait_until(Duration::from_secs(20), || speaker_name(&wasapi).is_none()),
        "the speaker is still there after the engine stopped"
    );
    println!("the speaker went away");
}
