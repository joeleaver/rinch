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
    transparent: bool,
}

impl SoftbufferRenderer {
    /// Create a new software renderer for the given window.
    pub fn new(window: Arc<dyn Window>, width: u32, height: u32, transparent: bool) -> Self {
        let width = width.max(1);
        let height = height.max(1);

        let context = softbuffer::Context::new(window.clone()).unwrap();
        let mut surface = softbuffer::Surface::new(&context, window).unwrap();

        let alpha_mode = if transparent && surface.supports_alpha_mode(AlphaMode::Premultiplied) {
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
            transparent,
        }
    }

    /// Present RGBA8 pixels to the window.
    ///
    /// `pixels` must be `width * height * 4` bytes in RGBA8 format.
    /// When transparency is enabled, the alpha channel is passed through
    /// (premultiplied, matching tiny-skia's internal format).
    pub fn present_pixels(&mut self, pixels: &[u8], width: u32, height: u32) {
        if width != self.width || height != self.height {
            self.resize(width, height);
        }

        let mut buffer = self.surface.next_buffer().unwrap();
        let buf_pixels = buffer.pixels();

        for (i, pixel) in buf_pixels.iter_mut().enumerate() {
            let src = i * 4;
            if src + 3 < pixels.len() {
                pixel.r = pixels[src];
                pixel.g = pixels[src + 1];
                pixel.b = pixels[src + 2];
                pixel.a = if self.transparent {
                    pixels[src + 3]
                } else {
                    0xFF
                };
            } else if !self.transparent {
                // Source shorter than the surface (a size/scale mismatch, e.g.
                // fractional HiDPI): the unwritten pixels must still be opaque or
                // softbuffer's `AlphaMode::Opaque` present() assertion fails.
                pixel.a = 0xFF;
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
