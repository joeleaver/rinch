//! The interop target (design record section 4): a str0m 0.24 peer shaped
//! like Playweft's hosted game server. ICE-lite, one UDP port, a host
//! candidate only, never told the client's candidates. It echoes every
//! message and counts what it received, so a test's positive control is the
//! server's own count, not the client's callbacks.
//!
//! Shared by the native tests (`tests/common/mod.rs`) and the
//! `examples/lite_echo.rs` server the browser tests talk to.
#![allow(dead_code)]

use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

const T: Duration = Duration::from_secs(15);

/// What the lite peer has seen.
#[derive(Debug, Default, Clone)]
pub struct LiteStats {
    /// Labels of channels that opened (negotiated or remote-opened).
    pub opened: Vec<String>,
    /// Each opened channel's settings as str0m holds them: `ordered`, and
    /// the reliability (`Reliable`, `MaxRetransmits { .. }`, ...) as text.
    /// For a channel the client opened in-band these came over DCEP.
    pub configs: HashMap<String, (bool, String)>,
    /// Bytes received per label.
    pub received: HashMap<String, usize>,
    /// Messages received per label.
    pub messages: HashMap<String, usize>,
    /// Channels that closed.
    pub closed: Vec<String>,
    /// Offers answered (1, then 2 after an ICE restart).
    pub offers: usize,
}

enum LiteCmd {
    Offer(String, mpsc::Sender<Result<String, String>>),
    /// Open an in-band channel from the server side.
    OpenInBand(String),
    /// Close the channel with this label.
    Close(String),
}

/// A str0m ICE-lite echo peer on its own thread and UDP port.
pub struct LitePeer {
    pub addr: SocketAddr,
    cmds: mpsc::Sender<LiteCmd>,
    stats: Arc<Mutex<LiteStats>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl LitePeer {
    /// Spawn one with pre-negotiated `control` (0, reliable) and `tick` (1,
    /// unordered, 0 retransmits), Playweft's layout.
    pub fn spawn() -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = socket.local_addr().unwrap();
        Self::spawn_on(socket, addr)
    }

    /// Spawn one on `socket`, advertising `advertise` as its one host
    /// candidate (a socket bound to `0.0.0.0` has no address to advertise).
    pub fn spawn_on(socket: UdpSocket, advertise: SocketAddr) -> Self {
        static CRYPTO: OnceLock<()> = OnceLock::new();
        CRYPTO.get_or_init(|| str0m::crypto::from_feature_flags().install_process_default());
        let addr = advertise;
        let (tx, rx) = mpsc::channel();
        let stats = Arc::new(Mutex::new(LiteStats::default()));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (s, st) = (stats.clone(), stop.clone());
        let thread = thread::Builder::new()
            .name("lite-peer".into())
            .spawn(move || lite_loop(socket, advertise, rx, s, st))
            .unwrap();
        LitePeer {
            addr,
            cmds: tx,
            stats,
            stop,
            thread: Some(thread),
        }
    }

    /// Answer an offer (the first, or an ICE restart's).
    pub fn answer(&self, offer: &str) -> String {
        let (tx, rx) = mpsc::channel();
        self.cmds
            .send(LiteCmd::Offer(offer.to_string(), tx))
            .unwrap();
        rx.recv_timeout(T)
            .unwrap()
            .expect("the lite peer accepts the offer")
    }

    pub fn open_in_band(&self, label: &str) {
        self.cmds.send(LiteCmd::OpenInBand(label.into())).unwrap();
    }

    pub fn close_channel(&self, label: &str) {
        self.cmds.send(LiteCmd::Close(label.into())).unwrap();
    }

    pub fn stats(&self) -> LiteStats {
        self.stats.lock().unwrap().clone()
    }

    /// Stop answering anything: the network goes silent for the client.
    pub fn stop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            t.join().unwrap();
        }
    }
}

impl Drop for LitePeer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn lite_loop(
    socket: UdpSocket,
    local: SocketAddr,
    cmds: mpsc::Receiver<LiteCmd>,
    stats: Arc<Mutex<LiteStats>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
) {
    use str0m::change::SdpOffer;
    use str0m::channel::{ChannelConfig, ChannelId, Reliability};
    use str0m::net::{Protocol, Receive};
    use str0m::{Candidate, Event, Input, Output, Rtc};

    let mut rtc: Option<Rtc> = None;
    let mut labels: HashMap<ChannelId, String> = HashMap::new();
    let mut buf = vec![0u8; 2000];
    // Channels opened from this side, by id, until str0m says they opened.
    let mut opening: HashMap<ChannelId, String> = HashMap::new();
    let mut backlog: std::collections::VecDeque<(ChannelId, Vec<u8>)> = Default::default();
    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
        while let Ok(cmd) = cmds.try_recv() {
            match cmd {
                LiteCmd::Offer(sdp, reply) => {
                    let offer = match SdpOffer::from_sdp_string(&sdp) {
                        Ok(o) => o,
                        Err(e) => {
                            let _ = reply.send(Err(e.to_string()));
                            continue;
                        }
                    };
                    let first = rtc.is_none();
                    let r = rtc.get_or_insert_with(|| {
                        // str0m's SCTP send buffer defaults to 128 KiB, and a write
                        // larger than the free buffer is never accepted, so by
                        // default it cannot send a message over 128 KiB at all
                        // although it advertises 256 KiB. Raised, so the echo
                        // of the tests' 200 kB message goes out.
                        let mut r = Rtc::builder()
                            .set_ice_lite(true)
                            .set_sctp_max_buffered_amount(1 << 20)
                            .build(Instant::now());
                        r.add_local_candidate(Candidate::host(local, "udp").unwrap())
                            .unwrap();
                        r
                    });
                    match r.sdp_api().accept_offer(offer) {
                        Ok(answer) => {
                            if first {
                                for (id, label, reliability, ordered) in [
                                    (0, "control", Reliability::Reliable, true),
                                    (
                                        1,
                                        "tick",
                                        Reliability::MaxRetransmits { retransmits: 0 },
                                        false,
                                    ),
                                ] {
                                    let cid = r.direct_api().create_data_channel(ChannelConfig {
                                        label: label.into(),
                                        ordered,
                                        reliability,
                                        negotiated: Some(id),
                                        protocol: String::new(),
                                    });
                                    labels.insert(cid, label.into());
                                }
                            }
                            stats.lock().unwrap().offers += 1;
                            let _ = reply.send(Ok(answer.to_sdp_string()));
                        }
                        Err(e) => {
                            let _ = reply.send(Err(e.to_string()));
                        }
                    }
                }
                LiteCmd::OpenInBand(label) => {
                    if let Some(r) = rtc.as_mut() {
                        let cid = r.direct_api().create_data_channel(ChannelConfig {
                            label: label.clone(),
                            ..Default::default()
                        });
                        opening.insert(cid, label.clone());
                        labels.insert(cid, label);
                    }
                }
                LiteCmd::Close(label) => {
                    if let Some(r) = rtc.as_mut()
                        && let Some((&cid, _)) = labels.iter().find(|(_, l)| **l == label)
                    {
                        r.direct_api().close_data_channel(cid);
                    }
                }
            }
        }
        let Some(r) = rtc.as_mut() else {
            thread::sleep(Duration::from_millis(2));
            continue;
        };
        while let Some((cid, data)) = backlog.pop_front() {
            // Written, or the channel is gone: either way, done with it.
            if let Some(Ok(false)) = r.channel(cid).map(|mut ch| ch.write(true, &data)) {
                backlog.push_front((cid, data));
                break;
            }
        }
        let timeout = loop {
            match r.poll_output() {
                Ok(Output::Timeout(t)) => break t,
                Ok(Output::Transmit(t)) => {
                    let _ = socket.send_to(&t.contents, t.destination);
                }
                Ok(Output::Event(Event::ChannelOpen(id, label))) => {
                    opening.remove(&id);
                    labels.insert(id, label.clone());
                    let config = r.channel(id).and_then(|ch| {
                        ch.config()
                            .map(|c| (c.ordered, format!("{:?}", c.reliability)))
                    });
                    let mut st = stats.lock().unwrap();
                    if let Some(c) = config {
                        st.configs.insert(label.clone(), c);
                    }
                    st.opened.push(label);
                }
                Ok(Output::Event(Event::ChannelData(d))) => {
                    let label = labels.get(&d.id).cloned().unwrap_or_default();
                    {
                        let mut s = stats.lock().unwrap();
                        *s.received.entry(label.clone()).or_default() += d.data.len();
                        *s.messages.entry(label).or_default() += 1;
                    }
                    // str0m refuses a write larger than its free send
                    // buffer: queue it and retry rather than drop the echo.
                    backlog.push_back((d.id, d.data));
                }
                Ok(Output::Event(Event::ChannelClose(id))) => {
                    let label = labels.get(&id).cloned().unwrap_or_default();
                    stats.lock().unwrap().closed.push(label);
                }
                Ok(Output::Event(_)) => {}
                Err(_) => break Instant::now() + Duration::from_millis(5),
            }
        };
        let wait = timeout
            .saturating_duration_since(Instant::now())
            .clamp(Duration::from_millis(1), Duration::from_millis(20));
        socket.set_read_timeout(Some(wait)).unwrap();
        let input = match socket.recv_from(&mut buf) {
            Ok((n, source)) => Input::Receive(
                Instant::now(),
                Receive {
                    proto: Protocol::Udp,
                    source,
                    destination: local,
                    contents: match (&buf[..n]).try_into() {
                        Ok(c) => c,
                        Err(_) => continue,
                    },
                },
            ),
            Err(_) => Input::Timeout(Instant::now()),
        };
        let _ = r.handle_input(input);
    }
}
