//! Browser ↔ native media (design record `docs/design/rtc-media.md`,
//! section 6): the browser sends a moving box to `examples/media_echo.rs`,
//! a native rinch-rtc peer that decodes it and sends it back re-encoded;
//! the box must come back. Both directions of the codec interop are in this
//! one loop.
//!
//! - `RINCH_RTC_MEDIA_URL`: a `media_echo` built with `media-vp8`, so with
//!   both codecs. A browser offers VP8 first, so this is **VP8** (libvpx on
//!   the native side). It must pass in every browser, Firefox included: VP8
//!   is why desktop video works with a Firefox that has no OpenH264 plugin
//!   (M2, 2026-10-27).
//! - `RINCH_RTC_MEDIA_H264_URL` (optional): a `media_echo` built with
//!   `media` alone (CI's: an app's default build), or one run with
//!   `--h264-only`, for **H.264** (openh264 on the native side): the
//!   browser's VP8-first offer answered in H.264. It runs where the browser
//!   can encode H.264. A browser that cannot (Firefox without Cisco's plugin)
//!   is named in the log and that half is not run: H.264 needs the plugin
//!   there, by design.
//!
//! The URLs come in at build time. Without `RINCH_RTC_MEDIA_URL` the test
//! logs that it skipped, unless `RINCH_RTC_REQUIRE_LIVE=1` (CI), when it
//! fails.
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
const H264_URL: Option<&str> = option_env!("RINCH_RTC_MEDIA_H264_URL");
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

/// Send the box to the echo at `url`, wait for it to come back.
/// The encoding name of an SDP's first video payload type (`VP8`, `H264`).
fn first_video_codec(sdp: &str) -> String {
    let Some(pt) = sdp
        .lines()
        .find(|l| l.starts_with("m=video"))
        .and_then(|l| l.split_whitespace().nth(3))
    else {
        return "no video".into();
    };
    let tag = format!("a=rtpmap:{pt} ");
    sdp.lines()
        .find_map(|l| l.strip_prefix(&tag))
        .and_then(|r| r.split('/').next())
        .unwrap_or("unknown")
        .to_string()
}

async fn round_trip(url: &str, label: &str, want: &str) {
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
    let offer = peer.create_offer().await.unwrap();
    let answer = signal(url, &offer.sdp).await;
    // The codec both sides send is the answer's first video payload type.
    let codec = first_video_codec(&answer);
    web_sys::console::log_1(&format!("web_media_native {label}: video in {codec}").into());
    assert_eq!(codec, want, "{label}: the answer's first video codec");
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
    until(&format!("{label}: echoed frames"), 30, || {
        frames.lock().unwrap().len() >= 60
    })
    .await;
    let seen = frames.lock().unwrap().clone();
    let boxes = seen.iter().filter(|b| **b).count();
    assert!(
        boxes * 10 >= seen.len() * 7,
        "{label}: {boxes} of {} echoed frames show the box",
        seen.len()
    );
    assert!(errors.borrow().is_empty(), "{label}: {:?}", errors.borrow());
    web_sys::window().unwrap().clear_interval_with_handle(timer);
    peer.close();
}

#[wasm_bindgen_test]
async fn video_round_trips_through_a_native_peer() {
    let Some(url) = URL else {
        assert!(
            REQUIRE.is_none(),
            "RINCH_RTC_REQUIRE_LIVE is set but RINCH_RTC_MEDIA_URL is not: the browser-native \
             media test would not run"
        );
        web_sys::console::log_1(&"web_media_native: skipped (no RINCH_RTC_MEDIA_URL)".into());
        return;
    };
    round_trip(url, "VP8", "VP8").await;
    let Some(h264_url) = H264_URL else {
        return;
    };
    let ua = web_sys::window()
        .unwrap()
        .navigator()
        .user_agent()
        .unwrap_or_default();
    if browser_h264_frames().await > 0.0 {
        round_trip(h264_url, "H.264", "H264").await;
    } else {
        web_sys::console::log_1(
            &format!(
                "web_media_native: H.264 half not run: this browser encodes no H.264 ({ua}); \
                 in Firefox that is Cisco's OpenH264 plugin missing"
            )
            .into(),
        );
    }
}
