//! Native media backend (design record `docs/design/rtc-media.md`, section 3):
//! webrtc-rs packetizes, openh264 and Opus encode and decode, rinch-av
//! captures and plays, and received video reaches `MediaView` through
//! rinch-core's named video sink (M6).
//!
//! Threads:
//! - **The main thread** keeps the local tracks' sources (`LOCALS`), the
//!   capture handles, and the view elements.
//! - **One pump thread per local track** (`rinch-rtc-enc-<key>`): video at
//!   the track's fps (latest frame, RGBA→I420, openh264), audio every 20 ms
//!   (resampled to 48 kHz, Opus). It feeds the track's self-views and every
//!   sender carrying the track: one encoder however many peers send it.
//! - **The rinch-rtc worker** (native.rs) owns the peer connections: it
//!   writes each sender's encoded frames as samples, and reads each remote
//!   track's RTP.
//! - **One decoder thread per remote video track** (`rinch-rtc-dec-<key>`):
//!   openh264, I420→RGBA, then the views and `on_frame`.
//! - **Remote audio** is decoded on the worker (Opus is cheap), buffered per
//!   track (60 ms before it starts), mixed, and played through one rinch-av
//!   output stream for the process.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
#[cfg(feature = "media-cisco-openh264")]
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::video_sink::VideoFrameSink;

use crate::RtcError;
use crate::media::{
    self, AudioQueue, AudioSourceOptions, AudioSourceWriter, CameraOptions, DeviceInfo, Direction,
    MediaError, MediaKind, MediaTrack, MicrophoneOptions, VideoSlot, VideoSourceOptions,
    VideoSourceWriter,
};

/// One encoded frame for the senders.
#[derive(Clone)]
pub(crate) struct Encoded {
    pub(crate) data: bytes::Bytes,
    pub(crate) duration: Duration,
}

/// How many encoded frames a sender may have waiting before new ones are
/// dropped (a stalled peer must not grow memory).
const SENDER_QUEUE: usize = 32;

// ================================================================ H.264

#[cfg(feature = "media-cisco-openh264")]
static CISCO_PATH: OnceLock<std::path::PathBuf> = OnceLock::new();

/// Use Cisco's prebuilt OpenH264 library at `path` for H.264 (the
/// `media-cisco-openh264` feature; M3). The app downloads it from Cisco
/// itself, separately from the app, as Cisco's licence requires. The file's
/// hash must be one openh264 0.9.9 knows. Call it once, before any video
/// track is sent or received.
#[cfg(feature = "media-cisco-openh264")]
pub fn load_openh264(path: impl AsRef<std::path::Path>) -> Result<(), MediaError> {
    let path = path.as_ref().to_path_buf();
    openh264::OpenH264API::from_blob_path(&path)
        .map_err(|e| MediaError::NotSupported(format!("OpenH264 library: {e}")))?;
    let _ = CISCO_PATH.set(path);
    Ok(())
}

fn h264_api() -> Result<openh264::OpenH264API, MediaError> {
    #[cfg(feature = "media-cisco-openh264")]
    {
        let path = CISCO_PATH.get().ok_or_else(|| {
            MediaError::NotSupported(
                "H.264: call rinch_rtc::media::load_openh264 with Cisco's library first".into(),
            )
        })?;
        openh264::OpenH264API::from_blob_path(path)
            .map_err(|e| MediaError::NotSupported(format!("OpenH264 library: {e}")))
    }
    #[cfg(not(feature = "media-cisco-openh264"))]
    {
        Ok(openh264::OpenH264API::from_source())
    }
}

// ============================================================ local tracks

/// A sender carrying a local track: (peer, sender key) and its queue.
type Subscriber = ((u64, u64), tokio::sync::mpsc::Sender<Encoded>);

/// A local track's source, shared with its pump thread and the senders.
pub(crate) struct LocalSource {
    kind: MediaKind,
    id: String,
    enabled: AtomicBool,
    stopped: AtomicBool,
    /// Video: the latest RGBA frame (a writer's, or the camera's).
    video: Option<Arc<VideoSlot>>,
    fps: u32,
    /// Audio: queued interleaved samples at the queue's rate and channels.
    audio: Option<Arc<AudioQueue>>,
    /// Every sender carrying this track: (peer, sender key) and its queue.
    subscribers: Mutex<Vec<Subscriber>>,
    /// The next video frame is a keyframe (a new sender, a decoder's PLI).
    keyframe: AtomicBool,
    /// `set_max_bitrate` per (peer, sender); the encoder runs at the lowest.
    caps: Mutex<HashMap<(u64, u64), u32>>,
    /// Self-views: (view id, sink).
    views: Mutex<Vec<(String, VideoFrameSink)>>,
}

impl LocalSource {
    fn new(
        kind: MediaKind,
        key: u64,
        video: Option<Arc<VideoSlot>>,
        fps: u32,
        audio: Option<Arc<AudioQueue>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            kind,
            id: media::local_id(key),
            enabled: AtomicBool::new(true),
            stopped: AtomicBool::new(false),
            video,
            fps: fps.clamp(1, 60),
            audio,
            subscribers: Mutex::new(Vec::new()),
            keyframe: AtomicBool::new(true),
            caps: Mutex::new(HashMap::new()),
            views: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn kind(&self) -> MediaKind {
        self.kind
    }

    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    /// A sender starts carrying this track; it gets a keyframe first.
    pub(crate) fn subscribe(&self, who: (u64, u64)) -> tokio::sync::mpsc::Receiver<Encoded> {
        let (tx, rx) = tokio::sync::mpsc::channel(SENDER_QUEUE);
        if let Ok(mut s) = self.subscribers.lock() {
            s.retain(|(w, _)| *w != who);
            s.push((who, tx));
        }
        self.keyframe.store(true, Ordering::Release);
        rx
    }

    pub(crate) fn unsubscribe(&self, who: (u64, u64)) {
        if let Ok(mut s) = self.subscribers.lock() {
            s.retain(|(w, _)| *w != who);
        }
        if let Ok(mut c) = self.caps.lock() {
            c.remove(&who);
        }
    }

    pub(crate) fn request_keyframe(&self) {
        self.keyframe.store(true, Ordering::Release);
    }

    fn has_subscribers(&self) -> bool {
        self.subscribers
            .lock()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    fn publish(&self, frame: Encoded) {
        if let Ok(mut s) = self.subscribers.lock() {
            // A closed queue is a sender that went away.
            s.retain(|(_, tx)| {
                !matches!(
                    tx.try_send(frame.clone()),
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_))
                )
            });
        }
    }

    /// The encoder's target bitrate: the lowest cap, or a default by size.
    fn bitrate(&self, width: u32, height: u32) -> u32 {
        let default = if width * height > 640 * 480 {
            1_500_000
        } else {
            800_000
        };
        self.caps
            .lock()
            .ok()
            .and_then(|c| c.values().copied().min())
            .map(|cap| cap.min(default))
            .unwrap_or(default)
    }

    fn show(&self, rgba: &[u8], w: u32, h: u32) {
        if let Ok(v) = self.views.lock() {
            for (_, sink) in v.iter() {
                sink(rgba, w, h);
            }
        }
    }
}

/// A device a local track holds open (main thread only: a cpal stream is
/// not `Send` everywhere).
enum Capture {
    Camera(#[allow(dead_code)] rinch_av::camera::CameraCapture),
    Microphone(#[allow(dead_code)] rinch_av::audio::AudioInputStream),
}

thread_local! {
    static LOCALS: RefCell<HashMap<u64, Arc<LocalSource>>> = RefCell::new(HashMap::new());
    static CAPTURES: RefCell<HashMap<u64, Capture>> = RefCell::new(HashMap::new());
}

pub(crate) fn local_source(key: u64) -> Option<Arc<LocalSource>> {
    LOCALS.with(|l| l.borrow().get(&key).cloned())
}

fn start_local(key: u64, source: Arc<LocalSource>) -> Result<(), MediaError> {
    let s = source.clone();
    std::thread::Builder::new()
        .name(format!("rinch-rtc-enc-{key}"))
        .spawn(move || match s.kind {
            MediaKind::Video => video_pump(s),
            MediaKind::Audio => audio_pump(s),
        })
        .map_err(|e| MediaError::Backend(format!("encoder thread: {e}")))?;
    LOCALS.with(|l| l.borrow_mut().insert(key, source));
    Ok(())
}

fn av_error(e: rinch_av::AvError) -> MediaError {
    match e {
        rinch_av::AvError::DeviceNotFound(m) => MediaError::DeviceNotFound(m),
        rinch_av::AvError::PermissionDenied => MediaError::PermissionDenied,
        rinch_av::AvError::DeviceInUse => MediaError::DeviceInUse,
        rinch_av::AvError::FormatUnsupported(m) => MediaError::NotSupported(m),
        other => MediaError::Backend(other.to_string()),
    }
}

pub(crate) fn open_camera(
    key: u64,
    o: CameraOptions,
) -> impl std::future::Future<Output = Result<(), MediaError>> + 'static {
    let opened = (|| {
        let writer = VideoSourceWriter::new();
        let slot = writer.slot.clone();
        let config = rinch_av::camera::CameraConfig {
            width: o.width,
            height: o.height,
            fps: o.fps,
            ..Default::default()
        };
        let on_frame = move |f: rinch_av::camera::VideoFrame| {
            writer.push_rgba(&f.data, f.width, f.height);
        };
        let capture = match &o.device {
            Some(d) => {
                rinch_av::camera::open_camera_on(&rinch_av::DeviceId(d.clone()), config, on_frame)
            }
            None => rinch_av::camera::open_camera(config, on_frame),
        }
        .map_err(av_error)?;
        start_local(
            key,
            LocalSource::new(MediaKind::Video, key, Some(slot), o.fps, None),
        )?;
        CAPTURES.with(|c| c.borrow_mut().insert(key, Capture::Camera(capture)));
        Ok(())
    })();
    std::future::ready(opened)
}

pub(crate) fn open_microphone(
    key: u64,
    o: MicrophoneOptions,
) -> impl std::future::Future<Output = Result<(), MediaError>> + 'static {
    let opened = (|| {
        // The queue takes the device's real format, which is known only
        // once it is open: fill a provisional one, then swap.
        let queue: Arc<Mutex<Option<Arc<AudioQueue>>>> = Arc::default();
        let q2 = queue.clone();
        let config = rinch_av::audio::AudioInputConfig {
            sample_rate: 48_000,
            channels: 1,
            buffer_size: 0,
        };
        let on_data = move |samples: &[f32]| {
            if let Ok(q) = q2.lock()
                && let Some(q) = q.as_ref()
            {
                push_queue(q, samples);
            }
        };
        let stream = match &o.device {
            Some(d) => rinch_av::audio::open_audio_input_on(
                &rinch_av::DeviceId(d.clone()),
                config,
                on_data,
            ),
            None => rinch_av::audio::open_audio_input(config, on_data),
        }
        .map_err(av_error)?;
        let real = stream.config().clone();
        let w = AudioSourceWriter::new(AudioSourceOptions {
            sample_rate: real.sample_rate,
            channels: real.channels,
        });
        if let Ok(mut q) = queue.lock() {
            *q = Some(w.queue.clone());
        }
        start_local(
            key,
            LocalSource::new(MediaKind::Audio, key, None, 50, Some(w.queue.clone())),
        )?;
        CAPTURES.with(|c| c.borrow_mut().insert(key, Capture::Microphone(stream)));
        Ok(())
    })();
    std::future::ready(opened)
}

fn push_queue(q: &AudioQueue, samples: &[f32]) {
    let cap = q.sample_rate as usize * q.channels as usize;
    if let Ok(mut d) = q.samples.lock() {
        d.extend(samples.iter().copied());
        while d.len() > cap {
            d.pop_front();
        }
    }
}

pub(crate) fn open_video_source(
    key: u64,
    o: VideoSourceOptions,
    w: VideoSourceWriter,
) -> Result<(), MediaError> {
    start_local(
        key,
        LocalSource::new(MediaKind::Video, key, Some(w.slot.clone()), o.fps, None),
    )
}

pub(crate) fn open_audio_source(
    key: u64,
    _o: AudioSourceOptions,
    w: AudioSourceWriter,
) -> Result<(), MediaError> {
    start_local(
        key,
        LocalSource::new(MediaKind::Audio, key, None, 50, Some(w.queue.clone())),
    )
}

pub(crate) fn waiting_for_gesture(_key: u64) -> bool {
    false
}

pub(crate) fn set_local_enabled(key: u64, enabled: bool) {
    if let Some(s) = local_source(key) {
        s.enabled.store(enabled, Ordering::Release);
        s.request_keyframe();
    }
}

pub(crate) fn stop_local(key: u64) {
    let gone = LOCALS.with(|l| l.borrow_mut().remove(&key));
    if let Some(s) = gone {
        s.stopped.store(true, Ordering::Release);
        // The senders' queues close: they send nothing more.
        if let Ok(mut subs) = s.subscribers.lock() {
            subs.clear();
        }
        if let Ok(mut v) = s.views.lock() {
            v.clear();
        }
    }
    let capture = CAPTURES.with(|c| c.borrow_mut().remove(&key));
    drop(capture);
}

/// Video: at the track's fps, the latest frame to the self-views and, when
/// someone sends the track, through openh264 to every sender.
fn video_pump(src: Arc<LocalSource>) {
    let Some(slot) = src.video.clone() else {
        return;
    };
    let period = Duration::from_secs_f64(1.0 / f64::from(src.fps));
    let mut next = Instant::now();
    let mut seen = u64::MAX;
    let mut frame: Vec<u8> = Vec::new();
    let (mut w, mut h) = (0u32, 0u32);
    let mut encoder: Option<(openh264::encoder::Encoder, u32, u32, u32)> = None;
    let mut last_key = Instant::now();
    let mut last_encode = Instant::now() - Duration::from_secs(1);
    let mut warned = false;
    let started = Instant::now();
    while !src.stopped.load(Ordering::Acquire) {
        next += period;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else {
            next = now;
        }
        let seq = slot.seq.load(Ordering::Acquire);
        let fresh = seq != seen;
        if fresh {
            seen = seq;
            if let Ok(l) = slot.latest.lock()
                && let Some((rgba, fw, fh)) = l.as_ref()
            {
                // openh264 wants even dimensions: drop an odd last row or
                // column.
                let (ew, eh) = (*fw & !1, *fh & !1);
                if ew == 0 || eh == 0 {
                    continue;
                }
                (w, h) = (ew, eh);
                frame.resize((w * h * 4) as usize, 0);
                for row in 0..h as usize {
                    let s = row * *fw as usize * 4;
                    let d = row * w as usize * 4;
                    frame[d..d + w as usize * 4].copy_from_slice(&rgba[s..s + w as usize * 4]);
                }
                slot.taken.fetch_add(1, Ordering::Relaxed);
            }
        }
        if w == 0 {
            continue;
        }
        let enabled = src.enabled.load(Ordering::Acquire);
        if !enabled {
            // Black, as a browser shows and sends a disabled track.
            frame.iter_mut().enumerate().for_each(|(i, p)| {
                *p = if i % 4 == 3 { 255 } else { 0 };
            });
        }
        if fresh || !enabled {
            src.show(&frame, w, h);
        }
        if !src.has_subscribers() {
            continue;
        }
        // A still source is re-sent at least twice a second (a viewer that
        // joins late gets a picture); a disabled one once a second.
        let gap = if enabled {
            Duration::from_millis(500)
        } else {
            Duration::from_secs(1)
        };
        if !fresh && last_encode.elapsed() < gap && !src.keyframe.load(Ordering::Acquire) {
            continue;
        }
        let bitrate = src.bitrate(w, h);
        if encoder
            .as_ref()
            .is_none_or(|(_, ew, eh, eb)| (*ew, *eh, *eb) != (w, h, bitrate))
        {
            let api = match h264_api() {
                Ok(api) => api,
                Err(e) => {
                    if !warned {
                        tracing::error!("rinch-rtc: no H.264 encoder: {e}");
                        warned = true;
                    }
                    continue;
                }
            };
            let config = openh264::encoder::EncoderConfig::new()
                .bitrate(openh264::encoder::BitRate::from_bps(bitrate))
                .max_frame_rate(openh264::encoder::FrameRate::from_hz(src.fps as f32))
                .usage_type(openh264::encoder::UsageType::CameraVideoRealTime)
                .profile(openh264::encoder::Profile::Baseline)
                .skip_frames(false);
            match openh264::encoder::Encoder::with_api_config(api, config) {
                Ok(e) => {
                    encoder = Some((e, w, h, bitrate));
                    src.request_keyframe();
                }
                Err(e) => {
                    if !warned {
                        tracing::error!("rinch-rtc: H.264 encoder: {e}");
                        warned = true;
                    }
                    continue;
                }
            }
        }
        let Some((enc, ..)) = encoder.as_mut() else {
            continue;
        };
        // A keyframe on request, and every 2 s so a lost one heals (no PLI
        // reaches the sender through webrtc-rs's default interceptors).
        if src.keyframe.swap(false, Ordering::AcqRel) || last_key.elapsed() > Duration::from_secs(2)
        {
            enc.force_intra_frame();
            last_key = Instant::now();
        }
        let yuv = openh264::formats::YUVBuffer::from_rgb_source(
            openh264::formats::RgbaSliceU8::new(&frame, (w as usize, h as usize)),
        );
        let ts = openh264::Timestamp::from_millis(started.elapsed().as_millis() as u64);
        match enc.encode_at(&yuv, ts) {
            Ok(bits) => {
                let data = bits.to_vec();
                if !data.is_empty() {
                    last_encode = Instant::now();
                    src.publish(Encoded {
                        data: bytes::Bytes::from(data),
                        duration: period,
                    });
                }
            }
            Err(e) => tracing::warn!("rinch-rtc: H.264 encode: {e}"),
        }
    }
}

/// A linear resampler for interleaved audio, keeping its phase across
/// chunks.
pub(crate) struct Resampler {
    from: u32,
    to: u32,
    channels: usize,
    /// Position of the next output sample, in input frames, relative to
    /// `last`'s frame (which is input frame -1).
    pos: f64,
    last: Vec<f32>,
}

impl Resampler {
    pub(crate) fn new(from: u32, to: u32, channels: usize) -> Self {
        Self {
            from,
            to,
            channels,
            pos: 0.0,
            last: vec![0.0; channels],
        }
    }

    /// Resample `input` (interleaved, whole frames) and append to `out`.
    pub(crate) fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let c = self.channels;
        if self.from == self.to {
            out.extend_from_slice(input);
            return;
        }
        let frames = input.len() / c;
        let step = f64::from(self.from) / f64::from(self.to);
        let at = |i: isize, ch: usize| -> f32 {
            if i < 0 {
                self.last[ch]
            } else {
                input[i as usize * c + ch]
            }
        };
        // Output frame k sits at input position `pos` (in frames, where -1
        // is the previous chunk's last frame).
        while self.pos <= frames as f64 - 1.0 {
            let i0 = self.pos.floor();
            let t = (self.pos - i0) as f32;
            let i0 = i0 as isize - 1;
            for ch in 0..c {
                let a = at(i0, ch);
                let b = at(i0 + 1, ch);
                out.push(a + (b - a) * t);
            }
            self.pos += step;
        }
        self.pos -= frames as f64;
        if frames > 0 {
            self.last
                .copy_from_slice(&input[(frames - 1) * c..frames * c]);
        }
    }
}

/// Audio: every 20 ms, 20 ms of the source (silence when short or
/// disabled), resampled to 48 kHz, Opus-encoded, to every sender.
fn audio_pump(src: Arc<LocalSource>) {
    let Some(queue) = src.audio.clone() else {
        return;
    };
    let in_ch = usize::from(queue.channels.max(1));
    let out_ch = if in_ch == 1 { 1 } else { 2 };
    let mut encoder = match opus::Encoder::new(
        48_000,
        if out_ch == 1 {
            opus::Channels::Mono
        } else {
            opus::Channels::Stereo
        },
        opus::Application::Voip,
    ) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("rinch-rtc: Opus encoder: {e}");
            return;
        }
    };
    let mut resampler = Resampler::new(queue.sample_rate, 48_000, out_ch);
    let per_tick = (queue.sample_rate as usize / 50) * in_ch;
    let mut pending: Vec<f32> = Vec::new();
    let mut out = vec![0u8; 4000];
    let period = Duration::from_millis(20);
    let mut next = Instant::now();
    while !src.stopped.load(Ordering::Acquire) {
        next += period;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else {
            next = now;
        }
        let mut chunk: Vec<f32> = match queue.samples.lock() {
            Ok(mut q) => {
                let n = q.len().min(per_tick);
                queue.taken.fetch_add(n as u64, Ordering::Relaxed);
                q.drain(..n).collect()
            }
            Err(_) => Vec::new(),
        };
        chunk.resize(per_tick, 0.0);
        if !src.enabled.load(Ordering::Acquire) {
            chunk.iter_mut().for_each(|s| *s = 0.0);
        }
        // To the encoder's channel count: first two channels (or the one).
        let frames = chunk.len() / in_ch;
        let mut mapped = Vec::with_capacity(frames * out_ch);
        for f in 0..frames {
            for c in 0..out_ch {
                mapped.push(chunk[f * in_ch + c.min(in_ch - 1)]);
            }
        }
        resampler.process(&mapped, &mut pending);
        let frame = 960 * out_ch;
        while pending.len() >= frame {
            let pcm: Vec<f32> = pending.drain(..frame).collect();
            if !src.has_subscribers() {
                continue;
            }
            match encoder.encode_float(&pcm, &mut out) {
                Ok(n) => src.publish(Encoded {
                    data: bytes::Bytes::copy_from_slice(&out[..n]),
                    duration: period,
                }),
                Err(e) => tracing::warn!("rinch-rtc: Opus encode: {e}"),
            }
        }
    }
}

// ============================================== per-peer state (main side)

/// What the main thread knows of a peer's transceivers. The worker fills
/// in what negotiation decides (mid, current direction); the main thread
/// answers from here without a round trip.
#[derive(Debug, Clone)]
pub(crate) struct TxMirror {
    pub(crate) key: u64,
    pub(crate) sender: u64,
    pub(crate) kind: MediaKind,
    pub(crate) direction: Direction,
    pub(crate) current: Option<Direction>,
    pub(crate) mid: Option<String>,
    pub(crate) stopped: bool,
}

static MIRRORS: Mutex<Option<HashMap<u64, Vec<TxMirror>>>> = Mutex::new(None);

pub(crate) fn with_mirror<R>(peer: u64, f: impl FnOnce(&mut Vec<TxMirror>) -> R) -> R {
    let mut m = MIRRORS.lock().unwrap_or_else(|e| e.into_inner());
    let map = m.get_or_insert_with(HashMap::new);
    f(map.entry(peer).or_default())
}

pub(crate) fn drop_mirror(peer: u64) {
    let mut m = MIRRORS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(map) = m.as_mut() {
        map.remove(&peer);
    }
}

thread_local! {
    /// (peer, sender key) → the local track it carries (for set_max_bitrate).
    static SENDING: RefCell<HashMap<(u64, u64), u64>> = RefCell::new(HashMap::new());
}

pub(crate) fn add_track(p: u64, l: u64, s: u64, t: u64) -> Result<(), RtcError> {
    let source =
        local_source(l).ok_or_else(|| RtcError::InvalidState("the track is not open".into()))?;
    let kind = source.kind;
    with_mirror(p, |m| {
        m.push(TxMirror {
            key: t,
            sender: s,
            kind,
            direction: Direction::SendRecv,
            current: None,
            mid: None,
            stopped: false,
        })
    });
    SENDING.with(|m| m.borrow_mut().insert((p, s), l));
    crate::native::post_media(
        p,
        MediaCmd::AddTrack {
            sender: s,
            tx: t,
            source,
        },
    );
    Ok(())
}

pub(crate) fn add_transceiver(
    p: u64,
    k: MediaKind,
    d: Direction,
    s: u64,
    t: u64,
) -> Result<(), RtcError> {
    with_mirror(p, |m| {
        m.push(TxMirror {
            key: t,
            sender: s,
            kind: k,
            direction: d,
            current: None,
            mid: None,
            stopped: false,
        })
    });
    crate::native::post_media(
        p,
        MediaCmd::AddTransceiver {
            kind: k,
            direction: d,
            sender: s,
            tx: t,
        },
    );
    Ok(())
}

pub(crate) fn transceivers(p: u64) -> Vec<(u64, u64, MediaKind)> {
    with_mirror(p, |m| m.iter().map(|t| (t.key, t.sender, t.kind)).collect())
}

pub(crate) fn replace_track(
    p: u64,
    s: u64,
    l: Option<u64>,
) -> impl std::future::Future<Output = Result<(), RtcError>> + 'static {
    let started = (|| {
        let source = match l {
            Some(l) => Some(
                local_source(l)
                    .ok_or_else(|| RtcError::InvalidState("the track is not open".into()))?,
            ),
            None => None,
        };
        let req = crate::registry::new_request(p);
        match l {
            Some(l) => SENDING.with(|m| m.borrow_mut().insert((p, s), l)),
            None => SENDING.with(|m| m.borrow_mut().remove(&(p, s))),
        };
        crate::native::post_media(
            p,
            MediaCmd::ReplaceTrack {
                req,
                sender: s,
                source,
            },
        );
        Ok(req)
    })();
    async move {
        let req = started?;
        crate::registry::reply(p, req).await.map(|_| ())
    }
}

pub(crate) fn set_max_bitrate(p: u64, s: u64, bps: Option<u32>) {
    let Some(l) = SENDING.with(|m| m.borrow().get(&(p, s)).copied()) else {
        return;
    };
    if let Some(src) = local_source(l)
        && let Ok(mut c) = src.caps.lock()
    {
        match bps {
            Some(b) => c.insert((p, s), b.max(30_000)),
            None => c.remove(&(p, s)),
        };
    }
}

pub(crate) fn transceiver_mid(p: u64, t: u64) -> Option<String> {
    with_mirror(p, |m| {
        m.iter().find(|x| x.key == t).and_then(|x| x.mid.clone())
    })
}

pub(crate) fn transceiver_direction(p: u64, t: u64) -> Direction {
    with_mirror(p, |m| {
        m.iter()
            .find(|x| x.key == t)
            .map(|x| {
                if x.stopped {
                    Direction::Inactive
                } else {
                    x.direction
                }
            })
            .unwrap_or(Direction::Inactive)
    })
}

pub(crate) fn set_transceiver_direction(p: u64, t: u64, d: Direction) {
    let ok = with_mirror(p, |m| {
        m.iter_mut()
            .find(|x| x.key == t && !x.stopped)
            .map(|x| x.direction = d)
            .is_some()
    });
    if ok {
        crate::native::post_media(
            p,
            MediaCmd::SetDirection {
                tx: t,
                direction: d,
            },
        );
    }
}

pub(crate) fn transceiver_current_direction(p: u64, t: u64) -> Option<Direction> {
    with_mirror(p, |m| m.iter().find(|x| x.key == t).and_then(|x| x.current))
}

pub(crate) fn stop_transceiver(p: u64, t: u64) {
    with_mirror(p, |m| {
        if let Some(x) = m.iter_mut().find(|x| x.key == t) {
            x.stopped = true;
        }
    });
    crate::native::post_media(p, MediaCmd::StopTransceiver { tx: t });
}

pub(crate) fn transceiver_sender(p: u64, t: u64) -> Option<u64> {
    with_mirror(p, |m| m.iter().find(|x| x.key == t).map(|x| x.sender))
}

/// The main thread's part of a peer closing.
pub(crate) fn peer_closed_main(p: u64) {
    drop_mirror(p);
    SENDING.with(|m| m.borrow_mut().retain(|(peer, _), _| *peer != p));
}

// ============================================================ remote tracks

/// A remote track's outputs, shared by the worker, its decoder and the
/// main thread.
pub(crate) struct RemoteSink {
    frame_cb: Mutex<Option<media::FrameCallback>>,
    views: Mutex<Vec<(String, VideoFrameSink)>>,
    /// f32 bits.
    volume: AtomicU32,
    playing: AtomicBool,
    pub(crate) ended: AtomicBool,
    /// The views have been told a frame exists.
    shown: AtomicBool,
}

impl RemoteSink {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            frame_cb: Mutex::new(None),
            views: Mutex::new(Vec::new()),
            volume: AtomicU32::new(1.0f32.to_bits()),
            playing: AtomicBool::new(true),
            ended: AtomicBool::new(false),
            shown: AtomicBool::new(false),
        })
    }

    pub(crate) fn deliver(&self, rgba: &[u8], w: u32, h: u32) {
        if let Ok(v) = self.views.lock() {
            for (_, sink) in v.iter() {
                sink(rgba, w, h);
            }
            if !v.is_empty() && !self.shown.swap(true, Ordering::AcqRel) {
                let ids: Vec<String> = v.iter().map(|(id, _)| id.clone()).collect();
                rinch_core::dispatch_main_callback(Box::new(move || view_ready(&ids)));
            }
        }
        if let Ok(mut cb) = self.frame_cb.lock()
            && let Some(cb) = cb.as_mut()
        {
            cb(rgba, w, h);
        }
    }

    pub(crate) fn volume(&self) -> f32 {
        if self.playing.load(Ordering::Acquire) {
            f32::from_bits(self.volume.load(Ordering::Acquire))
        } else {
            0.0
        }
    }
}

type RemoteMap = HashMap<(u64, u64), Arc<RemoteSink>>;

static REMOTES: Mutex<Option<RemoteMap>> = Mutex::new(None);

pub(crate) fn insert_remote(peer: u64, key: u64, sink: Arc<RemoteSink>) {
    let mut r = REMOTES.lock().unwrap_or_else(|e| e.into_inner());
    r.get_or_insert_with(HashMap::new).insert((peer, key), sink);
}

pub(crate) fn remote(peer: u64, key: u64) -> Option<Arc<RemoteSink>> {
    let r = REMOTES.lock().unwrap_or_else(|e| e.into_inner());
    r.as_ref().and_then(|m| m.get(&(peer, key)).cloned())
}

/// The worker's part of a peer closing: its remote tracks' outputs go.
pub(crate) fn drop_remotes(peer: u64) {
    let gone: Vec<Arc<RemoteSink>> = {
        let mut r = REMOTES.lock().unwrap_or_else(|e| e.into_inner());
        let Some(m) = r.as_mut() else { return };
        let keys: Vec<(u64, u64)> = m.keys().filter(|(p, _)| *p == peer).copied().collect();
        keys.into_iter().filter_map(|k| m.remove(&k)).collect()
    };
    for s in &gone {
        s.ended.store(true, Ordering::Release);
    }
    mixer_remove_peer(peer);
    // The sinks (and the app's on_frame callback in them) drop here, off
    // the main thread: a callback is Send.
    drop(gone);
}

pub(crate) fn set_remote_volume(p: u64, k: u64, v: f32) {
    if let Some(s) = remote(p, k) {
        s.volume.store(v.to_bits(), Ordering::Release);
    }
}

pub(crate) fn set_remote_playing(p: u64, k: u64, playing: bool) {
    if let Some(s) = remote(p, k) {
        s.playing.store(playing, Ordering::Release);
    }
}

pub(crate) fn on_remote_frame(p: u64, k: u64, cb: media::FrameCallback) {
    if let Some(s) = remote(p, k)
        && let Ok(mut c) = s.frame_cb.lock()
    {
        *c = Some(cb);
    }
}

/// A decoder thread for one remote video track: H.264 access units in,
/// RGBA out to the sink. A frame it cannot decode asks for a keyframe
/// through `pli` (at most once a second).
pub(crate) fn start_video_decoder(
    key: u64,
    sink: Arc<RemoteSink>,
    pli: tokio::sync::mpsc::UnboundedSender<()>,
) -> Option<std::sync::mpsc::SyncSender<bytes::Bytes>> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<bytes::Bytes>(64);
    let started = std::thread::Builder::new()
        .name(format!("rinch-rtc-dec-{key}"))
        .spawn(move || {
            let api = match h264_api() {
                Ok(a) => a,
                Err(e) => {
                    tracing::error!("rinch-rtc: no H.264 decoder: {e}");
                    return;
                }
            };
            let mut dec = match openh264::decoder::Decoder::with_api_config(
                api,
                openh264::decoder::DecoderConfig::new(),
            ) {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!("rinch-rtc: H.264 decoder: {e}");
                    return;
                }
            };
            let mut rgba: Vec<u8> = Vec::new();
            let mut last_pli = Instant::now() - Duration::from_secs(2);
            while let Ok(au) = rx.recv() {
                if sink.ended.load(Ordering::Acquire) {
                    break;
                }
                match dec.decode(&au) {
                    Ok(Some(yuv)) => {
                        use openh264::formats::YUVSource;
                        let (w, h) = yuv.dimensions();
                        rgba.resize(w * h * 4, 0);
                        yuv.write_rgba8(&mut rgba);
                        sink.deliver(&rgba, w as u32, h as u32);
                    }
                    Ok(None) => {}
                    Err(_) => {
                        if last_pli.elapsed() > Duration::from_secs(1) {
                            last_pli = Instant::now();
                            let _ = pli.send(());
                        }
                    }
                }
            }
        });
    started.ok().map(|_| tx)
}

// ================================================================== mixer

/// Remote audio, per track: 48 kHz stereo, buffered.
struct MixTrack {
    sink: Arc<RemoteSink>,
    buf: VecDeque<f32>,
    /// Started: 60 ms were buffered. An underrun un-primes it.
    primed: bool,
}

/// 60 ms of 48 kHz stereo before a track starts playing.
const PRIME: usize = 48 * 60 * 2;
/// At most 300 ms buffered; older samples go.
const MAX_BUF: usize = 48 * 300 * 2;

enum Output {
    /// Nothing opened yet.
    None,
    /// A device, opened on the main thread.
    Device,
    /// The device could not be opened: decode on, play nothing.
    Failed,
    /// Test hook: mixed audio is collected by `testing::capture_audio_output`.
    Capture,
}

struct Mixer {
    tracks: HashMap<(u64, u64), MixTrack>,
    output: Output,
}

static MIXER: Mutex<Option<Mixer>> = Mutex::new(None);

fn with_mixer<R>(f: impl FnOnce(&mut Mixer) -> R) -> R {
    let mut m = MIXER.lock().unwrap_or_else(|e| e.into_inner());
    f(m.get_or_insert_with(|| Mixer {
        tracks: HashMap::new(),
        output: Output::None,
    }))
}

thread_local! {
    /// The process's audio output (main thread: a cpal stream is not Send).
    static OUTPUT: RefCell<Option<rinch_av::audio::AudioOutputStream>> = const { RefCell::new(None) };
}

/// Decoded remote audio (48 kHz stereo) for track (peer, key).
pub(crate) fn mixer_push(peer: u64, key: u64, sink: &Arc<RemoteSink>, pcm: &[f32]) {
    let open = with_mixer(|m| {
        let t = m.tracks.entry((peer, key)).or_insert_with(|| MixTrack {
            sink: sink.clone(),
            buf: VecDeque::new(),
            primed: false,
        });
        t.buf.extend(pcm.iter().copied());
        while t.buf.len() > MAX_BUF {
            t.buf.pop_front();
        }
        if matches!(m.output, Output::None) {
            m.output = Output::Device;
            true
        } else {
            false
        }
    });
    if open {
        // Open the device on the main thread; failure (no device, as on a
        // CI runner) is said once and leaves decoding running.
        rinch_core::dispatch_main_callback(Box::new(move || open_output(peer)));
    }
}

fn open_output(peer: u64) {
    if OUTPUT.with(|o| o.borrow().is_some()) {
        return;
    }
    let rate = Arc::new(AtomicU32::new(48_000));
    let channels = Arc::new(AtomicU32::new(2));
    let (r2, c2) = (rate.clone(), channels.clone());
    let mut resampler: Option<Resampler> = None;
    let mut mixed: Vec<f32> = Vec::new();
    let mut converted: Vec<f32> = Vec::new();
    let fill = move |out: &mut [f32]| {
        let dev_rate = r2.load(Ordering::Relaxed);
        let dev_ch = c2.load(Ordering::Relaxed).max(1) as usize;
        let frames = out.len() / dev_ch;
        let rs = resampler.get_or_insert_with(|| Resampler::new(48_000, dev_rate, 2));
        // Pull enough 48 kHz stereo for `frames` device frames.
        while converted.len() < frames * 2 {
            let want =
                ((frames * 2 - converted.len()) / 2 * 48_000 / dev_rate as usize + 2).max(32);
            mixed.clear();
            mix(want, &mut mixed);
            rs.process(&mixed, &mut converted);
        }
        for f in 0..frames {
            let (l, r) = (converted[f * 2], converted[f * 2 + 1]);
            for c in 0..dev_ch {
                out[f * dev_ch + c] = match (dev_ch, c) {
                    (1, _) => (l + r) * 0.5,
                    (_, 0) => l,
                    (_, 1) => r,
                    _ => 0.0,
                };
            }
        }
        converted.drain(..frames * 2);
    };
    match rinch_av::audio::open_audio_output(rinch_av::audio::AudioOutputConfig::default(), fill) {
        Ok(stream) => {
            rate.store(stream.config().sample_rate, Ordering::Relaxed);
            channels.store(u32::from(stream.config().channels), Ordering::Relaxed);
            OUTPUT.with(|o| *o.borrow_mut() = Some(stream));
        }
        Err(e) => {
            with_mixer(|m| {
                if matches!(m.output, Output::Device) {
                    m.output = Output::Failed;
                }
            });
            crate::registry::dispatch(
                peer,
                crate::registry::Event::Error(RtcError::Media(format!(
                    "remote audio cannot play: no audio output ({e})"
                ))),
            );
        }
    }
}

/// Mix `frames` frames of 48 kHz stereo from every primed track into `out`.
fn mix(frames: usize, out: &mut Vec<f32>) {
    out.resize(frames * 2, 0.0);
    out.iter_mut().for_each(|s| *s = 0.0);
    with_mixer(|m| {
        for t in m.tracks.values_mut() {
            if !t.primed {
                if t.buf.len() >= PRIME {
                    t.primed = true;
                } else {
                    continue;
                }
            }
            let v = t.sink.volume();
            let n = (frames * 2).min(t.buf.len());
            for (i, s) in t.buf.drain(..n).enumerate() {
                out[i] += s * v;
            }
            if n < frames * 2 {
                t.primed = false;
            }
        }
    });
    for s in out.iter_mut() {
        *s = s.clamp(-1.0, 1.0);
    }
}

pub(crate) fn mixer_remove(peer: u64, key: u64) {
    with_mixer(|m| m.tracks.remove(&(peer, key)));
}

fn mixer_remove_peer(peer: u64) {
    with_mixer(|m| m.tracks.retain(|(p, _), _| *p != peer));
}

// ================================================================ the view

thread_local! {
    /// View id → its element and what it shows (main thread).
    static VIEWS: RefCell<HashMap<String, NodeHandle>> = RefCell::new(HashMap::new());
}

fn view_ready(ids: &[String]) {
    VIEWS.with(|v| {
        for id in ids {
            if let Some(el) = v.borrow().get(id) {
                el.set_attribute("data-viewport-ready", "true");
                el.set_style("background-color", "transparent");
            }
        }
    });
}

/// A div carrying `data-viewport` (the compositor's key for a video surface)
/// and `data-rtc-view` (the same identity the web's `<video>` carries), fed
/// through rinch-core's named video sink. Black until the first frame.
pub(crate) fn create_view(scope: &mut RenderScope, view_id: &str, t: &MediaTrack) -> NodeHandle {
    let el = scope.create_element("div");
    el.set_attribute("data-rtc-view", view_id);
    el.set_attribute("data-viewport", view_id);
    el.set_attribute("data-viewport-ready", "false");
    el.set_style("pointer-events", "none");
    let Some(sink) = rinch_core::video_sink::create_video_sink(view_id) else {
        // No shell (a test, a headless tool): nothing to draw into.
        return el;
    };
    let id = view_id.to_string();
    let attached = match t {
        MediaTrack::None => false,
        MediaTrack::Local(l) => local_source(l.key())
            .and_then(|s| {
                s.views
                    .lock()
                    .ok()
                    .map(|mut v| v.push((id.clone(), sink.clone())))
            })
            .is_some(),
        MediaTrack::Remote(r) => remote(r.peer, r.key)
            .and_then(|s| {
                s.shown.store(false, Ordering::Release);
                s.views
                    .lock()
                    .ok()
                    .map(|mut v| v.push((id.clone(), sink.clone())))
            })
            .is_some(),
    };
    if !attached {
        return el;
    }
    VIEWS.with(|v| v.borrow_mut().insert(id.clone(), el.clone()));
    let track = t.clone();
    scope.on_cleanup(move || {
        VIEWS.with(|v| v.borrow_mut().remove(&id));
        // The sink goes with its last clone, which unregisters the surface.
        match &track {
            MediaTrack::Local(l) => {
                if let Some(s) = local_source(l.key())
                    && let Ok(mut v) = s.views.lock()
                {
                    v.retain(|(vid, _)| *vid != id);
                }
            }
            MediaTrack::Remote(r) => {
                if let Some(s) = remote(r.peer, r.key)
                    && let Ok(mut v) = s.views.lock()
                {
                    v.retain(|(vid, _)| *vid != id);
                }
            }
            MediaTrack::None => {}
        }
    });
    el
}

pub(crate) async fn devices(kind: MediaKind) -> Vec<DeviceInfo> {
    let list = match kind {
        MediaKind::Video => rinch_av::camera::camera_devices(),
        MediaKind::Audio => rinch_av::audio::audio_input_devices(),
    };
    list.unwrap_or_default()
        .into_iter()
        .map(|d| DeviceInfo {
            id: d.id.0,
            name: d.name,
        })
        .collect()
}

// ======================================================== worker commands

/// Media commands for a peer's task on the worker (native.rs).
pub(crate) enum MediaCmd {
    AddTrack {
        sender: u64,
        tx: u64,
        source: Arc<LocalSource>,
    },
    AddTransceiver {
        kind: MediaKind,
        direction: Direction,
        sender: u64,
        tx: u64,
    },
    ReplaceTrack {
        req: u64,
        sender: u64,
        source: Option<Arc<LocalSource>>,
    },
    SetDirection {
        tx: u64,
        direction: Direction,
    },
    StopTransceiver {
        tx: u64,
    },
}

#[doc(hidden)]
pub mod testing {
    //! Hooks for rinch-rtc's own tests (not API).
    use std::sync::{Arc, Mutex};

    /// Collect the mixed remote audio (48 kHz stereo) instead of opening an
    /// output device. Call before any remote audio arrives.
    pub fn capture_audio_output() -> Arc<Mutex<Vec<f32>>> {
        let buf: Arc<Mutex<Vec<f32>>> = Arc::default();
        super::with_mixer(|m| m.output = super::Output::Capture);
        let b = buf.clone();
        std::thread::spawn(move || {
            let mut chunk = Vec::new();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(20));
                super::mix(960, &mut chunk);
                match b.lock() {
                    Ok(mut v) => v.extend_from_slice(&chunk),
                    Err(_) => return,
                }
                if Arc::strong_count(&b) == 1 {
                    return;
                }
            }
        });
        buf
    }

    /// The bitrate a local video track's encoder targets at `w`×`h`: the
    /// lowest `set_max_bitrate` of its senders, or the default.
    pub fn encoder_target_bitrate(track: &crate::media::LocalTrack, w: u32, h: u32) -> u32 {
        super::local_source(track.key())
            .map(|s| s.bitrate(w, h))
            .unwrap_or(0)
    }

    /// Frames a video source took into its track (painted or encoded).
    pub fn video_frames_taken(writer: &crate::media::VideoSourceWriter) -> u64 {
        writer.slot.taken.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Samples an audio source took off its queue.
    pub fn audio_samples_taken(writer: &crate::media::AudioSourceWriter) -> u64 {
        writer
            .queue
            .taken
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Register a frame sink for `viewport` the way a shell would (tests of
    /// `MediaView` without a window).
    pub fn install_video_sink_factory(
        f: impl Fn(&str) -> rinch_core::video_sink::VideoFrameSink + Send + Sync + 'static,
    ) {
        rinch_core::video_sink::set_video_sink_factory(f);
    }
}

#[cfg(test)]
mod tests {
    use super::Resampler;

    #[test]
    fn the_resampler_keeps_length_and_phase_across_chunks() {
        // 1 s of 44.1 kHz mono in 441-sample chunks gives 1 s at 48 kHz.
        let mut r = Resampler::new(44_100, 48_000, 1);
        let mut out = Vec::new();
        let tone: Vec<f32> = (0..44_100)
            .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 44_100.0).sin())
            .collect();
        for c in tone.chunks(441) {
            r.process(c, &mut out);
        }
        assert!((out.len() as i64 - 48_000).abs() <= 2, "{}", out.len());
        // Still a clean 440 Hz: compare with the ideal at a few points.
        for i in [100usize, 12_345, 40_000] {
            let ideal =
                ((i as f32 - 48_000.0 / 44_100.0) * 440.0 * std::f32::consts::TAU / 48_000.0).sin();
            assert!(
                (out[i] - ideal).abs() < 0.08,
                "sample {i}: {} vs {ideal}",
                out[i]
            );
        }
        // Same rate: a copy.
        let mut same = Resampler::new(48_000, 48_000, 2);
        let mut o = Vec::new();
        same.process(&[0.1, 0.2, 0.3, 0.4], &mut o);
        assert_eq!(o, vec![0.1, 0.2, 0.3, 0.4]);
    }
}

// ============================================================ the worker

pub(crate) mod worker {
    //! A peer's media on the rinch-rtc worker: its senders' tracks, the
    //! transceiver keys, and the remote tracks' readers.

    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::time::Instant;

    use rtc::interceptor::Registry;
    use rtc::media::Sample;
    use rtc::media::io::sample_builder::SampleBuilder;
    use rtc::media_stream::MediaStreamTrack;
    use rtc::peer_connection::configuration::interceptor_registry::register_default_interceptors;
    use rtc::peer_connection::configuration::media_engine::{
        MIME_TYPE_H264, MIME_TYPE_OPUS, MIME_TYPE_RTX, MediaEngine,
    };
    use rtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
    use rtc::rtp::codec::h264::H264Packet;
    use rtc::rtp_transceiver::rtp_sender::{
        RTCPFeedback, RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters,
        RTCRtpEncodingParameters, RtpCodecKind,
    };
    use rtc::rtp_transceiver::{
        RTCRtpTransceiverDirection, RTCRtpTransceiverId, RTCRtpTransceiverInit,
    };
    use webrtc::media_stream::Track;
    use webrtc::media_stream::track_local::TrackLocal;
    use webrtc::media_stream::track_local::static_sample::TrackLocalStaticSample;
    use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
    use webrtc::peer_connection::PeerConnection;
    use webrtc::rtp_transceiver::{RtpSender, RtpTransceiver};

    use super::{
        Encoded, LocalSource, MediaCmd, RemoteSink, TxMirror, drop_remotes, insert_remote,
        mixer_push, mixer_remove, remote, start_video_decoder, with_mirror,
    };
    use crate::RtcError;
    use crate::media::{Direction, MediaEvent, MediaKind};
    use crate::registry::Event;

    const H264_PT: u8 = 102;
    const OPUS_PT: u8 = 111;

    fn h264() -> RTCRtpCodec {
        RTCRtpCodec {
            mime_type: MIME_TYPE_H264.to_owned(),
            clock_rate: 90_000,
            channels: 0,
            sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"
                .to_owned(),
            rtcp_feedback: vec![
                RTCPFeedback {
                    typ: "nack".to_owned(),
                    parameter: String::new(),
                },
                RTCPFeedback {
                    typ: "nack".to_owned(),
                    parameter: "pli".to_owned(),
                },
                RTCPFeedback {
                    typ: "ccm".to_owned(),
                    parameter: "fir".to_owned(),
                },
            ],
        }
    }

    fn opus() -> RTCRtpCodec {
        RTCRtpCodec {
            mime_type: MIME_TYPE_OPUS.to_owned(),
            clock_rate: 48_000,
            channels: 2,
            sdp_fmtp_line: "minptime=10;useinbandfec=1".to_owned(),
            rtcp_feedback: vec![],
        }
    }

    /// The codecs a media peer offers and accepts: H.264 constrained
    /// baseline (with its RTX) and Opus. Nothing else: the native side can
    /// decode nothing else (M2).
    pub(crate) fn media_engine() -> Result<(MediaEngine, Registry), RtcError> {
        let mut me = MediaEngine::default();
        let err = |e: rtc::shared::error::Error| RtcError::Internal(format!("media engine: {e}"));
        me.register_codec(
            RTCRtpCodecParameters {
                rtp_codec: opus(),
                payload_type: OPUS_PT,
            },
            RtpCodecKind::Audio,
        )
        .map_err(err)?;
        me.register_codec(
            RTCRtpCodecParameters {
                rtp_codec: h264(),
                payload_type: H264_PT,
            },
            RtpCodecKind::Video,
        )
        .map_err(err)?;
        me.register_codec(
            RTCRtpCodecParameters {
                rtp_codec: RTCRtpCodec {
                    mime_type: MIME_TYPE_RTX.to_owned(),
                    clock_rate: 90_000,
                    channels: 0,
                    sdp_fmtp_line: format!("apt={H264_PT}"),
                    rtcp_feedback: vec![],
                },
                payload_type: H264_PT + 1,
            },
            RtpCodecKind::Video,
        )
        .map_err(err)?;
        let registry = register_default_interceptors(Registry::new(), &mut me).map_err(err)?;
        Ok((me, registry))
    }

    fn codec_kind(k: MediaKind) -> RtpCodecKind {
        match k {
            MediaKind::Audio => RtpCodecKind::Audio,
            MediaKind::Video => RtpCodecKind::Video,
        }
    }

    fn to_dir(d: Direction) -> RTCRtpTransceiverDirection {
        match d {
            Direction::SendRecv => RTCRtpTransceiverDirection::Sendrecv,
            Direction::SendOnly => RTCRtpTransceiverDirection::Sendonly,
            Direction::RecvOnly => RTCRtpTransceiverDirection::Recvonly,
            Direction::Inactive => RTCRtpTransceiverDirection::Inactive,
        }
    }

    fn from_dir(d: RTCRtpTransceiverDirection) -> Option<Direction> {
        match d {
            RTCRtpTransceiverDirection::Sendrecv => Some(Direction::SendRecv),
            RTCRtpTransceiverDirection::Sendonly => Some(Direction::SendOnly),
            RTCRtpTransceiverDirection::Recvonly => Some(Direction::RecvOnly),
            RTCRtpTransceiverDirection::Inactive => Some(Direction::Inactive),
            _ => None,
        }
    }

    struct SenderState {
        rtp: Arc<dyn RtpSender>,
        /// The one track this sender writes, for its whole life: a
        /// `replace_track` re-points the forwarder from one source to
        /// another (the SSRC and sequence numbers run on, as a browser's
        /// replaceTrack does). webrtc-rs's own `replace_track` would leave
        /// the new track unbound, and its writes fail.
        track: Option<Arc<TrackLocalStaticSample>>,
        /// The negotiated payload type, once a forwarder has learnt it.
        pt: Arc<std::sync::atomic::AtomicU32>,
        kind: MediaKind,
        source: Option<Arc<LocalSource>>,
        forward: Option<tokio::task::JoinHandle<()>>,
    }

    pub(crate) struct PeerWorker {
        peer: u64,
        stream_id: String,
        senders: HashMap<u64, SenderState>,
        tx_keys: HashMap<RTCRtpTransceiverId, (u64, u64)>,
        /// Remote tracks by the stack's track id: a track that comes back
        /// (its transceiver inactive, then active) is the same RemoteTrack.
        remote_keys: HashMap<String, u64>,
        /// Each remote track's transceiver and whether it is muted: webrtc-rs
        /// reports no mute when negotiation stops a track's media, so the
        /// worker does, as browsers do (and unmutes on the next packet).
        remote_state: HashMap<u64, (RTCRtpTransceiverId, Arc<std::sync::atomic::AtomicBool>)>,
        readers: Vec<tokio::task::JoinHandle<()>>,
    }

    fn emit(id: u64, event: Event) {
        rinch_core::dispatch_main_callback(Box::new(move || crate::registry::dispatch(id, event)));
    }

    fn track_for(
        kind: MediaKind,
        track_id: &str,
        stream_id: &str,
        ssrc: u32,
    ) -> Option<Arc<TrackLocalStaticSample>> {
        let codec = match kind {
            MediaKind::Video => h264(),
            MediaKind::Audio => opus(),
        };
        TrackLocalStaticSample::new(
            Instant::now(),
            MediaStreamTrack::new(
                stream_id.to_owned(),
                track_id.to_owned(),
                track_id.to_owned(),
                codec_kind(kind),
                vec![RTCRtpEncodingParameters {
                    rtp_coding_parameters: RTCRtpCodingParameters {
                        ssrc: Some(ssrc),
                        ..Default::default()
                    },
                    codec,
                    ..Default::default()
                }],
            ),
        )
        .ok()
        .map(Arc::new)
    }

    fn rand_ssrc() -> u32 {
        // Unique enough per process: a counter mixed with the clock.
        use std::hash::{BuildHasher, Hasher};
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(crate::next_id());
        (h.finish() as u32) | 1
    }

    /// Write a source's encoded frames to one sender's track until either
    /// goes away.
    async fn forward(
        rtp: Arc<dyn RtpSender>,
        track: Arc<TrackLocalStaticSample>,
        source: Arc<LocalSource>,
        mut rx: tokio::sync::mpsc::Receiver<Encoded>,
        known_pt: Arc<std::sync::atomic::AtomicU32>,
    ) {
        let Some(ssrc) = track.ssrcs().await.first().copied() else {
            return;
        };
        // Known from an earlier track on this sender: webrtc-rs reports no
        // codecs for a replaced track.
        let mut pt: Option<u8> = match known_pt.load(Ordering::Acquire) {
            0 => None,
            p => Some(p as u8),
        };
        while let Some(f) = rx.recv().await {
            if pt.is_none() {
                // The payload type is known once negotiation chose a codec;
                // until then frames are dropped, and the first one after
                // is a keyframe.
                pt = rtp
                    .get_parameters()
                    .await
                    .ok()
                    .and_then(|p| p.rtp_parameters.codecs.first().map(|c| c.payload_type));
                let Some(p) = pt else {
                    continue;
                };
                known_pt.store(u32::from(p), Ordering::Release);
                source.request_keyframe();
            }
            let sample = Sample {
                data: f.data,
                duration: f.duration,
                ..Sample::new(Instant::now())
            };
            if let Some(pt) = pt
                && track.write_sample(ssrc, pt, &sample, &[]).await.is_err()
            {
                // The sender has gone (replaced, stopped).
                break;
            }
        }
    }

    impl PeerWorker {
        pub(crate) fn new(peer: u64) -> Self {
            Self {
                peer,
                stream_id: format!("rinch-{peer}"),
                senders: HashMap::new(),
                tx_keys: HashMap::new(),
                remote_keys: HashMap::new(),
                remote_state: HashMap::new(),
                readers: Vec::new(),
            }
        }

        fn start_forward(
            &mut self,
            sender: u64,
            rtp: Arc<dyn RtpSender>,
            track: Arc<TrackLocalStaticSample>,
            source: Arc<LocalSource>,
            pt: Arc<std::sync::atomic::AtomicU32>,
        ) -> tokio::task::JoinHandle<()> {
            let rx = source.subscribe((self.peer, sender));
            tokio::spawn(forward(rtp, track, source, rx, pt))
        }

        pub(crate) async fn handle(&mut self, pc: &Arc<dyn PeerConnection>, cmd: MediaCmd) {
            match cmd {
                MediaCmd::AddTrack { sender, tx, source } => {
                    let kind = source.kind();
                    let Some(track) = track_for(kind, source.id(), &self.stream_id, rand_ssrc())
                    else {
                        emit(
                            self.peer,
                            Event::Error(RtcError::Media("could not make the track".into())),
                        );
                        return;
                    };
                    match pc.add_track(track.clone() as Arc<dyn TrackLocal>).await {
                        Ok(rtp) => {
                            for t in pc.get_transceivers().await {
                                if let Ok(Some(s)) = t.sender().await
                                    && s.id() == rtp.id()
                                {
                                    self.tx_keys.insert(t.id(), (tx, sender));
                                }
                            }
                            let pt = Arc::new(std::sync::atomic::AtomicU32::new(0));
                            let forward = self.start_forward(
                                sender,
                                rtp.clone(),
                                track.clone(),
                                source.clone(),
                                pt.clone(),
                            );
                            self.senders.insert(
                                sender,
                                SenderState {
                                    rtp,
                                    track: Some(track),
                                    pt,
                                    kind,
                                    source: Some(source),
                                    forward: Some(forward),
                                },
                            );
                        }
                        Err(e) => emit(
                            self.peer,
                            Event::Error(RtcError::Media(format!("add_track: {e}"))),
                        ),
                    }
                    self.refresh(pc).await;
                }
                MediaCmd::AddTransceiver {
                    kind,
                    direction,
                    sender,
                    tx,
                } => {
                    let init = RTCRtpTransceiverInit {
                        direction: to_dir(direction),
                        streams: vec![self.stream_id.clone()],
                        send_encodings: vec![],
                    };
                    // With a track of our own from the start, so a later
                    // replace_track has one to write.
                    let id = format!("rinch-none-{sender}");
                    let Some(track) = track_for(kind, &id, &self.stream_id, rand_ssrc()) else {
                        emit(
                            self.peer,
                            Event::Error(RtcError::Media("could not make the track".into())),
                        );
                        return;
                    };
                    match pc
                        .add_transceiver_from_track(
                            track.clone() as Arc<dyn TrackLocal>,
                            Some(init),
                        )
                        .await
                    {
                        Ok(t) => {
                            self.tx_keys.insert(t.id(), (tx, sender));
                            if let Ok(Some(rtp)) = t.sender().await {
                                self.senders.insert(
                                    sender,
                                    SenderState {
                                        rtp,
                                        track: Some(track),
                                        pt: Arc::default(),
                                        kind,
                                        source: None,
                                        forward: None,
                                    },
                                );
                            }
                        }
                        Err(e) => emit(
                            self.peer,
                            Event::Error(RtcError::Media(format!("add_transceiver: {e}"))),
                        ),
                    }
                    self.refresh(pc).await;
                }
                MediaCmd::ReplaceTrack {
                    req,
                    sender,
                    source,
                } => {
                    let result = self.replace(sender, source).await;
                    emit(
                        self.peer,
                        Event::Reply {
                            req,
                            result: result.map(|()| None),
                        },
                    );
                }
                MediaCmd::SetDirection { tx, direction } => {
                    if let Some(t) = self.transceiver(pc, tx).await
                        && let Err(e) = t.set_direction(to_dir(direction)).await
                    {
                        emit(
                            self.peer,
                            Event::Error(RtcError::Media(format!("set_direction: {e}"))),
                        );
                    }
                }
                MediaCmd::StopTransceiver { tx } => {
                    if let Some(t) = self.transceiver(pc, tx).await {
                        let _ = t.stop().await;
                    }
                }
            }
        }

        async fn replace(
            &mut self,
            sender: u64,
            source: Option<Arc<LocalSource>>,
        ) -> Result<(), RtcError> {
            let peer = self.peer;
            let stream_id = self.stream_id.clone();
            let st = self
                .senders
                .get_mut(&sender)
                .ok_or_else(|| RtcError::InvalidState("no such sender".into()))?;
            if let Some(src) = &source
                && src.kind() != st.kind
            {
                return Err(RtcError::InvalidState(
                    "replace_track: a track of another kind".into(),
                ));
            }
            let track = match &st.track {
                Some(t) => t.clone(),
                None => {
                    // A transceiver the far side's offer made: give its
                    // sender a track of ours (webrtc-rs binds it at the
                    // next negotiation).
                    let id = format!("rinch-none-{sender}");
                    let t = track_for(st.kind, &id, &stream_id, rand_ssrc())
                        .ok_or_else(|| RtcError::Internal("could not make the track".into()))?;
                    st.rtp
                        .replace_track(t.clone() as Arc<dyn TrackLocal>)
                        .await
                        .map_err(|e| RtcError::InvalidState(format!("replace_track: {e}")))?;
                    st.track = Some(t.clone());
                    t
                }
            };
            if let Some(f) = st.forward.take() {
                f.abort();
            }
            if let Some(old) = st.source.take() {
                old.unsubscribe((peer, sender));
            }
            if let Some(src) = source {
                let rx = src.subscribe((peer, sender));
                st.forward = Some(tokio::spawn(forward(
                    st.rtp.clone(),
                    track,
                    src.clone(),
                    rx,
                    st.pt.clone(),
                )));
                st.source = Some(src);
            }
            Ok(())
        }

        async fn transceiver(
            &self,
            pc: &Arc<dyn PeerConnection>,
            key: u64,
        ) -> Option<Arc<dyn RtpTransceiver>> {
            let id = self
                .tx_keys
                .iter()
                .find(|(_, (k, _))| *k == key)
                .map(|(id, _)| *id)?;
            pc.get_transceivers()
                .await
                .into_iter()
                .find(|t| t.id() == id)
        }

        async fn kind_of(t: &Arc<dyn RtpTransceiver>) -> MediaKind {
            if let Ok(Some(r)) = t.receiver().await
                && r.track().kind().await == RtpCodecKind::Audio
            {
                return MediaKind::Audio;
            }
            if let Ok(Some(s)) = t.sender().await
                && s.track().kind().await == RtpCodecKind::Audio
            {
                return MediaKind::Audio;
            }
            MediaKind::Video
        }

        /// Bring the main thread's mirror up to date: new transceivers (a
        /// remote offer's), mids, negotiated directions. The direction
        /// asked for is the main thread's own and is left alone.
        pub(crate) async fn refresh(&mut self, pc: &Arc<dyn PeerConnection>) {
            let mut fresh: Vec<TxMirror> = Vec::new();
            for t in pc.get_transceivers().await {
                let (key, sender) = *self
                    .tx_keys
                    .entry(t.id())
                    .or_insert_with(|| (crate::next_id(), crate::next_id()));
                if !self.senders.contains_key(&sender)
                    && let Ok(Some(rtp)) = t.sender().await
                {
                    let kind = Self::kind_of(&t).await;
                    self.senders.insert(
                        sender,
                        SenderState {
                            rtp,
                            track: None,
                            pt: Arc::default(),
                            kind,
                            source: None,
                            forward: None,
                        },
                    );
                }
                fresh.push(TxMirror {
                    key,
                    sender,
                    kind: Self::kind_of(&t).await,
                    direction: t
                        .direction()
                        .await
                        .ok()
                        .and_then(from_dir)
                        .unwrap_or(Direction::Inactive),
                    current: t.current_direction().await.ok().and_then(from_dir),
                    mid: t.mid().await.ok().flatten(),
                    stopped: false,
                });
            }
            // Remote tracks whose media negotiation stopped: muted.
            for t in pc.get_transceivers().await {
                let receiving = matches!(
                    t.current_direction().await.ok().and_then(from_dir),
                    Some(Direction::SendRecv) | Some(Direction::RecvOnly)
                );
                for (key, (tid, muted)) in &self.remote_state {
                    if *tid == t.id() && !receiving && !muted.swap(true, Ordering::AcqRel) {
                        emit(
                            self.peer,
                            Event::Media(MediaEvent::Muted {
                                key: *key,
                                muted: true,
                            }),
                        );
                    }
                }
            }
            with_mirror(self.peer, |m| {
                for f in fresh {
                    match m.iter_mut().find(|x| x.key == f.key) {
                        Some(x) => {
                            x.current = if x.stopped {
                                Some(Direction::Inactive)
                            } else {
                                f.current
                            };
                            x.mid = f.mid;
                        }
                        None => m.push(f),
                    }
                }
            });
        }

        /// A remote track arrived (webrtc-rs's `on_track`).
        pub(crate) async fn remote_track(
            &mut self,
            pc: &Arc<dyn PeerConnection>,
            track: Arc<dyn TrackRemote>,
        ) {
            let peer = self.peer;
            let track_id = track.track_id().await;
            let kind = if track.kind().await == RtpCodecKind::Audio {
                MediaKind::Audio
            } else {
                MediaKind::Video
            };
            // The same track again: one RemoteTrack (B1's rule, keyed here
            // on the stack's track id). A new reader takes over.
            let known = self
                .remote_keys
                .get(&track_id)
                .copied()
                .filter(|k| remote(peer, *k).is_some_and(|s| !s.ended.load(Ordering::Acquire)));
            let key = match known {
                Some(k) => k,
                None => {
                    let key = crate::next_id();
                    self.remote_keys.insert(track_id.clone(), key);
                    let mut tx_key = 0;
                    self.refresh(pc).await;
                    for t in pc.get_transceivers().await {
                        if let Ok(Some(r)) = t.receiver().await
                            && r.track().track_id().await == track_id
                        {
                            tx_key = self.tx_keys.get(&t.id()).map(|(k, _)| *k).unwrap_or(0);
                            self.remote_state.insert(key, (t.id(), Arc::default()));
                        }
                    }
                    let sink = RemoteSink::new();
                    insert_remote(peer, key, sink);
                    emit(
                        peer,
                        Event::Media(MediaEvent::Track {
                            key,
                            kind,
                            id: track_id,
                            stream_ids: vec![track.stream_id().await],
                            transceiver: tx_key,
                            muted: false,
                        }),
                    );
                    key
                }
            };
            let Some(sink) = remote(peer, key) else {
                return;
            };
            let muted = self
                .remote_state
                .get(&key)
                .map(|(_, m)| m.clone())
                .unwrap_or_default();
            self.readers.retain(|r| !r.is_finished());
            self.readers.push(tokio::spawn(read_remote(
                peer, key, kind, track, sink, muted,
            )));
        }

        /// The peer is closing: stop sending and reading.
        pub(crate) fn close(&mut self) {
            for (sender, st) in self.senders.drain() {
                if let Some(f) = st.forward {
                    f.abort();
                }
                if let Some(src) = st.source {
                    src.unsubscribe((self.peer, sender));
                }
            }
            for r in self.readers.drain(..) {
                r.abort();
            }
            drop_remotes(self.peer);
        }
    }

    /// Read one remote track until it ends: video to its decoder thread,
    /// audio through Opus into the mixer, mute and end to the main thread.
    async fn read_remote(
        peer: u64,
        key: u64,
        kind: MediaKind,
        track: Arc<dyn TrackRemote>,
        sink: Arc<RemoteSink>,
        muted: Arc<std::sync::atomic::AtomicBool>,
    ) {
        let (pli_tx, mut pli_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let decoder = match kind {
            MediaKind::Video => start_video_decoder(key, sink.clone(), pli_tx),
            MediaKind::Audio => None,
        };
        let mut builder = SampleBuilder::new(128, H264Packet::default(), 90_000);
        let mut opus = match kind {
            MediaKind::Audio => opus::Decoder::new(48_000, opus::Channels::Stereo).ok(),
            MediaKind::Video => None,
        };
        let mut pcm = vec![0f32; 5760 * 2];
        let mut ssrc = 0u32;
        loop {
            tokio::select! {
                ev = track.poll() => {
                    let Some(ev) = ev else { break };
                    match ev {
                        TrackRemoteEvent::OnRtpPacket(p) => {
                            ssrc = p.header.ssrc;
                            if muted.swap(false, Ordering::AcqRel) {
                                emit(peer, Event::Media(MediaEvent::Muted { key, muted: false }));
                            }
                            match kind {
                                MediaKind::Video => {
                                    builder.push(Instant::now(), p);
                                    while let Some(s) = builder.pop(Instant::now()) {
                                        if let Some(d) = &decoder {
                                            let _ = d.try_send(s.data);
                                        }
                                    }
                                }
                                MediaKind::Audio => {
                                    if let Some(dec) = opus.as_mut()
                                        && let Ok(n) = dec.decode_float(&p.payload, &mut pcm, false)
                                    {
                                        mixer_push(peer, key, &sink, &pcm[..n * 2]);
                                    }
                                }
                            }
                        }
                        TrackRemoteEvent::OnMute => {
                            muted.store(true, Ordering::Release);
                            emit(peer, Event::Media(MediaEvent::Muted { key, muted: true }));
                        }
                        TrackRemoteEvent::OnUnmute => {
                            muted.store(false, Ordering::Release);
                            emit(peer, Event::Media(MediaEvent::Muted { key, muted: false }));
                        }
                        TrackRemoteEvent::OnEnded => break,
                        _ => {}
                    }
                }
                Some(()) = pli_rx.recv() => {
                    if ssrc != 0 {
                        let _ = track
                            .write_rtcp(vec![Box::new(PictureLossIndication { sender_ssrc: 0, media_ssrc: ssrc })])
                            .await;
                    }
                }
            }
        }
        sink.ended.store(true, Ordering::Release);
        mixer_remove(peer, key);
        emit(peer, Event::Media(MediaEvent::Ended { key }));
    }
}
