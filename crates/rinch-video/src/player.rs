//! Platform-agnostic video player handle.
//!
//! [`VideoPlayer`] wraps a platform-specific backend (libmpv on desktop,
//! HTMLVideoElement on WASM) and exposes reactive [`Signal`]s for UI binding.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use rinch_core::Signal;

/// A thread-safe callback for receiving decoded video frames.
/// Called with (rgba_pixels, width, height).
pub type FrameSink = Arc<dyn Fn(&[u8], u32, u32) + Send + Sync>;

/// Global counter for unique player IDs.
pub(crate) static NEXT_PLAYER_ID: AtomicU32 = AtomicU32::new(0);

/// Playback state of the video player.
#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackState {
    /// No video loaded.
    Idle,
    /// Video is loading/buffering.
    Loading,
    /// Video is actively playing.
    Playing,
    /// Video is paused.
    Paused,
    /// Playback reached the end.
    Ended,
    /// An error occurred.
    Error(String),
}

/// Platform-specific backend trait.
///
/// Implemented by `MpvPlayer` (desktop) and `WebVideoPlayer` (WASM).
pub trait VideoPlayerBackend {
    fn play(&self);
    fn pause(&self);
    fn seek(&self, seconds: f64);
    fn set_volume(&self, vol: f32);
    fn set_muted(&self, muted: bool);
    fn set_source(&self, src: &str);
    fn cleanup(&self);

    /// Poll for updates from the backend (e.g., mpv event thread).
    ///
    /// Called from the main thread event loop to drain pending updates
    /// and apply them to signals. Default is a no-op.
    fn poll_updates(&self) {}

    /// Set a frame sink callback that receives decoded RGBA frames.
    ///
    /// When set, `render_sw_frame()` delivers frames through this sink
    /// instead of (or in addition to) storing in the internal frame buffer.
    /// The sink is `Send + Sync` and can be called from any thread.
    #[cfg(not(target_arch = "wasm32"))]
    fn set_frame_sink(&self, _sink: FrameSink) {}

    /// Desktop only: check if a new frame is available since last call.
    #[cfg(not(target_arch = "wasm32"))]
    fn has_new_frame(&self) -> bool {
        false
    }

    /// Desktop only: get the current frame pixels (RGBA8, width, height).
    /// Returns None if no frame is available.
    #[cfg(not(target_arch = "wasm32"))]
    fn take_frame(&self) -> Option<(Vec<u8>, u32, u32)> {
        None
    }
}

/// Handle to a video player instance.
///
/// Wraps a platform-specific backend and exposes reactive signals
/// that can be read by UI components (e.g., [`VideoControls`]).
///
/// `VideoPlayer` is cheaply cloneable (all data is behind `Rc`).
#[derive(Clone)]
pub struct VideoPlayer {
    /// Unique player ID, used for viewport naming.
    pub(crate) id: u32,
    pub(crate) inner: Rc<RefCell<Box<dyn VideoPlayerBackend>>>,
    /// Whether the video is currently playing.
    pub playing: Signal<bool>,
    /// Current playback position in seconds.
    pub position: Signal<f64>,
    /// Total duration in seconds.
    pub duration: Signal<f64>,
    /// Volume level (0.0 to 1.0).
    pub volume: Signal<f32>,
    /// Whether audio is muted.
    pub muted: Signal<bool>,
    /// Seconds buffered ahead of current position.
    pub buffered: Signal<f64>,
    /// Current playback state.
    pub state: Signal<PlaybackState>,
}

impl VideoPlayer {
    /// Create a new video player with the given backend.
    pub fn new(backend: Box<dyn VideoPlayerBackend>) -> Self {
        let id = NEXT_PLAYER_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            id,
            inner: Rc::new(RefCell::new(backend)),
            playing: Signal::new(false),
            position: Signal::new(0.0),
            duration: Signal::new(0.0),
            volume: Signal::new(1.0),
            muted: Signal::new(false),
            buffered: Signal::new(0.0),
            state: Signal::new(PlaybackState::Idle),
        }
    }

    /// Get the unique viewport name for this player.
    ///
    /// Used as the `data-viewport` attribute value on the `VideoViewport` div,
    /// allowing multiple viewports to coexist.
    pub fn viewport_id(&self) -> String {
        format!("video-{}", self.id)
    }

    /// Start or resume playback.
    ///
    /// If the video has ended, seeks to the beginning first.
    pub fn play(&self) {
        if self.state.get() == PlaybackState::Ended {
            self.inner.borrow().seek(0.0);
            self.position.set(0.0);
        }
        self.inner.borrow().play();
        self.playing.set(true);
        self.state.set(PlaybackState::Playing);
        crate::register_active_player(self.clone());
    }

    /// Pause playback.
    pub fn pause(&self) {
        self.inner.borrow().pause();
        self.playing.set(false);
        self.state.set(PlaybackState::Paused);
        crate::unregister_active_player(self);
    }

    /// Toggle between play and pause.
    pub fn toggle(&self) {
        if self.playing.get() {
            self.pause();
        } else {
            self.play();
        }
    }

    /// Seek to a position in seconds.
    pub fn seek(&self, seconds: f64) {
        self.inner.borrow().seek(seconds);
        self.position.set(seconds);
    }

    /// Set the volume (0.0 to 1.0).
    pub fn set_volume(&self, vol: f32) {
        let vol = vol.clamp(0.0, 1.0);
        self.inner.borrow().set_volume(vol);
        self.volume.set(vol);
    }

    /// Set whether audio is muted.
    pub fn set_muted(&self, muted: bool) {
        self.inner.borrow().set_muted(muted);
        self.muted.set(muted);
    }

    /// Load a new video source (file path or URL).
    pub fn set_source(&self, src: &str) {
        self.state.set(PlaybackState::Loading);
        self.position.set(0.0);
        self.duration.set(0.0);
        self.inner.borrow().set_source(src);
        crate::increment_video_loaded();
    }

    /// Poll the backend for updates (position, duration, state, etc.).
    ///
    /// Called automatically by `poll_active_players()` in the event loop.
    pub fn poll(&self) {
        self.inner.borrow().poll_updates();
    }

    /// Clean up resources. Called automatically on drop via the hook.
    pub fn cleanup(&self) {
        self.inner.borrow().cleanup();
        crate::unregister_active_player(self);
        crate::decrement_video_loaded();
    }

    /// Set a frame sink that receives decoded RGBA frames.
    ///
    /// When set, frames are delivered through this callback instead of
    /// being stored internally. Used by rinch to route video frames
    /// through the RenderSurface compositing pipeline.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_frame_sink(&self, sink: FrameSink) {
        self.inner.borrow().set_frame_sink(sink);
    }

    /// Check if a new decoded frame is available (desktop only).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn has_new_frame(&self) -> bool {
        self.inner.borrow().has_new_frame()
    }

    /// Take the latest decoded frame pixels (desktop only).
    /// Returns (rgba_pixels, width, height) or None.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn take_frame(&self) -> Option<(Vec<u8>, u32, u32)> {
        self.inner.borrow().take_frame()
    }
}

impl std::fmt::Debug for VideoPlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoPlayer")
            .field("playing", &self.playing.get())
            .field("position", &self.position.get())
            .field("duration", &self.duration.get())
            .field("state", &self.state.get())
            .finish()
    }
}
