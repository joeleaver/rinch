//! Audio and video tracks (the `media` feature). Design record:
//! `docs/design/rtc-media.md` (issue #1916, decided 2026-10-26).
//!
//! The model is the browser's, simplified:
//!
//! - A [`LocalTrack`] is something this app sends: the camera, the
//!   microphone, or a source the app feeds itself ([`LocalTrack::video_source`],
//!   [`LocalTrack::audio_source`]).
//! - [`Peer::add_track`](crate::Peer::add_track) sends one, through a
//!   [`Sender`] (`replace_track` swaps it without renegotiating).
//!   [`Peer::add_transceiver`](crate::Peer::add_transceiver) asks to
//!   receive without sending.
//! - [`Peer::on_track`](crate::Peer::on_track) hands over each
//!   [`RemoteTrack`] that arrives. Remote audio plays by itself; remote
//!   video is shown with [`MediaView`].
//! - Mute is [`LocalTrack::set_enabled`]`(false)`: black frames or silence,
//!   and the far side sees `on_mute`.
//!
//! Threading, callbacks and lifetime are the data-channel rules: call on
//! the UI thread; callbacks run there, in a batch, and are dropped when the
//! component that registered them unmounts.

mod view;

#[cfg(all(feature = "media-cisco-openh264", not(target_arch = "wasm32")))]
pub use crate::native_media::load_openh264;
pub use view::{MediaTrack, MediaView};

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::registry::{self, Slot, with_peer};
use crate::{Peer, RtcError, next_id};

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use crate::native_media as backend;
#[cfg(target_arch = "wasm32")]
pub(crate) use crate::wasm_media as backend;

// ------------------------------------------------------------------ types

/// Audio or video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaKind {
    Audio,
    Video,
}

/// A transceiver's direction (`RTCRtpTransceiverDirection`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    SendRecv,
    SendOnly,
    RecvOnly,
    Inactive,
}

/// Why a media call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MediaError {
    /// The user, or the browser's policy, refused the device.
    PermissionDenied,
    /// No such device.
    DeviceNotFound(String),
    /// The device is busy elsewhere.
    DeviceInUse,
    /// Not available here: no camera API, a codec missing, autoplay refused.
    NotSupported(String),
    /// The stack failed underneath.
    Backend(String),
}

impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaError::PermissionDenied => write!(f, "permission denied"),
            MediaError::DeviceNotFound(m) => write!(f, "device not found: {m}"),
            MediaError::DeviceInUse => write!(f, "device in use"),
            MediaError::NotSupported(m) => write!(f, "not supported: {m}"),
            MediaError::Backend(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for MediaError {}

/// A device a picker can offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
}

/// Options for [`LocalTrack::camera`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraOptions {
    /// `None`: the default camera.
    pub device: Option<String>,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

impl Default for CameraOptions {
    fn default() -> Self {
        Self {
            device: None,
            width: 640,
            height: 480,
            fps: 30,
        }
    }
}

/// Options for [`LocalTrack::microphone`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MicrophoneOptions {
    /// `None`: the default microphone.
    pub device: Option<String>,
}

/// Options for [`LocalTrack::video_source`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoSourceOptions {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

impl Default for VideoSourceOptions {
    fn default() -> Self {
        Self {
            width: 640,
            height: 480,
            fps: 30,
        }
    }
}

/// Options for [`LocalTrack::audio_source`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioSourceOptions {
    pub sample_rate: u32,
    pub channels: u16,
}

impl Default for AudioSourceOptions {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 1,
        }
    }
}

/// A remote video track's frame callback (`RemoteTrack::on_frame`).
pub(crate) type FrameCallback = Box<dyn FnMut(&[u8], u32, u32) + Send>;

// --------------------------------------------------------- source writers

/// The latest frame an app-fed video source holds.
pub(crate) struct VideoSlot {
    pub(crate) latest: Mutex<Option<(Vec<u8>, u32, u32)>>,
    /// Bumped by every push, so a reader can tell a new frame.
    pub(crate) seq: AtomicU64,
    /// Set when the track ends (stopped or dropped).
    pub(crate) ended: Arc<AtomicBool>,
    /// Frames the backend took into the track (test hook).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) taken: AtomicU64,
}

/// Feeds a [`LocalTrack::video_source`]. `Send + Sync`: push from any thread.
/// Only the latest frame is kept.
#[derive(Clone)]
pub struct VideoSourceWriter {
    pub(crate) slot: Arc<VideoSlot>,
}

impl VideoSourceWriter {
    pub(crate) fn new() -> Self {
        Self {
            slot: Arc::new(VideoSlot {
                latest: Mutex::new(None),
                seq: AtomicU64::new(0),
                ended: Arc::new(AtomicBool::new(false)),
                taken: AtomicU64::new(0),
            }),
        }
    }

    /// Hand over a frame: RGBA8, `width * height * 4` bytes, rows tightly
    /// packed. A frame of the wrong length is ignored.
    pub fn push_rgba(&self, rgba: &[u8], width: u32, height: u32) {
        if rgba.len() != (width as usize) * (height as usize) * 4 || width == 0 || height == 0 {
            tracing::warn!("rinch-rtc: video_source frame of the wrong size ignored");
            return;
        }
        if let Ok(mut l) = self.slot.latest.lock() {
            match l.as_mut() {
                Some((buf, w, h)) if buf.len() == rgba.len() => {
                    buf.copy_from_slice(rgba);
                    (*w, *h) = (width, height);
                }
                _ => *l = Some((rgba.to_vec(), width, height)),
            }
        }
        self.slot.seq.fetch_add(1, Ordering::Release);
    }

    /// The track ended (stopped, dropped, or its peer's sender let go of
    /// nothing: the *track* is what counts). Stop producing frames.
    pub fn is_ended(&self) -> bool {
        self.slot.ended.load(Ordering::Acquire)
    }
}

/// Samples queued by an app-fed audio source; at most one second is kept
/// (the oldest go first).
pub(crate) struct AudioQueue {
    pub(crate) samples: Mutex<VecDeque<f32>>,
    pub(crate) sample_rate: u32,
    pub(crate) channels: u16,
    /// Set when the track ends (stopped or dropped).
    pub(crate) ended: Arc<AtomicBool>,
    /// Samples the backend took off the queue (test hook).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) taken: AtomicU64,
}

/// Feeds a [`LocalTrack::audio_source`]. `Send + Sync`.
#[derive(Clone)]
pub struct AudioSourceWriter {
    pub(crate) queue: Arc<AudioQueue>,
}

impl AudioSourceWriter {
    pub(crate) fn new(options: AudioSourceOptions) -> Self {
        Self {
            queue: Arc::new(AudioQueue {
                samples: Mutex::new(VecDeque::new()),
                sample_rate: options.sample_rate.max(1),
                channels: options.channels.max(1),
                ended: Arc::new(AtomicBool::new(false)),
                taken: AtomicU64::new(0),
            }),
        }
    }

    /// Queue interleaved `f32` samples in [-1, 1], at the source's rate and
    /// channel count.
    pub fn push(&self, interleaved: &[f32]) {
        let cap = self.queue.sample_rate as usize * self.queue.channels as usize;
        if let Ok(mut q) = self.queue.samples.lock() {
            q.extend(interleaved.iter().copied());
            while q.len() > cap {
                q.pop_front();
            }
        }
    }

    /// The track ended: stop producing samples.
    pub fn is_ended(&self) -> bool {
        self.queue.ended.load(Ordering::Acquire)
    }
}

// ------------------------------------------------------------ local tracks

struct LocalEntry {
    kind: MediaKind,
    id: String,
    enabled: bool,
    ended: bool,
    on_ended: Option<Slot<()>>,
    /// An app-fed source's writer: told when the track ends.
    writer_ended: Option<Arc<AtomicBool>>,
}

thread_local! {
    static LOCALS: RefCell<HashMap<u64, LocalEntry>> = RefCell::new(HashMap::new());
}

pub(crate) struct LocalInner {
    key: u64,
}

impl Drop for LocalInner {
    fn drop(&mut self) {
        backend::stop_local(self.key);
        // `try_with`: see `stop_local` (a track dropped during thread exit).
        let gone = LOCALS
            .try_with(|l| l.borrow_mut().remove(&self.key))
            .ok()
            .flatten();
        if let Some(w) = gone.as_ref().and_then(|e| e.writer_ended.as_ref()) {
            w.store(true, Ordering::Release);
        }
        drop(gone);
    }
}

fn set_writer_ended(key: u64, flag: &Arc<AtomicBool>) {
    LOCALS.with(|l| {
        if let Some(e) = l.borrow_mut().get_mut(&key) {
            e.writer_ended = Some(flag.clone());
        }
    });
}

/// What a [`LocalTrack`] is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalStatus {
    /// Sending (or ready to, once added to a peer).
    Live,
    /// Web: the browser holds the source until the user interacts with the
    /// page (the autoplay policy). An `audio_source` made outside a click
    /// sends silence until then; the page's next click, tap or key press
    /// resumes it. Make sources from a click handler to never see this.
    WaitingForGesture,
    /// Stopped, or its device went away.
    Ended,
}

/// Something this app sends: a camera, a microphone, or an app-fed source.
/// Cheap to clone; `!Send`. The device is released by [`stop`](Self::stop)
/// or when the last clone drops.
#[derive(Clone)]
pub struct LocalTrack {
    inner: Rc<LocalInner>,
}

impl std::fmt::Debug for LocalTrack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalTrack")
            .field("kind", &self.kind())
            .field("id", &self.id())
            .finish()
    }
}

fn new_local(kind: MediaKind) -> LocalTrack {
    let key = next_id();
    LOCALS.with(|l| {
        l.borrow_mut().insert(
            key,
            LocalEntry {
                kind,
                id: format!(
                    "rinch-{}-{key}",
                    if kind == MediaKind::Audio {
                        "audio"
                    } else {
                        "video"
                    }
                ),
                enabled: true,
                ended: false,
                on_ended: None,
                writer_ended: None,
            },
        )
    });
    LocalTrack {
        inner: Rc::new(LocalInner { key }),
    }
}

impl LocalTrack {
    pub(crate) fn key(&self) -> u64 {
        self.inner.key
    }

    /// The camera. On the web it asks the user's permission
    /// (`getUserMedia`); on desktop it is rinch-av's camera.
    pub fn camera(options: CameraOptions) -> impl Future<Output = Result<LocalTrack, MediaError>> {
        let track = new_local(MediaKind::Video);
        let opened = backend::open_camera(track.key(), options);
        async move {
            opened.await?;
            Ok(track)
        }
    }

    /// The microphone. On the web it asks the user's permission; on desktop
    /// it is rinch-av's microphone.
    pub fn microphone(
        options: MicrophoneOptions,
    ) -> impl Future<Output = Result<LocalTrack, MediaError>> {
        let track = new_local(MediaKind::Audio);
        let opened = backend::open_microphone(track.key(), options);
        async move {
            opened.await?;
            Ok(track)
        }
    }

    /// A video track the app feeds: a game view, a test pattern, a
    /// processed camera. Push RGBA8 frames through the writer, from any
    /// thread.
    pub fn video_source(
        options: VideoSourceOptions,
    ) -> Result<(LocalTrack, VideoSourceWriter), MediaError> {
        let track = new_local(MediaKind::Video);
        let writer = VideoSourceWriter::new();
        set_writer_ended(track.key(), &writer.slot.ended);
        backend::open_video_source(track.key(), options, writer.clone())?;
        Ok((track, writer))
    }

    /// An audio track the app feeds: interleaved `f32` at the given rate
    /// and channel count.
    pub fn audio_source(
        options: AudioSourceOptions,
    ) -> Result<(LocalTrack, AudioSourceWriter), MediaError> {
        let track = new_local(MediaKind::Audio);
        let writer = AudioSourceWriter::new(options);
        set_writer_ended(track.key(), &writer.queue.ended);
        backend::open_audio_source(track.key(), options, writer.clone())?;
        Ok((track, writer))
    }

    pub fn kind(&self) -> MediaKind {
        LOCALS
            .with(|l| l.borrow().get(&self.key()).map(|e| e.kind))
            .unwrap_or(MediaKind::Video)
    }

    /// A stable id for this track (the sender's track id on the wire).
    pub fn id(&self) -> String {
        LOCALS
            .with(|l| l.borrow().get(&self.key()).map(|e| e.id.clone()))
            .unwrap_or_default()
    }

    /// `false`: video sends black frames and audio silence, and the far
    /// side sees `on_mute`. The device stays open. The browser's
    /// `track.enabled`.
    pub fn set_enabled(&self, enabled: bool) {
        let changed = LOCALS.with(|l| {
            l.borrow_mut()
                .get_mut(&self.key())
                .is_some_and(|e| !e.ended && std::mem::replace(&mut e.enabled, enabled) != enabled)
        });
        if changed {
            backend::set_local_enabled(self.key(), enabled);
        }
    }

    pub fn enabled(&self) -> bool {
        LOCALS
            .with(|l| l.borrow().get(&self.key()).map(|e| e.enabled))
            .unwrap_or(false)
    }

    /// Live, held by the browser until a gesture, or ended.
    pub fn status(&self) -> LocalStatus {
        if self.ended() {
            LocalStatus::Ended
        } else if backend::waiting_for_gesture(self.key()) {
            LocalStatus::WaitingForGesture
        } else {
            LocalStatus::Live
        }
    }

    /// Whether the track has ended (stopped, or its device went away).
    pub fn ended(&self) -> bool {
        LOCALS
            .with(|l| l.borrow().get(&self.key()).map(|e| e.ended))
            .unwrap_or(true)
    }

    /// End the track: the device is released, and every sender carrying it
    /// sends nothing more. `on_ended` fires.
    pub fn stop(&self) {
        backend::stop_local(self.key());
        local_ended(self.key());
    }

    /// The track ended: [`stop`](Self::stop), or the device went away.
    pub fn on_ended(&self, cb: impl FnMut() + 'static) -> &Self {
        let mut cb = cb;
        let slot = Slot::new(Box::new(move |()| cb()));
        let _old = LOCALS.with(|l| {
            l.borrow_mut()
                .get_mut(&self.key())
                .map(|e| e.on_ended.replace(slot))
        });
        self
    }
}

/// Local track `key`'s id (the sender's track id on the wire).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn local_id(key: u64) -> String {
    LOCALS
        .with(|l| l.borrow().get(&key).map(|e| e.id.clone()))
        .unwrap_or_else(|| format!("rinch-{key}"))
}

/// Whether local track `key` still has a handle (a device that opened
/// after its track was dropped is closed again).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn local_alive(key: u64) -> bool {
    LOCALS.with(|l| l.borrow().contains_key(&key))
}

/// Mark a local track ended and fire its `on_ended`, once.
pub(crate) fn local_ended(key: u64) {
    let slot = LOCALS.with(|l| {
        l.borrow_mut().get_mut(&key).and_then(|e| {
            if e.ended {
                None
            } else {
                e.ended = true;
                if let Some(w) = &e.writer_ended {
                    w.store(true, Ordering::Release);
                }
                e.on_ended.take()
            }
        })
    });
    if let Some(slot) = slot {
        let back = registry::run_slot(slot, ());
        drop(back);
    }
}

// ------------------------------------------------------- per-peer state

/// What a peer holds for media (inside its registry entry).
#[derive(Default)]
pub(crate) struct PeerMedia {
    pub(crate) remotes: HashMap<u64, RemoteEntry>,
    /// What each of our senders carries (`Sender::track`). Weak: a sender
    /// does not keep a track alive.
    pub(crate) sending: HashMap<u64, Weak<LocalInner>>,
    pub(crate) on_track: Option<Slot<RemoteTrack>>,
    pub(crate) on_negotiation_needed: Option<Slot<()>>,
}

pub(crate) struct RemoteEntry {
    kind: MediaKind,
    id: String,
    stream_ids: Vec<String>,
    transceiver: u64,
    muted: bool,
    ended: bool,
    pub(crate) on_mute: Option<Slot<()>>,
    pub(crate) on_unmute: Option<Slot<()>>,
    pub(crate) on_ended: Option<Slot<()>>,
}

/// What a backend reports about media on a peer. (Only the web backend
/// reports anything until the native one lands.)
#[derive(Debug)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) enum MediaEvent {
    /// A remote track arrived on transceiver `transceiver`.
    Track {
        key: u64,
        kind: MediaKind,
        id: String,
        stream_ids: Vec<String>,
        transceiver: u64,
        /// Whether the track starts muted (a browser's remote track does,
        /// until media arrives).
        muted: bool,
    },
    Muted {
        key: u64,
        muted: bool,
    },
    Ended {
        key: u64,
    },
    NegotiationNeeded,
}

pub(crate) fn dispatch(id: u64, event: MediaEvent) {
    match event {
        MediaEvent::Track {
            key,
            kind,
            id: track_id,
            stream_ids,
            transceiver,
            muted,
        } => {
            let added = with_peer(id, |e| {
                e.media.remotes.insert(
                    key,
                    RemoteEntry {
                        kind,
                        id: track_id,
                        stream_ids,
                        transceiver,
                        muted,
                        ended: false,
                        on_mute: None,
                        on_unmute: None,
                        on_ended: None,
                    },
                );
            });
            if added.is_some() {
                let t = RemoteTrack { peer: id, key };
                registry::invoke(id, t, |e| Some(&mut e.media.on_track));
            }
        }
        MediaEvent::Muted { key, muted } => {
            let changed = with_peer(id, |e| {
                e.media
                    .remotes
                    .get_mut(&key)
                    .is_some_and(|r| !r.ended && std::mem::replace(&mut r.muted, muted) != muted)
            });
            if changed == Some(true) {
                if muted {
                    registry::invoke(id, (), move |e| {
                        e.media.remotes.get_mut(&key).map(|r| &mut r.on_mute)
                    });
                } else {
                    registry::invoke(id, (), move |e| {
                        e.media.remotes.get_mut(&key).map(|r| &mut r.on_unmute)
                    });
                }
            }
        }
        MediaEvent::Ended { key } => remote_ended(id, key),
        MediaEvent::NegotiationNeeded => {
            registry::invoke(id, (), |e| Some(&mut e.media.on_negotiation_needed))
        }
    }
}

fn remote_ended(id: u64, key: u64) {
    let fire = with_peer(id, |e| {
        e.media
            .remotes
            .get_mut(&key)
            .is_some_and(|r| !std::mem::replace(&mut r.ended, true))
    });
    if fire == Some(true) {
        registry::invoke(id, (), move |e| {
            e.media.remotes.get_mut(&key).map(|r| &mut r.on_ended)
        });
    }
}

/// The peer is closing: every remote track ends.
pub(crate) fn peer_closed(id: u64) {
    let keys: Vec<u64> =
        with_peer(id, |e| e.media.remotes.keys().copied().collect()).unwrap_or_default();
    for key in keys {
        remote_ended(id, key);
    }
}

/// Dead-owner sweep for the peer's media slots (see registry::sweep_dead).
pub(crate) fn sweep_dead(media: &mut PeerMedia, dead: &mut Vec<Box<dyn std::any::Any>>) {
    registry::take_dead(&mut media.on_track, dead);
    registry::take_dead(&mut media.on_negotiation_needed, dead);
    for r in media.remotes.values_mut() {
        registry::take_dead(&mut r.on_mute, dead);
        registry::take_dead(&mut r.on_unmute, dead);
        registry::take_dead(&mut r.on_ended, dead);
    }
}

// ------------------------------------------------- senders, transceivers

/// Sends one transceiver's media (`RTCRtpSender`).
#[derive(Clone, Debug)]
pub struct Sender {
    pub(crate) peer: u64,
    pub(crate) key: u64,
}

impl Sender {
    /// Swap what is sent, without renegotiating (same kind), or `None` to
    /// send nothing. `RTCRtpSender.replaceTrack`.
    pub fn replace_track(
        &self,
        track: Option<&LocalTrack>,
    ) -> impl Future<Output = Result<(), RtcError>> + 'static {
        let (peer, key) = (self.peer, self.key);
        let weak = track.map(|t| Rc::downgrade(&t.inner));
        let replaced = backend::replace_track(peer, key, track.map(LocalTrack::key));
        async move {
            replaced.await?;
            with_peer(peer, |e| match weak {
                Some(w) => e.media.sending.insert(key, w),
                None => e.media.sending.remove(&key),
            });
            Ok(())
        }
    }

    /// The track this sender carries: the one `add_track` or the last
    /// successful `replace_track` gave it, while that track is alive.
    pub fn track(&self) -> Option<LocalTrack> {
        with_peer(self.peer, |e| e.media.sending.get(&self.key)?.upgrade())
            .flatten()
            .map(|inner| LocalTrack { inner })
    }

    /// Cap the bitrate (`encodings[0].maxBitrate`), or `None` for no cap.
    pub fn set_max_bitrate(&self, bps: Option<u32>) {
        backend::set_max_bitrate(self.peer, self.key, bps);
    }
}

/// One m-line: a sender and a receiver (`RTCRtpTransceiver`).
#[derive(Clone, Debug)]
pub struct Transceiver {
    peer: u64,
    key: u64,
    sender: u64,
    kind: MediaKind,
}

impl Transceiver {
    pub fn kind(&self) -> MediaKind {
        self.kind
    }

    /// The negotiated `mid`, once there is one.
    pub fn mid(&self) -> Option<String> {
        backend::transceiver_mid(self.peer, self.key)
    }

    pub fn sender(&self) -> Sender {
        Sender {
            peer: self.peer,
            key: self.sender,
        }
    }

    /// The direction asked for.
    pub fn direction(&self) -> Direction {
        backend::transceiver_direction(self.peer, self.key)
    }

    /// Takes effect at the next negotiation (on_negotiation_needed fires).
    pub fn set_direction(&self, direction: Direction) {
        backend::set_transceiver_direction(self.peer, self.key, direction);
    }

    /// What was last negotiated.
    pub fn current_direction(&self) -> Option<Direction> {
        backend::transceiver_current_direction(self.peer, self.key)
    }

    /// Stop it for good (takes effect at the next negotiation).
    pub fn stop(&self) {
        backend::stop_transceiver(self.peer, self.key);
    }
}

// --------------------------------------------------------- remote tracks

/// A track the other side sends (`!Send`, UI thread).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteTrack {
    pub(crate) peer: u64,
    pub(crate) key: u64,
}

impl RemoteTrack {
    fn with<R>(&self, f: impl FnOnce(&mut RemoteEntry) -> R) -> Option<R> {
        with_peer(self.peer, |e| e.media.remotes.get_mut(&self.key).map(f)).flatten()
    }

    pub fn kind(&self) -> MediaKind {
        self.with(|r| r.kind).unwrap_or(MediaKind::Video)
    }

    /// The sender's track id.
    pub fn id(&self) -> String {
        self.with(|r| r.id.clone()).unwrap_or_default()
    }

    /// The stream ids the sender grouped it in (one person's audio and
    /// video share one).
    pub fn stream_ids(&self) -> Vec<String> {
        self.with(|r| r.stream_ids.clone()).unwrap_or_default()
    }

    pub fn transceiver(&self) -> Option<Transceiver> {
        let t = self.with(|r| r.transceiver)?;
        Some(Transceiver {
            peer: self.peer,
            key: t,
            sender: backend::transceiver_sender(self.peer, t).unwrap_or(0),
            kind: self.kind(),
        })
    }

    pub fn muted(&self) -> bool {
        self.with(|r| r.muted).unwrap_or(true)
    }

    pub fn ended(&self) -> bool {
        self.with(|r| r.ended).unwrap_or(true)
    }

    fn install(
        &self,
        pick: fn(&mut RemoteEntry) -> &mut Option<Slot<()>>,
        mut cb: impl FnMut() + 'static,
    ) {
        let key = self.key;
        registry::install(
            self.peer,
            move |e| e.media.remotes.get_mut(&key).map(pick),
            Box::new(move |()| cb()),
        );
    }

    pub fn on_mute(&self, cb: impl FnMut() + 'static) -> &Self {
        self.install(|r| &mut r.on_mute, cb);
        self
    }

    pub fn on_unmute(&self, cb: impl FnMut() + 'static) -> &Self {
        self.install(|r| &mut r.on_unmute, cb);
        self
    }

    pub fn on_ended(&self, cb: impl FnMut() + 'static) -> &Self {
        self.install(|r| &mut r.on_ended, cb);
        self
    }

    /// Audio: the playback volume, 0.0 (silent) to 1.0.
    pub fn set_volume(&self, volume: f32) {
        backend::set_remote_volume(self.peer, self.key, volume.clamp(0.0, 1.0));
    }

    /// Audio: play or pause it. Remote audio plays as soon as it arrives.
    pub fn set_playing(&self, playing: bool) {
        backend::set_remote_playing(self.peer, self.key, playing);
    }

    /// Video: each decoded frame as RGBA8. For analysis, recording or
    /// tests; showing a track is [`MediaView`], which does not go through
    /// this. On the web it reads the frames back from a hidden `<video>`
    /// (costly); on desktop it runs on a decoder thread.
    pub fn on_frame(&self, cb: impl FnMut(&[u8], u32, u32) + Send + 'static) -> &Self {
        backend::on_remote_frame(self.peer, self.key, Box::new(cb));
        self
    }
}

// --------------------------------------------------------- Peer methods

impl Peer {
    /// Send `track`: a sendrecv transceiver of its kind. Renegotiate after
    /// (see [`on_negotiation_needed`](Self::on_negotiation_needed)) unless it
    /// was added before the first offer.
    pub fn add_track(&self, track: &LocalTrack) -> Result<Sender, RtcError> {
        if track.ended() {
            return Err(RtcError::InvalidState("the track has ended".into()));
        }
        let (sender, transceiver) = (next_id(), next_id());
        backend::add_track(self.id(), track.key(), sender, transceiver)?;
        with_peer(self.id(), |e| {
            e.media.sending.insert(sender, Rc::downgrade(&track.inner))
        });
        Ok(Sender {
            peer: self.id(),
            key: sender,
        })
    }

    /// A transceiver with nothing to send yet, e.g. `RecvOnly` to receive
    /// video without a camera.
    pub fn add_transceiver(
        &self,
        kind: MediaKind,
        direction: Direction,
    ) -> Result<Transceiver, RtcError> {
        let (sender, transceiver) = (next_id(), next_id());
        backend::add_transceiver(self.id(), kind, direction, sender, transceiver)?;
        Ok(Transceiver {
            peer: self.id(),
            key: transceiver,
            sender,
            kind,
        })
    }

    /// Every transceiver on the peer, in m-line order.
    pub fn transceivers(&self) -> Vec<Transceiver> {
        backend::transceivers(self.id())
            .into_iter()
            .map(|(key, sender, kind)| Transceiver {
                peer: self.id(),
                key,
                sender,
                kind,
            })
            .collect()
    }

    /// A remote track arrived.
    pub fn on_track(&self, cb: impl FnMut(RemoteTrack) + 'static) -> &Self {
        registry::install(self.id(), |e| Some(&mut e.media.on_track), Box::new(cb));
        self
    }

    /// Something changed that needs a new offer/answer (a track added, a
    /// direction changed). The browser's `negotiationneeded`.
    pub fn on_negotiation_needed(&self, mut cb: impl FnMut() + 'static) -> &Self {
        registry::install(
            self.id(),
            |e| Some(&mut e.media.on_negotiation_needed),
            Box::new(move |()| cb()),
        );
        self
    }
}

/// Cameras, for a picker (`enumerateDevices` on the web; rinch-av on desktop).
pub fn video_inputs() -> impl Future<Output = Vec<DeviceInfo>> {
    backend::devices(MediaKind::Video)
}

/// Microphones, for a picker.
pub fn audio_inputs() -> impl Future<Output = Vec<DeviceInfo>> {
    backend::devices(MediaKind::Audio)
}
