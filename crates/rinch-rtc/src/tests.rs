//! Unit tests for the main-thread half: the registry, the callback slots and
//! `send`'s refusals, driven with events by hand (no backend, no network).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch_core::reactive::Scope;

use super::registry::{ChannelEntry, Event, dispatch, insert_peer, remove_peer, with_peer};
use super::*;

/// A peer entry and an open channel on it, as a backend would leave them.
fn fake_channel(limit: usize, mms: usize) -> (u64, Channel) {
    let id = next_id();
    insert_peer(id);
    let key = next_id();
    with_peer(id, |e| {
        let mut c = ChannelEntry::new("c".into(), Some(3), limit);
        c.state = ChannelState::Open;
        c.max_message_size = mms;
        e.channels.insert(key, c);
    });
    (id, Channel::from_parts(id, key))
}

#[test]
fn send_refuses_not_open_too_large_and_queue_full_without_queueing() {
    let (id, ch) = fake_channel(100, 50);
    assert_eq!(
        ch.send(vec![0; 51]),
        Err(SendError::TooLarge { len: 51, max: 50 })
    );
    assert_eq!(ch.buffered_amount(), 0, "a refused send queues nothing");
    ch.send(vec![0; 40]).unwrap();
    ch.send(vec![0; 40]).unwrap();
    assert_eq!(ch.buffered_amount(), 80);
    assert_eq!(
        ch.send(vec![0; 40]),
        Err(SendError::QueueFull {
            buffered: 80,
            limit: 100
        })
    );
    assert_eq!(ch.buffered_amount(), 80);
    // The backend reports what it queued: the mirror follows.
    dispatch(
        id,
        Event::Buffered {
            key: ch.key,
            sent: 40,
            amount: 10,
        },
    );
    assert_eq!(ch.buffered_amount(), 50, "10 reported + 40 still pending");
    with_peer(id, |e| {
        e.channels.get_mut(&ch.key).unwrap().state = ChannelState::Closing
    });
    assert_eq!(ch.send(vec![1]), Err(SendError::NotOpen));
    remove_peer(id);
    assert_eq!(ch.send(vec![1]), Err(SendError::NotOpen));
    assert_eq!(ch.state(), ChannelState::Closed);
}

#[test]
fn an_empty_queue_admits_a_message_larger_than_the_limit() {
    let (id, ch) = fake_channel(10, 1000);
    ch.send(vec![0; 500]).unwrap();
    assert!(matches!(
        ch.send(vec![0; 1]),
        Err(SendError::QueueFull { .. })
    ));
    remove_peer(id);
}

#[test]
fn the_stacks_low_water_edge_fires_even_with_a_send_pending() {
    // The stack crossed its threshold; the app has since queued more (still
    // pending in the mirror). Gating on the mirror would swallow the edge,
    // and an app that sends only from on_low would stall for good.
    let (id, ch) = fake_channel(1 << 20, 1 << 16);
    let lows = Rc::new(Cell::new(0));
    let l = lows.clone();
    ch.on_buffered_amount_low(move || l.set(l.get() + 1));
    with_peer(id, |e| {
        e.channels.get_mut(&ch.key).unwrap().low_threshold = 100
    });
    ch.send(vec![0; 5_000]).unwrap();
    assert_eq!(ch.buffered_amount(), 5_000);
    dispatch(
        id,
        Event::Low {
            key: ch.key,
            amount: 0,
        },
    );
    assert_eq!(lows.get(), 1, "the falling edge reaches on_low");
    remove_peer(id);
}

#[test]
fn a_callback_registered_in_a_scope_is_pruned_after_it_disposes_and_sweeps_its_siblings() {
    let (id, ch) = fake_channel(1 << 20, 1 << 16);
    let ran = Rc::new(Cell::new(0));
    let scope = Scope::new();
    scope.run(|| {
        let r = ran.clone();
        ch.on_message(move |_| r.set(r.get() + 1));
        let r = ran.clone();
        ch.on_close(move || r.set(r.get() + 100));
    });
    dispatch(
        id,
        Event::Message {
            key: ch.key,
            data: vec![1],
        },
    );
    assert_eq!(ran.get(), 1, "live: it runs");
    scope.dispose();
    dispatch(
        id,
        Event::Message {
            key: ch.key,
            data: vec![2],
        },
    );
    assert_eq!(ran.get(), 1, "dead: it does not");
    let armed = with_peer(id, |e| {
        let c = &e.channels[&ch.key].cbs;
        (c.message.is_some(), c.close.is_some())
    });
    assert_eq!(
        armed,
        Some((false, false)),
        "pruned, and its dead sibling swept"
    );
    dispatch(id, Event::ChannelClosed { key: ch.key });
    assert_eq!(ran.get(), 1);
    remove_peer(id);
}

#[test]
fn an_ownerless_callback_keeps_app_lifetime() {
    let (id, ch) = fake_channel(1 << 20, 1 << 16);
    let got = Rc::new(RefCell::new(Vec::new()));
    let g = got.clone();
    ch.on_message(move |m| g.borrow_mut().push(m));
    dispatch(
        id,
        Event::Message {
            key: ch.key,
            data: vec![1],
        },
    );
    dispatch(
        id,
        Event::Message {
            key: ch.key,
            data: vec![2],
        },
    );
    assert_eq!(*got.borrow(), vec![vec![1], vec![2]]);
    remove_peer(id);
}

#[test]
fn a_callback_may_drop_its_peer_entry_while_it_runs() {
    let (id, ch) = fake_channel(1 << 20, 1 << 16);
    ch.on_message(move |_| remove_peer(id));
    dispatch(
        id,
        Event::Message {
            key: ch.key,
            data: vec![1],
        },
    );
    assert!(with_peer(id, |_| ()).is_none());
}

#[test]
fn close_fires_each_open_channel_once_then_the_peer_state_and_ends_the_entry() {
    let (id, ch) = fake_channel(1 << 20, 1 << 16);
    let order = Rc::new(RefCell::new(Vec::<String>::new()));
    let o = order.clone();
    ch.on_close(move || o.borrow_mut().push("channel".into()));
    let o = order.clone();
    registry::install(
        id,
        |e| Some(&mut e.cbs.connection_state),
        Box::new(move |s: PeerState| o.borrow_mut().push(format!("{s:?}"))),
    );
    dispatch(id, Event::ChannelClosed { key: ch.key });
    dispatch(id, Event::Closed);
    assert_eq!(*order.borrow(), ["channel", "Closed"]);
    assert!(with_peer(id, |_| ()).is_none());
}

#[test]
fn after_close_only_the_final_closed_is_reported() {
    let id = next_id();
    insert_peer(id);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let s = seen.clone();
    registry::install(
        id,
        |e| Some(&mut e.cbs.connection_state),
        Box::new(move |st: PeerState| s.borrow_mut().push(st)),
    );
    dispatch(id, Event::State(PeerState::Connected));
    with_peer(id, |e| e.closing = true);
    dispatch(id, Event::State(PeerState::Disconnected));
    dispatch(id, Event::Closed);
    assert_eq!(*seen.borrow(), [PeerState::Connected, PeerState::Closed]);
}

#[test]
fn a_waiting_call_resolves_closed_when_its_peer_goes() {
    let id = next_id();
    insert_peer(id);
    let req = registry::new_request(id);
    let mut fut = std::pin::pin!(registry::reply(id, req));
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(fut.as_mut().poll(&mut cx).is_pending());
    remove_peer(id);
    assert_eq!(
        fut.as_mut().poll(&mut cx),
        std::task::Poll::Ready(Err(RtcError::Closed))
    );
}

#[test]
fn config_validation() {
    let bad = |servers: Vec<IceServer>| {
        validate(&RtcConfig {
            ice_servers: servers,
            ..Default::default()
        })
    };
    assert!(bad(vec![IceServer::stun("stun:stun.example.org:3478")]).is_ok());
    assert!(bad(vec![IceServer::turn("turn:t.example.org:3478", "u", "p")]).is_ok());
    assert!(bad(vec![IceServer::stun("http://x")]).is_err());
    assert!(
        bad(vec![IceServer::stun("turn:t.example.org")]).is_err(),
        "TURN needs credentials"
    );
    assert!(
        bad(vec![IceServer {
            urls: vec![],
            ..Default::default()
        }])
        .is_err()
    );
}

#[test]
fn the_remote_sdp_sets_max_message_size_the_same_way_on_both_backends() {
    assert_eq!(
        advertised_max_message_size("v=0\r\na=max-message-size:262144\r\n"),
        Some(262_144)
    );
    assert_eq!(advertised_max_message_size("v=0\r\n"), None);
    for (advertised, stack, expect) in [
        (None, 262_144, 65_536),
        (Some(262_144), 262_144, 262_144),
        (Some(1_000_000), 262_144, 262_144),
        (Some(0), 262_144, 262_144),
    ] {
        let (id, ch) = fake_channel(1 << 20, 0);
        with_peer(id, |e| {
            e.remote_max_message_size = advertised;
            e.channels.get_mut(&ch.key).unwrap().state = ChannelState::Connecting;
        });
        dispatch(
            id,
            Event::Open {
                key: ch.key,
                id: None,
                max_message_size: stack,
            },
        );
        assert_eq!(ch.max_message_size(), expect, "advertised {advertised:?}");
        remove_peer(id);
    }
}

#[test]
fn overlapping_remote_descriptions_are_applied_each_on_its_own_reply() {
    // Two set_remote_description calls in flight; the first succeeds, the
    // second fails. The first must still mark the remote as set.
    let id = next_id();
    insert_peer(id);
    let (r1, r2) = (registry::new_request(id), registry::new_request(id));
    with_peer(id, |e| {
        e.remote_pending.push((r1, Some(262_144)));
        e.remote_pending.push((r2, None));
    });
    dispatch(
        id,
        Event::Reply {
            req: r1,
            result: Ok(None),
        },
    );
    dispatch(
        id,
        Event::Reply {
            req: r2,
            result: Err(RtcError::Sdp("bad".into())),
        },
    );
    assert_eq!(
        with_peer(id, |e| (e.has_remote, e.remote_max_message_size)),
        Some((true, Some(262_144)))
    );
    remove_peer(id);
}

#[test]
fn signaling_state_follows_the_exchange_and_counts_an_offer_in_flight() {
    use super::registry::{SignalingOp, signaling_request, signaling_state};
    let id = next_id();
    insert_peer(id);
    let reply = |req, ok: bool| Event::Reply {
        req,
        result: if ok {
            Ok(None)
        } else {
            Err(RtcError::Sdp("x".into()))
        },
    };
    assert_eq!(signaling_state(id), SignalingState::Stable);
    let r1 = next_id();
    signaling_request(id, r1, SignalingOp::Offer);
    assert_eq!(
        signaling_state(id),
        SignalingState::HaveLocalOffer,
        "in flight"
    );
    dispatch(id, reply(r1, true));
    assert_eq!(signaling_state(id), SignalingState::HaveLocalOffer);
    // Their offer over ours (glare, polite side): have-remote-offer.
    let r2 = next_id();
    signaling_request(id, r2, SignalingOp::Remote(SdpKind::Offer));
    dispatch(id, reply(r2, true));
    assert_eq!(signaling_state(id), SignalingState::HaveRemoteOffer);
    // A failed answer changes nothing; a good one is stable.
    let r3 = next_id();
    signaling_request(id, r3, SignalingOp::Answer);
    dispatch(id, reply(r3, false));
    assert_eq!(signaling_state(id), SignalingState::HaveRemoteOffer);
    let r4 = next_id();
    signaling_request(id, r4, SignalingOp::Answer);
    dispatch(id, reply(r4, true));
    assert_eq!(signaling_state(id), SignalingState::Stable);
    // A failed offer leaves no offer in flight.
    let r5 = next_id();
    signaling_request(id, r5, SignalingOp::Offer);
    dispatch(id, reply(r5, false));
    assert_eq!(signaling_state(id), SignalingState::Stable);
    // Rollback: back to stable.
    let r6 = next_id();
    signaling_request(id, r6, SignalingOp::Offer);
    dispatch(id, reply(r6, true));
    let r7 = next_id();
    signaling_request(id, r7, SignalingOp::Remote(SdpKind::Rollback));
    dispatch(id, reply(r7, true));
    assert_eq!(signaling_state(id), SignalingState::Stable);
    remove_peer(id);
    assert_eq!(signaling_state(id), SignalingState::Closed);
}

/// The `media` feature's main-thread half: remote-track events dispatched
/// by hand, as a backend reports them.
#[cfg(feature = "media")]
mod media_events {
    use super::*;
    use crate::media::{MediaEvent, MediaKind, RemoteTrack};

    fn fake_peer() -> Peer {
        let id = next_id();
        insert_peer(id);
        Peer {
            inner: Rc::new(PeerInner { id }),
        }
    }

    fn track_event(key: u64, kind: MediaKind, muted: bool) -> Event {
        Event::Media(MediaEvent::Track {
            key,
            kind,
            id: "t".into(),
            stream_ids: vec!["s".into()],
            transceiver: 99,
            muted,
        })
    }

    #[test]
    fn a_remote_track_is_handed_over_and_its_mute_and_end_fire_once() {
        let peer = fake_peer();
        let id = peer.id();
        let got: Rc<RefCell<Vec<RemoteTrack>>> = Rc::default();
        let g = got.clone();
        peer.on_track(move |t| g.borrow_mut().push(t));
        let key = next_id();
        dispatch(id, track_event(key, MediaKind::Audio, true));
        assert_eq!(got.borrow().len(), 1);
        let t = got.borrow()[0].clone();
        assert_eq!(t.kind(), MediaKind::Audio);
        assert_eq!(t.stream_ids(), vec!["s".to_string()]);
        assert!(t.muted(), "it starts as the backend said");

        let log: Rc<RefCell<Vec<&str>>> = Rc::default();
        let l = log.clone();
        t.on_mute(move || l.borrow_mut().push("mute"));
        let l = log.clone();
        t.on_unmute(move || l.borrow_mut().push("unmute"));
        let l = log.clone();
        t.on_ended(move || l.borrow_mut().push("ended"));
        let muted = |m| Event::Media(MediaEvent::Muted { key, muted: m });
        dispatch(id, muted(true)); // no change: nothing
        dispatch(id, muted(false));
        dispatch(id, muted(false));
        dispatch(id, muted(true));
        dispatch(id, Event::Media(MediaEvent::Ended { key }));
        dispatch(id, Event::Media(MediaEvent::Ended { key }));
        dispatch(id, muted(false)); // ended: nothing more
        assert_eq!(*log.borrow(), vec!["unmute", "mute", "ended"]);
        assert!(t.ended());
        drop(peer);
    }

    #[test]
    fn closing_the_peer_ends_its_remote_tracks() {
        let peer = fake_peer();
        let id = peer.id();
        let got: Rc<RefCell<Vec<RemoteTrack>>> = Rc::default();
        let g = got.clone();
        peer.on_track(move |t| g.borrow_mut().push(t));
        dispatch(id, track_event(next_id(), MediaKind::Video, false));
        dispatch(id, track_event(next_id(), MediaKind::Audio, false));
        let ended = Rc::new(Cell::new(0));
        for t in got.borrow().iter() {
            let e = ended.clone();
            t.on_ended(move || e.set(e.get() + 1));
        }
        dispatch(id, Event::Closed);
        assert_eq!(ended.get(), 2);
        assert!(got.borrow().iter().all(|t| t.ended()));
        drop(peer);
    }

    #[test]
    fn media_callbacks_registered_in_a_scope_are_dropped_with_it() {
        let peer = fake_peer();
        let id = peer.id();
        let ran = Rc::new(Cell::new(0));
        let scope = Scope::new();
        scope.run(|| {
            let r = ran.clone();
            peer.on_track(move |_| r.set(r.get() + 1));
            let r = ran.clone();
            peer.on_negotiation_needed(move || r.set(r.get() + 10));
        });
        dispatch(id, Event::Media(MediaEvent::NegotiationNeeded));
        dispatch(id, track_event(next_id(), MediaKind::Video, false));
        assert_eq!(ran.get(), 11);
        scope.dispose();
        dispatch(id, Event::Media(MediaEvent::NegotiationNeeded));
        dispatch(id, track_event(next_id(), MediaKind::Video, false));
        assert_eq!(ran.get(), 11, "pruned with their owner");
        drop(peer);
    }

    #[test]
    fn source_writers_keep_the_latest_frame_and_at_most_a_second_of_audio() {
        use crate::media::{AudioSourceOptions, AudioSourceWriter, VideoSourceWriter};
        use std::sync::atomic::Ordering;
        let v = VideoSourceWriter::new();
        v.push_rgba(&[1; 16], 2, 2);
        v.push_rgba(&[0; 7], 2, 2); // the wrong length: ignored
        v.push_rgba(&[2; 16], 2, 2);
        assert_eq!(v.slot.seq.load(Ordering::Acquire), 2);
        assert_eq!(
            v.slot.latest.lock().unwrap().as_ref().map(|f| f.0[0]),
            Some(2)
        );
        let a = AudioSourceWriter::new(AudioSourceOptions {
            sample_rate: 100,
            channels: 2,
        });
        a.push(&[0.5; 150]);
        a.push(&[0.25; 150]);
        let q = a.queue.samples.lock().unwrap();
        assert_eq!(q.len(), 200, "one second of 100 Hz stereo");
        assert_eq!(q.front(), Some(&0.5), "the oldest went first");
        assert_eq!(q.back(), Some(&0.25));
    }

    #[test]
    fn native_media_says_not_yet() {
        let r = crate::media::LocalTrack::video_source(Default::default());
        assert!(matches!(r, Err(crate::media::MediaError::NotSupported(_))));
        let peer = fake_peer();
        assert!(
            peer.add_transceiver(MediaKind::Video, crate::media::Direction::RecvOnly)
                .is_err()
        );
        drop(peer);
    }
}
