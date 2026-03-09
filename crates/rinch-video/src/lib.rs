#![allow(
    clippy::collapsible_if,
    clippy::too_many_arguments,
    clippy::type_complexity,
    dead_code
)]
//! Video playback for rinch applications.
//!
//! This crate provides video playback capability for rinch using libmpv
//! on desktop and `<video>` elements on WASM.
//!
//! # Architecture
//!
//! - **Desktop**: libmpv handles decoding, A/V sync, and audio output.
//!   Decoded frames are delivered through a frame sink (registered via
//!   [`set_frame_sink_factory`]) which routes them to a `SurfaceWriter`
//!   for compositing via the RenderSurface pipeline.
//! - **WASM**: A real `<video>` DOM element is created; the browser handles
//!   everything.
//!
//! # Usage
//!
//! ```ignore
//! use rinch::prelude::*;
//! use rinch_video::{VideoViewport, VideoControls, use_video_player};
//!
//! #[component]
//! fn app() -> NodeHandle {
//!     let player = use_video_player("video.mp4");
//!     rsx! {
//!         div { style: "display: flex; flex-direction: column; height: 100%;",
//!             VideoViewport { player: player.clone(), style: "flex: 1;" }
//!             VideoControls { player: player }
//!         }
//!     }
//! }
//! ```

mod controls;
mod hooks;
mod player;
pub(crate) mod viewport;

#[cfg(not(target_arch = "wasm32"))]
pub mod native;

#[cfg(target_arch = "wasm32")]
mod web;

pub use controls::VideoControls;
pub use hooks::{use_video_player, use_video_player_paused};
pub use player::{PlaybackState, VideoPlayer, VideoPlayerBackend};
pub use viewport::VideoViewport;

use std::cell::{Cell, RefCell};
use std::sync::{Mutex, OnceLock};

/// Callback type for creating a frame sink for a video player.
/// Called with the viewport_id; returns a FrameSink that delivers frames
/// to the rendering pipeline (e.g., via RenderSurface).
type FrameSinkFactory = Box<dyn Fn(&str) -> player::FrameSink + Send>;

/// Global factory for creating frame sinks.
/// Set by the rinch shell on startup.
static FRAME_SINK_FACTORY: OnceLock<Mutex<FrameSinkFactory>> = OnceLock::new();

/// Register a factory that creates frame sinks for video players.
///
/// Called once by the rinch shell during initialization. The factory
/// receives a viewport_id and returns a `FrameSink` that delivers
/// decoded frames to the compositor (typically via `SurfaceWriter`).
pub fn set_frame_sink_factory(f: impl Fn(&str) -> player::FrameSink + Send + 'static) {
    let _ = FRAME_SINK_FACTORY.set(Mutex::new(Box::new(f)));
}

/// Create a frame sink for the given viewport, if a factory is registered.
pub(crate) fn create_frame_sink(viewport_id: &str) -> Option<player::FrameSink> {
    FRAME_SINK_FACTORY
        .get()
        .and_then(|f| f.lock().ok())
        .map(|f| f(viewport_id))
}

thread_local! {
    /// Count of video players that have content loaded (playing, paused, or ended).
    /// Incremented on set_source(), decremented on cleanup().
    /// Used by the renderer to keep the last frame visible when paused.
    static VIDEO_LOADED_COUNT: Cell<u32> = const { Cell::new(0) };

    /// Registry of active (playing) video players.
    /// Polled each frame to drain backend updates into signals.
    static ACTIVE_PLAYERS: RefCell<Vec<VideoPlayer>> = const { RefCell::new(Vec::new()) };
}

/// Check if any video player is currently active (playing).
///
/// Derived from the ACTIVE_PLAYERS list — true if any player is registered.
/// Called by the rinch event loop in `AboutToWait` to determine
/// whether to keep the render loop spinning.
pub fn is_video_active() -> bool {
    ACTIVE_PLAYERS.with(|list| !list.borrow().is_empty())
}

/// Check if any video is loaded (playing, paused, or ended).
///
/// Returns true as long as at least one video player has content loaded,
/// even if it's paused or finished. Used by the renderer to keep
/// the last frame visible.
pub fn is_video_loaded() -> bool {
    VIDEO_LOADED_COUNT.with(|c| c.get() > 0)
}

/// Increment the video-loaded counter. Called on set_source().
pub(crate) fn increment_video_loaded() {
    VIDEO_LOADED_COUNT.with(|c| c.set(c.get() + 1));
}

/// Decrement the video-loaded counter. Called on cleanup().
pub(crate) fn decrement_video_loaded() {
    VIDEO_LOADED_COUNT.with(|c| {
        let val = c.get();
        if val > 0 {
            c.set(val - 1);
        }
    });
}

/// Register a player as active (called on play).
pub(crate) fn register_active_player(player: VideoPlayer) {
    // Set up the frame sink if a factory is available and sink isn't already set.
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(sink) = create_frame_sink(&player.viewport_id()) {
            player.set_frame_sink(sink);
        }
    }

    ACTIVE_PLAYERS.with(|list| {
        let mut list = list.borrow_mut();
        // Avoid duplicates (compare by Rc pointer identity)
        let already_registered = list
            .iter()
            .any(|p| std::rc::Rc::ptr_eq(&p.inner, &player.inner));
        if !already_registered {
            list.push(player);
        }
    });
}

/// Unregister a player (called on pause/cleanup).
pub(crate) fn unregister_active_player(player: &VideoPlayer) {
    ACTIVE_PLAYERS.with(|list| {
        let mut list = list.borrow_mut();
        list.retain(|p| !std::rc::Rc::ptr_eq(&p.inner, &player.inner));
    });
}

/// Poll all active video players for backend updates.
///
/// Called from the rinch event loop in `AboutToWait` to drain
/// mpv property-change events into reactive Signals.
/// Also unregisters players that have ended (EOF).
pub fn poll_active_players() {
    // Clone the list so we don't hold a borrow during poll().
    // poll() updates signals (e.g. duration) which can trigger Effects
    // that call play() → register_active_player() → borrow_mut().
    let players: Vec<_> = ACTIVE_PLAYERS.with(|list| list.borrow().clone());
    for player in &players {
        player.poll();
    }
    // Remove players that have ended (EOF)
    ACTIVE_PLAYERS.with(|list| {
        let mut list = list.borrow_mut();
        list.retain(|p| p.state.get() != PlaybackState::Ended);
    });
}
