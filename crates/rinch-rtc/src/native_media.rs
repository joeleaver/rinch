//! Native media backend (design record `docs/design/rtc-media.md`, section 3):
//! webrtc-rs negotiates and carries RTP; openh264 (H.264), libvpx (VP8, with
//! the `media-vp8` feature) and Opus encode and decode; rinch-av captures and plays; and received video
//! reaches `MediaView` through rinch-core's named video sink (M6).
//!
//! Threads:
//! - **The main thread** keeps the local tracks' sources (`LOCALS`), the
//!   capture handles, and the view elements.
//! - **One pump thread per local track** (`rinch-rtc-enc-<key>`): video at
//!   the track's fps (latest frame, RGBA→I420, then one encode per codec the
//!   track's senders negotiated), audio every 20 ms (resampled to 48 kHz,
//!   Opus). It feeds the track's self-views and every sender carrying the
//!   track: one encoder per codec however many peers send it.
//! - **The rinch-rtc worker** (native.rs) owns the peer connections: it
//!   packetizes each sender's encoded frames into RTP, forwards PLIs to the
//!   pumps, and reads each remote track's RTP.
//! - **One decoder thread per remote video track** (`rinch-rtc-dec-<key>`):
//!   H.264 or VP8, whichever the stream carries, I420→RGBA, then the views
//!   and `on_frame`. A decode error or a gap sends a PLI.
//! - **Remote audio** is decoded on the worker (Opus is cheap), buffered per
//!   track (60 ms before it starts; Opus FEC or PLC for a lost packet),
//!   mixed, and played through one rinch-av output stream for the process.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
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
use crate::native_codec::{I420, VideoCodec, VideoEncoder};

/// One encoded frame for the senders.
#[derive(Clone)]
pub(crate) struct Encoded {
    pub(crate) data: bytes::Bytes,
    pub(crate) duration: Duration,
    /// Video: which codec this frame is in (one encode per codec a sender
    /// negotiated). `None` for audio (always Opus).
    pub(crate) codec: Option<VideoCodec>,
    /// The rate its packets go out at, bits a second (0: at once). A large
    /// frame sent in one burst lost packets on the far side's socket, even
    /// on loopback.
    pub(crate) pace_bps: u32,
}

/// How many encoded frames a sender may have waiting before new ones are
/// dropped (a stalled peer must not grow memory).
const SENDER_QUEUE: usize = 32;

// ================================================================ H.264

/// Cisco's OpenH264 2.6.0 builds rinch-rtc accepts, by SHA-256: the one for
/// this platform only (from openh264-sys2 0.9.9's list of known builds).
#[cfg(feature = "media-cisco-openh264")]
const CISCO_SHA256: Option<&str> = if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    Some("2f0cde7c6a6abcf5cae76942894ea42897fa677bce4ed6c91a24dd1b041d5f04")
} else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
    Some("12e7b33623667cdab0e575170c147b1b36eadb77d0d2aa7ceb5afd3e58902140")
} else if cfg!(all(target_os = "linux", target_arch = "x86")) {
    Some("a46589ccc95df7565ff8b1722d3dead29c0809be28322dc763767e0aa35a6443")
} else if cfg!(all(target_os = "linux", target_arch = "arm")) {
    Some("df91866de0e93773019e30a8f2bdee8b15de4abe2bf89a228ae9f064ff1e85bb")
} else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    Some("052e98bfcf7a9167d22f3bbb3f5988ef79065591f36af8b52924b22b13624551")
} else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
    Some("e3dc8bc01fe69363f61fd3c02fd27798537a585eadd38cd808f303d1ee505a19")
} else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
    Some("2076cb5675ec6c1a4c70e7a2a322552f547b6eeed649d6dfcd9e02a543b24691")
} else if cfg!(all(target_os = "windows", target_arch = "x86")) {
    Some("b0098db6acbd290a1fe13997d61d461e7327e39b42bf868db41faf498b7621a2")
} else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
    Some("fb75103938f4f47d119b983e06334df41a803bc72fb5c46e3623f6fea5782732")
} else if cfg!(all(target_os = "android", target_arch = "aarch64")) {
    Some("4d9bc54d2d38e53eb7bd551ec61acb8ad8320d8b957bda751cfdbdcbfabc3b07")
} else if cfg!(all(target_os = "android", target_arch = "arm")) {
    Some("45d8d77c119a488cad42549987a4a1ac50f634cfd6b08454c5565652e482022c")
} else if cfg!(all(target_os = "android", target_arch = "x86_64")) {
    Some("548fa57935ff0eaf0a1d44c7963c1681daa5bf4cef2b34029fcd20a1e71adf81")
} else if cfg!(all(target_os = "android", target_arch = "x86")) {
    Some("f8433906e94e719bed537b54e0cd5ae573798cdc3a16a567a25336e8597444bc")
} else {
    None
};

/// The loaded library: its canonical path, and the first handle, kept open
/// for the life of the process. Every encoder and decoder opens the same
/// canonical path again, which the dynamic loader answers with this
/// already-resident image (a reference count), never by reading the file.
#[cfg(feature = "media-cisco-openh264")]
struct Cisco {
    path: std::path::PathBuf,
    _resident: Mutex<openh264::OpenH264API>,
}

#[cfg(feature = "media-cisco-openh264")]
static CISCO: std::sync::OnceLock<Cisco> = std::sync::OnceLock::new();

/// Use Cisco's prebuilt OpenH264 library for H.264 (the
/// `media-cisco-openh264` feature; M3).
///
/// - `path` is the library the app downloaded from Cisco itself, separately
///   from the app, as Cisco's licence requires (Cisco ships it bzip2'd;
///   decompress it first). It must be **OpenH264 2.6.0** for this platform:
///   the file's SHA-256 is checked against that one build.
/// - The path is made absolute (canonicalised) before anything reads it; a
///   bare file name is not looked up on the system's library path.
/// - **A window remains** between the hash and the load: the file is read
///   and hashed, then the dynamic loader opens it again by that path (it
///   cannot load from memory). Whoever can replace the file in that moment
///   can have other code loaded. Keep the library where only the app's
///   own user can write, as for any library the app loads.
/// - Call it once. A second call with the same file is a no-op; with a
///   different one it is an error.
/// - Until it has succeeded, desktop peers offer and accept no H.264. With
///   `media-vp8` as well they have VP8 meanwhile; without it, no video
///   codec until then.
#[cfg(feature = "media-cisco-openh264")]
pub fn load_openh264(path: impl AsRef<std::path::Path>) -> Result<(), MediaError> {
    let canon = std::fs::canonicalize(path.as_ref()).map_err(|e| {
        MediaError::NotSupported(format!("OpenH264 library {}: {e}", path.as_ref().display()))
    })?;
    if let Some(c) = CISCO.get() {
        return if c.path == canon {
            Ok(())
        } else {
            Err(MediaError::Backend(format!(
                "OpenH264 is already loaded from {}",
                c.path.display()
            )))
        };
    }
    let Some(want) = CISCO_SHA256 else {
        return Err(MediaError::NotSupported(
            "Cisco ships no OpenH264 2.6.0 build for this platform".into(),
        ));
    };
    let bytes = std::fs::read(&canon)
        .map_err(|e| MediaError::NotSupported(format!("OpenH264 library: {e}")))?;
    use sha2::Digest;
    let got: String = sha2::Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if got != want {
        return Err(MediaError::NotSupported(format!(
            "{} is not Cisco's OpenH264 2.6.0 for this platform (SHA-256 {got})",
            canon.display()
        )));
    }
    // SAFETY: the file at this canonical path was just checked to be the
    // known build for this platform (the window until the loader opens it
    // again is documented above).
    let api = unsafe { openh264::OpenH264API::from_blob_path_unchecked(&canon) }
        .map_err(|e| MediaError::NotSupported(format!("OpenH264 library: {e}")))?;
    let set = CISCO.set(Cisco {
        path: canon.clone(),
        _resident: Mutex::new(api),
    });
    if set.is_err() && CISCO.get().is_some_and(|c| c.path != canon) {
        return Err(MediaError::Backend(
            "OpenH264 was loaded concurrently from another path".into(),
        ));
    }
    Ok(())
}

/// Whether this build can encode and decode H.264 now.
pub(crate) fn h264_available() -> bool {
    #[cfg(feature = "media-cisco-openh264")]
    {
        CISCO.get().is_some()
    }
    #[cfg(not(feature = "media-cisco-openh264"))]
    {
        true
    }
}

pub(crate) fn h264_api() -> Result<openh264::OpenH264API, MediaError> {
    #[cfg(feature = "media-cisco-openh264")]
    {
        let c = CISCO.get().ok_or_else(|| {
            MediaError::NotSupported(
                "H.264: call rinch_rtc::media::load_openh264 with Cisco's library first".into(),
            )
        })?;
        // SAFETY: the same canonical path load_openh264 checked; its image
        // is resident (`_resident`), so this only bumps its reference count.
        unsafe { openh264::OpenH264API::from_blob_path_unchecked(&c.path) }
            .map_err(|e| MediaError::NotSupported(format!("OpenH264 library: {e}")))
    }
    #[cfg(not(feature = "media-cisco-openh264"))]
    {
        Ok(openh264::OpenH264API::from_source())
    }
}

// ============================================================ local tracks

/// A sender carrying a local track: (peer, sender key), its queue, and the
/// video codec it negotiated (0 not yet known, else `codec_id`).
type Subscriber = (
    (u64, u64),
    tokio::sync::mpsc::Sender<Encoded>,
    Arc<std::sync::atomic::AtomicU8>,
);

pub(crate) fn codec_id(c: VideoCodec) -> u8 {
    match c {
        VideoCodec::H264 => 1,
        #[cfg(feature = "media-vp8")]
        VideoCodec::Vp8 => 2,
    }
}

fn codec_of(id: u8) -> Option<VideoCodec> {
    match id {
        1 => Some(VideoCodec::H264),
        #[cfg(feature = "media-vp8")]
        2 => Some(VideoCodec::Vp8),
        _ => None,
    }
}

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
    /// Encoded bytes published, all codecs (test hook: the real bitrate).
    encoded_bytes: std::sync::atomic::AtomicU64,
    /// Keyframe requests (PLI, FIR) from the far side of any sender.
    far_requests: AtomicU32,
    /// The bitrate the pump last gave its encoders (a test's view of a cap
    /// reaching a running encoder).
    applied_bitrate: AtomicU32,
    /// The largest frame published since a test last asked (bytes).
    largest: std::sync::atomic::AtomicU64,
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
            encoded_bytes: std::sync::atomic::AtomicU64::new(0),
            far_requests: AtomicU32::new(0),
            largest: std::sync::atomic::AtomicU64::new(0),
            applied_bitrate: AtomicU32::new(0),
        })
    }

    pub(crate) fn kind(&self) -> MediaKind {
        self.kind
    }

    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    /// A sender starts carrying this track; it gets a keyframe first.
    pub(crate) fn subscribe(
        &self,
        who: (u64, u64),
        codec: Arc<std::sync::atomic::AtomicU8>,
    ) -> tokio::sync::mpsc::Receiver<Encoded> {
        let (tx, rx) = tokio::sync::mpsc::channel(SENDER_QUEUE);
        if let Ok(mut s) = self.subscribers.lock() {
            s.retain(|(w, _, _)| *w != who);
            s.push((who, tx, codec));
        }
        self.keyframe.store(true, Ordering::Release);
        rx
    }

    pub(crate) fn unsubscribe(&self, who: (u64, u64)) {
        if let Ok(mut s) = self.subscribers.lock() {
            s.retain(|(w, _, _)| *w != who);
        }
        if let Ok(mut c) = self.caps.lock() {
            c.remove(&who);
        }
    }

    pub(crate) fn request_keyframe(&self) {
        self.keyframe.store(true, Ordering::Release);
    }

    /// A receiver asked for a keyframe (its PLI or FIR, through
    /// `KeyframeRequests`).
    pub(crate) fn far_keyframe_request(&self) {
        self.far_requests.fetch_add(1, Ordering::Relaxed);
        self.request_keyframe();
    }

    fn has_subscribers(&self) -> bool {
        self.subscribers
            .lock()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    /// The video codecs some sender has negotiated.
    fn wanted_codecs(&self) -> Vec<VideoCodec> {
        let mut out = Vec::new();
        if let Ok(s) = self.subscribers.lock() {
            for (_, _, c) in s.iter() {
                if let Some(c) = codec_of(c.load(Ordering::Acquire))
                    && !out.contains(&c)
                {
                    out.push(c);
                }
            }
        }
        out
    }

    /// To every sender of `frame`'s codec (audio: every sender).
    fn publish(&self, frame: Encoded) {
        self.encoded_bytes
            .fetch_add(frame.data.len() as u64, Ordering::Relaxed);
        self.largest
            .fetch_max(frame.data.len() as u64, Ordering::Relaxed);
        if let Ok(mut s) = self.subscribers.lock() {
            // A closed queue is a sender that went away.
            s.retain(|(_, tx, c)| {
                let theirs = codec_of(c.load(Ordering::Acquire));
                if frame.codec.is_some() && theirs != frame.codec {
                    return !tx.is_closed();
                }
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
    // `try_with`: a track dropped while the thread's locals are torn down
    // (held in a signal at exit) finds these tables gone, and has nothing
    // left to stop. `with` panicked there, which aborts the process.
    let gone = LOCALS
        .try_with(|l| l.borrow_mut().remove(&key))
        .ok()
        .flatten();
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
    let capture = CAPTURES
        .try_with(|c| c.borrow_mut().remove(&key))
        .ok()
        .flatten();
    drop(capture);
}

/// Video: at the track's fps, the latest frame to the self-views and, when
/// a sender carries the track, encoded once per codec the senders
/// negotiated (H.264, VP8) to every sender of that codec.
/// One codec's encoder in a video pump.
struct PumpEncoder {
    enc: Box<dyn VideoEncoder>,
    /// The size it was made for (a new size makes a new encoder).
    w: u32,
    h: u32,
    bitrate: u32,
    /// Bits the send budget has left (negative: overspent).
    budget: f64,
    at: Instant,
    /// A new encoder: its first frame is a keyframe.
    fresh: bool,
    /// A keyframe was asked for and this encoder has not sent it yet.
    key_pending: bool,
    last_key: Instant,
    last_sent: Option<Instant>,
}

fn video_pump(src: Arc<LocalSource>) {
    let Some(slot) = src.video.clone() else {
        return;
    };
    let period = Duration::from_secs_f64(1.0 / f64::from(src.fps));
    let mut next = Instant::now();
    let mut seen = u64::MAX;
    let mut frame: Vec<u8> = Vec::new();
    let mut black: Vec<u8> = Vec::new();
    let (mut w, mut h) = (0u32, 0u32);
    // One encoder per codec the senders negotiated.
    let mut encoders: HashMap<VideoCodec, PumpEncoder> = HashMap::new();
    let mut last_encode = Instant::now() - Duration::from_secs(1);
    let mut warned = false;
    let mut was_enabled = true;
    let started = Instant::now();
    while !src.stopped.load(Ordering::Acquire) {
        next += period;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else {
            next = now;
        }
        let enabled = src.enabled.load(Ordering::Acquire);
        if enabled && !was_enabled {
            // Re-read the source on enable: a still source pushes no new
            // frame, and what is shown must be its picture again.
            seen = u64::MAX;
            src.request_keyframe();
        }
        was_enabled = enabled;
        let seq = slot.seq.load(Ordering::Acquire);
        let fresh = seq != seen;
        if fresh {
            seen = seq;
            if let Ok(l) = slot.latest.lock()
                && let Some((rgba, fw, fh)) = l.as_ref()
            {
                // openh264 takes nothing under 16 pixels a side: a smaller
                // picture is scaled up by a whole factor (each pixel a
                // k×k block). Then the codecs want even dimensions: an odd
                // last row or column is dropped.
                let (fw, fh) = (*fw as usize, *fh as usize);
                let k = 16usize.div_ceil(fw.min(fh)).max(1);
                let (ew, eh) = ((fw * k) & !1, (fh * k) & !1);
                (w, h) = (ew as u32, eh as u32);
                frame.resize(ew * eh * 4, 0);
                if k == 1 {
                    for row in 0..eh {
                        let s = row * fw * 4;
                        let d = row * ew * 4;
                        frame[d..d + ew * 4].copy_from_slice(&rgba[s..s + ew * 4]);
                    }
                } else {
                    for y in 0..eh {
                        for x in 0..ew {
                            let s = ((y / k) * fw + x / k) * 4;
                            let d = (y * ew + x) * 4;
                            frame[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
                        }
                    }
                }
                slot.taken.fetch_add(1, Ordering::Relaxed);
            }
        }
        if w == 0 {
            continue;
        }
        // Black, as a browser shows and sends a disabled track; the
        // source's own picture is kept for re-enable.
        let picture: &[u8] = if enabled {
            &frame
        } else {
            if black.len() != frame.len() {
                black = frame
                    .iter()
                    .enumerate()
                    .map(|(i, _)| if i % 4 == 3 { 255 } else { 0 })
                    .collect();
            }
            &black
        };
        if fresh || !enabled {
            src.show(picture, w, h);
        }
        let codecs = src.wanted_codecs();
        if codecs.is_empty() {
            continue;
        }
        // A still source is re-sent at least twice a second (a viewer that
        // joins late gets a picture); a disabled one once a second.
        let gap = if enabled {
            Duration::from_millis(500)
        } else {
            Duration::from_secs(1)
        };
        if !fresh
            && last_encode.elapsed() < gap
            && !src.keyframe.load(Ordering::Acquire)
            && !encoders.values().any(|e| e.key_pending)
        {
            continue;
        }
        let bitrate = src.bitrate(w, h);
        // A keyframe on request (a new sender, a receiver's PLI or FIR, an
        // enable) and every 10 s as a backstop; at most one a second (a
        // receiver repeats its PLI until a keyframe arrives, and a
        // keyframe still being paced out must not be followed by another).
        // A request is handed to every encoder and kept by each until it
        // sends its keyframe: one the send budget holds back waits for it,
        // and one codec's keyframe does not answer for another's.
        if src.keyframe.swap(false, Ordering::AcqRel) {
            for e in encoders.values_mut() {
                e.key_pending = true;
            }
        }
        let pic = I420::from_rgba(picture, w as usize, h as usize);
        let ts = started.elapsed().as_millis() as u64;
        let now = Instant::now();
        for codec in codecs {
            if encoders.get(&codec).is_none_or(|e| (e.w, e.h) != (w, h)) {
                match crate::native_codec::new_encoder(codec, w, h, src.fps, bitrate) {
                    Ok(enc) => {
                        // A new size is a new encoder, starting with a
                        // keyframe; the send budget and the timing carry
                        // over (a debt is still owed).
                        let old = encoders.remove(&codec);
                        encoders.insert(
                            codec,
                            PumpEncoder {
                                enc,
                                w,
                                h,
                                bitrate,
                                budget: old.as_ref().map_or(0.0, |o| o.budget),
                                at: old.as_ref().map_or(now, |o| o.at),
                                fresh: true,
                                key_pending: false,
                                last_key: now,
                                last_sent: old.as_ref().and_then(|o| o.last_sent),
                            },
                        );
                    }
                    Err(e) => {
                        if !warned {
                            tracing::error!("rinch-rtc: {codec:?} encoder: {e}");
                            warned = true;
                        }
                        continue;
                    }
                }
            }
            let Some(e) = encoders.get_mut(&codec) else {
                continue;
            };
            if e.bitrate != bitrate {
                // A new cap reaches the running encoder: no keyframe.
                if let Err(err) = e.enc.set_bitrate(bitrate) {
                    tracing::warn!("rinch-rtc: {codec:?} bitrate: {err}");
                }
                e.bitrate = bitrate;
            }
            src.applied_bitrate.store(e.bitrate, Ordering::Relaxed);
            // The send budget: what the bitrate allows since the last
            // frame, with at most half a second saved up. While it is spent
            // no frame is encoded, a keyframe included (it waits), and an
            // overshoot is paid back in skipped frames. Neither encoder
            // holds a cap alone on a hard picture: openh264 overshoots
            // several times even with frame skipping and has nothing to
            // limit a keyframe's size (no max-intra setting; only a QP
            // floor, which costs every frame), and libvpx's keyframes of
            // noise are 36 KB whatever its limit, so a keyframe a second
            // (a lossy link's PLIs) ran VP8 at three times a 100 kbps cap.
            let rate = f64::from(bitrate);
            e.budget = (e.budget + rate * now.duration_since(e.at).as_secs_f64())
                .clamp(-rate * 10.0, rate * 0.5);
            e.at = now;
            // Only an encoder's very first frame goes out on credit.
            if e.last_sent.is_some() && e.budget < 0.0 {
                continue;
            }
            // A keyframe on request (a new sender, a receiver's PLI or FIR,
            // an enable) and every 10 s as a backstop; at most one a second
            // (a receiver repeats its PLI until a keyframe arrives, and a
            // keyframe still being paced out must not be followed by
            // another).
            let since = now.duration_since(e.last_key);
            let key = e.fresh
                || (e.key_pending && since >= Duration::from_secs(1))
                || since > Duration::from_secs(10);
            match e.enc.encode(&pic, ts, key) {
                Ok(data) if !data.is_empty() => {
                    if key {
                        e.fresh = false;
                        e.key_pending = false;
                        e.last_key = now;
                    }
                    // Overspending is paid back in skipped frames, up to
                    // 10 s of debt (a picture no codec can compress).
                    e.budget = (e.budget - data.len() as f64 * 8.0).max(-rate * 10.0);
                    last_encode = now;
                    // The RTP timestamp advances by the time since this
                    // codec's last frame: the pump takes the latest picture
                    // at each tick, so that is capture time to within a
                    // tick, through skipped and repeated frames alike.
                    let duration = e.last_sent.map_or(period, |t| now.duration_since(t));
                    e.last_sent = Some(now);
                    src.publish(Encoded {
                        data: bytes::Bytes::from(data),
                        duration,
                        codec: Some(codec),
                        // WebRTC's pacing factor: 2.5 times the target.
                        pace_bps: bitrate.saturating_mul(5) / 2,
                    });
                }
                Ok(_) => {} // skipped by rate control
                Err(err) => tracing::warn!("rinch-rtc: {codec:?} encode: {err}"),
            }
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
    // In-band FEC: each packet carries a coarse copy of the one before, so
    // the far side can rebuild a single lost packet (OpusJitter).
    let _ = encoder.set_inband_fec(true);
    let _ = encoder.set_packet_loss_perc(10);
    let mut resampler = Resampler::new(queue.sample_rate, 48_000, out_ch);
    // Frames per 20 ms tick, with the remainder carried: a rate that is not
    // a multiple of 50 (11025, 22050) would otherwise take too few
    // frames each tick and fall behind.
    let rate = queue.sample_rate as u64;
    let mut ticks = 0u64;
    let mut taken_frames = 0u64;
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
        ticks += 1;
        let due = rate * ticks / 50;
        let per_tick = (due - taken_frames) as usize * in_ch;
        taken_frames = due;
        let mut chunk: Vec<f32> = match queue.samples.lock() {
            Ok(mut q) => {
                let n = q.len().min(per_tick);
                queue.taken.fetch_add(n as u64, Ordering::Relaxed);
                let chunk = q.drain(..n).collect();
                // Latency must not ratchet: after a stall (this thread
                // late, the process paused) the source has run ahead, and
                // the 20 ms ticks would carry that backlog for the rest of
                // the call. Past 60 ms the oldest audio is dropped, down to
                // one tick's worth.
                let tick = per_tick.max(in_ch);
                if q.len() > tick * 3 {
                    let drop = q.len() - tick;
                    q.drain(..drop);
                    queue.taken.fetch_add(drop as u64, Ordering::Relaxed);
                }
                chunk
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
                    codec: None,
                    pace_bps: 0,
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
    let _ = SENDING.try_with(|m| m.borrow_mut().retain(|(peer, _), _| *peer != p));
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
    /// Video: (arrival, RTP timestamp) of the last packets that ended a
    /// frame (test hook: timestamps against time).
    pub(crate) frame_times: Mutex<VecDeque<(Instant, u32)>>,
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
            frame_times: Mutex::new(VecDeque::new()),
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

/// A decoder thread for one remote video track: access units of `codec`
/// in, RGBA out to the sink. A frame it cannot decode asks for a keyframe
/// through `pli` (at most once a second).
pub(crate) fn start_video_decoder(
    key: u64,
    codec: VideoCodec,
    sink: Arc<RemoteSink>,
    pli: tokio::sync::mpsc::UnboundedSender<()>,
) -> Option<std::sync::mpsc::SyncSender<bytes::Bytes>> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<bytes::Bytes>(64);
    let started = std::thread::Builder::new()
        .name(format!("rinch-rtc-dec-{key}"))
        .spawn(move || {
            let mut dec = match crate::native_codec::new_decoder(codec) {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!("rinch-rtc: no {codec:?} decoder: {e}");
                    return;
                }
            };
            let mut rgba: Vec<u8> = Vec::new();
            let mut last_pli = Instant::now() - Duration::from_secs(2);
            while let Ok(au) = rx.recv() {
                if sink.ended.load(Ordering::Acquire) {
                    break;
                }
                match dec.decode(&au, &mut rgba) {
                    Ok(Some((w, h))) => sink.deliver(&rgba, w, h),
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
/// Past this much buffered (160 ms), the buffer is cut back to
/// `DRIFT_TARGET` (80 ms).
const DRIFT_HIGH: usize = 48 * 160 * 2;
const DRIFT_TARGET: usize = 48 * 80 * 2;

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
        // The far side's clock runs a little fast against ours: let the
        // buffer drift no further than 160 ms before cutting it back to
        // 80 ms (one cut, not a stream of them), so latency stays bounded.
        if t.buf.len() > DRIFT_HIGH {
            let cut = t.buf.len() - DRIFT_TARGET;
            t.buf.drain(..cut);
        }
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
    // 0 until the device's real rate is known: the first callback can come
    // before `open_audio_output` returns, and must not build a resampler
    // for a guessed rate.
    let rate = Arc::new(AtomicU32::new(0));
    let channels = Arc::new(AtomicU32::new(2));
    let (r2, c2) = (rate.clone(), channels.clone());
    let mut resampler: Option<Resampler> = None;
    let mut mixed: Vec<f32> = Vec::new();
    let mut converted: Vec<f32> = Vec::new();
    let fill = move |out: &mut [f32]| {
        let dev_rate = r2.load(Ordering::Acquire);
        if dev_rate == 0 {
            out.fill(0.0);
            return;
        }
        let dev_ch = c2.load(Ordering::Acquire).max(1) as usize;
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
            // Channels first: the rate is what releases the callback.
            channels.store(u32::from(stream.config().channels), Ordering::Release);
            rate.store(stream.config().sample_rate, Ordering::Release);
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

// ============================================================ test hooks

/// Loss injection for rinch-rtc's tests (`testing::drop_incoming_*`).
static DROP_VIDEO: AtomicBool = AtomicBool::new(false);
static DROP_AUDIO_EVERY: AtomicU32 = AtomicU32::new(0);
static AUDIO_SEEN: AtomicU32 = AtomicU32::new(0);
/// Offer H.264 first (`testing::prefer_h264`), or only (`h264_only`).
pub(crate) static PREFER_H264: AtomicBool = AtomicBool::new(false);
pub(crate) static H264_ONLY: AtomicBool = AtomicBool::new(false);

pub(crate) fn drop_video_packet() -> bool {
    DROP_VIDEO.load(Ordering::Relaxed)
}

pub(crate) fn drop_audio_packet() -> bool {
    let every = DROP_AUDIO_EVERY.load(Ordering::Relaxed);
    let n = AUDIO_SEEN.fetch_add(1, Ordering::Relaxed);
    every != 0 && n % every == every - 1
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
            // Paced to the clock (a device's pace), not 20 ms of sleep
            // plus the work.
            let start = std::time::Instant::now();
            let mut k = 0u32;
            loop {
                k += 1;
                let due = start + std::time::Duration::from_millis(20) * k;
                if let Some(d) = due.checked_duration_since(std::time::Instant::now()) {
                    std::thread::sleep(d);
                }
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

    /// Drop every incoming video packet while `on` (loss injection).
    pub fn drop_incoming_video(on: bool) {
        super::DROP_VIDEO.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// Drop every `n`th incoming audio packet (0: none).
    pub fn drop_incoming_audio_every(n: u32) {
        super::DROP_AUDIO_EVERY.store(n, std::sync::atomic::Ordering::Relaxed);
    }

    /// The deepest remote-audio buffer in the mixer, in ms.
    pub fn mixer_depth_ms() -> f32 {
        super::with_mixer(|m| m.tracks.values().map(|t| t.buf.len()).max().unwrap_or(0)) as f32
            / 96.0
    }

    /// The video codecs a local track is encoding in: one per codec its
    /// senders negotiated ("VP8", "H264").
    pub fn sending_codecs(track: &crate::media::LocalTrack) -> Vec<&'static str> {
        super::local_source(track.key())
            .map(|s| {
                s.wanted_codecs()
                    .into_iter()
                    .map(|c| match c {
                        #[cfg(feature = "media-vp8")]
                        crate::native_codec::VideoCodec::Vp8 => "VP8",
                        crate::native_codec::VideoCodec::H264 => "H264",
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The bitrate a local video track's running encoder was last given.
    pub fn applied_bitrate(track: &crate::media::LocalTrack) -> u32 {
        super::local_source(track.key())
            .map(|s| s.applied_bitrate.load(std::sync::atomic::Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Ask a local track for a keyframe as a receiver's PLI would.
    pub fn request_keyframe(track: &crate::media::LocalTrack) {
        if let Some(s) = super::local_source(track.key()) {
            s.request_keyframe();
        }
    }

    /// Keyframe requests (PLI, FIR) that reached a local track's encoder
    /// from the far side.
    pub fn keyframe_requests(track: &crate::media::LocalTrack) -> u32 {
        super::local_source(track.key())
            .map(|s| s.far_requests.load(std::sync::atomic::Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// The largest frame a local track published since the last call
    /// (bytes), and start again.
    pub fn take_largest_frame(track: &crate::media::LocalTrack) -> u64 {
        super::local_source(track.key())
            .map(|s| s.largest.swap(0, std::sync::atomic::Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Encoded bytes a local track has published (all codecs).
    pub fn encoded_bytes(track: &crate::media::LocalTrack) -> u64 {
        super::local_source(track.key())
            .map(|s| s.encoded_bytes.load(std::sync::atomic::Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Offer H.264 before VP8 (tests of the H.264 path; a `media-vp8`
    /// build prefers VP8, and a build without it offers H.264 alone, so
    /// this changes nothing there). Read when a peer's connection is built, on the worker
    /// shortly after `Peer::new`: keep it on until the peer's first offer
    /// or answer.
    pub fn prefer_h264(on: bool) {
        super::PREFER_H264.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// Offer and accept H.264 only, no VP8 (`media_echo --h264-only`:
    /// H.264 interop with a browser, which offers VP8 first). A build
    /// without `media-vp8` is H.264-only already. Read as `prefer_h264` is.
    pub fn h264_only(on: bool) {
        super::H264_ONLY.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// The bitrate a local video track's encoder targets at `w`×`h`: the
    /// lowest `set_max_bitrate` of its senders, or the default.
    pub fn encoder_target_bitrate(track: &crate::media::LocalTrack, w: u32, h: u32) -> u32 {
        super::local_source(track.key())
            .map(|s| s.bitrate(w, h))
            .unwrap_or(0)
    }

    /// (arrival, RTP timestamp) of the last frames a remote video track
    /// received (up to 512), oldest first.
    pub fn frame_rtp_times(track: &crate::media::RemoteTrack) -> Vec<(std::time::Instant, u32)> {
        super::remote(track.peer, track.key)
            .and_then(|s| {
                s.frame_times
                    .lock()
                    .ok()
                    .map(|t| t.iter().copied().collect())
            })
            .unwrap_or_default()
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

    /// Add a self-view sink to a local video track, as a `MediaView` of it
    /// does (tests of what the self-view shows, without a DOM).
    pub fn add_self_view(
        track: &crate::media::LocalTrack,
        sink: rinch_core::video_sink::VideoFrameSink,
    ) {
        if let Some(s) = super::local_source(track.key())
            && let Ok(mut v) = s.views.lock()
        {
            v.push(("testing".to_owned(), sink));
        }
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

    use std::collections::{BTreeMap, VecDeque};
    use std::time::Duration;

    use rtc::interceptor::{
        Attribute, Interceptor, Packet, Registry, Slot, StreamInfo, TaggedPacket,
    };
    use rtc::media::io::sample_builder::SampleBuilder;
    use rtc::media_stream::MediaStreamTrack;
    use rtc::peer_connection::configuration::interceptor_registry::register_default_interceptors;
    #[cfg(feature = "media-vp8")]
    use rtc::peer_connection::configuration::media_engine::MIME_TYPE_VP8;
    use rtc::peer_connection::configuration::media_engine::{
        MIME_TYPE_H264, MIME_TYPE_OPUS, MediaEngine,
    };
    use rtc::rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
    use rtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
    use rtc::rtp::codec::h264::H264Packet;
    #[cfg(feature = "media-vp8")]
    use rtc::rtp::codec::vp8::Vp8Packet;
    use rtc::rtp::packetizer::{Packetizer, new_packetizer};
    use rtc::rtp::sequence::{Sequencer, new_random_sequencer};
    use rtc::rtp_transceiver::rtp_sender::{
        RTCPFeedback, RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters,
        RTCRtpEncodingParameters, RtpCodecKind,
    };
    use rtc::rtp_transceiver::{
        RTCRtpTransceiverDirection, RTCRtpTransceiverId, RTCRtpTransceiverInit,
    };
    use rtc::sansio::Protocol;
    use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
    use webrtc::media_stream::track_local::{TrackLocal, TrackLocalEvent};
    use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
    use webrtc::peer_connection::PeerConnection;
    use webrtc::rtp_transceiver::{RtpReceiver, RtpSender, RtpTransceiver};

    use super::{
        Encoded, LocalSource, MediaCmd, RemoteSink, TxMirror, codec_id, drop_audio_packet,
        drop_remotes, drop_video_packet, insert_remote, mixer_push, mixer_remove, remote,
        start_video_decoder, with_mirror,
    };
    use crate::RtcError;
    use crate::media::{Direction, MediaEvent, MediaKind};
    use crate::native_codec::VideoCodec;
    use crate::registry::Event;

    const H264_PT: u8 = 102;
    #[cfg(feature = "media-vp8")]
    const VP8_PT: u8 = 96;
    const OPUS_PT: u8 = 111;
    /// The packetizer's MTU (as webrtc-rs's own sample track uses).
    const MTU: usize = 1200;

    fn video_feedback() -> Vec<RTCPFeedback> {
        vec![
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
        ]
    }

    #[cfg(feature = "media-vp8")]
    fn vp8() -> RTCRtpCodec {
        RTCRtpCodec {
            mime_type: MIME_TYPE_VP8.to_owned(),
            clock_rate: 90_000,
            channels: 0,
            sdp_fmtp_line: String::new(),
            rtcp_feedback: video_feedback(),
        }
    }

    fn h264() -> RTCRtpCodec {
        RTCRtpCodec {
            mime_type: MIME_TYPE_H264.to_owned(),
            clock_rate: 90_000,
            channels: 0,
            sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"
                .to_owned(),
            rtcp_feedback: video_feedback(),
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

    /// The codecs a media peer offers and accepts, in preference order:
    /// with `media-vp8`, VP8 first, then H.264 constrained baseline; without
    /// it, H.264 alone; and Opus (M2, decided 2026-10-27, VP8 made opt-in
    /// 2026-10-28). No RTX (see `media_engine`).
    ///
    /// VP8 first because every browser encodes it, while a Firefox without
    /// Cisco's OpenH264 plugin still *lists* H.264 in its SDP and then
    /// sends nothing: preferring H.264 would pick a codec that side cannot
    /// send. An answer follows the offer's order, so H.264 is used where
    /// the other side offers it first (Safari does). Without VP8, a
    /// browser's VP8-first offer is answered with H.264, the one codec in
    /// common: Chrome and Safari send it, such a Firefox sends no video.
    /// H.264 is left out altogether until it is available
    /// (`media-cisco-openh264` before `load_openh264`). The testing flags
    /// reorder or drop VP8.
    fn video_codecs() -> Vec<(RTCRtpCodec, u8)> {
        #[allow(unused_mut)]
        let mut video: Vec<(RTCRtpCodec, u8)> = Vec::new();
        #[cfg(feature = "media-vp8")]
        if !super::H264_ONLY.load(Ordering::Relaxed) {
            video.push((vp8(), VP8_PT));
        }
        if super::h264_available() {
            if super::PREFER_H264.load(Ordering::Relaxed) {
                video.insert(0, (h264(), H264_PT));
            } else {
                video.push((h264(), H264_PT));
            }
        }
        video
    }

    /// The codec a new video track is labelled with, and the codec
    /// preferences its transceiver gets (that codec first, then ours).
    ///
    /// The label matters to webrtc-rs 0.21 twice over. A track's codec
    /// becomes its transceiver's only codec preference, which is why the
    /// preferences are set again to every codec. And a sender is bound to
    /// the interceptors (the NACK responder that retransmits, the sender
    /// reports) only if its label is among the negotiated codecs
    /// (`interceptor_local_streams_op`); an unlabelled track never was, so
    /// nothing was retransmitted (#1947's third review). So the label is
    /// the codec negotiation will most likely pick: the far side's first
    /// one we support when its offer is already here, our first otherwise.
    /// A negotiation that leaves the label out would send unbound (no
    /// retransmission or sender reports), so the worker relabels the
    /// sender to a negotiated codec before the binding runs (#1954); a
    /// sender still unmatched afterwards gets one warning.
    async fn video_label(
        pc: &Arc<dyn PeerConnection>,
    ) -> (RTCRtpCodec, Vec<RTCRtpCodecParameters>) {
        let ours = video_codecs();
        let mut first = 0;
        if let Some(d) = pc.remote_description().await {
            for name in sdp_video_codecs(&d.sdp) {
                if let Some(i) = ours.iter().position(|(c, _)| {
                    c.mime_type
                        .rsplit('/')
                        .next()
                        .is_some_and(|n| n.eq_ignore_ascii_case(&name))
                }) {
                    first = i;
                    break;
                }
            }
        }
        let mut prefs: Vec<RTCRtpCodecParameters> = ours
            .iter()
            .map(|(c, pt)| RTCRtpCodecParameters {
                rtp_codec: c.clone(),
                payload_type: *pt,
            })
            .collect();
        if prefs.is_empty() {
            return (RTCRtpCodec::default(), prefs);
        }
        let label = prefs.remove(first);
        prefs.insert(0, label.clone());
        (label.rtp_codec, prefs)
    }

    /// The encoding names of an SDP's first video m-line, in its order.
    fn sdp_video_codecs(sdp: &str) -> Vec<String> {
        let mut pts: Vec<&str> = Vec::new();
        let mut names: HashMap<&str, &str> = HashMap::new();
        let mut in_video = false;
        for l in sdp.lines() {
            if let Some(m) = l.strip_prefix("m=") {
                if !pts.is_empty() {
                    break;
                }
                in_video = m.starts_with("video");
                if in_video {
                    pts = m.split(' ').skip(3).collect();
                }
            } else if in_video
                && let Some(r) = l.strip_prefix("a=rtpmap:")
                && let Some((pt, rest)) = r.split_once(' ')
            {
                names.insert(pt, rest.split('/').next().unwrap_or(""));
            }
        }
        pts.iter()
            .filter_map(|p| names.get(p).map(|n| n.to_string()))
            .collect()
    }

    /// What to tell the app when a remote offer's video cannot be answered:
    /// `Some(message)` when some video m-line (port not 0) lists codecs and
    /// none of them is one this build offers, so the answer rejects it and
    /// the call goes on without that video. webrtc-rs reports nothing then
    /// (a VP8-only offer to a build without `media-vp8` connected on audio
    /// alone, silently: #1975's review). The message names the codecs on
    /// each side and, where it is the cause, the feature or call that adds
    /// the missing one.
    pub(crate) fn unanswerable_video(sdp: &str) -> Option<String> {
        let ours: Vec<String> = video_codecs()
            .iter()
            .filter_map(|(c, _)| c.mime_type.rsplit('/').next().map(str::to_ascii_uppercase))
            .collect();
        let mut theirs: Vec<String> = Vec::new();
        let mut rejected = false;
        // One m-section at a time: (is live video, its payload types, names).
        let mut flush = |live: bool, pts: &[&str], names: &HashMap<&str, &str>| {
            if !live {
                return;
            }
            let listed: Vec<String> = pts
                .iter()
                .filter_map(|p| names.get(p).map(|n| n.to_ascii_uppercase()))
                .collect();
            if listed.is_empty() || listed.iter().any(|n| ours.contains(n)) {
                return;
            }
            rejected = true;
            for n in listed {
                if !theirs.contains(&n) {
                    theirs.push(n);
                }
            }
        };
        let mut live = false;
        let mut pts: Vec<&str> = Vec::new();
        let mut names: HashMap<&str, &str> = HashMap::new();
        for l in sdp.lines() {
            if let Some(m) = l.strip_prefix("m=") {
                flush(live, &pts, &names);
                let f: Vec<&str> = m.split(' ').collect();
                live = f.first() == Some(&"video") && f.get(1).is_some_and(|p| *p != "0");
                pts = f.iter().skip(3).copied().collect();
                names.clear();
            } else if live
                && let Some(r) = l.strip_prefix("a=rtpmap:")
                && let Some((pt, rest)) = r.split_once(' ')
            {
                names.insert(pt, rest.split('/').next().unwrap_or(""));
            }
        }
        flush(live, &pts, &names);
        if !rejected {
            return None;
        }
        let have = if ours.is_empty() {
            "none".to_string()
        } else {
            ours.join(", ")
        };
        let mut msg = format!(
            "the remote offer's video lists {} and this build offers {have}: \
             no codec in common, so that video is rejected and the call goes on without it",
            theirs.join(", ")
        );
        if cfg!(not(feature = "media-vp8")) && theirs.iter().any(|n| n == "VP8") {
            msg.push_str(
                ". VP8 needs rinch-rtc's `media-vp8` feature \
                 (features = [\"media\", \"media-vp8\"])",
            );
        }
        if theirs.iter().any(|n| n == "H264") && !super::h264_available() {
            msg.push_str(". H.264 needs `rinch_rtc::media::load_openh264` first");
        }
        Some(msg)
    }

    /// Give the transceiver carrying `rtp` our video codec preferences.
    async fn set_video_prefs(
        pc: &Arc<dyn PeerConnection>,
        rtp: &Arc<dyn RtpSender>,
        prefs: Vec<RTCRtpCodecParameters>,
    ) {
        for t in pc.get_transceivers().await {
            if let Ok(Some(s)) = t.sender().await
                && s.id() == rtp.id()
                && let Err(e) = t.set_codec_preferences(prefs.clone()).await
            {
                tracing::warn!("rinch-rtc: video codec preferences: {e}");
            }
        }
    }

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
        // No RTX (RFC 4588): a NACKed packet is resent as itself, same SSRC
        // and sequence number. webrtc-rs's NACK generator tracks arrivals
        // by SSRC before RTX is resolved, so a repair sent as RTX never
        // counted as received: it NACKed the same packet every 100 ms
        // (79 NACKs and 261 RTX packets for 5 losses in 10 s); resent as
        // itself, one NACK and one retransmission each. A browser offering
        // RTX is answered without it and retransmits the same way.
        for (codec, pt) in video_codecs() {
            me.register_codec(
                RTCRtpCodecParameters {
                    rtp_codec: codec,
                    payload_type: pt,
                },
                RtpCodecKind::Video,
            )
            .map_err(err)?;
        }
        let registry = register_default_interceptors(Registry::new(), &mut me).map_err(err)?;
        // Last in the chain: pass the far side's keyframe requests (PLI,
        // FIR) to the sender, which the default chain stops.
        let registry = registry.with(Slot::from(14_000), KeyframeRequests::default());
        Ok((me, registry))
    }

    /// Marks inbound PLI and FIR for the application (webrtc-rs's
    /// `rtcp-processing` example); everything else inbound stops here, as
    /// it would at the end of the default chain.
    #[derive(Default)]
    struct KeyframeRequests {
        read: VecDeque<TaggedPacket>,
        write: VecDeque<TaggedPacket>,
    }

    impl Protocol<TaggedPacket, TaggedPacket, ()> for KeyframeRequests {
        type Rout = TaggedPacket;
        type Wout = TaggedPacket;
        type Eout = ();
        type Error = rtc::shared::error::Error;
        type Time = Instant;

        fn handle_read(&mut self, mut msg: TaggedPacket) -> Result<(), Self::Error> {
            if let Packet::Rtcp(packets) = &msg.message.packet {
                let requests: Vec<Box<dyn rtc::rtcp::Packet>> = packets
                    .iter()
                    .filter(|p| {
                        let a = p.as_any();
                        a.is::<PictureLossIndication>() || a.is::<FullIntraRequest>()
                    })
                    .cloned()
                    .collect();
                if requests.is_empty() {
                    return Ok(());
                }
                msg.message.packet = Packet::Rtcp(requests);
                msg.message.add(Attribute::DeliverToApplication);
            }
            self.read.push_back(msg);
            Ok(())
        }

        fn poll_read(&mut self) -> Option<Self::Rout> {
            self.read.pop_front()
        }

        fn handle_write(&mut self, msg: TaggedPacket) -> Result<(), Self::Error> {
            self.write.push_back(msg);
            Ok(())
        }

        fn poll_write(&mut self) -> Option<Self::Wout> {
            self.write.pop_front()
        }
    }

    impl Interceptor for KeyframeRequests {
        fn bind_local_stream(&mut self, _info: &StreamInfo) {}
        fn unbind_local_stream(&mut self, _info: &StreamInfo) {}
        fn bind_remote_stream(&mut self, _info: &StreamInfo) {}
        fn unbind_remote_stream(&mut self, _info: &StreamInfo) {}
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
        track: Option<Arc<TrackLocalStaticRTP>>,
        /// What outlives one forwarder: the SSRC, the sequence numbers, and
        /// the codec the sender negotiated.
        shared: Arc<SendShared>,
        kind: MediaKind,
        source: Option<Arc<LocalSource>>,
        forward: Option<tokio::task::JoinHandle<()>>,
        /// The mime type `track` is labelled with as rtc sees it (empty:
        /// audio, or no track yet). `relabel` changes it until the sender
        /// is bound (#1954).
        label: String,
        /// A negotiation has completed with this sender in it: rtc bound
        /// it to the interceptors then, or never will (`check_labels`).
        settled: bool,
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

    /// A sender's track. It carries packets the forwarder packetizes
    /// itself, for whichever codec negotiation picks; `video` labels a
    /// video track (`video_label`). Audio is Opus.
    fn track_for(
        kind: MediaKind,
        track_id: &str,
        stream_id: &str,
        ssrc: u32,
        video: &RTCRtpCodec,
    ) -> Arc<TrackLocalStaticRTP> {
        let codec = match kind {
            MediaKind::Video => video.clone(),
            MediaKind::Audio => opus(),
        };
        Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
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
        )))
    }

    /// The mime type a track of `kind` labelled `video` carries as a
    /// video label (`SenderState::label`); empty for audio.
    fn video_mime(kind: MediaKind, video: &RTCRtpCodec) -> String {
        match kind {
            MediaKind::Video => video.mime_type.clone(),
            MediaKind::Audio => String::new(),
        }
    }

    /// Whether a label (a mime type) is among `codecs`, as rtc's binding
    /// looks for it (`codec_parameters_fuzzy_search` falls back to the mime
    /// type alone, case-insensitively).
    fn label_among(label: &str, codecs: &[RTCRtpCodecParameters]) -> bool {
        codecs
            .iter()
            .any(|c| c.rtp_codec.mime_type.eq_ignore_ascii_case(label))
    }

    /// The encoding names of the m-section whose `a=mid` is `mid`, in its
    /// order; empty when there is none or it is rejected (port 0).
    fn sdp_section_codecs(sdp: &str, mid: &str) -> Vec<String> {
        let mut sections: Vec<Vec<&str>> = Vec::new();
        for l in sdp.lines() {
            if l.starts_with("m=") {
                sections.push(vec![l]);
            } else if let Some(s) = sections.last_mut() {
                s.push(l);
            }
        }
        let Some(section) = sections
            .iter()
            .find(|s| s.iter().any(|l| l.strip_prefix("a=mid:") == Some(mid)))
        else {
            return Vec::new();
        };
        let m: Vec<&str> = section[0].split(' ').collect();
        if m.get(1) == Some(&"0") {
            return Vec::new();
        }
        let names: HashMap<&str, &str> = section
            .iter()
            .filter_map(|l| l.strip_prefix("a=rtpmap:"))
            .filter_map(|r| r.split_once(' '))
            .map(|(pt, rest)| (pt, rest.split('/').next().unwrap_or("")))
            .collect();
        m.iter()
            .skip(3)
            .filter_map(|p| names.get(p).map(|n| n.to_string()))
            .collect()
    }

    /// One sender's lasting state.
    pub(crate) struct SendShared {
        ssrc: u32,
        sequencer: Box<dyn Sequencer>,
        /// The video codec negotiated (`codec_id`; 0 not known yet), shared
        /// with the source so it encodes in that codec.
        codec: Arc<std::sync::atomic::AtomicU8>,
    }

    impl SendShared {
        fn new(ssrc: u32) -> Arc<Self> {
            Arc::new(Self {
                ssrc,
                sequencer: Box::new(new_random_sequencer()),
                codec: Arc::default(),
            })
        }
    }

    fn rand_ssrc() -> u32 {
        // Unique enough per process: a counter mixed with the clock.
        use std::hash::{BuildHasher, Hasher};
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(crate::next_id());
        (h.finish() as u32) | 1
    }

    /// Packetize a source's encoded frames for the codec the sender
    /// negotiated and write them to its track, until either goes away;
    /// pass the far side's keyframe requests to the source.
    async fn forward(
        rtp: Arc<dyn RtpSender>,
        track: Arc<TrackLocalStaticRTP>,
        source: Arc<LocalSource>,
        mut rx: tokio::sync::mpsc::Receiver<Encoded>,
        shared: Arc<SendShared>,
    ) {
        // (mime, payload type, clock rate, packetizer) once negotiation has
        // chosen a codec; looked at again every 2 s (a renegotiation). It
        // is looked for on a timer too, not only when a frame comes: a
        // video source encodes nothing until a sender says which codec.
        let mut current: Option<(String, u8, u32, Box<dyn Packetizer>)> = None;
        let mut checked = Instant::now() - Duration::from_secs(10);
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        loop {
            let frame = tokio::select! {
                f = rx.recv() => match f {
                    Some(f) => Some(f),
                    None => break,
                },
                _ = tick.tick() => None,
                ev = poll_rtcp(&track) => {
                    if let Some(TrackLocalEvent::OnRtcpPacket(_)) = ev {
                        source.far_keyframe_request();
                    }
                    None
                }
            };
            if current.is_none() || checked.elapsed() > Duration::from_secs(2) {
                checked = Instant::now();
                match rtp.get_parameters().await {
                    Ok(p) => {
                        if let Some(c) = p.rtp_parameters.codecs.first()
                            && current.as_ref().is_none_or(|(m, pt, ..)| {
                                *m != c.rtp_codec.mime_type || *pt != c.payload_type
                            })
                            && let Ok(payloader) = c.rtp_codec.payloader()
                        {
                            let pk = new_packetizer(
                                Instant::now(),
                                MTU,
                                c.payload_type,
                                shared.ssrc,
                                payloader,
                                shared.sequencer.clone(),
                                c.rtp_codec.clock_rate,
                            );
                            if let Some(v) = VideoCodec::from_mime(&c.rtp_codec.mime_type) {
                                shared.codec.store(codec_id(v), Ordering::Release);
                            }
                            current = Some((
                                c.rtp_codec.mime_type.clone(),
                                c.payload_type,
                                c.rtp_codec.clock_rate,
                                Box::new(pk),
                            ));
                            source.request_keyframe();
                        }
                    }
                    // The sender is gone.
                    Err(_) => break,
                }
            }
            let Some(f) = frame else { continue };
            let Some((mime, _, clock, pk)) = current.as_mut() else {
                continue;
            };
            if f.codec.is_some() && f.codec != VideoCodec::from_mime(mime) {
                continue;
            }
            // `duration` is the time since the previous frame: the
            // timestamp advances by it before this frame, not after
            // (`packetize`'s `samples` is added after the packets it makes,
            // which put every frame's timestamp one gap late).
            let samples = (f.duration.as_secs_f64() * f64::from(*clock)) as u32;
            pk.skip_samples(samples);
            let Ok(packets) = pk.packetize(Instant::now(), &f.data, 0) else {
                continue;
            };
            let mut at = tokio::time::Instant::now();
            for p in packets {
                if f.pace_bps > 0 {
                    // Each packet waits its share of the pacing rate.
                    let bits = (p.payload.len() + 40) as f64 * 8.0;
                    tokio::time::sleep_until(at).await;
                    at = at.max(tokio::time::Instant::now())
                        + Duration::from_secs_f64(bits / f64::from(f.pace_bps));
                }
                // Not bound yet: the frame is dropped.
                if track.write_rtp_with_extensions(p, &[]).await.is_err() {
                    break;
                }
            }
        }
    }

    /// The track's next RTCP event; before the track is bound (no events
    /// yet) wait a little and say nothing.
    async fn poll_rtcp(track: &TrackLocalStaticRTP) -> Option<TrackLocalEvent> {
        match track.poll().await {
            Some(e) => Some(e),
            None => {
                tokio::time::sleep(Duration::from_millis(200)).await;
                None
            }
        }
    }

    impl PeerWorker {
        /// After a remote offer is applied: report through `on_error` a
        /// video m-line the answer will reject for want of a codec
        /// (`unanswerable_video`).
        pub(crate) fn check_remote_offer(&self, sdp: &str) {
            if let Some(msg) = unanswerable_video(sdp) {
                emit(self.peer, Event::Error(RtcError::Media(msg)));
            }
        }

        /// Label a sender's track `codec` for rtc, keeping its id, stream
        /// and SSRC (#1954). rtc's `replace_track` swaps only rtc's copy of
        /// the track: webrtc-rs leaves the track it bound in place, so the
        /// forwarder writes to it as before. Done before negotiation
        /// completes, the sender is then bound with the new label.
        async fn relabel(st: &mut SenderState, stream_id: &str, codec: RTCRtpCodec) {
            let Some(track) = &st.track else { return };
            let id = track.track().await.track_id().to_owned();
            let t = track_for(st.kind, &id, stream_id, st.shared.ssrc, &codec);
            match st.rtp.replace_track(t as Arc<dyn TrackLocal>).await {
                Ok(()) => st.label = codec.mime_type,
                Err(e) => tracing::warn!("rinch-rtc: relabelling a video sender: {e}"),
            }
        }

        /// After a remote offer is applied, before our answer: a video
        /// sender whose label the offer left out (its track was added
        /// before the offer came) is labelled with the codec the answer
        /// will use, the first one the offer and we share. rtc binds the
        /// sender when our answer is set, with that label (#1954).
        pub(crate) async fn relabel_for_offer(&mut self) {
            for st in self.senders.values_mut() {
                if st.settled || st.track.is_none() || st.kind != MediaKind::Video {
                    continue;
                }
                let Ok(p) = st.rtp.get_parameters().await else {
                    continue;
                };
                let codecs = &p.rtp_parameters.codecs;
                if let Some(first) = codecs.first()
                    && !label_among(&st.label, codecs)
                {
                    let codec = first.rtp_codec.clone();
                    Self::relabel(st, &self.stream_id, codec).await;
                }
            }
        }

        /// Before a remote answer is applied (rtc binds our senders while
        /// applying it): a video sender whose label the answer's m-section
        /// leaves out is labelled with the answer's first codec we have
        /// (#1954). Our offer listed every codec, so an answer without our
        /// first one is an ordinary answer from a peer that lacks it.
        pub(crate) async fn relabel_for_answer(&mut self, pc: &Arc<dyn PeerConnection>, sdp: &str) {
            let ours = video_codecs();
            for t in pc.get_transceivers().await {
                let Some(&(_, key)) = self.tx_keys.get(&t.id()) else {
                    continue;
                };
                let Some(st) = self.senders.get_mut(&key) else {
                    continue;
                };
                if st.settled || st.track.is_none() || st.kind != MediaKind::Video {
                    continue;
                }
                let Ok(Some(mid)) = t.mid().await else {
                    continue;
                };
                let names = sdp_section_codecs(sdp, &mid);
                let named = |mime: &str, n: &str| {
                    mime.rsplit('/')
                        .next()
                        .is_some_and(|m| m.eq_ignore_ascii_case(n))
                };
                if names.is_empty() || names.iter().any(|n| named(&st.label, n)) {
                    continue;
                }
                if let Some((codec, _)) = names
                    .iter()
                    .find_map(|n| ours.iter().find(|(c, _)| named(&c.mime_type, n)))
                {
                    Self::relabel(st, &self.stream_id, codec.clone()).await;
                }
            }
        }

        /// After a negotiation completes: each video sender negotiated for
        /// the first time is now bound to the interceptors, or never will
        /// be (rtc binds a sender once). Warn once for one whose label is
        /// still not among the negotiated codecs: it sends, but nothing it
        /// sends is retransmitted and it sends no sender reports.
        pub(crate) async fn check_labels(&mut self, pc: &Arc<dyn PeerConnection>) {
            for t in pc.get_transceivers().await {
                let Some(&(_, key)) = self.tx_keys.get(&t.id()) else {
                    continue;
                };
                let Some(st) = self.senders.get_mut(&key) else {
                    continue;
                };
                if st.settled || st.track.is_none() || st.kind != MediaKind::Video {
                    continue;
                }
                if !matches!(t.mid().await, Ok(Some(_))) {
                    continue;
                }
                let Ok(p) = st.rtp.get_parameters().await else {
                    continue;
                };
                let codecs = &p.rtp_parameters.codecs;
                if codecs.is_empty() {
                    continue;
                }
                st.settled = true;
                if !label_among(&st.label, codecs) {
                    let negotiated: Vec<&str> = codecs
                        .iter()
                        .map(|c| c.rtp_codec.mime_type.as_str())
                        .collect();
                    tracing::warn!(
                        "rinch-rtc: a video sender labelled {:?} was negotiated {negotiated:?}: \
                         it sends, but without retransmission or sender reports",
                        st.label
                    );
                }
            }
        }

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
            track: Arc<TrackLocalStaticRTP>,
            source: Arc<LocalSource>,
            shared: Arc<SendShared>,
        ) -> tokio::task::JoinHandle<()> {
            let rx = source.subscribe((self.peer, sender), shared.codec.clone());
            tokio::spawn(forward(rtp, track, source, rx, shared))
        }

        pub(crate) async fn handle(&mut self, pc: &Arc<dyn PeerConnection>, cmd: MediaCmd) {
            match cmd {
                MediaCmd::AddTrack { sender, tx, source } => {
                    let kind = source.kind();
                    let shared = SendShared::new(rand_ssrc());
                    let (label, prefs) = video_label(pc).await;
                    let track = track_for(kind, source.id(), &self.stream_id, shared.ssrc, &label);
                    match pc.add_track(track.clone() as Arc<dyn TrackLocal>).await {
                        Ok(rtp) => {
                            if kind == MediaKind::Video {
                                set_video_prefs(pc, &rtp, prefs).await;
                            }
                            for t in pc.get_transceivers().await {
                                if let Ok(Some(s)) = t.sender().await
                                    && s.id() == rtp.id()
                                {
                                    self.tx_keys.insert(t.id(), (tx, sender));
                                }
                            }
                            let forward = self.start_forward(
                                sender,
                                rtp.clone(),
                                track.clone(),
                                source.clone(),
                                shared.clone(),
                            );
                            self.senders.insert(
                                sender,
                                SenderState {
                                    rtp,
                                    track: Some(track),
                                    shared,
                                    kind,
                                    source: Some(source),
                                    forward: Some(forward),
                                    label: video_mime(kind, &label),
                                    settled: false,
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
                    let shared = SendShared::new(rand_ssrc());
                    let (label, prefs) = video_label(pc).await;
                    let track = track_for(kind, &id, &self.stream_id, shared.ssrc, &label);
                    match pc
                        .add_transceiver_from_track(
                            track.clone() as Arc<dyn TrackLocal>,
                            Some(init),
                        )
                        .await
                    {
                        Ok(t) => {
                            if kind == MediaKind::Video
                                && let Err(e) = t.set_codec_preferences(prefs).await
                            {
                                tracing::warn!("rinch-rtc: video codec preferences: {e}");
                            }
                            self.tx_keys.insert(t.id(), (tx, sender));
                            if let Ok(Some(rtp)) = t.sender().await {
                                self.senders.insert(
                                    sender,
                                    SenderState {
                                        rtp,
                                        track: Some(track),
                                        shared,
                                        kind,
                                        source: None,
                                        forward: None,
                                        label: video_mime(kind, &label),
                                        settled: false,
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
                    // Labelled with the codec the offer already chose
                    // (`video_label` says why a label matters).
                    let label = st
                        .rtp
                        .get_parameters()
                        .await
                        .ok()
                        .and_then(|p| p.rtp_parameters.codecs.first().map(|c| c.rtp_codec.clone()))
                        .unwrap_or_default();
                    let t = track_for(st.kind, &id, &stream_id, st.shared.ssrc, &label);
                    st.rtp
                        .replace_track(t.clone() as Arc<dyn TrackLocal>)
                        .await
                        .map_err(|e| RtcError::InvalidState(format!("replace_track: {e}")))?;
                    st.track = Some(t.clone());
                    st.label = video_mime(st.kind, &label);
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
                let rx = src.subscribe((peer, sender), st.shared.codec.clone());
                st.forward = Some(tokio::spawn(forward(
                    st.rtp.clone(),
                    track,
                    src.clone(),
                    rx,
                    st.shared.clone(),
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
                            shared: SendShared::new(rand_ssrc()),
                            kind,
                            source: None,
                            forward: None,
                            label: String::new(),
                            settled: false,
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
                            id: track_id.clone(),
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
            // The receiver, for the payload types negotiation gave it (the
            // reader picks its decoder by each packet's).
            let mut receiver = None;
            for t in pc.get_transceivers().await {
                if let Ok(Some(r)) = t.receiver().await
                    && r.track().track_id().await == track_id
                {
                    receiver = Some(r);
                }
            }
            self.readers.retain(|r| !r.is_finished());
            self.readers.push(tokio::spawn(read_remote(
                peer, key, kind, track, receiver, sink, muted,
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
    /// Video depacketizing, for the codec the stream turned out to carry.
    enum Assembler {
        H264(SampleBuilder<H264Packet>),
        #[cfg(feature = "media-vp8")]
        Vp8(SampleBuilder<Vp8Packet>),
    }

    impl Assembler {
        fn new(codec: VideoCodec) -> Self {
            // A frame missing a packet is given up after 500 ms instead of
            // blocking every later one (the builder has no limit otherwise);
            // 512 packets is a frame of about 600 KB.
            let delay = Duration::from_millis(500);
            match codec {
                VideoCodec::H264 => Self::H264(
                    SampleBuilder::new(512, H264Packet::default(), 90_000)
                        .with_max_time_delay(delay),
                ),
                #[cfg(feature = "media-vp8")]
                VideoCodec::Vp8 => Self::Vp8(
                    SampleBuilder::new(512, Vp8Packet::default(), 90_000)
                        .with_max_time_delay(delay),
                ),
            }
        }

        fn push(&mut self, p: rtc::rtp::Packet) {
            match self {
                Self::H264(b) => b.push(Instant::now(), p),
                #[cfg(feature = "media-vp8")]
                Self::Vp8(b) => b.push(Instant::now(), p),
            }
        }

        /// The next whole frame, and whether packets were lost before it.
        fn pop(&mut self) -> Option<(bytes::Bytes, bool)> {
            let s = match self {
                Self::H264(b) => b.pop(Instant::now()),
                #[cfg(feature = "media-vp8")]
                Self::Vp8(b) => b.pop(Instant::now()),
            }?;
            Some((s.data, s.prev_dropped_packets > 0))
        }
    }

    /// Opus packets back in sequence order, with losses concealed: a gap
    /// is filled from the next packet's in-band FEC (the frame just
    /// before it) and Opus's packet-loss concealment (any earlier ones);
    /// duplicates and packets older than what was played are dropped. It
    /// holds one packet (20 ms) waiting for a late one: longer would
    /// outlast the mixer's 60 ms of buffer.
    pub(crate) struct OpusJitter {
        dec: opus::Decoder,
        /// Extended (wrap-free) sequence number of the next packet to play.
        next: Option<u64>,
        last_ext: u64,
        pending: BTreeMap<u64, bytes::Bytes>,
        pcm: Vec<f32>,
        /// Samples per channel of the last packet decoded: what one lost
        /// packet is concealed as (10, 20, 40 or 60 ms; 20 to start).
        frame: usize,
    }

    /// 20 ms at 48 kHz.
    const OPUS_FRAME: usize = 960;
    const OPUS_HOLD: usize = 1;
    /// A gap of more packets than this is not concealed packet by packet
    /// but taken as a resync (a sender that restarted its sequence, or a
    /// remote sending nonsense): about 300 ms of 20 ms packets. Without
    /// the bound, two packets 0x7fff apart made 655 s of concealment in one
    /// push, on the worker every peer shares.
    const MAX_GAP: u64 = 15;
    /// A packet this far behind is a sequence that went backwards, not a
    /// late one: start again from it rather than drop everything after it.
    const MAX_LATE: u64 = 50;

    impl OpusJitter {
        pub(crate) fn new() -> Option<Self> {
            Some(Self {
                dec: opus::Decoder::new(48_000, opus::Channels::Stereo).ok()?,
                next: None,
                last_ext: 0,
                pending: BTreeMap::new(),
                pcm: vec![0f32; 5760 * 2],
                frame: OPUS_FRAME,
            })
        }

        fn extend(&mut self, seq: u16) -> u64 {
            if self.next.is_none() && self.pending.is_empty() && self.last_ext == 0 {
                self.last_ext = u64::from(seq) + (1 << 16);
                return self.last_ext;
            }
            let base = self.last_ext & !0xffff;
            let mut ext = base | u64::from(seq);
            if ext + 0x8000 < self.last_ext {
                ext += 1 << 16;
            } else if ext > self.last_ext + 0x8000 && ext >= 1 << 16 {
                ext -= 1 << 16;
            }
            if ext > self.last_ext {
                self.last_ext = ext;
            }
            ext
        }

        /// Take one packet; append what can be played now (48 kHz stereo)
        /// to `out`.
        pub(crate) fn push(&mut self, seq: u16, payload: bytes::Bytes, out: &mut Vec<f32>) {
            let ext = self.extend(seq);
            let next = *self.next.get_or_insert(ext);
            if ext < next {
                if next - ext <= MAX_LATE {
                    return; // late or a duplicate
                }
                // The sequence went back: start again from here.
                self.pending.clear();
                self.last_ext = ext;
                self.next = Some(ext);
            } else if ext - next > MAX_GAP {
                // A jump forward: play what is waiting, then go on from
                // here, with nothing concealed in between.
                let waiting = std::mem::take(&mut self.pending);
                for p in waiting.values() {
                    self.decode(p, false, out);
                }
                self.next = Some(ext);
            }
            if self.pending.contains_key(&ext) {
                return; // a duplicate
            }
            self.pending.insert(ext, payload);
            loop {
                let next = self.next.unwrap_or(ext);
                if let Some(p) = self.pending.remove(&next) {
                    self.decode(&p, false, out);
                    self.next = Some(next + 1);
                    continue;
                }
                // A gap: wait for it while there is room, then conceal it
                // (at most MAX_GAP packets, as above).
                if self.pending.len() <= OPUS_HOLD {
                    break;
                }
                let Some((&first, first_payload)) = self.pending.iter().next() else {
                    break;
                };
                let first_payload = first_payload.clone();
                for lost in next..first {
                    if lost + 1 == first {
                        // The next packet's FEC carries this frame.
                        self.decode(&first_payload, true, out);
                    } else {
                        self.decode(&[], false, out);
                    }
                }
                self.next = Some(first);
            }
        }

        fn decode(&mut self, data: &[u8], fec: bool, out: &mut Vec<f32>) {
            // Concealment is one packet's worth: the lost packet's own
            // duration for FEC (the next packet's, which carries it), the
            // last packet's for PLC.
            let max = self.pcm.len() / 2;
            let frames = if data.is_empty() {
                self.frame
            } else if fec {
                self.dec.get_nb_samples(data).unwrap_or(self.frame).min(max)
            } else {
                max
            };
            match self
                .dec
                .decode_float(data, &mut self.pcm[..frames * 2], fec)
            {
                Ok(n) => {
                    if !data.is_empty() && !fec {
                        self.frame = n.clamp(1, max);
                    }
                    out.extend_from_slice(&self.pcm[..n * 2]);
                }
                Err(_) if !data.is_empty() && !fec => {
                    // A packet libopus refuses: conceal it instead.
                    let frames = self.frame;
                    if let Ok(n) = self
                        .dec
                        .decode_float(&[], &mut self.pcm[..frames * 2], false)
                    {
                        out.extend_from_slice(&self.pcm[..n * 2]);
                    }
                }
                Err(_) => {}
            }
        }
    }

    /// Which received video frames go to the decoder, and when to ask for
    /// a keyframe. A gap in the sequence numbers is a lost packet, which
    /// the far side retransmits on our NACK (the interceptors send it):
    /// the assembler holds the frame meanwhile, so nothing corrupt is
    /// decoded. A gap not repaired within `REPAIR` is given up on at once
    /// (a PLI, and frames held back until the keyframe) rather than when
    /// the assembler drops the frame 500 ms later. Frames also wait for a
    /// keyframe at the start, after a codec change, and after the
    /// assembler reports a frame lost.
    pub(crate) struct KeyGate {
        pub(crate) awaiting: bool,
        last_seq: Option<u16>,
        /// Sequence numbers in a gap not yet filled, and when the oldest
        /// such gap opened.
        missing: Vec<u16>,
        gap_at: Option<Instant>,
    }

    /// How long a gap may wait for a retransmission: a few round trips.
    pub(crate) const REPAIR: Duration = Duration::from_millis(200);

    impl Default for KeyGate {
        fn default() -> Self {
            Self {
                awaiting: true,
                last_seq: None,
                missing: Vec::new(),
                gap_at: None,
            }
        }
    }

    impl KeyGate {
        /// A packet arrived (before the assembler): note a gap, or the
        /// filling of one.
        pub(crate) fn packet(&mut self, seq: u16, now: Instant) {
            match self.last_seq {
                Some(last) => {
                    let d = seq.wrapping_sub(last);
                    if d < 0x8000 {
                        // Newer (by up to half the space).
                        if d > 256 {
                            // A jump no retransmission will fill.
                            self.give_up();
                        } else if d > 1 {
                            self.missing.extend((1..d).map(|k| last.wrapping_add(k)));
                            self.gap_at.get_or_insert(now);
                        }
                        self.last_seq = Some(seq);
                    } else {
                        // Older: a retransmission (or reordering) filling a
                        // gap.
                        self.missing.retain(|m| *m != seq);
                    }
                }
                None => self.last_seq = Some(seq),
            }
            if self.missing.is_empty() {
                self.gap_at = None;
            } else if self.gap_at.is_some_and(|t| now.duration_since(t) >= REPAIR) {
                self.give_up();
            }
        }

        fn give_up(&mut self) {
            self.awaiting = true;
            self.missing.clear();
            self.gap_at = None;
        }

        /// Whether an assembled frame goes to the decoder (`lost`: the
        /// assembler dropped packets before it).
        pub(crate) fn admit(&mut self, codec: VideoCodec, frame: &[u8], lost: bool) -> bool {
            if lost {
                self.awaiting = true;
            }
            if self.awaiting {
                if !codec.is_keyframe(frame) {
                    return false;
                }
                self.awaiting = false;
            }
            true
        }
    }

    /// The video payload types a receiver negotiated, and their codecs.
    async fn video_payload_types(receiver: Option<&dyn RtpReceiver>) -> HashMap<u8, VideoCodec> {
        let mut m = HashMap::new();
        if let Some(r) = receiver
            && let Ok(p) = r.get_parameters().await
        {
            for c in p.rtp_parameters.codecs {
                if let Some(v) = VideoCodec::from_mime(&c.rtp_codec.mime_type) {
                    m.insert(c.payload_type, v);
                }
            }
        }
        m
    }

    async fn read_remote(
        peer: u64,
        key: u64,
        kind: MediaKind,
        track: Arc<dyn TrackRemote>,
        receiver: Option<Arc<dyn RtpReceiver>>,
        sink: Arc<RemoteSink>,
        muted: Arc<std::sync::atomic::AtomicBool>,
    ) {
        let (pli_tx, mut pli_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        // Video: the decoder and depacketizer for the codec the packets
        // carry, by payload type. A renegotiation can change the codec
        // mid-stream (VP8 to H.264): a packet of another codec replaces
        // both.
        let mut video: Option<(
            VideoCodec,
            Option<std::sync::mpsc::SyncSender<bytes::Bytes>>,
            Assembler,
        )> = None;
        let mut pt_codecs: HashMap<u8, VideoCodec> = HashMap::new();
        let mut gate = KeyGate::default();
        let mut jitter = match kind {
            MediaKind::Audio => OpusJitter::new(),
            MediaKind::Video => None,
        };
        let mut pcm: Vec<f32> = Vec::new();
        let mut ssrc = 0u32;
        let mut last_pli = Instant::now() - Duration::from_secs(2);
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
                                    if drop_video_packet() {
                                        continue;
                                    }
                                    if p.header.marker
                                        && let Ok(mut t) = sink.frame_times.lock()
                                    {
                                        if t.len() >= 512 {
                                            t.pop_front();
                                        }
                                        t.push_back((Instant::now(), p.header.timestamp));
                                    }
                                    let pt = p.header.payload_type;
                                    if !pt_codecs.contains_key(&pt) {
                                        pt_codecs = video_payload_types(receiver.as_deref()).await;
                                        if pt_codecs.is_empty() {
                                            // No parameters to read: the
                                            // stream's first codec.
                                            let mime = track.codec(ssrc).await.map(|c| c.mime_type).unwrap_or_default();
                                            if let Some(c) = VideoCodec::from_mime(&mime) {
                                                pt_codecs.insert(pt, c);
                                            }
                                        }
                                    }
                                    let Some(&codec) = pt_codecs.get(&pt) else { continue };
                                    if video.as_ref().is_none_or(|(c, ..)| *c != codec) {
                                        // The old decoder thread ends with
                                        // its channel. The new one starts
                                        // from a keyframe.
                                        gate.awaiting = true;
                                        video = Some((
                                            codec,
                                            start_video_decoder(key, codec, sink.clone(), pli_tx.clone()),
                                            Assembler::new(codec),
                                        ));
                                    }
                                    let Some((codec, decoder, asm)) = video.as_mut() else { continue };
                                    gate.packet(p.header.sequence_number, Instant::now());
                                    asm.push(p);
                                    while let Some((frame, lost)) = asm.pop() {
                                        if gate.admit(*codec, &frame, lost)
                                            && let Some(d) = decoder
                                        {
                                            let _ = d.try_send(frame);
                                        }
                                    }
                                    if gate.awaiting && last_pli.elapsed() > Duration::from_millis(500) {
                                        last_pli = Instant::now();
                                        let _ = pli_tx.send(());
                                    }
                                }
                                MediaKind::Audio => {
                                    if drop_audio_packet() {
                                        continue;
                                    }
                                    if let Some(j) = jitter.as_mut() {
                                        pcm.clear();
                                        j.push(p.header.sequence_number, p.payload, &mut pcm);
                                        if !pcm.is_empty() {
                                            mixer_push(peer, key, &sink, &pcm);
                                        }
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

    #[cfg(test)]
    mod jitter_tests {
        use super::OpusJitter;

        /// 1 s of a voice-like signal, as the pump encodes it: 20 ms mono
        /// frames, in-band FEC on. Speech-like matters: libopus puts FEC
        /// only in frames its voice detector calls active speech, so a
        /// steady tone (or a steady buzz) gets none and its losses are PLC.
        fn packets() -> Vec<bytes::Bytes> {
            speech(50, 20)
        }

        /// `frames` packets of `ms` each of the same signal.
        fn speech(frames: usize, ms: usize) -> Vec<bytes::Bytes> {
            let n = 48 * ms;
            let mut enc =
                opus::Encoder::new(48_000, opus::Channels::Mono, opus::Application::Voip).unwrap();
            enc.set_inband_fec(true).unwrap();
            enc.set_packet_loss_perc(10).unwrap();
            let mut out = vec![0u8; 4000];
            (0..frames)
                .map(|f| {
                    let pcm: Vec<f32> = (0..n)
                        .map(|i| {
                            let t = (f * n + i) as f32 / 48_000.0;
                            // Syllables at 4 Hz with pauses, the pitch gliding
                            // 100-180 Hz: what SILK's voice detector takes for
                            // speech (it sends FEC only for active speech).
                            let env = (t * 4.0 * std::f32::consts::TAU).sin().max(0.0).powi(2);
                            let phase = 140.0 * t
                                - 40.0 / (1.3 * std::f32::consts::TAU)
                                    * (t * 1.3 * std::f32::consts::TAU).cos();
                            let v: f32 = (1..8)
                                .map(|k| {
                                    (phase * k as f32 * std::f32::consts::TAU).sin() / k as f32
                                })
                                .sum();
                            0.2 * env * v
                        })
                        .collect();
                    let n = enc.encode_float(&pcm, &mut out).unwrap();
                    bytes::Bytes::copy_from_slice(&out[..n])
                })
                .collect()
        }

        /// Play `(sequence number, packet index)` pairs.
        fn play_seqs(packets: &[bytes::Bytes], seqs: &[(u16, usize)]) -> Vec<f32> {
            let mut j = OpusJitter::new().unwrap();
            let mut out = Vec::new();
            for &(s, i) in seqs {
                j.push(s, packets[i].clone(), &mut out);
            }
            out
        }

        /// A jump in the sequence (two packets from any remote can make
        /// one) is a resync: bounded work and output in one push, and the
        /// packets after it are played, forwards or backwards. #1947's
        /// second review: +0x7fff took 0.8 s and made 655 s of
        /// concealment; +0x8000 or a step back dropped everything after it
        /// as "late" for minutes.
        #[test]
        fn a_sequence_jump_is_a_resync() {
            let p = speech(50, 20);
            for jump in [100u16, 0x7fff, 0x8000, 0x9000, 0xc000] {
                let mut seqs: Vec<(u16, usize)> = (0..25).map(|i| (1000 + i as u16, i)).collect();
                seqs.extend(
                    (25..50).map(|i| (1000u16.wrapping_add(jump).wrapping_add(i as u16), i)),
                );
                let t = std::time::Instant::now();
                let out = play_seqs(&p, &seqs);
                assert!(
                    t.elapsed() < std::time::Duration::from_millis(200),
                    "+{jump:#x}: {:?}",
                    t.elapsed()
                );
                // 50 packets of 20 ms, give or take the held one.
                let ms = out.len() / 96;
                assert!(
                    (960..=1000).contains(&ms),
                    "+{jump:#x}: {ms} ms out for 1000 ms in"
                );
            }
            let mut seqs: Vec<(u16, usize)> = (0..25).map(|i| (5000 + i as u16, i)).collect();
            seqs.extend((25..50).map(|i| (100 + i as u16, i)));
            let ms = play_seqs(&p, &seqs).len() / 96;
            assert!(
                (960..=1000).contains(&ms),
                "backwards: {ms} ms out for 1000 ms in"
            );
            // Wrapping through 65535 is no jump.
            let wrap: Vec<(u16, usize)> = (0..50)
                .map(|i| (65_530u16.wrapping_add(i as u16), i))
                .collect();
            let straight: Vec<(u16, usize)> = (0..50).map(|i| (1000 + i as u16, i)).collect();
            assert_eq!(play_seqs(&p, &wrap), play_seqs(&p, &straight));
        }

        /// A lost packet is concealed as long as the packets are: 60 ms
        /// packets were spliced (40 ms short a loss), 10 ms ones stretched.
        #[test]
        fn concealment_follows_the_packet_duration() {
            for (ms, n) in [(10usize, 100usize), (40, 25), (60, 20)] {
                let p = speech(n, ms);
                let all: Vec<(u16, usize)> = (0..n).map(|i| (1000 + i as u16, i)).collect();
                let clean = play_seqs(&p, &all).len();
                // Two lost in a row: the first is concealed by PLC, the
                // second rebuilt from the next packet's FEC.
                let lossy: Vec<(u16, usize)> = all
                    .iter()
                    .copied()
                    .filter(|&(_, i)| i != n / 2 && i != n / 2 + 1)
                    .collect();
                assert_eq!(
                    play_seqs(&p, &lossy).len(),
                    clean,
                    "{ms} ms packets, two lost"
                );
            }
        }

        fn play(packets: &[bytes::Bytes], order: &[usize]) -> Vec<f32> {
            let mut j = OpusJitter::new().unwrap();
            let mut out = Vec::new();
            for &i in order {
                j.push(1000 + i as u16, packets[i].clone(), &mut out);
            }
            out
        }

        /// Error against the loss-free decode over the frame `lost`.
        fn error(clean: &[f32], got: &[f32], lost: usize) -> f32 {
            let r = lost * 1920..(lost + 1) * 1920;
            clean[r.clone()]
                .iter()
                .zip(&got[r])
                .map(|(a, b)| (a - b) * (a - b))
                .sum::<f32>()
                .sqrt()
        }

        #[test]
        fn a_lost_packet_is_rebuilt_from_the_next_ones_fec() {
            let p = packets();
            let all: Vec<usize> = (0..50).collect();
            let clean = play(&p, &all);
            assert_eq!(clean.len(), 50 * 1920);
            // Frame 30 lost: rebuilt from 31's FEC.
            let lossy = play(
                &p,
                &all.iter().copied().filter(|&i| i != 30).collect::<Vec<_>>(),
            );
            // Frames 30 and 31 lost: 30 has no FEC to come from (PLC), 31
            // is rebuilt from 32's.
            let two = play(
                &p,
                &all.iter()
                    .copied()
                    .filter(|&i| i != 30 && i != 31)
                    .collect::<Vec<_>>(),
            );
            assert!(
                lossy.len() >= 49 * 1920,
                "nothing spliced out: {}",
                lossy.len()
            );
            let fec = error(&clean, &lossy, 30);
            let plc = error(&clean, &two, 30);
            assert!(
                fec * 2.0 < plc,
                "FEC {fec:.3} should be well under PLC {plc:.3} against the clean decode"
            );
        }

        #[test]
        fn late_and_duplicate_packets_are_dropped_and_order_restored() {
            let p = packets();
            let all: Vec<usize> = (0..50).collect();
            let clean = play(&p, &all);
            let mut order = all.clone();
            order.swap(10, 11); // one packet a frame late
            order.insert(20, 19); // a duplicate
            let got = play(&p, &order);
            assert_eq!(got.len(), clean.len());
            assert!(error(&clean, &got, 10) < 1e-3 && error(&clean, &got, 19) < 1e-3);
        }
    }

    #[cfg(all(test, feature = "media"))]
    mod unanswerable_tests {
        use super::unanswerable_video;

        const AUDIO: &str = "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=rtpmap:111 opus/48000/2\r\n";

        fn video(port: &str, codecs: &[(&str, &str)]) -> String {
            let pts: Vec<&str> = codecs.iter().map(|(p, _)| *p).collect();
            let mut s = format!("m=video {port} UDP/TLS/RTP/SAVPF {}\r\n", pts.join(" "));
            for (p, n) in codecs {
                s.push_str(&format!("a=rtpmap:{p} {n}/90000\r\n"));
            }
            s
        }

        /// Only a live video m-line with no codec in common is reported, on
        /// any m-line (not the first video one alone), naming what each side
        /// has. Every `media` build has H.264 (`load_openh264` aside).
        #[test]
        fn only_a_live_video_line_with_no_common_codec_is_reported() {
            let h264 = video("9", &[("102", "H264")]);
            let vp9 = video("9", &[("98", "VP9"), ("99", "AV1")]);
            assert_eq!(unanswerable_video(&format!("{AUDIO}{h264}")), None);
            // Lower case is the same codec.
            assert_eq!(unanswerable_video(&video("9", &[("102", "h264")])), None);
            // A rejected (port 0) m-line is not ours to report.
            assert_eq!(unanswerable_video(&video("0", &[("98", "VP9")])), None);
            assert_eq!(unanswerable_video(AUDIO), None);
            // The second video m-line, after a fine first one.
            let m = unanswerable_video(&format!("{AUDIO}{h264}{vp9}")).expect("reported");
            assert!(m.contains("VP9, AV1") && m.contains("H264"), "{m}");
            assert!(!m.contains("media-vp8"), "no VP8 in the offer: {m}");
            let m = unanswerable_video(&video("9", &[("96", "VP8")]));
            if cfg!(feature = "media-vp8") {
                assert_eq!(m, None);
            } else {
                let m = m.expect("reported");
                assert!(m.contains("VP8") && m.contains("`media-vp8`"), "{m}");
            }
        }
    }

    #[cfg(test)]
    mod gate_tests {
        use super::{KeyGate, REPAIR};
        use crate::native_codec::VideoCodec;

        /// H.264 access units: an IDR slice (NAL type 5) is a keyframe, a
        /// non-IDR slice (type 1) is not. In H.264 so that every `media`
        /// build runs this (VP8 is `media-vp8`'s).
        const KEY: &[u8] = &[0, 0, 0, 1, 0x65, 9];
        const DELTA: &[u8] = &[0, 0, 0, 1, 0x41, 9];

        /// VP8 frame tags: bit 0 clear is a key frame.
        #[cfg(feature = "media-vp8")]
        #[test]
        fn vp8_key_frames_are_told_by_the_frame_tag() {
            let v = VideoCodec::Vp8;
            assert!(v.is_keyframe(&[0x10, 0, 0]));
            assert!(!v.is_keyframe(&[0x11, 0, 0]));
            assert!(!v.is_keyframe(&[]));
            let mut g = KeyGate::default();
            g.packet(10, std::time::Instant::now());
            assert!(!g.admit(v, &[0x11, 0, 0], false));
            assert!(g.admit(v, &[0x10, 0, 0], false));
        }

        #[test]
        fn frames_wait_for_a_keyframe_after_a_gap() {
            let t0 = std::time::Instant::now();
            let at = |ms: u64| t0 + std::time::Duration::from_millis(ms);
            let mut g = KeyGate::default();
            let v = VideoCodec::H264;
            // From the start: deltas wait for the first keyframe.
            g.packet(10, at(0));
            assert!(!g.admit(v, DELTA, false));
            assert!(g.admit(v, KEY, false));
            g.packet(11, at(10));
            assert!(g.admit(v, DELTA, false));
            // A gap (12 lost) waits for the retransmission...
            g.packet(13, at(20));
            assert!(!g.awaiting, "a gap is not given up on at once");
            // ...which comes: nothing to give up.
            g.packet(12, at(60));
            g.packet(14, at(400));
            assert!(!g.awaiting, "a repaired gap");
            assert!(g.admit(v, DELTA, false));
            // A gap that stays open past REPAIR is given up on: the next
            // packet after that sees it, and deltas wait for a keyframe.
            g.packet(16, at(410));
            g.packet(17, at(500));
            assert!(!g.awaiting);
            g.packet(18, at(410 + REPAIR.as_millis() as u64));
            assert!(g.awaiting, "an unrepaired gap past REPAIR");
            assert!(!g.admit(v, DELTA, false));
            assert!(g.admit(v, KEY, false));
            assert!(g.admit(v, DELTA, false));
            // Wrapping is no gap; a big jump is given up on at once.
            let mut w = KeyGate::default();
            w.packet(65_535, at(0));
            w.packet(0, at(10));
            assert!(w.admit(v, KEY, false));
            w.packet(1, at(20));
            assert!(w.admit(v, DELTA, false));
            w.packet(2000, at(30));
            assert!(w.awaiting);
            // The assembler's own loss report counts too.
            let mut a = KeyGate::default();
            assert!(a.admit(v, KEY, false));
            assert!(!a.admit(v, DELTA, true));
            // H.264: an IDR slice (NAL type 5) is the keyframe.
            let mut h = KeyGate::default();
            assert!(!h.admit(VideoCodec::H264, &[0, 0, 0, 1, 0x41, 9], false));
            assert!(h.admit(
                VideoCodec::H264,
                &[0, 0, 0, 1, 0x67, 9, 0, 0, 1, 0x65, 9],
                false
            ));
        }
    }
}
