//! The interop target (design record section 4): rinch-rtc's native client
//! against a str0m ICE-lite peer shaped like Playweft's hosted game server.
//!
//! One `#[test]`: the main-thread stand-in is process-global. The cases run
//! in order, each against its own lite peer.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use common::{LitePeer, T, drive, init, pump_until};
use rinch_rtc::{
    Channel, ChannelOptions, ChannelState, Description, Gathering, PeerState, Peer, Reliability,
    RtcConfig, SendError,
};

type Log = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

struct Client {
    peer: Peer,
    control: Channel,
    tick: Channel,
    log: Log,
    states: Rc<RefCell<Vec<PeerState>>>,
    candidates: Rc<Cell<usize>>,
    ends: Rc<Cell<usize>>,
}

fn record(ch: &Channel, log: &Log) {
    let (log, label) = (log.clone(), ch.label());
    ch.on_message(move |b| log.borrow_mut().push((label.clone(), b)));
}

/// Offer to `lite` and connect, Playweft's way: two pre-negotiated channels,
/// trickle, the client's candidates never sent to the lite peer.
fn connect(lite: &LitePeer, gathering: Gathering) -> Client {
    let peer = Peer::new(RtcConfig {
        gathering,
        ..Default::default()
    })
    .unwrap();
    let control = peer
        .create_data_channel("control", ChannelOptions::negotiated(0, Reliability::Reliable))
        .unwrap();
    let tick = peer
        .create_data_channel(
            "tick",
            ChannelOptions::negotiated(1, Reliability::Unreliable { max_retransmits: 0 }),
        )
        .unwrap();
    let log: Log = Rc::default();
    record(&control, &log);
    record(&tick, &log);
    let states: Rc<RefCell<Vec<PeerState>>> = Rc::default();
    let s = states.clone();
    peer.on_connection_state(move |st| s.borrow_mut().push(st));
    let (candidates, ends) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let (c, e) = (candidates.clone(), ends.clone());
    peer.on_ice_candidate(move |cand| match cand {
        Some(_) => c.set(c.get() + 1),
        None => e.set(e.get() + 1),
    });

    let offer = drive(peer.create_offer(), T).expect("offer");
    let answer = lite.answer(&offer.sdp);
    drive(peer.set_remote_description(Description::answer(answer)), T).expect("answer");
    assert!(
        pump_until(
            || control.state() == ChannelState::Open && tick.state() == ChannelState::Open,
            T
        ),
        "both pre-negotiated channels open against the lite peer; states {:?}",
        states.borrow()
    );
    Client {
        peer,
        control,
        tick,
        log,
        states,
        candidates,
        ends,
    }
}

fn echoed(log: &Log, label: &str) -> usize {
    log.borrow()
        .iter()
        .filter(|(l, _)| l == label)
        .map(|(_, b)| b.len())
        .sum()
}

#[test]
fn interop_with_a_str0m_ice_lite_peer() {
    init();
    negotiated_channels_echo_and_the_server_counts_them();
    complete_gathering_puts_the_candidates_in_the_sdp();
    in_band_channels_both_ways();
    send_refusals_and_the_low_water_mark();
    the_server_closing_a_channel_closes_it();
    close_closes_every_channel_then_the_peer();
    ice_restart_keeps_the_channels();
    many_peers_on_one_worker();
    // Last: it shortens ICE timers for every peer made after it.
    a_silent_server_goes_disconnected_then_failed();
}

fn negotiated_channels_echo_and_the_server_counts_them() {
    let lite = LitePeer::spawn();
    let c = connect(&lite, Gathering::Trickle);
    assert_eq!(c.peer.state(), PeerState::Connected);
    assert_eq!(c.control.id(), Some(0));
    assert_eq!(c.tick.id(), Some(1));
    // str0m's answer has no a=max-message-size: RFC 8841's 64 KiB.
    assert_eq!(c.control.max_message_size(), 65_536);

    c.control.send(b"hello".to_vec()).unwrap();
    c.control.send(vec![7u8; 60_000]).unwrap();
    for i in 0..3u8 {
        c.tick.send(vec![i; 10]).unwrap();
    }
    // Positive control: the server's own count, not the client's callbacks.
    assert!(
        pump_until(
            || {
                let s = lite.stats();
                s.received.get("control") == Some(&60_005) && s.messages.get("tick").is_some()
            },
            T
        ),
        "server stats {:?}",
        lite.stats()
    );
    assert!(pump_until(|| echoed(&c.log, "control") == 60_005, T));
    // Unreliable: on loopback all three arrive, in some order.
    assert!(pump_until(|| echoed(&c.log, "tick") == 30, T));
    // Trickle: candidates were offered, and end-of-candidates came once.
    assert!(pump_until(|| c.ends.get() == 1, T));
    assert!(c.candidates.get() >= 1);
    assert_eq!(c.states.borrow().last(), Some(&PeerState::Connected));
}

fn complete_gathering_puts_the_candidates_in_the_sdp() {
    let lite = LitePeer::spawn();
    let peer = Peer::new(RtcConfig {
        gathering: Gathering::Complete,
        ..Default::default()
    })
    .unwrap();
    let control = peer
        .create_data_channel("control", ChannelOptions::negotiated(0, Reliability::Reliable))
        .unwrap();
    let trickled = Rc::new(Cell::new(0));
    let t = trickled.clone();
    peer.on_ice_candidate(move |_| t.set(t.get() + 1));
    let offer = drive(peer.create_offer(), T).unwrap();
    assert!(
        offer.sdp.contains("a=candidate:"),
        "a Complete offer carries its candidates:\n{}",
        offer.sdp
    );
    let answer = lite.answer(&offer.sdp);
    drive(peer.set_remote_description(Description::answer(answer)), T).unwrap();
    assert!(pump_until(|| control.state() == ChannelState::Open, T));
    assert_eq!(trickled.get(), 0, "Complete calls on_ice_candidate never");
}

fn in_band_channels_both_ways() {
    let lite = LitePeer::spawn();
    let c = connect(&lite, Gathering::Trickle);
    // Client-opened, in-band: the server hears it.
    let chat = c
        .peer
        .create_data_channel("chat", ChannelOptions::new(Reliability::Reliable))
        .unwrap();
    record(&chat, &c.log);
    assert!(pump_until(|| chat.state() == ChannelState::Open, T));
    assert!(pump_until(|| lite.stats().opened.contains(&"chat".to_string()), T));
    chat.send(b"hi".to_vec()).unwrap();
    assert!(pump_until(|| echoed(&c.log, "chat") == 2, T));
    assert_eq!(chat.id(), None, "native does not report an in-band stream id");

    // Server-opened, in-band: on_data_channel.
    let heard: Rc<RefCell<Option<Channel>>> = Rc::default();
    let h = heard.clone();
    c.peer.on_data_channel(move |ch| *h.borrow_mut() = Some(ch));
    lite.open_in_band("from-server");
    assert!(pump_until(|| heard.borrow().is_some(), T));
    let srv = heard.borrow().clone().unwrap();
    assert_eq!(srv.label(), "from-server");
    record(&srv, &c.log);
    assert!(pump_until(|| srv.state() == ChannelState::Open, T));
    srv.send(b"back".to_vec()).unwrap();
    assert!(pump_until(|| echoed(&c.log, "from-server") == 4, T));
    assert_eq!(lite.stats().received.get("from-server"), Some(&4));
}

fn send_refusals_and_the_low_water_mark() {
    let lite = LitePeer::spawn();
    let peer = Peer::new(RtcConfig::default()).unwrap();
    let not_open = peer
        .create_data_channel(
            "control",
            ChannelOptions::negotiated(0, Reliability::Reliable).with_send_buffer_limit(100_000),
        )
        .unwrap();
    assert_eq!(not_open.send(b"x".to_vec()), Err(SendError::NotOpen));
    let offer = drive(peer.create_offer(), T).unwrap();
    drive(
        peer.set_remote_description(Description::answer(lite.answer(&offer.sdp))),
        T,
    )
    .unwrap();
    let ch = not_open;
    assert!(pump_until(|| ch.state() == ChannelState::Open, T));
    let lows = Rc::new(Cell::new(0));
    let l = lows.clone();
    ch.on_buffered_amount_low(move || l.set(l.get() + 1));

    assert_eq!(
        ch.send(vec![0u8; 65_537]),
        Err(SendError::TooLarge {
            len: 65_537,
            max: 65_536
        })
    );
    // An empty queue admits one; the next would pass the 100 000 limit and
    // is refused, synchronously, with nothing queued.
    ch.send(vec![1u8; 60_000]).unwrap();
    assert!(ch.buffered_amount() >= 60_000);
    match ch.send(vec![2u8; 60_000]) {
        Err(SendError::QueueFull { limit: 100_000, .. }) => {}
        other => panic!("expected QueueFull, got {other:?}"),
    }
    assert!(pump_until(|| lows.get() >= 1 && ch.buffered_amount() == 0, T));
    // The channel is still open, and the queue takes the next one.
    assert_eq!(ch.state(), ChannelState::Open);
    ch.send(vec![3u8; 60_000]).unwrap();
    // Positive control: only the two admitted messages reached the server.
    assert!(pump_until(
        || lite.stats().received.get("control") == Some(&120_000),
        T
    ));
}

fn the_server_closing_a_channel_closes_it() {
    let lite = LitePeer::spawn();
    let c = connect(&lite, Gathering::Trickle);
    let closed = Rc::new(Cell::new(0));
    let k = closed.clone();
    // Used first: str0m 0.24.1 does not get a close through on a stream that
    // has carried nothing yet (closing right after the open is lost on
    // the way, with the client still Open; webrtc-rs to webrtc-rs is not
    // affected, see two_peers.rs). A game's tick channel has carried
    // ticks by the time the server closes it.
    c.tick.send(b"t".to_vec()).unwrap();
    assert!(pump_until(|| echoed(&c.log, "tick") == 1, T));
    c.tick.on_close(move || k.set(k.get() + 1));
    lite.close_channel("tick");
    assert!(
        pump_until(|| closed.get() == 1, T),
        "tick did not close; server stats {:?}, client tick state {:?}",
        lite.stats(),
        c.tick.state()
    );
    assert_eq!(c.tick.state(), ChannelState::Closed);
    assert_eq!(c.tick.send(b"x".to_vec()), Err(SendError::NotOpen));
    // The other channel and the peer are unaffected.
    c.control.send(b"still".to_vec()).unwrap();
    assert!(pump_until(|| echoed(&c.log, "control") == 5, T));
    assert_eq!(c.peer.state(), PeerState::Connected);
}

fn close_closes_every_channel_then_the_peer() {
    let lite = LitePeer::spawn();
    let c = connect(&lite, Gathering::Trickle);
    let order: Rc<RefCell<Vec<String>>> = Rc::default();
    for ch in [&c.control, &c.tick] {
        let (o, label) = (order.clone(), ch.label());
        ch.on_close(move || o.borrow_mut().push(label.clone()));
    }
    let o = order.clone();
    c.peer.on_connection_state(move |s| o.borrow_mut().push(format!("{s:?}")));
    c.peer.close();
    assert_eq!(c.peer.state(), PeerState::Closed, "Closed at once");
    assert!(pump_until(|| order.borrow().contains(&"Closed".to_string()), T));
    let order = order.borrow().clone();
    assert_eq!(order.len(), 3, "each channel once, then the peer: {order:?}");
    assert_eq!(order.last().unwrap(), "Closed");
    assert!(c.control.send(b"x".to_vec()).is_err());
    // The server sees the association end.
    assert!(
        pump_until(|| lite.stats().closed.len() >= 2, T),
        "server stats {:?}",
        lite.stats()
    );
}

fn ice_restart_keeps_the_channels() {
    let lite = LitePeer::spawn();
    let c = connect(&lite, Gathering::Trickle);
    c.peer.restart_ice();
    let offer = drive(c.peer.create_offer(), T).unwrap();
    let answer = lite.answer(&offer.sdp);
    assert_eq!(lite.stats().offers, 2);
    drive(c.peer.set_remote_description(Description::answer(answer)), T).unwrap();
    c.control.send(b"after-restart".to_vec()).unwrap();
    assert!(pump_until(|| echoed(&c.log, "control") == 13, T));
    assert_eq!(c.control.state(), ChannelState::Open);
}

fn many_peers_on_one_worker() {
    let lites: Vec<LitePeer> = (0..4).map(|_| LitePeer::spawn()).collect();
    let clients: Vec<Client> = lites
        .iter()
        .map(|l| connect(l, Gathering::Trickle))
        .collect();
    for (i, c) in clients.iter().enumerate() {
        c.control.send(vec![i as u8; 100 + i]).unwrap();
    }
    for (i, (c, l)) in clients.iter().zip(&lites).enumerate() {
        assert!(pump_until(|| echoed(&c.log, "control") == 100 + i, T));
        assert_eq!(l.stats().received.get("control"), Some(&(100 + i)));
    }
}

fn a_silent_server_goes_disconnected_then_failed() {
    rinch_rtc::testing::set_ice_timeouts(
        Duration::from_millis(800),
        Duration::from_secs(2),
        Duration::from_millis(200),
    );
    let mut lite = LitePeer::spawn();
    let c = connect(&lite, Gathering::Trickle);
    lite.stop();
    assert!(
        pump_until(|| c.states.borrow().contains(&PeerState::Failed), T),
        "states {:?}",
        c.states.borrow()
    );
    let states = c.states.borrow().clone();
    let d = states.iter().position(|s| *s == PeerState::Disconnected);
    let f = states.iter().position(|s| *s == PeerState::Failed);
    assert!(
        d.is_some() && d < f,
        "Disconnected comes before Failed: {states:?}"
    );
}
