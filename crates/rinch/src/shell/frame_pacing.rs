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
//!
//! **Not where the compositor offers `wp_fifo_v1`.** There Mesa paces `Fifo`
//! with the protocol's barriers (`set_barrier`/`wait_barrier`) and asks for no
//! frame callback, so a hidden surface cannot stall a present, and pacing by
//! callbacks as well only stacks a second wait on every frame: under KWin
//! 6.6 (which offers it and stops answering a hidden window's callbacks)
//! deskrinch measured a visible window's presents fall from 48.5 and 38.9/s
//! to 24.7 and 27.9/s with both. So by default ([`WaylandFramePacing::Auto`])
//! a surface is paced only when the registry has no `wp_fifo_manager_v1`
//! (sway and labwc today). Whether it has one is read once, from the winit
//! fork's `ActiveEventLoopExtWayland::wayland_globals` (the registry winit
//! already holds, as `window_roles_of` reads it for #2659) when the event
//! loop can create surfaces ([`note_compositor`]): no second connection or
//! roundtrip. `RINCH_WAYLAND_FRAME_PACING=auto|on|off`, or
//! [`App::wayland_frame_pacing`](crate::App::wayland_frame_pacing), forces it
//! (`on` for a driver that does not use fifo-v1 although the compositor
//! offers it, such as a Mesa older than its support; `off` for the old
//! behaviour), the variable over the app's choice.

use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use wgpu::PresentMode;
use winit::window::Window;

/// Whether the GPU renderer paces its presents on Wayland by frame
/// callbacks (#2669), so that a window the compositor does not show cannot
/// stall the app. Set with [`set_wayland_frame_pacing`] or
/// [`App::wayland_frame_pacing`](crate::App::wayland_frame_pacing); the
/// `RINCH_WAYLAND_FRAME_PACING` environment variable (`auto`, `on`, `off`)
/// overrides both. Other platforms, `Mailbox`/`Immediate` presents and the
/// software renderer are never paced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WaylandFramePacing {
    /// Pace when the compositor does not offer `wp_fifo_v1` (with it, the
    /// driver paces `Fifo` with its barriers and a hidden window cannot
    /// stall a present).
    #[default]
    Auto,
    /// Always pace on Wayland.
    On,
    /// Never pace (rinch before #2669).
    Off,
}

impl WaylandFramePacing {
    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "on" | "1" | "true" => Some(Self::On),
            "off" | "0" | "false" => Some(Self::Off),
            _ => None,
        }
    }

    fn to_u8(self) -> u8 {
        match self {
            Self::Auto => 0,
            Self::On => 1,
            Self::Off => 2,
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::On,
            2 => Self::Off,
            _ => Self::Auto,
        }
    }
}

/// The app's choice, as [`WaylandFramePacing::to_u8`].
static REQUESTED: AtomicU8 = AtomicU8::new(0);
/// The compositor offers `wp_fifo_manager_v1` ([`note_compositor`]).
static COMPOSITOR_FIFO: AtomicBool = AtomicBool::new(false);

/// Choose whether the GPU renderer paces its Wayland presents by frame
/// callbacks ([`WaylandFramePacing`]). Read when a renderer starts presenting
/// to a window; `RINCH_WAYLAND_FRAME_PACING` overrides it.
pub fn set_wayland_frame_pacing(mode: WaylandFramePacing) {
    REQUESTED.store(mode.to_u8(), Ordering::SeqCst);
}

/// The mode in force: the environment's, else the app's.
fn wayland_frame_pacing() -> WaylandFramePacing {
    static ENV: OnceLock<Option<WaylandFramePacing>> = OnceLock::new();
    let env = *ENV.get_or_init(|| {
        let v = std::env::var("RINCH_WAYLAND_FRAME_PACING").ok()?;
        if v.trim().is_empty() {
            return None;
        }
        let mode = WaylandFramePacing::parse(&v);
        if mode.is_none() {
            tracing::warn!("RINCH_WAYLAND_FRAME_PACING={v:?} is not auto, on or off: ignored");
        }
        mode
    });
    env.unwrap_or_else(|| WaylandFramePacing::from_u8(REQUESTED.load(Ordering::SeqCst)))
}

/// Whether a registry advertising `interfaces` offers `wp_fifo_v1`.
pub(crate) fn offers_fifo_v1<'a>(interfaces: impl IntoIterator<Item = &'a str>) -> bool {
    interfaces.into_iter().any(|i| i == "wp_fifo_manager_v1")
}

/// Read whether the compositor offers `wp_fifo_v1`, from the globals the
/// winit fork's registry holds. Called when the event loop can create
/// surfaces, before any window presents; nothing off Wayland.
pub(crate) fn note_compositor(event_loop: &dyn winit::event_loop::ActiveEventLoop) {
    #[cfg(target_os = "linux")]
    if super::overlay_window::Backend::of(event_loop) == super::overlay_window::Backend::Wayland {
        use winit::platform::wayland::ActiveEventLoopExtWayland;
        let globals = event_loop.wayland_globals();
        let fifo = offers_fifo_v1(globals.iter().map(|g| g.interface.as_str()));
        COMPOSITOR_FIFO.store(fifo, Ordering::SeqCst);
        tracing::debug!("rinch: the compositor offers wp_fifo_v1: {fifo} (#2669)");
    }
    let _ = event_loop;
}

/// Whether presents to a surface are paced by frame callbacks: a Wayland
/// surface, under `mode`, on a compositor that does or does not offer
/// `wp_fifo_v1`.
pub(crate) fn paces(wayland: bool, fifo_v1: bool, mode: WaylandFramePacing) -> bool {
    wayland
        && match mode {
            WaylandFramePacing::Auto => !fifo_v1,
            WaylandFramePacing::On => true,
            WaylandFramePacing::Off => false,
        }
}

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
    /// The surface's presents are paced by frame callbacks: a Wayland one,
    /// where [`paces`] says so.
    pub(crate) by_callbacks: bool,
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
            paced: self.by_callbacks && paced_mode(mode),
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
            by_callbacks: paces(
                is_wayland(&**window),
                COMPOSITOR_FIFO.load(Ordering::SeqCst),
                wayland_frame_pacing(),
            ),
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
                by_callbacks: true,
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
                by_callbacks: true,
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

    /// The decision deskrinch asked for: by default a Wayland surface is paced
    /// only when the compositor has no `wp_fifo_v1` (sway, labwc); with it
    /// (KWin) the driver's barriers pace Fifo. `on` and `off` force it, and
    /// nothing off Wayland is ever paced.
    #[test]
    fn auto_paces_only_without_fifo_v1_and_on_off_force_it_2669() {
        use WaylandFramePacing::{Auto, Off, On};
        assert!(paces(true, false, Auto), "sway, labwc: no fifo-v1");
        assert!(!paces(true, true, Auto), "KWin: fifo-v1");
        assert!(paces(true, true, On));
        assert!(paces(true, false, On));
        assert!(!paces(true, false, Off));
        assert!(!paces(true, true, Off));
        for fifo in [false, true] {
            for mode in [Auto, On, Off] {
                assert!(!paces(false, fifo, mode), "not Wayland: {fifo} {mode:?}");
            }
        }
        assert_eq!(WaylandFramePacing::default(), Auto);
    }

    /// `wp_fifo_v1`'s global is `wp_fifo_manager_v1`; a registry without it
    /// (or with only a look-alike) does not offer it.
    #[test]
    fn fifo_v1_is_read_from_the_registry_interfaces_2669() {
        let kwin = [
            "wl_compositor",
            "wp_fifo_manager_v1",
            "wp_commit_timing_manager_v1",
        ];
        assert!(offers_fifo_v1(kwin));
        let sway = ["wl_compositor", "zwlr_layer_shell_v1", "wp_presentation"];
        assert!(!offers_fifo_v1(sway));
        assert!(!offers_fifo_v1(["wp_fifo_manager_v2", "fifo"]));
        assert!(!offers_fifo_v1([]));
    }

    /// The variable's spellings.
    #[test]
    fn the_mode_parses_2669() {
        use WaylandFramePacing::{Auto, Off, On};
        assert_eq!(WaylandFramePacing::parse(" AUTO "), Some(Auto));
        assert_eq!(WaylandFramePacing::parse("on"), Some(On));
        assert_eq!(WaylandFramePacing::parse("off"), Some(Off));
        assert_eq!(WaylandFramePacing::parse("sometimes"), None);
        for m in [Auto, On, Off] {
            assert_eq!(WaylandFramePacing::from_u8(m.to_u8()), m);
        }
    }

    /// A callback outstanding from a Fifo present still holds a present after
    /// a switch to an uncapped mode: the driver's own callback for that
    /// commit is outstanding too.
    #[test]
    fn an_outstanding_callback_holds_whatever_the_mode_2669() {
        let mut gate = FrameGate {
            by_callbacks: true,
            awaiting: false,
        };
        gate.presented(gate.step(Fifo));
        assert_eq!(gate.step(Mailbox), PresentStep::Hold);
        gate.callback_arrived();
        assert_eq!(gate.step(Mailbox), PresentStep::Present { paced: false });
    }
}
