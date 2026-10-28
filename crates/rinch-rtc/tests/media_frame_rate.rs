//! Frame rate on camera-like content through caps and their removal
//! (#1947's third review, adopted). A box on black, which the other tests
//! send, costs almost nothing to encode and hid this: VP8 on a busy
//! picture fell to 4 fps at 300 kbps and stayed near 10 fps after the cap
//! was lifted (libvpx was given the wall clock as pts, read the frames the
//! send budget skipped as a falling frame rate, and made each frame
//! larger). Synthetic sources only.
#![cfg(all(not(target_arch = "wasm32"), feature = "media"))]

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::init;
use common::media::Pair;
use rinch_rtc::media::{LocalTrack, VideoSourceOptions};
use rinch_rtc::testing;

const W: u32 = 640;
const H: u32 = 480;

/// A camera-like loop: a gradient, a textured square moving across it, a
/// soft blob drifting, and sensor noise of ±3.
fn camera_frames(w: usize, h: usize, n: usize) -> Vec<Vec<u8>> {
    let mut seed = 99u32;
    (0..n)
        .map(|k| {
            let mut f = vec![0u8; w * h * 4];
            for (i, px) in f.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let (x, y) = (i % w, i / w);
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let nz = ((seed >> 24) as i32 % 7) - 3;
                let sq = (x + 100_000 - k * 4) % (w / 2) < w / 6
                    && (y + 100_000 - k * 2) % (h / 2) < h / 6;
                let cx = (w / 2) as f32 + (k as f32 * 0.1).sin() * (w / 4) as f32;
                let d = ((x as f32 - cx).powi(2) + (y as f32 - (h / 2) as f32).powi(2)).sqrt();
                let blob = (60.0 - d * 0.5).max(0.0) as i32;
                let t = if sq {
                    (((x ^ y) & 8) * 20) as i32 + 40
                } else {
                    ((x * 160 / w + y * 60 / h) / 2) as i32 + 30 + blob
                };
                let v = (t + nz).clamp(0, 255);
                *px = [
                    v as u8,
                    (v * 3 / 4 + 30).clamp(0, 255) as u8,
                    (200 - v / 2) as u8,
                    255,
                ];
            }
            f
        })
        .collect()
}

fn call(codec: &str) {
    let frames = camera_frames(W as usize, H as usize, 90);
    let run = Arc::new(AtomicBool::new(true));
    let (video, vw) = LocalTrack::video_source(VideoSourceOptions {
        width: W,
        height: H,
        fps: 30,
    })
    .unwrap();
    {
        let run = run.clone();
        std::thread::spawn(move || {
            let start = Instant::now();
            let mut k = 0u32;
            while run.load(Ordering::Relaxed) && !vw.is_ended() {
                vw.push_rgba(&frames[k as usize % frames.len()], W, H);
                k += 1;
                let due = start + Duration::from_millis(33) * k;
                if let Some(d) = due.checked_duration_since(Instant::now()) {
                    std::thread::sleep(d);
                }
            }
        });
    }
    let pair = Pair::new();
    let sender = pair.a.add_track(&video).unwrap();
    pair.connect(1);
    assert_eq!(testing::sending_codecs(&video), [codec]);
    let shown: Arc<Mutex<u32>> = Arc::default();
    let s = shown.clone();
    pair.remotes.borrow()[0].on_frame(move |_, fw, fh| {
        if (fw, fh) == (W, H) {
            *s.lock().unwrap() += 1;
        }
    });
    pair.wait(2000);
    // Each step: the cap, a second to settle, then the rate over 2 s.
    for cap in [None, Some(300_000), Some(1_000_000), None] {
        sender.set_max_bitrate(cap);
        pair.wait(1000);
        *shown.lock().unwrap() = 0;
        pair.wait(2000);
        let fps = *shown.lock().unwrap() as f64 / 2.0;
        eprintln!("{codec} cap {cap:?}: {fps:.1} fps");
        assert!(
            fps >= 20.0,
            "{codec} under {cap:?}: {fps:.1} fps from a 30 fps source"
        );
    }
    run.store(false, Ordering::Relaxed);
    pair.close();
    pair.wait(300);
}

#[test]
fn camera_like_content_keeps_its_frame_rate() {
    init();
    #[cfg(feature = "media-vp8")]
    call("VP8");
    // Read as each peer's connection is built: on for the whole call.
    testing::prefer_h264(true);
    call("H264");
    testing::prefer_h264(false);
}
