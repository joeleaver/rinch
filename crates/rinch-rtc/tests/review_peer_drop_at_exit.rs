//! Review fixture (#1970): a `Peer` still held when its thread exits, the
//! same shape as `media_drop_at_exit.rs` but for the peer, which a call
//! (ui-zoo's `InPageCall`, video-call's `Call`) also holds in closures and
//! signals. `PeerInner::drop` reaches `registry::PEERS` (and, with `media`,
//! `native_media::SENDING`) through `LocalKey::with`.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::cell::RefCell;

use rinch_rtc::{Peer, RtcConfig};

thread_local! {
    static HELD: RefCell<Vec<Peer>> = const { RefCell::new(Vec::new()) };
}

#[test]
fn a_peer_dropped_as_its_thread_exits_does_not_abort() {
    HELD.with(|h| h.borrow_mut().clear());
    common::init();
    let peer = Peer::new(RtcConfig::default()).unwrap();
    HELD.with(|h| h.borrow_mut().push(peer));
}
