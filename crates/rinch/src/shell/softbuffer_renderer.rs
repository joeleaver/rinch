//! Software presentation layer using softbuffer.
//!
//! Receives RGBA8 pixel data and presents it to a winit window via
//! softbuffer's CPU-based surface, bypassing the GPU entirely.
//! Supports window transparency via `AlphaMode::Premultiplied` on
//! platforms that support it (Wayland, macOS, DRM/KMS).

use std::num::NonZeroU32;
use std::sync::Arc;

use softbuffer::AlphaMode;
use winit::window::Window;

/// A software renderer that presents RGBA8 pixels to a window via softbuffer.
pub struct SoftbufferRenderer {
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
    /// The frame the surface shows, so a redraw that changed no pixel
    /// presents nothing (#2698).
    shown: ShownFrame,
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
    /// Create a new software renderer for the given window.
    pub fn new(window: Arc<dyn Window>, width: u32, height: u32, transparent: bool) -> Self {
        let width = width.max(1);
        let height = height.max(1);

        let shown = ShownFrame::new(retains_contents(&*window));
        let context = softbuffer::Context::new(window.clone()).unwrap();
        let mut surface = softbuffer::Surface::new(&context, window).unwrap();

        // Honor transparency only if the backend actually supports premultiplied
        // alpha; otherwise fall back to Opaque. Remember the *resolved* choice so
        // `present_pixels` can uphold the Opaque invariant regardless of request.
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
            shown,
        }
    }

    /// Present the painter's frame `generation` ([`Self::present_pixels`]),
    /// unless the surface already shows it ([`ShownFrame`], #2698).
    /// Returns whether it presented.
    pub(crate) fn present_frame(
        &mut self,
        pixels: &[u8],
        width: u32,
        height: u32,
        generation: u64,
    ) -> bool {
        // A size the surface does not have is always presented: the frame
        // of a resize moved the generation on, so this only guards it.
        if width != self.width || height != self.height {
            self.shown = ShownFrame::new(self.shown.retains);
        }
        if !self.shown.take(generation) {
            return false;
        }
        self.present_pixels(pixels, width, height);
        true
    }

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
    pub fn present_pixels(&mut self, pixels: &[u8], width: u32, height: u32) {
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

    /// Resize the surface to new dimensions.
    pub fn resize(&mut self, width: u32, height: u32) {
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
