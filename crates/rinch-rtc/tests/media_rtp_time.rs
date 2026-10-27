//! RTP timestamps follow time (#1947's third review, adopted): a frame's
//! timestamp advances by the time since the previous frame, so a still
//! source's half-second repeats and the frame after a pause carry their
//! gap, not the one before them (they were one frame late). Synthetic
//! sources only.
#![cfg(all(not(target_arch = "wasm32"), feature = "media"))]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use common::init;
use common::media::Pair;
use rinch_rtc::media::{LocalTrack, VideoSourceOptions};
use rinch_rtc::testing;

#[test]
fn timestamps_carry_the_gap_before_each_frame() {
    init();
    let (w, h) = (320u32, 240u32);
    let run = Arc::new(AtomicBool::new(true));
    let (video, vw) = LocalTrack::video_source(VideoSourceOptions {
        width: w,
        height: h,
        fps: 30,
    })
    .unwrap();
    // 30 fps, then nothing new from 3 s to 4.5 s (the pump repeats the
    // still picture twice a second), then 30 fps again.
    let r = run.clone();
    std::thread::spawn(move || {
        let start = Instant::now();
        let mut k = 0u32;
        while r.load(Ordering::Relaxed) && !vw.is_ended() {
            let t = start.elapsed();
            if t >= Duration::from_millis(3000) && t < Duration::from_millis(4500) {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            let x = (k * 4) % (w - 64);
            let mut f = vec![0u8; (w * h * 4) as usize];
            for px in f.as_chunks_mut::<4>().0 {
                px[3] = 255;
            }
            for row in 40..104 {
                for col in x..x + 64 {
                    let i = ((row * w + col) * 4) as usize;
                    f[i..i + 3].copy_from_slice(&[255, 255, 255]);
                }
            }
            vw.push_rgba(&f, w, h);
            k += 1;
            std::thread::sleep(Duration::from_millis(33));
        }
    });
    let pair = Pair::new();
    pair.a.add_track(&video).unwrap();
    pair.connect(1);
    let rv = pair.remotes.borrow()[0].clone();
    pair.wait(6000);
    let times = testing::frame_rtp_times(&rv);
    run.store(false, Ordering::Relaxed);
    pair.close();
    pair.wait(300);

    let mut long = 0;
    for p in times.windows(2) {
        let wall = p[1].0.duration_since(p[0].0).as_secs_f64() * 1000.0;
        let rtp = f64::from(p[1].1.wrapping_sub(p[0].1)) / 90.0;
        if wall > 300.0 {
            // A repeat or the first frame after the pause: its own gap.
            long += 1;
            assert!(
                rtp > wall * 0.6,
                "a frame {wall:.0} ms after the last carries {rtp:.0} ms"
            );
        } else if wall < 50.0 {
            // A frame right after one of those must not carry its gap.
            assert!(
                rtp < 150.0,
                "a frame {wall:.0} ms after the last carries {rtp:.0} ms"
            );
        }
    }
    assert!(long >= 2, "the pause showed as {long} long gaps: {times:?}");
}
