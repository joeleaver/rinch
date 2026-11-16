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

use super::{
    ElementBounds, NodeHandle, RinchDocument, Target, TestApp, WindowRequest, platform_button, read,
};
use crate::WindowHandle;
use crate::shell::overlay_window::Backend;
use crate::shell::rinch_runtime::ChildWindow;
use rinch_core::events::MouseButton;
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
    modifiers: Modifiers,
}

impl Child {
    /// Open the window `open_window` opened as `id`, as the shell makes a
    /// second window up to its OS window, then give it its first
    /// configure, as `create_window` does: the surface, its scale, and a
    /// `Resized` at its props' size. `None` when it is no longer waiting.
    pub(super) fn open(id: u64, app: &TestApp) -> Option<Self> {
        let window = ChildWindow::open(id, &app.fonts, app.scale)?;
        let props = window.app().window_props.clone().unwrap_or_default();
        // The harness stands in a Wayland compositor with layer shell: the
        // kind the shell asks for is the kind it gets.
        let kind = if crate::shell::layer_shell::uses_layer_shell(&props, Backend::Wayland) {
            WindowSurfaceKind::LayerShell
        } else {
            WindowSurfaceKind::Toplevel
        };
        crate::shell::layer_shell::publish_surface_kind(window.facts_key(), kind);
        let scale = window.app().device_pixel_ratio;
        let mut child = Self {
            window,
            size: (props.width.max(1), props.height.max(1)),
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
    fn configure(&mut self, size: (u32, u32), scale: f64) {
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
    /// close (`AppAction::Exit`).
    fn dispatch(&mut self, event: PlatformEvent) -> bool {
        let (physical, scale) = (self.physical(), self.scale);
        let actions = self.window.app_mut().handle_event(event, physical, scale);
        actions.contains(&AppAction::Exit)
    }

    /// A request its code made: recorded, and applied as the shell's own
    /// code would without an OS. Answers whether the window is to close.
    pub(super) fn request(&mut self, request: WindowRequest, control: bool) -> bool {
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
    fn close_by_system(&mut self) -> bool {
        match self.kind {
            WindowSurfaceKind::LayerShell => true,
            _ => self.dispatch(PlatformEvent::CloseRequested),
        }
    }

    /// Close it as the shell does.
    pub(super) fn close(self) {
        self.window.close();
    }

    fn document(&self) -> &Rc<RefCell<RinchDocument>> {
        self.window
            .app()
            .doc()
            .expect("an open child window has a document")
    }
}

impl TestApp {
    /// The windows the component opened that are open now, in the order
    /// they opened. See [Windows a component opens](self#windows-a-component-opens).
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
        let at = self
            .children
            .iter()
            .position(|c| c.id() == id)
            .unwrap_or_else(|| panic!("no child window {id} is open"));
        TestWindow { app: self, at }
    }

    /// The open child that `doc` is the document of, or whose id is `id`.
    pub(super) fn child_at(&self, id: Option<u64>, doc: Option<u64>) -> Option<usize> {
        self.children
            .iter()
            .position(|c| Some(c.id()) == id || (doc.is_some() && Some(c.doc()) == doc))
    }

    /// Close the child at `at`, as the shell does.
    pub(super) fn close_child(&mut self, at: usize) {
        let child = self.children.remove(at);
        child.close();
    }
}

/// An open child window of a [`TestApp`], borrowed from it: what it was
/// opened with, its requests, its document, input, and the configures a
/// compositor would send it. From [`TestApp::window`].
pub struct TestWindow<'a> {
    app: &'a mut TestApp,
    at: usize,
}

impl TestWindow<'_> {
    fn child(&self) -> &Child {
        &self.app.children[self.at]
    }

    fn child_mut(&mut self) -> &mut Child {
        &mut self.app.children[self.at]
    }

    /// Its handle.
    pub fn handle(&self) -> WindowHandle {
        WindowHandle::from_id(self.child().id())
    }

    /// The props it was opened with.
    pub fn props(&self) -> &WindowProps {
        &self.child().props
    }

    /// Its requests since the last call: the ones its code made of its
    /// window (`rinch::windows` inside its document) and those made through
    /// its [`WindowHandle`], in the order made. See
    /// [Windows a component opens](self#windows-a-component-opens) for the
    /// ones that are applied.
    pub fn take_requests(&mut self) -> Vec<WindowRequest> {
        std::mem::take(&mut self.child_mut().requests)
    }

    /// Its logical size.
    pub fn size(&self) -> (u32, u32) {
        self.child().size
    }

    /// Its scale factor.
    pub fn scale_factor(&self) -> f64 {
        self.child().scale
    }

    /// Its document's key: for the in-crate tests.
    #[cfg(test)]
    pub(crate) fn doc(&self) -> u64 {
        self.child().doc()
    }

    // ── What a compositor sends it ──────────────────────────────────────

    /// The compositor gives it `width` × `height` logical pixels: a
    /// `SurfaceResized`, with the surface set for it by
    /// [`Self::set_surface_size`] or else the stand-in (the logical size
    /// times the scale, rounded), then a turn.
    pub fn resize(&mut self, width: u32, height: u32) {
        let scale = self.child().scale;
        self.child_mut().configure((width, height), scale);
        self.app.after_child_input();
    }

    /// It moves to an output at `scale`: a `ScaleFactorChanged` and the
    /// `SurfaceResized` the shell gets with it, its logical size kept, then
    /// a turn.
    pub fn set_scale(&mut self, scale: f64) {
        assert!(scale.is_finite() && scale > 0.0, "a scale is positive");
        let size = self.child().size;
        self.child_mut().configure(size, scale);
        self.app.after_child_input();
    }

    /// The surface the next [`Self::resize`] or [`Self::set_scale`] carries,
    /// in device pixels, published with that event in one batch. After it
    /// the stand-in returns. Publishes nothing by itself.
    pub fn set_surface_size(&mut self, width: u32, height: u32) {
        self.child_mut().next_surface = Some((width, height));
    }

    /// The system closes it: a layer surface's `closed` (no veto), or a
    /// toplevel's title-bar close, which its `on_close_requested` may
    /// refuse. Then a turn.
    pub fn close_by_system(&mut self) {
        if self.child_mut().close_by_system() {
            self.app.close_child(self.at);
        }
        self.app.after_child_input();
    }

    // ── Input ───────────────────────────────────────────────────────────

    fn send(&mut self, event: PlatformEvent) {
        let close = self.child_mut().dispatch(event);
        if close {
            self.app.close_child(self.at);
        }
        self.app.after_child_input();
    }

    /// Move the mouse to `(x, y)` in its window.
    pub fn mouse_move(&mut self, x: f32, y: f32) {
        self.send(PlatformEvent::MouseMove { x, y });
    }

    /// Press `button` at `(x, y)`, moving there first if needed.
    pub fn mouse_down(&mut self, x: f32, y: f32, button: MouseButton) {
        if self.child().window.app().cursor_pos != Some((x, y)) {
            self.mouse_move(x, y);
        }
        self.send(PlatformEvent::MouseDown {
            x,
            y,
            button: platform_button(button),
        });
    }

    /// Release `button` at `(x, y)`, moving there first if needed.
    pub fn mouse_up(&mut self, x: f32, y: f32, button: MouseButton) {
        if self.child().window.app().cursor_pos != Some((x, y)) {
            self.mouse_move(x, y);
        }
        self.send(PlatformEvent::MouseUp {
            x,
            y,
            button: platform_button(button),
        });
    }

    /// A left click at `(x, y)`.
    pub fn click(&mut self, x: f32, y: f32) {
        self.mouse_down(x, y, MouseButton::Left);
        self.mouse_up(x, y, MouseButton::Left);
    }

    /// A left click at the centre of `target`'s box.
    pub fn click_on(&mut self, target: impl Target) {
        let (x, y) = self.center(target);
        self.click(x, y);
    }

    /// Press the key `key` names (see [`TestApp::key_down`]).
    pub fn key_down(&mut self, key: &str) {
        let mut actions = Vec::new();
        let child = self.child_mut();
        let (physical, scale, modifiers) = (child.physical(), child.scale, child.modifiers);
        let outcome = child.window.app_mut().dispatch_named_key_down(
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
        self.app.after_child_input();
    }

    /// Release the key `key` names.
    pub fn key_up(&mut self, key: &str) {
        let code = crate::app::key_names::keyname_to_keycode(key)
            .unwrap_or_else(|| panic!("unknown key name {key:?}"));
        let modifiers = self.child().modifiers;
        self.send(PlatformEvent::KeyUp {
            key: code,
            logical_key: crate::app::key_names::named_logical_key(key, modifiers.shift),
            modifiers,
        });
    }

    /// A keystroke.
    pub fn press_key(&mut self, key: &str) {
        self.key_down(key);
        self.key_up(key);
    }

    /// Type `text`, one keystroke per character (see [`TestApp::type_text`]).
    pub fn type_text(&mut self, text: &str) {
        let mut buf = [0u8; 4];
        for ch in text.chars() {
            let key = match ch {
                '\n' => "Enter",
                '\t' => "Tab",
                c => c.encode_utf8(&mut buf),
            };
            let key = key.to_owned();
            self.press_key(&key);
        }
    }

    // ── Reading its document ────────────────────────────────────────────

    /// The first element matching `selector` in its document.
    pub fn query(&self, selector: &str) -> Option<NodeHandle> {
        read::query_all(self.child().document(), selector)
            .into_iter()
            .next()
    }

    /// Every element matching `selector` in its document.
    pub fn query_all(&self, selector: &str) -> Vec<NodeHandle> {
        read::query_all(self.child().document(), selector)
    }

    /// The text in `target`'s subtree.
    pub fn text(&self, target: impl Target) -> String {
        let doc = self.child().document();
        read::text(doc, target.in_doc(doc))
    }

    /// `target`'s attribute `name`.
    pub fn attribute(&self, target: impl Target, name: &str) -> Option<String> {
        let doc = self.child().document();
        read::attribute(doc, target.in_doc(doc), name)
    }

    /// The box `target` is painted in, in logical pixels from its window's
    /// top-left.
    pub fn bounds(&self, target: impl Target) -> ElementBounds {
        let doc = self.child().document();
        read::bounds(doc, target.in_doc(doc))
    }

    /// The centre of [`Self::bounds`].
    pub fn center(&self, target: impl Target) -> (f32, f32) {
        let b = self.bounds(target);
        (b.x + b.width / 2.0, b.y + b.height / 2.0)
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
}
