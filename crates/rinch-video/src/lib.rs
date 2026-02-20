//! Video playback for rinch applications.
//!
//! This crate provides video playback capability for rinch using libmpv
//! on desktop and `<video>` elements on WASM.
//!
//! # Architecture
//!
//! - **Desktop**: libmpv handles decoding, A/V sync, and audio output.
//!   Decoded frames are uploaded as wgpu textures and composited underneath
//!   the Vello UI layer.
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

mod player;
pub(crate) mod viewport;
mod controls;
mod hooks;

#[cfg(not(target_arch = "wasm32"))]
pub mod native;

#[cfg(target_arch = "wasm32")]
mod web;

pub use player::{PlaybackState, VideoPlayer, VideoPlayerBackend};
pub use viewport::VideoViewport;
pub use controls::VideoControls;
pub use hooks::{use_video_player, use_video_player_paused};

use std::cell::{Cell, RefCell};

thread_local! {
    /// Count of video players that have content loaded (playing, paused, or ended).
    /// Incremented on set_source(), decremented on cleanup().
    /// Used by the renderer to keep the last frame visible when paused.
    static VIDEO_LOADED_COUNT: Cell<u32> = const { Cell::new(0) };

    /// Registry of active (playing) video players.
    /// Polled each frame to drain backend updates into signals.
    static ACTIVE_PLAYERS: RefCell<Vec<VideoPlayer>> = const { RefCell::new(Vec::new()) };

    /// Persistent frame cache: last decoded frame per player viewport.
    /// Ensures all players' frames are composited every cycle, not just
    /// the ones that decoded a new frame this particular cycle.
    #[cfg(not(target_arch = "wasm32"))]
    static FRAME_CACHE: RefCell<Vec<(String, Vec<u8>, u32, u32)>> = const { RefCell::new(Vec::new()) };
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
    ACTIVE_PLAYERS.with(|list| {
        let mut list = list.borrow_mut();
        // Avoid duplicates (compare by Rc pointer identity)
        let already_registered = list.iter().any(|p| std::rc::Rc::ptr_eq(&p.inner, &player.inner));
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
    ACTIVE_PLAYERS.with(|list| {
        // Poll all players first
        {
            let list = list.borrow();
            for player in list.iter() {
                player.poll();
            }
        }
        // Remove players that have ended (EOF)
        let mut list = list.borrow_mut();
        list.retain(|p| p.state.get() != PlaybackState::Ended);
    });
}

/// Update the persistent frame cache with any new frames from active players,
/// then return ALL cached frames for compositing.
///
/// This ensures every player's last frame is always composited, even if only
/// some players decoded a new frame this cycle.
#[cfg(not(target_arch = "wasm32"))]
pub fn collect_video_frames() -> Vec<(String, Vec<u8>, u32, u32)> {
    ACTIVE_PLAYERS.with(|list| {
        let list = list.borrow();
        FRAME_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            // Update cache with any new frames
            for player in list.iter() {
                if player.has_new_frame() {
                    if let Some((pixels, w, h)) = player.take_frame() {
                        let vid = player.viewport_id();
                        // Update or insert this player's cached frame
                        if let Some(entry) = cache.iter_mut().find(|(name, _, _, _)| *name == vid) {
                            *entry = (vid, pixels, w, h);
                        } else {
                            cache.push((vid, pixels, w, h));
                        }
                    }
                }
            }
            // Remove cached frames for players no longer active
            let active_ids: Vec<String> = list.iter().map(|p| p.viewport_id()).collect();
            cache.retain(|(name, _, _, _)| active_ids.contains(name));
            // Return clones of all cached frames
            cache.clone()
        })
    })
}

/// Clear the frame cache for a specific viewport (called on cleanup).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn clear_frame_cache(viewport_id: &str) {
    FRAME_CACHE.with(|cache| {
        cache.borrow_mut().retain(|(name, _, _, _)| name != viewport_id);
    });
}
