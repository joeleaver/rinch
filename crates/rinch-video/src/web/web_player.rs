//! HTMLVideoElement-based video player for WASM.
//!
//! On WASM, the browser handles all video decoding, rendering, and audio.
//! We just wrap an `HTMLVideoElement` and wire DOM events to Signals.

use rinch_core::Signal;

use crate::player::{PlaybackState, VideoPlayer, VideoPlayerBackend};

/// Web-based video player using HTMLVideoElement.
pub struct WebVideoPlayer {
    // Will hold web_sys::HtmlVideoElement when running in browser.
    // For now, this is a placeholder that compiles on all targets.
    _state_signal: Signal<PlaybackState>,
}

impl std::fmt::Debug for WebVideoPlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebVideoPlayer").finish_non_exhaustive()
    }
}

impl VideoPlayerBackend for WebVideoPlayer {
    fn play(&self) {
        // TODO: self.element.play()
    }

    fn pause(&self) {
        // TODO: self.element.pause()
    }

    fn seek(&self, _seconds: f64) {
        // TODO: self.element.set_current_time(seconds)
    }

    fn set_volume(&self, _vol: f32) {
        // TODO: self.element.set_volume(vol as f64)
    }

    fn set_muted(&self, _muted: bool) {
        // TODO: self.element.set_muted(muted)
    }

    fn set_source(&self, _src: &str) {
        // TODO: self.element.set_src(src)
    }

    fn cleanup(&self) {
        // Browser handles cleanup
    }
}

/// Create a web-based video player.
pub fn create_web_player(src: &str) -> VideoPlayer {
    let state_signal = Signal::new(PlaybackState::Loading);

    let backend = WebVideoPlayer {
        _state_signal: state_signal,
    };

    let player = VideoPlayer::new(Box::new(backend));
    if !src.is_empty() {
        player.set_source(src);
    }
    player
}
