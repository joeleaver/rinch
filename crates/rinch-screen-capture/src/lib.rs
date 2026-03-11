//! Cross-platform screen capture for rinch.
//!
//! Captures screen/window content and delivers frames via [`rinch_av::VideoTrack`].
//!
//! # Platform Support
//!
//! | Platform | Backend | Notes |
//! |----------|---------|-------|
//! | Linux (Wayland/X11) | PipeWire + XDG Portal | User selects source via system dialog |
//! | WASM | `getDisplayMedia()` | Browser-native screen sharing |
//!
//! # Example
//!
//! ```ignore
//! use rinch_screen_capture::{ScreenCapture, CaptureConfig};
//!
//! // Start capture — shows platform screen picker dialog
//! let capture = ScreenCapture::start(CaptureConfig::default())?;
//!
//! // Get the VideoTrack (same type as camera — works with WebRTC, RenderSurface, etc.)
//! let track = capture.video_track();
//!
//! // Read frames
//! if let Some(frame) = track.latest_frame() {
//!     println!("{}x{}", frame.width, frame.height);
//! }
//!
//! // Stop when done
//! capture.stop();
//! ```

#[cfg(target_os = "linux")]
mod pipewire_capture;

#[cfg(target_arch = "wasm32")]
mod web_capture;

use std::sync::{Arc, Mutex};

pub use rinch_av::VideoTrack;
use rinch_av::camera::VideoFrame;

/// Thread-safe frame delivery channel shared between capture thread and VideoTrack.
type FrameSink = Arc<Mutex<Option<VideoFrame>>>;

/// Configuration for screen capture.
#[derive(Debug, Clone)]
pub struct CaptureConfig {
    /// Maximum frames per second (0 = no limit). Default: 30.
    pub max_fps: u32,
    /// Whether to show the cursor in captured frames. Default: true.
    pub show_cursor: bool,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            max_fps: 30,
            show_cursor: true,
        }
    }
}

/// Error type for screen capture operations.
#[derive(Debug)]
pub enum CaptureError {
    /// The user cancelled the screen picker dialog.
    Cancelled,
    /// Platform-specific error.
    Platform(String),
    /// Screen capture is not supported on this platform.
    Unsupported,
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaptureError::Cancelled => write!(f, "screen capture cancelled by user"),
            CaptureError::Platform(msg) => write!(f, "screen capture error: {msg}"),
            CaptureError::Unsupported => write!(f, "screen capture not supported on this platform"),
        }
    }
}

impl std::error::Error for CaptureError {}

/// A running screen capture session.
///
/// Frames are delivered to the internal [`VideoTrack`] which can be read
/// via `video_track()`. Drop or call `stop()` to end capture.
pub struct ScreenCapture {
    track: VideoTrack,
    // Kept alive for Drop — stops capture on drop.
    #[cfg(target_os = "linux")]
    _inner: pipewire_capture::PipeWireCapture,
    #[cfg(target_arch = "wasm32")]
    _inner: web_capture::WebCapture,
}

impl ScreenCapture {
    /// Start screen capture.
    ///
    /// Shows a platform-native screen/window picker dialog. Blocks until the
    /// user selects a source (or cancels). Returns a `ScreenCapture` that
    /// delivers frames to an internal `VideoTrack`.
    ///
    /// # Errors
    ///
    /// - `CaptureError::Cancelled` if the user dismisses the picker
    /// - `CaptureError::Platform` for system-level errors
    /// - `CaptureError::Unsupported` on unsupported platforms
    pub fn start(config: CaptureConfig) -> Result<Self, CaptureError> {
        let track = VideoTrack::new("screen-capture");

        #[cfg(target_os = "linux")]
        {
            // Share the frame storage between capture thread and VideoTrack.
            // The capture thread writes frames, VideoTrack reads them.
            let sink = track.frame_sink();
            let inner = pipewire_capture::PipeWireCapture::start(sink, &config)?;
            Ok(Self {
                track,
                _inner: inner,
            })
        }

        #[cfg(target_arch = "wasm32")]
        {
            let inner = web_capture::WebCapture::start(&track, &config)?;
            Ok(Self {
                track,
                _inner: inner,
            })
        }

        #[cfg(not(any(target_os = "linux", target_arch = "wasm32")))]
        {
            let _ = (config, track);
            Err(CaptureError::Unsupported)
        }
    }

    /// Get the `VideoTrack` receiving captured frames.
    ///
    /// This is the same `VideoTrack` type used by cameras — it works with
    /// `RenderSurface`, WebRTC `PeerConnection`, or any other consumer.
    pub fn video_track(&self) -> &VideoTrack {
        &self.track
    }

    /// Stop the capture.
    pub fn stop(self) {
        drop(self);
    }
}
