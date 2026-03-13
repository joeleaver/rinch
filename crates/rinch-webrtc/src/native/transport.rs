//! UDP socket management and network I/O for the str0m run loop.

use std::io::ErrorKind;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};
use tracing::{debug, trace, warn};

use crate::types::ConnectionState;

/// Commands sent from the API thread to the network thread.
#[allow(dead_code)]
pub(crate) enum NetCommand {
    /// Set the remote SDP answer and pending offer on the Rtc instance.
    AcceptAnswer {
        answer_sdp: String,
        pending: str0m::change::SdpPendingOffer,
    },
    /// Accept a remote SDP offer and produce an answer.
    AcceptOffer {
        offer_sdp: String,
        reply: std::sync::mpsc::Sender<Result<String, String>>,
    },
    /// Add a remote ICE candidate.
    AddRemoteCandidate(String),
    /// Send encoded media to the remote peer.
    SendMedia {
        mid: str0m::media::Mid,
        pt: u8,
        data: Vec<u8>,
        /// RTP timestamp in codec clock units (48kHz for audio, 90kHz for video).
        rtp_timestamp: u64,
    },
    /// Shut down the network thread.
    Shutdown,
}

/// Events sent from the network thread back to the API thread.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) enum NetEvent {
    /// A local ICE candidate was gathered.
    IceCandidate(String),
    /// Connection state changed.
    StateChange(ConnectionState),
    /// A remote media track was added.
    MediaAdded {
        mid: str0m::media::Mid,
        kind: str0m::media::MediaKind,
    },
    /// Incoming media data from the remote peer.
    MediaData {
        mid: str0m::media::Mid,
        data: Vec<u8>,
        kind: str0m::media::MediaKind,
    },
}

/// Shared state between the network thread and the API.
pub(crate) struct NetThreadHandle {
    pub cmd_tx: std::sync::mpsc::Sender<NetCommand>,
    pub event_rx: Mutex<std::sync::mpsc::Receiver<NetEvent>>,
    pub stop: Arc<AtomicBool>,
    pub thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for NetThreadHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.cmd_tx.send(NetCommand::Shutdown);
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}

/// Start the network thread. Returns the handle, the local socket address, and the SDP offer string.
///
/// The `rtc` instance should already have media lines added via `sdp_api()`.
/// The `offer` and `pending` are the result of `sdp_api().apply()`.
pub(crate) fn spawn_network_thread(mut rtc: Rtc, socket: UdpSocket) -> NetThreadHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();

    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<NetCommand>();
    let (event_tx, event_rx) = std::sync::mpsc::channel::<NetEvent>();

    let thread = std::thread::Builder::new()
        .name("rinch-webrtc-net".into())
        .spawn(move || {
            run_loop(&mut rtc, &socket, &cmd_rx, &event_tx, &stop_clone);
            debug!("network thread exiting");
        })
        .expect("failed to spawn network thread");

    NetThreadHandle {
        cmd_tx,
        event_rx: Mutex::new(event_rx),
        stop,
        thread: Some(thread),
    }
}

fn run_loop(
    rtc: &mut Rtc,
    socket: &UdpSocket,
    cmd_rx: &std::sync::mpsc::Receiver<NetCommand>,
    event_tx: &std::sync::mpsc::Sender<NetEvent>,
    stop: &AtomicBool,
) {
    let mut buf = vec![0u8; 2000];
    let local_addr = socket.local_addr().unwrap();

    // Track media mids to their kinds for MediaData events.
    let mut mid_kinds: std::collections::HashMap<str0m::media::Mid, str0m::media::MediaKind> =
        std::collections::HashMap::new();

    while !stop.load(Ordering::Relaxed) {
        // Process any pending commands (non-blocking).
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                NetCommand::AcceptAnswer {
                    answer_sdp,
                    pending,
                } => match str0m::change::SdpAnswer::from_sdp_string(&answer_sdp) {
                    Ok(answer) => {
                        if let Err(e) = rtc.sdp_api().accept_answer(pending, answer) {
                            warn!("failed to accept answer: {e}");
                        }
                    }
                    Err(e) => warn!("failed to parse SDP answer: {e}"),
                },
                NetCommand::AcceptOffer { offer_sdp, reply } => {
                    match str0m::change::SdpOffer::from_sdp_string(&offer_sdp) {
                        Ok(offer) => match rtc.sdp_api().accept_offer(offer) {
                            Ok(answer) => {
                                let _ = reply.send(Ok(answer.to_sdp_string()));
                            }
                            Err(e) => {
                                let _ = reply.send(Err(format!("{e}")));
                            }
                        },
                        Err(e) => {
                            let _ = reply.send(Err(format!("parse offer: {e}")));
                        }
                    }
                }
                NetCommand::AddRemoteCandidate(candidate_str) => {
                    match Candidate::from_sdp_string(&candidate_str) {
                        Ok(c) => rtc.add_remote_candidate(c),
                        Err(e) => warn!("failed to parse remote candidate: {e}"),
                    }
                }
                NetCommand::SendMedia {
                    mid,
                    pt,
                    data,
                    rtp_timestamp,
                } => {
                    if let Some(writer) = rtc.writer(mid) {
                        let pt: str0m::media::Pt = pt.into();
                        let now = Instant::now();
                        let time = str0m::media::MediaTime::new(
                            rtp_timestamp,
                            str0m::media::Frequency::FORTY_EIGHT_KHZ,
                        );
                        if let Err(e) = writer.write(pt, now, time, data) {
                            trace!("str0m write error: {e}");
                        }
                    }
                }
                NetCommand::Shutdown => {
                    stop.store(true, Ordering::Relaxed);
                    return;
                }
            }
        }

        // Poll str0m for output.
        let timeout = match rtc.poll_output() {
            Ok(output) => match output {
                Output::Timeout(t) => t,
                Output::Transmit(t) => {
                    if let Err(e) = socket.send_to(&t.contents, t.destination) {
                        warn!("UDP send error: {e}");
                    }
                    continue;
                }
                Output::Event(ev) => {
                    handle_event(ev, event_tx, &mut mid_kinds);
                    continue;
                }
            },
            Err(e) => {
                warn!("str0m poll error: {e}");
                break;
            }
        };

        // Wait for either a UDP packet or the timeout.
        let now = Instant::now();
        let duration = timeout.saturating_duration_since(now);

        if duration.is_zero() {
            let _ = rtc.handle_input(Input::Timeout(Instant::now()));
            continue;
        }

        // Cap the timeout so we can check for commands periodically.
        let wait = duration.min(Duration::from_millis(5));
        socket.set_read_timeout(Some(wait)).ok();

        buf.resize(2000, 0);
        match socket.recv_from(&mut buf) {
            Ok((n, source)) => {
                buf.truncate(n);
                let input = Input::Receive(
                    Instant::now(),
                    Receive {
                        proto: Protocol::Udp,
                        source,
                        destination: local_addr,
                        contents: (&*buf).try_into().unwrap(),
                    },
                );
                if let Err(e) = rtc.handle_input(input) {
                    trace!("str0m input error: {e}");
                }
            }
            Err(e) => match e.kind() {
                ErrorKind::WouldBlock | ErrorKind::TimedOut => {
                    let _ = rtc.handle_input(Input::Timeout(Instant::now()));
                }
                _ => {
                    warn!("UDP recv error: {e}");
                    break;
                }
            },
        }
    }
}

fn handle_event(
    event: Event,
    event_tx: &std::sync::mpsc::Sender<NetEvent>,
    mid_kinds: &mut std::collections::HashMap<str0m::media::Mid, str0m::media::MediaKind>,
) {
    match event {
        Event::IceConnectionStateChange(state) => {
            let conn_state = match state {
                IceConnectionState::New => ConnectionState::New,
                IceConnectionState::Checking => ConnectionState::Connecting,
                IceConnectionState::Connected => ConnectionState::Connected,
                IceConnectionState::Completed => ConnectionState::Connected,
                IceConnectionState::Disconnected => ConnectionState::Disconnected,
            };
            debug!("ICE state: {state:?} -> {conn_state:?}");
            let _ = event_tx.send(NetEvent::StateChange(conn_state));
        }
        Event::Connected => {
            debug!("DTLS connected");
            let _ = event_tx.send(NetEvent::StateChange(ConnectionState::Connected));
        }
        Event::MediaAdded(added) => {
            debug!("media added: mid={}, kind={:?}", added.mid, added.kind);
            mid_kinds.insert(added.mid, added.kind);
            let _ = event_tx.send(NetEvent::MediaAdded {
                mid: added.mid,
                kind: added.kind,
            });
        }
        Event::MediaData(data) => {
            let kind = mid_kinds
                .get(&data.mid)
                .copied()
                .unwrap_or(str0m::media::MediaKind::Audio);
            let _ = event_tx.send(NetEvent::MediaData {
                mid: data.mid,
                data: data.data,
                kind,
            });
        }
        _ => {
            trace!("unhandled str0m event: {event:?}");
        }
    }
}
