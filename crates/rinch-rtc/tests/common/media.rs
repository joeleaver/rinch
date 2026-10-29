//! Shared pieces of the native media tests: a connected pair of peers and
//! synthetic sources (no device is ever opened).
#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rinch_rtc::media::{AudioSourceWriter, RemoteTrack, VideoSourceWriter};
use rinch_rtc::{IceCandidate, Peer, PeerState, RtcConfig};

use super::{T, drive, pump_until};

type Inbox = Rc<RefCell<Vec<Option<IceCandidate>>>>;

fn trickle(peer: &Peer) -> Inbox {
    let inbox: Inbox = Rc::default();
    let i = inbox.clone();
    peer.on_ice_candidate(move |c| i.borrow_mut().push(c));
    inbox
}

/// Two peers trickling candidates to each other; `b` records its remote
/// tracks.
pub struct Pair {
    pub a: Peer,
    pub b: Peer,
    a_out: Inbox,
    b_out: Inbox,
    pub remotes: Rc<RefCell<Vec<RemoteTrack>>>,
}

impl Pair {
    pub fn new() -> Self {
        let a = Peer::new(RtcConfig::default()).unwrap();
        let b = Peer::new(RtcConfig::default()).unwrap();
        Self::with(a, b)
    }

    /// Two given peers, wired as `new` wires its own (trickled candidates,
    /// `b`'s remote tracks collected).
    pub fn with(a: Peer, b: Peer) -> Self {
        let (a_out, b_out) = (trickle(&a), trickle(&b));
        let remotes: Rc<RefCell<Vec<RemoteTrack>>> = Rc::default();
        let r = remotes.clone();
        b.on_track(move |t| r.borrow_mut().push(t));
        Pair {
            a,
            b,
            a_out,
            b_out,
            remotes,
        }
    }

    pub fn pump(&self) {
        for c in self.a_out.borrow_mut().drain(..) {
            self.b.add_ice_candidate(c);
        }
        for c in self.b_out.borrow_mut().drain(..) {
            self.a.add_ice_candidate(c);
        }
    }

    pub fn negotiate(&self) {
        let offer = drive(self.a.create_offer(), T).unwrap();
        drive(self.b.set_remote_description(offer), T).unwrap();
        let answer = drive(self.b.create_answer(), T).unwrap();
        drive(self.a.set_remote_description(answer), T).unwrap();
        self.pump();
    }

    /// Negotiate and wait for `tracks` remote tracks on `b`.
    pub fn connect(&self, tracks: usize) {
        self.negotiate();
        self.until("connected, tracks arrived", || {
            self.a.state() == PeerState::Connected
                && self.b.state() == PeerState::Connected
                && self.remotes.borrow().len() >= tracks
        });
    }

    pub fn until(&self, what: &str, mut cond: impl FnMut() -> bool) {
        assert!(
            pump_until(
                || {
                    self.pump();
                    cond()
                },
                Duration::from_secs(15)
            ),
            "timed out waiting for {what}"
        );
    }

    pub fn wait(&self, ms: u64) {
        let _ = pump_until(
            || {
                self.pump();
                false
            },
            Duration::from_millis(ms),
        );
    }

    pub fn close(&self) {
        self.a.close();
        self.b.close();
    }
}

/// The synthetic sources' threads are named `synth-*`, so a test can wait
/// for them to exit (media_lifetime counts the heap after each call, and a
/// source thread still running holds its frame).
fn spawn_source(name: &str, f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(f)
        .unwrap();
}

/// A white `side`-pixel box moving over black, `w`×`h`, pushed at 30 fps
/// (deadline-paced) until `run` clears.
pub fn moving_box(writer: VideoSourceWriter, w: u32, h: u32, side: u32, run: Arc<AtomicBool>) {
    spawn_source("synth-box", move || {
        let start = Instant::now();
        let mut k = 0u32;
        while run.load(Ordering::Relaxed) && !writer.is_ended() {
            let x = (k * 4) % (w - side);
            let y = (k * 3) % (h - side);
            let mut f = vec![0u8; (w * h * 4) as usize];
            for px in f.as_chunks_mut::<4>().0 {
                px[3] = 255;
            }
            for row in y..y + side {
                for col in x..x + side {
                    let i = ((row * w + col) * 4) as usize;
                    f[i..i + 3].copy_from_slice(&[255, 255, 255]);
                }
            }
            writer.push_rgba(&f, w, h);
            k += 1;
            let due = start + Duration::from_millis(33) * k;
            if let Some(d) = due.checked_duration_since(Instant::now()) {
                std::thread::sleep(d);
            }
        }
    });
}

/// Full-frame noise (incompressible), pushed at 30 fps until `run` clears.
pub fn noise(writer: VideoSourceWriter, w: u32, h: u32, run: Arc<AtomicBool>) {
    spawn_source("synth-noise", move || {
        let start = Instant::now();
        let mut k = 0u32;
        let mut x = 0x1234_5678u32;
        while run.load(Ordering::Relaxed) && !writer.is_ended() {
            let mut f = vec![0u8; (w * h * 4) as usize];
            for px in f.as_chunks_mut::<4>().0 {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                px.copy_from_slice(&[(x >> 24) as u8, (x >> 16) as u8, (x >> 8) as u8, 255]);
            }
            writer.push_rgba(&f, w, h);
            k += 1;
            let due = start + Duration::from_millis(33) * k;
            if let Some(d) = due.checked_duration_since(Instant::now()) {
                std::thread::sleep(d);
            }
        }
    });
}

/// A 440 Hz tone at `rate`, 10 ms at a time, deadline-paced, until `run`
/// clears.
pub fn tone(writer: AudioSourceWriter, rate: u32, run: Arc<AtomicBool>) {
    spawn_source("synth-tone", move || {
        let start = Instant::now();
        let mut phase = 0f64;
        let mut pushed = 0u64;
        let mut k = 0u64;
        while run.load(Ordering::Relaxed) && !writer.is_ended() {
            k += 1;
            let target = u64::from(rate) * k / 100;
            let n = (target - pushed) as usize;
            let mut out = Vec::with_capacity(n);
            for _ in 0..n {
                out.push((phase.sin() * 0.5) as f32);
                phase += 2.0 * std::f64::consts::PI * 440.0 / f64::from(rate);
            }
            writer.push(&out);
            pushed = target;
            let due = start + Duration::from_millis(10) * k as u32;
            if let Some(d) = due.checked_duration_since(Instant::now()) {
                std::thread::sleep(d);
            }
        }
    });
}

/// The bright pixels of a frame: (count, bounding width, bounding height).
pub fn bright(rgba: &[u8], w: u32, h: u32) -> (u32, u32, u32) {
    let (mut x0, mut y0, mut x1, mut y1, mut n) = (u32::MAX, u32::MAX, 0, 0, 0u32);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let l = (u32::from(rgba[i]) + u32::from(rgba[i + 1]) + u32::from(rgba[i + 2])) / 3;
            if l > 128 {
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

pub fn is_box(rgba: &[u8], w: u32, h: u32, side: u32) -> bool {
    let (n, bw, bh) = bright(rgba, w, h);
    let near = |v: u32| v.abs_diff(side) <= side / 5 + 4;
    n > 0 && near(bw) && near(bh) && n * 10 >= side * side * 7
}

/// Audio defects in a mono 440 Hz stream: (clicks, gaps of 2 ms or more).
/// A click is a sample the 440 Hz recurrence does not predict.
pub fn defects(left: &[f32]) -> (usize, usize) {
    let w = 2.0 * std::f32::consts::PI * 440.0 / 48_000.0;
    let c = 2.0 * w.cos();
    let (mut clicks, mut last) = (0, 0usize);
    for n in 2..left.len() {
        let r = (left[n] - c * left[n - 1] + left[n - 2]).abs();
        if r > 0.08 && n > last + 48 {
            clicks += 1;
            last = n;
        }
    }
    let (mut gaps, mut run) = (0, 0usize);
    for &x in left {
        if x.abs() < 1e-3 {
            run += 1;
        } else {
            if run >= 96 {
                gaps += 1;
            }
            run = 0;
        }
    }
    (clicks, gaps)
}
