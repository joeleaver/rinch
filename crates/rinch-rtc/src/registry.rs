//! The main-thread half shared by both backends: every peer's state, its
//! channels' state, the (`!Send`) callbacks, and the replies the async
//! signalling calls wait for.
//!
//! A backend reports what happened as an [`Event`] for a peer id and calls
//! [`dispatch`] on the main thread: directly from a JS closure on the web,
//! through `rinch_core::run_on_main_thread` from the native worker. Only ids
//! and `Send` payloads cross a thread.
//!
//! The callback slots are rinch-ws's (`crates/rinch-ws/src/lib.rs`): each
//! records its registering owner and `CallbackFrame`, is taken out of the
//! registry while it runs, is dropped and not put back once its owner has
//! gone (#183), and the first such prune sweeps the peer's other dead slots.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::poll_fn;
use std::task::{Poll, Waker};

use rinch_core::reactive::{Owner, current_owner, unowned};

use crate::{
    Channel, ChannelState, Description, IceCandidate, PeerState, RtcError, SdpKind, SignalingState,
};

/// What a backend reports for one peer. Plain data, `Send`.
#[derive(Debug)]
pub(crate) enum Event {
    /// The answer to the signalling request `req` (the description for an
    /// offer or answer, `None` for set_remote_description).
    Reply {
        req: u64,
        result: Result<Option<Description>, RtcError>,
    },
    /// A trickled local candidate; `None` is end-of-candidates.
    Candidate(Option<IceCandidate>),
    State(PeerState),
    Error(RtcError),
    /// The remote side opened an in-band channel; the backend has given it
    /// `key`.
    Incoming {
        key: u64,
        label: String,
        id: Option<u16>,
    },
    Open {
        key: u64,
        /// The SCTP stream id, when the stack reports it (the web does).
        id: Option<u16>,
        max_message_size: usize,
    },
    Message {
        key: u64,
        data: Vec<u8>,
    },
    ChannelError {
        key: u64,
        error: RtcError,
    },
    ChannelClosed {
        key: u64,
    },
    /// The backend queued `sent` bytes that `send` counted as pending, and
    /// the channel's buffered amount is now `amount`.
    Buffered {
        key: u64,
        sent: usize,
        amount: usize,
    },
    /// The buffered amount fell to the low-water threshold or below.
    Low {
        key: u64,
        amount: usize,
    },
    /// `close()` finished: every channel and the peer are closed. Last event.
    Closed,
    /// Something about the peer's media tracks (the `media` feature).
    #[cfg(feature = "media-core")]
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    Media(crate::media::MediaEvent),
}

/// One registered callback and the scope that registered it.
pub(crate) struct Slot<T> {
    owner: Option<Owner>,
    frame: rinch_core::CallbackFrame,
    cb: Box<dyn FnMut(T)>,
}

impl<T> Slot<T> {
    pub(crate) fn new(cb: Box<dyn FnMut(T)>) -> Self {
        Self {
            owner: current_owner(),
            frame: rinch_core::CallbackFrame::current(),
            cb,
        }
    }

    fn is_dead(&self) -> bool {
        self.owner.as_ref().is_some_and(|o| !o.is_alive())
    }
}

#[derive(Default)]
pub(crate) struct PeerCallbacks {
    pub(crate) ice_candidate: Option<Slot<Option<IceCandidate>>>,
    pub(crate) connection_state: Option<Slot<PeerState>>,
    pub(crate) data_channel: Option<Slot<Channel>>,
    pub(crate) error: Option<Slot<RtcError>>,
}

#[derive(Default)]
pub(crate) struct ChannelCallbacks {
    pub(crate) open: Option<Slot<()>>,
    pub(crate) message: Option<Slot<Vec<u8>>>,
    pub(crate) close: Option<Slot<()>>,
    pub(crate) error: Option<Slot<RtcError>>,
    pub(crate) buffered_amount_low: Option<Slot<()>>,
}

/// A channel as the main thread knows it.
pub(crate) struct ChannelEntry {
    pub(crate) label: String,
    pub(crate) id: Option<u16>,
    pub(crate) state: ChannelState,
    pub(crate) limit: usize,
    pub(crate) max_message_size: usize,
    /// The buffered amount the backend last reported.
    pub(crate) reported: usize,
    /// Bytes `send` handed to the backend that it has not yet reported
    /// queueing. `reported + pending` is the buffered amount: it can
    /// overstate, never understate (design record section 5).
    pub(crate) pending: usize,
    pub(crate) low_threshold: usize,
    pub(crate) cbs: ChannelCallbacks,
}

impl ChannelEntry {
    pub(crate) fn new(label: String, id: Option<u16>, limit: usize) -> Self {
        Self {
            label,
            id,
            state: ChannelState::Connecting,
            limit,
            max_message_size: crate::DEFAULT_MAX_MESSAGE_SIZE,
            reported: 0,
            pending: 0,
            low_threshold: 0,
            cbs: ChannelCallbacks::default(),
        }
    }

    pub(crate) fn buffered(&self) -> usize {
        self.reported + self.pending
    }
}

struct Reply {
    result: Option<Result<Option<Description>, RtcError>>,
    waker: Option<Waker>,
}

/// A peer as the main thread knows it.
pub(crate) struct PeerEntry {
    pub(crate) state: PeerState,
    /// A remote description has been applied. Until then remote candidates
    /// wait in `early`.
    pub(crate) has_remote: bool,
    /// The remote SDP's `a=max-message-size`: `None` until a remote
    /// description is set or when it has none (RFC 8841: 64 KiB then),
    /// `Some(0)` for "no limit".
    pub(crate) remote_max_message_size: Option<usize>,
    /// The `set_remote_description` calls in flight, each with the
    /// max-message-size its SDP advertises; each is applied on its own
    /// successful reply (overlapping calls do not overwrite each other).
    pub(crate) remote_pending: Vec<(u64, Option<usize>)>,
    pub(crate) early: Vec<Option<IceCandidate>>,
    /// `restart_ice()` was called; the next offer restarts ICE.
    pub(crate) restart: bool,
    /// `close()` was called: `state()` answers `Closed` from now on, and
    /// the stack's own late states are ignored until the final
    /// [`Event::Closed`].
    pub(crate) closing: bool,
    pub(crate) cbs: PeerCallbacks,
    pub(crate) channels: HashMap<u64, ChannelEntry>,
    #[cfg(feature = "media-core")]
    pub(crate) media: crate::media::PeerMedia,
    replies: HashMap<u64, Reply>,
    /// The signalling state as of the last completed request.
    signaling: SignalingState,
    /// The signalling requests in flight, by request id.
    signaling_ops: HashMap<u64, SignalingOp>,
}

/// A signalling request, for [`signaling_state`].
#[derive(Debug, Clone, Copy)]
pub(crate) enum SignalingOp {
    Offer,
    Answer,
    Remote(crate::SdpKind),
}

/// Record that request `req` is a signalling call of kind `op`.
pub(crate) fn signaling_request(id: u64, req: u64, op: SignalingOp) {
    with_peer(id, |e| e.signaling_ops.insert(req, op));
}

/// Peer `id`'s signalling state; an offer in flight counts as
/// `HaveLocalOffer`.
pub(crate) fn signaling_state(id: u64) -> SignalingState {
    with_peer(id, |e| {
        if e.closing {
            SignalingState::Closed
        } else if e
            .signaling_ops
            .values()
            .any(|op| matches!(op, SignalingOp::Offer))
        {
            SignalingState::HaveLocalOffer
        } else {
            e.signaling
        }
    })
    .unwrap_or(SignalingState::Closed)
}

impl PeerEntry {
    pub(crate) fn new() -> Self {
        Self {
            state: PeerState::New,
            has_remote: false,
            remote_max_message_size: None,
            remote_pending: Vec::new(),
            early: Vec::new(),
            restart: false,
            closing: false,
            cbs: PeerCallbacks::default(),
            channels: HashMap::new(),
            #[cfg(feature = "media-core")]
            media: Default::default(),
            replies: HashMap::new(),
            signaling: SignalingState::Stable,
            signaling_ops: HashMap::new(),
        }
    }
}

thread_local! {
    static PEERS: RefCell<HashMap<u64, PeerEntry>> = RefCell::new(HashMap::new());
}

/// Read or change peer `id`'s entry; `None` once it is gone.
pub(crate) fn with_peer<R>(id: u64, f: impl FnOnce(&mut PeerEntry) -> R) -> Option<R> {
    PEERS.with(|p| p.borrow_mut().get_mut(&id).map(f))
}

pub(crate) fn insert_peer(id: u64) {
    PEERS.with(|p| p.borrow_mut().insert(id, PeerEntry::new()));
}

/// Remove peer `id`: its callbacks are dropped (outside the borrow: they are
/// user code whose `Drop` may re-enter) and its waiting calls resolve to
/// [`RtcError::Closed`].
pub(crate) fn remove_peer(id: u64) {
    // `try_with`: a peer dropped during thread exit (held in a signal)
    // finds the table already gone, and has nothing left to remove.
    let entry = PEERS.try_with(|p| p.borrow_mut().remove(&id)).ok().flatten();
    if let Some(entry) = entry {
        for reply in entry.replies.values() {
            if let Some(w) = &reply.waker {
                w.wake_by_ref();
            }
        }
        drop(entry);
    }
}

/// Remove every peer, as [`remove_peer`] does one.
pub(crate) fn remove_all_peers() {
    let ids: Vec<u64> = PEERS.with(|p| p.borrow().keys().copied().collect());
    for id in ids {
        remove_peer(id);
    }
}

/// Start a signalling request on peer `id`; its reply arrives as
/// [`Event::Reply`].
pub(crate) fn new_request(id: u64) -> u64 {
    let req = crate::next_id();
    with_peer(id, |e| {
        e.replies.insert(
            req,
            Reply {
                result: None,
                waker: None,
            },
        )
    });
    req
}

/// Wait for request `req`'s reply. A peer that is closed or dropped first
/// answers [`RtcError::Closed`].
pub(crate) async fn reply(id: u64, req: u64) -> Result<Option<Description>, RtcError> {
    poll_fn(|cx| {
        let polled = with_peer(id, |e| match e.replies.get_mut(&req) {
            Some(r) => match r.result.take() {
                Some(result) => {
                    e.replies.remove(&req);
                    Poll::Ready(result)
                }
                None => {
                    r.waker = Some(cx.waker().clone());
                    Poll::Pending
                }
            },
            None => Poll::Ready(Err(RtcError::Closed)),
        });
        polled.unwrap_or(Poll::Ready(Err(RtcError::Closed)))
    })
    .await
}

/// Deliver `event` to peer `id`. Main thread only.
pub(crate) fn dispatch(id: u64, event: Event) {
    match event {
        Event::Reply { req, result } => {
            // The signalling state follows each completed request.
            let ok = result.is_ok();
            with_peer(id, |e| {
                if let Some(op) = e.signaling_ops.remove(&req)
                    && ok
                {
                    e.signaling = match op {
                        SignalingOp::Offer => SignalingState::HaveLocalOffer,
                        SignalingOp::Answer => SignalingState::Stable,
                        // Their offer replaces (implicitly rolls back) ours.
                        SignalingOp::Remote(SdpKind::Offer) => SignalingState::HaveRemoteOffer,
                        SignalingOp::Remote(_) => SignalingState::Stable,
                    };
                }
            });
            // A set_remote_description that succeeded: from now on remote
            // candidates go straight to the stack, and the held ones follow it.
            let applied = result.is_ok();
            let early = with_peer(id, |e| {
                match e.remote_pending.iter().position(|(r, _)| *r == req) {
                    Some(at) => {
                        let (_, advertised) = e.remote_pending.remove(at);
                        if applied {
                            e.has_remote = true;
                            e.remote_max_message_size = advertised;
                            std::mem::take(&mut e.early)
                        } else {
                            Vec::new()
                        }
                    }
                    None => Vec::new(),
                }
            })
            .unwrap_or_default();
            for c in early {
                crate::backend::add_candidate(id, c);
            }
            let waker = with_peer(id, |e| {
                e.replies.get_mut(&req).and_then(|r| {
                    r.result = Some(result);
                    r.waker.take()
                })
            })
            .flatten();
            if let Some(w) = waker {
                w.wake();
            }
        }
        Event::Candidate(c) => invoke(id, c, |e| Some(&mut e.cbs.ice_candidate)),
        Event::State(s) => {
            let changed = with_peer(id, |e| {
                // Closed is final; a late state from the stack does not reopen it,
                // and after close() only the final Closed is reported.
                if e.state == s
                    || e.state == PeerState::Closed
                    || (e.closing && s != PeerState::Closed)
                {
                    false
                } else {
                    e.state = s;
                    true
                }
            });
            if changed == Some(true) {
                invoke(id, s, |e| Some(&mut e.cbs.connection_state));
            }
        }
        Event::Error(err) => invoke(id, err, |e| Some(&mut e.cbs.error)),
        Event::Incoming {
            key,
            label,
            id: sid,
        } => {
            let known = with_peer(id, |e| {
                e.channels.entry(key).or_insert_with(|| {
                    ChannelEntry::new(label, sid, crate::DEFAULT_SEND_BUFFER_LIMIT)
                });
            });
            if known.is_some() {
                let ch = Channel::from_parts(id, key);
                invoke(id, ch, |e| Some(&mut e.cbs.data_channel));
            }
        }
        Event::Open {
            key,
            id: sid,
            max_message_size,
        } => {
            let opened = with_peer(id, |e| {
                // What the remote SDP says, applied the same way on both
                // backends (RFC 8841: absent means 64 KiB, 0 means no limit).
                // Chrome reports 256 KiB to a peer whose SDP says nothing.
                let advertised = match e.remote_max_message_size {
                    Some(0) => usize::MAX,
                    Some(n) => n,
                    None => crate::DEFAULT_MAX_MESSAGE_SIZE,
                };
                e.channels.get_mut(&key).is_some_and(|c| {
                    c.max_message_size = max_message_size.min(advertised);
                    if sid.is_some() {
                        c.id = sid;
                    }
                    if c.state == ChannelState::Connecting {
                        c.state = ChannelState::Open;
                        true
                    } else {
                        false
                    }
                })
            });
            if opened == Some(true) {
                invoke(id, (), move |e| {
                    e.channels.get_mut(&key).map(|c| &mut c.cbs.open)
                });
            }
        }
        Event::Message { key, data } => invoke(id, data, move |e| {
            e.channels.get_mut(&key).map(|c| &mut c.cbs.message)
        }),
        Event::ChannelError { key, error } => invoke(id, error, move |e| {
            e.channels.get_mut(&key).map(|c| &mut c.cbs.error)
        }),
        Event::ChannelClosed { key } => close_channel_entry(id, key),
        Event::Buffered { key, sent, amount } => {
            with_peer(id, |e| {
                if let Some(c) = e.channels.get_mut(&key) {
                    c.pending = c.pending.saturating_sub(sent);
                    c.reported = amount;
                }
            });
        }
        Event::Low { key, amount } => {
            // The stack's own falling edge, passed on as a browser does: not
            // gated on the mirror, whose pending bytes (a send made since)
            // would swallow it and stall an app that sends from on_low.
            let low = with_peer(id, |e| {
                e.channels.get_mut(&key).is_some_and(|c| {
                    c.reported = amount;
                    c.state == ChannelState::Open
                })
            });
            if low == Some(true) {
                invoke(id, (), move |e| {
                    e.channels
                        .get_mut(&key)
                        .map(|c| &mut c.cbs.buffered_amount_low)
                });
            }
        }
        #[cfg(feature = "media-core")]
        Event::Media(m) => crate::media::dispatch(id, m),
        Event::Closed => {
            #[cfg(feature = "media-core")]
            crate::media::peer_closed(id);
            let keys: Vec<u64> =
                with_peer(id, |e| e.channels.keys().copied().collect()).unwrap_or_default();
            for key in keys {
                close_channel_entry(id, key);
            }
            dispatch(id, Event::State(PeerState::Closed));
            remove_peer(id);
        }
    }
}

/// Mark a channel closed and fire its `on_close`, once.
fn close_channel_entry(id: u64, key: u64) {
    let was_open = with_peer(id, |e| {
        e.channels.get_mut(&key).is_some_and(|c| {
            let fire = c.state != ChannelState::Closed;
            c.state = ChannelState::Closed;
            c.pending = 0;
            c.reported = 0;
            fire
        })
    });
    if was_open == Some(true) {
        invoke(id, (), move |e| {
            e.channels.get_mut(&key).map(|c| &mut c.cbs.close)
        });
    }
}

/// Install a callback into the slot `select` picks for peer `id`. A no-op if
/// the peer (or the channel) is gone. The displaced callback is dropped
/// outside the borrow.
pub(crate) fn install<T: 'static>(
    id: u64,
    select: impl FnOnce(&mut PeerEntry) -> Option<&mut Option<Slot<T>>>,
    cb: Box<dyn FnMut(T)>,
) {
    let slot = Slot::new(cb);
    let _displaced = with_peer(id, |e| select(e).map(|s| s.replace(slot)));
}

/// Run one callback without holding the registry borrow across the call
/// (rinch-ws's `invoke`): taken out, run in a batch under its owner and
/// frame, put back if its slot is still empty; dropped and the peer's other
/// dead slots swept if its owner is gone.
pub(crate) fn invoke<T: 'static>(
    id: u64,
    arg: T,
    select: impl Fn(&mut PeerEntry) -> Option<&mut Option<Slot<T>>>,
) {
    let taken = with_peer(id, |e| select(e).and_then(Option::take)).flatten();
    let Some(Slot {
        owner,
        frame,
        mut cb,
    }) = taken
    else {
        return;
    };
    if owner.as_ref().is_some_and(|o| !o.is_alive()) {
        tracing::debug!(
            "dropping rtc callback on peer {id}: the component that registered it was unmounted"
        );
        drop(cb);
        sweep_dead(id);
        return;
    }
    {
        let _frame = frame.enter();
        match &owner {
            Some(o) => rinch_core::batch(|| o.run(|| cb(arg))),
            None => rinch_core::batch(|| unowned(|| cb(arg))),
        }
    }
    let leftover = with_peer(id, |e| match select(e) {
        Some(slot) if slot.is_none() => {
            *slot = Some(Slot { owner, frame, cb });
            None
        }
        _ => Some(Slot { owner, frame, cb }),
    });
    drop(leftover);
}

/// Run a slot that lives outside a peer (a local track's `on_ended`) the way
/// [`invoke`] runs one: dropped, not run, once its owner is gone; otherwise
/// run in a batch under its owner and frame and handed back.
#[cfg(feature = "media-core")]
pub(crate) fn run_slot<T: 'static>(slot: Slot<T>, arg: T) -> Option<Slot<T>> {
    let Slot {
        owner,
        frame,
        mut cb,
    } = slot;
    if owner.as_ref().is_some_and(|o| !o.is_alive()) {
        return None;
    }
    {
        let _frame = frame.enter();
        match &owner {
            Some(o) => rinch_core::batch(|| o.run(|| cb(arg))),
            None => rinch_core::batch(|| unowned(|| cb(arg))),
        }
    }
    Some(Slot { owner, frame, cb })
}

/// Take a slot out if its owner is gone, into `dead` (dropped by the caller
/// outside any borrow).
pub(crate) fn take_dead<T: 'static>(
    s: &mut Option<Slot<T>>,
    dead: &mut Vec<Box<dyn std::any::Any>>,
) {
    if s.as_ref().is_some_and(Slot::is_dead)
        && let Some(slot) = s.take()
    {
        dead.push(Box::new(slot.cb));
    }
}

/// Take out every slot of peer `id` (and its channels) whose owner is gone.
fn sweep_dead(id: u64) {
    let _dead = with_peer(id, |e| {
        let mut dead: Vec<Box<dyn std::any::Any>> = Vec::new();
        take_dead(&mut e.cbs.ice_candidate, &mut dead);
        take_dead(&mut e.cbs.connection_state, &mut dead);
        take_dead(&mut e.cbs.data_channel, &mut dead);
        take_dead(&mut e.cbs.error, &mut dead);
        for c in e.channels.values_mut() {
            take_dead(&mut c.cbs.open, &mut dead);
            take_dead(&mut c.cbs.message, &mut dead);
            take_dead(&mut c.cbs.close, &mut dead);
            take_dead(&mut c.cbs.error, &mut dead);
            take_dead(&mut c.cbs.buffered_amount_low, &mut dead);
        }
        #[cfg(feature = "media-core")]
        crate::media::sweep_dead(&mut e.media, &mut dead);
        dead
    });
}
