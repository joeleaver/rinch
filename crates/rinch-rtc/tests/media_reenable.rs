//! A still app-fed source (one frame, never pushed again), disabled and
//! enabled again, shows its picture again (#1947's review, adopted): the
//! pump used to paint black over its own copy.
#![cfg(all(not(target_arch = "wasm32"), feature = "media"))]

mod common;

use std::sync::{Arc, Mutex};

use common::init;
use common::media::Pair;
use rinch_rtc::media::{LocalTrack, VideoSourceOptions};

#[test]
fn a_still_source_comes_back_after_enable() {
    init();
    let (w, h) = (320u32, 240u32);
    let (video, vw) = LocalTrack::video_source(VideoSourceOptions {
        width: w,
        height: h,
        fps: 30,
    })
    .unwrap();
    vw.push_rgba(&vec![200u8; (w * h * 4) as usize], w, h);
    let pair = Pair::new();
    pair.a.add_track(&video).unwrap();
    pair.connect(1);
    let rv = pair.remotes.borrow()[0].clone();
    let lumas: Arc<Mutex<Vec<u8>>> = Arc::default();
    let l = lumas.clone();
    rv.on_frame(move |rgba, _, _| l.lock().unwrap().push(rgba[(rgba.len() / 2) & !3]));
    let last = |ms: u64| {
        lumas.lock().unwrap().clear();
        pair.wait(ms);
        lumas.lock().unwrap().last().copied()
    };
    assert!(last(2500).unwrap_or(0) > 150, "grey before");
    video.set_enabled(false);
    assert!(last(2500).unwrap_or(255) < 20, "black while disabled");
    video.set_enabled(true);
    assert!(last(2500).unwrap_or(0) > 150, "grey again once enabled");
    pair.close();
}
