//! Software presentation layer using softbuffer.
//!
//! Receives RGBA8 pixel data and presents it to a winit window via
//! softbuffer's CPU-based surface, bypassing the GPU entirely.

use std::num::NonZeroU32;
use std::sync::Arc;

use winit::window::Window;

/// A software renderer that presents RGBA8 pixels to a window via softbuffer.
pub struct SoftbufferRenderer {
    surface: softbuffer::Surface<Arc<dyn Window>, Arc<dyn Window>>,
    width: u32,
    height: u32,
}

impl SoftbufferRenderer {
    /// Create a new software renderer for the given window.
    pub fn new(window: Arc<dyn Window>, width: u32, height: u32) -> Self {
        let width = width.max(1);
        let height = height.max(1);

        let context = softbuffer::Context::new(window.clone()).unwrap();
        let mut surface = softbuffer::Surface::new(&context, window).unwrap();
        surface
            .resize(
                NonZeroU32::new(width).unwrap(),
                NonZeroU32::new(height).unwrap(),
            )
            .unwrap();

        Self {
            surface,
            width,
            height,
        }
    }

    /// Present RGBA8 pixels to the window.
    ///
    /// `pixels` must be `width * height * 4` bytes in RGBA8 format.
    pub fn present_pixels(&mut self, pixels: &[u8], width: u32, height: u32) {
        if width != self.width || height != self.height {
            self.resize(width, height);
        }

        let mut buffer = self.surface.buffer_mut().unwrap();
        // Convert RGBA8 to softbuffer's native 0xAARRGGBB format.
        // Including the alpha channel enables window transparency on platforms
        // that support it (X11 with 32-bit ARGB visual, Wayland with ARGB8888).
        for (i, pixel) in buffer.iter_mut().enumerate() {
            let src = i * 4;
            if src + 3 < pixels.len() {
                let r = pixels[src] as u32;
                let g = pixels[src + 1] as u32;
                let b = pixels[src + 2] as u32;
                let a = pixels[src + 3] as u32;
                *pixel = (a << 24) | (r << 16) | (g << 8) | b;
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
