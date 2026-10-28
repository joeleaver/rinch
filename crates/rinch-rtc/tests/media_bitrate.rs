//! The bitrate cap holds on content the encoder cannot compress (#1947's
//! review, adopted): full-frame noise ran at 17 Mbps under a 300 kbps cap,
//! and its pictures (140+ packets each) never decoded. Measured as what the
//! encoder really produced, for VP8 and for H.264 (checked to be the codec
//! the call negotiated), at 300 and 100 kbps. The cap is set mid-call,
//! so it must reach an encoder that is already running (800 kbps by
//! default at 640×480).
#![cfg(all(not(target_arch = "wasm32"), feature = "media"))]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use common::init;
use common::media::{Pair, noise};
use rinch_rtc::media::{LocalTrack, VideoSourceOptions};
use rinch_rtc::testing;

fn noisy_call(codec: &str, cap: u32) {
    let (w, h) = (640u32, 480u32);
    let run = Arc::new(AtomicBool::new(true));
    let (video, vw) = LocalTrack::video_source(VideoSourceOptions {
        width: w,
        height: h,
        fps: 30,
    })
    .unwrap();
    noise(vw, w, h, run.clone());
    let pair = Pair::new();
    let sender = pair.a.add_track(&video).unwrap();
    pair.connect(1);
    let decoded = Arc::new(AtomicUsize::new(0));
    let d = decoded.clone();
    pair.remotes.borrow()[0].on_frame(move |_, fw, fh| {
        if (fw, fh) == (w, h) {
            d.fetch_add(1, Ordering::Relaxed);
        }
    });
    pair.wait(1500);
    assert!(
        testing::encoded_bytes(&video) > 0,
        "{codec}: encoding before the cap"
    );
    assert_eq!(
        testing::sending_codecs(&video),
        [codec],
        "the call negotiated the codec under test"
    );
    sender.set_max_bitrate(Some(cap));
    let d_cap = decoded.load(Ordering::Relaxed);
    // What the encoder saved up before the cap is spent first.
    pair.wait(2500);
    // The cap reached the running encoder (without it, the send budget
    // alone would hold the rate by skipping frames the encoder made for
    // 800 kbps).
    assert_eq!(
        testing::applied_bitrate(&video),
        cap,
        "{codec}: the encoder's bitrate"
    );
    testing::take_largest_frame(&video);
    let b0 = testing::encoded_bytes(&video);
    // A keyframe asked for every second, as a lossy link's receiver does:
    // a hard keyframe (openh264's is several times a second's budget) must
    // not be sent at that rate.
    for _ in 0..6 {
        testing::request_keyframe(&video);
        pair.wait(1000);
    }
    let bits = (testing::encoded_bytes(&video) - b0) * 8;
    let largest = testing::take_largest_frame(&video) * 8;
    // A sender may run ahead of its budget by half a second's worth and
    // one frame, keyframes asked for or not. A keyframe of full-frame noise is several times a
    // second's budget in H.264 (openh264 cannot make it smaller); the
    // frames after it are skipped to pay it back.
    let allowed = u64::from(cap) * 6 * 11 / 10 + u64::from(cap) / 2 + largest;
    assert!(
        bits <= allowed,
        "{codec}: {} bps over 6 s under a {cap} bps cap (allowed {}; largest frame {} B)",
        bits / 6,
        allowed / 6,
        largest / 8
    );
    // The pictures decode. At 100 kbps a keyframe of noise is three (VP8,
    // 36 KB) to seven (H.264, 87 KB) seconds of budget, and keyframes are
    // asked for every second: one may not land in this window.
    if cap >= 300_000 {
        assert!(
            decoded.load(Ordering::Relaxed) > d_cap,
            "{codec}: no noisy picture decoded in 8.5 s under {cap} bps"
        );
    }
    run.store(false, Ordering::Relaxed);
    pair.close();
}

#[test]
fn the_cap_holds_on_noise() {
    init();
    for cap in [300_000, 100_000] {
        #[cfg(feature = "media-vp8")]
        noisy_call("VP8", cap);
        // Read as each peer's connection is built: on for the whole call.
        testing::prefer_h264(true);
        noisy_call("H264", cap);
        testing::prefer_h264(false);
    }
}
