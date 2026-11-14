//! GPU presents paced by Wayland frame callbacks (#2669).
//!
//! A `Fifo` present on Wayland (Mesa's Vulkan WSI, and EGL with a swap
//! interval of 1) commits its frame with a `wl_surface.frame` callback and,
//! at the next present, waits inside the driver for that callback, on the
//! thread that presents: the main thread. A compositor answers a surface's
//! frame callback only while it shows the surface, so a present to a surface
//! it does not show (a layer surface under another on an overlapping output,
//! a toplevel on a workspace that is not shown, a minimised window on some
//! compositors) never returned, and every window of the app stopped with it.
//!
//! So a window's renderer asks winit for a frame callback of its own
//! ([`Window::pre_present_notify`]) just before each such present. The
//! callback goes in the same commit as the driver's, and a compositor
//! answers the callbacks of one commit together, so once winit's has arrived
//! the driver's has too and the next present cannot wait. winit holds the
//! window's `RedrawRequested` while its callback is outstanding and delivers
//! a redraw requested meanwhile once it arrives, so nothing is lost: a
//! surface the compositor stops showing stops being presented to, and starts
//! again when the compositor shows it. The few presents that do not come
//! from a `RedrawRequested` (a debug screenshot) are held here instead
//! ([`PresentStep::Hold`]): the frame stays in the UI texture and a redraw
//! is requested, which presents it once the callback is in.
//!
//! Only `Fifo` and `FifoRelaxed` are paced: `Mailbox` and `Immediate` never
//! wait for a callback, and pacing them would cap the uncapped frame rate
//! `set_vsync(false)` asks for (#2095). Only Wayland surfaces are paced:
//! `pre_present_notify` does nothing on X11, Windows and macOS, so there is
//! no callback to wait for, and their presents are as before.

use std::sync::Arc;

use wgpu::PresentMode;
use winit::window::Window;

/// What a present does, from [`FrameGate::step`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PresentStep {
    /// Present, asking for a frame callback first when `paced`.
    Present { paced: bool },
    /// Do not present: the last frame's callback has not arrived.
    Hold,
}

/// The pacing decision, without the window: whether the surface is paced
/// by frame callbacks and whether one is outstanding.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameGate {
    /// The surface is a Wayland one.
    pub(crate) wayland: bool,
    /// A paced present went out and its frame callback has not been seen
    /// arriving (no `RedrawRequested` since).
    pub(crate) awaiting: bool,
}

impl FrameGate {
    /// What a present in `mode` does now.
    pub(crate) fn step(&self, mode: PresentMode) -> PresentStep {
        if self.awaiting {
            return PresentStep::Hold;
        }
        PresentStep::Present {
            paced: self.wayland && paced_mode(mode),
        }
    }

    /// A present that [`Self::step`] allowed went out.
    pub(crate) fn presented(&mut self, step: PresentStep) {
        if step == (PresentStep::Present { paced: true }) {
            self.awaiting = true;
        }
    }

    /// The window got a `RedrawRequested`, which winit delivers only once
    /// the frame callback it asked for has arrived.
    pub(crate) fn callback_arrived(&mut self) {
        self.awaiting = false;
    }
}

/// Whether a present in `mode` waits on Wayland for the previous frame's
/// callback.
pub(crate) fn paced_mode(mode: PresentMode) -> bool {
    matches!(mode, PresentMode::Fifo | PresentMode::FifoRelaxed)
}

/// A renderer's pacing: the window it presents to and its [`FrameGate`].
#[derive(Default)]
pub(crate) struct FramePacing {
    window: Option<Arc<dyn Window>>,
    pub(crate) gate: FrameGate,
}

impl std::fmt::Debug for FramePacing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FramePacing")
            .field("window", &self.window.is_some())
            .field("gate", &self.gate)
            .finish()
    }
}

impl FramePacing {
    /// Pace presents by `window`, the window this renderer presents to. A
    /// different window than before starts with no callback outstanding.
    pub(crate) fn pace_by(&mut self, window: &Arc<dyn Window>) {
        if self.window.as_ref().is_some_and(|w| Arc::ptr_eq(w, window)) {
            return;
        }
        self.window = Some(window.clone());
        self.gate = FrameGate {
            wayland: is_wayland(&**window),
            awaiting: false,
        };
    }

    /// The renderer's surface went (a hide, #2545): no window, nothing
    /// outstanding.
    pub(crate) fn detach(&mut self) {
        *self = Self::default();
    }

    /// Ask winit for a frame callback, for the present `step` that is about
    /// to go out; nothing when it is not paced.
    pub(crate) fn before_present(&mut self, step: PresentStep) {
        if step == (PresentStep::Present { paced: true })
            && let Some(window) = &self.window
        {
            window.pre_present_notify();
        }
        self.gate.presented(step);
    }

    /// A present was held: redraw once the callback is in, to present the
    /// frame then.
    pub(crate) fn held(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

/// Whether `window` is a Wayland surface.
fn is_wayland(window: &dyn Window) -> bool {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    window
        .window_handle()
        .is_ok_and(|h| matches!(h.as_raw(), RawWindowHandle::Wayland(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use PresentMode::{Fifo, FifoRelaxed, Immediate, Mailbox};

    /// The decision #2669 needs: on Wayland a Fifo present asks for a frame
    /// callback, the next present is held while it is outstanding, and the
    /// one after the `RedrawRequested` that says it arrived goes out.
    #[test]
    fn a_wayland_fifo_present_is_held_until_its_callback_arrives_2669() {
        for mode in [Fifo, FifoRelaxed] {
            let mut gate = FrameGate {
                wayland: true,
                awaiting: false,
            };
            let first = gate.step(mode);
            assert_eq!(first, PresentStep::Present { paced: true }, "{mode:?}");
            gate.presented(first);
            // The compositor has not answered: every present is held, however
            // many redraws ask (a configure, an animation, a screenshot).
            for _ in 0..3 {
                assert_eq!(gate.step(mode), PresentStep::Hold, "{mode:?}");
            }
            gate.callback_arrived();
            let next = gate.step(mode);
            assert_eq!(next, PresentStep::Present { paced: true }, "{mode:?}");
            gate.presented(next);
            assert_eq!(gate.step(mode), PresentStep::Hold, "{mode:?}");
        }
    }

    /// Mailbox and Immediate never wait in the driver: they are not paced,
    /// so `set_vsync(false)` stays uncapped (#2095).
    #[test]
    fn uncapped_modes_are_not_paced_2669() {
        for mode in [Mailbox, Immediate] {
            let mut gate = FrameGate {
                wayland: true,
                awaiting: false,
            };
            for _ in 0..3 {
                let step = gate.step(mode);
                assert_eq!(step, PresentStep::Present { paced: false }, "{mode:?}");
                gate.presented(step);
            }
        }
    }

    /// X11, Windows and macOS have no frame callback to wait for: every
    /// present goes out, as before.
    #[test]
    fn other_platforms_are_not_paced_2669() {
        for mode in [Fifo, FifoRelaxed, Mailbox, Immediate] {
            let mut gate = FrameGate::default();
            for _ in 0..3 {
                let step = gate.step(mode);
                assert_eq!(step, PresentStep::Present { paced: false }, "{mode:?}");
                gate.presented(step);
            }
        }
    }

    /// A callback outstanding from a Fifo present still holds a present after
    /// a switch to an uncapped mode: the driver's own callback for that
    /// commit is outstanding too.
    #[test]
    fn an_outstanding_callback_holds_whatever_the_mode_2669() {
        let mut gate = FrameGate {
            wayland: true,
            awaiting: false,
        };
        gate.presented(gate.step(Fifo));
        assert_eq!(gate.step(Mailbox), PresentStep::Hold);
        gate.callback_arrived();
        assert_eq!(gate.step(Mailbox), PresentStep::Present { paced: false });
    }
}
