//! The web backend's channel edge cases (review of #1915), two peers in one
//! page. Each case is checked against what the browser itself holds, through
//! rinch-rtc's `testing` hooks, not only through rinch's own state.
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch_rtc::testing::{
    ChannelSettings, browser_low_threshold, channel_settings, live_channels,
    pending_gathering_waits,
};
use rinch_rtc::{
    Channel, ChannelOptions, ChannelState, Description, Gathering, IceCandidate, IceServer, Peer,
    PeerState, Reliability, RtcConfig, RtcError, SendError,
};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

async fn tick() {
    let p = js_sys::Promise::new(&mut |resolve, _| {
        let _ = web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 5);
    });
    let _ = wasm_bindgen_futures::JsFuture::from(p).await;
}

async fn until(what: &str, mut cond: impl FnMut() -> bool) {
    for _ in 0..2000 {
        if cond() {
            return;
        }
        tick().await;
    }
    panic!("timed out waiting for {what}");
}

fn firefox() -> bool {
    web_sys::window()
        .and_then(|w| w.navigator().user_agent().ok())
        .is_some_and(|ua| ua.contains("Firefox"))
}

type Inbox = Rc<RefCell<Vec<Option<IceCandidate>>>>;

fn trickle(peer: &Peer) -> Inbox {
    let inbox: Inbox = Rc::default();
    let i = inbox.clone();
    peer.on_ice_candidate(move |c| i.borrow_mut().push(c));
    inbox
}

/// Connect `a` (offerer) and `b`, optionally rewriting the answer.
async fn connect_with(a: &Peer, b: &Peer, rewrite: impl Fn(String) -> String) {
    let a_out = trickle(a);
    let b_out = trickle(b);
    let offer = a.create_offer().await.unwrap();
    b.set_remote_description(offer).await.unwrap();
    let answer = b.create_answer().await.unwrap();
    a.set_remote_description(Description::answer(rewrite(answer.sdp)))
        .await
        .unwrap();
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

async fn connect(a: &Peer, b: &Peer) {
    connect_with(a, b, |s| s).await
}

#[wasm_bindgen_test]
async fn calls_made_inside_on_data_channel_reach_the_browser() {
    let a = Peer::new(RtcConfig::default()).unwrap();
    let b = Peer::new(RtcConfig::default()).unwrap();
    let doomed = a
        .create_data_channel("doomed", ChannelOptions::new(Reliability::Reliable))
        .unwrap();
    let _paced = a
        .create_data_channel("paced", ChannelOptions::new(Reliability::Reliable))
        .unwrap();
    let heard: Rc<RefCell<Vec<Channel>>> = Rc::default();
    let h = heard.clone();
    b.on_data_channel(move |ch| {
        // Both calls go to the browser channel from inside the callback.
        match ch.label().as_str() {
            "doomed" => ch.close(),
            _ => ch.set_buffered_amount_low_threshold(50_000),
        }
        h.borrow_mut().push(ch);
    });
    let a_closed = Rc::new(Cell::new(0));
    let k = a_closed.clone();
    doomed.on_close(move || k.set(k.get() + 1));
    connect(&a, &b).await;
    until("both announced", || heard.borrow().len() == 2).await;
    let paced = heard
        .borrow()
        .iter()
        .find(|c| c.label() == "paced")
        .cloned()
        .unwrap();
    assert_eq!(
        browser_low_threshold(&paced),
        Some(50_000),
        "the threshold set inside on_data_channel reached the browser channel"
    );
    until("the close made inside on_data_channel reaches a", || {
        a_closed.get() == 1
    })
    .await;
    a.close();
    b.close();
}

#[wasm_bindgen_test]
async fn a_channel_the_browser_refuses_errors_then_closes_as_on_native() {
    let a = Peer::new(RtcConfig::default()).unwrap();
    for (label, opts) in [
        // Over 65535 bytes: createDataChannel throws a TypeError.
        (
            "x".repeat(65_536),
            ChannelOptions::new(Reliability::Reliable),
        ),
        // 65535 is not a valid stream id.
        (
            "max-id".to_string(),
            ChannelOptions::negotiated(65_535, Reliability::Reliable),
        ),
    ] {
        let ch = a
            .create_data_channel(&label, opts)
            .expect("refused by the browser, not by rinch");
        let events: Rc<RefCell<Vec<&str>>> = Rc::default();
        let e = events.clone();
        ch.on_error(move |_| e.borrow_mut().push("error"));
        let e = events.clone();
        ch.on_close(move || e.borrow_mut().push("close"));
        until("error then close", || events.borrow().len() == 2).await;
        assert_eq!(*events.borrow(), ["error", "close"]);
        assert_eq!(ch.state(), ChannelState::Closed);
    }
    // The peer still works.
    let ok = a
        .create_data_channel("fine", ChannelOptions::negotiated(7, Reliability::Reliable))
        .unwrap();
    assert_eq!(ok.state(), ChannelState::Connecting);
    a.close();
}

#[wasm_bindgen_test]
async fn a_negotiated_id_already_given_to_an_unopened_in_band_channel_is_refused() {
    let a = Peer::new(RtcConfig::default()).unwrap();
    let b = Peer::new(RtcConfig::default()).unwrap();
    let _first = a
        .create_data_channel("first", ChannelOptions::new(Reliability::Reliable))
        .unwrap();
    connect(&a, &b).await;
    // The DTLS role is known now, so the browser numbers this one at once,
    // before it opens.
    let second = a
        .create_data_channel("second", ChannelOptions::new(Reliability::Reliable))
        .unwrap();
    let sid = second
        .id()
        .expect("rinch read the browser's id back at creation");
    assert!(matches!(
        a.create_data_channel(
            "clash",
            ChannelOptions::negotiated(sid, Reliability::Reliable)
        ),
        Err(RtcError::InvalidState(_))
    ));
    a.close();
    b.close();
}

#[wasm_bindgen_test]
async fn the_browser_holds_the_reliability_each_channel_asked_for() {
    let a = Peer::new(RtcConfig::default()).unwrap();
    let b = Peer::new(RtcConfig::default()).unwrap();
    let unrel = Reliability::Unreliable { max_retransmits: 0 };
    let control = a
        .create_data_channel(
            "control",
            ChannelOptions::negotiated(0, Reliability::Reliable),
        )
        .unwrap();
    let tick_a = a
        .create_data_channel("tick", ChannelOptions::negotiated(1, unrel))
        .unwrap();
    let lossy = a
        .create_data_channel(
            "lossy",
            ChannelOptions::new(Reliability::Unreliable { max_retransmits: 3 }),
        )
        .unwrap();
    let heard: Rc<RefCell<Option<Channel>>> = Rc::default();
    let h = heard.clone();
    b.on_data_channel(move |ch| *h.borrow_mut() = Some(ch));
    let settings = |ordered, max_retransmits, negotiated| {
        Some(ChannelSettings {
            ordered,
            max_retransmits,
            negotiated,
        })
    };
    assert_eq!(channel_settings(&control), settings(true, None, true));
    assert_eq!(channel_settings(&tick_a), settings(false, Some(0), true));
    assert_eq!(channel_settings(&lossy), settings(false, Some(3), false));
    // The far side's channel, as DCEP described it.
    connect(&a, &b).await;
    until("lossy announced", || heard.borrow().is_some()).await;
    let far = heard.borrow().clone().unwrap();
    assert_eq!(channel_settings(&far), settings(false, Some(3), false));
    a.close();
    b.close();
}

#[wasm_bindgen_test]
async fn max_message_size_is_the_browsers_cap_under_a_huge_advertised_size() {
    // Firefox advertises 1073741823. The answer is rewritten to say so; the
    // offering browser's own cap still applies.
    let a = Peer::new(RtcConfig::default()).unwrap();
    let b = Peer::new(RtcConfig::default()).unwrap();
    let opts = ChannelOptions::negotiated(0, Reliability::Reliable);
    let ca = a.create_data_channel("c", opts).unwrap();
    let _cb = b.create_data_channel("c", opts).unwrap();
    connect_with(&a, &b, |sdp| {
        sdp.lines()
            .map(|l| {
                if l.starts_with("a=max-message-size") {
                    "a=max-message-size:1073741823".to_string()
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\r\n")
            + "\r\n"
    })
    .await;
    until("open", || ca.state() == ChannelState::Open).await;
    let max = ca.max_message_size();
    web_sys::console::log_1(&format!("max_message_size under 1073741823: {max}").into());
    if firefox() {
        // Firefox reports the remote's figure itself.
        assert_eq!(max, 1_073_741_823);
    } else {
        assert_eq!(max, 262_144, "Chrome's own cap");
        assert!(matches!(
            ca.send(vec![0u8; 300_000]),
            Err(SendError::TooLarge { max: 262_144, .. })
        ));
    }
    a.close();
    b.close();
}

#[wasm_bindgen_test]
async fn an_ice_restart_changes_the_credentials_and_keeps_the_channels() {
    let a = Peer::new(RtcConfig::default()).unwrap();
    let b = Peer::new(RtcConfig::default()).unwrap();
    let opts = ChannelOptions::negotiated(0, Reliability::Reliable);
    let ca = a.create_data_channel("c", opts).unwrap();
    let cb = b.create_data_channel("c", opts).unwrap();
    let a_out = trickle(&a);
    let b_out = trickle(&b);
    let pump = || {
        for c in a_out.borrow_mut().drain(..) {
            b.add_ice_candidate(c);
        }
        for c in b_out.borrow_mut().drain(..) {
            a.add_ice_candidate(c);
        }
    };
    let offer = a.create_offer().await.unwrap();
    b.set_remote_description(offer).await.unwrap();
    let answer = b.create_answer().await.unwrap();
    a.set_remote_description(answer).await.unwrap();
    until("open", || {
        pump();
        ca.state() == ChannelState::Open
    })
    .await;
    let ufrag = |s: &str| {
        s.lines()
            .find(|l| l.starts_with("a=ice-ufrag"))
            .unwrap_or("")
            .to_string()
    };
    // A plain re-offer keeps the credentials (the positive control)...
    let o1 = a.create_offer().await.unwrap();
    b.set_remote_description(o1.clone()).await.unwrap();
    let an1 = b.create_answer().await.unwrap();
    a.set_remote_description(an1).await.unwrap();
    // ...a restart offer does not.
    a.restart_ice();
    let o2 = a.create_offer().await.unwrap();
    assert_ne!(ufrag(&o1.sdp), ufrag(&o2.sdp));
    b.set_remote_description(o2).await.unwrap();
    let an2 = b.create_answer().await.unwrap();
    a.set_remote_description(an2).await.unwrap();
    let got = Rc::new(Cell::new(0));
    let g = got.clone();
    cb.on_message(move |m| g.set(m.len()));
    ca.send(vec![1u8; 77]).unwrap();
    until("a message after the restart", || {
        pump();
        got.get() == 77
    })
    .await;
    a.close();
    b.close();
}

#[wasm_bindgen_test]
async fn closing_during_a_complete_gathering_wait_settles_it() {
    // An unreachable STUN server keeps gathering going for a long while.
    let a = Peer::new(RtcConfig {
        ice_servers: vec![IceServer::stun("stun:192.0.2.1:3478")],
        gathering: Gathering::Complete,
        ..Default::default()
    })
    .unwrap();
    let _c = a
        .create_data_channel("c", ChannelOptions::negotiated(0, Reliability::Reliable))
        .unwrap();
    let before = pending_gathering_waits();
    let done = Rc::new(Cell::new(false));
    let d = done.clone();
    let offer = a.create_offer();
    wasm_bindgen_futures::spawn_local(async move {
        let _ = offer.await;
        d.set(true);
    });
    until("the offer is waiting for gathering", || {
        pending_gathering_waits() == before + 1
    })
    .await;
    a.close();
    until("the wait is settled", || {
        pending_gathering_waits() == before
    })
    .await;
    until("the offer's future has resolved", || done.get()).await;
}

#[wasm_bindgen_test]
async fn a_closed_channels_closures_go_with_it() {
    let a = Peer::new(RtcConfig::default()).unwrap();
    let b = Peer::new(RtcConfig::default()).unwrap();
    let opts = ChannelOptions::negotiated(0, Reliability::Reliable);
    let keep_a = a.create_data_channel("keep", opts).unwrap();
    let _keep_b = b.create_data_channel("keep", opts).unwrap();
    let opts = ChannelOptions::negotiated(1, Reliability::Reliable);
    let gone_a = a.create_data_channel("gone", opts).unwrap();
    let gone_b = b.create_data_channel("gone", opts).unwrap();
    connect(&a, &b).await;
    until("open", || {
        gone_a.state() == ChannelState::Open && gone_b.state() == ChannelState::Open
    })
    .await;
    let before = live_channels();
    gone_a.close();
    until("both sides closed", || {
        gone_a.state() == ChannelState::Closed && gone_b.state() == ChannelState::Closed
    })
    .await;
    until("their closures freed, the peers still open", || {
        live_channels() == before - 2
    })
    .await;
    assert_eq!(keep_a.state(), ChannelState::Open);
    a.close();
    b.close();
}
