//! Tests on the REAL Windows audio system. They need Voicemeeter's virtual devices, so they are ignored by
//! default: `cargo test --test wasapi_live -- --ignored --nocapture`.
//!
//! They must never make noise on the user's speakers or headphones: the tone is played on a virtual device that is
//! routed nowhere audible ("Voicemeeter In 1", an unused extension input), and the tests refuse to run if that
//! device is missing instead of falling back to a real one.
#![cfg(windows)]

use omu_engine::{Engine, EngineConfig, NodeState};
use omu_graph::validate;
use omu_hal::wasapi::{graph_thread_init, WasapiBackend};
use omu_hal::Backend;
use omu_types::{AppTarget, DeviceRef, EdgeSpec, GraphSpec, NodeKind, NodeSpec, PortRef};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn tone_exe() -> std::path::PathBuf {
    // target/<profile>/deps/<test>.exe -> target/<profile>/examples/tone.exe
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    p.pop();
    p.join("examples").join("tone.exe")
}

struct Tone(Child);
impl Drop for Tone {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Where the test tone is played: silent by construction.
const SILENT_DEVICE: &str = "Voicemeeter In 1";

fn start_tone(seconds: u32, hz: u32, amplitude: f32) -> Tone {
    let exe = tone_exe();
    assert!(
        exe.exists(),
        "build the example first: cargo build --example tone ({})",
        exe.display()
    );
    Tone(
        Command::new(exe)
            .args([
                "--device",
                SILENT_DEVICE,
                "--seconds",
                &seconds.to_string(),
                "--hz",
                &hz.to_string(),
                "--amplitude",
                &amplitude.to_string(),
            ])
            .stdout(Stdio::null())
            .spawn()
            .expect("cannot start the tone player"),
    )
}

fn left(rec: &[f32], ch: usize) -> Vec<f32> {
    rec.chunks(ch).map(|f| f[0]).collect()
}
fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
}
fn tone_hz(x: &[f32], rate: f32) -> f32 {
    x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count() as f32 / (x.len() as f32 / rate)
}

/// The sound of another program, captured by process loopback, goes through the engine to the virtual cable's
/// input; recording the cable's output proves the whole chain on this machine.
#[test]
#[ignore = "needs Voicemeeter's virtual devices"]
fn an_applications_sound_reaches_a_virtual_cable() {
    let backend = Arc::new(WasapiBackend::new());
    let devices = backend.list_devices().unwrap();
    assert!(
        devices
            .iter()
            .any(|d| d.direction == omu_hal::Direction::Render && d.name.contains(SILENT_DEVICE)),
        "{SILENT_DEVICE:?} is missing: refusing to play the test tone anywhere else"
    );

    let tone = start_tone(12, 1000, 0.1);
    std::thread::sleep(Duration::from_millis(1500)); // let the player open its stream

    let cfg = EngineConfig {
        thread_init: Some(graph_thread_init()),
        ..Default::default()
    };
    let engine = Engine::new(backend.clone() as Arc<dyn Backend>, None, cfg);
    let spec = GraphSpec {
        nodes: vec![
            NodeSpec {
                id: "app".into(),
                display: None,
                inputs: Default::default(),
                muted: false,
                kind: NodeKind::AppCapture {
                    target: AppTarget::Pid(tone.0.id()),
                },
            },
            NodeSpec {
                id: "cable".into(),
                display: None,
                inputs: Default::default(),
                muted: false,
                kind: NodeKind::DeviceRender {
                    device: DeviceRef::Name("Voicemeeter Input".into()),
                },
            },
        ],
        edges: vec![EdgeSpec {
            from: PortRef::new("app", "out"),
            to: PortRef::new("cable", "in"),
        }],
    };
    engine.apply(validate(&spec).unwrap());

    // record the cable's output (what another program, e.g. Discord, would hear as a microphone)
    let capture = backend
        .open_capture(&DeviceRef::Name("Voicemeeter Out B1".into()))
        .expect("Voicemeeter Out B1");
    let format = capture.format();
    let recorded = Arc::new(Mutex::new(Vec::<f32>::new()));
    let rec2 = recorded.clone();
    let running = capture
        .start(Box::new(move |b| rec2.lock().unwrap().extend_from_slice(b)))
        .unwrap();
    std::thread::sleep(Duration::from_millis(5000));
    let status = engine.status();
    drop(running);
    drop(engine);

    println!("{status:#?}");
    for n in &status.nodes {
        assert_eq!(n.state, NodeState::Running, "node {} not running", n.id);
    }
    let l = left(&recorded.lock().unwrap(), format.channels as usize);
    assert!(
        l.len() as f32 > format.sample_rate as f32 * 4.0,
        "only {} frames recorded",
        l.len()
    );
    let steady = &l[format.sample_rate as usize..]; // skip the first second
    let level = rms(steady);
    let hz = tone_hz(steady, format.sample_rate as f32);
    println!("recorded {} frames: rms {level:.4}, {hz:.0} Hz", l.len());
    // the tone has amplitude 0.1 (rms 0.0707); allow for the cable's own level handling
    assert!(level > 0.02, "almost silent: rms {level}");
    assert!((hz - 1000.0).abs() < 30.0, "wrong tone: {hz} Hz");
}
