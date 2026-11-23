//! The windows a component opens (`Window { … }`, [`open_window`]), driven
//! by the one [`TestApp`] that mounted it (#2793): made and closed by the
//! shell's own code, with no OS window and no renderer.
//!
//! A child shares the app's thread, reactive runtime and turn. It is opened
//! when the window requests are served after the turn that built its
//! element, never inside that turn, as the shell opens it from its queue on
//! a later wake. Each turn runs every open child's `AboutToWait` and, when
//! its layout is pending, the layout and bounds the shell resolves before a
//! frame, short of painting.
//!
//! [`open_window`]: crate::open_window

use rinch_core::element::{WindowProps, WindowSurfaceKind};
use rinch_platform::{AppAction, Modifiers, PlatformEvent};

use super::{RinchDocument, TestApp, TestWindow, WindowRequest};
use crate::WindowHandle;
use crate::shell::overlay_window::Backend;
use crate::shell::rinch_runtime::ChildWindow;
use crate::shell::rinch_runtime::RinchNativeEvent;
use std::cell::RefCell;
use std::rc::Rc;

/// An open child window, as the harness keeps it.
pub(super) struct Child {
    window: ChildWindow,
    /// What it was opened with.
    props: WindowProps,
    /// The surface kind a compositor with layer shell gives it.
    kind: WindowSurfaceKind,
    /// Logical size.
    size: (u32, u32),
    scale: f64,
    /// The surface the test set for the next event, if any.
    next_surface: Option<(u32, u32)>,
    /// Its requests since the last take.
    requests: Vec<WindowRequest>,
    /// The modifiers held for it, as the shell's `ModifiersChanged` last
    /// reported them to it ([`TestWindow::set_modifiers`]).
    modifiers: Modifiers,
}

impl Child {
    /// Open the window `open_window` opened as `id`, as the shell makes a
    /// second window up to its OS window, then give it its first
    /// configure, as `create_window` does: the surface, its scale, and a
    /// `Resized` at its props' size. `None` when it is no longer waiting.
    ///
    /// The shell first refuses a window placed by the id of an output that
    /// is not connected (`refuse_stale_output`): here, one not among the
    /// outputs played (`TestApp::set_outputs`). With none played that step
    /// is skipped, as no output is ever gone (#2836). `None` too when it is
    /// refused.
    pub(super) fn open(id: u64, app: &TestApp) -> Option<Self> {
        let connected = super::played_outputs::connected();
        let mut window = ChildWindow::open(id, &app.fonts, app.scale, connected.as_deref())?;
        let props = window
            .app()
            .window_props
            .clone()
            .expect("a window `open_window` made has the props it was opened with");
        // The harness stands in a Wayland compositor with layer shell: the
        // kind the shell asks for is the kind it gets.
        let kind = if crate::shell::layer_shell::uses_layer_shell(&props, Backend::Wayland) {
            WindowSurfaceKind::LayerShell
        } else {
            WindowSurfaceKind::Toplevel
        };
        crate::shell::layer_shell::publish_surface_kind(window.facts_key(), kind);
        let scale = window.app().device_pixel_ratio;
        // A popup is made at its content's size (#2552), as the shell makes it.
        let popup = crate::window_handle::with_popup(id, |p| (p.props.kind, p.parent));
        if let Some((kind, parent)) = popup {
            crate::window_perf::popup_opened(id, kind, parent);
        }
        let popup_size = popup.and_then(|_| window.app_mut().content_size());
        let mut child = Self {
            window,
            size: popup_size.unwrap_or((props.width.max(1), props.height.max(1))),
            props,
            kind,
            scale,
            next_surface: None,
            requests: Vec::new(),
            modifiers: Modifiers::default(),
        };
        let size = child.size;
        child.configure(size, scale);
        Some(child)
    }

    /// Its handle's id.
    pub(super) fn id(&self) -> u64 {
        self.window.id()
    }

    /// The outputs changed: what the shell's `monitors_changed` does for a
    /// second window (`ShellWindow::follow_outputs`).
    pub(super) fn follow_outputs(&mut self) {
        self.window.follow_outputs();
    }

    /// Its document's key.
    pub(super) fn doc(&self) -> u64 {
        self.window.app().doc_key()
    }

    /// The surface for the next event: the test's, once, else the stand-in,
    /// the logical size times the scale, rounded.
    fn take_surface(&mut self) -> (u32, u32) {
        self.next_surface.take().unwrap_or((
            (f64::from(self.size.0) * self.scale).round() as u32,
            (f64::from(self.size.1) * self.scale).round() as u32,
        ))
    }

    /// The physical size the shell hands `handle_event` for this window.
    fn physical(&self) -> (u32, u32) {
        self.window.app().surface_size.unwrap_or((
            (f64::from(self.size.0) * self.scale).round() as u32,
            (f64::from(self.size.1) * self.scale).round() as u32,
        ))
    }

    /// A configure: `size` logical pixels at `scale`, with its surface, as
    /// the shell's `SurfaceResized` (and a `ScaleFactorChanged` before it
    /// for a new scale) feeds a window, and its geometry published as the
    /// shell publishes a window's size with no OS window to read.
    pub(super) fn configure(&mut self, size: (u32, u32), scale: f64) {
        self.size = (size.0.max(1), size.1.max(1));
        let new_scale = scale != self.window.app().device_pixel_ratio;
        self.scale = scale;
        let surface = self.take_surface();
        let app = self.window.app_mut();
        app.set_surface_size((surface.0.max(1), surface.1.max(1)));
        if new_scale {
            let _ = app.handle_event(PlatformEvent::ScaleFactorChanged(scale), surface, scale);
        }
        app.set_granted_size(self.size.0 as f32, self.size.1 as f32);
        let _ = app.handle_event(
            PlatformEvent::Resized {
                width: self.size.0,
                height: self.size.1,
            },
            surface,
            scale,
        );
        crate::shell::window_geometry::publish_hidden(
            self.window.facts_key(),
            None,
            Some((f64::from(self.size.0), f64::from(self.size.1))),
        );
    }

    /// What the shell does for this window in a turn, short of painting: its
    /// `AboutToWait`, and its layout and bounds when they are pending.
    pub(super) fn turn(&mut self) {
        let (physical, scale, size) = (self.physical(), self.scale, self.size);
        let app = self.window.app_mut();
        let _ = app.handle_event(PlatformEvent::AboutToWait, physical, scale);
        if app.has_pending_layout() {
            app.resolve_and_repaint(size.0 as f32, size.1 as f32);
        }
    }

    /// `event` handed to this window's app; answers whether it agreed to
    /// close ([`Self::answered`]).
    pub(super) fn dispatch(&mut self, event: PlatformEvent) -> bool {
        let (physical, scale) = (self.physical(), self.scale);
        let actions = self.window.app_mut().handle_event(event, physical, scale);
        Self::answered(&actions)
    }

    /// The key `key` names pressed in this window, with its modifiers;
    /// answers whether its app agreed to close ([`Self::answered`]).
    pub(super) fn key_down(&mut self, key: &str) -> bool {
        let mut actions = Vec::new();
        let (physical, scale, modifiers) = (self.physical(), self.scale, self.modifiers);
        let outcome = self.window.app_mut().dispatch_named_key_down(
            key,
            modifiers,
            &mut actions,
            physical,
            scale,
        );
        assert!(
            outcome != crate::app::key_names::NamedKeyDown::UnknownKey,
            "unknown key name {key:?}"
        );
        Self::answered(&actions)
    }

    /// What the harness takes from a child app's answer to input, as the
    /// shell's `process_actions` takes it for a second window: an `Exit` is
    /// its close agreed to. The rest (a redraw, a cursor, a window drag)
    /// asks nothing of a window with no OS window and is not recorded.
    fn answered(actions: &[AppAction]) -> bool {
        actions.contains(&AppAction::Exit)
    }

    /// The props it was opened with.
    pub(super) fn props(&self) -> &WindowProps {
        &self.props
    }

    /// Its requests since the last take.
    pub(super) fn take_requests(&mut self) -> Vec<WindowRequest> {
        std::mem::take(&mut self.requests)
    }

    /// Its logical size.
    pub(super) fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Its scale factor.
    pub(super) fn scale(&self) -> f64 {
        self.scale
    }

    /// The surface for the next configure.
    pub(super) fn set_next_surface(&mut self, surface: (u32, u32)) {
        self.next_surface = Some(surface);
    }

    /// The modifiers held for it.
    pub(super) fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// Hold `modifiers` for it.
    pub(super) fn set_modifiers(&mut self, modifiers: Modifiers) {
        self.modifiers = modifiers;
    }

    /// Where its app last saw the pointer.
    pub(super) fn cursor_pos(&self) -> Option<(f32, f32)> {
        self.window.app().cursor_pos
    }

    /// A native event its code sent, through its handle or from its
    /// document, as the shell takes it for a second window: a platform event
    /// it injected is dispatched to it (`inject_platform_event`), a title its
    /// code stored is its title request, and the rest are its requests
    /// ([`Self::request`]). Answers whether the window is to close.
    pub(super) fn native(&mut self, event: RinchNativeEvent) -> bool {
        match event {
            RinchNativeEvent::InjectedPlatformEvent(event) => self.dispatch(event),
            // `rinch::window::set_window_title` stores the title for its
            // document and asks the shell to take it.
            RinchNativeEvent::ApplyWindowTitle => {
                let doc = self.doc();
                if let Some(title) = crate::window::take_stored_title(|d| d == Some(doc)) {
                    self.requests.push(WindowRequest::SetTitle(title));
                }
                false
            }
            event => {
                let control = matches!(event, RinchNativeEvent::CloseWindowControl);
                WindowRequest::from_native(event)
                    .is_some_and(|request| self.request(request, control))
            }
        }
    }

    /// A request its code made: recorded, and applied as the shell's own
    /// code would without an OS. Answers whether the window is to close.
    fn request(&mut self, request: WindowRequest, control: bool) -> bool {
        self.requests.push(request.clone());
        match request {
            // A toplevel is resized at once (winit's `request_surface_size`
            // answers the new size, and the shell feeds it back as a
            // `SurfaceResized`); a layer surface's size is the compositor's
            // to give, which the test plays with `resize`.
            WindowRequest::SetSize(width, height) if self.kind == WindowSurfaceKind::Toplevel => {
                let scale = self.scale;
                self.configure((width.round() as u32, height.round() as u32), scale);
                false
            }
            // `close_current_window`: the window's close control, which its
            // app may refuse (`on_close_requested`).
            WindowRequest::Close if control => self.dispatch(PlatformEvent::UserEvent(
                rinch_platform::UserEvent::CloseWindow,
            )),
            // `WindowHandle::close`: no veto.
            WindowRequest::Close => true,
            _ => false,
        }
    }

    /// The system's close: a layer surface's `closed` closes it with no
    /// veto; a toplevel's title-bar close asks its app, which may refuse.
    pub(super) fn close_by_system(&mut self) -> bool {
        match self.kind {
            WindowSurfaceKind::LayerShell => true,
            _ => self.dispatch(PlatformEvent::CloseRequested),
        }
    }

    /// Close it as the shell does.
    pub(super) fn close(self) {
        self.window.close();
    }

    pub(super) fn document(&self) -> &Rc<RefCell<RinchDocument>> {
        self.window
            .app()
            .doc()
            .expect("an open child window has a document")
    }
}

impl TestApp {
    /// The windows the component opened that are open now, in the order
    /// they opened. See [Windows a component opens](super#windows-a-component-opens).
    pub fn windows(&self) -> Vec<WindowHandle> {
        self.children
            .iter()
            .map(|c| WindowHandle::from_id(c.id()))
            .collect()
    }

    /// The open child window `handle` names, to drive and read.
    ///
    /// Panics when it is not open (not opened yet, or closed).
    pub fn window(&mut self, handle: WindowHandle) -> TestWindow<'_> {
        self.window_by_id(handle.id())
    }

    /// The open child window whose [`WindowHandle::id`] is `id`.
    ///
    /// Panics when it is not open.
    pub fn window_by_id(&mut self, id: u64) -> TestWindow<'_> {
        assert!(
            self.child_index(id).is_some(),
            "no child window {id} is open"
        );
        TestWindow { app: self, id }
    }

    /// Where the open child `id` is in the list, if it is open.
    pub(super) fn child_index(&self, id: u64) -> Option<usize> {
        self.children.iter().position(|c| c.id() == id)
    }

    /// The open child that `doc` is the document of, or whose id is `id`.
    pub(super) fn child_at(&self, id: Option<u64>, doc: Option<u64>) -> Option<usize> {
        self.children
            .iter()
            .position(|c| Some(c.id()) == id || (doc.is_some() && Some(c.doc()) == doc))
    }

    /// Close the child at `at`, as the shell does: every popup above it
    /// first, topmost first, each told it closed with its parent (#2552).
    pub(super) fn close_child(&mut self, at: usize) {
        let id = self.children[at].id();
        let links = crate::window_handle::popups();
        for above in crate::popup::chain::close_order(id, &links) {
            crate::window_handle::note_dismissal(
                above,
                rinch_core::element::Dismissal::ParentClosed,
            );
            if let Some(i) = self.child_index(above) {
                self.children.remove(i).close();
            }
        }
        if let Some(at) = self.child_index(id) {
            self.children.remove(at).close();
        }
    }

    /// The id the window registry gives this app's own window: `0`, a
    /// document that is no registry window.
    pub(super) fn window_id(&self) -> u64 {
        crate::window_handle::window_of_doc(self.app.doc_key())
    }

    /// Close the open popups `ids` (and what is above each), as the shell
    /// closes the menus a press dismissed at the end of the handler.
    pub(super) fn close_popups(&mut self, ids: &[u64]) {
        for &id in ids {
            if let Some(at) = self.child_index(id) {
                self.close_child(at);
            }
        }
        if !ids.is_empty() {
            self.after_child_input();
        }
    }

    /// The topmost menu popup a key reaching window `id` goes to, when it is
    /// open here and is not `id`.
    pub(super) fn popup_key_target(&self, id: u64) -> Option<u64> {
        let target = crate::popup::chain::key_target(id, &crate::window_handle::popups())?;
        (target != id && self.child_index(target).is_some()).then_some(target)
    }

    /// The system dismisses the popup `handle` (Wayland's `popup_done`): it
    /// closes with what is above it, and its `on_dismissed` is told
    /// [`Dismissal::System`](rinch_core::element::Dismissal::System).
    ///
    /// Panics when it is not an open popup.
    pub fn dismiss_popup_by_system(&mut self, handle: WindowHandle) {
        assert!(handle.is_popup(), "window {} is not a popup", handle.id());
        let at = self
            .child_index(handle.id())
            .unwrap_or_else(|| panic!("popup {} is not open", handle.id()));
        crate::window_handle::note_dismissal(handle.id(), rinch_core::element::Dismissal::System);
        self.close_child(at);
        self.after_child_input();
    }
}

#[cfg(test)]
mod tests {
    use crate::testing::TestApp;
    use rinch_core::dom::RenderScope;
    use rinch_core::element::WindowProps;

    /// After a child closes, by its handle or with its owner, its own facts
    /// (the per-window signals and the scope that owns them) are gone.
    #[test]
    fn a_closed_childs_facts_are_gone() {
        let mut app = TestApp::mount(|scope: &mut RenderScope| {
            crate::open_window(WindowProps::default(), |s: &mut RenderScope| {
                s.create_element("div")
            });
            scope.create_element("div")
        });
        let child = app.windows()[0];
        let doc = app.window(child).doc();
        assert!(crate::window::window_facts_kept(doc));
        child.close();
        app.settle();
        assert!(app.windows().is_empty());
        assert!(!crate::window::window_facts_kept(doc), "by its handle");

        let row = rinch_core::reactive::Scope::new();
        let handle = row.run(|| {
            crate::open_window(WindowProps::default(), |s: &mut RenderScope| {
                s.create_element("div")
            })
        });
        app.settle();
        let doc = app.window(handle).doc();
        row.dispose();
        app.settle();
        assert!(app.windows().is_empty());
        assert!(!crate::window::window_facts_kept(doc), "with its owner");
    }

    /// A content-sized child's largest size is its own output's (placed by
    /// id on the second listed, smaller than the first), and follows that
    /// output when the played outputs change, as the shell's
    /// `monitors_changed` makes each window follow (#2836).
    #[test]
    fn a_content_sized_childs_largest_size_follows_its_output() {
        use crate::outputs::{Output, OutputId};
        use rinch_core::element::{ContentSizing, LayerSurface};
        let screen = |id: u64, width: f64, height: f64| Output {
            id: OutputId::from_raw(id),
            name: Some(format!("DP-{id}")),
            position: (0.0, 0.0),
            size: (width, height),
            scale: 1.0,
        };
        let mut app = TestApp::mount(|scope: &mut RenderScope| scope.create_element("div"));
        app.set_outputs([screen(1, 1920.0, 1080.0), screen(2, 1280.0, 1024.0)]);
        let props = WindowProps {
            size_to_content: Some(ContentSizing::new()),
            wayland_layer: Some(LayerSurface {
                output_id: Some(OutputId::from_raw(2)),
                ..LayerSurface::default()
            }),
            ..WindowProps::default()
        };
        let handle = crate::open_window(props, |s: &mut RenderScope| s.create_element("div"));
        app.settle();
        let largest = |app: &TestApp| {
            let at = app.child_index(handle.id()).expect("the child is open");
            app.children[at].window.app().content_largest()
        };
        assert_eq!(
            largest(&app),
            Some((1280.0, 1024.0)),
            "its own output's size, not the first's"
        );
        app.set_outputs([screen(1, 1920.0, 1080.0), screen(2, 1600.0, 900.0)]);
        assert_eq!(
            largest(&app),
            Some((1600.0, 900.0)),
            "its output's size after the output changed"
        );
    }
}
