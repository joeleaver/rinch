//! Shared harness for rinch-rtc's native integration tests.
//!
//! - A stand-in for the rinch runtime, as in rinch-ws's tests: the test
//!   thread registers itself as the main thread, installs a dispatcher that
//!   parks main-thread work in a global queue, and pumps it. Registration is
//!   process-global, so each test *file* has exactly one `#[test]`.
//! - [`LitePeer`]: the interop target, a str0m 0.24 peer shaped like
//!   Playweft's hosted game server: ICE-lite, one UDP port, a host candidate
//!   only, never told the client's candidates. It echoes every message and
//!   counts what it received, so a test's positive control is the server's
//!   own count, not the client's callbacks.
#![allow(dead_code)]
#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashMap;
use std::future::Future;
use std::net::{SocketAddr, UdpSocket};
use std::pin::pin;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::{Duration, Instant};

use rinch_core::{register_main_thread, set_cross_thread_dispatcher};

pub type MainJob = Box<dyn FnOnce() + Send>;

static PUMP: OnceLock<Mutex<Vec<MainJob>>> = OnceLock::new();

fn dispatcher(f: MainJob) {
    PUMP.get().expect("pump").lock().unwrap().push(f);
}

/// Make this thread the main thread, and bind peers to loopback.
pub fn init() {
    PUMP.set(Mutex::new(Vec::new())).ok();
    register_main_thread();
    set_cross_thread_dispatcher(dispatcher);
    rinch_rtc::testing::set_bind_addrs(&["127.0.0.1:0"]);
}

pub fn drain_pump() {
    loop {
        let jobs: Vec<_> = PUMP.get().expect("pump").lock().unwrap().drain(..).collect();
        if jobs.is_empty() {
            return;
        }
        for job in jobs {
            job();
        }
    }
}

/// Pump until `cond` holds; false on timeout.
pub fn pump_until(mut cond: impl FnMut() -> bool, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        drain_pump();
        if cond() {
            return true;
        }
        if start.elapsed() > timeout {
            return false;
        }
        thread::sleep(Duration::from_millis(2));
    }
}

/// Pump while polling `fut` on this (main) thread until it resolves.
pub fn drive<T>(fut: impl Future<Output = T>, timeout: Duration) -> T {
    let mut fut = pin!(fut);
    let mut cx = Context::from_waker(Waker::noop());
    let start = Instant::now();
    loop {
        drain_pump();
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
        assert!(start.elapsed() < timeout, "future did not resolve in {timeout:?}");
        thread::sleep(Duration::from_millis(2));
    }
}

pub const T: Duration = Duration::from_secs(15);

// ------------------------------------------------------------- the lite peer

/// What the lite peer has seen.
#[derive(Debug, Default, Clone)]
pub struct LiteStats {
    /// Labels of channels that opened (negotiated or remote-opened).
    pub opened: Vec<String>,
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
        static CRYPTO: OnceLock<()> = OnceLock::new();
        CRYPTO.get_or_init(|| str0m::crypto::from_feature_flags().install_process_default());
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = socket.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        let stats = Arc::new(Mutex::new(LiteStats::default()));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (s, st) = (stats.clone(), stop.clone());
        let thread = thread::Builder::new()
            .name("lite-peer".into())
            .spawn(move || lite_loop(socket, rx, s, st))
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
        self.cmds.send(LiteCmd::Offer(offer.to_string(), tx)).unwrap();
        rx.recv_timeout(T).unwrap().expect("the lite peer accepts the offer")
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
    cmds: mpsc::Receiver<LiteCmd>,
    stats: Arc<Mutex<LiteStats>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
) {
    use str0m::change::SdpOffer;
    use str0m::channel::{ChannelConfig, ChannelId, Reliability};
    use str0m::net::{Protocol, Receive};
    use str0m::{Candidate, Event, Input, Output, Rtc};

    let local = socket.local_addr().unwrap();
    let mut rtc: Option<Rtc> = None;
    let mut labels: HashMap<ChannelId, String> = HashMap::new();
    let mut buf = vec![0u8; 2000];
    // Channels opened from this side, by id, until str0m says they opened.
    let mut opening: HashMap<ChannelId, String> = HashMap::new();
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
                        let mut r = Rtc::builder().set_ice_lite(true).build(Instant::now());
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
        let timeout = loop {
            match r.poll_output() {
                Ok(Output::Timeout(t)) => break t,
                Ok(Output::Transmit(t)) => {
                    let _ = socket.send_to(&t.contents, t.destination);
                }
                Ok(Output::Event(Event::ChannelOpen(id, label))) => {
                    opening.remove(&id);
                    labels.insert(id, label.clone());
                    stats.lock().unwrap().opened.push(label);
                }
                Ok(Output::Event(Event::ChannelData(d))) => {
                    let label = labels.get(&d.id).cloned().unwrap_or_default();
                    {
                        let mut s = stats.lock().unwrap();
                        *s.received.entry(label.clone()).or_default() += d.data.len();
                        *s.messages.entry(label).or_default() += 1;
                    }
                    if let Some(mut ch) = r.channel(d.id) {
                        let _ = ch.write(true, &d.data);
                    }
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
