//! A window's input region in the shell (#2741): computed in each paint from
//! the layout that paint draws, handed to the windowing system just before
//! the frame's present so that on Wayland it lands in that frame's commit,
//! and only when it changed.

use super::*;
use rinch_core::element::InputRegion;

/// What a window's OS surface was last given: the whole window, or these
/// pixel rects. Kept with the id of the OS window it was given to, so a
/// window re-created by a show is given its region again.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct AppliedRegion {
    window: winit::window::WindowId,
    region: Option<Vec<(i32, i32, i32, i32)>>,
    scale_bits: u64,
}

impl AppliedRegion {
    /// Whether `(x, y)` (logical px) is in the region as sent: the whole
    /// window for `None`, else one of the whole-pixel rects.
    pub(super) fn takes_press_at(&self, x: f32, y: f32) -> bool {
        self.region
            .as_deref()
            .is_none_or(|rects| crate::app::input_region::contains_pixels(rects, x, y))
    }
}

impl ShellWindow {
    /// The mode the window's region follows: click-through wins (#1519).
    pub(super) fn effective_input_region(&self) -> InputRegion {
        if self.click_through {
            InputRegion::None
        } else {
            self.input_region
        }
    }

    /// Whether a press at `(x, y)` (logical px) is in the window's input
    /// region, as the windowing system was last given it. A press outside
    /// is not dispatched: where the OS has the region it never sends one,
    /// and where it has none (Windows, macOS) this is the whole of it.
    ///
    /// Tested against the rects actually sent (`AppliedRegion`: edges
    /// rounded outwards to whole logical px), not the unrounded ones, so a
    /// press the compositor delivers just inside a rounded edge is kept.
    pub(super) fn takes_press_at(&self, x: f32, y: f32) -> bool {
        let current = self.window.as_ref().map(|w| w.raw().id());
        match &self.applied_region {
            Some(applied) if Some(applied.window) == current => applied.takes_press_at(x, y),
            _ => self
                .input_region_state
                .takes_press_at(self.effective_input_region(), x, y),
        }
    }

    /// A new mode (`set_input_region_current_window`, a click-through
    /// toggle): computed again and given to the window at the next paint,
    /// which this asks for. `All` and `None` need no layout, so they go out
    /// at once, committed: no frame may follow.
    pub(super) fn note_input_region_change(&mut self) {
        self.input_region_state.invalidate();
        let mode = self.effective_input_region();
        if matches!(mode, InputRegion::All | InputRegion::None) {
            self.send_input_region(true);
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Compute the window's region from its current layout and, if it is
    /// not what the OS surface has, give it. `commit: false` stages it for
    /// the present that follows (Wayland applies it with that commit);
    /// returns whether anything was staged, so that a paint that then
    /// presents nothing commits it itself ([`Self::commit_input_region`]).
    pub(super) fn send_input_region(&mut self, commit: bool) -> bool {
        let mode = self.effective_input_region();
        send_region(
            self.window.as_ref(),
            &self.app,
            mode,
            &mut self.input_region_state,
            &mut self.applied_region,
            commit,
        )
    }

    /// Commit a region [`Self::send_input_region`] staged for a present that
    /// did not happen (nothing changed on screen, or the present was held):
    /// the surface is committed with the buffer it already has, which shows
    /// the same frame the region was computed from.
    pub(super) fn commit_input_region(&mut self) {
        let Some(window) = &self.window else {
            return;
        };
        let rects: Option<Vec<peniko::kurbo::Rect>> = match self.effective_input_region() {
            InputRegion::All => None,
            _ => Some(self.input_region_state.rects().to_vec()),
        };
        overlay_window::set_input_region(
            window.raw(),
            rects.as_deref(),
            window.scale_factor(),
            true,
        );
    }
}

/// [`ShellWindow::send_input_region`] on the window's parts, for a caller
/// that holds another of its fields (the GPU paint holds the renderer).
pub(super) fn send_region(
    window: Option<&WinitWindow>,
    app: &RinchApp,
    mode: InputRegion,
    state: &mut crate::app::input_region::InputRegionState,
    applied_region: &mut Option<AppliedRegion>,
    commit: bool,
) -> bool {
    let Some(window) = window else {
        return false;
    };
    let Some(doc) = app.doc.as_ref() else {
        return false;
    };
    let viewport = window.logical_size();
    let scale = window.scale_factor();
    let doc = doc.borrow();
    let rects = state.region(&doc.tree, mode, viewport);
    // What the surface is given, in Wayland's surface-local logical px;
    // `overlay_window::set_input_region` makes X11's physical px itself.
    let applied = AppliedRegion {
        window: window.raw().id(),
        region: rects.map(|r| crate::app::input_region::to_pixels(r, 1.0)),
        scale_bits: scale.to_bits(),
    };
    if applied_region.as_ref() == Some(&applied) {
        return false;
    }
    // A new OS window takes input everywhere: `All` there is already so, and
    // a window that never asks for anything else never sends a region.
    let fresh = applied_region
        .as_ref()
        .is_none_or(|a| a.window != applied.window);
    if fresh && applied.region.is_none() {
        *applied_region = Some(applied);
        return false;
    }
    doc.tree
        .perf
        .bump(rinch_dom::perf::Counter::InputRegionSends);
    overlay_window::set_input_region(window.raw(), rects, scale, commit);
    *applied_region = Some(applied);
    !commit
}
