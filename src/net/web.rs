//! Sound through the web: WebRTC, the way browsers and many streaming programs send and receive sound in real time.
//!
//! Two kinds of blocks of the graph use it, each under a name:
//!
//! * a **web input** receives the sound a browser (a phone's microphone, the page `/remote`) or a program (OBS, FFmpeg,
//!   GStreamer) sends; the sender connects with **WHIP** (an HTTP request that carries the WebRTC offer);
//! * a **web output** sends the sound of the graph to whoever listens; a listener connects with **WHEP**.
//!
//! The [`Hub`] holds those names. The HTTP side of the daemon hands the offers to it ([`Hub::whip`], [`Hub::whep`]); the blocks
//! open their name in it ([`Hub::open_ingest`], [`Hub::open_egress`]) and get an ordinary capture or render stream, which the
//! engine runs like any other. A name only exists while the graph that has the block is running.
//!
//! WebRTC itself is `str0m`, which does no input or output of its own: this module owns the UDP sockets and the clock, one
//! thread per connection. The sound is Opus ([`crate::opus`]), encrypted by DTLS-SRTP like every WebRTC call.

use crate::opus;
use omu_hal::{
    CaptureCallback, CaptureStream, HalError, RenderCallback, RenderStream, Running, StreamHealth,
};
use omu_types::AudioFormat;
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard, Once};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use str0m::change::SdpOffer;
use str0m::format::Codec;
use str0m::media::{Frequency, MediaTime, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc, RtcConfig};

/// The longest name of a web input or output.
pub const MAX_NAME: usize = omu_types::MAX_WEB_NAME;
/// How much received sound is kept for the graph to take (more is thrown away, oldest first): one second.
const MAX_QUEUE: usize = opus::RATE as usize * opus::CHANNELS;

pub use omu_types::is_web_name as valid_name;

#[derive(Debug, thiserror::Error)]
pub enum WebError {
    #[error("no block is waiting for a sound named {0:?}: start the graph with a web block of that name")]
    NoEndpoint(String),
    #[error("the connection request cannot be used: {0}")]
    BadOffer(String),
    #[error("{0}")]
    Internal(String),
}

/// The reply to a connection request: the answer to send back, and the connection's identifier (to end it later).
#[derive(Debug, Clone)]
pub struct Answer {
    pub sdp: String,
    pub session: String,
}

/// What a block of the graph holds a name for.
struct Ingest {
    /// The sound received and not taken yet (interleaved stereo, 48 kHz).
    queue: Mutex<VecDeque<f32>>,
    sessions: AtomicUsize,
    stop: AtomicBool,
}

/// A connection that listens: its identifier, and where to send it each packet of sound.
type Listener = (String, mpsc::SyncSender<Arc<Vec<u8>>>);

struct Egress {
    /// The connections listening.
    listeners: Mutex<Vec<Listener>>,
    stop: AtomicBool,
}

#[derive(Default)]
struct Registry {
    ingests: HashMap<String, Arc<Ingest>>,
    egresses: HashMap<String, Arc<Egress>>,
    /// The connections that are open, by identifier: setting the flag ends one.
    sessions: HashMap<String, Arc<AtomicBool>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The names of the web blocks and the connections to them. Cheap to clone (it is a handle).
#[derive(Clone, Default)]
pub struct Hub {
    registry: Arc<Mutex<Registry>>,
    counter: Arc<AtomicU64>,
}

/// What the panel can show about one name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointInfo {
    pub name: String,
    /// true: sound comes in (a web input); false: sound goes out (a web output).
    pub receiving: bool,
    pub connections: usize,
}

impl Hub {
    pub fn new() -> Hub {
        Hub::default()
    }

    /// The names that are open now.
    pub fn endpoints(&self) -> Vec<EndpointInfo> {
        let reg = lock(&self.registry);
        let mut out: Vec<EndpointInfo> = reg
            .ingests
            .iter()
            .map(|(name, i)| EndpointInfo {
                name: name.clone(),
                receiving: true,
                connections: i.sessions.load(Ordering::Relaxed),
            })
            .chain(reg.egresses.iter().map(|(name, e)| EndpointInfo {
                name: name.clone(),
                receiving: false,
                connections: lock(&e.listeners).len(),
            }))
            .collect();
        out.sort_by(|a, b| {
            (a.receiving, &a.name)
                .cmp(&(b.receiving, &b.name))
                .reverse()
        });
        out
    }

    /// Opens `name` for a web input: the sound that is sent to it comes out of the capture stream.
    pub fn open_ingest(&self, name: &str) -> Result<WebCapture, HalError> {
        if !valid_name(name) {
            return Err(HalError::Unsupported(format!(
                "{name:?} cannot be a web name: use 1 to {MAX_NAME} letters, digits, - or _"
            )));
        }
        let ingest = Arc::new(Ingest {
            queue: Mutex::default(),
            sessions: AtomicUsize::new(0),
            stop: AtomicBool::new(false),
        });
        let mut reg = lock(&self.registry);
        if reg.ingests.contains_key(name) {
            return Err(HalError::Busy(format!(
                "another web input already uses the name {name:?}"
            )));
        }
        reg.ingests.insert(name.to_owned(), ingest.clone());
        Ok(WebCapture {
            owner: Owner {
                hub: self.clone(),
                name: name.to_owned(),
                receiving: true,
            },
            ingest,
            health: StreamHealth::new(),
        })
    }

    /// Opens `name` for a web output: what the render stream is given is sent to the listeners.
    pub fn open_egress(&self, name: &str) -> Result<WebRender, HalError> {
        if !valid_name(name) {
            return Err(HalError::Unsupported(format!(
                "{name:?} cannot be a web name: use 1 to {MAX_NAME} letters, digits, - or _"
            )));
        }
        let egress = Arc::new(Egress {
            listeners: Mutex::default(),
            stop: AtomicBool::new(false),
        });
        let mut reg = lock(&self.registry);
        if reg.egresses.contains_key(name) {
            return Err(HalError::Busy(format!(
                "another web output already uses the name {name:?}"
            )));
        }
        reg.egresses.insert(name.to_owned(), egress.clone());
        Ok(WebRender {
            owner: Owner {
                hub: self.clone(),
                name: name.to_owned(),
                receiving: false,
            },
            egress,
            health: StreamHealth::new(),
        })
    }

    /// A sender (a browser, a program) connects to the web input `name`: WHIP.
    pub fn whip(&self, name: &str, offer_sdp: &str) -> Result<Answer, WebError> {
        let ingest = lock(&self.registry)
            .ingests
            .get(name)
            .cloned()
            .ok_or_else(|| WebError::NoEndpoint(name.to_owned()))?;
        ingest.sessions.fetch_add(1, Ordering::Relaxed);
        let result = self.start_session(offer_sdp, Role::Ingest(ingest.clone()));
        if result.is_err() {
            ingest.sessions.fetch_sub(1, Ordering::Relaxed);
        }
        result
    }

    /// A listener connects to the web output `name`: WHEP.
    pub fn whep(&self, name: &str, offer_sdp: &str) -> Result<Answer, WebError> {
        let egress = lock(&self.registry)
            .egresses
            .get(name)
            .cloned()
            .ok_or_else(|| WebError::NoEndpoint(name.to_owned()))?;
        // a few packets of room: a listener that falls behind loses sound instead of holding up the graph
        let (tx, rx) = mpsc::sync_channel(16);
        let result = self.start_session(offer_sdp, Role::Egress(egress.clone(), rx));
        if let Ok(answer) = &result {
            lock(&egress.listeners).push((answer.session.clone(), tx));
        }
        result
    }

    /// Ends a connection (the request that ends it, or a block that stops).
    pub fn close_session(&self, session: &str) {
        if let Some(flag) = lock(&self.registry).sessions.get(session) {
            flag.store(true, Ordering::Relaxed);
        }
    }

    fn start_session(&self, offer_sdp: &str, role: Role) -> Result<Answer, WebError> {
        static CRYPTO: Once = Once::new();
        CRYPTO.call_once(|| str0m::crypto::from_feature_flags().install_process_default());

        let offer =
            SdpOffer::from_sdp_string(offer_sdp).map_err(|e| WebError::BadOffer(e.to_string()))?;
        // only Opus: it is what browsers send and play for sound
        let mut rtc = RtcConfig::new()
            .clear_codecs()
            .enable_opus(true, false)
            .build(Instant::now());
        let socket = UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| WebError::Internal(format!("cannot open a UDP port: {e}")))?;
        let port = socket
            .local_addr()
            .map_err(|e| WebError::Internal(e.to_string()))?
            .port();
        let locals = local_addresses();
        for ip in &locals {
            if let Ok(candidate) = Candidate::host(SocketAddr::new(*ip, port), "udp") {
                let _ = rtc.add_local_candidate(candidate);
            }
        }
        let answer = rtc
            .sdp_api()
            .accept_offer(offer)
            .map_err(|e| WebError::BadOffer(e.to_string()))?;

        // an identifier nobody can guess: the random keys the standard library draws for every program, hashed with a counter
        let session = {
            use std::hash::{BuildHasher, Hasher};
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_u64(self.counter.fetch_add(1, Ordering::Relaxed));
            format!("{:016x}", hasher.finish())
        };
        let stop = Arc::new(AtomicBool::new(false));
        lock(&self.registry)
            .sessions
            .insert(session.clone(), stop.clone());
        let hub = self.clone();
        let id = session.clone();
        std::thread::Builder::new()
            .name("web-connection".into())
            .spawn(move || {
                let mut link = Link {
                    rtc,
                    socket,
                    port,
                    locals,
                };
                drive(&mut link, &role, &stop);
                match &role {
                    Role::Ingest(ingest) => {
                        ingest.sessions.fetch_sub(1, Ordering::Relaxed);
                    }
                    Role::Egress(egress, _) => lock(&egress.listeners).retain(|(s, _)| *s != id),
                }
                lock(&hub.registry).sessions.remove(&id);
            })
            .map_err(|e| WebError::Internal(format!("cannot start a connection: {e}")))?;
        Ok(Answer {
            sdp: answer.to_sdp_string(),
            session,
        })
    }
}

/// The addresses of this computer a peer can reach: its networks (and loopback, for a browser on the same computer).
fn local_addresses() -> Vec<IpAddr> {
    let mut out: Vec<IpAddr> = if_addrs::get_if_addrs()
        .map(|all| {
            all.into_iter()
                .map(|i| i.ip())
                .filter(|ip| ip.is_ipv4())
                .filter(|ip| !matches!(ip, IpAddr::V4(v4) if v4.is_link_local()))
                .collect()
        })
        .unwrap_or_default();
    if !out.contains(&IpAddr::from([127, 0, 0, 1])) {
        out.push(IpAddr::from([127, 0, 0, 1]));
    }
    out
}

/// Which of our addresses a packet came to, as far as a single socket can tell: the one on the sender's network.
fn destination(source: SocketAddr, locals: &[IpAddr], port: u16) -> SocketAddr {
    let near = |a: &IpAddr| match (a, source.ip()) {
        (IpAddr::V4(a), IpAddr::V4(s)) => a.octets()[..3] == s.octets()[..3],
        _ => false,
    };
    let ip = if source.ip().is_loopback() {
        IpAddr::from([127, 0, 0, 1])
    } else {
        locals
            .iter()
            .find(|a| near(a))
            .or_else(|| locals.iter().find(|a| !a.is_loopback()))
            .copied()
            .unwrap_or_else(|| IpAddr::from([127, 0, 0, 1]))
    };
    SocketAddr::new(ip, port)
}

enum Role {
    Ingest(Arc<Ingest>),
    Egress(Arc<Egress>, mpsc::Receiver<Arc<Vec<u8>>>),
}

struct Link {
    rtc: Rtc,
    socket: UdpSocket,
    port: u16,
    locals: Vec<IpAddr>,
}

/// Runs one connection until it ends: the sockets, the clock and the media, for `str0m`.
fn drive(link: &mut Link, role: &Role, stop: &AtomicBool) {
    let mut decoder = match role {
        Role::Ingest(_) => opus::Decoder::new().ok(),
        Role::Egress(..) => None,
    };
    let mut mid: Option<Mid> = None;
    let mut rtp_time: u64 = 0;
    let mut buf = vec![0u8; 2048];
    loop {
        let ended = stop.load(Ordering::Relaxed)
            || match role {
                Role::Ingest(i) => i.stop.load(Ordering::Relaxed),
                Role::Egress(e, _) => e.stop.load(Ordering::Relaxed),
            };
        if ended {
            link.rtc.disconnect();
        }
        // everything the connection has to say, until it only waits
        let timeout = loop {
            match link.rtc.poll_output() {
                Ok(Output::Timeout(t)) => break t,
                Ok(Output::Transmit(t)) => {
                    let _ = link.socket.send_to(&t.contents, t.destination);
                }
                Ok(Output::Event(event)) => match event {
                    Event::IceConnectionStateChange(IceConnectionState::Disconnected) => return,
                    Event::MediaAdded(added) => mid = Some(added.mid),
                    Event::MediaData(data) => {
                        if let (Role::Ingest(ingest), Some(decoder)) = (role, decoder.as_mut()) {
                            if data.params.spec().codec == Codec::Opus && !data.data.is_empty() {
                                if let Ok(samples) = decoder.decode(&data.data) {
                                    let mut queue = lock(&ingest.queue);
                                    queue.extend(samples.iter().copied());
                                    let extra = queue.len().saturating_sub(MAX_QUEUE);
                                    queue.drain(..extra);
                                }
                            }
                        }
                    }
                    _ => {}
                },
                Err(_) => return,
            }
        };
        if !link.rtc.is_alive() {
            return;
        }
        // sound to send, as it comes from the graph
        if let (Role::Egress(_, frames), Some(mid)) = (role, mid) {
            while let Ok(frame) = frames.try_recv() {
                let Some(writer) = link.rtc.writer(mid) else {
                    break;
                };
                let Some(pt) = writer
                    .payload_params()
                    .find(|p| p.spec().codec == Codec::Opus)
                    .map(|p| p.pt())
                else {
                    break;
                };
                let time = MediaTime::new(rtp_time, Frequency::FORTY_EIGHT_KHZ);
                rtp_time += opus::FRAME as u64;
                // before the connection is complete the packet is dropped, which is what is wanted
                let _ = writer.write(pt, Instant::now(), time, frame.as_ref().clone());
            }
        }
        // wait for a packet, but not for long: sound to send arrives every 20 ms
        let wait = timeout
            .saturating_duration_since(Instant::now())
            .clamp(Duration::from_millis(1), Duration::from_millis(10));
        if link.socket.set_read_timeout(Some(wait)).is_err() {
            return;
        }
        let input = match link.socket.recv_from(&mut buf) {
            Ok((n, source)) => {
                let Ok(contents) = (&buf[..n]).try_into() else {
                    continue;
                };
                Input::Receive(
                    Instant::now(),
                    Receive {
                        proto: Protocol::Udp,
                        source,
                        destination: destination(source, &link.locals, link.port),
                        contents,
                    },
                )
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Input::Timeout(Instant::now())
            }
            // a UDP socket on Windows reports an earlier refused packet as an error of the next read: not the end
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => continue,
            Err(_) => return,
        };
        if link.rtc.handle_input(input).is_err() {
            return;
        }
    }
}

/// A name held in the hub: given back (and the connections to it ended) when this is dropped.
struct Owner {
    hub: Hub,
    name: String,
    receiving: bool,
}

impl Drop for Owner {
    fn drop(&mut self) {
        let mut reg = lock(&self.hub.registry);
        if self.receiving {
            if let Some(i) = reg.ingests.remove(&self.name) {
                i.stop.store(true, Ordering::Relaxed);
            }
        } else if let Some(e) = reg.egresses.remove(&self.name) {
            e.stop.store(true, Ordering::Relaxed);
        }
    }
}

struct Guard {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    _owner: Owner,
}

impl Running for Guard {}

impl Drop for Guard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The sound a web input receives, as a capture stream.
pub struct WebCapture {
    owner: Owner,
    ingest: Arc<Ingest>,
    health: Arc<StreamHealth>,
}

impl CaptureStream for WebCapture {
    fn format(&self) -> AudioFormat {
        AudioFormat::ENGINE
    }

    fn health(&self) -> Arc<StreamHealth> {
        self.health.clone()
    }

    fn start(self: Box<Self>, mut callback: CaptureCallback) -> Result<Box<dyn Running>, HalError> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let WebCapture {
            owner,
            ingest,
            health,
        } = *self;
        let thread = std::thread::Builder::new()
            .name("web-in".into())
            .spawn(move || {
                let mut chunk: Vec<f32> = Vec::new();
                while !flag.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(10));
                    chunk.clear();
                    chunk.extend(lock(&ingest.queue).drain(..));
                    if !chunk.is_empty() {
                        health.add_frames((chunk.len() / opus::CHANNELS) as u64);
                        callback(&chunk);
                    }
                }
            })
            .map_err(|e| HalError::Backend(format!("cannot start the web input: {e}")))?;
        Ok(Box::new(Guard {
            stop,
            thread: Some(thread),
            _owner: owner,
        }))
    }
}

/// The sound a web output sends, as a render stream.
pub struct WebRender {
    owner: Owner,
    egress: Arc<Egress>,
    health: Arc<StreamHealth>,
}

impl RenderStream for WebRender {
    fn format(&self) -> AudioFormat {
        AudioFormat::ENGINE
    }

    fn health(&self) -> Arc<StreamHealth> {
        self.health.clone()
    }

    fn start(self: Box<Self>, mut callback: RenderCallback) -> Result<Box<dyn Running>, HalError> {
        let mut encoder = opus::Encoder::new().map_err(|e| HalError::Backend(e.to_string()))?;
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let WebRender {
            owner,
            egress,
            health,
        } = *self;
        let thread = std::thread::Builder::new()
            .name("web-out".into())
            .spawn(move || {
                let mut frame = vec![0.0f32; opus::FRAME * opus::CHANNELS];
                let mut next = Instant::now();
                while !flag.load(Ordering::Relaxed) {
                    // the graph is asked for sound on the clock, listeners or not, as a speaker would
                    callback(&mut frame);
                    health.add_frames(opus::FRAME as u64);
                    let listening = !lock(&egress.listeners).is_empty();
                    if listening {
                        if let Ok(packet) = encoder.encode(&frame) {
                            let packet = Arc::new(packet.to_vec());
                            lock(&egress.listeners).retain(|(_, tx)| {
                                !matches!(
                                    tx.try_send(packet.clone()),
                                    Err(mpsc::TrySendError::Disconnected(_))
                                )
                            });
                        }
                    }
                    next += Duration::from_millis(20);
                    let now = Instant::now();
                    if next > now {
                        std::thread::sleep(next - now);
                    } else if now - next > Duration::from_millis(200) {
                        // far behind (the computer slept): start again from now instead of sending a burst
                        next = now;
                    }
                }
            })
            .map_err(|e| HalError::Backend(format!("cannot start the web output: {e}")))?;
        Ok(Box::new(Guard {
            stop,
            thread: Some(thread),
            _owner: owner,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_what_fits_in_an_address() {
        assert!(valid_name("phone") && valid_name("mic-2_b") && valid_name(&"a".repeat(MAX_NAME)));
        assert!(
            !valid_name("")
                && !valid_name("a b")
                && !valid_name("a/b")
                && !valid_name(&"a".repeat(MAX_NAME + 1))
        );
    }

    #[test]
    fn a_name_is_held_by_one_block_and_given_back() {
        let hub = Hub::new();
        let first = hub.open_ingest("phone").unwrap();
        assert!(matches!(hub.open_ingest("phone"), Err(HalError::Busy(_))));
        // an input and an output may share a name: they are different things
        let out = hub.open_egress("phone").unwrap();
        assert!(matches!(hub.open_egress("phone"), Err(HalError::Busy(_))));
        assert_eq!(hub.endpoints().len(), 2);
        drop(first);
        drop(out);
        assert!(hub.endpoints().is_empty());
        assert!(hub.open_ingest("phone").is_ok(), "free again");
        assert!(matches!(
            hub.open_ingest("not a name"),
            Err(HalError::Unsupported(_))
        ));
    }

    #[test]
    fn a_connection_needs_a_block_and_a_real_request() {
        let hub = Hub::new();
        assert!(matches!(
            hub.whip("nobody", "v=0"),
            Err(WebError::NoEndpoint(_))
        ));
        assert!(matches!(
            hub.whep("nobody", "v=0"),
            Err(WebError::NoEndpoint(_))
        ));
        let _input = hub.open_ingest("mic").unwrap();
        let err = hub.whip("mic", "this is not an SDP").unwrap_err();
        assert!(matches!(err, WebError::BadOffer(_)), "{err}");
        assert_eq!(
            hub.endpoints()[0].connections,
            0,
            "a refused request leaves no connection"
        );
    }

    #[test]
    fn the_address_a_packet_came_to_is_the_one_on_the_senders_network() {
        let locals = [
            IpAddr::from([192, 168, 1, 20]),
            IpAddr::from([10, 0, 0, 5]),
            IpAddr::from([127, 0, 0, 1]),
        ];
        let to = |ip: [u8; 4]| destination(SocketAddr::new(IpAddr::from(ip), 5000), &locals, 7000);
        assert_eq!(to([10, 0, 0, 77]), "10.0.0.5:7000".parse().unwrap());
        assert_eq!(to([192, 168, 1, 99]), "192.168.1.20:7000".parse().unwrap());
        assert_eq!(to([127, 0, 0, 1]), "127.0.0.1:7000".parse().unwrap());
        assert_eq!(
            to([8, 8, 8, 8]),
            "192.168.1.20:7000".parse().unwrap(),
            "no network in common: the first real one"
        );
    }
}
