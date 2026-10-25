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
//! `rinch_core::run_on_main_thread`; only ids and `Send` payloads cross. The
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
    New { id: u64, config: RtcConfig },
    Peer { id: u64, cmd: PeerCmd },
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
    },
}

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
        Ok(_) => Ok(WORKER.get_or_init(|| tx)),
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
                Event::Error(RtcError::Internal("the rinch-rtc worker has stopped".into())),
            );
            registry::dispatch(id, Event::Closed);
        }));
    }
}

/// Report `event` for peer `id` on the main thread.
fn emit(id: u64, event: Event) {
    rinch_core::run_on_main_thread(move || registry::dispatch(id, event));
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

pub(crate) fn create_channel(id: u64, key: u64, label: String, options: ChannelOptions) {
    post(id, PeerCmd::Channel { key, label, options });
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
    if let Some(tx) = WORKER.get() {
        let _ = tx.send(Cmd::Peer {
            id,
            cmd: PeerCmd::Close { notify },
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
                Cmd::Peer { id, cmd } => {
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
                if self.trickle {
                    emit(self.id, Event::Candidate(None));
                }
            }
            RTCIceGatheringState::Gathering => {
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

    async fn on_data_channel(&self, dc: Arc<dyn DataChannel>) {
        let key = crate::next_id();
        let label = dc.label().await.unwrap_or_default();
        // Incoming before any of the channel's own events: both go through
        // the same FIFO to the main thread, and the pump starts after.
        emit(
            self.id,
            Event::Incoming {
                key,
                label,
                id: None,
            },
        );
        // The peer's task keeps it and starts its pump (not awaited here:
        // that would stall the driver).
        let _ = self.me.send(PeerCmd::Adopt { key, dc });
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
    let mut setting = SettingEngineBuilder::new();
    if let Some((d, f, k)) = overrides.ice_timeouts {
        setting = setting.with_ice_timeouts(Some(d), Some(f), Some(k));
    }
    let bind = overrides
        .bind
        .unwrap_or_else(|| vec!["0.0.0.0:0".to_string()]);
    let pc = PeerConnectionBuilder::new()
        .with_configuration(rtc_config)
        .with_setting_engine(setting.build())
        .with_runtime(Arc::new(TokioRuntime))
        .with_handler(Arc::new(handler))
        .with_udp_addrs(bind)
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
    let trickle = config.gathering == Gathering::Trickle;
    let handler = Handler {
        id,
        trickle,
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
                    PeerCmd::Close { notify } => {
                        if notify {
                            emit(id, Event::Closed);
                        }
                        return;
                    }
                    _ => {}
                }
            }
            return;
        }
    };

    let mut channels: HashMap<u64, Arc<dyn DataChannel>> = HashMap::new();
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
                emit(id, Event::Reply { req, result: result.map(Some) });
            }
            PeerCmd::Answer { req } => {
                let result = describe(&*pc, &gathered, trickle, SdpKind::Answer).await;
                emit(id, Event::Reply { req, result: result.map(Some) });
            }
            PeerCmd::Remote { req, description } => {
                let parsed = match description.kind {
                    SdpKind::Offer => RTCSessionDescription::offer(description.sdp),
                    SdpKind::Answer => RTCSessionDescription::answer(description.sdp),
                };
                let result = match parsed {
                    Ok(d) => pc
                        .set_remote_description(d)
                        .await
                        .map(|()| None)
                        .map_err(|e| RtcError::Sdp(e.to_string())),
                    Err(e) => Err(RtcError::Sdp(e.to_string())),
                };
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
            PeerCmd::Close { notify } => {
                let _ = pc.close().await;
                if notify {
                    emit(id, Event::Closed);
                }
                return;
            }
        }
    }
    // Every sender is gone: the main thread can no longer reach this peer.
    let _ = pc.close().await;
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
        SdpKind::Answer => pc.create_answer(None).await,
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
async fn pump(
    id: u64,
    key: u64,
    dc: Arc<dyn DataChannel>,
    pc: Arc<dyn PeerConnection>,
) {
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
