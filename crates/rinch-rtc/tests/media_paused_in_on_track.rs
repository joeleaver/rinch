//! A remote audio track paused in `on_track` plays nothing, however late
//! the main thread runs that callback (#2649).
//!
//! The reader thread starts decoding a remote track as soon as it arrives,
//! and `on_track` reaches the app one main-thread hop later. A track starts
//! playing, so audio that arrived in between used to be mixed at full
//! volume until the app's `set_playing(false)` ran: ui-zoo's in-page call
//! (`examples/ui-zoo/tests/webrtc_in_page.rs`) heard a burst of A's tone
//! at peak 0.187 once in CI. The window is opened on purpose here: B's
//! `on_track` holds the main thread until the tone has reached the mixer.
//!
//! The second call is the positive control: a track left playing is heard
//! after the same stall, so the first call's silence is not a dead mixer.
#![cfg(all(not(target_arch = "wasm32"), feature = "media"))]

mod common;

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::init;
use common::media::{Pair, tone};
use rinch_rtc::media::{AudioSourceOptions, LocalTrack, MediaKind};

fn peak(mixed: &Mutex<Vec<f32>>) -> f32 {
    mixed
        .lock()
        .unwrap()
        .iter()
        .fold(0f32, |acc, s| acc.max(s.abs()))
}

/// A call in which B's `on_track` runs late: it first blocks the main
/// thread until A's tone is reaching B's mixer, and 300 ms more, as a busy
/// main thread would, and only then sets `playing` on the audio. Returns
/// the mixed peak.
fn call(mixed: &Arc<Mutex<Vec<f32>>>, playing: bool) -> f32 {
    let pair = Pair::new();
    let stalled = Rc::new(Cell::new(false));
    let s = stalled.clone();
    pair.b.on_track(move |t| {
        if t.kind() != MediaKind::Audio {
            return;
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while rinch_rtc::testing::mixer_depth_ms() == 0.0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        s.set(rinch_rtc::testing::mixer_depth_ms() > 0.0);
        std::thread::sleep(Duration::from_millis(300));
        t.set_playing(playing);
    });
    let (audio, aw) = LocalTrack::audio_source(AudioSourceOptions {
        sample_rate: 48_000,
        channels: 1,
    })
    .unwrap();
    let run = Arc::new(AtomicBool::new(true));
    tone(aw.clone(), 48_000, run.clone());
    pair.a.add_track(&audio).unwrap();
    pair.negotiate();
    pair.until("B's on_track", || stalled.get());
    pair.wait(500);

    run.store(false, Ordering::Relaxed);
    pair.close();
    pair.wait(100);
    let p = peak(mixed);
    mixed.lock().unwrap().clear();
    p
}

#[test]
fn audio_paused_in_on_track_plays_nothing_after_a_main_thread_stall() {
    init();
    let mixed = rinch_rtc::testing::capture_audio_output();

    let paused = call(&mixed, false);
    let played = call(&mixed, true);
    assert!(
        played > 0.1,
        "positive control: a playing track is heard ({played})"
    );
    assert!(
        paused < 1e-3,
        "B played audio before its on_track paused it: peak {paused}"
    );
}
