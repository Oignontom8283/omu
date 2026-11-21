//! Sound through WebRTC, with a real peer: a second `str0m` stands in for the browser, on this computer. It connects the way a
//! browser or OBS does (an offer in, an answer back), then sends a tone to a web input, or listens to a web output.

use omu_hal::{CaptureStream, RenderStream};
use omu_net::opus;
use omu_net::web::Hub;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use str0m::change::{SdpAnswer, SdpPendingOffer};
use str0m::format::Codec;
use str0m::media::{Direction, Frequency, MediaKind, MediaTime, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc, RtcConfig};

const TONE: f32 = 440.0;
const LEVEL: f32 = 0.5;

/// 20 ms of the tone, stereo, from frame number `n`.
fn tone(n: usize) -> Vec<f32> {
    (0..opus::FRAME)
        .flat_map(|i| {
            let t = (n * opus::FRAME + i) as f32 / opus::RATE as f32;
            let s = LEVEL * (2.0 * std::f32::consts::PI * TONE * t).sin();
            [s, s]
        })
        .collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// What a browser is for these tests: an offer to send, and a way to run the connection once the answer is known.
struct Browser {
    rtc: Rtc,
    socket: UdpSocket,
    mid: Mid,
    pending: Option<SdpPendingOffer>,
    offer: String,
}

fn browser(direction: Direction) -> Browser {
    crate_crypto();
    let mut rtc = RtcConfig::new()
        .clear_codecs()
        .enable_opus(true, false)
        .build(Instant::now());
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = socket.local_addr().unwrap();
    rtc.add_local_candidate(Candidate::host(addr, "udp").unwrap())
        .unwrap();
    let mut api = rtc.sdp_api();
    let mid = api.add_media(MediaKind::Audio, direction, None, None, None);
    let (offer, pending) = api.apply().expect("an offer");
    Browser {
        rtc,
        socket,
        mid,
        pending: Some(pending),
        offer: offer.to_sdp_string(),
    }
}

fn crate_crypto() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| str0m::crypto::from_feature_flags().install_process_default());
}

impl Browser {
    /// Runs the connection on a thread until `stop`: sends the tone when `send` is set, and keeps what it hears in `heard`.
    fn run(
        mut self,
        answer_sdp: &str,
        send: bool,
        stop: Arc<AtomicBool>,
        heard: Arc<Mutex<Vec<f32>>>,
    ) -> std::thread::JoinHandle<()> {
        let answer = SdpAnswer::from_sdp_string(answer_sdp).expect("the answer is an SDP");
        self.rtc
            .sdp_api()
            .accept_answer(self.pending.take().unwrap(), answer)
            .expect("the answer is accepted");
        std::thread::spawn(move || {
            let mut encoder = opus::Encoder::new().unwrap();
            let mut decoder = opus::Decoder::new().unwrap();
            let mut connected = false;
            let mut frame = 0usize;
            let mut next = Instant::now();
            let mut buf = vec![0u8; 2048];
            while !stop.load(Ordering::Relaxed) {
                let timeout = loop {
                    match self.rtc.poll_output().unwrap() {
                        Output::Timeout(t) => break t,
                        Output::Transmit(t) => {
                            let _ = self.socket.send_to(&t.contents, t.destination);
                        }
                        Output::Event(Event::Connected) => connected = true,
                        Output::Event(Event::IceConnectionStateChange(
                            IceConnectionState::Disconnected,
                        )) => return,
                        Output::Event(Event::MediaData(d)) => {
                            if d.params.spec().codec == Codec::Opus {
                                if let Ok(samples) = decoder.decode(&d.data) {
                                    heard.lock().unwrap().extend_from_slice(samples);
                                }
                            }
                        }
                        Output::Event(_) => {}
                    }
                };
                if send && connected && Instant::now() >= next {
                    let packet = encoder.encode(&tone(frame)).unwrap().to_vec();
                    let writer = self.rtc.writer(self.mid).expect("the media is there");
                    let pt = writer
                        .payload_params()
                        .find(|p| p.spec().codec == Codec::Opus)
                        .map(|p| p.pt())
                        .unwrap();
                    let time =
                        MediaTime::new((frame * opus::FRAME) as u64, Frequency::FORTY_EIGHT_KHZ);
                    writer.write(pt, Instant::now(), time, packet).unwrap();
                    frame += 1;
                    next += Duration::from_millis(20);
                }
                let wait = timeout
                    .saturating_duration_since(Instant::now())
                    .clamp(Duration::from_millis(1), Duration::from_millis(5));
                self.socket.set_read_timeout(Some(wait)).unwrap();
                let input = match self.socket.recv_from(&mut buf) {
                    Ok((n, source)) => Input::Receive(
                        Instant::now(),
                        Receive {
                            proto: Protocol::Udp,
                            source,
                            destination: self.socket.local_addr().unwrap(),
                            contents: (&buf[..n]).try_into().unwrap(),
                        },
                    ),
                    Err(_) => Input::Timeout(Instant::now()),
                };
                if self.rtc.handle_input(input).is_err() {
                    return;
                }
            }
        })
    }
}

fn wait_until(what: &str, seconds: u64, mut done: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < end {
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("gave up waiting for {what}");
}

#[test]
fn a_browser_sends_a_tone_to_a_web_input_and_the_graph_hears_it() {
    let hub = Hub::new();
    let capture = hub.open_ingest("phone").unwrap();
    let heard = Arc::new(Mutex::new(Vec::<f32>::new()));
    let sink = heard.clone();
    let running = Box::new(capture)
        .start(Box::new(move |samples: &[f32]| {
            sink.lock().unwrap().extend_from_slice(samples)
        }))
        .unwrap();

    let peer = browser(Direction::SendOnly);
    let answer = hub
        .whip("phone", &peer.offer)
        .expect("the connection is accepted");
    assert!(answer.sdp.contains("m=audio"), "{}", answer.sdp);
    let stop = Arc::new(AtomicBool::new(false));
    let handle = peer.run(&answer.sdp, true, stop.clone(), Arc::default());

    // one and a half seconds of sound arrive (the start of the call is not counted)
    wait_until("the tone to arrive", 20, || {
        heard.lock().unwrap().len() >= 3 * opus::RATE as usize * opus::CHANNELS / 2
    });
    assert_eq!(hub.endpoints()[0].connections, 1);
    let all = heard.lock().unwrap().clone();
    let level = rms(&all[all.len() - opus::RATE as usize * opus::CHANNELS..]);
    assert!(
        (level - LEVEL / 2f32.sqrt()).abs() < 0.05,
        "the tone keeps its level through the call: rms {level}"
    );

    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    hub.close_session(&answer.session);
    drop(running);
    assert!(
        hub.endpoints().is_empty(),
        "the name is free when the block stops"
    );
}

#[test]
fn a_browser_listens_to_a_web_output_and_hears_the_graph() {
    let hub = Hub::new();
    let render = hub.open_egress("speaker").unwrap();
    // the graph plays a tone
    let mut frame = 0usize;
    let running = Box::new(render)
        .start(Box::new(move |out: &mut [f32]| {
            out.copy_from_slice(&tone(frame));
            frame += 1;
        }))
        .unwrap();

    let peer = browser(Direction::RecvOnly);
    let answer = hub
        .whep("speaker", &peer.offer)
        .expect("the connection is accepted");
    let stop = Arc::new(AtomicBool::new(false));
    let heard = Arc::new(Mutex::new(Vec::<f32>::new()));
    let handle = peer.run(&answer.sdp, false, stop.clone(), heard.clone());

    wait_until("the tone to be heard", 20, || {
        heard.lock().unwrap().len() >= 3 * opus::RATE as usize * opus::CHANNELS / 2
    });
    assert_eq!(hub.endpoints()[0].connections, 1);
    let all = heard.lock().unwrap().clone();
    let level = rms(&all[all.len() - opus::RATE as usize * opus::CHANNELS..]);
    assert!(
        (level - LEVEL / 2f32.sqrt()).abs() < 0.05,
        "the listener hears the tone at its level: rms {level}"
    );

    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    drop(running);
    assert!(hub.endpoints().is_empty());
}
