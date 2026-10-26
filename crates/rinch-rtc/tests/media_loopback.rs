//! Native media end to end (design record `docs/design/rtc-media.md`,
//! section 6): two rinch-rtc peers on one worker, H.264 and Opus through
//! webrtc-rs on loopback. Synthetic sources only: a moving box pushed
//! through `LocalTrack::video_source`, a 440 Hz tone through
//! `LocalTrack::audio_source`. Remote audio is read from the mixer
//! (`testing::capture_audio_output`), so no audio device is opened.
#![cfg(all(not(target_arch = "wasm32"), feature = "media"))]

mod common;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{T, drive, init, pump_until};
use rinch_rtc::media::{
    AudioSourceOptions, AudioSourceWriter, Direction, LocalTrack, MediaKind, RemoteTrack,
    VideoSourceOptions, VideoSourceWriter,
};
use rinch_rtc::{Description, IceCandidate, Peer, PeerState, RtcConfig, SignalingState};

const W: u32 = 320;
const H: u32 = 240;
const BOX: u32 = 64;

type Inbox = Rc<RefCell<Vec<Option<IceCandidate>>>>;

fn trickle(peer: &Peer) -> Inbox {
    let inbox: Inbox = Rc::default();
    let i = inbox.clone();
    peer.on_ice_candidate(move |c| i.borrow_mut().push(c));
    inbox
}

struct Pair {
    a: Peer,
    b: Peer,
    a_out: Inbox,
    b_out: Inbox,
    errors: Rc<RefCell<Vec<String>>>,
}

impl Pair {
    fn new() -> Self {
        let a = Peer::new(RtcConfig::default()).unwrap();
        let b = Peer::new(RtcConfig::default()).unwrap();
        let (a_out, b_out) = (trickle(&a), trickle(&b));
        let errors: Rc<RefCell<Vec<String>>> = Rc::default();
        for p in [&a, &b] {
            let e = errors.clone();
            p.on_error(move |err| e.borrow_mut().push(err.to_string()));
        }
        Pair {
            a,
            b,
            a_out,
            b_out,
            errors,
        }
    }

    fn pump(&self) {
        for c in self.a_out.borrow_mut().drain(..) {
            self.b.add_ice_candidate(c);
        }
        for c in self.b_out.borrow_mut().drain(..) {
            self.a.add_ice_candidate(c);
        }
    }

    fn negotiate(&self) {
        let offer = drive(self.a.create_offer(), T).unwrap();
        drive(self.b.set_remote_description(offer), T).unwrap();
        let answer = drive(self.b.create_answer(), T).unwrap();
        drive(self.a.set_remote_description(answer), T).unwrap();
        self.pump();
    }

    fn connect(&self) {
        self.negotiate();
        assert!(
            pump_until(
                || {
                    self.pump();
                    self.a.state() == PeerState::Connected && self.b.state() == PeerState::Connected
                },
                T
            ),
            "connected"
        );
    }

    fn until(&self, what: &str, mut cond: impl FnMut() -> bool) {
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

    fn wait(&self, ms: u64) {
        let _ = pump_until(
            || {
                self.pump();
                false
            },
            Duration::from_millis(ms),
        );
    }
}

/// A thread pushing a white box moving over black at 30 fps; `side` is the
/// box's side. Stops when `run` clears.
fn moving_box(writer: VideoSourceWriter, side: u32, run: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let mut k = 0u32;
        while run.load(Ordering::Relaxed) && !writer.is_ended() {
            let x = (k * 4) % (W - side);
            let y = (k * 3) % (H - side);
            let mut f = vec![0u8; (W * H * 4) as usize];
            for px in f.as_chunks_mut::<4>().0 {
                px[3] = 255;
            }
            for row in y..y + side {
                for col in x..x + side {
                    let i = ((row * W + col) * 4) as usize;
                    f[i..i + 3].copy_from_slice(&[255, 255, 255]);
                }
            }
            writer.push_rgba(&f, W, H);
            k += 1;
            std::thread::sleep(Duration::from_millis(33));
        }
    });
}

/// A thread pushing a 440 Hz tone, 20 ms at a time.
fn tone(writer: AudioSourceWriter, rate: u32, run: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let mut phase = 0f64;
        while run.load(Ordering::Relaxed) && !writer.is_ended() {
            let n = rate as usize / 50;
            let mut out = Vec::with_capacity(n);
            for _ in 0..n {
                out.push((phase.sin() * 0.5) as f32);
                phase += 2.0 * std::f64::consts::PI * 440.0 / f64::from(rate);
            }
            writer.push(&out);
            std::thread::sleep(Duration::from_millis(20));
        }
    });
}

#[derive(Debug, Clone, Copy, Default)]
struct Seen {
    bright: u32,
    w: u32,
    h: u32,
}

fn measure(rgba: &[u8], w: u32, h: u32) -> Seen {
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
        return Seen::default();
    }
    Seen {
        bright: n,
        w: x1 - x0 + 1,
        h: y1 - y0 + 1,
    }
}

fn is_box(s: Seen, side: u32) -> bool {
    let near = |v: u32| v.abs_diff(side) <= side / 5 + 4;
    near(s.w) && near(s.h) && s.bright * 10 >= side * side * 7
}

type Frames = Arc<Mutex<Vec<Seen>>>;

fn record(t: &RemoteTrack) -> Frames {
    let frames: Frames = Arc::default();
    let f = frames.clone();
    t.on_frame(move |rgba, w, h| {
        if w == W && h == H {
            f.lock().unwrap().push(measure(rgba, w, h));
        }
    });
    frames
}

fn fresh(pair: &Pair, frames: &Frames, settle: u64, want: usize) -> Vec<Seen> {
    pair.wait(settle);
    frames.lock().unwrap().clear();
    pair.until("fresh frames", || frames.lock().unwrap().len() >= want);
    frames.lock().unwrap().clone()
}

fn share(frames: &[Seen], side: u32) -> f64 {
    frames.iter().filter(|s| is_box(**s, side)).count() as f64 / frames.len() as f64
}

/// Goertzel power at `hz` over `samples` (mono, 48 kHz), relative to the
/// mean power at a few other frequencies.
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
    // 479 Hz is where 440 Hz lands if a 44.1 kHz source were sent as
    // 48 kHz without resampling.
    let others = [300.0, 405.0, 479.0, 600.0, 1000.0];
    let base = others.iter().map(|f| power(*f)).sum::<f32>() / others.len() as f32;
    power(hz) / base.max(1e-9)
}

fn remotes(peer: &Peer) -> Rc<RefCell<Vec<RemoteTrack>>> {
    let got: Rc<RefCell<Vec<RemoteTrack>>> = Rc::default();
    let g = got.clone();
    peer.on_track(move |t| g.borrow_mut().push(t));
    got
}

#[test]
fn native_media_end_to_end() {
    init();
    let mixed = rinch_rtc::testing::capture_audio_output();
    let run = Arc::new(AtomicBool::new(true));

    // ------------------------------------------------ video, then audio
    let pair = Pair::new();
    let (video, vw) = LocalTrack::video_source(VideoSourceOptions {
        width: W,
        height: H,
        fps: 30,
    })
    .unwrap();
    moving_box(vw.clone(), BOX, run.clone());
    let (audio, aw) = LocalTrack::audio_source(AudioSourceOptions {
        sample_rate: 44_100,
        channels: 1,
    })
    .unwrap();
    tone(aw.clone(), 44_100, run.clone());

    let needed = Rc::new(Cell::new(0));
    let n = needed.clone();
    pair.a.on_negotiation_needed(move || n.set(n.get() + 1));
    let got = remotes(&pair.b);
    let vsender = pair.a.add_track(&video).unwrap();
    pair.a.add_track(&audio).unwrap();
    assert_eq!(pair.a.transceivers().len(), 2, "known at once");
    pair.until("negotiationneeded", || needed.get() > 0);
    pair.connect();
    pair.until("both tracks", || got.borrow().len() == 2);
    let rv = got
        .borrow()
        .iter()
        .find(|t| t.kind() == MediaKind::Video)
        .cloned()
        .unwrap();
    let ra = got
        .borrow()
        .iter()
        .find(|t| t.kind() == MediaKind::Audio)
        .cloned()
        .unwrap();
    assert_eq!(
        rv.stream_ids(),
        ra.stream_ids(),
        "one stream for a's tracks"
    );
    assert_eq!(rv.id(), video.id());
    let txs = pair.a.transceivers();
    assert!(txs.iter().all(|t| t.mid().is_some()));
    assert_eq!(txs[0].current_direction(), Some(Direction::SendOnly));

    // The box arrives as a box in >= 80% of frames.
    let frames = record(&rv);
    let seen = fresh(&pair, &frames, 500, 30);
    assert!(share(&seen, BOX) >= 0.8, "box frames: {seen:?}");

    // The tone arrives (mixed output, 48 kHz stereo; left channel).
    pair.wait(1500);
    let left: Vec<f32> = {
        let m = mixed.lock().unwrap();
        let tail = &m[m.len().saturating_sub(48_000 * 2)..];
        tail.iter().step_by(2).copied().collect()
    };
    let ratio = tone_ratio(&left, 440.0);
    assert!(ratio > 100.0, "440 Hz stands {ratio}x above other bins");
    ra.set_volume(0.0);
    pair.wait(600);
    let quiet: f32 = {
        let m = mixed.lock().unwrap();
        m[m.len().saturating_sub(9600)..]
            .iter()
            .map(|s| s * s)
            .sum()
    };
    assert!(quiet < 1e-6, "volume 0 is silent: {quiet}");
    ra.set_volume(1.0);

    // Disabled: black frames.
    video.set_enabled(false);
    let seen = fresh(&pair, &frames, 1500, 2);
    assert!(seen.iter().all(|s| s.bright == 0), "black: {seen:?}");
    video.set_enabled(true);

    // replace_track to a smaller box, no renegotiation.
    let (video2, vw2) = LocalTrack::video_source(VideoSourceOptions {
        width: W,
        height: H,
        fps: 30,
    })
    .unwrap();
    moving_box(vw2, 32, run.clone());
    let before = needed.get();
    drive(vsender.replace_track(Some(&video2)), T).unwrap();
    assert_eq!(vsender.track().map(|t| t.id()), Some(video2.id()));
    let seen = fresh(&pair, &frames, 1500, 30);
    assert!(share(&seen, 32) >= 0.8, "small box: {seen:?}");
    assert_eq!(needed.get(), before, "replace_track renegotiated");
    vsender.set_max_bitrate(Some(200_000));
    assert_eq!(
        rinch_rtc::testing::encoder_target_bitrate(&video2, W, H),
        200_000
    );
    let seen = fresh(&pair, &frames, 1000, 10);
    assert!(
        share(&seen, 32) >= 0.7,
        "still the box at 200 kbps: {seen:?}"
    );
    vsender.set_max_bitrate(None);
    assert_eq!(
        rinch_rtc::testing::encoder_target_bitrate(&video2, W, H),
        800_000
    );

    // Inactive (renegotiated) mutes the far side; back to SendRecv unmutes
    // the same RemoteTrack (no second on_track).
    let muted = Rc::new(Cell::new(0));
    let unmuted = Rc::new(Cell::new(0));
    let (m, u) = (muted.clone(), unmuted.clone());
    rv.on_mute(move || m.set(m.get() + 1));
    rv.on_unmute(move || u.set(u.get() + 1));
    let vtx = txs
        .iter()
        .find(|t| t.kind() == MediaKind::Video)
        .unwrap()
        .clone();
    vtx.set_direction(Direction::Inactive);
    pair.negotiate();
    assert_eq!(vtx.current_direction(), Some(Direction::Inactive));
    pair.until("far track mutes", || muted.get() > 0);
    vtx.set_direction(Direction::SendRecv);
    pair.negotiate();
    pair.until("far track unmutes", || unmuted.get() > 0 && !rv.muted());
    assert!(!rv.ended());
    assert_eq!(got.borrow().len(), 2, "on_track again for the same track");
    let seen = fresh(&pair, &frames, 1000, 10);
    assert!(share(&seen, 32) >= 0.7, "frames again: {seen:?}");

    // Close: the remote tracks end.
    let ended = Rc::new(Cell::new(0));
    for t in got.borrow().iter() {
        let e = ended.clone();
        t.on_ended(move || e.set(e.get() + 1));
    }
    pair.b.close();
    pair.until("remote tracks end", || ended.get() == 2);
    pair.a.close();
    assert!(
        pair.errors.borrow().is_empty(),
        "{:?}",
        pair.errors.borrow()
    );

    // ------------------------------------------- glare and rollback
    let pair = Pair::new();
    let _ch = pair.a.create_data_channel("x", Default::default()).unwrap();
    pair.connect();
    let (ta, twa) = LocalTrack::video_source(VideoSourceOptions::default()).unwrap();
    let (tb, twb) = LocalTrack::video_source(VideoSourceOptions::default()).unwrap();
    for w in [twa, twb] {
        w.push_rgba(&vec![90u8; 640 * 480 * 4], 640, 480);
    }
    let (ra, rb) = (remotes(&pair.a), remotes(&pair.b));
    pair.a.add_track(&ta).unwrap();
    pair.b.add_track(&tb).unwrap();
    let (oa, ob) = (pair.a.create_offer(), pair.b.create_offer());
    assert_eq!(pair.b.signaling_state(), SignalingState::HaveLocalOffer);
    let oa = drive(oa, T).unwrap();
    let _ob = drive(ob, T).unwrap();
    // Polite b takes a's offer: webrtc-rs needs the rollback rinch-rtc does.
    drive(pair.b.set_remote_description(oa), T).unwrap();
    assert_eq!(pair.b.signaling_state(), SignalingState::HaveRemoteOffer);
    let answer = drive(pair.b.create_answer(), T).unwrap();
    drive(pair.a.set_remote_description(answer), T).unwrap();
    pair.until("b hears a's track", || !rb.borrow().is_empty());
    let offer = drive(pair.b.create_offer(), T).unwrap();
    drive(pair.a.set_remote_description(offer), T).unwrap();
    let answer = drive(pair.a.create_answer(), T).unwrap();
    drive(pair.b.set_remote_description(answer), T).unwrap();
    pair.until("a hears b's track", || !ra.borrow().is_empty());
    // An explicit rollback of our own offer.
    let (tc, _wc) = LocalTrack::video_source(VideoSourceOptions::default()).unwrap();
    pair.a.add_track(&tc).unwrap();
    let _ = drive(pair.a.create_offer(), T).unwrap();
    drive(pair.a.set_remote_description(Description::rollback()), T).unwrap();
    assert_eq!(pair.a.signaling_state(), SignalingState::Stable);
    pair.a.close();
    pair.b.close();

    run.store(false, Ordering::Relaxed);
    // Writers see their tracks end.
    audio.stop();
    assert!(aw.is_ended());
    drop(video);
    assert!(vw.is_ended());
}
