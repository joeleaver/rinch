//! Web renderer stub.
//!
//! A full implementation would use wgpu-on-WebGPU or a canvas 2D fallback.
//! For now this is a compilation stub that returns errors.

use rinch_platform::{PlatformRenderer, RenderError};
use vello::Scene;
use peniko::Color;

/// Stub web renderer.
///
/// A real implementation would:
/// - Initialize wgpu with the WebGPU backend (when available)
/// - Fall back to WebGL or Canvas 2D for older browsers
/// - Handle surface creation from an HTML canvas
/// - Render vello scenes using the appropriate backend
///
/// For now this is a minimal stub to enable compilation for wasm32-unknown-unknown.
pub struct WebRenderer {
    width: u32,
    height: u32,
}

impl WebRenderer {
    /// Create a new web renderer with the given dimensions.
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

impl PlatformRenderer for WebRenderer {
    fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    fn render_scene(
        &mut self,
        _scene: &Scene,
        _width: u32,
        _height: u32,
        _base_color: Color,
    ) -> Result<(), RenderError> {
        // TODO: Implement actual rendering
        // Options:
        // 1. wgpu with WebGPU backend (modern browsers)
        // 2. wgpu with WebGL backend (fallback)
        // 3. Canvas 2D API (ultimate fallback, limited features)
        Err(RenderError::Internal("Web renderer not yet implemented".into()))
    }

    fn capture_screenshot(&self) -> Result<(u32, u32, Vec<u8>), RenderError> {
        // TODO: Implement screenshot capture via canvas.toDataURL() or similar
        Err(RenderError::Internal("Screenshots not supported on web yet".into()))
    }
}
