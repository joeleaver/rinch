//! VideoViewport component.
//!
//! On desktop, renders as a transparent div (like `GameViewport`) — the
//! compositor draws the video frame underneath. On WASM, renders as a
//! real `<video>` element.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

#[cfg(not(target_arch = "wasm32"))]
use crate::PlaybackState;
use crate::VideoPlayer;

/// A viewport region where video frames are rendered.
///
/// On desktop, this is a transparent placeholder (similar to `GameViewport`).
/// The compositing pass reads the viewport rect and blits the decoded video
/// frame into that region, underneath the Vello UI layer.
///
/// On WASM, this creates a real `<video>` DOM element.
///
/// # Example
///
/// ```ignore
/// use rinch::prelude::*;
/// use rinch_video::{VideoViewport, use_video_player};
///
/// #[component]
/// fn app() -> NodeHandle {
///     let player = use_video_player("video.mp4");
///     rsx! {
///         VideoViewport { player: player.clone(), style: "flex: 1;" }
///     }
/// }
/// ```
#[derive(Debug)]
pub struct VideoViewport {
    /// The video player handle.
    pub player: VideoPlayer,
}

impl Default for VideoViewport {
    fn default() -> Self {
        // Create a dummy no-op player for Default
        Self {
            player: VideoPlayer::new(Box::new(NoopBackend)),
        }
    }
}

impl Component for VideoViewport {
    fn render(&self, scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        video_viewport_render(scope, &self.player)
    }
}

/// Desktop: transparent div with data-video-viewport attribute.
/// The compositor queries this rect to position the video frame.
///
/// The hole is only transparent once there is a frame to show through it.
/// Until then — before the first decode, or after a `PlaybackState::Error` —
/// the div declares `data-viewport-ready="false"`, which stops paint cutting
/// the hole, and paints its own black background as the placeholder. Without
/// that, an errored video is see-through to the desktop on a transparent
/// window (issue #186).
#[cfg(not(target_arch = "wasm32"))]
fn video_viewport_render(scope: &mut RenderScope, player: &VideoPlayer) -> NodeHandle {
    let __scope = scope;
    let el = rinch_macros::rsx! {
        div {
            class: "rinch-video-viewport",
        }
    };
    el.set_attribute("data-viewport", &player.viewport_id());

    {
        let p = player.clone();
        let node = el.clone();
        __scope.create_effect(move || {
            // Ready == the backend has delivered at least one real frame and
            // is not in an error state. Not ready => nothing will fill the
            // hole, so we own these pixels and paint them.
            let ready = p.has_frame.get() && !matches!(p.state.get(), PlaybackState::Error(_));
            node.set_attribute("data-viewport-ready", if ready { "true" } else { "false" });
            // `set_attribute("style", ...)` REPLACES the attribute, so every
            // declaration is re-emitted. `pointer-events: none` is load-bearing:
            // it is the author declaration that beats the UA
            // `[data-viewport] { pointer-events: auto }` rule from #209, and
            // rinch-video deliberately opts out of hittability.
            node.set_attribute(
                "style",
                if ready {
                    "pointer-events: none; background: transparent; width: 100%; height: 100%;"
                } else {
                    "pointer-events: none; background: #000; width: 100%; height: 100%;"
                },
            );
        });
    }

    el
}

/// WASM: create a real <video> element (browser handles rendering).
/// Tagged with `data-video-player` so `WebVideoPlayer` can find it.
#[cfg(target_arch = "wasm32")]
fn video_viewport_render(scope: &mut RenderScope, player: &VideoPlayer) -> NodeHandle {
    let __scope = scope;
    let el = rinch_macros::rsx! {
        video {
            class: "rinch-video-viewport",
            style: "width: 100%; height: 100%; object-fit: contain; background: black;",
            playsinline: "true",
        }
    };
    el.set_attribute("data-video-player", &player.viewport_id());
    el
}

/// No-op backend used only for the Default impl.
#[derive(Debug)]
pub(crate) struct NoopBackend;

impl crate::VideoPlayerBackend for NoopBackend {
    fn play(&self) {}
    fn pause(&self) {}
    fn seek(&self, _seconds: f64) {}
    fn set_volume(&self, _vol: f32) {}
    fn set_muted(&self, _muted: bool) {}
    fn set_source(&self, _src: &str) {}
    fn cleanup(&self) {}
}
