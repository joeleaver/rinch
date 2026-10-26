//! Cross-platform WebRTC **data channels** for rinch, with the shape of
//! `rinch-ws`: callbacks on the UI thread, bytes in and out, one API on web
//! and desktop. Design record: `docs/design/webrtc-data-channels.md`
//! (issue #1896).
//!
//! - **Native** (`cfg(not(target_arch = "wasm32"))`): webrtc-rs 0.21, all
//!   peers on one worker thread running a current-thread tokio runtime.
//!   STUN and TURN (UDP) gathering, trickle ICE, ICE restart.
//! - **Web** (`cfg(target_arch = "wasm32")`): the browser's
//!   `RTCPeerConnection` and `RTCDataChannel`, through web-sys.
//!
//! # Signalling is the app's
//!
//! rinch hands out and takes in plain values: [`Description`] (an SDP offer
//! or answer) and [`IceCandidate`]. Carry them over your own channel, a
//! `rinch-ws` socket or HTTP. A peer that offers:
//!
//! ```ignore
//! use rinch_rtc::*;
//!
//! let peer = Peer::new(RtcConfig {
//!     ice_servers: vec![IceServer::stun("stun:stun.example.org:3478")],
//!     ..Default::default()
//! })?;
//! // Pre-negotiated: the same id and options on both sides.
//! let control = peer.create_data_channel("control", ChannelOptions::negotiated(0, Reliability::Reliable))?;
//! let tick = peer.create_data_channel(
//!     "tick",
//!     ChannelOptions::negotiated(1, Reliability::Unreliable { max_retransmits: 0 }),
//! )?;
//! tick.on_message(move |bytes| world.update(|w| w.apply(&bytes)));
//! control.on_open(move || status.set("connected"));
//!
//! let ws2 = ws.clone();
//! peer.on_ice_candidate(move |c| ws2.send_text(encode_candidate(c)));
//! let (p, ws2) = (peer.clone(), ws.clone());
//! spawn_local(async move {
//!     if let Ok(offer) = p.create_offer().await {
//!         ws2.send_text(encode_offer(offer));
//!     }
//! });
//! // When the answer and the other side's candidates come back:
//! //     spawn_local(async move { let _ = peer.set_remote_description(answer).await; });
//! //     peer.add_ice_candidate(candidate);
//! ```
//!
//! The signalling calls are queued **when they are called**, in call order,
//! and the returned future only waits for the result. A candidate that
//! arrives before any remote description is held until one is set.
//!
//! # Threading
//!
//! Call everything on the main (UI) thread. The callbacks live in a
//! main-thread registry, may be `!Send`, and are invoked on the main thread
//! in a reactive batch, under the document and context root that registered
//! them. On native, delivery relies on the rinch runtime's cross-thread
//! dispatcher; outside a running app, install one (see the crate's tests).
//!
//! # Lifetime: cancel on unmount, with an opt-out
//!
//! A [`Peer`] created **while a component renders** is closed when that
//! component unmounts, silently: its callbacks belong to the component and are
//! gone with it. That releases its sockets and any TURN allocation. Each
//! callback is also pruned on its own registering component's unmount, as in
//! `rinch-ws` (#183).
//!
//! To keep a peer past its component (a lobby connection kept in a store
//! across route changes), create it outside any render or under
//! [`unowned`](rinch_core::reactive::unowned):
//!
//! ```ignore
//! let peer = rinch_core::reactive::unowned(|| Peer::new(config))?;
//! ```
//!
//! Such a peer lives until [`Peer::close`] or until its last clone drops.
//!
//! # Process exit
//!
//! On native, peers still open when the process exits are closed on the
//! way out (and those whose close was already under way are let finish), so
//! the far end sees them go at once rather than when its ICE checks time
//! out. That runs from an `atexit` handler, waiting at most about half a
//! second:
//!
//! - **Linux, macOS, Android:** returning from `main` and
//!   `std::process::exit` both run it.
//! - **Windows:** returning from `main` runs it; `std::process::exit` may not
//!   (it ends the process through `ExitProcess`). Call [`shutdown`] first.
//! - A process killed outright (SIGKILL, a crash) closes nothing.
//!
//! [`shutdown`] does the same on demand, on every platform.
//!
//! # Without `App::run` (headless)
//!
//! Events reach the main thread through rinch-core's main-callback queue. A
//! rinch app's event loop drains it; a program that never runs one (a CLI
//! tool, a test) must drive it itself, on the thread that made the peers:
//!
//! ```ignore
//! rinch_core::register_main_thread();
//! let peer = Peer::new(config)?;
//! loop {
//!     rinch_core::drain_main_callbacks(); // runs callbacks, resolves replies
//!     // ...poll your futures, sleep a little...
//! }
//! ```
//!
//! Until something drains it the queue only grows, payloads included, and
//! the signalling futures do not resolve. (Or install a dispatcher of your
//! own with `rinch_core::set_cross_thread_dispatcher`, as the crate's tests
//! do.) A
//! callback that captures a clone of its own peer keeps it alive; capture a
//! [`WeakPeer`] (from [`Peer::downgrade`]) instead.

mod error;
mod registry;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
use native as backend;

#[cfg(target_arch = "wasm32")]
mod wasm;
#[cfg(target_arch = "wasm32")]
use wasm as backend;

#[cfg(feature = "media-core")]
pub mod media;
#[cfg(all(feature = "media-core", not(target_arch = "wasm32")))]
mod native_media;
#[cfg(all(feature = "media-core", target_arch = "wasm32"))]
mod wasm_media;

pub use error::{RtcError, SendError};

use std::future::Future;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering};

use registry::{ChannelEntry, with_peer};

/// RFC 8841's default when the remote SDP has no `a=max-message-size`.
pub const DEFAULT_MAX_MESSAGE_SIZE: usize = 65_536;

/// The largest message the native backend accepts, and so advertises:
/// 256 KiB, what Chrome advertises.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const NATIVE_MAX_MESSAGE_SIZE: usize = 262_144;

/// The largest [`ChannelOptions::send_buffer_limit`]: 16 MiB (Chromium's
/// send-queue cap). A larger limit is clamped to it. On native it is also
/// webrtc-rs's own per-channel backstop.
pub const MAX_SEND_BUFFER_LIMIT: usize = 16 << 20;

/// [`ChannelOptions::send_buffer_limit`]'s default: 1 MiB.
pub const DEFAULT_SEND_BUFFER_LIMIT: usize = 1 << 20;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// One counter for peer ids, channel keys and request ids, shared with the
/// native worker (which keys channels the remote side opens).
pub(crate) fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

// ------------------------------------------------------------------ config

/// How a peer connects: which STUN/TURN servers, which candidates, and
/// whether candidates trickle.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RtcConfig {
    /// STUN and TURN servers. Empty: host candidates only, which is enough
    /// to reach a peer with a public address (an ICE-lite server).
    pub ice_servers: Vec<IceServer>,
    /// [`IceTransportPolicy::Relay`] uses TURN candidates only.
    pub ice_transport_policy: IceTransportPolicy,
    /// Trickle candidates, or wait for gathering to finish.
    pub gathering: Gathering,
}

/// A STUN or TURN server.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IceServer {
    /// `stun:host[:port]` or `turn:host[:port][?transport=udp]`. On native,
    /// `turns:` and TCP TURN are skipped (design record D11).
    pub urls: Vec<String>,
    /// The TURN username.
    pub username: Option<String>,
    /// The TURN credential.
    pub credential: Option<String>,
}

impl IceServer {
    /// A STUN server.
    pub fn stun(url: impl Into<String>) -> Self {
        Self {
            urls: vec![url.into()],
            username: None,
            credential: None,
        }
    }

    /// A TURN server with long-term credentials.
    pub fn turn(
        url: impl Into<String>,
        username: impl Into<String>,
        credential: impl Into<String>,
    ) -> Self {
        Self {
            urls: vec![url.into()],
            username: Some(username.into()),
            credential: Some(credential.into()),
        }
    }
}

/// Which local candidates a peer uses (`RTCIceTransportPolicy`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IceTransportPolicy {
    /// Host, server-reflexive and relay candidates.
    #[default]
    All,
    /// Relay (TURN) candidates only.
    Relay,
}

/// How local candidates reach the other side.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Gathering {
    /// `create_offer`/`create_answer` return at once, and candidates arrive
    /// through [`Peer::on_ice_candidate`], ending with `None`.
    #[default]
    Trickle,
    /// `create_offer`/`create_answer` return once gathering has finished,
    /// with every candidate inside the SDP; `on_ice_candidate` is not called.
    Complete,
}

// ------------------------------------------------------------- signalling

/// Whether a [`Description`] is an offer, an answer or a rollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
#[non_exhaustive]
pub enum SdpKind {
    Offer,
    Answer,
    /// Undo the pending offer, ours or theirs, back to `Stable`
    /// ([`Description::rollback`], given to `set_remote_description`).
    /// Perfect negotiation rarely needs it: a polite peer's
    /// `set_remote_description(their_offer)` rolls its own offer back by
    /// itself. Web now; desktop returns an error until the native media PR.
    Rollback,
}

/// Where the offer/answer exchange stands (`RTCSignalingState`), as
/// [`Peer::signaling_state`] reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SignalingState {
    /// No exchange in progress.
    Stable,
    /// We are making an offer, or made one and wait for the answer.
    HaveLocalOffer,
    /// Their offer is set; we owe an answer.
    HaveRemoteOffer,
    Closed,
}

/// An SDP offer or answer, to carry to the other side.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Description {
    pub kind: SdpKind,
    pub sdp: String,
}

impl Description {
    /// An offer.
    pub fn offer(sdp: impl Into<String>) -> Self {
        Self {
            kind: SdpKind::Offer,
            sdp: sdp.into(),
        }
    }

    /// An answer.
    pub fn answer(sdp: impl Into<String>) -> Self {
        Self {
            kind: SdpKind::Answer,
            sdp: sdp.into(),
        }
    }
    /// A rollback (no SDP).
    pub fn rollback() -> Self {
        Self {
            kind: SdpKind::Rollback,
            sdp: String::new(),
        }
    }
}

/// An ICE candidate: the browser's `RTCIceCandidateInit`, which is what
/// signalling channels carry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct IceCandidate {
    /// The `candidate:` attribute string.
    pub candidate: String,
    pub sdp_mid: Option<String>,
    pub sdp_m_line_index: Option<u16>,
}

/// A peer's connection state (`RTCPeerConnectionState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerState {
    New,
    Connecting,
    Connected,
    /// Transient: connectivity checks are failing, and the stack keeps
    /// trying. It may return to `Connected` by itself.
    Disconnected,
    /// ICE gave up. Only an ICE restart ([`Peer::restart_ice`]) or a new
    /// peer recovers.
    Failed,
    /// Final.
    Closed,
}

/// A channel's state (`RTCDataChannelState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelState {
    Connecting,
    Open,
    Closing,
    Closed,
}

// ---------------------------------------------------------------- channels

/// How a channel delivers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Reliability {
    /// Ordered and reliable.
    #[default]
    Reliable,
    /// Unordered; each message is retransmitted at most `max_retransmits`
    /// times (0: sent once).
    Unreliable { max_retransmits: u16 },
}

/// Options for [`Peer::create_data_channel`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelOptions {
    pub reliability: Reliability,
    /// `Some(id)`: pre-negotiated, declared with the same id and options on
    /// both sides; no in-band announcement and no `on_data_channel`. Declare
    /// negotiated channels before any in-band one, and keep their ids low.
    ///
    /// An id already used by an open channel of the same peer is refused
    /// with [`RtcError::InvalidState`]: another negotiated channel, or an
    /// in-band one whose id the stack has reported (the web reports it once
    /// the channel opens; webrtc-rs 0.21 never does, so on native only
    /// negotiated ids are checked).
    pub negotiated: Option<u16>,
    /// Bytes [`Channel::send`] may have buffered before it refuses with
    /// [`SendError::QueueFull`]. Default [`DEFAULT_SEND_BUFFER_LIMIT`], at
    /// most [`MAX_SEND_BUFFER_LIMIT`].
    pub send_buffer_limit: usize,
}

impl Default for ChannelOptions {
    fn default() -> Self {
        Self::new(Reliability::Reliable)
    }
}

impl ChannelOptions {
    /// An in-band channel (announced to the other side, which hears it in
    /// [`Peer::on_data_channel`]).
    pub fn new(reliability: Reliability) -> Self {
        Self {
            reliability,
            negotiated: None,
            send_buffer_limit: DEFAULT_SEND_BUFFER_LIMIT,
        }
    }

    /// A channel pre-negotiated by `id`.
    pub fn negotiated(id: u16, reliability: Reliability) -> Self {
        Self {
            negotiated: Some(id),
            ..Self::new(reliability)
        }
    }

    /// The same options with another send-buffer limit.
    pub fn with_send_buffer_limit(mut self, bytes: usize) -> Self {
        self.send_buffer_limit = bytes;
        self
    }
}

// -------------------------------------------------------------------- Peer

struct PeerInner {
    id: u64,
}

impl Drop for PeerInner {
    fn drop(&mut self) {
        backend::close(self.id, false);
        registry::remove_peer(self.id);
    }
}

/// One peer connection (`RTCPeerConnection`): one per remote party, any
/// number per app.
///
/// `!Send`: it lives on the main thread with its callbacks. Cheap to clone;
/// it closes when [`close`](Self::close) is called, when the last clone
/// drops, or, for a peer created during a render, when that component
/// unmounts (see the [crate docs](crate#lifetime-cancel-on-unmount-with-an-opt-out)).
#[derive(Clone)]
pub struct Peer {
    inner: Rc<PeerInner>,
}

/// A [`Peer`] that does not keep it alive; for callbacks that need their own
/// peer.
#[derive(Clone)]
pub struct WeakPeer(Weak<PeerInner>);

impl WeakPeer {
    /// The peer, if it is still alive.
    pub fn upgrade(&self) -> Option<Peer> {
        self.0.upgrade().map(|inner| Peer { inner })
    }
}

impl std::fmt::Debug for Peer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Peer")
            .field("id", &self.inner.id)
            .field("state", &self.state())
            .finish()
    }
}

impl Peer {
    /// A new peer. Fails for a malformed ICE server URL, or when the
    /// platform refuses (on native: the worker could not start).
    pub fn new(config: RtcConfig) -> Result<Peer, RtcError> {
        validate(&config)?;
        let id = next_id();
        registry::insert_peer(id);
        if let Err(e) = backend::new_peer(id, &config) {
            registry::remove_peer(id);
            return Err(e);
        }
        // Cancel on unmount (design record 1.5). One cleanup per peer: peers
        // are made once per connection, not per keystroke.
        rinch_core::reactive::on_cleanup(move || {
            backend::close(id, false);
            registry::remove_peer(id);
        });
        Ok(Peer {
            inner: Rc::new(PeerInner { id }),
        })
    }

    fn id(&self) -> u64 {
        self.inner.id
    }

    /// A handle that does not keep the peer alive.
    pub fn downgrade(&self) -> WeakPeer {
        WeakPeer(Rc::downgrade(&self.inner))
    }

    /// Where the offer/answer exchange stands. An offer still being made
    /// (`create_offer` called, not yet resolved) already counts as
    /// `HaveLocalOffer`, so in perfect negotiation an incoming offer
    /// collides exactly when this is not `Stable`; no "making an offer"
    /// flag of the app's own is needed (see the guide's "Glare").
    pub fn signaling_state(&self) -> SignalingState {
        registry::signaling_state(self.id())
    }

    /// The connection state. `Closed` once [`close`](Self::close) was called.
    pub fn state(&self) -> PeerState {
        with_peer(self.id(), |e| {
            if e.closing {
                PeerState::Closed
            } else {
                e.state
            }
        })
        .unwrap_or(PeerState::Closed)
    }

    fn live(&self) -> Result<(), RtcError> {
        match with_peer(self.id(), |e| e.closing) {
            Some(false) => Ok(()),
            _ => Err(RtcError::Closed),
        }
    }

    /// Create an offer and set it as the local description; resolves to the
    /// SDP to send. Queued now; the future waits for the result.
    ///
    /// After [`restart_ice`](Self::restart_ice), this offer restarts ICE.
    pub fn create_offer(&self) -> impl Future<Output = Result<Description, RtcError>> + 'static {
        let id = self.id();
        let queued = self.live().map(|()| {
            let restart = with_peer(id, |e| std::mem::take(&mut e.restart)).unwrap_or(false);
            let req = registry::new_request(id);
            registry::signaling_request(id, req, registry::SignalingOp::Offer);
            backend::create_offer(id, req, restart);
            req
        });
        async move {
            let req = queued?;
            expect_description(registry::reply(id, req).await)
        }
    }

    /// Create an answer to the remote offer and set it as the local
    /// description; resolves to the SDP to send.
    pub fn create_answer(&self) -> impl Future<Output = Result<Description, RtcError>> + 'static {
        let id = self.id();
        let queued = self.live().map(|()| {
            let req = registry::new_request(id);
            registry::signaling_request(id, req, registry::SignalingOp::Answer);
            backend::create_answer(id, req);
            req
        });
        async move {
            let req = queued?;
            expect_description(registry::reply(id, req).await)
        }
    }

    /// Set the other side's offer or answer. Once it has been applied,
    /// the remote candidates held so far are added, in the order they
    /// arrived; if it fails, they stay held for the next one.
    pub fn set_remote_description(
        &self,
        description: Description,
    ) -> impl Future<Output = Result<(), RtcError>> + 'static {
        let id = self.id();
        let queued = self.live().map(|()| {
            let req = registry::new_request(id);
            registry::signaling_request(id, req, registry::SignalingOp::Remote(description.kind));
            // Held candidates go out when this succeeds (registry::dispatch,
            // on its reply), whether or not the caller awaits the future. A
            // rollback applies no remote description.
            if description.kind != SdpKind::Rollback {
                let advertised = advertised_max_message_size(&description.sdp);
                with_peer(id, |e| e.remote_pending.push((req, advertised)));
            }
            backend::set_remote(id, req, description);
            req
        });
        async move {
            let req = queued?;
            registry::reply(id, req).await.map(|_| ())
        }
    }

    /// Add a remote candidate, or `None` for the remote end-of-candidates.
    /// Held until a remote description is set. A candidate the stack refuses
    /// is reported through [`on_error`](Self::on_error).
    pub fn add_ice_candidate(&self, candidate: Option<IceCandidate>) {
        let id = self.id();
        let forward = with_peer(id, |e| {
            if e.closing {
                None
            } else if e.has_remote {
                Some(candidate)
            } else {
                e.early.push(candidate);
                None
            }
        })
        .flatten();
        if let Some(c) = forward {
            backend::add_candidate(id, c);
        }
    }

    /// Make the next [`create_offer`](Self::create_offer) an ICE restart
    /// (new credentials, candidates regathered). Send that offer to the
    /// other side as usual; the channels survive a restart that succeeds.
    pub fn restart_ice(&self) {
        with_peer(self.id(), |e| e.restart = true);
    }

    /// Create a data channel. An in-band one (`negotiated: None`) is
    /// announced to the other side; declare at least one channel before
    /// [`create_offer`](Self::create_offer) so the offer carries the data
    /// section.
    pub fn create_data_channel(
        &self,
        label: &str,
        options: ChannelOptions,
    ) -> Result<Channel, RtcError> {
        self.live()?;
        let id = self.id();
        if let Some(sid) = options.negotiated {
            let taken = with_peer(id, |e| {
                e.channels
                    .values()
                    .any(|c| c.id == Some(sid) && c.state != ChannelState::Closed)
            });
            if taken == Some(true) {
                return Err(RtcError::InvalidState(format!(
                    "stream id {sid} is already used by another open channel on this peer"
                )));
            }
        }
        let key = next_id();
        with_peer(id, |e| {
            e.channels.insert(
                key,
                ChannelEntry::new(
                    label.to_string(),
                    options.negotiated,
                    options.send_buffer_limit.min(MAX_SEND_BUFFER_LIMIT),
                ),
            )
        });
        // The id the stack gave it, if it says at once (the web assigns an
        // in-band channel's as soon as the DTLS role is known), so the
        // duplicate check above sees it before the channel opens.
        if let Some(sid) = backend::create_channel(id, key, label.to_string(), options) {
            with_peer(id, |e| {
                if let Some(c) = e.channels.get_mut(&key)
                    && c.id.is_none()
                {
                    c.id = Some(sid);
                }
            });
        }
        Ok(Channel { peer: id, key })
    }

    /// Close the peer and every channel. Each open channel's `on_close` and
    /// then `on_connection_state(Closed)` fire once the stack has closed;
    /// TURN allocations are released.
    pub fn close(&self) {
        let id = self.id();
        let first = with_peer(id, |e| !std::mem::replace(&mut e.closing, true));
        if first == Some(true) {
            backend::close(id, true);
        }
    }

    /// A local candidate to send to the other side; `None` once gathering is
    /// complete (end-of-candidates). Not called with [`Gathering::Complete`].
    pub fn on_ice_candidate(&self, cb: impl FnMut(Option<IceCandidate>) + 'static) -> &Self {
        registry::install(self.id(), |e| Some(&mut e.cbs.ice_candidate), Box::new(cb));
        self
    }

    /// The connection state changed.
    pub fn on_connection_state(&self, cb: impl FnMut(PeerState) + 'static) -> &Self {
        registry::install(
            self.id(),
            |e| Some(&mut e.cbs.connection_state),
            Box::new(cb),
        );
        self
    }

    /// The other side opened an in-band channel. Keep the [`Channel`] to use
    /// it; dropping the handle does not close the channel.
    pub fn on_data_channel(&self, cb: impl FnMut(Channel) + 'static) -> &Self {
        registry::install(self.id(), |e| Some(&mut e.cbs.data_channel), Box::new(cb));
        self
    }

    /// ICE candidate errors (an unreachable STUN server, a TURN server
    /// refusing the credentials) and remote candidates the stack refused.
    /// Informational: [`on_connection_state`](Self::on_connection_state)
    /// says whether the peer still connects.
    pub fn on_error(&self, cb: impl FnMut(RtcError) + 'static) -> &Self {
        registry::install(self.id(), |e| Some(&mut e.cbs.error), Box::new(cb));
        self
    }
}

/// Close every peer now and wait (at most about half a second) until the
/// far ends have been told, closes already under way included. On the web
/// it closes every peer at once (the browser sends the closes; there is
/// nothing to wait for). Done
/// automatically at normal process exit on native (see the [crate
/// docs](crate#process-exit)); call it before `std::process::exit` on
/// Windows, or whenever the program is about to stop without returning
/// from `main`. Every peer then reads as `Closed`, silently (no callbacks
/// run for these closes), and a peer used after it does nothing.
pub fn shutdown() {
    backend::shutdown();
    // Every peer reads as Closed from here, as after a drop: no callbacks.
    registry::remove_all_peers();
}

fn expect_description(r: Result<Option<Description>, RtcError>) -> Result<Description, RtcError> {
    r?.ok_or_else(|| RtcError::Internal("the backend answered no description".into()))
}

/// The `a=max-message-size:` value in an SDP, if it has one.
fn advertised_max_message_size(sdp: &str) -> Option<usize> {
    sdp.lines()
        .find_map(|l| l.trim().strip_prefix("a=max-message-size:"))
        .and_then(|v| v.trim().parse().ok())
}

fn validate(config: &RtcConfig) -> Result<(), RtcError> {
    for server in &config.ice_servers {
        if server.urls.is_empty() {
            return Err(RtcError::InvalidState("an ICE server has no URL".into()));
        }
        for url in &server.urls {
            let scheme = url.split(':').next().unwrap_or("");
            if !matches!(scheme, "stun" | "stuns" | "turn" | "turns") || !url.contains(':') {
                return Err(RtcError::InvalidState(format!(
                    "ICE server URL {url:?} is not stun:, stuns:, turn: or turns:"
                )));
            }
            if scheme.starts_with("turn")
                && (server.username.is_none() || server.credential.is_none())
            {
                return Err(RtcError::InvalidState(format!(
                    "TURN server {url:?} needs a username and a credential"
                )));
            }
        }
    }
    Ok(())
}

// ----------------------------------------------------------------- Channel

/// One data channel (`RTCDataChannel`). Cheap to clone. Dropping a handle
/// does not close the channel: it lives as long as its peer, or until
/// [`close`](Self::close).
#[derive(Clone, PartialEq, Eq)]
pub struct Channel {
    peer: u64,
    key: u64,
}

impl std::fmt::Debug for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Channel")
            .field("label", &self.label())
            .field("state", &self.state())
            .finish()
    }
}

impl Channel {
    pub(crate) fn from_parts(peer: u64, key: u64) -> Self {
        Self { peer, key }
    }

    fn with<R>(&self, f: impl FnOnce(&mut ChannelEntry) -> R) -> Option<R> {
        with_peer(self.peer, |e| e.channels.get_mut(&self.key).map(f)).flatten()
    }

    /// The label it was created with.
    pub fn label(&self) -> String {
        self.with(|c| c.label.clone()).unwrap_or_default()
    }

    /// The SCTP stream id. A pre-negotiated channel's is its `negotiated`
    /// id. An in-band channel's is reported on the web once it opens, and
    /// not at all on native (webrtc-rs 0.21 does not expose it): `None`
    /// there.
    pub fn id(&self) -> Option<u16> {
        self.with(|c| c.id).flatten()
    }

    /// The channel's state; `Closed` once its peer is gone.
    pub fn state(&self) -> ChannelState {
        self.with(|c| c.state).unwrap_or(ChannelState::Closed)
    }

    /// The largest message the other side accepts: its SDP's
    /// `a=max-message-size` (RFC 8841), capped by what the local stack
    /// supports. [`DEFAULT_MAX_MESSAGE_SIZE`] until the channel opens, and
    /// when the remote SDP does not say, on both backends.
    pub fn max_message_size(&self) -> usize {
        self.with(|c| c.max_message_size)
            .unwrap_or(DEFAULT_MAX_MESSAGE_SIZE)
    }

    /// Bytes queued and not yet sent. On native this is kept on the main
    /// thread, counted up by `send` and corrected by the worker after each
    /// write and at the low-water mark, so it can overstate, never
    /// understate.
    pub fn buffered_amount(&self) -> usize {
        self.with(|c| c.buffered()).unwrap_or(0)
    }

    /// The buffered amount at or below which
    /// [`on_buffered_amount_low`](Self::on_buffered_amount_low) fires.
    /// Default 0 (fires when the queue drains).
    pub fn set_buffered_amount_low_threshold(&self, bytes: usize) {
        if self.with(|c| c.low_threshold = bytes).is_some() {
            backend::set_low_threshold(self.peer, self.key, bytes);
        }
    }

    /// Queue a message. Refused, and nothing queued, when the channel is
    /// not open, when the message exceeds
    /// [`max_message_size`](Self::max_message_size), or when it would take
    /// [`buffered_amount`](Self::buffered_amount) past the channel's
    /// `send_buffer_limit` (a message onto an empty queue is always
    /// admitted, so one larger than the limit can still go).
    pub fn send(&self, bytes: impl Into<Vec<u8>>) -> Result<(), SendError> {
        let bytes = bytes.into();
        let len = bytes.len();
        self.with(|c| {
            if c.state != ChannelState::Open {
                return Err(SendError::NotOpen);
            }
            if len > c.max_message_size {
                return Err(SendError::TooLarge {
                    len,
                    max: c.max_message_size,
                });
            }
            let buffered = c.buffered();
            if buffered != 0 && buffered + len > c.limit {
                return Err(SendError::QueueFull {
                    buffered,
                    limit: c.limit,
                });
            }
            c.pending += len;
            Ok(())
        })
        .unwrap_or(Err(SendError::NotOpen))?;
        backend::send(self.peer, self.key, bytes);
        Ok(())
    }

    /// Close this channel; its `on_close` fires once it has closed.
    pub fn close(&self) {
        let first = self.with(|c| {
            let open = matches!(c.state, ChannelState::Connecting | ChannelState::Open);
            if open {
                c.state = ChannelState::Closing;
            }
            open
        });
        if first == Some(true) {
            backend::close_channel(self.peer, self.key);
        }
    }

    fn install<T: 'static>(
        &self,
        pick: fn(&mut registry::ChannelCallbacks) -> &mut Option<registry::Slot<T>>,
        cb: Box<dyn FnMut(T)>,
    ) {
        let key = self.key;
        registry::install(
            self.peer,
            move |e| e.channels.get_mut(&key).map(|c| pick(&mut c.cbs)),
            cb,
        );
    }

    /// The channel opened.
    pub fn on_open(&self, mut cb: impl FnMut() + 'static) -> &Self {
        self.install(|c| &mut c.open, Box::new(move |()| cb()));
        self
    }

    /// A message arrived. A text message from a browser arrives as its
    /// UTF-8 bytes.
    pub fn on_message(&self, cb: impl FnMut(Vec<u8>) + 'static) -> &Self {
        self.install(|c| &mut c.message, Box::new(cb));
        self
    }

    /// The channel closed (either side closed it, or the peer closed). Fires
    /// once.
    pub fn on_close(&self, mut cb: impl FnMut() + 'static) -> &Self {
        self.install(|c| &mut c.close, Box::new(move |()| cb()));
        self
    }

    /// The channel failed; `on_close` follows.
    pub fn on_error(&self, cb: impl FnMut(RtcError) + 'static) -> &Self {
        self.install(|c| &mut c.error, Box::new(cb));
        self
    }

    /// The buffered amount fell to the low-water threshold or below.
    pub fn on_buffered_amount_low(&self, mut cb: impl FnMut() + 'static) -> &Self {
        self.install(|c| &mut c.buffered_amount_low, Box::new(move |()| cb()));
        self
    }
}

#[doc(hidden)]
pub mod testing {
    //! Hooks for rinch-rtc's own tests. Not API.
    #[cfg(not(target_arch = "wasm32"))]
    pub use crate::native::testing::*;
    #[cfg(all(feature = "media-core", not(target_arch = "wasm32")))]
    pub use crate::native_media::testing::*;
    #[cfg(target_arch = "wasm32")]
    pub use crate::wasm::testing::*;
    #[cfg(all(feature = "media-core", target_arch = "wasm32"))]
    pub use crate::wasm_media::testing::*;
}

/// A channel's settings as the stack holds them (test hook,
/// `testing::channel_settings`).
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelSettings {
    pub ordered: bool,
    pub max_retransmits: Option<u16>,
    pub negotiated: bool,
}

#[cfg(test)]
mod tests;
