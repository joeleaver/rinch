//! Software presentation layer using softbuffer.
//!
//! Receives RGBA8 pixel data and presents it to a winit window via
//! softbuffer's CPU-based surface, bypassing the GPU entirely.
//! Supports window transparency via `AlphaMode::Premultiplied` on
//! platforms that support it (Wayland, macOS, DRM/KMS).
//!
//! A window that has drawn nothing for its idle period drops its softbuffer
//! surface, and the buffers with it (`Presenter`, #2843).

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rinch_core::element::IdleTrim;
use softbuffer::AlphaMode;
use winit::window::Window;

/// A software renderer that presents RGBA8 pixels to a window via softbuffer.
///
/// Its softbuffer surface is dropped once the window has drawn nothing for
/// its idle period ([`IdleTrim`], #2843), the buffers with it, and the next
/// present makes a new one. See `Presenter`.
pub struct SoftbufferRenderer {
    inner: IdleTrimmed<SoftSurface>,
}

/// What presents a frame's pixels: a softbuffer surface ([`SoftSurface`]),
/// or a test's stand-in.
pub(crate) trait PresentTarget {
    /// Present `pixels` (`width * height * 4` bytes, RGBA8, tightly packed).
    fn present(&mut self, pixels: &[u8], width: u32, height: u32);
}

/// A window's presentation (#2843): the target it presents to while it has
/// one, which frame that shows ([`ShownFrame`]), and when the target is
/// dropped for idleness.
///
/// The target is made by `make` at a present that finds none: the first, and
/// the first after an idle trim. Every present copies the whole frame and
/// damages the whole surface, so a new target loses no history: the frame
/// after a trim is complete, whatever the painter repainted for it.
///
/// Dropping a softbuffer surface on Wayland destroys both its `wl_buffer`s
/// and their pools and unmaps them. The compositor keeps showing what it was
/// last given: "Destroying the wl_buffer after wl_buffer.release does not
/// change the surface contents", and before the release it is allowed too,
/// the storage never being written again (`wl_surface.attach`).
pub(crate) struct Presenter<T> {
    target: Option<T>,
    make: Box<dyn FnMut(u32, u32) -> T>,
    /// Which frame the surface shows, and the size it was presented at.
    shown: ShownFrame,
    size: (u32, u32),
    /// How long the window draws nothing before the target is dropped;
    /// `None`, never. Only a surface that keeps its contents
    /// ([`ShownFrame::retains`], a Wayland one) is trimmed: elsewhere an
    /// expose has to present again anyway, and on macOS dropping the surface
    /// takes its layer, and the picture, off the window.
    idle_period: Option<Duration>,
    /// When the target last presented.
    last_present: Option<Instant>,
    /// The target was dropped by an idle trim, and none has been made since.
    trimmed: bool,
    /// An idle trim dropped the target, and [`Self::take_trimmed`] has not
    /// been asked since (#2852). One-shot, wherever the trim ran: at a wake,
    /// or inside a redraw that presented nothing.
    trim_unreported: bool,
    /// A present made the target again after a trim, and
    /// [`Self::take_untrimmed`] has not been asked since (#2852).
    untrimmed: bool,
}

impl<T: PresentTarget> Presenter<T> {
    /// A presentation with no target yet; `make(width, height)` makes one.
    /// `idle_period` as [`Self::idle_period`]; ignored unless `retains`.
    pub(crate) fn new(
        retains: bool,
        idle_period: Option<Duration>,
        make: impl FnMut(u32, u32) -> T + 'static,
    ) -> Self {
        Self {
            target: None,
            make: Box::new(make),
            shown: ShownFrame::new(retains),
            size: (0, 0),
            idle_period: idle_period.filter(|_| retains),
            last_present: None,
            trimmed: false,
            trim_unreported: false,
            untrimmed: false,
        }
    }

    /// Present the painter's frame `generation`, unless the surface already
    /// shows it ([`ShownFrame`], #2698). Returns whether it presented.
    pub(crate) fn present_frame(
        &mut self,
        pixels: &[u8],
        width: u32,
        height: u32,
        generation: u64,
        now: Instant,
    ) -> bool {
        // A size the surface does not have is always presented: the frame
        // of a resize moved the generation on, so this only guards it.
        if (width, height) != self.size {
            self.shown = ShownFrame::new(self.shown.retains);
        }
        if !self.shown.take(generation) {
            return false;
        }
        self.present_pixels(pixels, width, height, now);
        true
    }

    /// Present `pixels`, whatever the surface shows, making the target first
    /// if there is none.
    pub(crate) fn present_pixels(&mut self, pixels: &[u8], width: u32, height: u32, now: Instant) {
        if self.target.is_none() && std::mem::take(&mut self.trimmed) {
            self.untrimmed = true;
        }
        let make = &mut self.make;
        let target = self.target.get_or_insert_with(|| make(width, height));
        target.present(pixels, width, height);
        self.size = (width, height);
        self.last_present = Some(now);
    }

    /// When the target is dropped if nothing presents before then: `None`
    /// while there is no target, or when none is ever dropped.
    pub(crate) fn idle_deadline(&self) -> Option<Instant> {
        self.target.as_ref()?;
        Some(self.last_present? + self.idle_period?)
    }

    /// Drop the target if the window has presented nothing for its idle
    /// period at `now`. Returns whether it did. What the surface shows is
    /// unchanged.
    pub(crate) fn trim_if_idle(&mut self, now: Instant) -> bool {
        if self.idle_deadline().is_none_or(|deadline| now < deadline) {
            return false;
        }
        self.target = None;
        self.trimmed = true;
        self.trim_unreported = true;
        true
    }

    /// Whether an idle trim dropped the target since the last call: the
    /// window's buffers were given back (#2852).
    pub(crate) fn take_trimmed(&mut self) -> bool {
        std::mem::take(&mut self.trim_unreported)
    }

    /// Whether a present made the target again after an idle trim since the
    /// last call: the window's buffers are allocated again (#2852).
    pub(crate) fn take_untrimmed(&mut self) -> bool {
        std::mem::take(&mut self.untrimmed)
    }

    /// The target, while there is one.
    #[cfg(test)]
    pub(crate) fn target(&self) -> Option<&T> {
        self.target.as_ref()
    }
}

/// A [`Presenter`] and the timer that wakes the event loop at its idle
/// deadline (#2843).
///
/// One timer at a time. A frame presented while it is armed does not touch
/// it: the timer's wake ([`Self::service_idle_trim`]) finds the window drew
/// since, and arms one for the later deadline. So a window that animates
/// pays one wake per idle period, not a timer per frame, and a window that
/// has stopped pays the one at its deadline and then none (a cleared timer
/// wakes nothing, #2832).
pub(crate) struct IdleTrimmed<T> {
    presenter: Presenter<T>,
    /// The armed timer, and the deadline it was armed for.
    timer: Option<(rinch_core::TimeoutHandle, Instant)>,
    timers: TimerOps,
}

/// How [`IdleTrimmed`] arms and clears its timer: `rinch_core`'s timers,
/// or a test's that records them.
#[derive(Clone, Copy)]
pub(crate) struct TimerOps {
    /// Arm a timer `delay_ms` from now whose callback runs nothing.
    pub(crate) set: fn(u32) -> rinch_core::TimeoutHandle,
    pub(crate) clear: fn(rinch_core::TimeoutHandle),
}

impl TimerOps {
    /// `rinch_core::set_timeout` with a no-op callback, and
    /// `rinch_core::clear_timeout`.
    pub(crate) const CORE: Self = Self {
        set: |delay_ms| rinch_core::set_timeout(delay_ms, || {}),
        clear: rinch_core::clear_timeout,
    };
}

impl<T: PresentTarget> IdleTrimmed<T> {
    pub(crate) fn new(presenter: Presenter<T>) -> Self {
        Self::with_timers(presenter, TimerOps::CORE)
    }

    /// [`Self::new`] with `timers` in place of `rinch_core`'s.
    pub(crate) fn with_timers(presenter: Presenter<T>, timers: TimerOps) -> Self {
        Self {
            presenter,
            timer: None,
            timers,
        }
    }

    /// [`Presenter::present_frame`] at `now`, arming the idle timer if it
    /// presented. A redraw that presents nothing (its surface shows the
    /// frame already) is served as a wake ([`Self::service_idle_trim`]):
    /// the timer's own wake can ask for a frame too, and the runtime then
    /// leaves the trim to that frame, which presents nothing.
    pub(crate) fn present_frame(
        &mut self,
        pixels: &[u8],
        width: u32,
        height: u32,
        generation: u64,
        now: Instant,
    ) -> bool {
        let presented = self
            .presenter
            .present_frame(pixels, width, height, generation, now);
        if presented {
            self.arm(now);
        } else {
            self.service_idle_trim(now);
        }
        presented
    }

    /// [`Presenter::present_pixels`] at `now`, arming the idle timer.
    pub(crate) fn present_pixels(&mut self, pixels: &[u8], width: u32, height: u32, now: Instant) {
        self.presenter.present_pixels(pixels, width, height, now);
        self.arm(now);
    }

    /// Drop the target if the window has drawn nothing for its idle period
    /// at `now`, and keep a timer armed for the deadline while it has not.
    /// Returns whether it dropped the target. Called at every wake of the
    /// event loop; the timer's is one of them.
    pub(crate) fn service_idle_trim(&mut self, now: Instant) -> bool {
        let trimmed = self.presenter.trim_if_idle(now);
        // After a trim there is no deadline, and the timer is cleared; else
        // a timer that has fired is replaced by one for the deadline the
        // frames since it was armed moved on to.
        self.arm(now);
        trimmed
    }

    /// Arm the timer for the presenter's deadline, unless one is armed
    /// already that has yet to fire: that one wakes first, and its wake arms
    /// the next.
    fn arm(&mut self, now: Instant) {
        if self.timer.is_some_and(|(_, at)| at > now) {
            return;
        }
        // One that has fired is spent (clearing it is a no-op, #2832).
        self.cancel();
        let Some(deadline) = self.presenter.idle_deadline() else {
            return;
        };
        let delay = deadline.saturating_duration_since(now);
        // Rounded up, so the wake finds the deadline passed.
        let delay_ms = u32::try_from(delay.as_micros().div_ceil(1000)).unwrap_or(u32::MAX);
        // The callback runs nothing: its wake is what the runtime serves
        // (`RinchRuntime::proxy_wake_up` calls `service_idle_trim`).
        let timer = (self.timers.set)(delay_ms);
        self.timer = Some((timer, deadline));
    }

    /// [`Presenter::take_trimmed`]. Covers the trim served by a redraw that
    /// presented nothing ([`Self::present_frame`]) as well as a wake's.
    pub(crate) fn take_trimmed(&mut self) -> bool {
        self.presenter.take_trimmed()
    }

    /// [`Presenter::take_untrimmed`].
    pub(crate) fn take_untrimmed(&mut self) -> bool {
        self.presenter.take_untrimmed()
    }

    fn cancel(&mut self) {
        if let Some((timer, _)) = self.timer.take() {
            (self.timers.clear)(timer);
        }
    }

    /// The deadline the armed timer wakes at, if one is armed.
    #[cfg(test)]
    pub(crate) fn timer_deadline(&self) -> Option<Instant> {
        self.timer.map(|(_, at)| at)
    }

    #[cfg(test)]
    pub(crate) fn presenter(&self) -> &Presenter<T> {
        &self.presenter
    }
}

impl<T> Drop for IdleTrimmed<T> {
    fn drop(&mut self) {
        // A hidden or closed window's timer wakes nothing (#2832).
        if let Some((timer, _)) = self.timer.take() {
            (self.timers.clear)(timer);
        }
    }
}

/// A softbuffer surface on a window: a desktop window's [`PresentTarget`].
/// Its buffers live as long as it does.
pub(crate) struct SoftSurface {
    surface: softbuffer::Surface<Arc<dyn Window>, Arc<dyn Window>>,
    width: u32,
    height: u32,
    /// Whether the surface was actually configured with premultiplied alpha (i.e.
    /// transparency is honored).
    ///
    /// This tracks the *resolved* alpha mode, NOT the requested `transparent`
    /// flag, because the two can diverge: e.g. the X11 backend does not support
    /// `Premultiplied`, so a `transparent: true` window still falls back to
    /// `AlphaMode::Opaque` — under which *every* presented pixel must be opaque or
    /// softbuffer's `present()` assertion fires. Gating alpha on the request
    /// instead of the resolved mode would write translucent pixels into an opaque
    /// surface and panic.
    premultiplied: bool,
}

/// Which painted frame a window's surface shows (#2698), by the painter's
/// [`pixels_generation`](crate::app::RinchApp::pixels_generation).
///
/// A software present converts and copies every pixel of the frame, so a
/// redraw that changed none of them and presents anyway pays that copy for
/// nothing. Redraws like that come at a show: winit asks for one at every
/// configure (a layer surface's or a toplevel's, its size changed or not),
/// and an input method enabled for a focused field asks for one; each that
/// lands after the show's first present presented the same pixels again.
///
/// Only a surface that keeps what it was given skips: a Wayland surface
/// shows its last committed buffer until it is given another. An X11
/// window's contents can be lost (an `Expose` is a redraw that has to
/// present), so elsewhere every redraw presents.
#[derive(Debug)]
pub(crate) struct ShownFrame {
    /// Whether the surface keeps its contents between presents.
    retains: bool,
    /// The frame last presented to it, if any.
    shown: Option<u64>,
}

impl ShownFrame {
    /// A new surface, showing nothing yet; `retains` as [`Self::retains`].
    pub(crate) fn new(retains: bool) -> Self {
        Self {
            retains,
            shown: None,
        }
    }

    /// Whether the frame of pixels `generation` is to be presented: not
    /// when the surface keeps its contents and already shows it. When it
    /// is, it is the frame shown from now on.
    pub(crate) fn take(&mut self, generation: u64) -> bool {
        if self.retains && self.shown == Some(generation) {
            return false;
        }
        self.shown = Some(generation);
        true
    }
}

/// Whether `window` is a Wayland surface, which keeps the buffer it was last
/// given.
fn retains_contents(window: &dyn Window) -> bool {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    window
        .window_handle()
        .is_ok_and(|h| matches!(h.as_raw(), RawWindowHandle::Wayland(_)))
}

impl SoftbufferRenderer {
    /// Create a new software renderer for the given window. It keeps its
    /// buffers for its life ([`IdleTrim::Never`]): a trim needs the event
    /// loop to call `service_idle_trim` at the timer's wake, which only the
    /// runtime's own windows do (they are made with `with_idle_trim`).
    pub fn new(window: Arc<dyn Window>, width: u32, height: u32, transparent: bool) -> Self {
        Self::with_idle_trim(window, width, height, transparent, IdleTrim::Never)
    }

    /// [`Self::new`] with the window's [`IdleTrim`]
    /// (`WindowProps::idle_trim`).
    pub(crate) fn with_idle_trim(
        window: Arc<dyn Window>,
        width: u32,
        height: u32,
        transparent: bool,
        idle_trim: IdleTrim,
    ) -> Self {
        let retains = retains_contents(&*window);
        let context = softbuffer::Context::new(window.clone()).unwrap();
        let make =
            move |w: u32, h: u32| SoftSurface::new(&context, window.clone(), w, h, transparent);
        // The first surface is made now, as it always was: a window that
        // cannot have one fails here rather than at its first frame. Later
        // ones are made by the first present after an idle trim.
        let mut first = Some(make(width.max(1), height.max(1)));
        let make = move |w, h| first.take().unwrap_or_else(|| make(w, h));
        Self {
            inner: IdleTrimmed::new(Presenter::new(retains, idle_trim.period(), make)),
        }
    }

    /// Present the painter's frame `generation`, unless the surface already
    /// shows it ([`ShownFrame`], #2698). Returns whether it presented.
    pub(crate) fn present_frame(
        &mut self,
        pixels: &[u8],
        width: u32,
        height: u32,
        generation: u64,
    ) -> bool {
        self.inner
            .present_frame(pixels, width, height, generation, Instant::now())
    }

    /// Present RGBA8 pixels to the window, whatever it shows
    /// (`SoftSurface`'s `present`).
    pub fn present_pixels(&mut self, pixels: &[u8], width: u32, height: u32) {
        self.inner
            .present_pixels(pixels, width, height, Instant::now());
    }

    /// Resize the surface to new dimensions. A present at another size
    /// resizes it too; with no surface (after an idle trim) the next one is
    /// made at the size of the frame it presents.
    pub fn resize(&mut self, width: u32, height: u32) {
        if let Some(surface) = self.inner.presenter.target.as_mut() {
            surface.resize(width, height);
        }
    }

    /// Drop the surface, and its buffers, if the window has drawn nothing
    /// for its idle period (#2843). Called at every wake of the event loop.
    /// Returns [`Self::take_trimmed`] (the window's `[PERF] trimmed` line,
    /// #2852).
    pub(crate) fn service_idle_trim(&mut self) -> bool {
        self.inner.service_idle_trim(Instant::now());
        self.take_trimmed()
    }

    /// Whether an idle trim dropped the surface since the last call, at a
    /// wake or inside a redraw that presented nothing (the window's
    /// `[PERF] trimmed` line, #2852).
    pub(crate) fn take_trimmed(&mut self) -> bool {
        let trimmed = self.inner.take_trimmed();
        if trimmed {
            tracing::debug!("rinch: an idle window's software buffers were released (#2843)");
        }
        trimmed
    }

    /// Whether a present made the surface again after an idle trim since the
    /// last call (the window's `[PERF] untrimmed` line, #2852).
    pub(crate) fn take_untrimmed(&mut self) -> bool {
        self.inner.take_untrimmed()
    }
}

impl SoftSurface {
    /// A surface on `window`, configured at `width` x `height`. Its buffers
    /// are allocated by its first present.
    fn new(
        context: &softbuffer::Context<Arc<dyn Window>>,
        window: Arc<dyn Window>,
        width: u32,
        height: u32,
        transparent: bool,
    ) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        let mut surface = softbuffer::Surface::new(context, window).unwrap();

        // Honor transparency only if the backend actually supports premultiplied
        // alpha; otherwise fall back to Opaque. Remember the *resolved* choice so
        // `present` can uphold the Opaque invariant regardless of request.
        let premultiplied = transparent && surface.supports_alpha_mode(AlphaMode::Premultiplied);
        let alpha_mode = if premultiplied {
            AlphaMode::Premultiplied
        } else {
            AlphaMode::Opaque
        };

        surface
            .configure(
                NonZeroU32::new(width).unwrap(),
                NonZeroU32::new(height).unwrap(),
                alpha_mode,
            )
            .unwrap();

        Self {
            surface,
            width,
            height,
            premultiplied,
        }
    }

    /// Resize the surface to new dimensions.
    fn resize(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.surface
            .resize(
                NonZeroU32::new(self.width).unwrap(),
                NonZeroU32::new(self.height).unwrap(),
            )
            .unwrap();
    }
}

impl PresentTarget for SoftSurface {
    /// Present RGBA8 pixels to the window.
    ///
    /// `pixels` is `width * height * 4` bytes, RGBA8, tightly packed. The
    /// softbuffer surface, however, may pad each row to a wider `byte_stride`
    /// (notably the Wayland backend), so `buffer.pixels()` is generally *longer*
    /// than the source and must be walked **row by row** via `pixel_rows()`. A
    /// flat 1:1 index copy would misalign every row past the first and shear the
    /// image on any padded-stride surface.
    ///
    /// Under `AlphaMode::Opaque` every presented pixel must be opaque (softbuffer
    /// asserts this in `present()`), so the padding columns beyond the source
    /// width — which the source never covers — are still written opaque here.
    /// Under premultiplied alpha the source alpha is passed through and the
    /// padding is left fully transparent.
    fn present(&mut self, pixels: &[u8], width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        if width != self.width || height != self.height {
            self.resize(width, height);
        }

        let mut buffer = self.surface.next_buffer().unwrap();

        let w = width as usize;
        let src_stride = w * 4;
        let premultiplied = self.premultiplied;

        // `pixel_rows()` yields exactly one row per surface row (length
        // `byte_stride / 4`); map surface row `y` back to the tightly-packed
        // source row at `y * width * 4`.
        for (y, row) in buffer.pixel_rows().enumerate() {
            let src_offset = y * src_stride;
            for (x, pixel) in row.iter_mut().enumerate() {
                let base = src_offset + x * 4;
                if x < w && base + 3 < pixels.len() {
                    let a = if premultiplied {
                        pixels[base + 3]
                    } else {
                        0xFF
                    };
                    *pixel = softbuffer::Pixel::new_rgba(
                        pixels[base],
                        pixels[base + 1],
                        pixels[base + 2],
                        a,
                    );
                } else if premultiplied {
                    // Row padding is outside the visible image — keep it clear.
                    *pixel = softbuffer::Pixel::new_rgba(0, 0, 0, 0);
                } else {
                    // Opaque surface: uncovered/padding pixels must still be opaque
                    // or `present()` asserts. Color is irrelevant (off-image).
                    *pixel = softbuffer::Pixel::new_rgba(0, 0, 0, 0xFF);
                }
            }
        }

        buffer.present().unwrap();
    }
}

#[cfg(test)]
#[path = "softbuffer_idle_trim_2843_tests.rs"]
mod idle_trim_2843_tests;
