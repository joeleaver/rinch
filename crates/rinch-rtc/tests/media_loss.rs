//! Loss on the receiving side (#1947's review, adopted): packets dropped as
//! they arrive (`testing::drop_incoming_*`), synthetic sources only.
//!
//! - Audio: one Opus packet in 25 lost is concealed (the next packet's FEC,
//!   Opus PLC), not spliced: no more clicks than with no loss, and no gaps.
//! - Video: after a 300 ms outage the picture comes back within 1.5 s,
//!   five times at different points of the stream (the receiver asks for a
//!   keyframe; the sender answers it). It used to freeze for good.
#![cfg(all(not(target_arch = "wasm32"), feature = "media"))]

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use common::init;
use common::media::{Pair, defects, is_box, moving_box, tone};
use rinch_rtc::media::{AudioSourceOptions, LocalTrack, MediaKind, VideoSourceOptions};
use rinch_rtc::testing;

const W: u32 = 320;
const H: u32 = 240;

#[test]
fn losses_are_concealed_and_recovered() {
    init();
    let mixed = testing::capture_audio_output();
    let run = Arc::new(AtomicBool::new(true));
    let (audio, aw) = LocalTrack::audio_source(AudioSourceOptions {
        sample_rate: 48_000,
        channels: 1,
    })
    .unwrap();
    tone(aw, 48_000, run.clone());
    let (video, vw) = LocalTrack::video_source(VideoSourceOptions {
        width: W,
        height: H,
        fps: 30,
    })
    .unwrap();
    moving_box(vw, W, H, 64, run.clone());
    let pair = Pair::new();
    pair.a.add_track(&audio).unwrap();
    pair.a.add_track(&video).unwrap();
    pair.connect(2);
    let rv = pair
        .remotes
        .borrow()
        .iter()
        .find(|t| t.kind() == MediaKind::Video)
        .cloned()
        .unwrap();
    let frames: Arc<Mutex<Vec<(Instant, bool)>>> = Arc::default();
    let f = frames.clone();
    rv.on_frame(move |rgba, w, h| {
        f.lock()
            .unwrap()
            .push((Instant::now(), is_box(rgba, w, h, 64)))
    });
    pair.wait(2000);

    // ---- audio
    let left_since = |mark: usize| -> Vec<f32> {
        let m = mixed.lock().unwrap();
        m[mark..].iter().step_by(2).copied().collect()
    };
    let mark = mixed.lock().unwrap().len();
    pair.wait(4000);
    let (clicks0, gaps0) = defects(&left_since(mark));
    testing::drop_incoming_audio_every(25);
    let mark = mixed.lock().unwrap().len();
    pair.wait(4000);
    testing::drop_incoming_audio_every(0);
    let lossy = left_since(mark);
    let (clicks, gaps) = defects(&lossy);
    assert!(
        lossy.len() > 48_000 * 3,
        "audio kept flowing: {} samples",
        lossy.len()
    );
    assert_eq!(gaps0, 0, "the control has no gaps");
    assert_eq!(gaps, 0, "1 in 25 lost: no silent gaps");
    assert!(
        clicks <= clicks0 + 2,
        "1 in 25 lost: {clicks} clicks (control {clicks0})"
    );

    // ---- video
    for (i, extra) in [0u64, 450, 900, 1350, 1800].iter().enumerate() {
        pair.wait(1500 + extra);
        testing::drop_incoming_video(true);
        pair.wait(300);
        testing::drop_incoming_video(false);
        let resumed = Instant::now();
        frames.lock().unwrap().clear();
        pair.until("a box after the outage", || {
            frames.lock().unwrap().iter().any(|f| f.1)
        });
        let back = frames
            .lock()
            .unwrap()
            .iter()
            .find(|f| f.1)
            .map(|f| f.0.duration_since(resumed).as_millis())
            .unwrap();
        assert!(
            back < 1500,
            "outage {i}: the picture came back after {back} ms"
        );
    }
    run.store(false, Ordering::Relaxed);
    pair.close();
}
