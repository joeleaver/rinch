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
//!   on ([`ShellWindow::content_surface_size`]).
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
pub(super) fn app_for_window(
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
    content_sizing::warn_ignored_size(props);
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

impl ShellWindow {
    /// The layer surface this window is made as, when the compositor offers
    /// the layer shell and the props ask for one.
    fn content_layer_surface(&self, props: &WindowProps) -> Option<LayerSurface> {
        let layer = crate::window_roles::layer_shell_available()
            && (props.wayland_layer.is_some()
                || props.wayland_layer_shell.unwrap_or(props.non_activating));
        let position = self.layer.and_then(|l| l.position);
        layer.then(|| {
            LayerSpec::for_window(props, position, self.output_override.clone(), "").surface
        })
    }

    /// A content-sized window's largest size now, logical px: from its props,
    /// its layer surface and the output it is on (see
    /// [`content_sizing::content_max`]). `None` for any other window.
    pub(super) fn content_max(&self) -> Option<(f32, f32)> {
        if !self.app.is_content_sized() {
            return None;
        }
        let props = self.app.window_props.as_ref()?;
        let sizing = props.size_to_content?;
        let surface = self.content_layer_surface(props);
        let outputs = rinch_core::reactive::untracked(crate::outputs::outputs);
        let output = content_sizing::sizing_output(surface.as_ref(), props, &outputs);
        Some(content_sizing::content_max(
            &sizing,
            surface.as_ref(),
            output,
            (props.width, props.height),
        ))
    }

    /// The size to make a new surface at, for a content-sized window: a new
    /// negotiation, free at the largest size for where it goes, and the
    /// content's size there. `None` for any other window.
    pub(super) fn content_surface_size(&mut self) -> Option<(u32, u32)> {
        let max = self.content_max()?;
        self.app.restart_content_limit(max);
        self.app.content_size()
    }

    /// A new negotiation on the live surface (its layer configuration or its
    /// output changed): free again at the new largest size, and the
    /// content's size there asked for at the next turn's end
    /// ([`RinchRuntime::follow_content_size`]). Nothing for any other window.
    pub(super) fn restart_content_size(&mut self) {
        if let Some(max) = self.content_max() {
            self.app.restart_content_limit(max);
            self.app.remeasure_content();
        }
    }
}

impl RinchRuntime {
    /// [`Self::follow_content_size`] before a frame: the layout a change of
    /// the content waits for is done first, so the frame is drawn at the size
    /// it asks for.
    pub(super) fn follow_content_size_before_paint(
        &mut self,
        role: WindowRole,
        event_loop: &dyn ActiveEventLoop,
    ) {
        let w = self.w(role);
        if !w.app.is_content_sized() || w.window.is_none() {
            return;
        }
        if w.app.has_pending_layout() {
            let (width, height) = w.window.as_ref().map_or((1, 1), |w| w.logical_size());
            w.app.resolve_and_repaint(width as f32, height as f32);
        }
        self.follow_content_size(role, event_loop);
    }

    /// Ask `role`'s window for its content's size when a change of the
    /// content laid out to another one. A window that applies the size at
    /// once (a layer surface, a toplevel with no state) answers with it, and
    /// that is handled as the `SurfaceResized` it is; any other answers later,
    /// in a configure. A hidden window asks nothing: its show measures.
    pub(super) fn follow_content_size(
        &mut self,
        role: WindowRole,
        event_loop: &dyn ActiveEventLoop,
    ) {
        let w = self.w(role);
        if w.window.is_none() {
            return;
        }
        let Some((width, height)) = w.app.content_size_changed() else {
            return;
        };
        let size = Size::Logical(LogicalSize::new(width as f64, height as f64));
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
