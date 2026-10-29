//! Native backend: webrtc-rs 0.21 on one worker thread.
//!
//! The first [`new_peer`] starts a thread, `rinch-rtc`, that runs a
//! current-thread tokio runtime for the life of the process. Every peer
//! lives there as one task that owns its webrtc-rs `PeerConnection` and
//! works through its own command queue in order, so a peer's calls keep the
//! order the app made them in, and a slow one (an offer waiting for
//! gathering) holds up only that peer. Each channel has a task forwarding its
//! events.
//!
//! Everything reaches the main thread as a [`registry::Event`] through
//! `rinch_core::dispatch_main_callback`; only ids and `Send` payloads cross. The
//! design (and the spike it rests on) is in
//! `docs/design/webrtc-data-channels.md`, section 3.3.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use bytes::BytesMut;
use tokio::sync::{mpsc, watch};
use webrtc::data_channel::{DataChannel, DataChannelEvent, RTCDataChannelInit};
use webrtc::peer_connection::{
    PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCConfigurationBuilder,
    RTCIceCandidateInit, RTCIceGatheringState, RTCIceServer, RTCIceTransportPolicy,
    RTCPeerConnectionIceErrorEvent, RTCPeerConnectionIceEvent, RTCPeerConnectionState,
    RTCSessionDescription, SettingEngineBuilder,
};
use webrtc::runtime::TokioRuntime;

use crate::registry::{self, Event};
use crate::{
    ChannelOptions, Description, Gathering, IceCandidate, IceTransportPolicy, PeerState,
    Reliability, RtcConfig, RtcError, SdpKind,
};

/// Commands from the main thread.
enum Cmd {
    New {
        id: u64,
        config: RtcConfig,
    },
    Peer {
        id: u64,
        cmd: PeerCmd,
    },
    /// The process is exiting (an `atexit` handler): close every peer, so
    /// the far ends see the association end rather than waiting for ICE to
    /// time out, and TURN allocations are released. `ack` once done.
    Shutdown {
        ack: std::sync::mpsc::Sender<()>,
    },
}

/// One peer's commands, handled in order by its task.
enum PeerCmd {
    Channel {
        key: u64,
        label: String,
        options: ChannelOptions,
    },
    Offer {
        req: u64,
        restart: bool,
    },
    Answer {
        req: u64,
    },
    Remote {
        req: u64,
        description: Description,
    },
    Candidate(Option<IceCandidate>),
    Send {
        key: u64,
        bytes: Vec<u8>,
    },
    LowThreshold {
        key: u64,
        bytes: usize,
    },
    CloseChannel {
        key: u64,
    },
    /// From the peer's own handler: a channel the remote side opened.
    Adopt {
        key: u64,
        dc: Arc<dyn DataChannel>,
    },
    Close {
        notify: bool,
        done: Option<tokio::sync::oneshot::Sender<()>>,
    },
    /// Test hook: the channel's settings as webrtc-rs holds them.
    Inspect {
        key: u64,
        reply: std::sync::mpsc::Sender<Option<ChannelSettings>>,
    },
    /// Test hook: whether a local / remote description is pending (an
    /// offer made or taken and not yet answered), as webrtc-rs holds it.
    Pending {
        reply: std::sync::mpsc::Sender<(bool, bool)>,
    },
    /// Audio and video (the `media` feature).
    #[cfg(feature = "media-core")]
    Media(crate::native_media::MediaCmd),
    /// From the peer's own handler: a remote track arrived.
    #[cfg(feature = "media-core")]
    RemoteTrack(Arc<dyn webrtc::media_stream::track_remote::TrackRemote>),
}

use crate::ChannelSettings;

static WORKER: OnceLock<mpsc::UnboundedSender<Cmd>> = OnceLock::new();

fn worker() -> Result<&'static mpsc::UnboundedSender<Cmd>, RtcError> {
    if let Some(tx) = WORKER.get() {
        return Ok(tx);
    }
    let (tx, rx) = mpsc::unbounded_channel();
    let spawned = std::thread::Builder::new()
        .name("rinch-rtc".into())
        .spawn(move || run(rx));
    match spawned {
        // Another thread cannot race this: peers are made on the main thread.
        Ok(_) => {
            let tx = WORKER.get_or_init(|| tx);
            // SAFETY: registering a plain `extern "C" fn` with no captures.
            unsafe {
                libc::atexit(shut_down_at_exit);
            }
            Ok(tx)
        }
        Err(e) => Err(RtcError::Internal(format!(
            "could not start the rinch-rtc worker: {e}"
        ))),
    }
}

fn post(id: u64, cmd: PeerCmd) {
    let sent = WORKER
        .get()
        .is_some_and(|tx| tx.send(Cmd::Peer { id, cmd }).is_ok());
    if !sent {
        // The worker is gone (it does not exit, so it panicked): say so
        // rather than hang. Deferred, so no callback runs inside the call
        // that posted.
        rinch_core::dispatch_main_callback(Box::new(move || {
            registry::dispatch(
                id,
                Event::Error(RtcError::Internal(
                    "the rinch-rtc worker has stopped".into(),
                )),
            );
            registry::dispatch(id, Event::Closed);
        }));
    }
}

/// Post a media command to peer `id`'s task.
#[cfg(feature = "media-core")]
pub(crate) fn post_media(id: u64, cmd: crate::native_media::MediaCmd) {
    post(id, PeerCmd::Media(cmd));
}

/// How long process exit waits for the worker to close every peer.
const EXIT_GRACE: Duration = Duration::from_millis(500);

/// `atexit`: close every peer before the process goes (design record 1.5).
/// Without it the far end of a peer still open at exit hears nothing until
/// its ICE consent checks time out. A process killed outright (SIGKILL, a
/// crash) still closes nothing.
extern "C" fn shut_down_at_exit() {
    shutdown();
}

/// Close every peer, including closes already on their way, and wait for
/// the worker to have sent them (at most [`EXIT_GRACE`] plus a little). A
/// no-op when no peer was ever made.
pub(crate) fn shutdown() {
    if let Some(tx) = WORKER.get() {
        let (ack, done) = std::sync::mpsc::channel();
        if tx.send(Cmd::Shutdown { ack }).is_ok() {
            let _ = done.recv_timeout(EXIT_GRACE + Duration::from_millis(100));
        }
    }
}

/// Report `event` for peer `id` on the main thread. Through
/// `dispatch_main_callback`, not `run_on_main_thread`: a peer made before
/// the host has installed its dispatcher (before `App::run`) has its events
/// queued until the host drains them, where `run_on_main_thread` would panic
/// the worker.
fn emit(id: u64, event: Event) {
    rinch_core::dispatch_main_callback(Box::new(move || registry::dispatch(id, event)));
}

// ------------------------------------------------- the main-thread surface

pub(crate) fn new_peer(id: u64, config: &RtcConfig) -> Result<(), RtcError> {
    worker()?
        .send(Cmd::New {
            id,
            config: config.clone(),
        })
        .map_err(|_| RtcError::Internal("the rinch-rtc worker has stopped".into()))
}

/// Returns the stream id when it is known now: a negotiated channel's
/// (webrtc-rs 0.21 never reports an in-band one's).
pub(crate) fn create_channel(
    id: u64,
    key: u64,
    label: String,
    options: ChannelOptions,
) -> Option<u16> {
    let sid = options.negotiated;
    post(
        id,
        PeerCmd::Channel {
            key,
            label,
            options,
        },
    );
    sid
}

pub(crate) fn create_offer(id: u64, req: u64, restart: bool) {
    post(id, PeerCmd::Offer { req, restart });
}

pub(crate) fn create_answer(id: u64, req: u64) {
    post(id, PeerCmd::Answer { req });
}

pub(crate) fn set_remote(id: u64, req: u64, description: Description) {
    post(id, PeerCmd::Remote { req, description });
}

pub(crate) fn add_candidate(id: u64, candidate: Option<IceCandidate>) {
    post(id, PeerCmd::Candidate(candidate));
}

pub(crate) fn send(id: u64, key: u64, bytes: Vec<u8>) {
    post(id, PeerCmd::Send { key, bytes });
}

pub(crate) fn set_low_threshold(id: u64, key: u64, bytes: usize) {
    post(id, PeerCmd::LowThreshold { key, bytes });
}

pub(crate) fn close_channel(id: u64, key: u64) {
    post(id, PeerCmd::CloseChannel { key });
}

/// Close peer `id`. `notify`: report the final [`Event::Closed`] (an
/// explicit `close()`); otherwise the peer's registry entry is already gone
/// (a drop, an unmount) and nothing is reported.
pub(crate) fn close(id: u64, notify: bool) {
    #[cfg(feature = "media-core")]
    crate::native_media::peer_closed_main(id);
    if let Some(tx) = WORKER.get() {
        let _ = tx.send(Cmd::Peer {
            id,
            cmd: PeerCmd::Close { notify, done: None },
        });
    }
}

// ------------------------------------------------------------ test hooks

/// What rinch-rtc's tests change about every peer created after the call.
#[derive(Debug, Clone, Default)]
struct Overrides {
    bind: Option<Vec<String>>,
    ice_timeouts: Option<(Duration, Duration, Duration)>,
}

static OVERRIDES: Mutex<Overrides> = Mutex::new(Overrides {
    bind: None,
    ice_timeouts: None,
});

pub mod testing {
    //! Hooks for rinch-rtc's own tests (`#[doc(hidden)]`, not API).
    use std::time::Duration;

    /// Bind new peers' sockets to these addresses (default `0.0.0.0:0`).
    /// Tests bind `127.0.0.1:0`: webrtc-rs leaves loopback out of the
    /// candidates it gathers from a wildcard bind.
    pub fn set_bind_addrs(addrs: &[&str]) {
        super::OVERRIDES.lock().unwrap().bind = Some(addrs.iter().map(|s| s.to_string()).collect());
    }

    /// Shorten new peers' ICE timers, so a test of `Disconnected`/`Failed`
    /// takes seconds.
    pub fn set_ice_timeouts(disconnected: Duration, failed: Duration, keepalive: Duration) {
        super::OVERRIDES.lock().unwrap().ice_timeouts = Some((disconnected, failed, keepalive));
    }

    pub use crate::ChannelSettings;

    /// Whether webrtc-rs holds a pending local / remote description for
    /// `peer` (blocks up to 5 s).
    pub fn pending_descriptions(peer: &crate::Peer) -> Option<(bool, bool)> {
        let (reply, rx) = std::sync::mpsc::channel();
        super::post(peer.id(), super::PeerCmd::Pending { reply });
        rx.recv_timeout(Duration::from_secs(5)).ok()
    }

    /// The channel's settings as webrtc-rs holds them, asked of the worker
    /// directly (blocks up to 5 s; no main-thread pumping needed).
    pub fn channel_settings(channel: &crate::Channel) -> Option<ChannelSettings> {
        let (reply, rx) = std::sync::mpsc::channel();
        super::post(
            channel.peer,
            super::PeerCmd::Inspect {
                key: channel.key,
                reply,
            },
        );
        rx.recv_timeout(Duration::from_secs(5)).ok().flatten()
    }
}

// --------------------------------------------------------------- worker

fn run(mut rx: mpsc::UnboundedReceiver<Cmd>) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!("rinch-rtc: could not build the worker runtime: {e}");
            return;
        }
    };
    rt.block_on(async move {
        let mut peers: HashMap<u64, mpsc::UnboundedSender<PeerCmd>> = HashMap::new();
        // Closes already on their way (close(), a drop, an unmount): a
        // Shutdown must wait for these too, or a `close(); exit()` dies
        // before the close is sent.
        let mut closing: Vec<tokio::sync::oneshot::Receiver<()>> = Vec::new();
        while let Some(cmd) = rx.recv().await {
            match cmd {
                Cmd::New { id, config } => {
                    let (tx, prx) = mpsc::unbounded_channel();
                    let task = tokio::spawn(peer_task(id, config, prx, tx.clone()));
                    // A panic in the stack ends only this peer, audibly.
                    tokio::spawn(async move {
                        if let Err(e) = task.await
                            && e.is_panic()
                        {
                            emit(
                                id,
                                Event::Error(RtcError::Internal(
                                    "the peer's task panicked in the WebRTC stack".into(),
                                )),
                            );
                            emit(id, Event::Closed);
                        }
                    });
                    peers.insert(id, tx);
                }
                Cmd::Shutdown { ack } => {
                    let mut waits = std::mem::take(&mut closing);
                    for (_, tx) in peers.drain() {
                        let (done, wait) = tokio::sync::oneshot::channel();
                        if tx
                            .send(PeerCmd::Close {
                                notify: false,
                                done: Some(done),
                            })
                            .is_ok()
                        {
                            waits.push(wait);
                        }
                    }
                    let all = async {
                        for w in waits {
                            let _ = w.await;
                        }
                    };
                    let _ = tokio::time::timeout(EXIT_GRACE, all).await;
                    let _ = ack.send(());
                }
                Cmd::Peer { id, cmd } => {
                    let cmd = match cmd {
                        PeerCmd::Close { notify, done: None } => {
                            let (done, wait) = tokio::sync::oneshot::channel();
                            closing.retain_mut(|w| {
                                matches!(
                                    w.try_recv(),
                                    Err(tokio::sync::oneshot::error::TryRecvError::Empty)
                                )
                            });
                            closing.push(wait);
                            PeerCmd::Close {
                                notify,
                                done: Some(done),
                            }
                        }
                        other => other,
                    };
                    let last = matches!(cmd, PeerCmd::Close { .. });
                    if let Some(tx) = peers.get(&id) {
                        let _ = tx.send(cmd);
                    }
                    if last {
                        peers.remove(&id);
                    }
                }
            }
        }
    });
}

struct Handler {
    id: u64,
    trickle: bool,
    /// End-of-candidates was reported for the current gathering round:
    /// webrtc-rs can report `Complete` more than once per round (host and
    /// relay gathering each finish), and `on_ice_candidate(None)` is once.
    /// A round starts at `Gathering`, or at an ICE restart's offer (webrtc-rs
    /// does not always report `Gathering` for it).
    ended: Arc<std::sync::atomic::AtomicBool>,
    gathered: Arc<watch::Sender<bool>>,
    me: mpsc::UnboundedSender<PeerCmd>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_candidate(&self, e: RTCPeerConnectionIceEvent) {
        if !self.trickle {
            return;
        }
        match e.candidate.to_json() {
            Ok(init) => emit(
                self.id,
                Event::Candidate(Some(IceCandidate {
                    candidate: init.candidate,
                    sdp_mid: init.sdp_mid,
                    sdp_m_line_index: init.sdp_mline_index,
                })),
            ),
            Err(err) => tracing::warn!("rinch-rtc: unserializable local candidate: {err}"),
        }
    }

    async fn on_ice_gathering_state_change(&self, s: RTCIceGatheringState) {
        match s {
            RTCIceGatheringState::Complete => {
                let _ = self.gathered.send(true);
                let first = !self.ended.swap(true, std::sync::atomic::Ordering::Relaxed);
                if self.trickle && first {
                    emit(self.id, Event::Candidate(None));
                }
            }
            RTCIceGatheringState::Gathering => {
                self.ended
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                let _ = self.gathered.send(false);
            }
            _ => {}
        }
    }

    async fn on_ice_candidate_error(&self, e: RTCPeerConnectionIceErrorEvent) {
        emit(
            self.id,
            Event::Error(RtcError::Ice {
                url: e.url,
                code: e.error_code,
                text: e.error_text,
            }),
        );
    }

    async fn on_connection_state_change(&self, s: RTCPeerConnectionState) {
        let state = match s {
            RTCPeerConnectionState::New => PeerState::New,
            RTCPeerConnectionState::Connecting => PeerState::Connecting,
            RTCPeerConnectionState::Connected => PeerState::Connected,
            RTCPeerConnectionState::Disconnected => PeerState::Disconnected,
            RTCPeerConnectionState::Failed => PeerState::Failed,
            RTCPeerConnectionState::Closed => PeerState::Closed,
            _ => return,
        };
        emit(self.id, Event::State(state));
    }

    #[cfg(feature = "media-core")]
    async fn on_track(&self, track: Arc<dyn webrtc::media_stream::track_remote::TrackRemote>) {
        // The peer's task finds its transceiver and starts its reader.
        let _ = self.me.send(PeerCmd::RemoteTrack(track));
    }

    #[cfg(feature = "media-core")]
    async fn on_negotiation_needed(&self) {
        emit(
            self.id,
            Event::Media(crate::media::MediaEvent::NegotiationNeeded),
        );
    }

    async fn on_data_channel(&self, dc: Arc<dyn DataChannel>) {
        let key = crate::next_id();
        let label = dc.label().await.unwrap_or_default();
        // The peer's task keeps it and starts its pump (not awaited here:
        // that would stall the driver). Adopted *before* the main thread hears
        // of it, so a call the app makes from inside on_data_channel (close,
        // a low-water threshold) finds the channel already there.
        let _ = self.me.send(PeerCmd::Adopt { key, dc });
        emit(
            self.id,
            Event::Incoming {
                key,
                label,
                id: None,
            },
        );
    }
}

async fn build(
    id: u64,
    config: &RtcConfig,
    handler: Handler,
) -> Result<Arc<dyn PeerConnection>, RtcError> {
    let overrides = OVERRIDES.lock().map(|o| o.clone()).unwrap_or_default();
    let servers = config
        .ice_servers
        .iter()
        .map(|s| RTCIceServer {
            urls: s.urls.clone(),
            username: s.username.clone().unwrap_or_default(),
            credential: s.credential.clone().unwrap_or_default(),
        })
        .collect();
    let rtc_config = RTCConfigurationBuilder::new()
        .with_ice_servers(servers)
        .with_ice_transport_policy(match config.ice_transport_policy {
            IceTransportPolicy::All => RTCIceTransportPolicy::All,
            IceTransportPolicy::Relay => RTCIceTransportPolicy::Relay,
        })
        .build();
    // Accept (and so advertise, and send up to) 256 KiB messages, Chrome's
    // limit, instead of webrtc-rs's 64 KiB default: the same app then gets the
    // same max_message_size() on both backends.
    let mut setting = SettingEngineBuilder::new().with_sctp_max_message_size(
        rtc::peer_connection::configuration::setting_engine::SctpMaxMessageSize::Bounded(
            crate::NATIVE_MAX_MESSAGE_SIZE as u32,
        ),
    );
    if let Some((d, f, k)) = overrides.ice_timeouts {
        setting = setting.with_ice_timeouts(Some(d), Some(f), Some(k));
    }
    let bind = overrides
        .bind
        .unwrap_or_else(|| vec!["0.0.0.0:0".to_string()]);
    let builder = PeerConnectionBuilder::new();
    // Media: the codecs the native side can encode and decode (H.264,
    // Opus) and the default RTCP interceptors (NACK, reports, TWCC).
    #[cfg(feature = "media-core")]
    let builder = {
        let (me, registry) = crate::native_media::worker::media_engine()?;
        builder
            .with_media_engine(me)
            .with_interceptor_registry(registry)
    };
    let pc = builder
        .with_configuration(rtc_config)
        .with_setting_engine(setting.build())
        .with_runtime(Arc::new(TokioRuntime))
        .with_handler(Arc::new(handler))
        .with_udp_addrs(bind)
        // The backstop behind rinch's own per-channel limit (design record
        // 5): should the main thread's mirror ever undercount, sends wait on
        // the worker instead of growing without bound.
        .with_data_channel_send_buffer_limit(crate::MAX_SEND_BUFFER_LIMIT)
        .build()
        .await
        .map_err(|e| RtcError::Internal(format!("peer {id}: {e}")))?;
    Ok(Arc::new(pc))
}

async fn peer_task(
    id: u64,
    config: RtcConfig,
    mut rx: mpsc::UnboundedReceiver<PeerCmd>,
    me: mpsc::UnboundedSender<PeerCmd>,
) {
    let gathered = Arc::new(watch::Sender::new(false));
    let ended = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let trickle = config.gathering == Gathering::Trickle;
    let handler = Handler {
        id,
        trickle,
        ended: ended.clone(),
        gathered: gathered.clone(),
        me,
    };
    let pc = match build(id, &config, handler).await {
        Ok(pc) => pc,
        Err(e) => {
            emit(id, Event::Error(e.clone()));
            // Answer every call made meanwhile, then end.
            while let Some(cmd) = rx.recv().await {
                match cmd {
                    PeerCmd::Offer { req, .. }
                    | PeerCmd::Answer { req }
                    | PeerCmd::Remote { req, .. } => emit(
                        id,
                        Event::Reply {
                            req,
                            result: Err(e.clone()),
                        },
                    ),
                    PeerCmd::Close { notify, done } => {
                        if notify {
                            emit(id, Event::Closed);
                        }
                        if let Some(d) = done {
                            let _ = d.send(());
                        }
                        return;
                    }
                    PeerCmd::Inspect { reply, .. } => {
                        let _ = reply.send(None);
                    }
                    PeerCmd::Pending { reply } => {
                        let _ = reply.send((false, false));
                    }
                    #[cfg(feature = "media-core")]
                    PeerCmd::Media(crate::native_media::MediaCmd::ReplaceTrack { req, .. }) => {
                        emit(
                            id,
                            Event::Reply {
                                req,
                                result: Err(e.clone()),
                            },
                        )
                    }
                    _ => {}
                }
            }
            return;
        }
    };

    let mut channels: HashMap<u64, Arc<dyn DataChannel>> = HashMap::new();
    #[cfg(feature = "media-core")]
    let mut media = crate::native_media::worker::PeerWorker::new(id);
    while let Some(cmd) = rx.recv().await {
        match cmd {
            PeerCmd::Channel {
                key,
                label,
                options,
            } => {
                let (ordered, max_retransmits) = match options.reliability {
                    Reliability::Reliable => (true, None),
                    Reliability::Unreliable { max_retransmits } => (false, Some(max_retransmits)),
                };
                let init = RTCDataChannelInit {
                    ordered,
                    max_retransmits,
                    negotiated: options.negotiated,
                    ..Default::default()
                };
                match pc.create_data_channel(&label, Some(init)).await {
                    Ok(dc) => {
                        channels.insert(key, dc.clone());
                        tokio::spawn(pump(id, key, dc, pc.clone()));
                    }
                    Err(e) => {
                        emit(
                            id,
                            Event::ChannelError {
                                key,
                                error: RtcError::Channel(e.to_string()),
                            },
                        );
                        emit(id, Event::ChannelClosed { key });
                    }
                }
            }
            PeerCmd::Offer { req, restart } => {
                if restart {
                    ended.store(false, std::sync::atomic::Ordering::Relaxed);
                }
                if restart && let Err(e) = pc.restart_ice().await {
                    emit(
                        id,
                        Event::Reply {
                            req,
                            result: Err(RtcError::InvalidState(e.to_string())),
                        },
                    );
                    continue;
                }
                let result = describe(&*pc, &gathered, trickle, SdpKind::Offer).await;
                #[cfg(feature = "media-core")]
                media.refresh(&pc).await;
                emit(
                    id,
                    Event::Reply {
                        req,
                        result: result.map(Some),
                    },
                );
            }
            PeerCmd::Answer { req } => {
                let result = describe(&*pc, &gathered, trickle, SdpKind::Answer).await;
                #[cfg(feature = "media-core")]
                {
                    media.refresh(&pc).await;
                    if result.is_ok() {
                        media.check_labels(&pc).await;
                    }
                }
                emit(
                    id,
                    Event::Reply {
                        req,
                        result: result.map(Some),
                    },
                );
            }
            PeerCmd::Remote { req, description } => {
                #[cfg(feature = "media-core")]
                let offer =
                    matches!(description.kind, SdpKind::Offer).then(|| description.sdp.clone());
                #[cfg(feature = "media-core")]
                let answer = matches!(description.kind, SdpKind::Answer);
                // rtc binds our senders to the interceptors while it applies
                // an answer, by their tracks' codec labels: label them with
                // what the answer chose first (#1954).
                #[cfg(feature = "media-core")]
                if answer {
                    media.relabel_for_answer(&pc, &description.sdp).await;
                }
                let result = apply_remote(&*pc, description).await;
                #[cfg(feature = "media-core")]
                {
                    media.refresh(&pc).await;
                    if result.is_ok()
                        && let Some(sdp) = offer
                    {
                        // A video m-line the answer will reject: say so (#1975).
                        media.check_remote_offer(&sdp);
                        // Senders the offer's codecs leave unlabelled, before
                        // our answer binds them (#1954).
                        media.relabel_for_offer().await;
                    } else if result.is_ok() && answer {
                        media.check_labels(&pc).await;
                    }
                }
                emit(id, Event::Reply { req, result });
            }
            PeerCmd::Candidate(c) => {
                // An empty candidate is the remote end-of-candidates.
                let init = match c {
                    Some(c) => RTCIceCandidateInit {
                        candidate: c.candidate,
                        sdp_mid: c.sdp_mid,
                        sdp_mline_index: c.sdp_m_line_index,
                        ..Default::default()
                    },
                    None => RTCIceCandidateInit::default(),
                };
                if let Err(e) = pc.add_ice_candidate(init).await {
                    emit(id, Event::Error(RtcError::Candidate(e.to_string())));
                }
            }
            PeerCmd::Send { key, bytes } => {
                let Some(dc) = channels.get(&key) else {
                    continue;
                };
                let sent = bytes.len();
                if let Err(e) = dc.send(BytesMut::from(&bytes[..])).await {
                    emit(
                        id,
                        Event::ChannelError {
                            key,
                            error: RtcError::Channel(e.to_string()),
                        },
                    );
                }
                let amount = dc.outstanding_bytes().await.unwrap_or(0);
                emit(id, Event::Buffered { key, sent, amount });
            }
            PeerCmd::LowThreshold { key, bytes } => {
                if let Some(dc) = channels.get(&key) {
                    let t = u32::try_from(bytes).unwrap_or(u32::MAX);
                    let _ = dc.set_buffered_amount_low_threshold(t).await;
                }
            }
            PeerCmd::CloseChannel { key } => {
                if let Some(dc) = channels.get(&key) {
                    // The channel's pump reports the close when it lands.
                    let _ = dc.close().await;
                }
            }
            PeerCmd::Adopt { key, dc } => {
                channels.insert(key, dc.clone());
                tokio::spawn(pump(id, key, dc, pc.clone()));
            }
            PeerCmd::Close { notify, done } => {
                #[cfg(feature = "media-core")]
                media.close();
                let _ = pc.close().await;
                if notify {
                    emit(id, Event::Closed);
                }
                if let Some(d) = done {
                    let _ = d.send(());
                }
                return;
            }
            #[cfg(feature = "media-core")]
            PeerCmd::Media(cmd) => media.handle(&pc, cmd).await,
            #[cfg(feature = "media-core")]
            PeerCmd::RemoteTrack(track) => media.remote_track(&pc, track).await,
            PeerCmd::Pending { reply } => {
                let local = pc.pending_local_description().await.is_some();
                let remote = pc.pending_remote_description().await.is_some();
                let _ = reply.send((local, remote));
            }
            PeerCmd::Inspect { key, reply } => {
                let settings = match channels.get(&key) {
                    Some(dc) => match (
                        dc.ordered().await,
                        dc.max_retransmits().await,
                        dc.negotiated().await,
                    ) {
                        (Ok(ordered), Ok(max_retransmits), Ok(negotiated)) => {
                            Some(ChannelSettings {
                                ordered,
                                max_retransmits,
                                negotiated,
                            })
                        }
                        _ => None,
                    },
                    None => None,
                };
                let _ = reply.send(settings);
            }
        }
    }
    // Every sender is gone: the main thread can no longer reach this peer.
    #[cfg(feature = "media-core")]
    media.close();
    let _ = pc.close().await;
}

/// Apply a remote description, with the rollbacks browsers do:
/// - an offer that arrives while ours is pending rolls ours back first
///   (webrtc-rs 0.21 has no implicit rollback: design record 1.7);
/// - an explicit rollback undoes whichever offer is pending.
async fn apply_remote(
    pc: &dyn PeerConnection,
    description: Description,
) -> Result<Option<Description>, RtcError> {
    use webrtc::peer_connection::RTCSdpType;
    let local_offer = pc
        .pending_local_description()
        .await
        .is_some_and(|d| d.sdp_type == RTCSdpType::Offer);
    let sdp_err = |e: webrtc::error::Error| RtcError::Sdp(e.to_string());
    let rollback =
        || RTCSessionDescription::rollback(None).map_err(|e| RtcError::Sdp(e.to_string()));
    match description.kind {
        SdpKind::Offer => {
            if local_offer {
                pc.set_local_description(rollback()?)
                    .await
                    .map_err(sdp_err)?;
            }
            let d = RTCSessionDescription::offer(description.sdp)
                .map_err(|e| RtcError::Sdp(e.to_string()))?;
            pc.set_remote_description(d).await.map_err(sdp_err)?;
        }
        SdpKind::Answer => {
            let d = RTCSessionDescription::answer(description.sdp)
                .map_err(|e| RtcError::Sdp(e.to_string()))?;
            pc.set_remote_description(d).await.map_err(sdp_err)?;
        }
        _ => {
            if local_offer {
                pc.set_local_description(rollback()?)
                    .await
                    .map_err(sdp_err)?;
            } else {
                pc.set_remote_description(rollback()?)
                    .await
                    .map_err(sdp_err)?;
            }
        }
    }
    Ok(None)
}

/// Create an offer or answer, set it as the local description, and (when
/// not trickling) wait for gathering to finish so the SDP carries every
/// candidate.
async fn describe(
    pc: &dyn PeerConnection,
    gathered: &watch::Sender<bool>,
    trickle: bool,
    kind: SdpKind,
) -> Result<Description, RtcError> {
    let sdp = match kind {
        SdpKind::Offer => pc.create_offer(None).await,
        _ => pc.create_answer(None).await,
    }
    .map_err(|e| RtcError::InvalidState(e.to_string()))?;
    // Gathering starts with this local description; a "complete" seen
    // before belongs to an earlier round.
    let mut done = (!trickle).then(|| {
        gathered.send_replace(false);
        gathered.subscribe()
    });
    pc.set_local_description(sdp.clone())
        .await
        .map_err(|e| RtcError::Sdp(e.to_string()))?;
    let sdp = if let Some(done) = done.as_mut() {
        let _ = done.wait_for(|d| *d).await;
        pc.local_description()
            .await
            .map(|d| d.sdp)
            .unwrap_or(sdp.sdp)
    } else {
        sdp.sdp
    };
    Ok(Description { kind, sdp })
}

/// Forward one channel's events to the main thread until it closes.
async fn pump(id: u64, key: u64, dc: Arc<dyn DataChannel>, pc: Arc<dyn PeerConnection>) {
    while let Some(event) = dc.poll().await {
        match event {
            DataChannelEvent::OnOpen => {
                let mut max = crate::DEFAULT_MAX_MESSAGE_SIZE;
                if let Some(sctp) = pc.sctp().await
                    && let Ok(m) = sctp.max_message_size().await
                    && m > 0
                {
                    max = m as usize;
                }
                emit(
                    id,
                    Event::Open {
                        key,
                        id: None,
                        max_message_size: max,
                    },
                );
            }
            DataChannelEvent::OnMessage(m) => emit(
                id,
                Event::Message {
                    key,
                    data: m.data.to_vec(),
                },
            ),
            DataChannelEvent::OnBufferedAmountLow => {
                let amount = dc.outstanding_bytes().await.unwrap_or(0);
                emit(id, Event::Low { key, amount });
            }
            DataChannelEvent::OnError => emit(
                id,
                Event::ChannelError {
                    key,
                    error: RtcError::Channel("the data channel reported an error".into()),
                },
            ),
            DataChannelEvent::OnClose => break,
            _ => {}
        }
    }
    emit(id, Event::ChannelClosed { key });
}
