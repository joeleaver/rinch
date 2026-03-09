//! libmpv-based video player backend with software frame rendering.
//!
//! Uses the `libmpv2` crate to interface with system-installed libmpv.
//! A background thread polls mpv events (property changes, end-of-file, etc.)
//! and sends updates to the main thread via a channel. The main thread
//! applies these updates to Signals during the event loop tick.
//!
//! Video frames are extracted via mpv's software render API: mpv decodes
//! (with hardware acceleration when available via `--hwdec=auto`) and
//! renders each frame into a CPU pixel buffer. The pixels are then uploaded
//! to a wgpu texture for compositing with the Vello UI layer.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::os::raw::c_int;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rinch_core::Signal;

use crate::player::{PlaybackState, VideoPlayer, VideoPlayerBackend};

// ── Property observation IDs ─────────────────────────────────────────────────

const PROP_TIME_POS: u64 = 1;
const PROP_DURATION: u64 = 2;
const PROP_PAUSE: u64 = 3;
const PROP_VOLUME: u64 = 4;
const PROP_MUTE: u64 = 5;
const PROP_DWIDTH: u64 = 6;
const PROP_DHEIGHT: u64 = 7;
const PROP_EOF_REACHED: u64 = 8;
const PROP_CACHE_DURATION: u64 = 9;
const PROP_PAUSED_FOR_CACHE: u64 = 10;

// ── Updates from mpv event thread → main thread ─────────────────────────────

enum MpvUpdate {
    Position(f64),
    Duration(f64),
    Paused(bool),
    Volume(f32),
    Muted(bool),
    EofReached(bool),
    CacheDuration(f64),
    PausedForCache(bool),
    FileLoaded,
    EndFile,
    Error(String),
    VideoWidth(i64),
    VideoHeight(i64),
}

// ── MpvPlayer ───────────────────────────────────────────────────────────────

/// libmpv-based video player with software frame rendering.
pub struct MpvPlayer {
    mpv: Arc<Mutex<libmpv2::Mpv>>,
    /// Reusable pixel buffer for SW rendering (avoids per-frame allocation).
    render_buffer: RefCell<Vec<u8>>,
    /// SW render context for frame extraction (raw pointer, main-thread only).
    render_ctx: *mut libmpv2_sys::mpv_render_context,
    /// Current video display width (from dwidth property).
    video_width: Arc<Mutex<u32>>,
    /// Current video display height (from dheight property).
    video_height: Arc<Mutex<u32>>,
    /// Receive updates from the mpv event polling thread.
    update_rx: mpsc::Receiver<MpvUpdate>,
    /// Signal references for applying updates on the main thread.
    signals: PlayerSignals,
    /// Flag to stop the event thread.
    running: Arc<Mutex<bool>>,
    /// Set by mpv's render update callback when a new frame is available.
    /// Checked and cleared on each poll to render only when needed.
    needs_render: Arc<AtomicBool>,
    /// Set to true on EndFile — short-circuits all mpv interaction in poll_updates.
    /// Cleared when play() restarts playback.
    finished: Cell<bool>,
    /// Optional frame sink for delivering frames to the RenderSurface pipeline.
    frame_sink: RefCell<Option<crate::player::FrameSink>>,
}

// Safety: MpvPlayer is only used from the main thread (behind Rc<RefCell<...>>).
// The render_ctx raw pointer is only accessed from poll_updates() on the main thread.
unsafe impl Send for MpvPlayer {}

#[derive(Clone)]
struct PlayerSignals {
    playing: Signal<bool>,
    position: Signal<f64>,
    duration: Signal<f64>,
    volume: Signal<f32>,
    muted: Signal<bool>,
    buffered: Signal<f64>,
    state: Signal<PlaybackState>,
}

impl std::fmt::Debug for MpvPlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MpvPlayer").finish_non_exhaustive()
    }
}

/// Update callback called by mpv from an internal thread when a new frame is available.
unsafe extern "C" fn render_update_callback(ctx: *mut c_void) {
    if !ctx.is_null() {
        let flag = unsafe { &*(ctx as *const AtomicBool) };
        flag.store(true, Ordering::Release);
    }
}

impl VideoPlayerBackend for MpvPlayer {
    fn poll_updates(&self) {
        // Short-circuit: once playback ended, stop all mpv interaction.
        // This prevents any render API calls that might block after EndFile.
        if self.finished.get() {
            return;
        }

        // Drain mpv event updates (position, duration, state, video dimensions)
        while let Ok(update) = self.update_rx.try_recv() {
            match update {
                MpvUpdate::Position(pos) => self.signals.position.set(pos),
                MpvUpdate::Duration(dur) => self.signals.duration.set(dur),
                MpvUpdate::Paused(paused) => {
                    self.signals.playing.set(!paused);
                    if paused {
                        self.signals.state.set(PlaybackState::Paused);
                    } else {
                        self.signals.state.set(PlaybackState::Playing);
                    }
                }
                MpvUpdate::Volume(vol) => self.signals.volume.set(vol),
                MpvUpdate::Muted(m) => self.signals.muted.set(m),
                MpvUpdate::EofReached(eof) => {
                    if eof {
                        // With keep-open=yes, mpv pauses at the last frame and sets
                        // eof-reached=true instead of sending EndFile.
                        self.signals.playing.set(false);
                        self.signals.state.set(PlaybackState::Ended);
                        self.finished.set(true);
                        // Active state derived from ACTIVE_PLAYERS list
                    }
                }
                MpvUpdate::CacheDuration(secs) => {
                    self.signals.buffered.set(secs);
                }
                MpvUpdate::PausedForCache(stalled) => {
                    if stalled {
                        self.signals.state.set(PlaybackState::Loading);
                    } else if self.signals.playing.get() {
                        self.signals.state.set(PlaybackState::Playing);
                    }
                }
                MpvUpdate::FileLoaded => {
                    self.signals.state.set(PlaybackState::Playing);
                }
                MpvUpdate::EndFile => {
                    self.signals.playing.set(false);
                    self.signals.state.set(PlaybackState::Ended);
                    self.finished.set(true);
                    // Active state derived from ACTIVE_PLAYERS list
                }
                MpvUpdate::Error(msg) => {
                    self.signals.state.set(PlaybackState::Error(msg));
                }
                MpvUpdate::VideoWidth(w) => {
                    if w > 0 {
                        if let Ok(mut vw) = self.video_width.lock() {
                            *vw = w as u32;
                        }
                    }
                }
                MpvUpdate::VideoHeight(h) => {
                    if h > 0 {
                        if let Ok(mut vh) = self.video_height.lock() {
                            *vh = h as u32;
                        }
                    }
                }
            }
        }

        // Try to query video dimensions synchronously if we don't have them yet.
        {
            let have_dims = {
                let vw = self.video_width.lock().map(|v| *v).unwrap_or(0);
                let vh = self.video_height.lock().map(|v| *v).unwrap_or(0);
                vw > 0 && vh > 0
            };
            if !have_dims {
                if let Ok(mpv) = self.mpv.try_lock() {
                    if let Ok(w) = mpv.get_property::<i64>("width") {
                        if w > 0 {
                            if let Ok(mut vw) = self.video_width.lock() {
                                *vw = w as u32;
                            }
                        }
                    }
                    if let Ok(h) = mpv.get_property::<i64>("height") {
                        if h > 0 {
                            if let Ok(mut vh) = self.video_height.lock() {
                                *vh = h as u32;
                            }
                        }
                    }
                }
            }
        }

        // Render when mpv signals a new frame is available via the update callback.
        // Since we use BLOCK=0 and never call report_swap(), rendering is a
        // non-blocking snapshot of the current audio-clock position — safe to call
        // at display refresh rate without causing fast-forward.
        if !self.render_ctx.is_null() {
            let state = self.signals.state.get();
            let should_render = matches!(state, PlaybackState::Playing | PlaybackState::Loading);
            if should_render && self.needs_render.swap(false, Ordering::AcqRel) {
                self.render_sw_frame();
            }
        }
    }

    fn play(&self) {
        self.finished.set(false);
        if let Ok(mpv) = self.mpv.lock()
            && let Err(e) = mpv.set_property("pause", false)
        {
            tracing::error!("mpv play error: {e}");
        }
    }

    fn pause(&self) {
        if let Ok(mpv) = self.mpv.lock()
            && let Err(e) = mpv.set_property("pause", true)
        {
            tracing::error!("mpv pause error: {e}");
        }
    }

    fn seek(&self, seconds: f64) {
        if let Ok(mpv) = self.mpv.lock()
            && let Err(e) = mpv.command("seek", &[&seconds.to_string(), "absolute"])
        {
            tracing::error!("mpv seek error: {e}");
        }
    }

    fn set_volume(&self, vol: f32) {
        if let Ok(mpv) = self.mpv.lock() {
            let mpv_vol = (vol * 100.0) as i64;
            if let Err(e) = mpv.set_property("volume", mpv_vol) {
                tracing::error!("mpv set_volume error: {e}");
            }
        }
    }

    fn set_muted(&self, muted: bool) {
        if let Ok(mpv) = self.mpv.lock()
            && let Err(e) = mpv.set_property("mute", muted)
        {
            tracing::error!("mpv set_muted error: {e}");
        }
    }

    fn set_source(&self, src: &str) {
        if let Ok(mpv) = self.mpv.lock() {
            if let Err(e) = mpv.command("loadfile", &[src]) {
                tracing::error!("mpv loadfile error: {e:?}");
            }
        }
    }

    fn cleanup(&self) {
        // Signal the event thread to stop
        if let Ok(mut running) = self.running.lock() {
            *running = false;
        }
        if let Ok(mpv) = self.mpv.lock() {
            let _ = mpv.command("stop", &[]);
        }
        // Note: render context is freed in Drop
    }

    fn set_frame_sink(&self, sink: crate::player::FrameSink) {
        *self.frame_sink.borrow_mut() = Some(sink);
    }
}

impl MpvPlayer {
    /// Render the current video frame using the SW render API and deliver via the frame sink.
    ///
    /// IMPORTANT: mpv requires `mpv_render_context_render` to be called every time
    /// the update callback fires, otherwise it won't fire again. When video dimensions
    /// aren't known yet, we render into a small scratch buffer to advance mpv's state.
    fn render_sw_frame(&self) {
        let (w, h) = {
            let vw = self.video_width.lock().map(|v| *v).unwrap_or(0);
            let vh = self.video_height.lock().map(|v| *v).unwrap_or(0);
            (vw, vh)
        };

        // If dimensions aren't known yet, still call render into a scratch buffer
        // so mpv advances its internal state and keeps firing update callbacks.
        if w == 0 || h == 0 {
            self.render_scratch();
            return;
        }

        let stride = w as usize * 4; // 4 bytes per pixel (rgb0)
        let buf_size = stride * h as usize;

        let mut buf = self.render_buffer.borrow_mut();
        if buf.len() != buf_size {
            buf.resize(buf_size, 0);
        }

        self.do_sw_render(buf.as_mut_ptr(), w, h, stride);

        // Fix alpha channel: rgb0 format gives A=0, we need A=255 for opaque video
        for pixel in buf.chunks_exact_mut(4) {
            pixel[3] = 255;
        }

        // Deliver frame through the sink (RenderSurface compositing pipeline).
        if let Some(sink) = self.frame_sink.borrow().as_ref() {
            sink(&buf, w, h);
        }
    }

    /// Render into a small scratch buffer when dimensions aren't known yet.
    /// This ensures mpv's render pipeline advances and keeps firing callbacks.
    fn render_scratch(&self) {
        let (w, h) = (16u32, 16u32);
        let stride = w as usize * 4;
        let mut scratch = vec![0u8; stride * h as usize];
        self.do_sw_render(scratch.as_mut_ptr(), w, h, stride);
    }

    /// Call mpv_render_context_render with the given buffer.
    fn do_sw_render(&self, pixel_ptr: *mut u8, w: u32, h: u32, stride: usize) {
        let mut size: [c_int; 2] = [w as c_int, h as c_int];
        let format = b"rgb0\0"; // RGBX format (4 bytes per pixel)
        let mut stride_val = stride;
        let block: c_int = 0; // Don't block — frame pacing is handled by the callback flag

        let params = [
            libmpv2_sys::mpv_render_param {
                type_: 17, // MPV_RENDER_PARAM_SW_SIZE
                data: size.as_mut_ptr() as *mut c_void,
            },
            libmpv2_sys::mpv_render_param {
                type_: 18, // MPV_RENDER_PARAM_SW_FORMAT
                data: format.as_ptr() as *mut c_void,
            },
            libmpv2_sys::mpv_render_param {
                type_: 19, // MPV_RENDER_PARAM_SW_STRIDE
                data: &mut stride_val as *mut usize as *mut c_void,
            },
            libmpv2_sys::mpv_render_param {
                type_: 20, // MPV_RENDER_PARAM_SW_POINTER
                data: pixel_ptr as *mut c_void,
            },
            libmpv2_sys::mpv_render_param {
                type_: 12, // MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME
                data: &block as *const c_int as *mut c_void,
            },
            libmpv2_sys::mpv_render_param {
                type_: 0, // terminator
                data: ptr::null_mut(),
            },
        ];

        let ret = unsafe {
            libmpv2_sys::mpv_render_context_render(
                self.render_ctx,
                params.as_ptr() as *mut libmpv2_sys::mpv_render_param,
            )
        };

        if ret < 0 {
            tracing::warn!("mpv SW render failed: ret={ret}");
        }

        // Note: we do NOT call report_swap() here. With SW rendering, each
        // report_swap() advances mpv's internal timing, causing fast-forward.
        // Instead, we let the audio clock drive playback and just render the
        // frame at the current audio position via BLOCK=0.
    }
}

impl Drop for MpvPlayer {
    fn drop(&mut self) {
        // Free the render context before the Mpv handle is destroyed
        if !self.render_ctx.is_null() {
            unsafe {
                libmpv2_sys::mpv_render_context_free(self.render_ctx);
            }
            self.render_ctx = ptr::null_mut();
        }
    }
}

/// Create a new mpv-based video player with SW frame rendering.
///
/// Returns an error if libmpv is not installed or initialization fails.
/// Hardware decoding is enabled via `--hwdec=auto`.
pub fn create_mpv_player(src: &str) -> Result<VideoPlayer, Box<dyn std::error::Error>> {
    create_mpv_player_impl(src, false)
}

/// Create a video player that starts paused (no autoplay).
pub fn create_mpv_player_paused(src: &str) -> Result<VideoPlayer, Box<dyn std::error::Error>> {
    create_mpv_player_impl(src, true)
}

fn create_mpv_player_impl(
    src: &str,
    start_paused: bool,
) -> Result<VideoPlayer, Box<dyn std::error::Error>> {
    let mpv = libmpv2::Mpv::new()?;

    // Explicitly set vo=libmpv so mpv routes frames through the render API
    mpv.set_property("vo", "libmpv")?;
    // Enable hardware decoding (VA-API, NVDEC, etc.)
    mpv.set_property("hwdec", "auto")?;
    // Keep video open at end (don't destroy decode state)
    mpv.set_property("keep-open", "yes")?;
    // Enable video and audio tracks
    mpv.set_property("vid", "auto")?;
    mpv.set_property("aid", "auto")?;
    // Start paused if requested (must be set before loadfile)
    if start_paused {
        mpv.set_property("pause", true)?;
    }
    // Don't set video-sync — we render on demand via the update callback flag.
    // Setting video-sync modes can cause blocking near end-of-file when mpv
    // tries to sync to a clock we don't control.

    // Create SW render context
    let render_ctx = {
        let raw_handle = mpv.ctx.as_ptr();

        let mut ctx: *mut libmpv2_sys::mpv_render_context = ptr::null_mut();
        let api_type = libmpv2_sys::MPV_RENDER_API_TYPE_SW;
        let create_params = [
            libmpv2_sys::mpv_render_param {
                type_: 1, // MPV_RENDER_PARAM_API_TYPE
                data: api_type.as_ptr() as *mut c_void,
            },
            libmpv2_sys::mpv_render_param {
                type_: 0, // terminator
                data: ptr::null_mut(),
            },
        ];

        let ret = unsafe {
            libmpv2_sys::mpv_render_context_create(
                &mut ctx,
                raw_handle,
                create_params.as_ptr() as *mut libmpv2_sys::mpv_render_param,
            )
        };

        if ret < 0 {
            return Err(format!(
                "Failed to create mpv SW render context: {}",
                libmpv2_sys::mpv_error_str(ret)
            )
            .into());
        }

        ctx
    };

    // Log actual VO in use
    if let Ok(vo) = mpv.get_property::<String>("vo") {
        tracing::info!("rinch-video: vo={vo}");
    }

    // Set up render update callback. mpv calls this from an internal thread
    // when a new frame is available, setting the needs_render flag.
    let needs_render = Arc::new(AtomicBool::new(false));
    {
        let flag = Arc::clone(&needs_render);
        let flag_ptr = Arc::into_raw(flag);
        unsafe {
            libmpv2_sys::mpv_render_context_set_update_callback(
                render_ctx,
                Some(render_update_callback),
                flag_ptr as *mut c_void,
            );
        }
        // Reconstruct Arc so it's not leaked; the allocation stays alive via needs_render
        unsafe { Arc::from_raw(flag_ptr) };
    }

    // Observe properties for reactive updates
    mpv.observe_property("time-pos", libmpv2::Format::Double, PROP_TIME_POS)?;
    mpv.observe_property("duration", libmpv2::Format::Double, PROP_DURATION)?;
    mpv.observe_property("pause", libmpv2::Format::Flag, PROP_PAUSE)?;
    mpv.observe_property("volume", libmpv2::Format::Double, PROP_VOLUME)?;
    mpv.observe_property("mute", libmpv2::Format::Flag, PROP_MUTE)?;
    // Observe video dimensions for SW render target sizing
    mpv.observe_property("dwidth", libmpv2::Format::Int64, PROP_DWIDTH)?;
    mpv.observe_property("dheight", libmpv2::Format::Int64, PROP_DHEIGHT)?;
    // Observe eof-reached to detect end-of-file with keep-open=yes
    // (EndFile event is NOT sent when keep-open is enabled)
    mpv.observe_property("eof-reached", libmpv2::Format::Flag, PROP_EOF_REACHED)?;
    // Observe cache/buffering properties for network streams
    mpv.observe_property(
        "demuxer-cache-duration",
        libmpv2::Format::Double,
        PROP_CACHE_DURATION,
    )?;
    mpv.observe_property(
        "paused-for-cache",
        libmpv2::Format::Flag,
        PROP_PAUSED_FOR_CACHE,
    )?;

    let mpv = Arc::new(Mutex::new(mpv));
    let video_width = Arc::new(Mutex::new(0u32));
    let video_height = Arc::new(Mutex::new(0u32));

    let playing = Signal::new(false);
    let position = Signal::new(0.0);
    let duration = Signal::new(0.0);
    let volume = Signal::new(1.0);
    let muted = Signal::new(false);
    let buffered = Signal::new(0.0);
    let state = Signal::new(PlaybackState::Loading);

    let signals = PlayerSignals {
        playing,
        position,
        duration,
        volume,
        muted,
        buffered,
        state,
    };

    let (update_tx, update_rx) = mpsc::channel();
    let running = Arc::new(Mutex::new(true));

    // Spawn event polling thread
    {
        let mpv = mpv.clone();
        let running = running.clone();
        let tx = update_tx;

        std::thread::Builder::new()
            .name("rinch-video-mpv-events".to_string())
            .spawn(move || {
                mpv_event_loop(mpv, running, tx);
            })
            .expect("failed to spawn mpv event thread");
    }

    let backend = MpvPlayer {
        mpv: mpv.clone(),
        render_buffer: RefCell::new(Vec::new()),
        render_ctx,
        video_width,
        video_height,
        update_rx,
        signals: signals.clone(),
        running,
        needs_render,
        finished: Cell::new(false),
        frame_sink: RefCell::new(None),
    };

    // Load source if provided
    if !src.is_empty() {
        backend.set_source(src);
    }

    let id = crate::player::NEXT_PLAYER_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let player = VideoPlayer {
        id,
        inner: std::rc::Rc::new(RefCell::new(Box::new(backend))),
        playing: signals.playing,
        position: signals.position,
        duration: signals.duration,
        volume: signals.volume,
        muted: signals.muted,
        buffered: signals.buffered,
        state: signals.state,
    };

    if start_paused {
        // Don't register — play() will register when user starts playback.
        player.playing.set(false);
        player.state.set(crate::PlaybackState::Paused);
    } else {
        // Register as active so the event loop polls for updates.
        // mpv auto-plays on loadfile, so we need polling right away.
        crate::register_active_player(player.clone());
    }

    Ok(player)
}

/// Background thread: poll mpv events and send updates to main thread.
fn mpv_event_loop(
    mpv: Arc<Mutex<libmpv2::Mpv>>,
    running: Arc<Mutex<bool>>,
    tx: mpsc::Sender<MpvUpdate>,
) {
    use libmpv2::events::{Event, PropertyData};

    loop {
        // Check if we should stop
        if let Ok(r) = running.lock()
            && !*r
        {
            break;
        }

        // Lock mpv briefly with non-blocking event poll.
        //
        // CRITICAL: We use wait_event(0.0) (non-blocking) instead of wait_event(0.1)
        // to minimize lock hold time. With a blocking wait, the event thread holds the
        // mpv mutex for up to 100ms, preventing the main thread from calling play(),
        // pause(), seek(), or render — causing UI freezes/deadlocks.
        //
        // We drain all pending events in a tight loop, then sleep after releasing the lock.
        let mut got_event = true;
        while got_event {
            got_event = false;

            let should_break = {
                let Ok(mut mpv) = mpv.lock() else {
                    return; // Mutex poisoned
                };

                let mut brk = false;
                match mpv.wait_event(0.0) {
                    Some(Ok(event)) => {
                        got_event = true;
                        match event {
                            Event::PropertyChange {
                                name,
                                change,
                                reply_userdata: _,
                            } => {
                                let update = match (name, &change) {
                                    ("time-pos", PropertyData::Double(v)) => {
                                        Some(MpvUpdate::Position(*v))
                                    }
                                    ("duration", PropertyData::Double(v)) => {
                                        Some(MpvUpdate::Duration(*v))
                                    }
                                    ("pause", PropertyData::Flag(v)) => Some(MpvUpdate::Paused(*v)),
                                    ("volume", PropertyData::Double(v)) => {
                                        Some(MpvUpdate::Volume((*v as f32) / 100.0))
                                    }
                                    ("mute", PropertyData::Flag(v)) => Some(MpvUpdate::Muted(*v)),
                                    ("dwidth", PropertyData::Int64(v)) => {
                                        Some(MpvUpdate::VideoWidth(*v))
                                    }
                                    ("dheight", PropertyData::Int64(v)) => {
                                        Some(MpvUpdate::VideoHeight(*v))
                                    }
                                    ("eof-reached", PropertyData::Flag(v)) => {
                                        Some(MpvUpdate::EofReached(*v))
                                    }
                                    ("demuxer-cache-duration", PropertyData::Double(v)) => {
                                        Some(MpvUpdate::CacheDuration(*v))
                                    }
                                    ("paused-for-cache", PropertyData::Flag(v)) => {
                                        Some(MpvUpdate::PausedForCache(*v))
                                    }
                                    _ => None,
                                };
                                if let Some(u) = update {
                                    let _ = tx.send(u);
                                }
                            }
                            Event::FileLoaded => {
                                // Sync-query dimensions while we hold the lock
                                if let Ok(w) = mpv.get_property::<i64>("width") {
                                    if w > 0 {
                                        let _ = tx.send(MpvUpdate::VideoWidth(w));
                                    }
                                }
                                if let Ok(h) = mpv.get_property::<i64>("height") {
                                    if h > 0 {
                                        let _ = tx.send(MpvUpdate::VideoHeight(h));
                                    }
                                }
                                let _ = tx.send(MpvUpdate::FileLoaded);
                            }
                            Event::EndFile(_reason) => {
                                let _ = tx.send(MpvUpdate::EndFile);
                            }
                            Event::Shutdown => {
                                brk = true;
                            }
                            _ => {}
                        }
                    }
                    Some(Err(e)) => {
                        let _ = tx.send(MpvUpdate::Error(format!("{e:?}")));
                    }
                    None => {
                        // No pending events
                    }
                }
                brk
            };
            // Lock released here ^

            if should_break {
                return;
            }
        }

        // Sleep after draining all events — keeps CPU usage low while ensuring
        // the main thread can always acquire the mpv lock promptly.
        std::thread::sleep(Duration::from_millis(8));
    }
}
