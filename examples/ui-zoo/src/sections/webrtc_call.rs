//! The call behind the WebRTC section: two rinch-rtc peers in one window,
//! with no server (design record `docs/design/rtc-media.md` 5.2).
//!
//! - Peer A ("you") sends a generated test pattern and a 440 Hz tone
//!   (`LocalTrack::video_source` / `audio_source`). [`InPageCall::use_camera`]
//!   swaps in the camera and the microphone with `Sender::replace_track`, and
//!   [`InPageCall::use_pattern`] swaps back and releases them. Nothing here
//!   opens a device except `use_camera`.
//! - Peer B ("the far side") sends a pattern of its own back, so both peers
//!   receive.
//! - Signalling, which an app carries over its own socket, is a direct
//!   hand-off between the two peers.
//!
//! The logic is kept apart from the section's rsx so that
//! `tests/webrtc_in_page.rs` can run it without a window. The section makes
//! the call while it renders, so the peers, the timers feeding the sources
//! and every callback are the section's: they stop when it unmounts.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::{Signal, set_interval, spawn_local};
use rinch_rtc::media::{
    AudioSourceOptions, CameraOptions, LocalTrack, MediaKind, MediaTrack, MicrophoneOptions,
    RemoteTrack, Sender, VideoSourceOptions,
};
use rinch_rtc::{Peer, PeerState, RtcConfig, RtcError};

/// The generated pictures' size.
pub const W: u32 = 320;
pub const H: u32 = 240;
/// The moving box's side, in pixels.
pub const BOX: u32 = 64;

/// What peer A is sending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The generated pattern and tone.
    Pattern,
    /// Waiting for the camera and microphone (a permission prompt, on the web).
    Requesting,
    Camera,
}

/// One video to show, for a `for` over a `Vec<Shown>`: equal (and keyed)
/// by the track's id, so a new track renders a new `MediaView` (its track is
/// fixed for life) and the same one is left alone.
#[derive(Clone)]
pub struct Shown(pub MediaTrack);

impl Shown {
    pub fn id(&self) -> String {
        match &self.0 {
            MediaTrack::None => String::new(),
            MediaTrack::Local(t) => t.id(),
            MediaTrack::Remote(t) => t.id(),
        }
    }
}

impl PartialEq for Shown {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl std::fmt::Debug for Shown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.id())
    }
}

/// The call's state, for the section to show.
#[derive(Clone, Copy)]
pub struct CallView {
    pub state_a: Signal<PeerState>,
    pub state_b: Signal<PeerState>,
    pub source: Signal<Source>,
    /// What A sends (its self-view).
    pub local: Signal<Vec<Shown>>,
    /// A's video, as B receives it.
    pub at_b: Signal<Vec<Shown>>,
    /// B's video, as A receives it.
    pub at_a: Signal<Vec<Shown>>,
    pub muted: Signal<bool>,
    pub video_off: Signal<bool>,
    /// Whether B plays the audio it receives (off at first: a tone that
    /// starts by itself would be unwelcome).
    pub hear: Signal<bool>,
    pub error: Signal<String>,
}

struct Inner {
    a: Peer,
    b: Peer,
    pattern: LocalTrack,
    tone: LocalTrack,
    /// B's own pattern. Kept so that it outlives the call's setup.
    _back: LocalTrack,
    video: Sender,
    audio: Sender,
    /// The camera and microphone while they are in use.
    camera: RefCell<Option<(LocalTrack, LocalTrack)>>,
    /// The audio B receives from A.
    b_audio: RefCell<Option<RemoteTrack>>,
    view: CallView,
}

/// A running in-page call. Cheap to clone. The peers close when the last
/// clone drops, or on [`InPageCall::close`].
#[derive(Clone)]
pub struct InPageCall(Rc<Inner>);

/// One frame of a white box moving over a dark field tinted `tint`: frame
/// `k` puts the box at `box_at(k)`.
pub fn pattern_frame(k: u32, tint: [u8; 3]) -> Vec<u8> {
    let mut f = Vec::with_capacity((W * H * 4) as usize);
    for _ in 0..W * H {
        f.extend_from_slice(&[tint[0], tint[1], tint[2], 255]);
    }
    let (x, y) = box_at(k);
    for row in y..y + BOX {
        for col in x..x + BOX {
            let i = ((row * W + col) * 4) as usize;
            f[i..i + 3].copy_from_slice(&[255, 255, 255]);
        }
    }
    f
}

/// Where the box is in frame `k` (its top-left corner).
pub fn box_at(k: u32) -> (u32, u32) {
    ((k * 4) % (W - BOX), (k * 3) % (H - BOX))
}

/// A's field is dark blue, B's dark green: both stay far below the box's
/// brightness, so a receiver can find the box.
const TINT_A: [u8; 3] = [16, 24, 64];
const TINT_B: [u8; 3] = [16, 56, 24];

impl InPageCall {
    /// Make both peers, their sources and tracks, and start negotiating.
    /// Opens no device.
    pub fn start() -> Result<InPageCall, String> {
        let view = CallView {
            state_a: Signal::new(PeerState::New),
            state_b: Signal::new(PeerState::New),
            source: Signal::new(Source::Pattern),
            local: Signal::new(Vec::new()),
            at_b: Signal::new(Vec::new()),
            at_a: Signal::new(Vec::new()),
            muted: Signal::new(false),
            video_off: Signal::new(false),
            hear: Signal::new(false),
            error: Signal::new(String::new()),
        };
        let a = Peer::new(RtcConfig::default()).map_err(|e| e.to_string())?;
        let b = Peer::new(RtcConfig::default()).map_err(|e| e.to_string())?;
        a.on_connection_state(move |s| view.state_a.set(s));
        b.on_connection_state(move |s| view.state_b.set(s));
        for p in [&a, &b] {
            p.on_error(move |e| view.error.set(e.to_string()));
        }

        // Trickle each side's candidates straight to the other. Weak
        // handles: the call owns the peers.
        let wb = b.downgrade();
        a.on_ice_candidate(move |c| {
            if let Some(b) = wb.upgrade() {
                b.add_ice_candidate(c)
            }
        });
        let wa = a.downgrade();
        b.on_ice_candidate(move |c| {
            if let Some(a) = wa.upgrade() {
                a.add_ice_candidate(c)
            }
        });

        let video_opts = VideoSourceOptions {
            width: W,
            height: H,
            fps: 30,
        };
        let (pattern, pattern_w) =
            LocalTrack::video_source(video_opts).map_err(|e| e.to_string())?;
        let (back, back_w) = LocalTrack::video_source(video_opts).map_err(|e| e.to_string())?;
        let (tone, tone_w) = LocalTrack::audio_source(AudioSourceOptions {
            sample_rate: 48_000,
            channels: 1,
        })
        .map_err(|e| e.to_string())?;

        let video = a.add_track(&pattern).map_err(|e| e.to_string())?;
        let audio = a.add_track(&tone).map_err(|e| e.to_string())?;
        b.add_track(&back).map_err(|e| e.to_string())?;
        view.local.set(vec![Shown(pattern.clone().into())]);

        // Feed the sources from the UI thread: the writers take frames from
        // any thread, and a timer works the same on desktop and web.
        let mut k = 0u32;
        set_interval(33, move || {
            k = k.wrapping_add(1);
            pattern_w.push_rgba(&pattern_frame(k, TINT_A), W, H);
            back_w.push_rgba(&pattern_frame(k.wrapping_mul(2), TINT_B), W, H);
        });
        let mut phase = 0f32;
        set_interval(20, move || {
            let step = 2.0 * std::f32::consts::PI * 440.0 / 48_000.0;
            let chunk: Vec<f32> = (0..960)
                .map(|_| {
                    phase = (phase + step) % (2.0 * std::f32::consts::PI);
                    phase.sin() * 0.2
                })
                .collect();
            tone_w.push(&chunk);
        });

        let call = InPageCall(Rc::new(Inner {
            a: a.clone(),
            b: b.clone(),
            pattern,
            tone,
            _back: back,
            video,
            audio,
            camera: RefCell::new(None),
            b_audio: RefCell::new(None),
            view,
        }));

        let weak = Rc::downgrade(&call.0);
        b.on_track(move |remote| match remote.kind() {
            MediaKind::Video => view.at_b.set(vec![Shown(remote.into())]),
            MediaKind::Audio => {
                remote.set_playing(view.hear.get());
                if let Some(inner) = weak.upgrade() {
                    *inner.b_audio.borrow_mut() = Some(remote);
                }
            }
        });
        a.on_track(move |remote| {
            if remote.kind() == MediaKind::Video {
                view.at_a.set(vec![Shown(remote.into())]);
            }
        });

        spawn_local(async move {
            if let Err(e) = negotiate(&a, &b).await {
                view.error.set(format!("negotiation: {e}"));
            }
        });
        Ok(call)
    }

    pub fn view(&self) -> CallView {
        self.0.view
    }

    pub fn peers(&self) -> (Peer, Peer) {
        (self.0.a.clone(), self.0.b.clone())
    }

    /// What A's video sender carries now.
    pub fn sending(&self) -> Option<LocalTrack> {
        self.0.video.track()
    }

    /// Send the camera and microphone instead of the pattern and tone. This
    /// asks for both devices (a permission prompt on the web); the call goes
    /// on with the pattern if either is refused.
    pub fn use_camera(&self) {
        let view = self.0.view;
        if view.source.get() != Source::Pattern {
            return;
        }
        view.source.set(Source::Requesting);
        let call = self.clone();
        spawn_local(async move {
            let got = async {
                let camera = LocalTrack::camera(CameraOptions::default()).await?;
                let mic = match LocalTrack::microphone(MicrophoneOptions::default()).await {
                    Ok(mic) => mic,
                    Err(e) => {
                        camera.stop();
                        return Err(e);
                    }
                };
                Ok((camera, mic))
            }
            .await;
            match got {
                Ok((camera, mic)) => {
                    if call.swap(Some((camera, mic))).await.is_ok() {
                        view.source.set(Source::Camera);
                    }
                }
                Err(e) => {
                    view.error.set(format!("camera: {e}"));
                    view.source.set(Source::Pattern);
                }
            }
        });
    }

    /// Back to the pattern and tone; the camera and microphone are released.
    pub fn use_pattern(&self) {
        if self.0.view.source.get() != Source::Camera {
            return;
        }
        let call = self.clone();
        spawn_local(async move {
            if call.swap(None).await.is_ok() {
                call.0.view.source.set(Source::Pattern);
            }
        });
    }

    /// Put `devices` (or the generated sources) on A's senders, without
    /// renegotiating, and release whatever devices they replace.
    async fn swap(&self, devices: Option<(LocalTrack, LocalTrack)>) -> Result<(), RtcError> {
        let inner = &self.0;
        let (video, audio) = match &devices {
            Some((camera, mic)) => (camera.clone(), mic.clone()),
            None => (inner.pattern.clone(), inner.tone.clone()),
        };
        // Mute and video-off carry over to the new tracks.
        video.set_enabled(!inner.view.video_off.get());
        audio.set_enabled(!inner.view.muted.get());
        let result = async {
            inner.video.replace_track(Some(&video)).await?;
            inner.audio.replace_track(Some(&audio)).await
        }
        .await;
        if let Err(e) = &result {
            inner.view.error.set(format!("replace_track: {e}"));
        }
        let old = inner.camera.replace(devices);
        if let Some((camera, mic)) = old {
            camera.stop();
            mic.stop();
        }
        inner.view.local.set(vec![Shown(video.into())]);
        result
    }

    /// Mute A's audio: silence is sent, and B sees `on_mute`.
    pub fn set_muted(&self, muted: bool) {
        self.0.view.muted.set(muted);
        if let Some(t) = self.0.audio.track() {
            t.set_enabled(!muted);
        }
    }

    /// Turn A's video off: black frames are sent.
    pub fn set_video_off(&self, off: bool) {
        self.0.view.video_off.set(off);
        if let Some(t) = self.0.video.track() {
            t.set_enabled(!off);
        }
    }

    /// Whether B plays the audio it receives.
    pub fn set_hear(&self, hear: bool) {
        self.0.view.hear.set(hear);
        if let Some(t) = self.0.b_audio.borrow().as_ref() {
            t.set_playing(hear);
        }
    }

    /// Close both peers and release any device.
    pub fn close(&self) {
        self.0.a.close();
        self.0.b.close();
        if let Some((camera, mic)) = self.0.camera.borrow_mut().take() {
            camera.stop();
            mic.stop();
        }
    }
}

/// Offer from A, answer from B; then, if B's own track found no m-line in
/// A's offer, the same the other way round.
async fn negotiate(a: &Peer, b: &Peer) -> Result<(), RtcError> {
    exchange(a, b).await?;
    if b.transceivers().iter().any(|t| t.mid().is_none()) {
        exchange(b, a).await?;
    }
    Ok(())
}

async fn exchange(offerer: &Peer, answerer: &Peer) -> Result<(), RtcError> {
    let offer = offerer.create_offer().await?;
    answerer.set_remote_description(offer).await?;
    let answer = answerer.create_answer().await?;
    offerer.set_remote_description(answer).await
}
