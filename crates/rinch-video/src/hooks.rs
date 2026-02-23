//! Video player creation.
//!
//! Provides [`use_video_player`] for creating and managing a video player
//! instance within a component.

use crate::VideoPlayer;

/// Create a video player for the given source.
///
/// The player is created directly since component functions run once.
/// No hook registry needed — just create the player and return it.
///
/// # Example
///
/// ```ignore
/// use rinch::prelude::*;
/// use rinch_video::use_video_player;
///
/// #[component]
/// fn app() -> NodeHandle {
///     let player = use_video_player("my_video.mp4");
///     rsx! {
///         div {
///             p { {|| format!("Position: {:.1}s", player.position.get())} }
///             button { onclick: { let p = player.clone(); move || p.toggle() }, "Play/Pause" }
///         }
///     }
/// }
/// ```
pub fn use_video_player(src: &str) -> VideoPlayer {
    use_video_player_impl(src, false)
}

/// Create a video player that starts paused (no autoplay).
///
/// The user must call `player.play()` to begin playback.
/// Useful when multiple players exist and only one should play at a time.
pub fn use_video_player_paused(src: &str) -> VideoPlayer {
    use_video_player_impl(src, true)
}

fn use_video_player_impl(src: &str, start_paused: bool) -> VideoPlayer {
    // Component functions run once, so just create the player directly.
    create_platform_player(src, start_paused)
}

/// Create a platform-appropriate video player.
#[cfg(not(target_arch = "wasm32"))]
fn create_platform_player(src: &str, start_paused: bool) -> VideoPlayer {
    let result = if start_paused {
        crate::native::create_mpv_player_paused(src)
    } else {
        crate::native::create_mpv_player(src)
    };
    match result {
        Ok(player) => player,
        Err(e) => {
            tracing::error!("Failed to create mpv player: {e}");
            // Return a player with error state
            let player = VideoPlayer::new(Box::new(crate::viewport::NoopBackend));
            player.state.set(crate::PlaybackState::Error(e.to_string()));
            player
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn create_platform_player(src: &str, _start_paused: bool) -> VideoPlayer {
    crate::web::create_web_player(src)
}
