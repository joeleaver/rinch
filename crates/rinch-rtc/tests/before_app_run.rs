//! A peer made before the host has installed its cross-thread dispatcher
//! (before `App::run`, or in `main` ahead of it) must not panic the worker:
//! its events wait in the main-callback queue until the host drains it.
//!
//! No dispatcher is installed in this process; the test drains the queue
//! itself, as a host does once it starts, and then as a headless program
//! (a CLI tool) does for good: the crate docs' "Without App::run" recipe,
//! two peers connected and talking with nothing but `drain_main_callbacks`.
#![cfg(not(target_arch = "wasm32"))]

use std::pin::pin;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use rinch_core::{drain_main_callbacks, register_main_thread};
use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;

use rinch_rtc::{ChannelOptions, ChannelState, IceCandidate, Peer, Reliability, RtcConfig};

/// The headless loop: drain, poll, sleep.
fn headless<T>(fut: impl Future<Output = T>) -> T {
    let mut fut = pin!(fut);
    let mut cx = Context::from_waker(Waker::noop());
    let start = Instant::now();
    loop {
        drain_main_callbacks();
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
        assert!(start.elapsed() < Duration::from_secs(15), "headless: stuck");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn headless_until(mut cond: impl FnMut() -> bool) {
    headless(std::future::poll_fn(|_| {
        if cond() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }))
}

#[test]
fn a_peer_made_before_the_host_runs_queues_its_events() {
    register_main_thread();
    rinch_rtc::testing::set_bind_addrs(&["127.0.0.1:0"]);
    let peer = Peer::new(RtcConfig::default()).unwrap();
    let _c = peer
        .create_data_channel("c", ChannelOptions::negotiated(0, Reliability::Reliable))
        .unwrap();
    let mut offer = pin!(peer.create_offer());
    let mut cx = Context::from_waker(Waker::noop());
    // The worker answers into the queue; nothing drains it yet.
    std::thread::sleep(Duration::from_millis(500));
    assert!(offer.as_mut().poll(&mut cx).is_pending());
    // The "host" starts and drains: the reply was kept, and the worker is
    // alive to make a second one.
    let start = Instant::now();
    let sdp = loop {
        drain_main_callbacks();
        if let Poll::Ready(r) = offer.as_mut().poll(&mut cx) {
            break r.expect("the offer");
        }
        assert!(start.elapsed() < Duration::from_secs(10), "no reply");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(sdp.sdp.contains("m=application"));
    let mut again = pin!(peer.create_offer());
    let start = Instant::now();
    loop {
        drain_main_callbacks();
        if let Poll::Ready(r) = again.as_mut().poll(&mut cx) {
            r.expect("the worker survived");
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "worker gone");
        std::thread::sleep(Duration::from_millis(5));
    }
    peer.close();

    // Headless for good: two peers, signalling by hand, nothing but drains.
    let a = Peer::new(RtcConfig::default()).unwrap();
    let b = Peer::new(RtcConfig::default()).unwrap();
    type Inbox = Rc<RefCell<Vec<Option<IceCandidate>>>>;
    let (a_out, b_out): (Inbox, Inbox) = Default::default();
    let o = a_out.clone();
    a.on_ice_candidate(move |c| o.borrow_mut().push(c));
    let o = b_out.clone();
    b.on_ice_candidate(move |c| o.borrow_mut().push(c));
    let opts = ChannelOptions::negotiated(2, Reliability::Reliable);
    let ca = a.create_data_channel("c", opts).unwrap();
    let cb = b.create_data_channel("c", opts).unwrap();
    let got: Rc<RefCell<Vec<u8>>> = Rc::default();
    let g = got.clone();
    cb.on_message(move |m| g.borrow_mut().extend(m));
    let offer = headless(a.create_offer()).unwrap();
    headless(b.set_remote_description(offer)).unwrap();
    let answer = headless(b.create_answer()).unwrap();
    headless(a.set_remote_description(answer)).unwrap();
    headless_until(|| {
        for c in a_out.borrow_mut().drain(..) {
            b.add_ice_candidate(c);
        }
        for c in b_out.borrow_mut().drain(..) {
            a.add_ice_candidate(c);
        }
        ca.state() == ChannelState::Open && cb.state() == ChannelState::Open
    });
    ca.send(b"headless".to_vec()).unwrap();
    headless_until(|| *got.borrow() == b"headless");
    a.close();
    b.close();
    headless_until(|| ca.state() == ChannelState::Closed);
}
