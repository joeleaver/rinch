//! [`TestWindow`]: one open child window of a [`TestApp`], borrowed from it
//! and named by its id, looked up on every use, so a window that has closed
//! is never confused with another.

use rinch_core::element::WindowProps;
use rinch_core::events::{ModifierState, MouseButton};
use rinch_platform::{Modifiers, PlatformEvent};

use super::children::Child;
use super::{ElementBounds, NodeHandle, Target, TestApp, WindowRequest, platform_button, read};
use crate::WindowHandle;

/// An open child window of a [`TestApp`], borrowed from it: what it was
/// opened with, its requests, its document, input, and the configures a
/// compositor would send it. From [`TestApp::window`].
///
/// It names its window by id. Every use looks the window up, and panics,
/// naming it, once it is no longer open; an input of several steps (a
/// click's press and release, a keystroke's down and up, typed text) stops
/// at the step after which the window closed.
pub struct TestWindow<'a> {
    pub(super) app: &'a mut TestApp,
    pub(super) id: u64,
}

impl TestWindow<'_> {
    /// Whether its window is still open.
    pub fn is_open(&self) -> bool {
        self.app.child_index(self.id).is_some()
    }

    fn at(&self) -> usize {
        self.app
            .child_index(self.id)
            .unwrap_or_else(|| panic!("child window {} is no longer open", self.id))
    }

    fn child(&self) -> &Child {
        &self.app.children[self.at()]
    }

    fn child_mut(&mut self) -> &mut Child {
        let at = self.at();
        &mut self.app.children[at]
    }

    /// Its handle.
    pub fn handle(&self) -> WindowHandle {
        WindowHandle::from_id(self.id)
    }

    /// The props it was opened with.
    pub fn props(&self) -> &WindowProps {
        self.child().props()
    }

    /// Its requests since the last call: the ones its code made of its
    /// window (`rinch::windows` inside its document) and those made through
    /// its [`WindowHandle`], in the order made. See
    /// [Windows a component opens](super#windows-a-component-opens) for the
    /// ones that are applied.
    pub fn take_requests(&mut self) -> Vec<WindowRequest> {
        self.child_mut().take_requests()
    }

    /// Its logical size.
    pub fn size(&self) -> (u32, u32) {
        self.child().size()
    }

    /// Its scale factor.
    pub fn scale_factor(&self) -> f64 {
        self.child().scale()
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
        let scale = self.child().scale();
        self.child_mut().configure((width, height), scale);
        self.app.after_child_input();
    }

    /// It moves to an output at `scale`: a `ScaleFactorChanged` and the
    /// `SurfaceResized` the shell gets with it, its logical size kept, then
    /// a turn.
    pub fn set_scale(&mut self, scale: f64) {
        assert!(scale.is_finite() && scale > 0.0, "a scale is positive");
        let size = self.child().size();
        self.child_mut().configure(size, scale);
        self.app.after_child_input();
    }

    /// The surface the next [`Self::resize`] or [`Self::set_scale`] carries,
    /// in device pixels, published with that event in one batch. After it
    /// the stand-in returns. Publishes nothing by itself.
    pub fn set_surface_size(&mut self, width: u32, height: u32) {
        self.child_mut().set_next_surface((width, height));
    }

    /// The system closes it: a layer surface's `closed` (no veto), or a
    /// toplevel's title-bar close, which its `on_close_requested` may
    /// refuse. Then a turn.
    pub fn close_by_system(&mut self) {
        if self.child_mut().close_by_system() {
            let at = self.at();
            self.app.close_child(at);
        }
        self.app.after_child_input();
    }

    // ── Input ───────────────────────────────────────────────────────────

    /// `event` to its window as the shell dispatches it, its app's answer
    /// taken as the shell takes it (an `Exit` closes it), then a turn.
    fn send(&mut self, event: PlatformEvent) {
        let close = self.child_mut().dispatch(event);
        self.after_input(close);
    }

    /// After input whose answer was `close`: the window closed if so, then
    /// a turn of the loop and the requests.
    fn after_input(&mut self, close: bool) {
        if close {
            let at = self.at();
            self.app.close_child(at);
        }
        self.app.after_child_input();
    }

    /// Hold `modifiers` (and release any not held) for the keys, clicks and
    /// wheels sent to this window, as the shell's `ModifiersChanged` reports
    /// them to the window that has the keyboard. Each window keeps its own,
    /// as the shell tracks them per window.
    pub fn set_modifiers(&mut self, modifiers: ModifierState) {
        let ModifierState {
            shift,
            ctrl,
            alt,
            meta,
        } = modifiers;
        let held = Modifiers {
            shift,
            ctrl,
            alt,
            meta,
        };
        self.child_mut().set_modifiers(held);
        self.send(PlatformEvent::ModifiersChanged(held));
    }

    /// Move the mouse to `(x, y)` in its window.
    pub fn mouse_move(&mut self, x: f32, y: f32) {
        self.send(PlatformEvent::MouseMove { x, y });
    }

    /// Move there first unless the pointer is there; answers whether the
    /// window is still open.
    fn move_unless_there(&mut self, x: f32, y: f32) -> bool {
        if self.child().cursor_pos() != Some((x, y)) {
            self.mouse_move(x, y);
        }
        self.is_open()
    }

    /// Press `button` at `(x, y)`, moving there first if needed.
    pub fn mouse_down(&mut self, x: f32, y: f32, button: MouseButton) {
        if self.move_unless_there(x, y) {
            // The menus above this window a press here closes (#2552).
            let closes = crate::popup::light_dismiss(self.id, Some((x, y)));
            self.send(PlatformEvent::MouseDown {
                x,
                y,
                button: platform_button(button),
            });
            self.app.close_popups(&closes);
        }
    }

    /// Release `button` at `(x, y)`, moving there first if needed.
    pub fn mouse_up(&mut self, x: f32, y: f32, button: MouseButton) {
        if self.move_unless_there(x, y) {
            self.send(PlatformEvent::MouseUp {
                x,
                y,
                button: platform_button(button),
            });
        }
    }

    /// A left click at `(x, y)`: press, then release unless the press
    /// closed the window.
    pub fn click(&mut self, x: f32, y: f32) {
        self.mouse_down(x, y, MouseButton::Left);
        if self.is_open() {
            self.mouse_up(x, y, MouseButton::Left);
        }
    }

    /// A left click at the centre of `target`'s box.
    pub fn click_on(&mut self, target: impl Target) {
        let (x, y) = self.center(target);
        self.click(x, y);
    }

    /// Press the key `key` names (see [`TestApp::key_down`]), with this
    /// window's held modifiers.
    pub fn key_down(&mut self, key: &str) {
        // While a menu popup of its chain is open, its topmost menu takes
        // the keys (#2552).
        if let Some(menu) = self.app.popup_key_target(self.id) {
            self.app.window_by_id(menu).key_down(key);
            return;
        }
        let close = self.child_mut().key_down(key);
        self.after_input(close);
    }

    /// Release the key `key` names.
    pub fn key_up(&mut self, key: &str) {
        let code = crate::app::key_names::keyname_to_keycode(key)
            .unwrap_or_else(|| panic!("unknown key name {key:?}"));
        let modifiers = self.child().modifiers();
        self.send(PlatformEvent::KeyUp {
            key: code,
            logical_key: crate::app::key_names::named_logical_key(key, modifiers.shift),
            modifiers,
        });
    }

    /// A keystroke: down, then up unless the window closed.
    pub fn press_key(&mut self, key: &str) {
        self.key_down(key);
        if self.is_open() {
            self.key_up(key);
        }
    }

    /// Type `text`, one keystroke per character (see
    /// [`TestApp::type_text`]), stopping if the window closes.
    pub fn type_text(&mut self, text: &str) {
        let mut buf = [0u8; 4];
        for ch in text.chars() {
            if !self.is_open() {
                return;
            }
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
        self.query_all(selector).into_iter().next()
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
