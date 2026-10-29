//! A `Peer` dropped by rinch-rtc's own peer table as the table is destroyed
//! (#1952): a peer owned by another peer's callback, with that other peer
//! still registered at thread exit. The table drops the callback while it is
//! being torn down, so the owned peer's drop always finds the table gone,
//! whatever order the thread's locals are destroyed in. (`review_peer_drop_at_exit.rs`
//! is the order-dependent shape: a peer in an app's own thread-local.)
//!
//! `PeerInner::drop` must reach the table with `LocalKey::try_with`; `with`
//! panics there, and a panic in a thread-local destructor aborts this binary.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use rinch_rtc::{Peer, RtcConfig};

#[test]
fn a_peer_owned_by_a_callback_drops_quietly_as_the_peer_table_is_destroyed() {
    common::init();
    let owner = Peer::new(RtcConfig::default()).unwrap();
    let owned = Peer::new(RtcConfig::default()).unwrap();
    owner.on_error(move |_| {
        let _keep = &owned;
    });
    // Still registered when the test thread exits: the table holds `owned`.
    std::mem::forget(owner);
}
