//! Data-channel round trips with and without 720p video both ways on the
//! same peers (#1947's review, adopted as a probe). It measures and
//! prints; it asserts nothing about timing, which depends on the machine.
//! Ignored by default:
//!
//! ```text
//! cargo test -p rinch-rtc --features media --test media_dc_latency -- --ignored --nocapture
//! ```
//!
//! Knobs: `PROBE_W` / `PROBE_H` (1280x720), `PROBE_CAP` (a bitrate cap),
//! `PROBE_BOX` / `PROBE_CAM` (content: a box, camera-like, or noise).
#![cfg(all(not(target_arch = "wasm32"), feature = "media"))]
mod common;
use common::{T, drive, init, pump_until};
use rinch_rtc::media::{LocalTrack, RemoteTrack, VideoSourceOptions};
use rinch_rtc::{Channel, IceCandidate, Peer, PeerState, RtcConfig};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

type Inbox = Rc<RefCell<Vec<Option<IceCandidate>>>>;
fn trickle(p: &Peer) -> Inbox {
    let i: Inbox = Rc::default();
    let j = i.clone();
    p.on_ice_candidate(move |c| j.borrow_mut().push(c));
    i
}

fn stats(name: &str, mut v: Vec<f64>) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p = |q: f64| v[((v.len() - 1) as f64 * q) as usize];
    eprintln!(
        "PROBE {name}: n={} p50={:.2}ms p90={:.2}ms p99={:.2}ms max={:.2}ms",
        v.len(),
        p(0.5),
        p(0.9),
        p(0.99),
        v[v.len() - 1]
    );
}

#[test]
#[ignore = "a timing probe: run with --ignored"]
fn dc_latency_under_video() {
    init();
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
    let ch = a.create_data_channel("ping", Default::default()).unwrap();
    let bch: Rc<RefCell<Option<Channel>>> = Rc::default();
    let bc = bch.clone();
    b.on_data_channel(move |c| {
        let c2 = c.clone();
        c.on_message(move |m| {
            let _ = c2.send(m);
        });
        *bc.borrow_mut() = Some(c);
    });
    let rtts: Rc<RefCell<Vec<f64>>> = Rc::default();
    let sent: Rc<RefCell<Option<Instant>>> = Rc::default();
    {
        let r = rtts.clone();
        let s = sent.clone();
        ch.on_message(move |_| {
            if let Some(t) = s.borrow_mut().take() {
                r.borrow_mut().push(t.elapsed().as_secs_f64() * 1000.0);
            }
        });
    }
    let neg = |a: &Peer, b: &Peer| {
        let offer = drive(a.create_offer(), T).unwrap();
        drive(b.set_remote_description(offer), T).unwrap();
        let ans = drive(b.create_answer(), T).unwrap();
        drive(a.set_remote_description(ans), T).unwrap();
    };
    neg(&a, &b);
    let open = Rc::new(RefCell::new(false));
    {
        let o = open.clone();
        ch.on_open(move || *o.borrow_mut() = true);
    }
    assert!(pump_until(
        || {
            x(&a, &b);
            a.state() == PeerState::Connected && *open.borrow()
        },
        T
    ));
    let measure = |name: &str| {
        rtts.borrow_mut().clear();
        for _ in 0..300 {
            *sent.borrow_mut() = Some(Instant::now());
            ch.send(vec![1u8; 64]).unwrap();
            let _ = pump_until(|| sent.borrow().is_none(), Duration::from_secs(2));
            let _ = pump_until(|| false, Duration::from_millis(10));
        }
        stats(name, rtts.borrow().clone());
    };
    measure("baseline (no media)");
    let w: u32 = std::env::var("PROBE_W")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1280);
    let h: u32 = std::env::var("PROBE_H")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(720);
    let run = Arc::new(AtomicBool::new(true));
    let mut tracks = Vec::new();
    for seed in [1u32, 7] {
        let (t, wr) = LocalTrack::video_source(VideoSourceOptions {
            width: w,
            height: h,
            fps: 30,
        })
        .unwrap();
        let run = run.clone();
        std::thread::spawn(move || {
            let mut k = seed;
            let mut f = vec![0u8; (w * h * 4) as usize];
            while run.load(Ordering::Relaxed) && !wr.is_ended() {
                // Noisy content: hard to encode, many packets.
                if std::env::var("PROBE_CAM").is_ok() {
                    let off = (k as usize * 3) % 256;
                    for (i, px) in f.chunks_mut(4).enumerate() {
                        let (x, y) = (i % 1280, i / 1280);
                        k = k.wrapping_mul(1664525).wrapping_add(1013904223);
                        let n = ((k >> 24) % 13) as i32 - 6;
                        let base = (((x + off) / 4) as i32 + (y / 8) as i32) % 256;
                        let v = (base + n).clamp(0, 255) as u8;
                        px[0] = v;
                        px[1] = v / 2 + 40;
                        px[2] = 255 - v;
                        px[3] = 255;
                    }
                } else if std::env::var("PROBE_BOX").is_ok() {
                    f.iter_mut().for_each(|p| *p = 0);
                    let x = (k as usize * 8) % 1000;
                    for row in 100..300usize {
                        for col in x..x + 200 {
                            let i = (row * 1280 + col) * 4;
                            f[i] = 255;
                            f[i + 1] = 255;
                            f[i + 2] = 255;
                            f[i + 3] = 255;
                        }
                    }
                    k += 1;
                } else {
                    for px in f.chunks_mut(4) {
                        k = k.wrapping_mul(1664525).wrapping_add(1013904223);
                        px[0] = (k >> 24) as u8;
                        px[1] = (k >> 16) as u8;
                        px[2] = (k >> 8) as u8;
                        px[3] = 255;
                    }
                }
                wr.push_rgba(&f, w, h);
                std::thread::sleep(Duration::from_millis(33));
            }
        });
        tracks.push(t);
    }
    let got_b: Rc<RefCell<Vec<RemoteTrack>>> = Rc::default();
    let got_a: Rc<RefCell<Vec<RemoteTrack>>> = Rc::default();
    {
        let g = got_b.clone();
        b.on_track(move |t| g.borrow_mut().push(t));
    }
    {
        let g = got_a.clone();
        a.on_track(move |t| g.borrow_mut().push(t));
    }
    let s0 = a.add_track(&tracks[0]).unwrap();
    if let Some(c) = std::env::var("PROBE_CAP").ok().and_then(|s| s.parse().ok()) {
        s0.set_max_bitrate(Some(c));
    }
    neg(&a, &b);
    let s1 = b.add_track(&tracks[1]).unwrap();
    if let Some(c) = std::env::var("PROBE_CAP").ok().and_then(|s| s.parse().ok()) {
        s1.set_max_bitrate(Some(c));
    }
    neg(&b, &a);
    assert!(pump_until(
        || {
            x(&a, &b);
            !got_a.borrow().is_empty() && !got_b.borrow().is_empty()
        },
        T
    ));
    let n = Arc::new(AtomicUsize::new(0));
    for t in got_a.borrow().iter().chain(got_b.borrow().iter()) {
        let n = n.clone();
        t.on_frame(move |_, _, _| {
            n.fetch_add(1, Ordering::Relaxed);
        });
    }
    let _ = pump_until(|| false, Duration::from_secs(2));
    let n0 = n.load(Ordering::Relaxed);
    eprintln!("PROBE frames before measurement: {n0}");
    let t0 = Instant::now();
    measure("720p noise both ways");
    let fps = (n.load(Ordering::Relaxed) - n0) as f64 / t0.elapsed().as_secs_f64() / 2.0;
    eprintln!("PROBE decoded fps per direction during measurement: {fps:.1}");
    run.store(false, Ordering::Relaxed);
    a.close();
    b.close();
}
