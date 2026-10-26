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

struct WebPeer {
    pc: RtcPeerConnection,
    queue: Queue,
    channels: Rc<RefCell<HashMap<u64, WebChannel>>>,
    _closures: Vec<Closure<dyn FnMut(JsValue)>>,
    on_ice_error: Closure<dyn FnMut(JsValue)>,
}

thread_local! {
    static PEERS: RefCell<HashMap<u64, WebPeer>> = RefCell::new(HashMap::new());
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
    let channels: Rc<RefCell<HashMap<u64, WebChannel>>> = Rc::default();

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
        registry::dispatch(
            id,
            Event::Incoming {
                key,
                label: dc.label(),
                id: dc.id(),
            },
        );
        let ch = wire(id, key, dc.clone(), &pc3);
        chans.borrow_mut().insert(key, ch);
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
    wasm_bindgen_futures::spawn_local(work(id, pc.clone(), queue.clone(), trickle));

    PEERS.with(|p| {
        p.borrow_mut().insert(
            id,
            WebPeer {
                pc,
                queue,
                channels,
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

pub(crate) fn create_channel(id: u64, key: u64, label: String, options: ChannelOptions) {
    let Some(pc) = with_pc(id, |p| p.pc.clone()) else {
        return;
    };
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
    let dc = pc.create_data_channel_with_data_channel_dict(&label, &init);
    let ch = wire(id, key, dc, &pc);
    with_pc(id, |p| p.channels.borrow_mut().insert(key, ch));
}

/// Attach the channel's event closures; they report to the registry.
fn wire(id: u64, key: u64, dc: RtcDataChannel, pc: &RtcPeerConnection) -> WebChannel {
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
    let on_close = Closure::<dyn FnMut(JsValue)>::new(move |_e: JsValue| {
        registry::dispatch(id, Event::ChannelClosed { key });
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

async fn work(id: u64, pc: RtcPeerConnection, queue: Queue, trickle: bool) {
    while let Some(op) = next_op(&queue).await {
        match op {
            Op::Offer { req, restart } => {
                let result = describe(&pc, SdpKind::Offer, restart, trickle).await;
                registry::dispatch(
                    id,
                    Event::Reply {
                        req,
                        result: result.map(Some),
                    },
                );
            }
            Op::Answer { req } => {
                let result = describe(&pc, SdpKind::Answer, false, trickle).await;
                registry::dispatch(
                    id,
                    Event::Reply {
                        req,
                        result: result.map(Some),
                    },
                );
            }
            Op::Remote { req, description } => {
                let init = RtcSessionDescriptionInit::new(match description.kind {
                    SdpKind::Offer => RtcSdpType::Offer,
                    SdpKind::Answer => RtcSdpType::Answer,
                });
                init.set_sdp(&description.sdp);
                let result = JsFuture::from(pc.set_remote_description(&init))
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
) -> Result<Description, RtcError> {
    let created = match kind {
        SdpKind::Offer => {
            let opts = RtcOfferOptions::new();
            opts.set_ice_restart(restart);
            pc.create_offer_with_rtc_offer_options(&opts)
        }
        SdpKind::Answer => pc.create_answer(),
    };
    let desc = JsFuture::from(created)
        .await
        .map_err(|e| RtcError::InvalidState(js_err(&e)))?;
    let init: RtcSessionDescriptionInit = desc.unchecked_into();
    JsFuture::from(pc.set_local_description(&init))
        .await
        .map_err(|e| RtcError::Sdp(js_err(&e)))?;
    if !trickle {
        gathering_complete(pc).await;
    }
    let sdp = pc
        .local_description()
        .map(|d| d.sdp())
        .or_else(|| init.get_sdp())
        .unwrap_or_default();
    Ok(Description { kind, sdp })
}

/// Resolve once `iceGatheringState` is `complete`.
async fn gathering_complete(pc: &RtcPeerConnection) {
    if pc.ice_gathering_state() == RtcIceGatheringState::Complete {
        return;
    }
    let pc2 = pc.clone();
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let pc3 = pc2.clone();
        let listener: Rc<RefCell<Option<JsCallback>>> = Rc::default();
        let l2 = listener.clone();
        let cb = Closure::<dyn FnMut(JsValue)>::new(move |_e: JsValue| {
            if pc3.ice_gathering_state() == RtcIceGatheringState::Complete {
                let _ = resolve.call0(&JsValue::NULL);
                if let Some(cb) = l2.borrow_mut().take() {
                    let _ = pc3.remove_event_listener_with_callback(
                        "icegatheringstatechange",
                        cb.as_ref().unchecked_ref(),
                    );
                    // Dropped after this call returns, not inside it.
                    wasm_bindgen_futures::spawn_local(async move { drop(cb) });
                }
            }
        });
        let _ = pc2.add_event_listener_with_callback(
            "icegatheringstatechange",
            cb.as_ref().unchecked_ref(),
        );
        *listener.borrow_mut() = Some(cb);
    });
    let _ = JsFuture::from(promise).await;
}
