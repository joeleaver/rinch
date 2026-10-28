//! The WebRTC section's in-page call, without a window (#1916, design record
//! `docs/design/rtc-media.md` 5.2): the section's own `InPageCall` makes two
//! rinch-rtc peers in this process, and what each sends arrives at the other.
//!
//! Synthetic sources only, as the section uses by default; `use_camera` is
//! never called. Remote audio is read through rinch-rtc's mixer hook
//! (`testing::capture_audio_output`), so no audio device is opened either.
//!
//! As in rinch-rtc's own tests, the test thread stands in for the rinch
//! runtime: it registers as the main thread and pumps the main-thread queue
//! (timers, tasks and peer callbacks all arrive there). Registration is
//! process-global, so this file has one `#[test]`.
#![cfg(not(target_arch = "wasm32"))]

use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use rinch_core::{register_main_thread, set_cross_thread_dispatcher};
use rinch_rtc::PeerState;
use rinch_rtc::media::{MediaTrack, RemoteTrack};
use ui_zoo::sections::webrtc_call::{BOX, H, InPageCall, W};

type MainJob = Box<dyn FnOnce() + Send>;
static PUMP: OnceLock<Mutex<Vec<MainJob>>> = OnceLock::new();

fn dispatcher(f: MainJob) {
    PUMP.get().expect("pump").lock().unwrap().push(f);
}

fn drain() {
    loop {
        let jobs: Vec<_> = PUMP.get().unwrap().lock().unwrap().drain(..).collect();
        if jobs.is_empty() {
            return;
        }
        for job in jobs {
            job();
        }
    }
}

fn pump_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        drain();
        if cond() {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(2));
    }
}

fn pump_for(ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        drain();
        thread::sleep(Duration::from_millis(2));
    }
}

/// The bright region of a frame: its pixel count and bounding box size.
fn bright_box(rgba: &[u8], w: u32, h: u32) -> (u32, u32, u32) {
    let (mut x0, mut y0, mut x1, mut y1, mut n) = (u32::MAX, u32::MAX, 0, 0, 0u32);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let luma = (u32::from(rgba[i]) + u32::from(rgba[i + 1]) + u32::from(rgba[i + 2])) / 3;
            if luma > 128 {
                n += 1;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if n == 0 {
        (0, 0, 0)
    } else {
        (n, x1 - x0 + 1, y1 - y0 + 1)
    }
}

fn is_box((n, w, h): (u32, u32, u32)) -> bool {
    let near = |v: u32| v.abs_diff(BOX) <= BOX / 5 + 4;
    near(w) && near(h) && n * 10 >= BOX * BOX * 7
}

type Frames = Arc<Mutex<Vec<(u32, u32, u32)>>>;

fn record(track: &RemoteTrack) -> Frames {
    let frames: Frames = Arc::default();
    let f = frames.clone();
    track.on_frame(move |rgba, w, h| {
        if w == W && h == H {
            f.lock().unwrap().push(bright_box(rgba, w, h));
        }
    });
    frames
}

/// `want` frames that arrive after `settle` ms.
fn fresh(frames: &Frames, settle: u64, want: usize) -> Vec<(u32, u32, u32)> {
    pump_for(settle);
    frames.lock().unwrap().clear();
    pump_until("fresh frames", || frames.lock().unwrap().len() >= want);
    frames.lock().unwrap().clone()
}

fn box_share(frames: &[(u32, u32, u32)]) -> f64 {
    frames.iter().filter(|f| is_box(**f)).count() as f64 / frames.len() as f64
}

fn remote_of(shown: &[ui_zoo::sections::webrtc_call::Shown]) -> RemoteTrack {
    match &shown.first().expect("a received video").0 {
        MediaTrack::Remote(t) => t.clone(),
        other => panic!("not a remote track: {other:?}"),
    }
}

/// Goertzel power at `hz` in 48 kHz mono `samples`, over the mean at a few
/// other frequencies.
fn tone_ratio(samples: &[f32], hz: f32) -> f32 {
    let power = |f: f32| {
        let w = 2.0 * std::f32::consts::PI * f / 48_000.0;
        let c = 2.0 * w.cos();
        let (mut s1, mut s2) = (0f32, 0f32);
        for &x in samples {
            let s = x + c * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        s1 * s1 + s2 * s2 - c * s1 * s2
    };
    let others = [300.0, 600.0, 1000.0];
    let base = others.iter().map(|f| power(*f)).sum::<f32>() / others.len() as f32;
    power(hz) / base.max(1e-9)
}

#[test]
fn the_in_page_call_carries_both_patterns_and_the_tone() {
    PUMP.set(Mutex::new(Vec::new())).ok();
    register_main_thread();
    set_cross_thread_dispatcher(dispatcher);
    rinch_rtc::testing::set_bind_addrs(&["127.0.0.1:0"]);
    let mixed = rinch_rtc::testing::capture_audio_output();

    let call = InPageCall::start().expect("the call starts");
    let view = call.view();
    let (a, b) = call.peers();
    pump_until("both peers connected", || {
        a.state() == PeerState::Connected && b.state() == PeerState::Connected
    });
    assert_eq!(view.state_a.get(), PeerState::Connected);
    assert_eq!(view.state_b.get(), PeerState::Connected);

    // A's pattern reaches B, and B's reaches A.
    pump_until("B receives A's video", || !view.at_b.get().is_empty());
    pump_until("A receives B's video", || !view.at_a.get().is_empty());
    let at_b = record(&remote_of(&view.at_b.get()));
    let at_a = record(&remote_of(&view.at_a.get()));
    let seen = fresh(&at_b, 500, 30);
    assert!(box_share(&seen) >= 0.8, "A's box at B: {seen:?}");
    let seen = fresh(&at_a, 0, 30);
    assert!(box_share(&seen) >= 0.8, "B's box at A: {seen:?}");

    // The section's video-off sends black frames.
    call.set_video_off(true);
    let seen = fresh(&at_b, 1500, 3);
    assert!(
        seen.iter().all(|f| f.0 == 0),
        "black after video off: {seen:?}"
    );
    call.set_video_off(false);
    let seen = fresh(&at_b, 1500, 10);
    assert!(box_share(&seen) >= 0.7, "the box again: {seen:?}");

    // Review fixture (#1970): B plays nothing until asked (rtc-media.md 5.4),
    // although A's tone has been arriving all along.
    pump_for(1500);
    let loudest = {
        let m = mixed.lock().unwrap();
        m.iter().fold(0f32, |acc, s| acc.max(s.abs()))
    };
    assert!(
        loudest < 1e-3,
        "B played A's audio before it was asked: peak {loudest}"
    );

    // B plays A's tone once asked to.
    call.set_hear(true);
    pump_for(1500);
    let left: Vec<f32> = {
        let m = mixed.lock().unwrap();
        let tail = &m[m.len().saturating_sub(48_000 * 2)..];
        tail.iter().step_by(2).copied().collect()
    };
    let ratio = tone_ratio(&left, 440.0);
    assert!(ratio > 100.0, "440 Hz stands {ratio}x above other bins");

    // Review fixture (#1970): muting A sends silence (set_enabled), so B's
    // played audio goes quiet.
    call.set_muted(true);
    pump_for(1500);
    let quiet: Vec<f32> = {
        let m = mixed.lock().unwrap();
        let tail = &m[m.len().saturating_sub(48_000 / 2 * 2)..];
        tail.iter().step_by(2).copied().collect()
    };
    let peak = quiet.iter().fold(0f32, |acc, s| acc.max(s.abs()));
    assert!(peak < 0.01, "A is muted but B still hears it: peak {peak}");
    call.set_muted(false);

    // Nothing in the call asked for a device: A still sends the pattern.
    assert_eq!(call.sending().map(|t| t.id()), {
        match &view.local.get()[0].0 {
            MediaTrack::Local(t) => Some(t.id()),
            _ => None,
        }
    });
    assert!(view.error.get().is_empty(), "error: {}", view.error.get());
    call.close();
}
