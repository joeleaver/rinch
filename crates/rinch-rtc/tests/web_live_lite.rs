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
/// The whole test's wall-clock budget, from its first line: every wait ends
/// at it. It stays under wasm-bindgen-test-runner's per-binary timeout so
/// that a failure's report always prints before the runner kills the page:
/// the runner's default is 20 s (CI sets 180 s on these steps), and after
/// the budget a timed-out wait still spends up to `2 * FETCH_MS` reading
/// lite_echo's `/stats`, so the worst case is 16 s plus the page's load.
/// The slowest wait measured (#3014, 152 runs in Chrome and Firefox, some
/// with the browser and lite_echo sharing one or two CPUs with busy loops)
/// was 487 ms, and CI's passing runs finish the whole test in about a
/// second: 12 s is more than ten times that.
const BUDGET_MS: f64 = 12_000.0;
/// The bound on each step of a fetch to lite_echo (the response, then its
/// body), so a stuck server cannot hold the test past the runner's timeout.
const FETCH_MS: i32 = 2_000;
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

/// `p`'s outcome, or an error saying it took longer than `FETCH_MS`.
async fn bounded(p: js_sys::Promise) -> Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue> {
    let timer = js_sys::Promise::new(&mut |_, reject| {
        let _ = web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_1(
                &reject,
                FETCH_MS,
                &format!("no answer within {FETCH_MS} ms").into(),
            );
    });
    JsFuture::from(js_sys::Promise::race(&js_sys::Array::of2(&p, &timer))).await
}

/// What the test can see of the connection, for a timeout's message.
struct Probe {
    url: &'static str,
    /// When the test started (`now_ms`); every wait ends at `start + BUDGET_MS`.
    start: f64,
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
            "{:.0} ms into the test (budget {BUDGET_MS} ms); peer {:?}; {}; errors {:?}",
            now_ms() - self.start,
            self.peer.state(),
            channels.join("; "),
            self.errors.borrow()
        )
    }

    /// Whether the test's budget is spent.
    fn expired(&self) -> bool {
        now_ms() - self.start > BUDGET_MS
    }

    /// Panic for a wait on `what` that began at `since`: how long it waited,
    /// what the browser side looks like, and what lite_echo saw (`/stats`).
    async fn fail(&self, what: &str, since: f64) -> ! {
        let waited = now_ms() - since;
        // The browser side as it was at the timeout, before the fetch.
        let browser = self.describe();
        let server = server_stats(self.url).await;
        panic!("timed out waiting for {what} after {waited:.0} ms: {browser}; lite_echo: {server}");
    }
}

/// Wait for `cond` until the test's budget runs out; on a timeout,
/// `Probe::fail` reports.
async fn until(probe: &Probe, what: &str, mut cond: impl FnMut() -> bool) {
    let start = now_ms();
    while !cond() {
        if probe.expired() {
            probe.fail(what, start).await;
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
    let resp = match bounded(window.fetch_with_str(&format!("{url}/stats"))).await {
        Ok(r) => r.dyn_into::<web_sys::Response>().unwrap(),
        Err(e) => return format!("GET /stats failed: {e:?}"),
    };
    match resp.text() {
        Ok(p) => match bounded(p).await {
            Ok(t) => t.as_string().unwrap_or_else(|| "unreadable body".into()),
            Err(e) => format!("body failed: {e:?}"),
        },
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
    let resp: web_sys::Response = bounded(window.fetch_with_request(&req))
        .await
        .expect("lite_echo answers the POST")
        .dyn_into()
        .unwrap();
    assert!(resp.ok(), "lite_echo status {}", resp.status());
    bounded(resp.text().unwrap())
        .await
        .expect("lite_echo's answer has a body")
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
    let start = now_ms();
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
        start,
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
    until(&probe, "both channels open and the peer connected", || {
        control.state() == ChannelState::Open
            && tick_ch.state() == ChannelState::Open
            && peer.state() == PeerState::Connected
    })
    .await;
    // str0m's answer advertises a=max-message-size:262144.
    assert_eq!(control.max_message_size(), 262_144);
    assert!(matches!(
        control.send(vec![0u8; 262_145]),
        Err(SendError::TooLarge { .. })
    ));

    // Bytes that come back crossed the server: the positive control. The
    // ticks go out beside the 200 kB burst, so unordered, unreliable traffic
    // shares the association with a reliable burst in flight.
    control.send(b"hello".to_vec()).unwrap();
    control.send(vec![7u8; 200_000]).unwrap();

    // `tick` is unordered with no retransmits, so a datagram lost on the way
    // takes its message with it: nothing resends it. That is not a fault to
    // test for: the burst overflows lite_echo's UDP receive buffer often
    // enough (the kernel counts the drops) that three ticks sent once beside
    // it all came back only most of the time (#3014). So, as a game resends
    // its state, every tick not yet echoed is resent each `RESEND_MS`; what
    // is tested is that the channel carries messages both ways while the
    // burst is in flight.
    let ticks: Vec<Vec<u8>> = (0..3u8)
        .map(|i| vec![b't', i, 0, 0, 0, 0, 0, 0, 0, 0])
        .collect();
    let back = |msg: &Vec<u8>| got.borrow().iter().any(|(l, m)| l == "tick" && m == msg);
    let mut sends = [0u32; 3];
    let ticks_start = now_ms();
    while !ticks.iter().all(back) {
        if probe.expired() {
            let what = format!("every tick echoed (sends per tick {sends:?})");
            probe.fail(&what, ticks_start).await;
        }
        for (msg, n) in ticks.iter().zip(sends.iter_mut()) {
            if !back(msg) {
                tick_ch.send(msg.clone()).unwrap();
                *n += 1;
            }
        }
        let sent = now_ms();
        while !ticks.iter().all(back) && now_ms() - sent < RESEND_MS && !probe.expired() {
            tick().await;
        }
    }
    web_sys::console::log_1(
        &format!(
            "web_live_lite: every tick echoed after {:.0} ms, sends per tick {sends:?}, \
             control echoed {} B of 200005 by then",
            now_ms() - ticks_start,
            echoed(&got, "control")
        )
        .into(),
    );
    until(&probe, "control echoed", || {
        echoed(&got, "control") == 200_005
    })
    .await;

    // An in-band channel opened by the browser: the server hears and echoes it.
    let chat = peer
        .create_data_channel("chat", ChannelOptions::new(Reliability::Reliable))
        .unwrap();
    watch(&chat, &probe);
    until(&probe, "chat open", || chat.state() == ChannelState::Open).await;
    chat.send(b"hi".to_vec()).unwrap();
    until(&probe, "chat echoed", || echoed(&got, "chat") == 2).await;

    let closed = Rc::new(Cell::new(0));
    let k = closed.clone();
    peer.on_connection_state(move |s| {
        if s == PeerState::Closed {
            k.set(k.get() + 1)
        }
    });
    peer.close();
    until(&probe, "closed", || closed.get() == 1).await;
    assert!(
        !trickled.borrow().is_empty(),
        "the browser trickled candidates"
    );
}
