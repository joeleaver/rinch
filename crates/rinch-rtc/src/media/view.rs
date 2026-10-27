//! `MediaView`: show a local or remote video track.
//!
//! - Web: a `<video autoplay playsinline muted>` whose `srcObject` holds
//!   the track; the browser decodes and paints it.
//! - Desktop: a div carrying `data-viewport`, fed through the named video
//!   sink (design record rtc-media.md 1.4, M6).
//!
//! Both fit the frame with `contain`.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

use super::{LocalTrack, RemoteTrack};

/// A track [`MediaView`] can show.
#[derive(Clone, Debug, Default)]
pub enum MediaTrack {
    /// Nothing (the view shows black).
    #[default]
    None,
    Local(LocalTrack),
    Remote(RemoteTrack),
}

impl From<LocalTrack> for MediaTrack {
    fn from(t: LocalTrack) -> Self {
        MediaTrack::Local(t)
    }
}

impl From<&LocalTrack> for MediaTrack {
    fn from(t: &LocalTrack) -> Self {
        MediaTrack::Local(t.clone())
    }
}

impl From<RemoteTrack> for MediaTrack {
    fn from(t: RemoteTrack) -> Self {
        MediaTrack::Remote(t)
    }
}

impl From<&RemoteTrack> for MediaTrack {
    fn from(t: &RemoteTrack) -> Self {
        MediaTrack::Remote(t.clone())
    }
}

/// Shows a video track.
///
/// ```ignore
/// rsx! { MediaView { track: remote_video.clone(), style: "width: 640px; height: 360px;" } }
/// rsx! { MediaView { track: camera.clone(), mirrored: true } } // a self-view
/// ```
///
/// The track is fixed for the component's life; key the component on the
/// track to show another one.
#[derive(Debug, Default)]
pub struct MediaView {
    pub track: MediaTrack,
    /// Flip horizontally (a self-view reads as a mirror).
    pub mirrored: bool,
    /// Inline style for the element (its size, for one).
    pub style: String,
    /// Extra classes.
    pub class: String,
}

impl Component for MediaView {
    fn render(&self, scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let view_id = format!("rtc-view-{}", crate::next_id());
        let el = super::backend::create_view(scope, &view_id, &self.track);
        if !self.style.is_empty() {
            el.set_attribute("style", &self.style);
        }
        let class = if self.class.is_empty() {
            "rinch-media-view".to_string()
        } else {
            format!("rinch-media-view {}", self.class)
        };
        el.set_attribute("class", &class);
        // Base look after the app's style: black backdrop, contain fit.
        el.set_style("background-color", "#000");
        el.set_style("object-fit", "contain");
        if self.mirrored {
            el.set_style("transform", "scaleX(-1)");
        }
        for child in children {
            el.append_child(child);
        }
        el
    }
}
