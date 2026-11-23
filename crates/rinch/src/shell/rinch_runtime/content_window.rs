//! Windows sized by their content ([`WindowProps::size_to_content`], #2552):
//! the shell's half. The document is in the content-sized mode
//! (`app/content_size.rs`), which measures; the shell asks the window for
//! the size, hands each size the window is given back to the document, and
//! starts a new negotiation for each new surface.
//!
//! - **Made** ([`app_for_window`]): in the mode when the props ask and an
//!   axis is left to size ([`content_sizing::content_axes`]).
//! - **Mounted** before its OS window at its largest size
//!   (`mount_document_before_window`), so the window is made at its content's
//!   size: an `open_window` window always, a main window when it is sized by
//!   its content (`create_window`).
//! - **Each new surface** (its creation, a show, a re-creation for a new
//!   output) starts free at its largest size, recomputed for the output it is
//!   on ([`ShellWindow::start_surface_negotiation`]).
//! - **Each change of the content** asks the window for its new size
//!   ([`RinchRuntime::follow_content_size`]); each size the window is given
//!   (`SurfaceResized`) is the document's grant.
//!
//! [`WindowProps::size_to_content`]: rinch_core::element::WindowProps::size_to_content

use super::*;
use crate::shell::content_sizing::{self, AxisSizing};
use crate::shell::layer_shell::LayerSpec;
use rinch_core::element::{LayerSurface, WindowProps};

/// The app for a window with `props`: in the content-sized mode when they ask
/// for it and an axis is left to size, else an ordinary one. The largest size
/// here is a first guess (the outputs may not be listed yet); the mount and
/// every new surface set it ([`ShellWindow::content_max`]).
pub(crate) fn app_for_window(
    component: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static,
    props: &WindowProps,
) -> RinchApp {
    let Some(sizing) = props.size_to_content else {
        return RinchApp::new(component);
    };
    let Some(axes) = content_sizing::content_axes(props.wayland_layer.as_ref()) else {
        content_sizing::warn_nothing_to_size();
        return RinchApp::new(component);
    };
    let max = content_sizing::content_max(
        &sizing,
        props.wayland_layer.as_ref(),
        None,
        (props.width, props.height),
    );
    RinchApp::new_content_sized(
        component,
        max,
        (
            axes.width == AxisSizing::Fill,
            axes.height == AxisSizing::Fill,
        ),
    )
}

/// Whether the compositor offers the layer shell: as the shell's event loop
/// published it (`window_roles`), or, on a thread where `rinch::testing`
/// drives the windows, as the harness stands in a Wayland compositor with
/// layer shell (#2836). The harness publishes no window roles (they are the
/// process's, and its tests run in parallel), so without this a child it
/// opens would be sized as if it were no layer surface, against the first
/// output rather than its own.
fn layer_shell_offered() -> bool {
    if crate::window_roles::layer_shell_available() {
        return true;
    }
    #[cfg(feature = "testing")]
    {
        crate::testing::stands_in_layer_shell()
    }
    #[cfg(not(feature = "testing"))]
    {
        false
    }
}

/// The layer surface a window with `props` is made as, when the compositor
/// offers the layer shell and the props ask for one: at `position` (a layer
/// surface's margins, once it has some) on the output `output` overrides,
/// if any. What a window's content sizing reads of its surface.
fn content_layer_surface(
    props: &WindowProps,
    position: Option<(f64, f64)>,
    output: Option<Option<String>>,
) -> Option<LayerSurface> {
    let layer = layer_shell_offered()
        && (props.wayland_layer.is_some()
            || props.wayland_layer_shell.unwrap_or(props.non_activating));
    layer.then(|| LayerSpec::for_window(props, position, output, "").surface)
}

/// The largest size, logical px, of a window whose document is `app`, at
/// `position` on the output `output` overrides (see
/// [`content_layer_surface`]): from its props, its layer surface and the
/// output it is on ([`content_sizing::content_max`]). `None` for a window
/// whose document is not content-sized: one that does not ask, and one
/// whose compositor sizes both axes ([`app_for_window`]). The shell's every
/// window and `rinch::testing`'s main window are sized by it (#2836).
pub(crate) fn content_max_of(
    app: &RinchApp,
    position: Option<(f64, f64)>,
    output: Option<Option<String>>,
) -> Option<(f32, f32)> {
    if !app.is_content_sized() {
        return None;
    }
    let props = app.window_props.as_ref()?;
    let sizing = props.size_to_content?;
    let surface = content_layer_surface(props, position, output);
    let outputs = rinch_core::reactive::untracked(crate::outputs::outputs);
    let output = content_sizing::sizing_output(surface.as_ref(), props, &outputs);
    Some(content_sizing::content_max(
        &sizing,
        surface.as_ref(),
        output,
        (props.width, props.height),
    ))
}

impl ShellWindow {
    /// A content-sized window's largest size now, in its document's CSS px:
    /// [`content_max_of`] (the display's logical px, at its layer surface's
    /// position and on the output chosen for it) divided by the app's zoom,
    /// so a zoomed window asks for no more of the output than an unzoomed
    /// one. `None` for any other window.
    pub(super) fn content_max(&self) -> Option<(f32, f32)> {
        let (width, height) = content_max_of(
            &self.app,
            self.layer.and_then(|l| l.position),
            self.output_override.clone(),
        )?;
        let zoom = crate::zoom::current() as f32;
        Some(((width / zoom).max(1.0), (height / zoom).max(1.0)))
    }

    /// The scale a second window's document is mounted at: the main window's
    /// (`main`), except that a window sized by its content is measured at the
    /// scale its surface will start at (#2652), so its first buffer is its
    /// content's size there; with no outputs listed yet, that is the main
    /// window's too (#2886).
    pub(super) fn mount_scale(&self, main: f64) -> f64 {
        if self.app.is_content_sized() {
            crate::zoom::apply(self.initial_surface_scale_or(true, main))
        } else {
            main
        }
    }

    /// Give a content-sized window's document the scale its new surface
    /// starts at (#2652, `initial_surface_scale`), before it is measured for
    /// that surface: its creation (`first_appearance`) or a show. So the
    /// first buffer is the content's size at that scale, and no second
    /// request follows when the compositor confirms it. Nothing for any
    /// other window, nor when the scale is already that.
    pub(super) fn start_at_surface_scale(&mut self, first_appearance: bool) {
        if !self.app.is_content_sized() {
            return;
        }
        let scale = crate::zoom::apply(self.initial_surface_scale(first_appearance));
        if scale != self.app.device_pixel_ratio {
            self.app.set_device_pixel_ratio(scale);
        }
    }

    /// Start a new surface's negotiation for a content-sized window: free at
    /// the largest size for where it goes, what is waiting laid out (a change
    /// of the content made while it was hidden, a new scale), and the
    /// content's size there, which this answers and remembers as asked: the
    /// size to make (or prepare) the surface at. `None` for any other window.
    pub(super) fn start_surface_negotiation(&mut self) -> Option<(u32, u32)> {
        let max = self.content_max()?;
        self.app.restart_content_limit(max);
        self.app.content_size()
    }

    /// A new negotiation on the live surface, its layer configuration having
    /// changed: free again at the new largest size, and the content's size
    /// there staged with the changes when the drain sends them
    /// ([`Self::stage_content_size_for_layer_turn`]). Nothing for any other
    /// window.
    pub(super) fn restart_content_size(&mut self) {
        if let Some(max) = self.content_max() {
            self.app.restart_content_limit(max);
            self.app.remeasure_content();
        }
    }

    /// A content-sized layer surface whose configuration this turn changed
    /// (staged layer changes, [`Self::restart_content_size`]): its content's
    /// size in the new configuration, staged with the changes, so the
    /// drain's one request carries both and the compositor sees the new
    /// configuration at its new size in one commit (#2829). Asked by
    /// [`RinchRuntime::follow_content_size`] after the drain instead, it was
    /// a commit of its own (the fork commits a layer surface's size request
    /// when it is made). Nothing for any other window, nor for a turn that
    /// changed no layer state.
    pub(super) fn stage_content_size_for_layer_turn(&mut self) {
        if self.pending_layer.is_empty() || !self.app.is_content_sized() {
            return;
        }
        let Some(viewport) = self.window.as_ref().map(|w| w.logical_size()) else {
            return;
        };
        if let Some((width, height)) = self.content_size_to_ask(viewport) {
            // CSS pixels; winit's logical size is the display's, which the
            // app's zoom makes larger (#2861), as `follow_content_size` asks.
            let zoom = crate::zoom::current();
            self.stage_layer_size(Size::Logical(LogicalSize::new(
                width as f64 * zoom,
                height as f64 * zoom,
            )));
        }
    }

    /// The outputs changed: a new negotiation only when this window's own
    /// largest size changed (its output, that output's size, or its margins),
    /// not on a change of another output, so a limited dimension stays
    /// limited. Nothing for any other window.
    pub(super) fn follow_outputs(&mut self) {
        let Some(max) = self.content_max() else {
            return;
        };
        if self.app.content_largest() != Some(max) {
            self.app.restart_content_limit(max);
            self.app.remeasure_content();
        }
    }

    /// The size to ask the window for after a change of the content (or of
    /// its scale), laid out first at `viewport` (the window's logical size),
    /// so it is measured from what is waiting, not from the layout before:
    /// [`RinchApp::content_size_changed`] after that layout.
    pub(super) fn content_size_to_ask(&mut self, viewport: (u32, u32)) -> Option<(u32, u32)> {
        if self.app.has_pending_layout() {
            self.app
                .resolve_and_repaint(viewport.0 as f32, viewport.1 as f32);
        }
        self.app.content_size_changed()
    }
}

impl RinchRuntime {
    /// The outputs changed: each content-sized window whose own largest size
    /// changed starts a new negotiation ([`ShellWindow::follow_outputs`]).
    pub(super) fn follow_outputs_in_every_window(&mut self) {
        for role in self.roles() {
            self.w(role).follow_outputs();
        }
    }

    /// Ask `role`'s window for its content's size when a change of the
    /// content (or of its scale) laid out to another one, before the frame
    /// or at the end of a turn. What is waiting is laid out first, so the
    /// size is measured from it and a frame is drawn at that size. A window
    /// that applies the size at once (a layer surface, a toplevel with no
    /// state) answers with it, and that is handled as the `SurfaceResized`
    /// it is; any other answers later, in a configure. A hidden window asks
    /// nothing: its show measures.
    pub(super) fn follow_content_size(
        &mut self,
        role: WindowRole,
        event_loop: &dyn ActiveEventLoop,
    ) {
        let w = self.w(role);
        if !w.app.is_content_sized() {
            return;
        }
        let Some(viewport) = w.window.as_ref().map(|w| w.logical_size()) else {
            return;
        };
        let Some((width, height)) = w.content_size_to_ask(viewport) else {
            return;
        };
        // The content's size is in CSS pixels; winit's logical size is the
        // display's, which the app's zoom makes larger.
        let zoom = crate::zoom::current();
        let size = Size::Logical(LogicalSize::new(width as f64 * zoom, height as f64 * zoom));
        let Some(physical) = w
            .window
            .as_ref()
            .and_then(|w| w.raw().request_surface_size(size))
        else {
            return;
        };
        self.window_event_for(role, event_loop, WindowEvent::SurfaceResized(physical));
    }
}

#[cfg(test)]
#[path = "content_window_tests.rs"]
mod tests;
