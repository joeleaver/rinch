//! Two rinch-rtc peers in one app (both native, on the one worker thread):
//! the offerer opens an in-band channel the answerer hears through
//! `on_data_channel`, both declare a pre-negotiated one, candidates trickle
//! both ways through the "app" (this test), and a channel closed on one side
//! closes on the other.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use common::{T, drive, init, pump_until};
use rinch_rtc::{
    Channel, ChannelOptions, ChannelState, IceCandidate, Peer, PeerState, Reliability, RtcConfig,
};

type Inbox = Rc<RefCell<Vec<Option<IceCandidate>>>>;
type Got = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

fn trickle_into(peer: &Peer) -> (Inbox, Rc<Cell<usize>>) {
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

#[test]
fn two_peers_in_one_app() {
    init();
    let a = Peer::new(RtcConfig::default()).unwrap();
    let b = Peer::new(RtcConfig::default()).unwrap();
    let (a_out, a_ends) = trickle_into(&a);
    let (b_out, b_ends) = trickle_into(&b);
    // A candidate added before its remote description would be refused by
    // the stack (and reported here) if rinch-rtc did not hold it.
    let errors: Rc<RefCell<Vec<String>>> = Rc::default();
    for p in [&a, &b] {
        let e = errors.clone();
        p.on_error(move |err| e.borrow_mut().push(err.to_string()));
    }

    let chat = a
        .create_data_channel("chat", ChannelOptions::new(Reliability::Reliable))
        .unwrap();
    let tick_opts = ChannelOptions::negotiated(5, Reliability::Unreliable { max_retransmits: 0 });
    let tick_a = a.create_data_channel("tick", tick_opts).unwrap();
    let tick_b = b.create_data_channel("tick", tick_opts).unwrap();

    let heard: Rc<RefCell<Option<Channel>>> = Rc::default();
    let h = heard.clone();
    b.on_data_channel(move |ch| *h.borrow_mut() = Some(ch));
    let got: Got = Rc::default();
    let g = got.clone();
    tick_b.on_message(move |m| g.borrow_mut().push(("tick".into(), m)));

    // Signalling, the way an app would carry it. Candidates that arrive
    // before the remote description are held by rinch-rtc, so they are
    // forwarded as they come.
    let offer = drive(a.create_offer(), T).unwrap();
    for c in a_out.borrow_mut().drain(..) {
        b.add_ice_candidate(c);
    }
    drive(b.set_remote_description(offer), T).unwrap();
    let answer = drive(b.create_answer(), T).unwrap();
    drive(a.set_remote_description(answer), T).unwrap();
    assert!(
        pump_until(
            || {
                for c in a_out.borrow_mut().drain(..) {
                    b.add_ice_candidate(c);
                }
                for c in b_out.borrow_mut().drain(..) {
                    a.add_ice_candidate(c);
                }
                a.state() == PeerState::Connected && b.state() == PeerState::Connected
            },
            T
        ),
        "both connect: a {:?}, b {:?}",
        a.state(),
        b.state()
    );
    assert!(pump_until(|| a_ends.get() == 1 && b_ends.get() == 1, T));
    assert!(
        errors.borrow().is_empty(),
        "no candidate was refused: {:?}",
        errors.borrow()
    );

    // The in-band channel reaches b.
    assert!(pump_until(|| heard.borrow().is_some(), T));
    let chat_b = heard.borrow().clone().unwrap();
    assert_eq!(chat_b.label(), "chat");
    let g = got.clone();
    chat_b.on_message(move |m| g.borrow_mut().push(("chat".into(), m)));
    assert!(pump_until(
        || chat.state() == ChannelState::Open
            && tick_a.state() == ChannelState::Open
            && tick_b.state() == ChannelState::Open,
        T
    ));
    chat.send(b"over dcep".to_vec()).unwrap();
    tick_a.send(b"over a negotiated id".to_vec()).unwrap();
    assert!(pump_until(|| got.borrow().len() == 2, T));
    let mut got_now = got.borrow().clone();
    got_now.sort();
    assert_eq!(
        got_now,
        vec![
            ("chat".to_string(), b"over dcep".to_vec()),
            ("tick".to_string(), b"over a negotiated id".to_vec()),
        ]
    );

    // A channel closed on one side closes on the other.
    let closed_b = Rc::new(Cell::new(0));
    let k = closed_b.clone();
    chat_b.on_close(move || k.set(k.get() + 1));
    chat.close();
    assert!(
        pump_until(|| closed_b.get() == 1, T),
        "b hears the close; a's chat is {:?}",
        chat.state()
    );
    assert!(pump_until(|| chat.state() == ChannelState::Closed, T));
}
