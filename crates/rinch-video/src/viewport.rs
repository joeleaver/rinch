//! VideoViewport component.
//!
//! On desktop, renders as a transparent div (like `GameViewport`) — the
//! compositor draws the video frame underneath. On WASM, renders as a
//! real `<video>` element.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

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
        video_viewport_render(scope, &self.player.viewport_id())
    }
}

/// Desktop: transparent div with data-video-viewport attribute.
/// The compositor queries this rect to position the video frame.
#[cfg(not(target_arch = "wasm32"))]
fn video_viewport_render(scope: &mut RenderScope, viewport_name: &str) -> NodeHandle {
    let div = scope.create_element("div");
    div.set_attribute("class", "rinch-video-viewport");
    div.set_attribute("data-viewport", viewport_name);
    div.set_attribute(
        "style",
        "pointer-events: none; background: transparent; width: 100%; height: 100%;",
    );
    div
}

/// WASM: create a real <video> element (browser handles rendering).
#[cfg(target_arch = "wasm32")]
fn video_viewport_render(scope: &mut RenderScope) -> NodeHandle {
    let video = scope.create_element("video");
    video.set_attribute("class", "rinch-video-viewport");
    video.set_attribute(
        "style",
        "width: 100%; height: 100%; object-fit: contain; background: black;",
    );
    video.set_attribute("playsinline", "true");
    video
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
