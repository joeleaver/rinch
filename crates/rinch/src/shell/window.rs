//! Desktop window wrapper around winit.
//!
//! Separated from the GPU renderer so it can be used by both the
//! software (softbuffer) and GPU (wgpu/vello) rendering paths.

use std::sync::Arc;

use rinch_platform::PlatformWindow;
use winit::window::Window;

/// Desktop window backed by winit.
pub struct WinitWindow {
    pub(crate) window: Arc<dyn Window>,
}

impl WinitWindow {
    pub fn new(window: Box<dyn Window>) -> Self {
        Self {
            window: Arc::from(window),
        }
    }

    /// Get the raw winit window reference.
    pub fn raw(&self) -> &dyn Window {
        &*self.window
    }
}

impl PlatformWindow for WinitWindow {
    fn inner_size(&self) -> (u32, u32) {
        let s = self.window.surface_size();
        (s.width, s.height)
    }

    fn scale_factor(&self) -> f64 {
        self.window.scale_factor()
    }

    fn request_redraw(&self) {
        self.window.request_redraw();
    }

    fn set_minimized(&self, minimized: bool) {
        self.window.set_minimized(minimized);
    }

    fn set_maximized(&self, maximized: bool) {
        self.window.set_maximized(maximized);
    }

    fn set_visible(&self, visible: bool) {
        self.window.set_visible(visible);
    }

    fn is_maximized(&self) -> bool {
        self.window.is_maximized()
    }

    fn drag_window(&self) -> Result<(), String> {
        self.window
            .drag_window()
            .map_err(|e| format!("drag_window failed: {e}"))
    }

    fn drag_resize_window(&self, direction: rinch_platform::ResizeDirection) -> Result<(), String> {
        use rinch_platform::ResizeDirection as RD;
        use winit::window::ResizeDirection as WRD;
        let wd = match direction {
            RD::North => WRD::North,
            RD::South => WRD::South,
            RD::East => WRD::East,
            RD::West => WRD::West,
            RD::NorthEast => WRD::NorthEast,
            RD::NorthWest => WRD::NorthWest,
            RD::SouthEast => WRD::SouthEast,
            RD::SouthWest => WRD::SouthWest,
        };
        self.window
            .drag_resize_window(wd)
            .map_err(|e| format!("drag_resize_window failed: {e}"))
    }

    fn set_title(&self, title: &str) {
        self.window.set_title(title);
    }
}
