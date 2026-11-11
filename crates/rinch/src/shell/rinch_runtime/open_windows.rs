//! The windows `open_window` opens (#2416): making them, closing them, and
//! the exit rule ([`crate::ExitWhen`]).

use super::*;
use crate::window_handle;

impl RinchRuntime {
    /// Make the window `open_window` opened as `id`, if it is still waiting
    /// (closed before it was made, it is not).
    pub(super) fn open_pending_window(&mut self, id: u64, event_loop: &dyn ActiveEventLoop) {
        let Some(pending) = window_handle::take_pending(id) else {
            return;
        };
        let mut window = Self::new_other_window(pending, &self.fonts);
        let scale = self.main.scale_factor();
        window_handle::mounting(id, || window.mount_before_window(scale));
        window_handle::note_mounted(id, window.app.doc_key());
        window.create_window(event_loop, None);
        self.others.push(window);
        self.about_to_wait_for(WindowRole::Other(id), event_loop);
    }

    /// A second window's state before its OS window is made: its document
    /// keeps its own stores (falling back to the app's), publishes its own
    /// facts, and is made with the app's fonts.
    fn new_other_window(
        pending: window_handle::Pending,
        fonts: &[crate::font::AppFont],
    ) -> ShellWindow {
        let window_handle::Pending { id, props, content } = pending;
        if props.menu_in_titlebar {
            warn_menu_in_titlebar_once();
        }
        let mut app = RinchApp::new(content);
        app.register_app_fonts(fonts);
        app.set_publishes_own_window_state();
        app.scope_context_to_doc = true;
        let (title, width, height) = (props.title.clone(), props.width, props.height);
        app.window_props = Some(props);
        let mut window = ShellWindow::new(app, &title, width, height);
        window.handle = id;
        window
    }

    /// `role`'s window is to close, with no veto asked: `WindowHandle::close`,
    /// or its own close agreed to. Closed once the handler running returns
    /// ([`Self::settle_closes`]), so no handler finds its window gone.
    /// Answers whether the event loop ends at once, which it does for the
    /// main window of an app that exits with it.
    pub(super) fn close_soon(&mut self, role: WindowRole) -> bool {
        match role {
            WindowRole::Main if self.exit_when == crate::ExitWhen::MainWindowCloses => true,
            WindowRole::Main => {
                self.main_closing = true;
                false
            }
            WindowRole::DevTools => {
                self.devtools_closing = true;
                false
            }
            WindowRole::Other(id) => {
                if !self.others_closing.contains(&id) {
                    self.others_closing.push(id);
                }
                false
            }
        }
    }

    /// Close the windows asked to close, and end the event loop when the exit
    /// rule says so. At the end of every handler.
    pub(super) fn settle_closes(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.settle_closes_now() {
            event_loop.exit();
        }
    }

    /// [`Self::settle_closes`] without the event loop: answers whether it
    /// ends.
    pub(super) fn settle_closes_now(&mut self) -> bool {
        self.settle_devtools_close();
        let closing = std::mem::take(&mut self.others_closing);
        let main = std::mem::take(&mut self.main_closing);
        if closing.is_empty() && !main {
            return false;
        }
        for id in closing {
            self.close_other(id);
        }
        if main {
            self.close_main();
        }
        self.exits_after_close()
    }

    /// Whether the event loop ends now that a window closed.
    pub(super) fn exits_after_close(&self) -> bool {
        match self.exit_when {
            crate::ExitWhen::MainWindowCloses => self.main_closed,
            crate::ExitWhen::LastWindowCloses => self.main_closed && self.others.is_empty(),
            crate::ExitWhen::Explicitly => false,
        }
    }

    /// Close window `id` now: its published facts go, its document is
    /// disposed (its effects, handlers, stores and the windows it owns), its
    /// OS window and renderer are dropped, and then its handle says it is
    /// closed. One closed before it was made only leaves the pending list.
    pub(super) fn close_other(&mut self, id: u64) {
        if let Some(at) = self.others.iter().position(|w| w.handle == id) {
            let window = self.others.remove(at);
            window.close();
        }
        window_handle::note_closed(id);
    }

    /// Close the main window of an app that runs on without it: its OS
    /// window goes as a hide does, then its document is disposed.
    fn close_main(&mut self) {
        if self.main_closed {
            return;
        }
        self.main.hide_window(self.native_menu.as_ref());
        self.main.dispose_document();
        self.main_closed = true;
        window_handle::note_closed(self.main_handle);
    }

    /// Hide `role`'s window (its OS window goes; its document lives), the
    /// native menu bar with it for the main window.
    pub(super) fn hide_role(&mut self, role: WindowRole) {
        match role {
            WindowRole::Main => self.hide_window(),
            other => self.w(other).hide_window(None),
        }
    }

    /// Show `role`'s window again, made over its document.
    pub(super) fn show_role(&mut self, role: WindowRole, event_loop: &dyn ActiveEventLoop) {
        match role {
            WindowRole::Main => self.show_window(event_loop),
            other => self.w(other).show_window(event_loop, None),
        }
    }

    /// Mount a windowless app's root (`App::windowless`, #2416 stage 4c): in
    /// a document with no OS window, once, at the props' size and scale 1.
    /// It draws nothing; it holds the components that open the windows.
    pub(super) fn mount_windowless_root(&mut self) {
        if self.main.app.doc.is_some() {
            return;
        }
        self.main.app.set_device_pixel_ratio(1.0);
        let (width, height) = (
            self.main.width.max(1) as f32,
            self.main.height.max(1) as f32,
        );
        self.main.app.mount_component(width, height);
        self.warn_windowless_root_content();
    }

    /// Content in a windowless root shows nowhere: warned of once, when its
    /// document first holds some (visible text, an image, a control).
    pub(super) fn warn_windowless_root_content(&mut self) {
        if self.windowless_root_warned {
            return;
        }
        let Some(doc) = &self.main.app.doc else {
            return;
        };
        let painted = doc.borrow().tree.nodes.iter().find_map(|(_, node)| {
            if node.parent.is_none() {
                return None;
            }
            match &node.kind {
                rinch_dom::node::NodeKind::Text(t) if !t.content.trim().is_empty() => {
                    Some(format!("the text \"{}\"", t.content.trim()))
                }
                rinch_dom::node::NodeKind::Element(el)
                    if matches!(
                        el.tag.as_str(),
                        "img"
                            | "svg"
                            | "canvas"
                            | "video"
                            | "input"
                            | "textarea"
                            | "select"
                            | "button"
                    ) =>
                {
                    Some(format!("a <{}>", el.tag))
                }
                _ => None,
            }
        });
        if let Some(what) = painted {
            self.windowless_root_warned = true;
            tracing::warn!(
                "{what} is in the windowless root (App::windowless), which has no window and \
                 shows nothing: put it in a Window's content"
            );
        }
    }

    /// Close every other window (the runtime is shutting down).
    pub(super) fn close_all_others(&mut self) {
        let ids: Vec<u64> = self.others.iter().map(|w| w.handle).collect();
        for id in ids {
            self.close_other(id);
        }
    }

    /// Each other window's turn at the end of a batch, as the main window's.
    pub(super) fn about_to_wait_others(&mut self, event_loop: &dyn ActiveEventLoop) {
        let ids: Vec<u64> = self.others.iter().map(|w| w.handle).collect();
        for id in ids {
            if self.others.iter().any(|w| w.handle == id) {
                self.about_to_wait_for(WindowRole::Other(id), event_loop);
            }
        }
    }
}

impl ShellWindow {
    /// Mount a second window's document before its OS window exists, at its
    /// props' size and `scale` (the main window's, the best guess before its
    /// own is known): its content is built, laid out and its code runs from
    /// here, and `create_window` gives it the real scale and size. A window
    /// sized by its content (a popup, #2552) is measured this way.
    pub(super) fn mount_before_window(&mut self, scale: f64) {
        self.app.set_device_pixel_ratio(scale);
        let (width, height) = (self.width.max(1) as f32, self.height.max(1) as f32);
        self.app.mount_component(width, height);
        self.facts_key = Some(self.app.doc_key());
    }

    /// Paint a second window: the layer changes staged so far go before the
    /// frame, in their one commit (#2464), then the frame, on the GPU when it
    /// has a renderer.
    pub(super) fn paint_window(&mut self) -> Result<(), String> {
        self.flush_layer_changes();
        #[cfg(feature = "gpu")]
        if self.renderer.is_some() {
            return self.paint_gpu();
        }
        self.paint_software()
    }

    /// Close this window: what it published goes, then its document, then
    /// its renderer and OS window.
    pub(super) fn close(mut self) {
        self.forget_published();
        self.dispose_document();
        #[cfg(feature = "gpu")]
        drop(self.renderer.take());
        drop(self.soft_renderer.take());
        drop(self.window.take());
    }

    /// What this window published under its own key (#2416): its facts,
    /// geometry, surface kind, pointer-lock host, token requests and render
    /// surfaces. The main window publishes under the thread's, which stay.
    pub(super) fn forget_published(&self) {
        let Some(doc) = self.facts_key else {
            return;
        };
        crate::pointer_lock::set_pointer_lock_host_for(Some(doc), None);
        crate::shell::activation_tokens::window_gone(Some(doc));
        crate::window::forget_window(doc);
        window_geometry::forget(Some(doc));
        crate::shell::layer_shell::forget_surface_kind(Some(doc));
        crate::render_surface::forget_window_surfaces(Some(doc));
    }

    /// Dispose this window's document: the component, its scopes (their
    /// effects, handlers, stores and owned windows), then the document.
    pub(super) fn dispose_document(&mut self) {
        drop(self.app.component.take());
        drop(self.app._render_scope.take());
        drop(self.app.doc.take());
    }
}

/// `menu_in_titlebar` on a second window: the native menu bar is the main
/// window's. Once per process.
fn warn_menu_in_titlebar_once() {
    static WARNED: std::sync::Once = std::sync::Once::new();
    WARNED.call_once(|| {
        tracing::warn!(
            "WindowProps::menu_in_titlebar on a window opened with open_window is ignored: \
             the native menu bar is the main window's (its shortcuts work in every window)"
        );
    });
}

#[cfg(test)]
#[path = "open_windows_tests.rs"]
mod tests;
