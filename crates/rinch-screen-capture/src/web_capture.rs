//! WASM screen capture via `getDisplayMedia()`.
//!
//! The browser handles the picker dialog and capture. We just read frames
//! from the MediaStream using a hidden <video> element + <canvas> readback.

use rinch_av::VideoTrack;

use crate::{CaptureConfig, CaptureError};

/// Handle to a browser screen capture session.
pub(crate) struct WebCapture {
    // TODO: Hold references to MediaStream, video element, animation frame loop.
    // For now this is a stub — WASM screen capture requires async and is
    // better implemented with wasm-bindgen-futures + requestAnimationFrame.
    _phantom: (),
}

impl WebCapture {
    pub fn start(_track: &VideoTrack, _config: &CaptureConfig) -> Result<Self, CaptureError> {
        // getDisplayMedia() is async and requires user gesture context.
        // Full implementation would:
        // 1. Call navigator.mediaDevices.getDisplayMedia()
        // 2. Attach resulting MediaStream to a hidden <video> element
        // 3. Use requestAnimationFrame to draw video to <canvas>
        // 4. Read pixels from canvas via getImageData()
        // 5. Push RGBA frames to the VideoTrack
        //
        // For WebRTC on WASM, the browser's MediaStream can be passed
        // directly to RTCPeerConnection.addTrack() without frame readback.
        Err(CaptureError::Platform(
            "WASM screen capture not yet implemented — use browser getDisplayMedia() API directly"
                .into(),
        ))
    }
}

impl Drop for WebCapture {
    fn drop(&mut self) {}
}
