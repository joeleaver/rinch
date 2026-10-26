//! A long call stays responsive (#1947's review, adopted): video latency
//! (a level change pushed at the sender, seen at the far side) and the
//! remote-audio buffer stay bounded. 20 s by default;
//! `RINCH_RTC_LONGRUN_SECS=300` for the review's five minutes. The mixer
//! used to drift to its 300 ms cap and stay there.
#![cfg(all(not(target_arch = "wasm32"), feature = "media"))]
mod common;
use common::{T, drive, init, pump_until};
use rinch_rtc::media::{
    AudioSourceOptions, LocalTrack, MediaKind, RemoteTrack, VideoSourceOptions,
};
use rinch_rtc::{IceCandidate, Peer, PeerState, RtcConfig};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Inbox = Rc<RefCell<Vec<Option<IceCandidate>>>>;
fn trickle(p: &Peer) -> Inbox {
    let i: Inbox = Rc::default();
    let j = i.clone();
    p.on_ice_candidate(move |c| j.borrow_mut().push(c));
    i
}
const W: u32 = 320;
const H: u32 = 240;
const LEVELS: u64 = 40;

#[test]
fn a_long_call_stays_bounded() {
    init();
    let _mixed = rinch_rtc::testing::capture_audio_output();
    let secs: u64 = std::env::var("RINCH_RTC_LONGRUN_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let a = Peer::new(RtcConfig::default()).unwrap();
    let b = Peer::new(RtcConfig::default()).unwrap();
    let (ao, bo) = (trickle(&a), trickle(&b));
    let x = |a: &Peer, b: &Peer| {
        for c in ao.borrow_mut().drain(..) {
            b.add_ice_candidate(c);
        }
        for c in bo.borrow_mut().drain(..) {
            a.add_ice_candidate(c);
        }
    };
    let run = Arc::new(AtomicBool::new(true));
    let (audio, aw) = LocalTrack::audio_source(AudioSourceOptions {
        sample_rate: 48_000,
        channels: 1,
    })
    .unwrap();
    let (video, vw) = LocalTrack::video_source(VideoSourceOptions {
        width: W,
        height: H,
        fps: 30,
    })
    .unwrap();
    // push time of frame k (k = level index sequence)
    let pushed: Arc<Mutex<VecDeque<(u64, Instant)>>> = Arc::default();
    let aw2 = aw.clone();
    {
        let run_v = run.clone();
        let run = run.clone();
        std::thread::spawn(move || {
            let start = Instant::now();
            let mut k = 0u64;
            let mut phase = 0f64;
            while run.load(Ordering::Relaxed) && !aw2.is_ended() {
                k += 1;
                let due = start + Duration::from_millis(10 * k);
                if let Some(d) = due.checked_duration_since(Instant::now()) {
                    std::thread::sleep(d);
                }
                let mut v = Vec::with_capacity(480);
                for _ in 0..480 {
                    v.push((phase.sin() * 0.3) as f32);
                    phase += 2.0 * std::f64::consts::PI * 440.0 / 48_000.0;
                }
                aw2.push(&v);
            }
        });
        let run = run_v;
        let pushed = pushed.clone();
        std::thread::spawn(move || {
            let start = Instant::now();
            let mut k = 0u64;
            while run.load(Ordering::Relaxed) && !vw.is_ended() {
                // Change level every 3 frames (100 ms): 40 levels, 4 s period.
                let idx = k / 3;
                let level = (20 + (idx % LEVELS) * 5) as u8;
                let f = vec![level; (W * H * 4) as usize];
                vw.push_rgba(&f, W, H);
                if k.is_multiple_of(3) {
                    let mut p = pushed.lock().unwrap();
                    p.push_back((idx, Instant::now()));
                    while p.len() > 200 {
                        p.pop_front();
                    }
                }
                k += 1;
                let due = start + Duration::from_micros(33_333 * k);
                if let Some(d) = due.checked_duration_since(Instant::now()) {
                    std::thread::sleep(d);
                }
            }
        });
    }
    let got: Rc<RefCell<Vec<RemoteTrack>>> = Rc::default();
    {
        let g = got.clone();
        b.on_track(move |t| g.borrow_mut().push(t));
    }
    a.add_track(&audio).unwrap();
    a.add_track(&video).unwrap();
    let offer = drive(a.create_offer(), T).unwrap();
    drive(b.set_remote_description(offer), T).unwrap();
    let ans = drive(b.create_answer(), T).unwrap();
    drive(a.set_remote_description(ans), T).unwrap();
    assert!(pump_until(
        || {
            x(&a, &b);
            a.state() == PeerState::Connected && got.borrow().len() == 2
        },
        T
    ));
    let rv = got
        .borrow()
        .iter()
        .find(|t| t.kind() == MediaKind::Video)
        .cloned()
        .unwrap();
    // latency samples: level change seen -> matching push time
    let lat: Arc<Mutex<Vec<f64>>> = Arc::default();
    {
        let lat = lat.clone();
        let pushed = pushed.clone();
        let mut last_level: i64 = -1;
        rv.on_frame(move |rgba, _w, _h| {
            let mid = (rgba.len() / 2) & !3;
            let l = i64::from(rgba[mid]);
            let lvl = ((l - 20 + 2).max(0) / 5) as u64 % LEVELS;
            if lvl as i64 != last_level {
                last_level = lvl as i64;
                let now = Instant::now();
                let p = pushed.lock().unwrap();
                // newest push with this level
                if let Some((_, t)) = p.iter().rev().find(|(i, _)| i % LEVELS == lvl) {
                    lat.lock()
                        .unwrap()
                        .push(now.duration_since(*t).as_secs_f64() * 1000.0);
                }
            }
        });
    }
    let t0 = Instant::now();
    let mut next = 5u64;
    while t0.elapsed().as_secs() < secs {
        let _ = pump_until(
            || {
                x(&a, &b);
                false
            },
            Duration::from_millis(500),
        );
        if t0.elapsed().as_secs() >= next {
            next += 5;
            let mut v = std::mem::take(&mut *lat.lock().unwrap());
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let q = |p: f64| {
                if v.is_empty() {
                    f64::NAN
                } else {
                    v[((v.len() - 1) as f64 * p) as usize]
                }
            };
            let queued = rinch_rtc::testing::audio_samples_taken(&aw);
            let depth = rinch_rtc::testing::mixer_depth_ms();
            eprintln!(
                "t={:>3}s video latency n={} p50={:.1}ms p90={:.1}ms max={:.1}ms | rx audio buffer {:.1} ms | tx audio taken {}",
                t0.elapsed().as_secs(),
                v.len(),
                q(0.5),
                q(0.9),
                q(1.0),
                depth,
                queued
            );
            assert!(!v.is_empty(), "level changes seen in the last 5 s");
            assert!(q(0.9) < 150.0, "video latency p90 {:.1} ms", q(0.9));
            assert!(depth <= 170.0, "remote audio buffered {depth:.1} ms");
        }
    }
    run.store(false, Ordering::Relaxed);
    a.close();
    b.close();
}
