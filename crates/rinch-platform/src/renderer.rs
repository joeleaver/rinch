//! Platform renderer abstraction.

use peniko::Color;
use vello::Scene;

/// Abstraction over the GPU rendering pipeline.
///
/// Each platform backend provides its own implementation:
/// - Desktop: wgpu with native backends (Vulkan, DX12, Metal)
/// - Web: wgpu with WebGPU or WebGL backend
///
/// The renderer receives a `vello::Scene` (built by rinch-dom's paint system)
/// and presents it to the window surface. The scene-building is entirely
/// platform-agnostic -- only surface creation and presentation differ.
pub trait PlatformRenderer {
    /// Resize the rendering surface.
    fn resize(&mut self, width: u32, height: u32);

    /// Render a Vello scene to the window surface.
    ///
    /// The `scene` is built by `rinch_dom::paint::paint_document()`.
    /// The `base_color` is the background clear color (transparent for
    /// transparent windows, white otherwise).
    fn render_scene(
        &mut self,
        scene: &Scene,
        width: u32,
        height: u32,
        base_color: Color,
    ) -> Result<(), RenderError>;

    /// Capture a screenshot of the current render as RGBA bytes.
    ///
    /// Used by the debug protocol for remote inspection.
    /// Returns (width, height, rgba_bytes).
    fn capture_screenshot(&self) -> Result<(u32, u32, Vec<u8>), RenderError>;
}

/// Errors that can occur during rendering.
#[derive(Debug, Clone)]
pub enum RenderError {
    /// The surface texture could not be acquired.
    SurfaceLost,
    /// The GPU device was lost.
    DeviceLost,
    /// An internal rendering error.
    Internal(String),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SurfaceLost => write!(f, "surface lost"),
            Self::DeviceLost => write!(f, "device lost"),
            Self::Internal(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for RenderError {}
