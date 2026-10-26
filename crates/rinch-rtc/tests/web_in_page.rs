//! The web backend in a real browser, with no server: two rinch-rtc peers in
//! one page connect to each other (host candidates, trickled through the
//! test). The positive control in each case is bytes that crossed a real
//! `RTCPeerConnection`: they can only arrive through the browser's stack.
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch_core::reactive::{Scope, unowned};
use rinch_rtc::{
    Channel, ChannelOptions, ChannelState, IceCandidate, Peer, PeerState, Reliability, RtcConfig,
    SendError,
};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// Let the browser run: one macrotask.
async fn tick() {
    let p = js_sys::Promise::new(&mut |resolve, _| {
        let _ = web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 5);
    });
    let _ = wasm_bindgen_futures::JsFuture::from(p).await;
}

/// Wait until `cond` holds, or panic after ~10 s.
async fn until(what: &str, mut cond: impl FnMut() -> bool) {
    for _ in 0..2000 {
        if cond() {
            return;
        }
        tick().await;
    }
    panic!("timed out waiting for {what}");
}

type Inbox = Rc<RefCell<Vec<Option<IceCandidate>>>>;
type Got = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

fn trickle(peer: &Peer) -> (Inbox, Rc<Cell<usize>>) {
    let inbox: Inbox = Rc::default();
    let ends = Rc::new(Cell::new(0));
    let (i, e) = (inbox.clone(), ends.clone());
    peer.on_ice_candidate(move |c| {
        if c.is_none() {
            e.set(e.get() + 1);
        }
        i.borrow_mut().push(c);
    });
    (inbox, ends)
}

fn record(ch: &Channel, got: &Got) {
    let (g, label) = (got.clone(), ch.label());
    ch.on_message(move |m| g.borrow_mut().push((label.clone(), m)));
}

/// Offer from `a`, answer from `b`, trickling both ways until connected.
async fn connect(a: &Peer, b: &Peer) {
    let (a_out, _) = trickle(a);
    let (b_out, _) = trickle(b);
    let offer = a.create_offer().await.unwrap();
    // Candidates may already be here: rinch-rtc holds them for b.
    for c in a_out.borrow_mut().drain(..) {
        b.add_ice_candidate(c);
    }
    b.set_remote_description(offer).await.unwrap();
    let answer = b.create_answer().await.unwrap();
    a.set_remote_description(answer).await.unwrap();
    until("both connected", || {
        for c in a_out.borrow_mut().drain(..) {
            b.add_ice_candidate(c);
        }
        for c in b_out.borrow_mut().drain(..) {
            a.add_ice_candidate(c);
        }
        a.state() == PeerState::Connected && b.state() == PeerState::Connected
    })
    .await;
}

#[wasm_bindgen_test]
async fn two_peers_in_one_page() {
    let a = Peer::new(RtcConfig::default()).unwrap();
    let b = Peer::new(RtcConfig::default()).unwrap();
    let errors: Rc<RefCell<Vec<String>>> = Rc::default();
    for p in [&a, &b] {
        let e = errors.clone();
        p.on_error(move |err| e.borrow_mut().push(err.to_string()));
    }
    let chat = a
        .create_data_channel("chat", ChannelOptions::new(Reliability::Reliable))
        .unwrap();
    let opts = ChannelOptions::negotiated(5, Reliability::Unreliable { max_retransmits: 0 });
    let tick_a = a.create_data_channel("tick", opts).unwrap();
    let tick_b = b.create_data_channel("tick", opts).unwrap();
    let heard: Rc<RefCell<Option<Channel>>> = Rc::default();
    let h = heard.clone();
    b.on_data_channel(move |ch| *h.borrow_mut() = Some(ch));
    let got: Got = Rc::default();
    record(&tick_b, &got);

    connect(&a, &b).await;
    assert!(errors.borrow().is_empty(), "{:?}", errors.borrow());
    until("chat announced to b", || heard.borrow().is_some()).await;
    let chat_b = heard.borrow().clone().unwrap();
    assert_eq!(chat_b.label(), "chat");
    record(&chat_b, &got);
    until("all open", || {
        chat.state() == ChannelState::Open
            && chat_b.state() == ChannelState::Open
            && tick_a.state() == ChannelState::Open
            && tick_b.state() == ChannelState::Open
    })
    .await;
    assert_eq!(tick_a.id(), Some(5));
    let chat_id = chat.id().expect("the web reports an in-band stream id");
    // A negotiated channel on the in-band channel's id is refused.
    assert!(matches!(
        a.create_data_channel(
            "clash",
            ChannelOptions::negotiated(chat_id, Reliability::Reliable)
        ),
        Err(rinch_rtc::RtcError::InvalidState(_))
    ));
    assert!(chat.max_message_size() >= 65_536);

    chat.send(b"over dcep".to_vec()).unwrap();
    tick_a.send(b"over a negotiated id".to_vec()).unwrap();
    until("both messages", || got.borrow().len() == 2).await;
    let mut g = got.borrow().clone();
    g.sort();
    assert_eq!(
        g,
        vec![
            ("chat".to_string(), b"over dcep".to_vec()),
            ("tick".to_string(), b"over a negotiated id".to_vec()),
        ]
    );

    // Refusals: nothing queued, the channel stays open.
    let max = chat.max_message_size();
    assert!(matches!(
        chat.send(vec![0u8; max + 1]),
        Err(SendError::TooLarge { .. })
    ));

    // A remote close.
    let closed_b = Rc::new(Cell::new(0));
    let k = closed_b.clone();
    chat_b.on_close(move || k.set(k.get() + 1));
    chat.close();
    until("b hears the close", || closed_b.get() == 1).await;
    until("a's side closed", || chat.state() == ChannelState::Closed).await;
    assert_eq!(chat.send(b"x".to_vec()), Err(SendError::NotOpen));

    // close(): each open channel once, then the peer.
    let order: Rc<RefCell<Vec<String>>> = Rc::default();
    let o = order.clone();
    tick_a.on_close(move || o.borrow_mut().push("tick".into()));
    let o = order.clone();
    a.on_connection_state(move |s| o.borrow_mut().push(format!("{s:?}")));
    a.close();
    assert_eq!(a.state(), PeerState::Closed);
    until("a closed", || {
        order.borrow().contains(&"Closed".to_string())
    })
    .await;
    assert_eq!(*order.borrow(), ["tick", "Closed"]);
    b.close();
}

#[wasm_bindgen_test]
async fn queue_full_and_the_low_water_mark() {
    let a = Peer::new(RtcConfig::default()).unwrap();
    let b = Peer::new(RtcConfig::default()).unwrap();
    let opts = ChannelOptions::negotiated(0, Reliability::Reliable).with_send_buffer_limit(100_000);
    let ch = a.create_data_channel("bulk", opts).unwrap();
    let ch_b = b.create_data_channel("bulk", opts).unwrap();
    assert_eq!(ch.send(b"early".to_vec()), Err(SendError::NotOpen));
    let received = Rc::new(Cell::new(0usize));
    let r = received.clone();
    ch_b.on_message(move |m| r.set(r.get() + m.len()));
    connect(&a, &b).await;
    until("open", || ch.state() == ChannelState::Open).await;

    let lows = Rc::new(Cell::new(0));
    let l = lows.clone();
    ch.on_buffered_amount_low(move || l.set(l.get() + 1));
    // Fill to the limit in one go (no yield, so nothing drains between).
    let mut admitted = 0;
    let mut refused = None;
    for _ in 0..10 {
        match ch.send(vec![1u8; 40_000]) {
            Ok(()) => admitted += 1,
            Err(e) => {
                refused = Some(e);
                break;
            }
        }
    }
    assert!(
        matches!(refused, Some(SendError::QueueFull { limit: 100_000, .. })),
        "refused {refused:?} after {admitted}"
    );
    until("drained", || lows.get() >= 1 && ch.buffered_amount() == 0).await;
    ch.send(vec![2u8; 40_000]).unwrap();
    let expected = (admitted + 1) * 40_000;
    until("everything admitted arrived, nothing refused did", || {
        received.get() == expected
    })
    .await;
    a.close();
    b.close();
}

#[wasm_bindgen_test]
async fn shutdown_closes_every_peer() {
    let a = Peer::new(RtcConfig::default()).unwrap();
    let b = Peer::new(RtcConfig::default()).unwrap();
    let opts = ChannelOptions::negotiated(0, Reliability::Reliable);
    let ca = a.create_data_channel("c", opts).unwrap();
    let cb = b.create_data_channel("c", opts).unwrap();
    connect(&a, &b).await;
    until("open", || {
        ca.state() == ChannelState::Open && cb.state() == ChannelState::Open
    })
    .await;
    rinch_rtc::shutdown();
    assert_eq!(a.state(), PeerState::Closed);
    assert_eq!(ca.state(), ChannelState::Closed);
    assert!(ca.send(b"x".to_vec()).is_err());
    // A peer made after it works as usual.
    let c = Peer::new(RtcConfig::default()).unwrap();
    assert_eq!(c.state(), PeerState::New);
}

#[wasm_bindgen_test]
async fn a_peer_closes_with_its_component_unless_unowned() {
    let component = Scope::new();
    let (owned, kept) = component.run(|| {
        (
            Peer::new(RtcConfig::default()).unwrap(),
            unowned(|| Peer::new(RtcConfig::default())).unwrap(),
        )
    });
    let far_owned = Peer::new(RtcConfig::default()).unwrap();
    let far_kept = Peer::new(RtcConfig::default()).unwrap();
    let opts = ChannelOptions::negotiated(0, Reliability::Reliable);
    let c_owned = owned.create_data_channel("c", opts).unwrap();
    let f_owned = far_owned.create_data_channel("c", opts).unwrap();
    let c_kept = kept.create_data_channel("c", opts).unwrap();
    let f_kept = far_kept.create_data_channel("c", opts).unwrap();
    connect(&owned, &far_owned).await;
    connect(&kept, &far_kept).await;
    until("open", || {
        c_owned.state() == ChannelState::Open && c_kept.state() == ChannelState::Open
    })
    .await;
    let far_closed = Rc::new(Cell::new(0));
    let k = far_closed.clone();
    f_owned.on_close(move || k.set(k.get() + 1));

    component.dispose();

    assert_eq!(owned.state(), PeerState::Closed);
    until("the far end of the owned peer sees it go", || {
        far_closed.get() == 1 || far_owned.state() != PeerState::Connected
    })
    .await;
    // The unowned peer, same unmount: still up, still carrying bytes.
    assert_eq!(kept.state(), PeerState::Connected);
    let got = Rc::new(Cell::new(0));
    let g = got.clone();
    f_kept.on_message(move |m| g.set(m.len()));
    c_kept.send(b"still here".to_vec()).unwrap();
    until("unowned peer still carries bytes", || got.get() == 10).await;
    kept.close();
    far_kept.close();
    far_owned.close();
}
