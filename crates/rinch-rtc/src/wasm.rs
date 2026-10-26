//! Web backend: the browser's `RTCPeerConnection` and `RTCDataChannel`.
//!
//! Everything runs on the one web thread, so the JS event closures call
//! [`registry::dispatch`] directly. The closures are owned here, detached
//! (`set_on*(None)`, `remove_event_listener`) before they drop, as in
//! rinch-ws's `wasm.rs`.
//!
//! The signalling calls go through a per-peer queue worked by one local task,
//! the native backend's shape: the browser's own operations chain orders
//! the calls it has been *given*, and `create_offer`'s
//! `setLocalDescription` is only given once its `createOffer` resolved, so a
//! `set_remote_description` made meanwhile would otherwise overtake it.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::task::{Poll, Waker};

use js_sys::{Array, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Event as DomEvent, MessageEvent, RtcConfiguration, RtcDataChannel, RtcDataChannelEvent,
    RtcDataChannelInit, RtcDataChannelType, RtcIceCandidateInit, RtcIceGatheringState,
    RtcIceServer, RtcIceTransportPolicy, RtcOfferOptions, RtcPeerConnection,
    RtcPeerConnectionIceErrorEvent, RtcPeerConnectionIceEvent, RtcPeerConnectionState, RtcSdpType,
    RtcSessionDescriptionInit,
};

use crate::registry::{self, Event};
use crate::{
    ChannelOptions, Description, Gathering, IceCandidate, IceTransportPolicy, PeerState,
    Reliability, RtcConfig, RtcError, SdpKind,
};

/// A signalling call, worked in order by the peer's task.
enum Op {
    Offer { req: u64, restart: bool },
    Answer { req: u64 },
    Remote { req: u64, description: Description },
    Candidate(Option<IceCandidate>),
}

#[derive(Default)]
struct OpQueue {
    ops: VecDeque<Op>,
    waker: Option<Waker>,
    closed: bool,
}

type Queue = Rc<RefCell<OpQueue>>;
type JsCallback = Closure<dyn FnMut(JsValue)>;

struct WebChannel {
    dc: RtcDataChannel,
    _closures: Vec<Closure<dyn FnMut(JsValue)>>,
}

impl WebChannel {
    fn detach(&self) {
        self.dc.set_onopen(None);
        self.dc.set_onmessage(None);
        self.dc.set_onclose(None);
        self.dc.set_onerror(None);
        self.dc.set_onbufferedamountlow(None);
    }
}

type Channels = Rc<RefCell<HashMap<u64, WebChannel>>>;

/// A `Gathering::Complete` wait in progress: its promise's `resolve` and
/// its `icegatheringstatechange` listener. Taken (and the listener removed)
/// when gathering completes or the peer closes, whichever comes first.
type GatherWait = Rc<RefCell<Option<(js_sys::Function, JsCallback)>>>;

struct WebPeer {
    pc: RtcPeerConnection,
    queue: Queue,
    channels: Channels,
    gather: GatherWait,
    _closures: Vec<Closure<dyn FnMut(JsValue)>>,
    on_ice_error: Closure<dyn FnMut(JsValue)>,
}

thread_local! {
    static PEERS: RefCell<HashMap<u64, WebPeer>> = RefCell::new(HashMap::new());
    /// Gathering waits not yet settled (test hook).
    static GATHER_WAITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// `createDataChannel`, catching: web-sys's binding does not, so a throw
/// (a label over 65535 bytes, id 65535, an id in use) unwound out of wasm
/// with no destructor run. `Function::call2` returns the exception as `Err`.
fn create_data_channel_catching(
    pc: &RtcPeerConnection,
    label: &str,
    init: &RtcDataChannelInit,
) -> Result<RtcDataChannel, JsValue> {
    let f: js_sys::Function = Reflect::get(pc, &"createDataChannel".into())?.dyn_into()?;
    f.call2(pc, &JsValue::from_str(label), init)
        .map(|dc| dc.unchecked_into())
}

/// Settle a gathering wait: remove its listener, resolve its promise, and
/// drop the listener after the current call (it may be the caller).
fn settle_gather(pc: &RtcPeerConnection, gather: &GatherWait) {
    let taken = gather.borrow_mut().take();
    if let Some((resolve, cb)) = taken {
        let _ = pc.remove_event_listener_with_callback(
            "icegatheringstatechange",
            cb.as_ref().unchecked_ref(),
        );
        let _ = resolve.call0(&JsValue::NULL);
        GATHER_WAITS.with(|n| n.set(n.get().saturating_sub(1)));
        wasm_bindgen_futures::spawn_local(async move { drop(cb) });
    }
}

pub mod testing {
    //! Hooks for rinch-rtc's own tests (`#[doc(hidden)]`, not API).
    use wasm_bindgen::JsValue;

    pub use crate::ChannelSettings;

    /// The channel's settings as the browser's RTCDataChannel holds them.
    pub fn channel_settings(channel: &crate::Channel) -> Option<ChannelSettings> {
        super::with_channel(channel.peer, channel.key, |dc| {
            let get = |k: &str| js_sys::Reflect::get(dc, &JsValue::from_str(k)).ok();
            ChannelSettings {
                ordered: get("ordered").and_then(|v| v.as_bool()).unwrap_or(true),
                max_retransmits: dc.max_retransmits(),
                negotiated: get("negotiated").and_then(|v| v.as_bool()).unwrap_or(false),
            }
        })
    }

    /// The browser's own `bufferedAmountLowThreshold` for the channel.
    pub fn browser_low_threshold(channel: &crate::Channel) -> Option<u32> {
        super::with_channel(channel.peer, channel.key, |dc| {
            dc.buffered_amount_low_threshold()
        })
    }

    /// `Gathering::Complete` waits not yet settled, over every peer.
    pub fn pending_gathering_waits() -> usize {
        super::GATHER_WAITS.with(|n| n.get())
    }

    /// Channels whose JS closures are still held, over every peer.
    pub fn live_channels() -> usize {
        super::PEERS.with(|p| p.borrow().values().map(|w| w.channels.borrow().len()).sum())
    }
}

fn js_err(e: &JsValue) -> String {
    e.as_string()
        .or_else(|| {
            Reflect::get(e, &"message".into())
                .ok()
                .and_then(|m| m.as_string())
        })
        .unwrap_or_else(|| format!("{e:?}"))
}

fn with_pc<R>(id: u64, f: impl FnOnce(&WebPeer) -> R) -> Option<R> {
    PEERS.with(|p| p.borrow().get(&id).map(f))
}

/// Run `event` for peer `id` after the current call returns, so no callback
/// runs inside the API call that caused it.
fn defer(id: u64, event: Event) {
    wasm_bindgen_futures::spawn_local(async move { registry::dispatch(id, event) });
}

// ------------------------------------------------- the main-thread surface

pub(crate) fn new_peer(id: u64, config: &RtcConfig) -> Result<(), RtcError> {
    let rtc_config = RtcConfiguration::new();
    let servers = Array::new();
    for s in &config.ice_servers {
        let server = RtcIceServer::new();
        let urls: Array = s.urls.iter().map(|u| JsValue::from_str(u)).collect();
        server.set_urls(&urls);
        if let Some(u) = &s.username {
            server.set_username(u);
        }
        if let Some(c) = &s.credential {
            server.set_credential(c);
        }
        servers.push(&server);
    }
    rtc_config.set_ice_servers(&servers);
    rtc_config.set_ice_transport_policy(match config.ice_transport_policy {
        IceTransportPolicy::All => RtcIceTransportPolicy::All,
        IceTransportPolicy::Relay => RtcIceTransportPolicy::Relay,
    });
    let pc = RtcPeerConnection::new_with_configuration(&rtc_config)
        .map_err(|e| RtcError::InvalidState(js_err(&e)))?;

    let trickle = config.gathering == Gathering::Trickle;
    let channels: Channels = Rc::default();
    let gather: GatherWait = Rc::default();

    let on_candidate = Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| {
        if !trickle {
            return;
        }
        let Ok(e) = e.dyn_into::<RtcPeerConnectionIceEvent>() else {
            return;
        };
        // The null candidate is end-of-candidates. Some engines also send an
        // empty-string candidate first ("end of this generation"); that one
        // is not passed on.
        match e.candidate() {
            Some(c) if c.candidate().is_empty() => {}
            Some(c) => registry::dispatch(
                id,
                Event::Candidate(Some(IceCandidate {
                    candidate: c.candidate(),
                    sdp_mid: c.sdp_mid(),
                    sdp_m_line_index: c.sdp_m_line_index(),
                })),
            ),
            None => registry::dispatch(id, Event::Candidate(None)),
        }
    });
    pc.set_onicecandidate(Some(on_candidate.as_ref().unchecked_ref()));

    let pc2 = pc.clone();
    let on_state = Closure::<dyn FnMut(JsValue)>::new(move |_e: JsValue| {
        let state = match pc2.connection_state() {
            RtcPeerConnectionState::New => PeerState::New,
            RtcPeerConnectionState::Connecting => PeerState::Connecting,
            RtcPeerConnectionState::Connected => PeerState::Connected,
            RtcPeerConnectionState::Disconnected => PeerState::Disconnected,
            RtcPeerConnectionState::Failed => PeerState::Failed,
            RtcPeerConnectionState::Closed => PeerState::Closed,
            _ => return,
        };
        registry::dispatch(id, Event::State(state));
    });
    pc.set_onconnectionstatechange(Some(on_state.as_ref().unchecked_ref()));

    let (pc3, chans) = (pc.clone(), channels.clone());
    let on_channel = Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| {
        let Ok(e) = e.dyn_into::<RtcDataChannelEvent>() else {
            return;
        };
        let dc = e.channel();
        let key = crate::next_id();
        // Wired and kept *before* the app hears of it: a call made from
        // inside on_data_channel (close, a low-water threshold) must find it.
        let ch = wire(id, key, dc.clone(), &pc3, &chans);
        chans.borrow_mut().insert(key, ch);
        registry::dispatch(
            id,
            Event::Incoming {
                key,
                label: dc.label(),
                id: dc.id(),
            },
        );
        // An in-band channel is often already open when it is announced.
        if dc.ready_state() == web_sys::RtcDataChannelState::Open {
            registry::dispatch(
                id,
                Event::Open {
                    key,
                    id: dc.id(),
                    max_message_size: max_message_size(&pc3),
                },
            );
        }
    });
    pc.set_ondatachannel(Some(on_channel.as_ref().unchecked_ref()));

    // No setter in web-sys 0.3.103: an event listener.
    let on_ice_error = Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| {
        if let Ok(e) = e.dyn_into::<RtcPeerConnectionIceErrorEvent>() {
            registry::dispatch(
                id,
                Event::Error(RtcError::Ice {
                    url: e.url(),
                    code: e.error_code(),
                    text: e.error_text(),
                }),
            );
        }
    });
    let _ = pc.add_event_listener_with_callback(
        "icecandidateerror",
        on_ice_error.as_ref().unchecked_ref(),
    );

    let queue: Queue = Rc::default();
    wasm_bindgen_futures::spawn_local(work(id, pc.clone(), queue.clone(), trickle, gather.clone()));

    #[cfg(feature = "media")]
    crate::wasm_media::attach(id, &pc);

    PEERS.with(|p| {
        p.borrow_mut().insert(
            id,
            WebPeer {
                pc,
                queue,
                channels,
                gather,
                _closures: vec![on_candidate, on_state, on_channel],
                on_ice_error,
            },
        )
    });
    Ok(())
}

fn push(id: u64, op: Op) {
    let queued = with_pc(id, |p| {
        let mut q = p.queue.borrow_mut();
        q.ops.push_back(op);
        q.waker.take()
    });
    if let Some(Some(w)) = queued {
        w.wake();
    }
}

pub(crate) fn create_offer(id: u64, req: u64, restart: bool) {
    push(id, Op::Offer { req, restart });
}

pub(crate) fn create_answer(id: u64, req: u64) {
    push(id, Op::Answer { req });
}

pub(crate) fn set_remote(id: u64, req: u64, description: Description) {
    push(id, Op::Remote { req, description });
}

pub(crate) fn add_candidate(id: u64, candidate: Option<IceCandidate>) {
    push(id, Op::Candidate(candidate));
}

/// Returns the stream id the browser gave the channel, when it has one
/// already (a negotiated channel's, or an in-band one's once the DTLS role
/// is known).
pub(crate) fn create_channel(
    id: u64,
    key: u64,
    label: String,
    options: ChannelOptions,
) -> Option<u16> {
    let (pc, chans) = with_pc(id, |p| (p.pc.clone(), p.channels.clone()))?;
    let init = RtcDataChannelInit::new();
    match options.reliability {
        Reliability::Reliable => init.set_ordered(true),
        Reliability::Unreliable { max_retransmits } => {
            init.set_ordered(false);
            init.set_max_retransmits(max_retransmits);
        }
    }
    if let Some(sid) = options.negotiated {
        init.set_negotiated(true);
        init.set_id(sid);
    }
    match create_data_channel_catching(&pc, &label, &init) {
        Ok(dc) => {
            let sid = dc.id();
            let ch = wire(id, key, dc, &pc, &chans);
            chans.borrow_mut().insert(key, ch);
            sid
        }
        // As native reports a channel the stack refused: an error, then the
        // close, after the call returns.
        Err(e) => {
            let error = RtcError::Channel(js_err(&e));
            wasm_bindgen_futures::spawn_local(async move {
                registry::dispatch(id, Event::ChannelError { key, error });
                registry::dispatch(id, Event::ChannelClosed { key });
            });
            None
        }
    }
}

/// Attach the channel's event closures; they report to the registry.
fn wire(
    id: u64,
    key: u64,
    dc: RtcDataChannel,
    pc: &RtcPeerConnection,
    chans: &Channels,
) -> WebChannel {
    dc.set_binary_type(RtcDataChannelType::Arraybuffer);
    let (pc, dc2) = (pc.clone(), dc.clone());
    let on_open = Closure::<dyn FnMut(JsValue)>::new(move |_e: JsValue| {
        registry::dispatch(
            id,
            Event::Open {
                key,
                id: dc2.id(),
                max_message_size: max_message_size(&pc),
            },
        );
    });
    let on_message = Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| {
        let Ok(e) = e.dyn_into::<MessageEvent>() else {
            return;
        };
        let data = e.data();
        let bytes = if let Some(text) = data.as_string() {
            text.into_bytes()
        } else if data.is_instance_of::<js_sys::ArrayBuffer>() {
            js_sys::Uint8Array::new(&data).to_vec()
        } else {
            // binaryType is ArrayBuffer, so a Blob is not produced.
            return;
        };
        registry::dispatch(id, Event::Message { key, data: bytes });
    });
    let chans = chans.clone();
    let on_close = Closure::<dyn FnMut(JsValue)>::new(move |_e: JsValue| {
        registry::dispatch(id, Event::ChannelClosed { key });
        // This channel's closures go now, not with the peer: detached, and
        // dropped after this call (this closure is one of them).
        let chans = chans.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let gone = chans.borrow_mut().remove(&key);
            if let Some(ch) = gone {
                ch.detach();
            }
        });
    });
    let on_error = Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| {
        let text = e
            .dyn_ref::<DomEvent>()
            .and_then(|e| Reflect::get(e, &"error".into()).ok())
            .map(|err| js_err(&err))
            .unwrap_or_else(|| "the data channel reported an error".into());
        registry::dispatch(
            id,
            Event::ChannelError {
                key,
                error: RtcError::Channel(text),
            },
        );
    });
    let dc3 = dc.clone();
    let on_low = Closure::<dyn FnMut(JsValue)>::new(move |_e: JsValue| {
        registry::dispatch(
            id,
            Event::Low {
                key,
                amount: dc3.buffered_amount() as usize,
            },
        );
    });
    dc.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    dc.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    dc.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    dc.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    dc.set_onbufferedamountlow(Some(on_low.as_ref().unchecked_ref()));
    WebChannel {
        dc,
        _closures: vec![on_open, on_message, on_close, on_error, on_low],
    }
}

/// `pc.sctp.maxMessageSize` (no binding in web-sys 0.3.103), or RFC 8841's
/// default when the engine does not say.
fn max_message_size(pc: &RtcPeerConnection) -> usize {
    Reflect::get(pc, &"sctp".into())
        .ok()
        .filter(|s| !s.is_null() && !s.is_undefined())
        .and_then(|s| Reflect::get(&s, &"maxMessageSize".into()).ok())
        .and_then(|m| m.as_f64())
        .filter(|m| *m > 0.0)
        .map(|m| {
            if m.is_finite() {
                m as usize
            } else {
                usize::MAX
            }
        })
        .unwrap_or(crate::DEFAULT_MAX_MESSAGE_SIZE)
}

fn with_channel<R>(id: u64, key: u64, f: impl FnOnce(&RtcDataChannel) -> R) -> Option<R> {
    with_pc(id, |p| p.channels.borrow().get(&key).map(|c| f(&c.dc))).flatten()
}

pub(crate) fn send(id: u64, key: u64, bytes: Vec<u8>) {
    let sent = bytes.len();
    let result = with_channel(id, key, |dc| {
        dc.send_with_u8_array(&bytes)
            .map(|()| dc.buffered_amount() as usize)
    });
    match result {
        // The browser's own count: the mirror is exact on the web.
        Some(Ok(amount)) => registry::dispatch(id, Event::Buffered { key, sent, amount }),
        Some(Err(e)) => {
            registry::dispatch(
                id,
                Event::Buffered {
                    key,
                    sent,
                    amount: with_channel(id, key, |dc| dc.buffered_amount() as usize).unwrap_or(0),
                },
            );
            defer(
                id,
                Event::ChannelError {
                    key,
                    error: RtcError::Channel(js_err(&e)),
                },
            );
        }
        None => {}
    }
}

pub(crate) fn set_low_threshold(id: u64, key: u64, bytes: usize) {
    with_channel(id, key, |dc| {
        dc.set_buffered_amount_low_threshold(u32::try_from(bytes).unwrap_or(u32::MAX))
    });
}

pub(crate) fn close_channel(id: u64, key: u64) {
    with_channel(id, key, |dc| dc.close());
}

/// Close peer `id`, detaching every closure first so no late browser event
/// reaches a dropped one. `notify`: report the final [`Event::Closed`], which
/// closes the channels in the registry (the browser fires no events once
/// detached).
pub(crate) fn close(id: u64, notify: bool) {
    let peer = PEERS.with(|p| p.borrow_mut().remove(&id));
    let Some(peer) = peer else {
        return;
    };
    peer.pc.set_onicecandidate(None);
    peer.pc.set_onconnectionstatechange(None);
    peer.pc.set_ondatachannel(None);
    let _ = peer.pc.remove_event_listener_with_callback(
        "icecandidateerror",
        peer.on_ice_error.as_ref().unchecked_ref(),
    );
    for ch in peer.channels.borrow().values() {
        ch.detach();
    }
    // A Gathering::Complete wait in progress: settle it, so its task ends
    // and its listener goes.
    settle_gather(&peer.pc, &peer.gather);
    #[cfg(feature = "media")]
    crate::wasm_media::detach(id);
    peer.pc.close();
    {
        let mut q = peer.queue.borrow_mut();
        q.closed = true;
        if let Some(w) = q.waker.take() {
            w.wake();
        }
    }
    if notify {
        defer(id, Event::Closed);
    }
}

/// Close every peer, silently (see `crate::shutdown`).
pub(crate) fn shutdown() {
    let ids: Vec<u64> = PEERS.with(|p| p.borrow().keys().copied().collect());
    for id in ids {
        close(id, false);
    }
}

// ------------------------------------------------------- the peer's task

async fn next_op(queue: &Queue) -> Option<Op> {
    std::future::poll_fn(|cx| {
        let mut q = queue.borrow_mut();
        if q.closed {
            return Poll::Ready(None);
        }
        match q.ops.pop_front() {
            Some(op) => Poll::Ready(Some(op)),
            None => {
                q.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    })
    .await
}

async fn work(id: u64, pc: RtcPeerConnection, queue: Queue, trickle: bool, gather: GatherWait) {
    while let Some(op) = next_op(&queue).await {
        match op {
            Op::Offer { req, restart } => {
                let result = describe(&pc, SdpKind::Offer, restart, trickle, &gather).await;
                registry::dispatch(
                    id,
                    Event::Reply {
                        req,
                        result: result.map(Some),
                    },
                );
            }
            Op::Answer { req } => {
                let result = describe(&pc, SdpKind::Answer, false, trickle, &gather).await;
                registry::dispatch(
                    id,
                    Event::Reply {
                        req,
                        result: result.map(Some),
                    },
                );
            }
            Op::Remote { req, description } => {
                let rollback = description.kind == SdpKind::Rollback;
                let init = RtcSessionDescriptionInit::new(match description.kind {
                    SdpKind::Offer => RtcSdpType::Offer,
                    SdpKind::Answer => RtcSdpType::Answer,
                    _ => RtcSdpType::Rollback,
                });
                if !rollback {
                    init.set_sdp(&description.sdp);
                }
                // A rollback undoes whichever offer is pending: ours through
                // setLocalDescription, theirs through setRemoteDescription.
                let promise = if rollback
                    && pc.signaling_state() == web_sys::RtcSignalingState::HaveLocalOffer
                {
                    pc.set_local_description(&init)
                } else {
                    pc.set_remote_description(&init)
                };
                let result = JsFuture::from(promise)
                    .await
                    .map(|_| None)
                    .map_err(|e| RtcError::Sdp(js_err(&e)));
                registry::dispatch(id, Event::Reply { req, result });
            }
            Op::Candidate(c) => {
                let promise = match &c {
                    Some(c) => {
                        let init = RtcIceCandidateInit::new(&c.candidate);
                        init.set_sdp_mid(c.sdp_mid.as_deref());
                        init.set_sdp_m_line_index(c.sdp_m_line_index);
                        pc.add_ice_candidate_with_opt_rtc_ice_candidate_init(Some(&init))
                    }
                    None => pc.add_ice_candidate_with_opt_rtc_ice_candidate_init(None),
                };
                if let Err(e) = JsFuture::from(promise).await {
                    registry::dispatch(id, Event::Error(RtcError::Candidate(js_err(&e))));
                }
            }
        }
    }
}

/// createOffer/createAnswer + setLocalDescription; with Complete gathering,
/// wait for it to finish and return the SDP that carries the candidates.
async fn describe(
    pc: &RtcPeerConnection,
    kind: SdpKind,
    restart: bool,
    trickle: bool,
    gather: &GatherWait,
) -> Result<Description, RtcError> {
    let created = match kind {
        SdpKind::Offer => {
            let opts = RtcOfferOptions::new();
            opts.set_ice_restart(restart);
            pc.create_offer_with_rtc_offer_options(&opts)
        }
        _ => pc.create_answer(),
    };
    let desc = JsFuture::from(created)
        .await
        .map_err(|e| RtcError::InvalidState(js_err(&e)))?;
    let init: RtcSessionDescriptionInit = desc.unchecked_into();
    JsFuture::from(pc.set_local_description(&init))
        .await
        .map_err(|e| RtcError::Sdp(js_err(&e)))?;
    if !trickle {
        gathering_complete(pc, gather).await;
    }
    let sdp = pc
        .local_description()
        .map(|d| d.sdp())
        .or_else(|| init.get_sdp())
        .unwrap_or_default();
    Ok(Description { kind, sdp })
}

/// Resolve once `iceGatheringState` is `complete`, or once the peer closes
/// (`close` settles the wait).
async fn gathering_complete(pc: &RtcPeerConnection, gather: &GatherWait) {
    if pc.ice_gathering_state() == RtcIceGatheringState::Complete {
        return;
    }
    let (pc2, slot) = (pc.clone(), gather.clone());
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let (pc3, slot3) = (pc2.clone(), slot.clone());
        let cb = Closure::<dyn FnMut(JsValue)>::new(move |_e: JsValue| {
            if pc3.ice_gathering_state() == RtcIceGatheringState::Complete {
                settle_gather(&pc3, &slot3);
            }
        });
        let _ = pc2.add_event_listener_with_callback(
            "icegatheringstatechange",
            cb.as_ref().unchecked_ref(),
        );
        // One wait per peer at a time: the peer's ops run in order.
        *slot.borrow_mut() = Some((resolve, cb));
        GATHER_WAITS.with(|n| n.set(n.get() + 1));
    });
    let _ = JsFuture::from(promise).await;
}
