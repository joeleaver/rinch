//! The windows `open_window` opens (#2416): making them, closing them, and
//! the exit rule ([`crate::ExitWhen`]).

use super::*;
use crate::window_handle;

impl RinchRuntime {
    /// Make the window `open_window` opened as `id`, if it is still waiting
    /// (closed before it was made, it is not).
    pub(super) fn open_pending_window(&mut self, id: u64, event_loop: &dyn ActiveEventLoop) {
        // A popup is made against its parent (#2552).
        if window_handle::with_popup(id, |_| ()).is_some() {
            self.open_pending_popup(id, event_loop);
            return;
        }
        let Some(pending) = window_handle::take_pending(id) else {
            return;
        };
        let connected: Vec<u64> = event_loop
            .available_monitors()
            .map(|m| m.native_id())
            .collect();
        if Self::refuse_stale_output(id, &pending.props, Backend::of(event_loop), &connected) {
            return;
        }
        let mut window = Self::mount_other_window(pending, &self.fonts, self.main.scale_factor());
        window.create_window(event_loop, None);
        self.others.push(window);
        self.about_to_wait_for(WindowRole::Other(id), event_loop);
    }

    /// Window `id` is placed by an output id no connected output has: the
    /// output went between `outputs()` and now (#2658,
    /// `layer_shell::stale_output_id`). It is not made: its handle closes as a
    /// window closed before it was made does (`is_open` false, gone from
    /// `windows()`, its `on_closed` called once, `on_close_requested` never
    /// asked). Answers whether it was refused. `connected` is the outputs'
    /// ids: winit's monitors here, the outputs `rinch::testing` plays there
    /// (#2836).
    pub(super) fn refuse_stale_output(
        id: u64,
        props: &rinch_core::element::WindowProps,
        backend: Backend,
        connected: &[u64],
    ) -> bool {
        let Some(gone) = crate::shell::layer_shell::stale_output_id(props, backend, connected)
        else {
            return false;
        };
        tracing::warn!(
            "the window \"{}\" is placed on output {gone}, which is no longer connected: \
             it is not made",
            props.title
        );
        window_handle::note_closed(id);
        true
    }

    /// A second window made up to its OS window, from `pending`: its state,
    /// its document mounted at its mount scale (`main_scale`, the main
    /// window's, unless its content sizes it), and its handle told it is
    /// mounted. What the shell does before `create_window`, and all
    /// `rinch::testing` does for a window a component opens (its
    /// `ChildWindow`, with the `testing` feature).
    pub(super) fn mount_other_window(
        pending: window_handle::Pending,
        fonts: &[crate::font::AppFont],
        main_scale: f64,
    ) -> ShellWindow {
        let id = pending.id;
        let mut window = Self::new_other_window(pending, fonts);
        let scale = window.mount_scale(main_scale);
        window_handle::mounting(id, || window.mount_before_window(scale));
        window_handle::note_mounted(id, window.app.doc_key());
        window.register_perf(false);
        window
    }

    /// A second window's state before its OS window is made: its document
    /// keeps its own stores (falling back to the app's), publishes its own
    /// facts, and is made with the app's fonts.
    pub(super) fn new_other_window(
        pending: window_handle::Pending,
        fonts: &[crate::font::AppFont],
    ) -> ShellWindow {
        let window_handle::Pending { id, props, content } = pending;
        if props.menu_in_titlebar {
            warn_menu_in_titlebar_once();
        }
        let mut app = super::content_window::app_for_window(content, &props);
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

    /// `role`'s OS window was destroyed under the app while rinch still held
    /// it (#2489): on X11, another client's `XDestroyWindow` (`xdotool
    /// windowclose`), which winit reports as `WindowEvent::Destroyed`. A
    /// window rinch drops itself is no longer one of its roles when that
    /// event comes, so this is only ever someone else's doing. The window
    /// closes as one whose close was agreed to (`close_soon`): no
    /// `on_close_requested` is asked, there being nothing left to keep open,
    /// and the exit rule decides whether the app runs on. Answers whether the
    /// event loop ends at once.
    pub(super) fn window_destroyed(&mut self, role: WindowRole) -> bool {
        tracing::warn!(
            "the window \"{}\" was destroyed from outside the app: it is closed",
            self.w(role).title
        );
        self.close_soon(role)
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
        // Every popup above it first, topmost first (#2552): wlroots ends
        // the connection of a client that destroys a popup's parent first.
        self.close_popups_above(id);
        self.close_one_other(id);
    }

    /// [`Self::close_other`] for window `id` alone, its popups closed
    /// already.
    pub(super) fn close_one_other(&mut self, id: u64) {
        let popup = window_handle::with_popup(id, |_| ()).is_some();
        // A menu closed while it waited for its parent's keyboard waits no
        // more, so the raised parent is put back below (#2552).
        self.popups.awaiting_keyboard.retain(|(_, p)| p.id != id);
        match self.others.iter().position(|w| w.handle == id) {
            Some(at) => Self::close_made(self.others.remove(at)),
            None => Self::closed_unmade(id),
        }
        if popup {
            self.note_popup_closed();
        }
    }

    /// Close the made window `window`: its own teardown
    /// ([`ShellWindow::close`]), then its counters and its handle. The
    /// shell's close of a second window, and `rinch::testing`'s
    /// (`ChildWindow::close`, with the `testing` feature).
    pub(super) fn close_made(window: ShellWindow) {
        let id = window.handle;
        window.close();
        Self::closed_unmade(id);
    }

    /// What closing window `id` tells beyond its teardown: its counters go,
    /// and its handle says it is closed. All there is for one never made.
    fn closed_unmade(id: u64) {
        crate::window_perf::closed(id);
        window_handle::note_closed(id);
    }

    /// Close the main window of an app that runs on without it: its OS
    /// window goes as a hide does, then its document is disposed.
    fn close_main(&mut self) {
        if self.main_closed {
            return;
        }
        let main = self.main_handle;
        self.close_popups_above(main);
        self.main.hide_window(self.native_menu.as_ref());
        self.main.app.dispose_document();
        self.main_closed = true;
        crate::window_perf::closed(self.main_handle);
        window_handle::note_closed(self.main_handle);
    }

    /// Hide `role`'s window (its OS window goes; its document lives), the
    /// native menu bar with it for the main window.
    pub(super) fn hide_role(&mut self, role: WindowRole) {
        // A hidden window's popups have nothing to hang from (#2552).
        if let Some(id) = self.window_id_of(role) {
            self.close_popups_above(id);
        }
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

    /// The event loop's start, once the outputs are published: publish which
    /// window roles the display offers (#2659), then, for a windowless app,
    /// mount its root, which may read them. Before the main window is made
    /// too, so its first render reads them. Answers whether the app is
    /// windowless (no main window to make).
    pub(super) fn start_surfaces(&mut self, roles: crate::window::WindowRoles) -> bool {
        crate::window_roles::publish(roles);
        if self.windowless {
            self.mount_windowless_root();
            return true;
        }
        false
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
            node.parent?;
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
        self.mount_document_before_window(scale);
        self.facts_key = Some(self.app.doc_key());
    }

    /// Mount the document with no OS window yet, at the props' size, or, for
    /// a window sized by its content, at its largest size, laid out so its
    /// content's size is known before the window is made.
    pub(super) fn mount_document_before_window(&mut self, scale: f64) {
        self.app.set_device_pixel_ratio(scale);
        let (width, height) = match self.content_max() {
            Some(max) => {
                self.app.restart_content_limit(max);
                max
            }
            None => (self.width.max(1) as f32, self.height.max(1) as f32),
        };
        self.app.mount_component(width, height);
        if self.app.is_content_sized() {
            self.app.resolve_and_repaint(width, height);
        }
    }

    /// Paint a second window, on the GPU when it has a renderer. Its staged
    /// layer changes are not on the surface until the drain sends them
    /// (#2464), so the frame's commit carries none of them.
    pub(super) fn paint_window(&mut self) -> Result<(), String> {
        #[cfg(feature = "gpu-renderer")]
        let painted = if self.renderer.is_some() {
            self.paint_gpu()
        } else {
            self.paint_software()
        };
        #[cfg(not(feature = "gpu-renderer"))]
        let painted = self.paint_software();
        // One redraw is one performance frame, in every window (#2657).
        self.app.end_perf_frame();
        painted
    }

    /// Close this window: its document goes, then what it published, then
    /// its renderer and OS window. The document first, so that code its
    /// disposal runs (an `on_cleanup`, an effect's last run) still reads its
    /// own window's facts and never the thread's (#2789's rule, to the
    /// end).
    pub(super) fn close(mut self) {
        self.app.dispose_document();
        self.forget_published();
        // A popup's renderer is kept for the next popup (#2552).
        #[cfg(feature = "gpu-renderer")]
        if let Some(renderer) = self.renderer.take() {
            if window_handle::with_popup(self.handle, |_| ()).is_some() {
                let idle = self
                    .app
                    .window_props
                    .as_ref()
                    .map_or(Default::default(), |p| p.idle_trim)
                    .period();
                super::popups::keep_spare_renderer(renderer, idle);
            }
        }
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
        // A title its code stored and no window took (one set by a cleanup).
        let _ = crate::window::take_stored_title(|d| d == Some(doc));
        window_geometry::forget(Some(doc));
        crate::shell::layer_shell::forget_surface_kind(Some(doc));
        crate::render_surface::forget_window_surfaces(Some(doc));
    }
}

/// A window a component opened, made by the shell's own path up to its OS
/// window ([`RinchRuntime::mount_other_window`]) and closed by the shell's
/// own teardown ([`RinchRuntime::close_made`]): what `rinch::testing`
/// drives as a child window, with no OS window and no renderer.
#[cfg(feature = "testing")]
pub(crate) struct ChildWindow(ShellWindow);

#[cfg(feature = "testing")]
impl ChildWindow {
    /// Make the window `open_window` opened as `id`, mounted at `main_scale`
    /// unless its content sizes it, with `fonts`; `None` when it is no
    /// longer waiting (closed before it was made), or when it is placed by
    /// an output id that is not among the `connected` ones and so refused
    /// as the shell refuses it ([`RinchRuntime::refuse_stale_output`], on
    /// the Wayland the harness stands in for). `connected` is `None` when
    /// no outputs are listed, and nothing is refused then (#2836).
    pub(crate) fn open(
        id: u64,
        fonts: &[crate::font::AppFont],
        main_scale: f64,
        connected: Option<&[u64]>,
    ) -> Option<Self> {
        let pending = window_handle::take_pending(id)?;
        if let Some(connected) = connected {
            if RinchRuntime::refuse_stale_output(id, &pending.props, Backend::Wayland, connected) {
                return None;
            }
        }
        Some(Self(RinchRuntime::mount_other_window(
            pending, fonts, main_scale,
        )))
    }

    /// The outputs changed: a content-sized window whose largest size
    /// changed starts a new negotiation, as the shell's `monitors_changed`
    /// does for each window ([`ShellWindow::follow_outputs`]).
    pub(crate) fn follow_outputs(&mut self) {
        self.0.follow_outputs();
    }

    /// Its handle's id.
    pub(crate) fn id(&self) -> u64 {
        self.0.handle
    }

    /// The key its facts are published under: its document's.
    pub(crate) fn facts_key(&self) -> Option<u64> {
        self.0.facts_key
    }

    /// Its app.
    pub(crate) fn app(&self) -> &RinchApp {
        &self.0.app
    }

    /// Its app, to drive.
    pub(crate) fn app_mut(&mut self) -> &mut RinchApp {
        &mut self.0.app
    }

    /// Close it as the shell closes a second window.
    pub(crate) fn close(self) {
        RinchRuntime::close_made(self.0);
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
