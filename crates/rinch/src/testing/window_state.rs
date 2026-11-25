//! The main window as a test sees and drives it: its requests, props,
//! size, scale factor and focus.

use super::*;

impl TestApp {
    /// The requests the app's code made of its own window since the last
    /// call (`hide_current_window`, `show_current_window`,
    /// `prepare_current_window`, a size, a position, …), oldest first. Shows,
    /// hides, prepares, focus and size requests have also been applied (see
    /// [The window](super#the-window)); the rest are only recorded.
    pub fn take_window_requests(&mut self) -> Vec<WindowRequest> {
        std::mem::take(&mut self.requests)
    }

    /// The [`WindowProps`] the app was mounted with
    /// ([`TestAppBuilder::window_props`], the defaults otherwise), as given:
    /// [`Self::is_visible`] and [`Self::size`] follow what happened
    /// since, these do not.
    pub fn window_props(&self) -> &WindowProps {
        self.app
            .window_props
            .as_ref()
            .expect("a TestApp's app is given its props at the mount")
    }

    /// Whether the window is shown: from the props' `visible`, then each
    /// show and hide the app asked for.
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Whether the window has the keyboard: what
    /// [`window_focused`](crate::window::window_focused) answers the app.
    pub fn is_window_focused(&self) -> bool {
        self.focused
    }

    /// The actions the app asked the window for since the last call
    /// (a cursor change, a window drag, exit), oldest first, without the
    /// frame requests the harness itself serves.
    pub fn take_actions(&mut self) -> Vec<AppAction> {
        std::mem::take(&mut self.actions)
            .into_iter()
            .filter(|a| *a != AppAction::RequestRedraw)
            .collect()
    }

    /// The app being driven, for anything this harness does not wrap.
    pub fn app(&mut self) -> &mut RinchApp {
        &mut self.app
    }

    /// The window size, logical pixels.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The scale factor.
    pub fn scale_factor(&self) -> f64 {
        self.scale
    }

    /// Resize the window to `width`x`height` logical pixels, as a
    /// `SurfaceResized` does. [`window_size`](crate::window::window_size)
    /// follows.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        self.app
            .set_granted_size(self.size.0 as f32, self.size.1 as f32);
        self.send(PlatformEvent::Resized {
            width: self.size.0,
            height: self.size.1,
        });
    }

    /// Move the window to a display with scale factor `scale`, as a
    /// `ScaleFactorChanged` does. The logical size stays;
    /// [`scale_factor`](crate::window::scale_factor) follows.
    pub fn set_scale_factor(&mut self, scale: f64) {
        assert!(
            scale.is_finite() && scale > 0.0,
            "a scale factor is positive"
        );
        self.scale = scale;
        self.send(PlatformEvent::ScaleFactorChanged(scale));
        // The surface's new size, which the shell's `SurfaceResized` after a
        // `ScaleFactorChanged` publishes.
        self.app.set_surface_size(self.physical_size());
        self.app.publish_window_state(WindowFacts {
            physical_size: Some(self.physical_size()),
            ..Default::default()
        });
    }

    /// The window gaining (`true`) or losing keyboard focus.
    /// [`window_focused`](crate::window::window_focused) follows.
    ///
    /// A hidden window cannot gain the keyboard: `window_focus(true)` does
    /// nothing while [`Self::is_visible`] is false.
    pub fn window_focus(&mut self, focused: bool) {
        if focused && !self.visible {
            return;
        }
        self.focused = focused;
        self.send(PlatformEvent::WindowFocus(focused));
    }
}
