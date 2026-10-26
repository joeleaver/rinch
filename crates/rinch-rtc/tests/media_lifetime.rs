//! 30 calls, each closed mid-call with tracks, views and callbacks dropped
//! (#1947's review, adopted). Each must leave no thread behind, and memory
//! must stay bounded.
//!
//! **Memory.** #1947's review measured about 1 MB of RSS growth a call
//! (0.33 MB for a data-only call). It was this test's own capture of the
//! mixed audio: `testing::capture_audio_output` keeps every sample, 384 kB
//! per second of call (massif over 24 data-only calls put 83% of the heap
//! there). With the capture cleared after each call, RSS over 60 calls is
//! flat with `MALLOC_ARENA_MAX=1` (31.2 MB after 10 calls and after 60)
//! and levels off at about 62 MB with glibc's default arenas (one per
//! thread, up to 8 per core, keeping freed memory). The test runs itself as
//! a child with `MALLOC_ARENA_MAX=1`, where growth would be a leak.
#![cfg(all(not(target_arch = "wasm32"), feature = "media"))]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use common::media::{Pair, moving_box, tone};
use common::{init, pump_until};
use rinch_rtc::media::{AudioSourceOptions, LocalTrack, MediaKind, VideoSourceOptions};

const CHILD: &str = "RINCH_RTC_LIFETIME_CHILD";

fn threads() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir("/proc/self/task")
        .map(|d| {
            d.flatten()
                .filter_map(|e| std::fs::read_to_string(e.path().join("comm")).ok())
                .map(|s| s.trim().to_string())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(0)
}

#[test]
fn thirty_calls_leave_nothing_behind() {
    if std::env::var(CHILD).is_err() {
        if !cfg!(target_os = "linux") {
            return; // /proc and glibc's arenas are Linux's
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "thirty_calls_leave_nothing_behind",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("MALLOC_ARENA_MAX", "1")
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stderr).into_owned()
            + &String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "child failed:\n{text}");
        assert!(text.contains("1 passed"), "the child ran the test:\n{text}");
        return;
    }
    init();
    let mixed = rinch_rtc::testing::capture_audio_output();
    let (w, h) = (320u32, 240u32);
    let mut rss_at_10 = 0;
    let calls: usize = std::env::var("RINCH_RTC_LIFETIME_CALLS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    for call in 0..calls {
        let run = Arc::new(AtomicBool::new(true));
        let (video, vw) = LocalTrack::video_source(VideoSourceOptions {
            width: w,
            height: h,
            fps: 30,
        })
        .unwrap();
        moving_box(vw, w, h, 64, run.clone());
        let (audio, aw) = LocalTrack::audio_source(AudioSourceOptions::default()).unwrap();
        tone(aw, 48_000, run.clone());
        let (back, bw) = LocalTrack::video_source(VideoSourceOptions {
            width: w,
            height: h,
            fps: 30,
        })
        .unwrap();
        bw.push_rgba(&vec![128u8; (w * h * 4) as usize], w, h);
        let pair = Pair::new();
        pair.a.add_track(&video).unwrap();
        pair.a.add_track(&audio).unwrap();
        pair.b.add_track(&back).unwrap();
        pair.connect(2);
        let rv = pair
            .remotes
            .borrow()
            .iter()
            .find(|t| t.kind() == MediaKind::Video)
            .cloned()
            .unwrap();
        let n = Arc::new(AtomicUsize::new(0));
        let n2 = n.clone();
        let held = Arc::new(vec![1u8; 1 << 20]);
        rv.on_frame(move |_, _, _| {
            let _ = &held;
            n2.fetch_add(1, Ordering::Relaxed);
        });
        pair.until("frames", || n.load(Ordering::Relaxed) >= 5);
        #[allow(clippy::drop_non_drop)] // the handle going away is the point
        drop(rv);
        pair.a.close();
        pair.wait(100);
        pair.remotes.borrow_mut().clear();
        pair.b.close();
        drop((video, audio, back, pair));
        run.store(false, Ordering::Relaxed);
        let _ = pump_until(|| false, Duration::from_millis(300));
        // The test's own capture of the mixed audio grows 384 kB a second
        // of call (48 kHz stereo f32): not the calls' memory.
        {
            let mut m = mixed.lock().unwrap();
            m.clear();
            m.shrink_to_fit();
        }
        let left: Vec<String> = threads()
            .into_iter()
            .filter(|t| t.starts_with("rinch-rtc-"))
            .collect();
        assert!(left.is_empty(), "call {call}: threads left {left:?}");
        if call == 10 {
            rss_at_10 = rss_kb();
        }
        if call % 10 == 9 {
            eprintln!("RSS after {} calls: {} kB", call + 1, rss_kb());
        }
    }
    let grown = rss_kb().saturating_sub(rss_at_10);
    eprintln!("RSS grew {grown} kB over calls 10-29");
    assert!(
        grown < 10_000,
        "RSS grew {grown} kB over 20 calls (a leak of 0.5 MB a call)"
    );
}
