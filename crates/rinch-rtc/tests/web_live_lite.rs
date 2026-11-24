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
/// Each wait's wall-clock deadline. The slowest wait measured (#3014, 152
/// runs in Chrome and Firefox, some with the browser and lite_echo sharing
/// one or two CPUs with busy loops) was 487 ms, and CI's passing runs finish
/// the whole test in about a second: 10 s is some twenty times that, and was
/// the old loop's nominal budget (2000 timers of 5 ms, which counted ticks,
/// not time).
const DEADLINE_MS: f64 = 10_000.0;
/// How long a tick waits for its echo before it is sent again: about ten
/// times the measured round trip (26-27 ms; lite_echo's 20 ms read timeout
/// is most of it).
const RESEND_MS: f64 = 250.0;

async fn tick() {
    let p = js_sys::Promise::new(&mut |resolve, _| {
        let _ = web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 5);
    });
    let _ = JsFuture::from(p).await;
}

/// Wall-clock milliseconds (`performance.now()`).
fn now_ms() -> f64 {
    web_sys::window().unwrap().performance().unwrap().now()
}

/// What the test can see of the connection, for a timeout's message.
struct Probe {
    url: &'static str,
    peer: Peer,
    channels: RefCell<Vec<Channel>>,
    got: Got,
    errors: Rc<RefCell<Vec<String>>>,
}

impl Probe {
    fn describe(&self) -> String {
        let channels: Vec<String> = self
            .channels
            .borrow()
            .iter()
            .map(|c| {
                format!(
                    "{}: {:?}, buffered {}, echoed {} B",
                    c.label(),
                    c.state(),
                    c.buffered_amount(),
                    echoed(&self.got, &c.label())
                )
            })
            .collect();
        format!(
            "peer {:?}; {}; errors {:?}",
            self.peer.state(),
            channels.join("; "),
            self.errors.borrow()
        )
    }
}

/// Wait for `cond`, for at most `deadline_ms` of wall-clock time. On a
/// timeout the panic says how long it waited, what the browser side looks
/// like, and what lite_echo saw (its `/stats`).
async fn until(probe: &Probe, what: &str, deadline_ms: f64, mut cond: impl FnMut() -> bool) {
    let start = now_ms();
    while !cond() {
        if now_ms() - start > deadline_ms {
            let server = server_stats(probe.url).await;
            panic!(
                "timed out waiting for {what} after {:.0} ms (deadline {deadline_ms} ms): {}; \
                 lite_echo: {server}",
                now_ms() - start,
                probe.describe()
            );
        }
        tick().await;
    }
    web_sys::console::log_1(
        &format!("web_live_lite: {what} after {:.0} ms", now_ms() - start).into(),
    );
}

/// lite_echo's `GET /stats`, or why it could not be read.
async fn server_stats(url: &str) -> String {
    let window = web_sys::window().unwrap();
    let resp = match JsFuture::from(window.fetch_with_str(&format!("{url}/stats"))).await {
        Ok(r) => r.dyn_into::<web_sys::Response>().unwrap(),
        Err(e) => return format!("GET /stats failed: {e:?}"),
    };
    match resp.text() {
        Ok(p) => JsFuture::from(p)
            .await
            .ok()
            .and_then(|t| t.as_string())
            .unwrap_or_else(|| "unreadable body".into()),
        Err(e) => format!("no body: {e:?}"),
    }
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

/// Record `ch`'s messages and errors, and show it in `probe`'s reports.
fn watch(ch: &Channel, probe: &Probe) {
    let (g, label) = (probe.got.clone(), ch.label());
    ch.on_message(move |m| g.borrow_mut().push((label.clone(), m)));
    let (e, label) = (probe.errors.clone(), ch.label());
    ch.on_error(move |err| e.borrow_mut().push(format!("{label}: {err}")));
    probe.channels.borrow_mut().push(ch.clone());
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
    let errors: Rc<RefCell<Vec<String>>> = Rc::default();
    let peer = Peer::new(RtcConfig::default()).unwrap();
    let e = errors.clone();
    peer.on_error(move |err| e.borrow_mut().push(format!("peer: {err}")));
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
    let probe = Probe {
        url,
        peer: peer.clone(),
        channels: RefCell::new(Vec::new()),
        got: got.clone(),
        errors: errors.clone(),
    };
    watch(&control, &probe);
    watch(&tick_ch, &probe);
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
    // The browser fires a channel's `open` and the connection's
    // `connectionstatechange` as separate tasks, in either order: a channel
    // can be open while `state()` still says `Connecting` (seen under load,
    // #3014). So the wait is for both, not an assertion after the first.
    until(
        &probe,
        "both channels open and the peer connected",
        DEADLINE_MS,
        || {
            control.state() == ChannelState::Open
                && tick_ch.state() == ChannelState::Open
                && peer.state() == PeerState::Connected
        },
    )
    .await;
    // str0m's answer advertises a=max-message-size:262144.
    assert_eq!(control.max_message_size(), 262_144);
    assert!(matches!(
        control.send(vec![0u8; 262_145]),
        Err(SendError::TooLarge { .. })
    ));

    // Bytes that come back crossed the server: the positive control.
    control.send(b"hello".to_vec()).unwrap();
    control.send(vec![7u8; 200_000]).unwrap();
    until(&probe, "control echoed", DEADLINE_MS, || {
        echoed(&got, "control") == 200_005
    })
    .await;

    // `tick` is unordered with no retransmits, so a datagram lost on the way
    // takes its message with it: nothing resends it. That is not a fault to
    // test for: the 200 kB burst above overflows lite_echo's UDP receive
    // buffer often enough (the kernel counts the drops) that three ticks
    // sent beside it all came back only most of the time (#3014). So, as a
    // game resends its state, each tick is resent until its echo arrives;
    // what is tested is that the channel carries messages both ways.
    for i in 0..3u8 {
        let msg = vec![b't', i, 0, 0, 0, 0, 0, 0, 0, 0];
        let back = || got.borrow().iter().any(|(l, m)| l == "tick" && *m == msg);
        let start = now_ms();
        let mut sends = 0;
        while !back() {
            if now_ms() - start > DEADLINE_MS {
                panic!(
                    "tick {i} never echoed: sent {sends} times over {:.0} ms: {}; lite_echo: {}",
                    now_ms() - start,
                    probe.describe(),
                    server_stats(url).await
                );
            }
            tick_ch.send(msg.clone()).unwrap();
            sends += 1;
            let sent = now_ms();
            while !back() && now_ms() - sent < RESEND_MS {
                tick().await;
            }
        }
        web_sys::console::log_1(
            &format!(
                "web_live_lite: tick {i} echoed after {:.0} ms, {sends} send(s)",
                now_ms() - start
            )
            .into(),
        );
    }

    // An in-band channel opened by the browser: the server hears and echoes it.
    let chat = peer
        .create_data_channel("chat", ChannelOptions::new(Reliability::Reliable))
        .unwrap();
    watch(&chat, &probe);
    until(&probe, "chat open", DEADLINE_MS, || {
        chat.state() == ChannelState::Open
    })
    .await;
    chat.send(b"hi".to_vec()).unwrap();
    until(&probe, "chat echoed", DEADLINE_MS, || {
        echoed(&got, "chat") == 2
    })
    .await;

    let closed = Rc::new(Cell::new(0));
    let k = closed.clone();
    peer.on_connection_state(move |s| {
        if s == PeerState::Closed {
            k.set(k.get() + 1)
        }
    });
    peer.close();
    until(&probe, "closed", DEADLINE_MS, || closed.get() == 1).await;
    assert!(
        !trickled.borrow().is_empty(),
        "the browser trickled candidates"
    );
}
