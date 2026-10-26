//! The bitrate cap holds on content the encoder cannot compress (#1947's
//! review, adopted): full-frame noise ran at 17 Mbps under a 300 kbps cap,
//! and its pictures (140+ packets each) never decoded. Measured as what the
//! encoder really produced, for VP8 and for H.264.
#![cfg(all(not(target_arch = "wasm32"), feature = "media"))]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use common::init;
use common::media::{Pair, noise};
use rinch_rtc::media::{LocalTrack, VideoSourceOptions};
use rinch_rtc::testing;

const CAP: u32 = 300_000;

fn noisy_call(codec: &str) {
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
    sender.set_max_bitrate(Some(CAP));
    pair.connect(1);
    let decoded = Arc::new(AtomicUsize::new(0));
    let d = decoded.clone();
    pair.remotes.borrow()[0].on_frame(move |_, fw, fh| {
        if (fw, fh) == (w, h) {
            d.fetch_add(1, Ordering::Relaxed);
        }
    });
    pair.wait(2000);
    let (b0, d0) = (
        testing::encoded_bytes(&video),
        decoded.load(Ordering::Relaxed),
    );
    pair.wait(4000);
    let bps = (testing::encoded_bytes(&video) - b0) * 8 / 4;
    let frames = decoded.load(Ordering::Relaxed) - d0;
    assert!(
        bps <= u64::from(CAP) * 3 / 2,
        "{codec}: {bps} bps under a {CAP} bps cap"
    );
    assert!(
        frames >= 4,
        "{codec}: {frames} noisy pictures decoded in 4 s"
    );
    run.store(false, Ordering::Relaxed);
    pair.close();
}

#[test]
fn the_cap_holds_on_noise() {
    init();
    noisy_call("VP8");
    testing::prefer_h264(true);
    noisy_call("H.264");
    testing::prefer_h264(false);
}
