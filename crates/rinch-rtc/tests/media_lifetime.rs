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
//! a child with `MALLOC_ARENA_MAX=1`.
//!
//! RSS still moved in allocator steps, over 3 MB in one ten-call window
//! with no leak (#2009). So the test measures the heap in use instead,
//! twice: what Rust code holds (a counting `#[global_allocator]`) and what
//! glibc's malloc has handed out (`mallinfo2`, which also sees the C
//! codecs' memory). Neither counts memory kept after a free, and both are
//! read after the call's threads have exited.
#![cfg(all(not(target_arch = "wasm32"), feature = "media"))]

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use common::media::{Pair, moving_box, tone};
use common::{drain_pump, init, pump_until};
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

/// For the log only: RSS moves in allocator steps (#2009).
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

/// The bytes Rust code holds right now: every allocation through the
/// global allocator, less every deallocation.
static LIVE: AtomicUsize = AtomicUsize::new(0);

struct Counting;

// SAFETY: forwards every call to `System` unchanged; only counts.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            LIVE.fetch_add(new_size, Ordering::Relaxed);
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        p
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// The malloc heap in use, C code's included (the codecs': openh264, opus,
/// libvpx), which `LIVE` cannot see. glibc's own count of allocated chunks,
/// not RSS: memory the allocator keeps after a free is not in it.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn malloc_in_use() -> usize {
    // SAFETY: no arguments; reads glibc's counters.
    let m = unsafe { libc::mallinfo2() };
    m.uordblks + m.hblkhd
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn malloc_in_use() -> usize {
    0
}

fn named(prefix: &str) -> Vec<String> {
    threads()
        .into_iter()
        .filter(|t| t.starts_with(prefix))
        .collect()
}

fn rtc_threads() -> Vec<String> {
    named("rinch-rtc-")
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
    // The heap (Rust's, and malloc's) after calls 10, 20 and 30.
    let mut heap: Vec<(usize, usize)> = Vec::new();
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
        // Every thread the call started exits, rinch-rtc's and the test's
        // own sources (which hold a frame while they run); under load that
        // can take well over the 300 ms this used to wait (#2009).
        let _ = pump_until(
            || rtc_threads().is_empty() && named("synth-").is_empty(),
            Duration::from_secs(10),
        );
        let left = rtc_threads();
        assert!(left.is_empty(), "call {call}: threads left {left:?}");
        let sources = named("synth-");
        assert!(sources.is_empty(), "call {call}: sources left {sources:?}");
        // What they posted to the main thread on the way out is still
        // queued: deliver it before the heap is counted.
        drain_pump();
        // The test's own capture of the mixed audio grows 384 kB a second
        // of call (48 kHz stereo f32): not the calls' memory.
        {
            let mut m = mixed.lock().unwrap();
            m.clear();
            m.shrink_to_fit();
        }
        let (live, malloc) = (LIVE.load(Ordering::Relaxed), malloc_in_use());
        eprintln!(
            "after {} calls: {live} B live, {malloc} B malloc, RSS {} kB",
            call + 1,
            rss_kb()
        );
        if call % 10 == 9 {
            heap.push((live, malloc));
        }
    }
    // A leak grows every ten calls. Both counts are exact (no allocator
    // step, no freed memory kept), so each ten-call growth is checked, not
    // the smaller of two. Measured on a loaded host (#2009): Rust's heap
    // grew 0 to 128 B over ten calls, malloc's 9 to 31 kB. 1 MB is 30
    // times that; the 0.3 MB a call the old RSS bound allowed fails by 3x.
    const BOUND: i64 = 1_000_000;
    if let [a, b, c] = heap.as_slice() {
        for (name, x, y, z) in [
            ("Rust's heap", a.0, b.0, c.0),
            ("the malloc heap", a.1, b.1, c.1),
        ] {
            let (g1, g2) = (y as i64 - x as i64, z as i64 - y as i64);
            assert!(
                g1 < BOUND && g2 < BOUND,
                "{name} grew {g1} B over calls 10-20 and {g2} B over calls 20-30: a leak"
            );
        }
    }
}
