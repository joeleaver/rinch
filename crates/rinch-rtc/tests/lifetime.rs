//! Cancel on unmount, with the `unowned` opt-out (design record 1.5, D5):
//! a peer created while a component renders closes when it unmounts, and
//! the lite peer on the other end sees its channels close; a peer created
//! under `unowned` in the same component survives, and so does its link.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use common::{LitePeer, T, drive, init, pump_until};
use rinch_core::reactive::{Scope, unowned};
use rinch_rtc::{
    Channel, ChannelOptions, ChannelState, Description, Peer, PeerState, Reliability, RtcConfig,
};

fn channels(peer: &Peer) -> (Channel, Channel) {
    let control = peer
        .create_data_channel(
            "control",
            ChannelOptions::negotiated(0, Reliability::Reliable),
        )
        .unwrap();
    let tick = peer
        .create_data_channel(
            "tick",
            ChannelOptions::negotiated(1, Reliability::Unreliable { max_retransmits: 0 }),
        )
        .unwrap();
    (control, tick)
}

fn connect(peer: &Peer, lite: &LitePeer, control: &Channel) {
    let offer = drive(peer.create_offer(), T).unwrap();
    drive(
        peer.set_remote_description(Description::answer(lite.answer(&offer.sdp))),
        T,
    )
    .unwrap();
    assert!(pump_until(|| control.state() == ChannelState::Open, T));
    // Used, so a close travels (see interop_lite.rs on str0m and unused streams).
    control.send(b"up".to_vec()).unwrap();
    assert!(pump_until(
        || lite.stats().received.get("control") == Some(&2),
        T
    ));
}

#[test]
fn a_peer_closes_with_its_component_unless_unowned() {
    init();
    let owned_lite = LitePeer::spawn();
    let kept_lite = LitePeer::spawn();

    let component = Scope::new();
    let fired_after_unmount = Rc::new(Cell::new(0));
    let (owned, kept) = component.run(|| {
        let owned = Peer::new(RtcConfig::default()).unwrap();
        let kept = unowned(|| Peer::new(RtcConfig::default())).unwrap();
        // Registered by the component on the peer that outlives it: pruned
        // at unmount, as in rinch-ws (#183).
        let f = fired_after_unmount.clone();
        kept.on_connection_state(move |_| f.set(f.get() + 1));
        (owned, kept)
    });
    let (owned_control, _owned_tick) = channels(&owned);
    let (kept_control, _kept_tick) = channels(&kept);
    connect(&owned, &owned_lite, &owned_control);
    connect(&kept, &kept_lite, &kept_control);
    let before = fired_after_unmount.get();
    assert!(
        before >= 1,
        "the component's callback ran while it was mounted"
    );

    component.dispose();

    // The owned peer: closed at once, silently, and its far end sees it go.
    assert_eq!(owned.state(), PeerState::Closed);
    assert_eq!(owned_control.state(), ChannelState::Closed);
    assert!(
        pump_until(|| !owned_lite.stats().closed.is_empty(), T),
        "the lite peer sees the owned peer's channels close: {:?}",
        owned_lite.stats()
    );

    // The unowned one (the positive control for "closed by the unmount"):
    // still connected after the same unmount and the same wait, still echoing.
    assert_eq!(kept.state(), PeerState::Connected);
    kept_control.send(b"still here".to_vec()).unwrap();
    assert!(pump_until(
        || kept_lite.stats().received.get("control") == Some(&12),
        T
    ));
    assert!(kept_lite.stats().closed.is_empty());

    // ...and the unmounted component's callback on it no longer runs.
    kept.close();
    assert!(pump_until(
        || kept_control.state() == ChannelState::Closed,
        T
    ));
    let _ = pump_until(|| false, Duration::from_millis(200));
    assert_eq!(fired_after_unmount.get(), before);
    assert_eq!(kept.state(), PeerState::Closed);
}
