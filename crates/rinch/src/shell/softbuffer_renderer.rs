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
}

impl SoftbufferRenderer {
    /// Create a new software renderer for the given window.
    pub fn new(window: Arc<dyn Window>, width: u32, height: u32, transparent: bool) -> Self {
        let width = width.max(1);
        let height = height.max(1);

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
        }
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
