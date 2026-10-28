//! The GPU window's frame with `GameViewport`s as split base layers, and its
//! fallback when a base split is refused (#1866, R1; Joe 2026-10-27).
//!
//! `RinchRuntime::paint_gpu` builds each frame through [`BaseFallback::build`]:
//! with the viewports named to paint as base layers, the frame is built once;
//! if paint refused one of them (`rinch_dom::paint::take_base_refused`: a
//! group effect, a rotation or skew, an overlap, the memory cap, a taint),
//! the window switches to the layered path (`fall_back`: the games as layers
//! under the UI) and the frame is built again, **in full**, with nothing
//! split. The first build consumed the frame's damage (its dirty lists and
//! `scene_dirty`), so a second build left to its own damage would be
//! `Unchanged` and the change lost (#1883 review, D1). The window then stays
//! layered for [`BASE_FALLBACK_FRAMES`] frames before it tries again.
//!
//! The first frame that names base viewports after one that did not (the
//! fallback's end, or the window's first) is built in full too. On a
//! transparent window the UI texture's base is transparent with layers and
//! without, so the renderer does not ask for one; a partial frame that does
//! not reach a viewport neither splits nor refuses it, the layers are gone,
//! and the viewport showed the layered UI texture's hole with no game behind
//! it until something repainted it (#1883 verification, D2).
use std::collections::HashMap;

use crate::app::RinchApp;
use crate::split_frame::SplitChange;
use rinch_dom::paint::BaseViewport;
use rinch_dom::perf::FullRepaintReason;

/// Frames the GPU window composites its `GameViewport`s as layers after a
/// base split was refused, before it tries the split again.
pub(crate) const BASE_FALLBACK_FRAMES: u32 = 120;

/// The fallback's state: how many frames stay layered.
#[derive(Default)]
pub(crate) struct BaseFallback {
    frames: u32,
    /// The last frame split its base viewports (named them and none was
    /// refused).
    based: bool,
}

impl BaseFallback {
    /// Whether this frame may try to split its `GameViewport`s; counts a
    /// frame of the fallback down.
    pub(crate) fn try_split(&mut self) -> bool {
        let ok = self.frames == 0;
        self.frames = self.frames.saturating_sub(1);
        ok
    }

    /// Build `app`'s frame. `bases` (`None`: not split this frame) are named
    /// to paint; `split` is whether the frame may split at all; `target` is
    /// the renderer's `ui_invalid_reason`. On a refusal `fall_back` switches
    /// the renderer to the layered path and the frame is built again, in
    /// full. The frame to present is `app.last_gpu_frame()`; the split
    /// change to apply is returned (the second build's, on a refusal, which
    /// drops every kept slice).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build(
        &mut self,
        app: &mut RinchApp,
        scale: f64,
        size: (u32, u32),
        bases: Option<HashMap<String, BaseViewport>>,
        split: bool,
        target: Option<FullRepaintReason>,
        fall_back: impl FnOnce(),
    ) -> Option<SplitChange> {
        let use_base = bases.is_some();
        // Entering the split path: every viewport must be painted, and split,
        // this frame (D2).
        let target = if use_base && !self.based {
            Some(FullRepaintReason::Invalidated)
        } else {
            target
        };
        rinch_dom::paint::set_base_viewports(bases);
        let _ = rinch_dom::paint::take_base_refused();
        app.set_split_enabled(split);
        let change = app.build_gpu_frame_split(scale, size, target).1;
        rinch_dom::paint::set_base_viewports(None);
        if !(use_base && rinch_dom::paint::take_base_refused()) {
            self.based = use_base;
            return change;
        }
        self.based = false;
        self.frames = BASE_FALLBACK_FRAMES;
        #[cfg(debug_assertions)]
        tracing::debug!(
            "rinch: a GameViewport could not be split as the base layer; its game is \
             composited as a layer for {BASE_FALLBACK_FRAMES} frames (#1866)"
        );
        fall_back();
        app.set_split_enabled(false);
        let change = app
            .build_gpu_frame_split(scale, size, Some(FullRepaintReason::Invalidated))
            .1;
        let _ = rinch_dom::paint::take_base_refused();
        change
    }
}

/// A CPU `GameViewport` frame as the split path uploads it
/// (`SplitFrame::upload_split_frames`): shared, not copied, and marked new
/// only when it is (`collect_surface_frames_shared`'s flag), so an unchanged
/// game frame is not uploaded again.
pub(crate) fn pending_base_frame(
    id: usize,
    pixels: &std::sync::Arc<Vec<u8>>,
    (width, height): (u32, u32),
    fresh: bool,
) -> crate::split_frame::PendingFrame {
    (
        id,
        rinch_dom::paint::SurfacePixelData {
            data: std::sync::Arc::clone(pixels),
            width,
            height,
            opaque: false,
        },
        fresh,
    )
}
