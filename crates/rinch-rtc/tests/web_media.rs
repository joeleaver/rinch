//! The `media` feature's web backend in a real browser (design record
//! `docs/design/rtc-media.md`, section 6): two rinch-rtc peers in one page.
//!
//! Every source is synthetic, made by the test: a moving box pushed through
//! `LocalTrack::video_source`, and a 440 Hz tone pushed through
//! `LocalTrack::audio_source`. No `getUserMedia`, so no device, no permission
//! prompt and no fake-device flags. The positive control in each case is
//! what arrived at the far side, which can only have come through the
//! browser's encoder, RTP and decoder.
//!
//! `webdriver.json` (this crate's) lets audio play without a user gesture.
#![cfg(all(target_arch = "wasm32", feature = "media"))]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use rinch_core::Component;
use rinch_rtc::media::{
    AudioSourceOptions, AudioSourceWriter, Direction, LocalTrack, MediaKind, MediaView,
    RemoteTrack, VideoSourceOptions, VideoSourceWriter,
};
use rinch_rtc::{IceCandidate, Peer, PeerState, RtcConfig};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const W: u32 = 320;
const H: u32 = 240;
const BOX: u32 = 64;

/// Let the browser run: one macrotask.
async fn tick() {
    let p = js_sys::Promise::new(&mut |resolve, _| {
        let _ = web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 5);
    });
    let _ = wasm_bindgen_futures::JsFuture::from(p).await;
}

async fn sleep(ms: i32) {
    let p = js_sys::Promise::new(&mut |resolve, _| {
        let _ = web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
    });
    let _ = wasm_bindgen_futures::JsFuture::from(p).await;
}

/// Wait until `cond` holds, or panic after ~15 s.
async fn until(what: &str, mut cond: impl FnMut() -> bool) {
    for _ in 0..3000 {
        if cond() {
            return;
        }
        tick().await;
    }
    panic!("timed out waiting for {what}");
}

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
}

impl Pair {
    fn new() -> Self {
        let a = Peer::new(RtcConfig::default()).unwrap();
        let b = Peer::new(RtcConfig::default()).unwrap();
        let (a_out, b_out) = (trickle(&a), trickle(&b));
        Pair { a, b, a_out, b_out }
    }

    fn pump(&self) {
        for c in self.a_out.borrow_mut().drain(..) {
            self.b.add_ice_candidate(c);
        }
        for c in self.b_out.borrow_mut().drain(..) {
            self.a.add_ice_candidate(c);
        }
    }

    /// One offer/answer round, `a` offering.
    async fn negotiate(&self) {
        let offer = self.a.create_offer().await.unwrap();
        self.b.set_remote_description(offer).await.unwrap();
        let answer = self.b.create_answer().await.unwrap();
        self.a.set_remote_description(answer).await.unwrap();
        self.pump();
    }

    async fn connect(&self) {
        self.negotiate().await;
        until("both connected", || {
            self.pump();
            self.a.state() == PeerState::Connected && self.b.state() == PeerState::Connected
        })
        .await;
    }
}

/// A video source fed by a page timer: a white box moving over black.
/// `size` is the box's side. Returns the writer's timer to clear.
fn moving_box(writer: VideoSourceWriter, size: Rc<Cell<u32>>) -> (i32, Closure<dyn FnMut()>) {
    let n = Cell::new(0u32);
    let cb = Closure::<dyn FnMut()>::new(move || {
        let s = size.get();
        let k = n.get();
        n.set(k + 1);
        let x = (k * 4) % (W - s);
        let y = (k * 3) % (H - s);
        let mut frame = vec![0u8; (W * H * 4) as usize];
        for px in frame.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
        for row in y..y + s {
            for col in x..x + s {
                let i = ((row * W + col) * 4) as usize;
                frame[i..i + 3].copy_from_slice(&[255, 255, 255]);
            }
        }
        writer.push_rgba(&frame, W, H);
    });
    let id = web_sys::window()
        .unwrap()
        .set_interval_with_callback_and_timeout_and_arguments_0(cb.as_ref().unchecked_ref(), 33)
        .unwrap();
    (id, cb)
}

/// What a received frame shows: the bright pixels' bounding box and count.
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
            let luma = (rgba[i] as u32 + rgba[i + 1] as u32 + rgba[i + 2] as u32) / 3;
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

/// The box of side `side` is there, and nothing else is bright: its bounding
/// box is within a codec's blur of `side`, and it is mostly filled.
fn is_box(s: Seen, side: u32) -> bool {
    let near = |v: u32| v.abs_diff(side) <= side / 5 + 4;
    near(s.w) && near(s.h) && s.bright * 10 >= side * side * 7
}

type Frames = Arc<Mutex<Vec<Seen>>>;

fn record_frames(t: &RemoteTrack) -> Frames {
    let frames: Frames = Arc::default();
    let f = frames.clone();
    t.on_frame(move |rgba, w, h| {
        if w == W && h == H {
            f.lock().unwrap().push(measure(rgba, w, h));
        }
    });
    frames
}

fn remote_slot(peer: &Peer) -> Rc<RefCell<Vec<RemoteTrack>>> {
    let got: Rc<RefCell<Vec<RemoteTrack>>> = Rc::default();
    let g = got.clone();
    peer.on_track(move |t| g.borrow_mut().push(t));
    got
}

/// Frames from now on, after `settle` ms.
async fn fresh_frames(frames: &Frames, settle: i32, want: usize) -> Vec<Seen> {
    sleep(settle).await;
    frames.lock().unwrap().clear();
    until("fresh frames", || frames.lock().unwrap().len() >= want).await;
    frames.lock().unwrap().clone()
}

fn share(frames: &[Seen], side: u32) -> f64 {
    frames.iter().filter(|s| is_box(**s, side)).count() as f64 / frames.len() as f64
}

#[wasm_bindgen_test]
async fn video_crosses_and_follows_the_track() {
    let pair = Pair::new();
    let (track, writer) = LocalTrack::video_source(VideoSourceOptions {
        width: W,
        height: H,
        fps: 30,
    })
    .unwrap();
    assert_eq!(track.kind(), MediaKind::Video);
    let size = Rc::new(Cell::new(BOX));
    let (timer, _feed) = moving_box(writer, size.clone());

    let needed = Rc::new(Cell::new(0));
    let n = needed.clone();
    pair.a.on_negotiation_needed(move || n.set(n.get() + 1));
    let remotes = remote_slot(&pair.b);
    let sender = pair.a.add_track(&track).unwrap();
    until("negotiationneeded after add_track", || needed.get() > 0).await;
    pair.connect().await;
    until("b hears the track", || !remotes.borrow().is_empty()).await;
    let remote = remotes.borrow()[0].clone();
    assert_eq!(remote.kind(), MediaKind::Video);
    assert!(!remote.stream_ids().is_empty());
    let txs = pair.a.transceivers();
    assert_eq!(txs.len(), 1);
    assert_eq!(txs[0].kind(), MediaKind::Video);
    assert!(txs[0].mid().is_some());
    assert_eq!(txs[0].current_direction(), Some(Direction::SendOnly));

    // The box arrives where... anywhere, but as a box: >= 80% of frames.
    let frames = record_frames(&remote);
    let seen = fresh_frames(&frames, 500, 30).await;
    assert!(share(&seen, BOX) >= 0.8, "box frames: {seen:?}");

    // Disabled: black frames (the browser sends black, not nothing).
    track.set_enabled(false);
    assert!(!track.enabled());
    // The browser track the sender carries is the one disabled (not a
    // JS `clone()` of it).
    let mt = rinch_rtc::testing::local_media_track(&track).unwrap();
    assert!(!mt.enabled());
    assert_eq!(rinch_rtc::testing::sender_track_id(&sender), Some(mt.id()));
    let seen = fresh_frames(&frames, 1000, 10).await;
    assert!(
        seen.iter().filter(|s| s.bright == 0).count() * 10 >= seen.len() * 8,
        "black frames: {seen:?}"
    );
    track.set_enabled(true);

    // replace_track to another source (a smaller box): what arrives
    // changes, with no renegotiation.
    let (track2, writer2) = LocalTrack::video_source(VideoSourceOptions {
        width: W,
        height: H,
        fps: 30,
    })
    .unwrap();
    let (timer2, _feed2) = moving_box(writer2, Rc::new(Cell::new(32)));
    let before = needed.get();
    sender.replace_track(Some(&track2)).await.unwrap();
    assert_eq!(sender.track().map(|t| t.id()), Some(track2.id()));
    let seen = fresh_frames(&frames, 1000, 30).await;
    assert!(share(&seen, 32) >= 0.8, "small box frames: {seen:?}");
    assert_eq!(needed.get(), before, "replace_track renegotiated");
    // The bitrate cap is accepted (the far side keeps receiving).
    sender.set_max_bitrate(Some(300_000));
    sender.set_max_bitrate(None);

    // Inactive, renegotiated: the far side's track mutes.
    let muted = Rc::new(Cell::new(false));
    let m = muted.clone();
    remote.on_mute(move || m.set(true));
    txs[0].set_direction(Direction::Inactive);
    assert_eq!(txs[0].direction(), Direction::Inactive);
    until("negotiationneeded after set_direction", || {
        needed.get() > before
    })
    .await;
    pair.negotiate().await;
    assert_eq!(txs[0].current_direction(), Some(Direction::Inactive));
    until("b's track mutes", || muted.get()).await;
    assert!(remote.muted());

    // Closing ends the remote track.
    let ended = Rc::new(Cell::new(false));
    let e = ended.clone();
    remote.on_ended(move || e.set(true));
    pair.b.close();
    until("remote on_ended on close", || ended.get()).await;
    assert!(remote.ended());
    pair.a.close();

    let w = web_sys::window().unwrap();
    w.clear_interval_with_handle(timer);
    w.clear_interval_with_handle(timer2);
    // Stop: on_ended once, the browser track released.
    let stopped = Rc::new(Cell::new(0));
    let s = stopped.clone();
    track.on_ended(move || s.set(s.get() + 1));
    track.stop();
    track.stop();
    assert_eq!(stopped.get(), 1);
    assert!(track.ended());
    let live = rinch_rtc::testing::live_local_tracks();
    drop(track2);
    assert_eq!(rinch_rtc::testing::live_local_tracks(), live - 1);
    drop(track);
}

/// The 440 Hz bin against the spectrum's mean: the tone stands well above.
fn tone_ratio(analyser: &web_sys::AnalyserNode, rate: f32) -> f32 {
    let mut bins = vec![0f32; analyser.frequency_bin_count() as usize];
    analyser.get_float_frequency_data(&mut bins);
    let hz_per_bin = rate / analyser.fft_size() as f32;
    let at = (440.0 / hz_per_bin).round() as usize;
    let peak = bins[at.saturating_sub(2)..=at + 2]
        .iter()
        .cloned()
        .fold(f32::MIN, f32::max);
    let (lo, hi) = (at + 20, bins.len().min(at + 400));
    let mean = bins[lo..hi].iter().sum::<f32>() / (hi - lo) as f32;
    peak - mean
}

fn feed_tone(writer: AudioSourceWriter, rate: u32) -> (i32, Closure<dyn FnMut()>) {
    let phase = Cell::new(0f64);
    let cb = Closure::<dyn FnMut()>::new(move || {
        // 20 ms of tone per 20 ms tick.
        let n = rate as usize / 50;
        let mut out = Vec::with_capacity(n);
        let mut p = phase.get();
        for _ in 0..n {
            out.push((p.sin() * 0.5) as f32);
            p += 2.0 * std::f64::consts::PI * 440.0 / rate as f64;
        }
        phase.set(p % (2.0 * std::f64::consts::PI));
        writer.push(&out);
    });
    let id = web_sys::window()
        .unwrap()
        .set_interval_with_callback_and_timeout_and_arguments_0(cb.as_ref().unchecked_ref(), 20)
        .unwrap();
    (id, cb)
}

#[wasm_bindgen_test]
async fn audio_crosses_and_plays() {
    let pair = Pair::new();
    let (track, writer) = LocalTrack::audio_source(AudioSourceOptions {
        sample_rate: 48_000,
        channels: 1,
    })
    .unwrap();
    assert_eq!(track.kind(), MediaKind::Audio);
    let (timer, _feed) = feed_tone(writer, 48_000);
    let remotes = remote_slot(&pair.b);
    let errors: Rc<RefCell<Vec<String>>> = Rc::default();
    let e = errors.clone();
    pair.b
        .on_error(move |err| e.borrow_mut().push(err.to_string()));
    pair.a.add_track(&track).unwrap();
    pair.connect().await;
    until("b hears the track", || !remotes.borrow().is_empty()).await;
    let remote = remotes.borrow()[0].clone();
    assert_eq!(remote.kind(), MediaKind::Audio);
    // Remote audio plays by itself, through a hidden <audio>.
    assert_eq!(rinch_rtc::testing::remote_audio_elements(), 1);
    remote.set_volume(0.5);

    // The far side's audio, analysed: the tone is there.
    let ctx = web_sys::AudioContext::new().unwrap();
    let _ = ctx.resume();
    let mt = rinch_rtc::testing::remote_media_track(&remote).unwrap();
    let own: js_sys::Array = [JsValue::from(mt)].into_iter().collect();
    let stream = web_sys::MediaStream::new_with_tracks(&own).unwrap();
    let src = ctx.create_media_stream_source(&stream).unwrap();
    let analyser = ctx.create_analyser().unwrap();
    analyser.set_fft_size(4096);
    src.connect_with_audio_node(&analyser).unwrap();
    let rate = ctx.sample_rate();
    let mut best = f32::MIN;
    for _ in 0..100 {
        sleep(100).await;
        best = best.max(tone_ratio(&analyser, rate));
        if best > 30.0 {
            break;
        }
    }
    assert!(best > 30.0, "440 Hz stands {best} dB above the spectrum");
    assert!(errors.borrow().is_empty(), "{:?}", errors.borrow());

    // Close: the <audio> goes.
    pair.b.close();
    pair.a.close();
    until("remote audio element removed", || {
        rinch_rtc::testing::remote_audio_elements() == 0
    })
    .await;
    web_sys::window().unwrap().clear_interval_with_handle(timer);
    let _ = ctx.close();
}

#[wasm_bindgen_test]
async fn media_view_shows_the_remote_track() {
    let pair = Pair::new();
    let (track, writer) = LocalTrack::video_source(VideoSourceOptions {
        width: W,
        height: H,
        fps: 30,
    })
    .unwrap();
    let (timer, _feed) = moving_box(writer, Rc::new(Cell::new(BOX)));
    let remotes = remote_slot(&pair.b);
    // Receive-only on b's side, created before the offer arrives: the
    // offer's m-line pairs with it.
    pair.a.add_track(&track).unwrap();
    pair.connect().await;
    until("b hears the track", || !remotes.borrow().is_empty()).await;
    let remote = remotes.borrow()[0].clone();

    let doc = web_sys::window().unwrap().document().unwrap();
    let host = doc.create_element("div").unwrap();
    doc.body().unwrap().append_child(&host).unwrap();
    let r = remote.clone();
    let root = rinch_web::mount_into(&host, Default::default(), move |scope| {
        MediaView {
            track: r.into(),
            style: "width: 320px; height: 240px;".into(),
            ..Default::default()
        }
        .render(scope, &[])
    });
    let video: web_sys::HtmlVideoElement = {
        let mut found = None;
        until("the view's <video>", || {
            found = host
                .query_selector("video[data-rtc-view]")
                .ok()
                .flatten()
                .and_then(|e| e.dyn_into().ok());
            found.is_some()
        })
        .await;
        found.unwrap()
    };
    until("the view plays", || {
        video.src_object().is_some() && video.video_width() == W && !video.paused()
    })
    .await;
    assert!(
        video.muted(),
        "a remote video view is muted (audio has its own element)"
    );

    // Read it back through a canvas: the box is there.
    let canvas: web_sys::HtmlCanvasElement = doc.create_element("canvas").unwrap().unchecked_into();
    canvas.set_width(W);
    canvas.set_height(H);
    let ctx: web_sys::CanvasRenderingContext2d =
        canvas.get_context("2d").unwrap().unwrap().unchecked_into();
    let mut boxes = 0;
    for _ in 0..20 {
        sleep(50).await;
        ctx.draw_image_with_html_video_element(&video, 0.0, 0.0)
            .unwrap();
        let img = ctx.get_image_data(0.0, 0.0, W as f64, H as f64).unwrap();
        if is_box(measure(&img.data(), W, H), BOX) {
            boxes += 1;
        }
    }
    assert!(boxes >= 16, "{boxes} of 20 read-backs showed the box");

    root.unmount();
    host.remove();
    pair.a.close();
    pair.b.close();
    web_sys::window().unwrap().clear_interval_with_handle(timer);
}

#[wasm_bindgen_test]
async fn a_recvonly_transceiver_and_refusals() {
    let pair = Pair::new();
    // b asks to receive video with nothing to send.
    let rx = pair
        .b
        .add_transceiver(MediaKind::Video, Direction::RecvOnly)
        .unwrap();
    assert_eq!(rx.kind(), MediaKind::Video);
    assert_eq!(rx.direction(), Direction::RecvOnly);
    assert_eq!(pair.b.transceivers().len(), 1);

    // A stopped track cannot be added.
    let (track, _w) = LocalTrack::video_source(VideoSourceOptions::default()).unwrap();
    track.stop();
    assert!(pair.a.add_track(&track).is_err());

    // A dropped track releases its browser track; a stopped one already
    // has.
    let before = rinch_rtc::testing::live_local_tracks();
    let (t2, _w2) = LocalTrack::video_source(VideoSourceOptions::default()).unwrap();
    assert_eq!(rinch_rtc::testing::live_local_tracks(), before + 1);
    drop(t2);
    drop(track);
    assert_eq!(rinch_rtc::testing::live_local_tracks(), before);

    // A frame of the wrong size is ignored (no panic).
    let (t3, w3) = LocalTrack::video_source(VideoSourceOptions::default()).unwrap();
    w3.push_rgba(&[0; 7], 2, 2);
    drop(t3);
    pair.a.close();
    pair.b.close();
}

// ------------------------------------------------- #1933's review, adopted

fn count(sel: &str) -> u32 {
    web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .query_selector_all(sel)
        .unwrap()
        .length()
}

/// A transceiver that goes inactive and comes back makes the browser fire
/// `track` again for the same browser track: it is still one RemoteTrack,
/// with one `<audio>`, and that handle unmutes.
#[wasm_bindgen_test]
async fn a_track_that_comes_back_is_handed_over_once() {
    let pair = Pair::new();
    let (track, writer) = LocalTrack::audio_source(AudioSourceOptions::default()).unwrap();
    let (timer, _feed) = feed_tone(writer, 48_000);
    let remotes = remote_slot(&pair.b);
    pair.a.add_track(&track).unwrap();
    pair.connect().await;
    until("b hears the track", || !remotes.borrow().is_empty()).await;
    let first = remotes.borrow()[0].clone();
    until("first unmuted", || !first.muted()).await;
    let tx = pair.a.transceivers()[0].clone();
    tx.set_direction(Direction::Inactive);
    pair.negotiate().await;
    until("first mutes", || first.muted()).await;
    tx.set_direction(Direction::SendRecv);
    pair.negotiate().await;
    until("first unmutes again", || !first.muted()).await;
    sleep(500).await;
    assert_eq!(
        remotes.borrow().len(),
        1,
        "on_track fired again for the same track"
    );
    assert_eq!(
        rinch_rtc::testing::remote_audio_elements(),
        1,
        "a second <audio>"
    );
    pair.a.close();
    pair.b.close();
    web_sys::window().unwrap().clear_interval_with_handle(timer);
}

/// SendOnly and RecvOnly reach the far side: a sender turned RecvOnly stops
/// sending (the far track mutes), SendOnly sends again.
#[wasm_bindgen_test]
async fn send_only_and_recv_only_take_effect() {
    let pair = Pair::new();
    let (track, writer) = LocalTrack::video_source(VideoSourceOptions {
        width: W,
        height: H,
        fps: 30,
    })
    .unwrap();
    let (timer, _feed) = moving_box(writer, Rc::new(Cell::new(BOX)));
    let remotes = remote_slot(&pair.b);
    pair.a.add_track(&track).unwrap();
    pair.connect().await;
    until("b hears the track", || !remotes.borrow().is_empty()).await;
    let remote = remotes.borrow()[0].clone();
    until("unmuted", || !remote.muted()).await;
    let tx = pair.a.transceivers()[0].clone();
    tx.set_direction(Direction::RecvOnly);
    pair.negotiate().await;
    assert_eq!(tx.current_direction(), Some(Direction::Inactive));
    until("far track mutes", || remote.muted()).await;
    tx.set_direction(Direction::SendOnly);
    assert_eq!(tx.direction(), Direction::SendOnly);
    pair.negotiate().await;
    assert_eq!(tx.current_direction(), Some(Direction::SendOnly));
    until("far track unmutes", || !remote.muted()).await;
    assert_eq!(remotes.borrow().len(), 1);
    pair.a.close();
    pair.b.close();
    web_sys::window().unwrap().clear_interval_with_handle(timer);
}

/// Perfect negotiation: both sides add a track and offer at once. The
/// impolite side ignores the incoming offer (its own is in flight:
/// `signaling_state()` is not Stable); the polite side takes it, which
/// rolls its own offer back by itself, answers, and offers again.
#[wasm_bindgen_test]
async fn glare_resolves_with_perfect_negotiation() {
    use rinch_rtc::{Description, SignalingState};
    let pair = Pair::new();
    let _ch = pair.a.create_data_channel("x", Default::default()).unwrap();
    pair.connect().await;
    let (ta, _wa) = LocalTrack::video_source(VideoSourceOptions::default()).unwrap();
    let (tb, _wb) = LocalTrack::video_source(VideoSourceOptions::default()).unwrap();
    let (ra, rb) = (remote_slot(&pair.a), remote_slot(&pair.b));
    pair.a.add_track(&ta).unwrap();
    pair.b.add_track(&tb).unwrap();
    assert_eq!(pair.a.signaling_state(), SignalingState::Stable);
    let (oa, ob) = (pair.a.create_offer(), pair.b.create_offer());
    // An offer in flight already counts.
    assert_eq!(pair.a.signaling_state(), SignalingState::HaveLocalOffer);
    assert_eq!(pair.b.signaling_state(), SignalingState::HaveLocalOffer);
    let (oa, ob) = (oa.await.unwrap(), ob.await.unwrap());
    // Impolite a: b's offer collides; ignored.
    assert_ne!(pair.a.signaling_state(), SignalingState::Stable);
    drop(ob);
    // Polite b: takes a's offer over its own (implicit rollback).
    pair.b.set_remote_description(oa).await.unwrap();
    assert_eq!(pair.b.signaling_state(), SignalingState::HaveRemoteOffer);
    let answer = pair.b.create_answer().await.unwrap();
    assert_eq!(pair.b.signaling_state(), SignalingState::Stable);
    pair.a.set_remote_description(answer).await.unwrap();
    assert_eq!(pair.a.signaling_state(), SignalingState::Stable);
    pair.pump();
    until("b hears a's track", || !rb.borrow().is_empty()).await;
    // b's rolled-back track still needs sending: b offers again.
    let offer = pair.b.create_offer().await.unwrap();
    pair.a.set_remote_description(offer).await.unwrap();
    let answer = pair.a.create_answer().await.unwrap();
    pair.b.set_remote_description(answer).await.unwrap();
    pair.pump();
    until("a hears b's track", || !ra.borrow().is_empty()).await;

    // An explicit rollback of our own offer.
    let (tc, _wc) = LocalTrack::video_source(VideoSourceOptions::default()).unwrap();
    pair.a.add_track(&tc).unwrap();
    let _ = pair.a.create_offer().await.unwrap();
    assert_eq!(pair.a.signaling_state(), SignalingState::HaveLocalOffer);
    pair.a
        .set_remote_description(Description::rollback())
        .await
        .unwrap();
    assert_eq!(pair.a.signaling_state(), SignalingState::Stable);
    pair.a.close();
    pair.b.close();
}

/// negotiationneeded after connecting, for a second track; and recvonly
/// end to end: b, with nothing to send, offers a RecvOnly transceiver and
/// a answers carrying a track.
#[wasm_bindgen_test]
async fn renegotiation_and_recvonly_end_to_end() {
    let pair = Pair::new();
    web_sys::console::log_1(&"STEP 1".into());
    let rx = pair
        .b
        .add_transceiver(MediaKind::Video, Direction::RecvOnly)
        .unwrap();
    web_sys::console::log_1(&"STEP 2".into());
    let remotes = remote_slot(&pair.b);
    web_sys::console::log_1(&"STEP 3".into());
    let offer = pair.b.create_offer().await.unwrap();
    web_sys::console::log_1(&"STEP 4".into());
    pair.a.set_remote_description(offer).await.unwrap();
    web_sys::console::log_1(&"STEP 5".into());
    let (track, writer) = LocalTrack::video_source(VideoSourceOptions {
        width: W,
        height: H,
        fps: 30,
    })
    .unwrap();
    web_sys::console::log_1(&"STEP 6".into());
    let (timer, _feed) = moving_box(writer, Rc::new(Cell::new(BOX)));
    web_sys::console::log_1(&"STEP 7".into());
    pair.a.add_track(&track).unwrap();
    web_sys::console::log_1(&"STEP 8".into());
    let answer = pair.a.create_answer().await.unwrap();
    web_sys::console::log_1(&"STEP 9".into());
    pair.b.set_remote_description(answer).await.unwrap();
    web_sys::console::log_1(&"STEP 10".into());
    pair.pump();
    web_sys::console::log_1(&"STEP 11".into());
    until("connected", || {
        pair.pump();
    web_sys::console::log_1(&"STEP 12".into());
        pair.a.state() == PeerState::Connected && pair.b.state() == PeerState::Connected
    })
    .await;
    web_sys::console::log_1(&"STEP 13".into());
    until("b hears the track", || !remotes.borrow().is_empty()).await;
    web_sys::console::log_1(&"STEP 14".into());
    let remote = remotes.borrow()[0].clone();
    web_sys::console::log_1(&"STEP 15".into());
    let frames = record_frames(&remote);
    web_sys::console::log_1(&"STEP 16".into());
    let seen = fresh_frames(&frames, 500, 20).await;
    web_sys::console::log_1(&"STEP 17".into());
    assert!(share(&seen, BOX) >= 0.8, "{seen:?}");
    web_sys::console::log_1(&"STEP 18".into());
    assert_eq!(rx.current_direction(), Some(Direction::RecvOnly));
    web_sys::console::log_1(&"STEP 19".into());
    assert_eq!(remote.transceiver().unwrap().mid(), rx.mid());
    web_sys::console::log_1(&"STEP 20".into());

    // A second track after connecting: negotiationneeded, then it arrives.
    let n = Rc::new(Cell::new(0));
    web_sys::console::log_1(&"STEP 21".into());
    let m = n.clone();
    web_sys::console::log_1(&"STEP 22".into());
    pair.a.on_negotiation_needed(move || m.set(m.get() + 1));
    web_sys::console::log_1(&"STEP 23".into());
    let (t2, _w2) = LocalTrack::audio_source(AudioSourceOptions::default()).unwrap();
    web_sys::console::log_1(&"STEP 24".into());
    let s2 = pair.a.add_track(&t2).unwrap();
    web_sys::console::log_1(&"STEP 25".into());
    assert_eq!(s2.track().map(|t| t.id()), Some(t2.id()));
    web_sys::console::log_1(&"STEP 26".into());
    until("negotiationneeded after connect", || n.get() > 0).await;
    web_sys::console::log_1(&"STEP 27".into());
    let offer = pair.a.create_offer().await.unwrap();
    web_sys::console::log_1(&"STEP 28".into());
    pair.b.set_remote_description(offer).await.unwrap();
    web_sys::console::log_1(&"STEP 29".into());
    let answer = pair.b.create_answer().await.unwrap();
    web_sys::console::log_1(&"STEP 30".into());
    pair.a.set_remote_description(answer).await.unwrap();
    web_sys::console::log_1(&"STEP 31".into());
    until("b hears the second track", || remotes.borrow().len() == 2).await;
    web_sys::console::log_1(&"STEP 32".into());
    pair.a.close();
    web_sys::console::log_1(&"STEP 33".into());
    pair.b.close();
    web_sys::console::log_1(&"STEP 34".into());
    web_sys::window().unwrap().clear_interval_with_handle(timer);
    web_sys::console::log_1(&"STEP 35".into());
}

/// The far side stops its transceiver: the remote track ends through the
/// browser's `ended` event (not the peer-close path), and its `<audio>` goes.
#[wasm_bindgen_test]
async fn a_stopped_transceiver_ends_the_remote_track() {
    let pair = Pair::new();
    let (track, writer) = LocalTrack::audio_source(AudioSourceOptions::default()).unwrap();
    let (timer, _feed) = feed_tone(writer, 48_000);
    let remotes = remote_slot(&pair.b);
    pair.a.add_track(&track).unwrap();
    pair.connect().await;
    until("b hears the track", || !remotes.borrow().is_empty()).await;
    let remote = remotes.borrow()[0].clone();
    let ended = Rc::new(Cell::new(0));
    let e = ended.clone();
    remote.on_ended(move || e.set(e.get() + 1));
    let before = rinch_rtc::testing::remote_audio_elements();
    pair.a.transceivers()[0].stop();
    pair.negotiate().await;
    until("remote ended", || ended.get() == 1).await;
    assert!(remote.ended());
    assert_eq!(rinch_rtc::testing::remote_audio_elements(), before - 1);
    web_sys::window().unwrap().clear_interval_with_handle(timer);
    pair.a.close();
    pair.b.close();
}

/// Unmounting a MediaView lets go of the track (`srcObject` cleared).
#[wasm_bindgen_test]
async fn media_view_unmount_clears_the_source() {
    let (track, writer) = LocalTrack::video_source(VideoSourceOptions {
        width: W,
        height: H,
        fps: 30,
    })
    .unwrap();
    let (timer, _feed) = moving_box(writer, Rc::new(Cell::new(BOX)));
    let doc = web_sys::window().unwrap().document().unwrap();
    let host = doc.create_element("div").unwrap();
    doc.body().unwrap().append_child(&host).unwrap();
    let t = track.clone();
    let root = rinch_web::mount_into(&host, Default::default(), move |scope| {
        MediaView {
            track: t.into(),
            ..Default::default()
        }
        .render(scope, &[])
    });
    let mut found: Option<web_sys::HtmlVideoElement> = None;
    until("the view plays", || {
        found = host
            .query_selector("video[data-rtc-view]")
            .ok()
            .flatten()
            .and_then(|e| e.dyn_into().ok());
        found.as_ref().is_some_and(|v| v.src_object().is_some())
    })
    .await;
    let video = found.unwrap();
    root.unmount();
    assert!(video.src_object().is_none(), "srcObject kept after unmount");
    host.remove();
    web_sys::window().unwrap().clear_interval_with_handle(timer);
}

/// `on_frame`'s reader is freed when the peer closes: the loop's closure
/// (and the app's callback in it) does not hold itself for ever.
#[wasm_bindgen_test]
async fn on_frame_is_freed_when_the_peer_closes() {
    let pair = Pair::new();
    let (track, writer) = LocalTrack::video_source(VideoSourceOptions {
        width: W,
        height: H,
        fps: 30,
    })
    .unwrap();
    let (timer, _feed) = moving_box(writer, Rc::new(Cell::new(BOX)));
    let remotes = remote_slot(&pair.b);
    pair.a.add_track(&track).unwrap();
    pair.connect().await;
    until("b hears the track", || !remotes.borrow().is_empty()).await;
    let remote = remotes.borrow()[0].clone();
    let token = Arc::new(());
    let got = Arc::new(Mutex::new(0u32));
    let (tk, g) = (token.clone(), got.clone());
    remote.on_frame(move |_, _, _| {
        let _ = &tk;
        *g.lock().unwrap() += 1;
    });
    until("frames", || *got.lock().unwrap() > 5).await;
    pair.b.close();
    pair.a.close();
    until("the reader's closure freed", || {
        Arc::strong_count(&token) == 1
    })
    .await;
    web_sys::window().unwrap().clear_interval_with_handle(timer);
}

/// 50 rounds of connect, add audio and video, receive, replace, close and
/// drop leave no callback, local track or `<audio>` behind.
#[wasm_bindgen_test]
async fn fifty_rounds_leave_nothing_behind() {
    let token = Rc::new(());
    let base_locals = rinch_rtc::testing::live_local_tracks();
    let base_video = count("video");
    for _ in 0..50 {
        let pair = Pair::new();
        let (tv, wv) = LocalTrack::video_source(VideoSourceOptions {
            width: 64,
            height: 48,
            fps: 30,
        })
        .unwrap();
        let (ta, wa) = LocalTrack::audio_source(AudioSourceOptions::default()).unwrap();
        wv.push_rgba(&[255u8; 64 * 48 * 4], 64, 48);
        wa.push(&[0.1f32; 4800]);
        let t1 = token.clone();
        pair.a.on_negotiation_needed(move || {
            let _ = &t1;
        });
        let got = Rc::new(RefCell::new(Vec::<RemoteTrack>::new()));
        let (g, t2) = (got.clone(), token.clone());
        pair.b.on_track(move |r| {
            let t3 = t2.clone();
            r.on_ended(move || {
                let _ = &t3;
            });
            g.borrow_mut().push(r);
        });
        let t5 = token.clone();
        ta.on_ended(move || {
            let _ = &t5;
        });
        let s = pair.a.add_track(&tv).unwrap();
        pair.a.add_track(&ta).unwrap();
        pair.connect().await;
        until("both tracks", || got.borrow().len() == 2).await;
        s.replace_track(None).await.unwrap();
        assert!(s.track().is_none());
        pair.a.close();
        pair.b.close();
        tick().await;
        drop((got, pair, tv, ta, wv, wa));
    }
    sleep(500).await;
    assert_eq!(Rc::strong_count(&token), 1, "callbacks leaked");
    assert_eq!(rinch_rtc::testing::live_local_tracks(), base_locals);
    assert_eq!(rinch_rtc::testing::remote_audio_elements(), 0);
    assert_eq!(count("video"), base_video);
}

/// An app-fed audio source says whether the browser runs it. With this
/// crate's webdriver.json (autoplay allowed) it is Live; built with
/// RINCH_RTC_NO_AUTOPLAY=1 and run without the flag (a CI step, Chrome),
/// it is WaitingForGesture, not silently live.
#[wasm_bindgen_test]
async fn audio_source_status_says_whether_it_runs() {
    use rinch_rtc::media::LocalStatus;
    let no_autoplay = option_env!("RINCH_RTC_NO_AUTOPLAY").is_some();
    let (track, writer) = LocalTrack::audio_source(AudioSourceOptions::default()).unwrap();
    let (timer, _feed) = feed_tone(writer.clone(), 48_000);
    let mut status = track.status();
    for _ in 0..50 {
        sleep(100).await;
        status = track.status();
        if status == LocalStatus::Live && !no_autoplay {
            break;
        }
    }
    let taken = rinch_rtc::testing::audio_samples_taken(&writer);
    if no_autoplay {
        assert_eq!(status, LocalStatus::WaitingForGesture);
    } else {
        assert_eq!(status, LocalStatus::Live);
        assert!(taken > 0, "a running source takes samples");
    }
    web_sys::window().unwrap().clear_interval_with_handle(timer);
    track.stop();
    assert_eq!(track.status(), LocalStatus::Ended);
}

/// The writers keep up: a source takes what is pushed (video: most frames
/// at 30 fps; audio: every sample, within the one-second cap), and they
/// see their track end.
#[wasm_bindgen_test]
async fn source_writers_keep_up_and_see_the_end() {
    let (vt, vw) = LocalTrack::video_source(VideoSourceOptions {
        width: 64,
        height: 48,
        fps: 30,
    })
    .unwrap();
    let (at, aw) = LocalTrack::audio_source(AudioSourceOptions::default()).unwrap();
    let pushes = Rc::new(Cell::new(0u64));
    let p = pushes.clone();
    let vw2 = vw.clone();
    let cb = Closure::<dyn FnMut()>::new(move || {
        p.set(p.get() + 1);
        vw2.push_rgba(&[200u8; 64 * 48 * 4], 64, 48);
    });
    let vtimer = web_sys::window()
        .unwrap()
        .set_interval_with_callback_and_timeout_and_arguments_0(cb.as_ref().unchecked_ref(), 33)
        .unwrap();
    let (atimer, _feed) = feed_tone(aw.clone(), 48_000);
    sleep(3000).await;
    let (pushed, painted) = (pushes.get(), rinch_rtc::testing::video_frames_taken(&vw));
    let taken = rinch_rtc::testing::audio_samples_taken(&aw);
    let w = web_sys::window().unwrap();
    w.clear_interval_with_handle(vtimer);
    w.clear_interval_with_handle(atimer);
    assert!(
        painted * 10 >= pushed * 7,
        "painted {painted} of {pushed} frames"
    );
    // 3 s of 48 kHz mono were pushed; all but the queue (<= 1 s) is taken.
    assert!(taken >= 48_000 * 2, "took {taken} samples in 3 s");
    assert!(!vw.is_ended() && !aw.is_ended());
    vt.stop();
    assert!(vw.is_ended());
    drop(at);
    assert!(aw.is_ended(), "dropping the track ends it for the writer");
}

/// A camera request whose future is dropped while the prompt is open (its
/// component unmounted): the track the browser grants later is stopped.
/// getUserMedia is a stand-in that answers with a canvas stream; no device.
#[wasm_bindgen_test]
async fn a_dropped_camera_request_stops_the_late_track() {
    use rinch_rtc::media::CameraOptions;
    js_sys::eval(
        "window.__gum = []; \
         navigator.mediaDevices.getUserMedia = () => new Promise(r => window.__gum.push(r));",
    )
    .unwrap();
    let grant = || {
        js_sys::eval(
            "(() => { const c = document.createElement('canvas'); c.width = 8; c.height = 8; \
              c.getContext('2d').fillRect(0, 0, 8, 8); const s = c.captureStream(5); \
              const t = s.getVideoTracks()[0]; window.__gum.shift()(s); return t; })()",
        )
        .unwrap()
        .unchecked_into::<web_sys::MediaStreamTrack>()
    };
    // Dropped before the grant.
    let pending = LocalTrack::camera(CameraOptions::default());
    drop(pending);
    // The request is made from its own task: let it run (the prompt opens).
    sleep(10).await;
    let late = grant();
    sleep(100).await;
    assert_eq!(late.ready_state(), web_sys::MediaStreamTrackState::Ended);
    // Positive control: awaited, the granted track is kept and live.
    let asked = LocalTrack::camera(CameraOptions::default());
    sleep(10).await;
    let kept = grant();
    let track = asked.await.unwrap();
    assert_eq!(kept.ready_state(), web_sys::MediaStreamTrackState::Live);
    drop(track);
    assert_eq!(kept.ready_state(), web_sys::MediaStreamTrackState::Ended);
}
