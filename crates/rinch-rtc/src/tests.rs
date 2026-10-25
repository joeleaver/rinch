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
