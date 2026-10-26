//! The interop target from the browser (design record section 4, D10): the
//! web backend against a str0m ICE-lite peer shaped like Playweft's hosted
//! game server, served by `examples/lite_echo.rs`.
//!
//! The URL comes in at build time: `RINCH_RTC_TEST_URL=http://127.0.0.1:N`.
//! Without it the test logs that it skipped, **unless**
//! `RINCH_RTC_REQUIRE_LIVE=1` was also set at build time (CI does), when it
//! fails: a skip there must not read as a pass.
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch_rtc::{
    Channel, ChannelOptions, ChannelState, Description, IceCandidate, Peer, PeerState, Reliability,
    RtcConfig, SendError,
};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const URL: Option<&str> = option_env!("RINCH_RTC_TEST_URL");
const REQUIRE: Option<&str> = option_env!("RINCH_RTC_REQUIRE_LIVE");

async fn tick() {
    let p = js_sys::Promise::new(&mut |resolve, _| {
        let _ = web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 5);
    });
    let _ = JsFuture::from(p).await;
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

/// POST the offer to lite_echo, return its answer.
async fn signal(url: &str, offer: &str) -> String {
    let init = web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_mode(web_sys::RequestMode::Cors);
    init.set_body(&offer.into());
    let req = web_sys::Request::new_with_str_and_init(&format!("{url}/offer"), &init).unwrap();
    let window = web_sys::window().unwrap();
    let resp: web_sys::Response = JsFuture::from(window.fetch_with_request(&req))
        .await
        .expect("lite_echo answers the POST")
        .dyn_into()
        .unwrap();
    assert!(resp.ok(), "lite_echo status {}", resp.status());
    JsFuture::from(resp.text().unwrap())
        .await
        .unwrap()
        .as_string()
        .unwrap()
}

type Got = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

fn record(ch: &Channel, got: &Got) {
    let (g, label) = (got.clone(), ch.label());
    ch.on_message(move |m| g.borrow_mut().push((label.clone(), m)));
}

fn echoed(got: &Got, label: &str) -> usize {
    got.borrow()
        .iter()
        .filter(|(l, _)| l == label)
        .map(|(_, b)| b.len())
        .sum()
}

#[wasm_bindgen_test]
async fn the_browser_reaches_a_str0m_ice_lite_peer() {
    let Some(url) = URL else {
        assert!(
            REQUIRE.is_none(),
            "RINCH_RTC_REQUIRE_LIVE is set but RINCH_RTC_TEST_URL is not: the live \
             interop test would not run"
        );
        web_sys::console::log_1(
            &"web_live_lite SKIPPED: build with RINCH_RTC_TEST_URL (see the file's docs)".into(),
        );
        return;
    };
    let peer = Peer::new(RtcConfig::default()).unwrap();
    let control = peer
        .create_data_channel(
            "control",
            ChannelOptions::negotiated(0, Reliability::Reliable),
        )
        .unwrap();
    let tick_ch = peer
        .create_data_channel(
            "tick",
            ChannelOptions::negotiated(1, Reliability::Unreliable { max_retransmits: 0 }),
        )
        .unwrap();
    let got: Got = Rc::default();
    record(&control, &got);
    record(&tick_ch, &got);
    // Trickled candidates are never sent: an ICE-lite host learns the
    // browser's address from its checks.
    let trickled: Rc<RefCell<Vec<Option<IceCandidate>>>> = Rc::default();
    let t = trickled.clone();
    peer.on_ice_candidate(move |c| t.borrow_mut().push(c));

    let offer = peer.create_offer().await.unwrap();
    let answer = signal(url, &offer.sdp).await;
    peer.set_remote_description(Description::answer(answer))
        .await
        .unwrap();
    until("both channels open", || {
        control.state() == ChannelState::Open && tick_ch.state() == ChannelState::Open
    })
    .await;
    assert_eq!(peer.state(), PeerState::Connected);
    // str0m's answer advertises a=max-message-size:262144.
    assert_eq!(control.max_message_size(), 262_144);
    assert!(matches!(
        control.send(vec![0u8; 262_145]),
        Err(SendError::TooLarge { .. })
    ));

    // Bytes that come back crossed the server: the positive control.
    control.send(b"hello".to_vec()).unwrap();
    control.send(vec![7u8; 200_000]).unwrap();
    for i in 0..3u8 {
        tick_ch.send(vec![i; 10]).unwrap();
    }
    until("control echoed", || echoed(&got, "control") == 200_005).await;
    until("tick echoed", || echoed(&got, "tick") == 30).await;

    // An in-band channel opened by the browser: the server hears and echoes it.
    let chat = peer
        .create_data_channel("chat", ChannelOptions::new(Reliability::Reliable))
        .unwrap();
    record(&chat, &got);
    until("chat open", || chat.state() == ChannelState::Open).await;
    chat.send(b"hi".to_vec()).unwrap();
    until("chat echoed", || echoed(&got, "chat") == 2).await;

    let closed = Rc::new(Cell::new(0));
    let k = closed.clone();
    peer.on_connection_state(move |s| {
        if s == PeerState::Closed {
            k.set(k.get() + 1)
        }
    });
    peer.close();
    until("closed", || closed.get() == 1).await;
    assert!(
        !trickled.borrow().is_empty(),
        "the browser trickled candidates"
    );
}
