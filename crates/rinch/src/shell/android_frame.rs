//! The app-level half of one Android frame: turn the frame clock, then say
//! what the frame still needs before the surface is presented.
//!
//! `android_runtime` compiles only for `target_os = "android"`, but what a
//! frame *is* costs nothing to run on a host, and it is the part that has to be
//! pinned down by tests — so it lives here and is compiled for the host test
//! build too, the way `touch_gesture` and `android_ime` are.

use rinch_platform::{AppAction, PlatformEvent};

use crate::app::RinchApp;

/// What one frame asked of the shell.
pub(crate) struct Frame {
    /// What `RinchApp` wants done — a redraw request, an exit, and so on.
    /// Read by the loop in `android_runtime`, which the host test build does
    /// not compile.
    #[cfg_attr(not(all(feature = "android", target_os = "android")), allow(dead_code))]
    pub actions: Vec<AppAction>,
    /// Layout the shell still has to resolve before it paints.
    pub pending_layout: bool,
    /// Pixels the shell still has to present.
    ///
    /// A transition on a paint-only property — `opacity`, `transform`, a
    /// colour — marks its node `PAINT`-dirty and nothing else, so
    /// `has_pending_layout` stays false and the loop's other two reasons to
    /// present (a `RequestRedraw` action, scroll momentum) never fire either.
    /// Turning the clock without this is turning it in private: the sheet
    /// slides in the tree and the surface still shows the frame from before
    /// the tap, until something unrelated forces a present — the next tap,
    /// which by then is landing on a sheet that is logically already open.
    pub needs_paint: bool,
}

/// Turn the frame clock and report what the frame needs.
///
/// [`PlatformEvent::AboutToWait`] *is* the clock. It is the one place
/// `RinchApp` advances CSS transitions and CSS animations, marks the scene
/// dirty when either moved, resolves the dirty state that the input handlers
/// deliberately leave for it to batch, and drains the pending focus requests
/// effects raise.
///
/// A shell that never sends it has no clock at all, and the failure is quiet
/// rather than obviously frozen: a transitioned property is sampled once, at
/// the instant the transition starts — which is its *old* value — and stays
/// there for ever, while every un-transitioned property on the same element
/// applies immediately. A bottom sheet written the ordinary way (a full-screen
/// root whose `pointer-events` flips, a scrim whose opacity fades, a panel that
/// slides up from below the fold) then answers a tap by going pointer-active
/// and not moving: the sheet is logically open, invisible, and covering the
/// screen, so the next tap anywhere lands on its scrim and closes it again.
/// Nothing about the app looks broken — every screen still paints, every
/// un-animated control still works — which is why this was found by tapping a
/// chip on a phone rather than by anything in CI.
///
/// The Android loop polls with a 16ms timeout, so calling this once per
/// iteration gives transitions and animations a ~60Hz clock, the same one the
/// winit shell gets from `ActiveEventLoop::about_to_wait`.
pub(crate) fn pump_frame(app: &mut RinchApp, window_size: (u32, u32), scale_factor: f64) -> Frame {
    let actions = app.handle_event(PlatformEvent::AboutToWait, window_size, scale_factor);
    Frame {
        pending_layout: app.has_pending_layout(),
        needs_paint: app.scene_dirty,
        actions,
    }
}
