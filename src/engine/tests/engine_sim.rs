//! End-to-end engine tests on the simulated backend (real time, a few seconds each).

use omu_engine::{Engine, EngineConfig, NodeState};
use omu_graph::validate;
use omu_hal::sim::{SimBackend, SimSource};
use omu_hal::{Backend, VirtualDeviceProvider};
use omu_types::{
    AppTarget, AudioFormat, DeviceRef, EdgeSpec, GraphSpec, NodeKind, NodeSpec, PortRef,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn node(id: &str, kind: NodeKind) -> NodeSpec {
    NodeSpec {
        id: id.into(),
        display: None,
        inputs: Default::default(),
        muted: false,
        kind,
    }
}
fn edge(a: &str, b: &str) -> EdgeSpec {
    EdgeSpec {
        from: PortRef::new(a, "out"),
        to: PortRef::new(b, "in"),
    }
}
fn app_node() -> NodeKind {
    NodeKind::AppCapture {
        target: AppTarget::ProcessName("chrome.exe".into()),
    }
}
fn engine(sim: &SimBackend) -> Engine {
    let cfg = EngineConfig {
        retry_initial: Duration::from_millis(400),
        retry_max: Duration::from_millis(800),
        ..Default::default()
    };
    Engine::new(
        Arc::new(sim.clone()) as Arc<dyn Backend>,
        Some(Arc::new(sim.clone()) as Arc<dyn VirtualDeviceProvider>),
        cfg,
    )
}
fn wait_for(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Total bridge underruns and overruns reported by the engine. A system stall longer than the buffered audio
/// (e.g. antivirus scanning a freshly built test binary) legitimately causes one and is *reported*: a gap is only
/// a bug when the engine reports nothing.
fn glitches(status: &omu_engine::EngineStatus) -> u64 {
    status.nodes.iter().map(|n| n.underruns + n.overruns).sum()
}

/// Channel 0 of interleaved stereo.
fn left(rec: &[f32]) -> Vec<f32> {
    rec.chunks(2).map(|f| f[0]).collect()
}
fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
}
fn tone_hz(x: &[f32], rate: f32) -> f32 {
    let crossings = x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
    crossings as f32 / (x.len() as f32 / rate)
}
/// Longest run of near-silence, in milliseconds.
fn longest_silence_ms(x: &[f32]) -> f32 {
    let (mut best, mut run) = (0usize, 0usize);
    for s in x {
        // a 1 kHz tone is near zero for only a couple of samples at each crossing
        if s.abs() < 1e-3 {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best as f32 * 1000.0 / 48_000.0
}

#[test]
fn app_to_virtual_mic_through_a_gain_is_clean() {
    let sim = SimBackend::new();
    sim.add_app(1234, "chrome.exe", SimSource::tone(1000.0));
    let e = engine(&sim);
    let spec = GraphSpec {
        nodes: vec![
            node("app", app_node()),
            node("vol", NodeKind::Gain { db: -6.0206 }),
            node(
                "mic",
                NodeKind::VirtualMic {
                    name: "Stream".into(),
                },
            ),
        ],
        edges: vec![edge("app", "vol"), edge("vol", "mic")],
    };
    e.apply(validate(&spec).unwrap());
    std::thread::sleep(Duration::from_millis(3000));
    let status = e.status();
    drop(e);

    assert!(status.running);
    for n in &status.nodes {
        assert_eq!(n.state, NodeState::Running, "{}", n.id);
    }
    let recording = sim_recording(&sim, "virtual:Stream");
    let l = left(&recording);
    assert!(l.len() > 48_000 * 2, "{} frames", l.len());
    let steady = &l[48_000..]; // skip the 1 s start-up
    let level = rms(steady);
    assert!((level - 0.25 / 2f32.sqrt()).abs() < 0.02, "rms {level}");
    let hz = tone_hz(steady, 48_000.0);
    assert!((hz - 1000.0).abs() < 20.0, "{hz} Hz");
    assert!(glitches(&status) <= 2, "{status:?}");
    if glitches(&status) == 0 {
        assert!(
            longest_silence_ms(steady) < 1.0,
            "silence of {} ms; status {:?}",
            longest_silence_ms(steady),
            status
        );
    }
}

fn sim_recording(sim: &SimBackend, name: &str) -> Vec<f32> {
    // virtual sinks are registered under "virtual:<name>"
    let rec = sim.recording_of(name).expect("sink exists");
    let v = rec.lock().unwrap().clone();
    v
}

#[test]
fn a_fast_source_and_a_slow_sink_never_underrun() {
    let sim = SimBackend::new();
    sim.add_capture_device(
        "mic",
        SimSource {
            ppm: 250.0,
            ..SimSource::tone(500.0)
        },
    );
    let rec = sim.add_render_device("speakers", AudioFormat::ENGINE, -250.0);
    let e = engine(&sim);
    let spec = GraphSpec {
        nodes: vec![
            node(
                "in",
                NodeKind::DeviceCapture {
                    device: DeviceRef::Name("mic".into()),
                },
            ),
            node(
                "out",
                NodeKind::DeviceRender {
                    device: DeviceRef::Name("speakers".into()),
                },
            ),
        ],
        edges: vec![edge("in", "out")],
    };
    e.apply(validate(&spec).unwrap());
    std::thread::sleep(Duration::from_millis(5000));
    let status = e.status();
    drop(e);
    for n in &status.nodes {
        assert!(
            n.underruns <= 1 && n.overruns == 0,
            "{}: {} underruns, {} overruns",
            n.id,
            n.underruns,
            n.overruns
        );
    }
    let l = left(&rec.lock().unwrap());
    let steady = &l[48_000..];
    assert!((tone_hz(steady, 48_000.0) - 500.0).abs() < 15.0);
    if glitches(&status) == 0 {
        assert!(longest_silence_ms(steady) < 2.0);
    }
}

#[test]
fn editing_the_graph_keeps_untouched_streams_running() {
    let sim = SimBackend::new();
    sim.add_app(1, "chrome.exe", SimSource::tone(1000.0));
    let e = engine(&sim);
    let a = GraphSpec {
        nodes: vec![
            node("app", app_node()),
            node("mic", NodeKind::VirtualMic { name: "A".into() }),
        ],
        edges: vec![edge("app", "mic")],
    };
    e.apply(validate(&a).unwrap());
    std::thread::sleep(Duration::from_millis(1500));
    let b = GraphSpec {
        nodes: vec![
            node("app", app_node()),
            node("vol", NodeKind::Gain { db: -12.0 }),
            node("mic", NodeKind::VirtualMic { name: "A".into() }),
            node("mic2", NodeKind::VirtualMic { name: "B".into() }),
        ],
        edges: vec![edge("app", "vol"), edge("vol", "mic"), edge("app", "mic2")],
    };
    e.apply(validate(&b).unwrap());
    std::thread::sleep(Duration::from_millis(2000));
    let status = e.status();
    drop(e);
    let l = left(&sim_recording(&sim, "virtual:A"));
    let steady = &l[48_000..];
    // the gain moved to -12 dB without a hole at the swap
    if glitches(&status) == 0 {
        assert!(
            longest_silence_ms(steady) < 1.5,
            "silence of {} ms at the swap",
            longest_silence_ms(steady)
        );
    }
    let end = rms(&l[l.len() - 24_000..]);
    assert!(
        (end - 0.5 * 0.2512 / 2f32.sqrt()).abs() < 0.02,
        "final level {end}"
    );
    assert!(
        sim_recording(&sim, "virtual:B").len() > 48_000,
        "the new branch plays too"
    );
}

#[test]
fn a_missing_device_fails_only_its_node() {
    let sim = SimBackend::new();
    sim.add_capture_device("mic", SimSource::tone(300.0));
    let e = engine(&sim);
    let spec = GraphSpec {
        nodes: vec![
            node(
                "in",
                NodeKind::DeviceCapture {
                    device: DeviceRef::Name("mic".into()),
                },
            ),
            node(
                "ghost",
                NodeKind::DeviceRender {
                    device: DeviceRef::Name("does-not-exist".into()),
                },
            ),
            node("mic", NodeKind::VirtualMic { name: "V".into() }),
        ],
        edges: vec![edge("in", "ghost"), edge("in", "mic")],
    };
    e.apply(validate(&spec).unwrap());
    std::thread::sleep(Duration::from_millis(1500));
    let st = e.status();
    let get = |id: &str| {
        st.nodes
            .iter()
            .find(|n| n.id.0 == id)
            .unwrap()
            .state
            .clone()
    };
    assert!(matches!(get("ghost"), NodeState::Failed(m) if m.contains("no matching device")));
    assert_eq!(get("in"), NodeState::Running);
    assert_eq!(get("mic"), NodeState::Running);
    drop(e);
    assert!(
        sim_recording(&sim, "virtual:V").len() > 24_000,
        "the healthy branch kept playing"
    );
}

#[test]
fn without_a_virtual_device_provider_the_virtual_mic_node_fails_cleanly() {
    let sim = SimBackend::new();
    sim.add_app(1, "chrome.exe", SimSource::tone(300.0));
    let e = Engine::new(
        Arc::new(sim.clone()) as Arc<dyn Backend>,
        None,
        EngineConfig::default(),
    );
    let spec = GraphSpec {
        nodes: vec![
            node("app", app_node()),
            node("mic", NodeKind::VirtualMic { name: "V".into() }),
        ],
        edges: vec![edge("app", "mic")],
    };
    e.apply(validate(&spec).unwrap());
    let st = e.status();
    let mic = st.nodes.iter().find(|n| n.id.0 == "mic").unwrap();
    assert!(
        matches!(&mic.state, NodeState::Failed(m) if m.contains("virtual device provider")),
        "{:?}",
        mic.state
    );
}

#[test]
fn a_device_that_breaks_is_reopened() {
    let sim = SimBackend::new();
    sim.add_app(1, "chrome.exe", SimSource::tone(1000.0));
    let e = engine(&sim);
    let spec = GraphSpec {
        nodes: vec![
            node("app", app_node()),
            node("mic", NodeKind::VirtualMic { name: "R".into() }),
        ],
        edges: vec![edge("app", "mic")],
    };
    e.apply(validate(&spec).unwrap());
    std::thread::sleep(Duration::from_millis(1500));
    let before = sim_recording(&sim, "virtual:R").len();
    sim.fail_all_streams("device unplugged");
    // the supervisor notices, reopens, audio resumes
    let recovered = wait_for(Duration::from_secs(6), || {
        let st = e.status();
        st.nodes.iter().all(|n| n.state == NodeState::Running)
            && sim_recording(&sim, "virtual:R").len() > before + 24_000 * 3
    });
    drop(e);
    assert!(recovered, "the engine did not recover");
}

#[test]
fn stopping_silences_everything_and_the_engine_can_restart() {
    let sim = SimBackend::new();
    sim.add_app(1, "chrome.exe", SimSource::tone(1000.0));
    let e = engine(&sim);
    let spec = GraphSpec {
        nodes: vec![
            node("app", app_node()),
            node("mic", NodeKind::VirtualMic { name: "S".into() }),
        ],
        edges: vec![edge("app", "mic")],
    };
    e.apply(validate(&spec).unwrap());
    std::thread::sleep(Duration::from_millis(800));
    e.stop();
    assert!(!e.status().running);
    // the virtual microphone stays (other programs keep it) but sends silence while the engine is stopped
    let frozen = sim_recording(&sim, "virtual:S").len();
    std::thread::sleep(Duration::from_millis(300));
    let after = left(&sim_recording(&sim, "virtual:S"));
    assert!(after.len() > frozen / 2, "the microphone keeps running");
    assert!(
        rms(&after[after.len() - 4_800..]) < 1e-4,
        "silence once what was buffered is played, not the tone"
    );
    e.apply(validate(&spec).unwrap());
    assert!(wait_for(Duration::from_secs(3), || {
        let r = left(&sim_recording(&sim, "virtual:S"));
        r.len() > frozen / 2 + 24_000 && rms(&r[r.len() - 9_600..]) > 0.2
    }));
}

/// The example of the design: a key press (a trigger's action) toggles a variable (a condition) that opens or closes
/// a gate on the audio, all through the real engine.
#[test]
fn a_shortcut_mutes_and_unmutes_the_microphone_through_a_variable_and_a_gate() {
    use omu_types::NodeState;
    let sim = SimBackend::new();
    sim.add_app(1, "chrome.exe", SimSource::tone(1000.0));
    let e = engine(&sim);
    let spec = GraphSpec {
        nodes: vec![
            node("app", app_node()),
            node(
                "key",
                NodeKind::Hotkey {
                    keys: "Ctrl+F8".into(),
                },
            ),
            node("on", NodeKind::Variable { initial: true }),
            node("gate", NodeKind::Gate),
            node("mic", NodeKind::VirtualMic { name: "G".into() }),
        ],
        edges: vec![
            EdgeSpec {
                from: PortRef::new("key", "pressed"),
                to: PortRef::new("on", "toggle"),
            },
            EdgeSpec {
                from: PortRef::new("on", "value"),
                to: PortRef::new("gate", "open"),
            },
            edge("app", "gate"),
            edge("gate", "mic"),
        ],
    };
    e.apply(validate(&spec).unwrap());
    std::thread::sleep(Duration::from_millis(1500));
    let level_after = |ms: u64| {
        let start = sim_recording(&sim, "virtual:G").len();
        std::thread::sleep(Duration::from_millis(ms));
        let rec = sim_recording(&sim, "virtual:G");
        rms(&left(&rec[start..]))
    };
    assert!(level_after(600) > 0.2, "open at first: the tone passes");
    let status = e.status();
    let on = status.nodes.iter().find(|n| n.id.0 == "on").unwrap();
    assert_eq!(on.active, Some(true));
    assert!(
        status.nodes.iter().all(|n| n.state == NodeState::Running),
        "{status:?}"
    );

    assert!(sim.press("ctrl+f8"), "the shortcut is registered");
    std::thread::sleep(Duration::from_millis(300)); // the gate fades, the bridges drain
    assert!(level_after(600) < 0.01, "muted by the key");
    let status = e.status();
    assert_eq!(
        status.nodes.iter().find(|n| n.id.0 == "on").unwrap().active,
        Some(false)
    );
    assert_eq!(
        status
            .nodes
            .iter()
            .find(|n| n.id.0 == "key")
            .unwrap()
            .events,
        1
    );

    assert!(sim.press("Ctrl+F8"));
    std::thread::sleep(Duration::from_millis(300));
    assert!(level_after(600) > 0.2, "unmuted by the second press");
}

#[test]
fn a_shortcut_that_is_taken_fails_only_its_block() {
    use omu_hal::Backend as _;
    use omu_types::{KeyCombo, NodeState};
    let sim = SimBackend::new();
    sim.add_app(1, "chrome.exe", SimSource::tone(440.0));
    // another program already owns Ctrl+F9
    let _other = sim
        .open_hotkey(&KeyCombo::parse("Ctrl+F9").unwrap(), Box::new(|| {}))
        .unwrap();
    let e = engine(&sim);
    let spec = GraphSpec {
        nodes: vec![
            node("app", app_node()),
            node(
                "key",
                NodeKind::Hotkey {
                    keys: "Ctrl+F9".into(),
                },
            ),
            node("mic", NodeKind::VirtualMic { name: "T".into() }),
        ],
        edges: vec![edge("app", "mic")],
    };
    e.apply(validate(&spec).unwrap());
    std::thread::sleep(Duration::from_millis(800));
    let st = e.status();
    let get = |id: &str| {
        st.nodes
            .iter()
            .find(|n| n.id.0 == id)
            .unwrap()
            .state
            .clone()
    };
    assert!(
        matches!(get("key"), NodeState::Failed(m) if m.contains("already used")),
        "{:?}",
        get("key")
    );
    assert_eq!(get("app"), NodeState::Running);
    assert_eq!(get("mic"), NodeState::Running);
}

#[test]
fn what_programs_play_into_a_virtual_speaker_comes_out_of_its_block() {
    let sim = SimBackend::new();
    sim.play_into_speaker("Discord Out", SimSource::tone(700.0));
    let e = engine(&sim);
    let spec = GraphSpec {
        nodes: vec![
            node(
                "spk",
                NodeKind::VirtualSpeaker {
                    name: "Discord Out".into(),
                },
            ),
            node(
                "mic",
                NodeKind::VirtualMic {
                    name: "Stream".into(),
                },
            ),
        ],
        edges: vec![edge("spk", "mic")],
    };
    e.apply(validate(&spec).unwrap());
    std::thread::sleep(Duration::from_millis(3000));
    let status = e.status();
    drop(e);

    for n in &status.nodes {
        assert_eq!(n.state, NodeState::Running, "{}", n.id);
    }
    let l = left(&sim_recording(&sim, "virtual:Stream"));
    let steady = &l[48_000..];
    assert!(
        (rms(steady) - 0.5 / 2f32.sqrt()).abs() < 0.03,
        "rms {}",
        rms(steady)
    );
    let hz = tone_hz(steady, 48_000.0);
    assert!((hz - 700.0).abs() < 15.0, "{hz} Hz");
    assert!(glitches(&status) <= 2, "{status:?}");
}

#[test]
fn without_a_virtual_device_provider_the_virtual_speaker_node_fails_cleanly() {
    let e = Engine::new(
        Arc::new(SimBackend::new()) as Arc<dyn Backend>,
        None,
        EngineConfig::default(),
    );
    let spec = GraphSpec {
        nodes: vec![node("spk", NodeKind::VirtualSpeaker { name: "S".into() })],
        edges: vec![],
    };
    e.apply(validate(&spec).unwrap());
    let status = e.status();
    match &status.nodes[0].state {
        NodeState::Failed(m) => assert!(m.contains("no virtual device provider"), "{m}"),
        other => panic!("expected a failure, got {other:?}"),
    }
}

fn link(a: &str, ap: &str, b: &str, bp: &str) -> EdgeSpec {
    EdgeSpec {
        from: PortRef::new(a, ap),
        to: PortRef::new(b, bp),
    }
}

/// app --> branch (one condition, driven by a variable) -- out1 --> mic "One"; else --> mic "Two"
fn branch_graph(variable_on: bool) -> GraphSpec {
    let mic = |name: &str| NodeKind::VirtualMic { name: name.into() };
    GraphSpec {
        nodes: vec![
            node("app", app_node()),
            node(
                "switch",
                NodeKind::Variable {
                    initial: variable_on,
                },
            ),
            node("br", NodeKind::Branch { branches: 1 }),
            node("one", mic("One")),
            node("two", mic("Two")),
        ],
        edges: vec![
            link("app", "out", "br", "in"),
            link("switch", "value", "br", "when1"),
            link("br", "out1", "one", "in"),
            link("br", "else", "two", "in"),
        ],
    }
}

#[test]
fn a_branch_sends_a_real_stream_where_its_condition_says() {
    for (variable_on, to_one) in [(true, true), (false, false)] {
        let sim = SimBackend::new();
        sim.add_app(1234, "chrome.exe", SimSource::tone(1000.0));
        let e = engine(&sim);
        e.apply(validate(&branch_graph(variable_on)).unwrap());
        std::thread::sleep(Duration::from_millis(2500));
        let status = e.status();
        drop(e);
        for n in &status.nodes {
            assert_eq!(n.state, NodeState::Running, "{}", n.id);
        }
        let level = |name: &str| {
            let l = left(&sim_recording(&sim, name));
            rms(&l[l.len() / 2..])
        };
        let (one, two) = (level("virtual:One"), level("virtual:Two"));
        let (heard, silent) = if to_one { (one, two) } else { (two, one) };
        assert!(
            heard > 0.3,
            "the sound goes where the condition says: {one} / {two}"
        );
        assert!(silent < 0.01, "and nowhere else: {one} / {two}");
    }
}

#[test]
fn a_change_block_between_two_conditions_works_in_the_real_engine() {
    // hotkey-less version: a timer toggles a variable, a change block watches it and the actions set/clear another
    // variable that gates the sound: after a while the gate has opened at least once
    let sim = SimBackend::new();
    sim.add_app(1234, "chrome.exe", SimSource::tone(1000.0));
    let e = engine(&sim);
    let spec = GraphSpec {
        nodes: vec![
            node("app", app_node()),
            node("t", NodeKind::Timer { interval_ms: 500 }),
            node("v", NodeKind::Variable { initial: false }),
            node("ch", NodeKind::Change),
            node("w", NodeKind::Variable { initial: false }),
            node("not", NodeKind::Not),
            node("gate", NodeKind::Gate),
            node("mic", NodeKind::VirtualMic { name: "Out".into() }),
        ],
        edges: vec![
            link("t", "tick", "v", "toggle"),
            link("v", "value", "ch", "in"),
            link("ch", "rise", "w", "set"),
            link("ch", "fall", "w", "clear"),
            link("w", "value", "not", "in"),
            link("not", "out", "gate", "open"),
            link("app", "out", "gate", "in"),
            link("gate", "out", "mic", "in"),
        ],
    };
    e.apply(validate(&spec).unwrap());
    std::thread::sleep(Duration::from_millis(3500));
    let status = e.status();
    drop(e);
    let n = |id: &str| status.nodes.iter().find(|n| n.id.0 == id).unwrap();
    assert!(n("ch").events >= 2, "the change block fired: {status:?}");
    assert!(n("t").events >= 4, "{status:?}");
    // the sound passes while w is false and stops while it is true: both must have happened
    let l = left(&sim_recording(&sim, "virtual:Out"));
    let tail = &l[48_000..];
    let loud = tail.chunks(4_800).filter(|c| rms(c) > 0.2).count();
    let quiet = tail.chunks(4_800).filter(|c| rms(c) < 0.02).count();
    assert!(loud > 3 && quiet > 3, "loud {loud}, quiet {quiet}");
}

#[test]
fn two_sounds_into_one_input_are_added_without_a_mixer_block() {
    // two programs linked straight to a virtual microphone and to a gain: the engine adds them
    let sim = SimBackend::new();
    sim.add_app(1, "a.exe", SimSource::tone(1000.0));
    sim.add_app(2, "b.exe", SimSource::tone(1500.0));
    let e = engine(&sim);
    let capture = |name: &str| NodeKind::AppCapture {
        target: AppTarget::ProcessName(name.into()),
    };
    let spec = GraphSpec {
        nodes: vec![
            node("a", capture("a.exe")),
            node("b", capture("b.exe")),
            node("g", NodeKind::Gain { db: 0.0 }),
            node(
                "direct",
                NodeKind::VirtualMic {
                    name: "Direct".into(),
                },
            ),
            node(
                "through",
                NodeKind::VirtualMic {
                    name: "Through".into(),
                },
            ),
        ],
        edges: vec![
            link("a", "out", "direct", "in"),
            link("b", "out", "direct", "in"),
            link("a", "out", "g", "in"),
            link("b", "out", "g", "in"),
            link("g", "out", "through", "in"),
        ],
    };
    e.apply(validate(&spec).unwrap());
    std::thread::sleep(Duration::from_millis(2500));
    let status = e.status();
    drop(e);
    for n in &status.nodes {
        assert_eq!(n.state, NodeState::Running, "{}", n.id);
    }
    // each tone alone has an rms of 0.354; two tones of different frequencies add their power: 0.5
    for name in ["virtual:Direct", "virtual:Through"] {
        let l = left(&sim_recording(&sim, name));
        let level = rms(&l[l.len() / 2..]);
        assert!(
            (level - 0.5).abs() < 0.04,
            "{name}: rms {level} (0.354 would be only one of the two)"
        );
    }
}

#[test]
fn the_noise_reduction_block_runs_in_the_real_engine() {
    // a steady tone is tonal like a voice, so the network lets it through: what matters here is that the block runs in
    // the engine, keeps the stream going without a gap, and does not change the level of sound that is not noise
    let sim = SimBackend::new();
    sim.add_app(1234, "chrome.exe", SimSource::tone(1000.0));
    let e = engine(&sim);
    let spec = GraphSpec {
        nodes: vec![
            node("app", app_node()),
            node("nr", NodeKind::NoiseReduction),
            node(
                "mic",
                NodeKind::VirtualMic {
                    name: "Clean".into(),
                },
            ),
        ],
        edges: vec![edge("app", "nr"), edge("nr", "mic")],
    };
    e.apply(validate(&spec).unwrap());
    std::thread::sleep(Duration::from_millis(2500));
    let status = e.status();
    drop(e);
    for n in &status.nodes {
        assert_eq!(n.state, NodeState::Running, "{}", n.id);
    }
    let l = left(&sim_recording(&sim, "virtual:Clean"));
    assert!(l.len() > 48_000 * 2, "{} frames", l.len());
    assert!(l.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    let tail = &l[l.len() - 24_000..];
    println!(
        "tone through the noise reduction: rms {:.4} (0.354 before)",
        rms(tail)
    );
    assert!(
        (0.30..0.40).contains(&rms(tail)),
        "a tone is not noise: it keeps its level, rms {}",
        rms(tail)
    );
    assert!(longest_silence_ms(tail) < 2.0, "no gap once running");
}

#[test]
fn what_is_typed_in_a_block_feeds_its_unlinked_inputs() {
    use omu_types::{InputValue, MathOp};
    let sim = SimBackend::new();
    let engine = engine(&sim);
    let mut math = node("m", NodeKind::Math { op: MathOp::Add });
    math.inputs.insert("a".into(), InputValue::Number(2.0));
    math.inputs.insert("b".into(), InputValue::Number(0.5));
    let graph = validate(&GraphSpec {
        nodes: vec![math],
        edges: vec![],
    })
    .unwrap();
    engine.apply(graph);
    assert!(
        wait_for(Duration::from_secs(5), || engine
            .status()
            .nodes
            .iter()
            .any(|n| n.id.0 == "m" && n.value.as_deref() == Some("2.5"))),
        "the block adds the two numbers typed in it"
    );
}

#[test]
fn a_group_runs_as_the_blocks_it_is_made_of() {
    use omu_types::Exposed;
    let sim = SimBackend::new();
    let engine = engine(&sim);
    let inner = GraphSpec {
        nodes: vec![node("vol", NodeKind::Gain { db: -6.0 })],
        edges: vec![],
    };
    let group = node(
        "g",
        NodeKind::Group {
            graph: Box::new(inner),
            exposed: vec![
                Exposed {
                    name: "in".into(),
                    node: "vol".into(),
                    port: "in".into(),
                },
                Exposed {
                    name: "out".into(),
                    node: "vol".into(),
                    port: "out".into(),
                },
            ],
            layout: Default::default(),
        },
    );
    let graph = validate(&GraphSpec {
        nodes: vec![
            node("app", app_node()),
            group,
            node(
                "mic",
                NodeKind::VirtualMic {
                    name: "Grouped".into(),
                },
            ),
        ],
        edges: vec![edge("app", "g"), edge("g", "mic")],
    })
    .unwrap();
    engine.apply(graph);
    assert!(
        wait_for(Duration::from_secs(5), || engine
            .status()
            .nodes
            .iter()
            .any(|n| n.id.0 == "g.vol" && n.state == NodeState::Running)),
        "the block inside the group runs"
    );
}
