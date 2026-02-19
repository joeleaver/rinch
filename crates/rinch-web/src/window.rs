//! Web window implementation wrapping an HTML canvas element.

use rinch_platform::PlatformWindow;
use web_sys::HtmlCanvasElement;

/// Web window backed by an HTML canvas element.
pub struct WebWindow {
    canvas: HtmlCanvasElement,
}

impl WebWindow {
    /// Create a new web window from an HTML canvas element.
    pub fn new(canvas: HtmlCanvasElement) -> Self {
        Self { canvas }
    }

    /// Get a reference to the underlying canvas element.
    pub fn canvas(&self) -> &HtmlCanvasElement {
        &self.canvas
    }
}

impl PlatformWindow for WebWindow {
    fn inner_size(&self) -> (u32, u32) {
        (self.canvas.width(), self.canvas.height())
    }

    fn scale_factor(&self) -> f64 {
        web_sys::window()
            .map(|w| w.device_pixel_ratio())
            .unwrap_or(1.0)
    }

    fn request_redraw(&self) {
        // In web, redraws are requested via requestAnimationFrame.
        // This is a stub - the event loop handles this.
    }

    fn set_minimized(&self, _minimized: bool) {
        // No-op on web - browsers don't expose window minimization.
    }

    fn set_visible(&self, _visible: bool) {
        // No-op on web - the canvas is always visible.
    }

    fn set_maximized(&self, _maximized: bool) {
        // Could toggle fullscreen in the future.
        // TODO: Implement fullscreen API integration.
    }

    fn is_maximized(&self) -> bool {
        // TODO: Check document.fullscreenElement
        false
    }

    fn drag_window(&self) -> Result<(), String> {
        // No-op on web - browser windows can't be dragged programmatically.
        Err("Window dragging not supported on web".into())
    }

    fn set_title(&self, title: &str) {
        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            doc.set_title(title);
        }
    }
}
