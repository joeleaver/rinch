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

    /// Set video layers to composite underneath the Vello UI.
    ///
    /// Each layer contains a video texture and a viewport rect (in logical
    /// pixels). When layers are present, `render_scene` composites them
    /// underneath the UI instead of doing a simple texture copy.
    ///
    /// Default implementation does nothing (no video support).
    fn set_video_layers(&mut self, _layers: Vec<VideoLayer>) {}

    /// Check whether there are active video layers to composite.
    fn has_video_layers(&self) -> bool {
        false
    }
}

/// A video layer to composite underneath the Vello UI.
pub struct VideoLayer {
    /// RGBA pixel data of the decoded video frame.
    pub pixels: Vec<u8>,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Viewport rectangle in logical pixels: (x, y, w, h).
    pub viewport: (f32, f32, f32, f32),
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
