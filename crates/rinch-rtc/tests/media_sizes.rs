//! Odd and tiny picture sizes (#1947's second review, adopted). An odd
//! width or height loses its last column or row and nothing else: the
//! picture is not sheared (each row copied from its own place in the
//! source). A picture under 16 pixels a side, which openh264 refuses, is
//! scaled up and still sent. Both codecs.
#![cfg(all(not(target_arch = "wasm32"), feature = "media"))]

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::init;
use common::media::Pair;
use rinch_rtc::media::{LocalTrack, VideoSourceOptions};
use rinch_rtc::testing;

/// Grey, with a white vertical bar 16 wide at `bx` and a white horizontal
/// bar 8 high at `by`.
fn pic(w: u32, h: u32, bx: u32, by: u32) -> Vec<u8> {
    let mut f = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let on = (bx..bx + 16).contains(&x) || (by..by + 8).contains(&y);
            let v = if on { 250 } else { 60 };
            f[i..i + 4].copy_from_slice(&[v, v, v, 255]);
        }
    }
    f
}

/// The vertical bar's centre in row `y`, or None when the row has no bar.
fn bar_at(rgba: &[u8], w: u32, y: u32) -> Option<f32> {
    let (mut sum, mut n) = (0f32, 0f32);
    for x in 0..w {
        if rgba[((y * w + x) * 4) as usize] > 160 {
            sum += x as f32;
            n += 1.0;
        }
    }
    (n > 0.0 && n < w as f32 * 0.5).then(|| sum / n)
}

type Last = Arc<Mutex<Option<(u32, u32, Option<f32>, Option<f32>)>>>;

fn sizes(codec: &str) {
    let (video, vw) = LocalTrack::video_source(VideoSourceOptions {
        width: 320,
        height: 240,
        fps: 30,
    })
    .unwrap();
    vw.push_rgba(&pic(320, 240, 96, 200), 320, 240);
    let pair = Pair::new();
    pair.a.add_track(&video).unwrap();
    pair.connect(1);
    assert_eq!(testing::sending_codecs(&video), [codec]);
    let last: Last = Arc::default();
    let l = last.clone();
    pair.remotes.borrow()[0].on_frame(move |rgba, w, h| {
        let rows = (w > 4 && h > 4).then(|| (bar_at(rgba, w, 1), bar_at(rgba, w, h * 4 / 10)));
        let (top, mid) = rows.unwrap_or((None, None));
        *l.lock().unwrap() = Some((w, h, top, mid));
    });
    let run = Arc::new(AtomicBool::new(true));
    for (w, h) in [
        (641u32, 481u32),
        (333, 201),
        (1, 1),
        (3, 3),
        (15, 15),
        (320, 240),
    ] {
        let bx = w * 3 / 10;
        let f = pic(w, h, bx, h * 8 / 10);
        let (vw2, r2) = (vw.clone(), run.clone());
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let feeder = std::thread::spawn(move || {
            while !s2.load(Ordering::Relaxed) && r2.load(Ordering::Relaxed) {
                vw2.push_rgba(&f, w, h);
                std::thread::sleep(Duration::from_millis(33));
            }
        });
        *last.lock().unwrap() = None;
        pair.until(&format!("{codec} {w}x{h}: a frame of the new size"), || {
            last.lock()
                .unwrap()
                .is_some_and(|(fw, fh, ..)| w < 16 || (fw.abs_diff(w) <= 1 && fh.abs_diff(h) <= 1))
        });
        pair.wait(800);
        stop.store(true, Ordering::Relaxed);
        feeder.join().unwrap();
        let (fw, fh, top, mid) = last.lock().unwrap().unwrap();
        if w < 16 {
            assert!(
                fw >= 16 && fh >= 16,
                "{codec} {w}x{h}: arrived as {fw}x{fh}"
            );
            continue;
        }
        assert_eq!(
            (fw, fh),
            (w & !1, h & !1),
            "{codec}: {w}x{h} arrived as {fw}x{fh}"
        );
        let (top, mid) = (top.unwrap(), mid.unwrap());
        let want = bx as f32 + 7.5;
        assert!(
            (top - want).abs() < 2.0 && (mid - want).abs() < 2.0,
            "{codec} {w}x{h}: the bar at x {top:.1} (top) and {mid:.1} (40% down), drawn at {want}"
        );
    }
    run.store(false, Ordering::Relaxed);
    pair.close();
    pair.wait(300);
}

#[test]
fn odd_and_tiny_sizes() {
    init();
    #[cfg(feature = "media-vp8")]
    sizes("VP8");
    // Read as each peer's connection is built: on for the whole call.
    testing::prefer_h264(true);
    sizes("H264");
    testing::prefer_h264(false);
}
