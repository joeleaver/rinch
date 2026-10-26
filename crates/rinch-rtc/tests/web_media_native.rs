//! Browser ↔ native media (design record `docs/design/rtc-media.md`,
//! section 6): the browser sends a moving box as H.264 to
//! `examples/media_echo.rs`, a native rinch-rtc peer that decodes it with
//! openh264 and sends it back re-encoded; the box must come back. Both
//! directions of openh264 ↔ the browser's H.264 are in this one loop.
//!
//! The URL comes in at build time: `RINCH_RTC_MEDIA_URL=http://127.0.0.1:N`.
//! Without it the test logs that it skipped, unless `RINCH_RTC_REQUIRE_LIVE=1`
//! was also set (CI does), when it fails.
//!
//! **Firefox** gets H.264 from Cisco's OpenH264 plugin, which it downloads
//! itself. A Firefox without it offers no H.264, and the native side (H.264
//! only, M2) cannot answer its video. That is reported as a failure naming
//! the cause, not skipped.
#![cfg(all(target_arch = "wasm32", feature = "media-core"))]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use rinch_rtc::media::{LocalTrack, MediaKind, RemoteTrack, VideoSourceOptions, VideoSourceWriter};
use rinch_rtc::{Description, Gathering, Peer, PeerState, RtcConfig};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const URL: Option<&str> = option_env!("RINCH_RTC_MEDIA_URL");
const REQUIRE: Option<&str> = option_env!("RINCH_RTC_REQUIRE_LIVE");
const W: u32 = 320;
const H: u32 = 240;
const BOX: u32 = 64;

async fn sleep(ms: i32) {
    let p = js_sys::Promise::new(&mut |resolve, _| {
        let _ = web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
    });
    let _ = JsFuture::from(p).await;
}

async fn until(what: &str, secs: u32, mut cond: impl FnMut() -> bool) {
    for _ in 0..secs * 50 {
        if cond() {
            return;
        }
        sleep(20).await;
    }
    panic!("timed out waiting for {what}");
}

async fn signal(url: &str, offer: &str) -> String {
    let init = web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_mode(web_sys::RequestMode::Cors);
    init.set_body(&offer.into());
    let req = web_sys::Request::new_with_str_and_init(&format!("{url}/offer"), &init).unwrap();
    let resp: web_sys::Response =
        JsFuture::from(web_sys::window().unwrap().fetch_with_request(&req))
            .await
            .expect("media_echo answers the POST")
            .dyn_into()
            .unwrap();
    let status = resp.status();
    let text = JsFuture::from(resp.text().unwrap())
        .await
        .unwrap()
        .as_string()
        .unwrap();
    assert_eq!(status, 200, "media_echo: {text}");
    text
}

fn moving_box(writer: VideoSourceWriter) -> (i32, Closure<dyn FnMut()>) {
    let mut k = 0u32;
    let cb = Closure::<dyn FnMut()>::new(move || {
        let x = (k * 4) % (W - BOX);
        let y = (k * 3) % (H - BOX);
        k += 1;
        let mut f = vec![0u8; (W * H * 4) as usize];
        for px in f.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
        for row in y..y + BOX {
            for col in x..x + BOX {
                let i = ((row * W + col) * 4) as usize;
                f[i..i + 3].copy_from_slice(&[255, 255, 255]);
            }
        }
        writer.push_rgba(&f, W, H);
    });
    let id = web_sys::window()
        .unwrap()
        .set_interval_with_callback_and_timeout_and_arguments_0(cb.as_ref().unchecked_ref(), 33)
        .unwrap();
    (id, cb)
}

fn is_box(rgba: &[u8], w: u32, h: u32) -> bool {
    let (mut x0, mut y0, mut x1, mut y1, mut n) = (u32::MAX, u32::MAX, 0, 0, 0u32);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let luma = (u32::from(rgba[i]) + u32::from(rgba[i + 1]) + u32::from(rgba[i + 2])) / 3;
            if luma > 128 {
                n += 1;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    let near = |v: u32| v.abs_diff(BOX) <= BOX / 5 + 4;
    n > 0 && near(x1 - x0 + 1) && near(y1 - y0 + 1) && n * 10 >= BOX * BOX * 7
}

/// Frames this browser's own H.264 encoder produces in 3 s of a
/// browser-only loopback (two RTCPeerConnections, H.264 forced).
async fn browser_h264_frames() -> f64 {
    let p: js_sys::Promise = js_sys::eval(
        "(async () => { const a = new RTCPeerConnection(), b = new RTCPeerConnection(); \
          a.onicecandidate = e => e.candidate && b.addIceCandidate(e.candidate); \
          b.onicecandidate = e => e.candidate && a.addIceCandidate(e.candidate); \
          const c = document.createElement('canvas'); c.width = 160; c.height = 120; \
          const x = c.getContext('2d'); let n = 0; \
          const t = setInterval(() => { x.fillStyle = n++ % 2 ? '#fff' : '#000'; x.fillRect(0, 0, 160, 120); }, 33); \
          const tr = a.addTransceiver(c.captureStream(30).getVideoTracks()[0], { direction: 'sendonly' }); \
          const h264 = RTCRtpSender.getCapabilities('video').codecs.filter(k => /H264/i.test(k.mimeType)); \
          if (!h264.length) { clearInterval(t); a.close(); b.close(); return 0; } \
          tr.setCodecPreferences(h264); \
          await a.setLocalDescription(); await b.setRemoteDescription(a.localDescription); \
          await b.setLocalDescription(); await a.setRemoteDescription(b.localDescription); \
          await new Promise(r => setTimeout(r, 3000)); \
          let f = 0; (await a.getStats()).forEach(s => { if (s.type === 'outbound-rtp') f += s.framesEncoded || 0; }); \
          clearInterval(t); a.close(); b.close(); return f; })()",
    )
    .unwrap()
    .unchecked_into();
    JsFuture::from(p).await.unwrap().as_f64().unwrap_or(0.0)
}

#[wasm_bindgen_test]
async fn h264_round_trips_through_a_native_peer() {
    let Some(url) = URL else {
        assert!(
            REQUIRE.is_none(),
            "RINCH_RTC_REQUIRE_LIVE is set but RINCH_RTC_MEDIA_URL is not: the browser-native \
             media test would not run"
        );
        web_sys::console::log_1(&"web_media_native: skipped (no RINCH_RTC_MEDIA_URL)".into());
        return;
    };
    let peer = Peer::new(RtcConfig {
        gathering: Gathering::Complete,
        ..Default::default()
    })
    .unwrap();
    let errors: Rc<RefCell<Vec<String>>> = Rc::default();
    let e = errors.clone();
    peer.on_error(move |err| e.borrow_mut().push(err.to_string()));
    let (track, writer) = LocalTrack::video_source(VideoSourceOptions {
        width: W,
        height: H,
        fps: 30,
    })
    .unwrap();
    let (timer, _feed) = moving_box(writer);
    peer.add_track(&track).unwrap();
    let got: Rc<RefCell<Vec<RemoteTrack>>> = Rc::default();
    let g = got.clone();
    peer.on_track(move |t| g.borrow_mut().push(t));

    // The cause, named, when this browser cannot encode H.264. Firefox
    // lists H.264 in its SDP and capabilities even without Cisco's
    // OpenH264 plugin, then encodes nothing; only sending tells.
    let ua = web_sys::window()
        .unwrap()
        .navigator()
        .user_agent()
        .unwrap_or_default();
    let encoded = browser_h264_frames().await;
    assert!(
        encoded > 0.0,
        "this browser encodes no H.264 ({ua}): 0 frames in 3 s of a browser-only H.264 \
         loopback. In Firefox that is Cisco's OpenH264 plugin missing (Firefox downloads \
         it itself). The native side is H.264 only (M2), so video between desktop and this \
         browser cannot work until VP8 is added there."
    );
    let offer = peer.create_offer().await.unwrap();
    let answer = signal(url, &offer.sdp).await;
    peer.set_remote_description(Description::answer(answer))
        .await
        .unwrap();
    until("connected", 20, || peer.state() == PeerState::Connected).await;
    until("the echoed track", 20, || {
        got.borrow().iter().any(|t| t.kind() == MediaKind::Video)
    })
    .await;
    let remote = got
        .borrow()
        .iter()
        .find(|t| t.kind() == MediaKind::Video)
        .cloned()
        .unwrap();
    let frames: Arc<Mutex<Vec<bool>>> = Arc::default();
    let f = frames.clone();
    remote.on_frame(move |rgba, w, h| {
        if w == W && h == H {
            f.lock().unwrap().push(is_box(rgba, w, h));
        }
    });
    until("echoed frames", 30, || frames.lock().unwrap().len() >= 60).await;
    let seen = frames.lock().unwrap().clone();
    let boxes = seen.iter().filter(|b| **b).count();
    assert!(
        boxes * 10 >= seen.len() * 7,
        "{boxes} of {} echoed frames show the box",
        seen.len()
    );
    assert!(errors.borrow().is_empty(), "{:?}", errors.borrow());
    web_sys::window().unwrap().clear_interval_with_handle(timer);
    peer.close();
}
