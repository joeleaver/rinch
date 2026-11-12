//! A headless harness for testing an app's own views: mount a component, send
//! it input the way the desktop runtime does, and read the document back.
//!
//! Enable the `testing` feature (it needs `desktop`, the default), usually as
//! a dev-dependency:
//!
//! ```toml
//! [dev-dependencies]
//! rinch = { workspace = true, features = ["desktop", "testing"] }
//! ```
//!
//! [`TestApp`] owns a [`RinchApp`] with no window. Every input method sends the
//! [`PlatformEvent`] the desktop shell makes of the matching winit event, to
//! the same [`RinchApp::handle_event`] the shell calls, so a test sees what a
//! real press, key or wheel does: hit testing, which handler runs, focus,
//! default actions, the menus a key reaches first. After each input the
//! harness runs one turn of the event loop — the main-thread callback queue,
//! the work other threads sent the app, `AboutToWait` (transitions, animations,
//! layout), and the frame painted with the software renderer when one was
//! asked for — so effects and layout are current when the call returns.
//!
//! No display, compositor or GPU is needed.
//!
//! # Which handler runs
//!
//! A row with its own `onmousedown`/`onmouseup` inside a container that has
//! them too: a press on the row runs the row's handler only, the nearest one
//! (as `rinch_core::events` documents); a press on the container's padding runs
//! the container's.
//!
//! ```
//! use rinch::prelude::*;
//! use rinch::testing::TestApp;
//! use std::cell::RefCell;
//! use std::rc::Rc;
//!
//! let log = Rc::new(RefCell::new(Vec::<&str>::new()));
//! let l = log.clone();
//! let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
//!     let (a, b, c, d) = (l.clone(), l.clone(), l.clone(), l.clone());
//!     rsx! {
//!         div {
//!             id: "list",
//!             style: "padding: 20px; width: 300px",
//!             onmousedown: move || a.borrow_mut().push("list down"),
//!             onmouseup: move || b.borrow_mut().push("list up"),
//!             div {
//!                 id: "row",
//!                 style: "height: 30px",
//!                 onmousedown: move || c.borrow_mut().push("row down"),
//!                 onmouseup: move || d.borrow_mut().push("row up"),
//!                 "First result"
//!             }
//!         }
//!     }
//! });
//!
//! app.click_on("#row");
//! assert_eq!(*log.borrow(), ["row down", "row up"]);
//!
//! log.borrow_mut().clear();
//! let list = app.bounds("#list");
//! app.click(list.x + 5.0, list.y + 5.0); // the container's own padding
//! assert_eq!(*log.borrow(), ["list down", "list up"]);
//! ```
//!
//! # What it does not do
//!
//! - It drives one window. Menus a key reaches are matched (as the shell does
//!   before the app sees a key), but the native menu bar, the tray, other
//!   windows and the OS (clipboard aside) are not there.
//! - Time is real: a transition or an animation advances with the wall clock,
//!   as in the app. [`TestApp::settle`] runs further turns; it does not skip
//!   ahead.
//! - `set_timeout` and `set_interval` do not fire: their native scheduler hands
//!   the callback back to the main thread the runtime registers, and a test
//!   has none.
//! - The main-thread callback queue is process-wide. A `Signal::send` from a
//!   worker thread of *another* test can be run by this test's turn, on the
//!   wrong thread; a test that sends from a worker thread should run alone in
//!   its process, or under `--test-threads=1`.
//! - Render surfaces (`RenderSurface`, `GameViewport`) are laid out but not
//!   driven: no per-frame render callback runs.
//! - A chord on a punctuation key (`Ctrl+/`, `Ctrl+=`) does not reach a menu
//!   shortcut: only letters, digits and the named keys are matched, as with
//!   the MCP `key_press` tool. The key still reaches the app.
//! - The window actions the app asks for are recorded, not performed: a
//!   right click in a text field yields `AppAction::ShowTextContextMenu` in
//!   [`TestApp::take_actions`], and no menu opens.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use rinch_core::dom::{DomDocument, NodeHandle, NodeId, RenderScope};
use rinch_core::events::{ModifierState, MouseButton};
use rinch_core::reactive::ElementBounds;
use rinch_dom::RinchDocument;
use rinch_platform::{AppAction, ImeEvent, Modifiers, PlatformEvent};

use crate::app::RinchApp;
use crate::app::key_names::{NamedKeyDown, keyname_to_keycode, named_logical_key};
use crate::font::AppFont;

/// The window size a [`TestApp`] mounts at unless told otherwise, in logical
/// (CSS) pixels.
pub const DEFAULT_SIZE: (u32, u32) = (800, 600);

/// The shell's button for the one an app's handlers are told about.
fn platform_button(button: MouseButton) -> rinch_platform::MouseButton {
    match button {
        MouseButton::Left => rinch_platform::MouseButton::Left,
        MouseButton::Middle => rinch_platform::MouseButton::Middle,
        MouseButton::Right => rinch_platform::MouseButton::Right,
    }
}

/// How many turns [`TestApp::settle`] runs at most before it gives up waiting
/// for the app to stop asking for frames (a running animation never does).
const SETTLE_TURNS: usize = 16;

/// A component mounted into a headless app, for a test to drive.
///
/// Built by [`TestApp::mount`] (an 800x600 window at scale 1) or
/// [`TestApp::builder`]. See the [module documentation](self).
///
/// Positions are logical (CSS) pixels at every scale factor — the space
/// [`TestApp::bounds`] answers in and the pointer arrives in on every host —
/// so a test reads a box and presses inside it without converting anything.
pub struct TestApp {
    app: RinchApp,
    /// Logical size.
    size: (u32, u32),
    scale: f64,
    /// The modifiers held, as the shell's `ModifiersChanged` last reported.
    modifiers: Modifiers,
    actions: Vec<AppAction>,
}

/// Options for a [`TestApp`], from [`TestApp::builder`].
pub struct TestAppBuilder {
    app: RinchApp,
    size: (u32, u32),
    scale: f64,
}

impl TestAppBuilder {
    /// The window's size in logical (CSS) pixels. Default 800x600.
    pub fn size(mut self, width: u32, height: u32) -> Self {
        self.size = (width.max(1), height.max(1));
        self
    }

    /// The display's scale factor (device pixel ratio). Default `1.0`.
    /// Layout and input stay in logical pixels; media queries on resolution
    /// and the painted frame see the scale.
    pub fn scale_factor(mut self, scale: f64) -> Self {
        assert!(
            scale.is_finite() && scale > 0.0,
            "a scale factor is positive"
        );
        self.scale = scale;
        self
    }

    /// Fonts the app bundles, as [`crate::App::fonts`] registers them.
    ///
    /// A test that compares horizontal text geometry should bundle its font
    /// (`AppFont::sans_serif(..)`): otherwise widths depend on the fonts the
    /// host has installed.
    pub fn fonts(mut self, fonts: &[AppFont]) -> Self {
        self.app.register_app_fonts(fonts);
        self
    }

    /// The theme, as [`crate::App::theme`] sets it: the CSS variables the
    /// components read.
    #[cfg(feature = "theme")]
    pub fn theme(mut self, theme: crate::ThemeProviderProps) -> Self {
        self.app
            .set_owned_theme_css(Some(crate::generate_theme_css_string(&theme)));
        self
    }

    /// Mount the component, lay it out and paint its first frame, as the
    /// shell does when the window opens, and give the window keyboard focus.
    pub fn mount(self) -> TestApp {
        let TestAppBuilder {
            mut app,
            size,
            scale,
        } = self;
        // The shell's order (`ShellWindow::create_window`): the scale first,
        // so the first style resolution sees the device pixel ratio (#211),
        // then the mount at the logical size.
        app.set_device_pixel_ratio(scale);
        app.mount_component(size.0 as f32, size.1 as f32);
        let mut test = TestApp {
            app,
            size,
            scale,
            modifiers: Modifiers::default(),
            actions: Vec::new(),
        };
        test.paint();
        // A window the user is typing into or clicking has focus.
        test.send(PlatformEvent::WindowFocus(true));
        test
    }
}

/// What a [`TestApp`] method acts on: a CSS selector (`"#row"`, `".item"`,
/// the first match), or a [`NodeHandle`] the component kept.
///
/// Sealed: implemented for `&str`, `String`, `NodeHandle` and `&NodeHandle`
/// only.
pub trait Target: sealed::Sealed {
    /// The node this names in `app`'s document. Panics when it names none.
    fn node_id(&self, app: &TestApp) -> NodeId;
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for &str {}
    impl Sealed for String {}
    impl Sealed for super::NodeHandle {}
    impl Sealed for &super::NodeHandle {}
}

impl Target for &str {
    fn node_id(&self, app: &TestApp) -> NodeId {
        app.query(self)
            .unwrap_or_else(|| panic!("no element matches {self:?}"))
            .node_id()
    }
}

impl Target for String {
    fn node_id(&self, app: &TestApp) -> NodeId {
        self.as_str().node_id(app)
    }
}

impl Target for NodeHandle {
    fn node_id(&self, _app: &TestApp) -> NodeId {
        NodeHandle::node_id(self)
    }
}

impl Target for &NodeHandle {
    fn node_id(&self, _app: &TestApp) -> NodeId {
        NodeHandle::node_id(self)
    }
}

impl TestApp {
    /// Mount `component` in an 800x600 window at scale 1.
    pub fn mount(component: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> Self {
        Self::builder(component).mount()
    }

    /// Choose the size, scale factor, fonts or theme before mounting.
    pub fn builder(
        component: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static,
    ) -> TestAppBuilder {
        TestAppBuilder {
            app: RinchApp::new(component),
            size: DEFAULT_SIZE,
            scale: 1.0,
        }
    }

    // ── The event loop ──────────────────────────────────────────────────

    /// The window size the shell hands `handle_event`: physical pixels.
    fn physical_size(&self) -> (u32, u32) {
        (
            (self.size.0 as f64 * self.scale).round() as u32,
            (self.size.1 as f64 * self.scale).round() as u32,
        )
    }

    /// Hand `event` to the app as the shell's `dispatch_event` does, then run
    /// one turn of the loop.
    fn send(&mut self, event: PlatformEvent) {
        rinch_core::clear_signals_changed();
        let asked = self.dispatch(event);
        self.turn(asked);
    }

    /// `handle_event` alone, keeping what the app asked for.
    fn dispatch(&mut self, event: PlatformEvent) -> bool {
        let actions = self
            .app
            .handle_event(event, self.physical_size(), self.scale);
        self.note(actions)
    }

    /// Keep `actions`; answer whether they ask for a frame.
    fn note(&mut self, actions: Vec<AppAction>) -> bool {
        let redraw = actions.contains(&AppAction::RequestRedraw);
        self.actions.extend(actions);
        redraw
    }

    /// One turn of the desktop loop after an event: the wake's queues
    /// (`proxy_wake_up`), `AboutToWait`, and the frame if anything asked for
    /// one — `redraw_asked` is whether the event itself did (the shell's
    /// `process_actions` requests that redraw before the loop turns). The
    /// caller clears `signals_changed` before the event, as `proxy_wake_up`
    /// does before its drain, so a signal the event or the drain changed asks
    /// for the frame too. Answers whether this turn painted.
    fn turn(&mut self, redraw_asked: bool) -> bool {
        // The main-thread callbacks, then the work other threads sent this
        // app (#328) — `RinchRuntime::proxy_wake_up`'s order.
        rinch_core::drain_main_callbacks();
        let ran = self.app.run_deferred_work();
        let changed = rinch_core::signals_changed();
        let size = self.physical_size();
        let actions = self
            .app
            .handle_event(PlatformEvent::AboutToWait, size, self.scale);
        let asked = self.note(actions);
        if redraw_asked
            || asked
            || ran > 0
            || changed
            || self.app.has_pending_layout()
            || self.app.scene_dirty
        {
            self.paint();
            true
        } else {
            false
        }
    }

    /// One redraw, as `ShellWindow::paint_software` does it: the queues it
    /// drains first, the layout it resolves, the frame.
    fn paint(&mut self) {
        rinch_core::drain_main_callbacks();
        rinch_core::reactive::drain_polls();
        if self.app.has_pending_layout() {
            self.app
                .resolve_and_repaint(self.size.0 as f32, self.size.1 as f32);
        }
        let transparent = self.app.is_transparent();
        let _ = self.app.build_pixels(self.scale, self.size, transparent);
        self.app.end_perf_frame();
    }

    /// Run turns of the loop until the app stops asking for frames — what
    /// an idle event loop does after the last input — or 16 turns have run (an
    /// animation that is still going asks every turn).
    ///
    /// Every input method already runs one turn; call this after a change made
    /// outside input (a signal written by the test) or to let a short
    /// transition finish.
    pub fn settle(&mut self) {
        for _ in 0..SETTLE_TURNS {
            rinch_core::clear_signals_changed();
            if !self.turn(false) && !rinch_core::main_callbacks_pending() {
                return;
            }
        }
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
    /// `SurfaceResized` does.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        self.send(PlatformEvent::Resized {
            width: self.size.0,
            height: self.size.1,
        });
    }

    /// The window gaining (`true`) or losing keyboard focus.
    pub fn window_focus(&mut self, focused: bool) {
        self.send(PlatformEvent::WindowFocus(focused));
    }

    // ── Pointer ─────────────────────────────────────────────────────────

    /// Move the mouse to `(x, y)`.
    pub fn mouse_move(&mut self, x: f32, y: f32) {
        self.send(PlatformEvent::MouseMove { x, y });
    }

    /// Press `button` at `(x, y)`. The pointer moves there first if it is not
    /// there already: a real button is pressed where the pointer is, and the
    /// shell presses at the position its last move left.
    pub fn mouse_down(&mut self, x: f32, y: f32, button: MouseButton) {
        self.move_unless_there(x, y);
        self.send(PlatformEvent::MouseDown {
            x,
            y,
            button: platform_button(button),
        });
    }

    /// Release `button` at `(x, y)`, moving there first if needed.
    pub fn mouse_up(&mut self, x: f32, y: f32, button: MouseButton) {
        self.move_unless_there(x, y);
        self.send(PlatformEvent::MouseUp {
            x,
            y,
            button: platform_button(button),
        });
    }

    /// A left click at `(x, y)`: press and release.
    pub fn click(&mut self, x: f32, y: f32) {
        self.mouse_down(x, y, MouseButton::Left);
        self.mouse_up(x, y, MouseButton::Left);
    }

    /// A right click at `(x, y)` (the context-menu press).
    pub fn right_click(&mut self, x: f32, y: f32) {
        self.mouse_down(x, y, MouseButton::Right);
        self.mouse_up(x, y, MouseButton::Right);
    }

    /// Move the mouse to the centre of `target`'s box.
    pub fn hover(&mut self, target: impl Target) {
        let (x, y) = self.center(target);
        self.mouse_move(x, y);
    }

    /// Press `button` at the centre of `target`'s box.
    ///
    /// The press goes through hit testing like any other: it lands on
    /// whatever is painted at that point, which is `target` unless something
    /// covers it (or `target` has no box of its own).
    pub fn press(&mut self, target: impl Target, button: MouseButton) {
        let (x, y) = self.center(target);
        self.mouse_down(x, y, button);
    }

    /// Release `button` at the centre of `target`'s box.
    pub fn release(&mut self, target: impl Target, button: MouseButton) {
        let (x, y) = self.center(target);
        self.mouse_up(x, y, button);
    }

    /// A left click at the centre of `target`'s box. See [`Self::press`].
    pub fn click_on(&mut self, target: impl Target) {
        let (x, y) = self.center(target);
        self.click(x, y);
    }

    /// A wheel or trackpad scroll at `(x, y)` of `delta_x`/`delta_y` pixels,
    /// with the DOM's sign: positive `delta_y` scrolls the content down (the
    /// `WheelEvent.deltaY` an `onwheel` handler receives), as the MCP
    /// `scroll` tool takes it. Scrolls directly, like a trackpad.
    pub fn wheel(&mut self, x: f32, y: f32, delta_x: f64, delta_y: f64) {
        self.move_unless_there(x, y);
        // A wheel delta is negative toward "down" (winit's sign, which
        // `handle_event`'s `MouseWheel` arm subtracts from the offset).
        self.send(PlatformEvent::MouseWheel {
            x,
            y,
            delta_x: -delta_x,
            delta_y: -delta_y,
        });
    }

    /// A mouse wheel's notches at `(x, y)`, DOM sign as in [`Self::wheel`]:
    /// the scroll glides, as one from a real wheel does (unless the app
    /// turned smooth wheel scrolling off).
    pub fn wheel_lines(&mut self, x: f32, y: f32, lines_x: f64, lines_y: f64) {
        self.move_unless_there(x, y);
        self.send(PlatformEvent::MouseWheelLines {
            x,
            y,
            lines_x: -lines_x,
            lines_y: -lines_y,
        });
    }

    /// [`Self::wheel`] at the centre of `target`'s box.
    pub fn wheel_on(&mut self, target: impl Target, delta_x: f64, delta_y: f64) {
        let (x, y) = self.center(target);
        self.wheel(x, y, delta_x, delta_y);
    }

    fn move_unless_there(&mut self, x: f32, y: f32) {
        if self.app.cursor_pos != Some((x, y)) {
            self.mouse_move(x, y);
        }
    }

    // ── Keyboard ────────────────────────────────────────────────────────

    /// Hold `modifiers` (and release any not in it), as the shell's
    /// `ModifiersChanged` reports a modifier key. Every later key, click and
    /// wheel sees them held until the next call.
    pub fn set_modifiers(&mut self, modifiers: ModifierState) {
        let ModifierState {
            shift,
            ctrl,
            alt,
            meta,
        } = modifiers;
        self.modifiers = Modifiers {
            shift,
            ctrl,
            alt,
            meta,
        };
        self.send(PlatformEvent::ModifiersChanged(self.modifiers));
    }

    /// The modifiers held.
    pub fn modifiers(&self) -> ModifierState {
        let Modifiers {
            shift,
            ctrl,
            alt,
            meta,
        } = self.modifiers;
        ModifierState {
            shift,
            ctrl,
            alt,
            meta,
        }
    }

    /// Press the key `key` names, with the [held modifiers](Self::set_modifiers).
    ///
    /// Names are the W3C `KeyboardEvent.key` spellings: a single character
    /// (`"a"`, `"A"`, `"/"`) is that character, and `"Enter"`, `"Tab"`,
    /// `"Escape"`, `"Backspace"`, `"Delete"`, `"Space"`, `"ArrowLeft"`,
    /// `"ArrowRight"`, `"ArrowUp"`, `"ArrowDown"`, `"Home"`, `"End"`,
    /// `"PageUp"`, `"PageDown"` and `"F1"`–`"F12"` are those keys — the names
    /// the MCP `key_press` tool takes. With Shift held, a letter types its
    /// capital (`"c"` types `"C"`), as a QWERTY layout does; other characters
    /// type themselves (name `"!"`, not Shift+`"1"`). A menu shortcut matching the chord takes it
    /// first, and the app then sees no key, as with a real keyboard.
    ///
    /// Panics on a name it does not know.
    pub fn key_down(&mut self, key: &str) {
        rinch_core::clear_signals_changed();
        let mut actions = Vec::new();
        let size = self.physical_size();
        let outcome =
            self.app
                .dispatch_named_key_down(key, self.modifiers, &mut actions, size, self.scale);
        assert!(
            outcome != NamedKeyDown::UnknownKey,
            "unknown key name {key:?}"
        );
        let asked = self.note(actions);
        self.turn(asked);
    }

    /// Release the key `key` names (see [`Self::key_down`]).
    pub fn key_up(&mut self, key: &str) {
        let code = keyname_to_keycode(key).unwrap_or_else(|| panic!("unknown key name {key:?}"));
        self.send(PlatformEvent::KeyUp {
            key: code,
            logical_key: named_logical_key(key, self.modifiers.shift),
            modifiers: self.modifiers,
        });
    }

    /// A keystroke: [`Self::key_down`] then [`Self::key_up`].
    pub fn press_key(&mut self, key: &str) {
        self.key_down(key);
        self.key_up(key);
    }

    /// Type `text`, one keystroke per character: `'\n'` is Enter, `'\t'` Tab,
    /// any other character its own key. With the held modifiers, like
    /// [`Self::press_key`].
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

    /// An input method committing `text` to the focused field, as an IME
    /// (or a dictation or emoji picker) does.
    pub fn ime_commit(&mut self, text: &str) {
        self.send(PlatformEvent::Ime(ImeEvent::Commit(text.to_owned())));
    }

    /// Focus `target` as app code does with [`NodeHandle::focus`], then run
    /// a turn.
    pub fn focus(&mut self, target: impl Target) {
        rinch_core::clear_signals_changed();
        self.handle(target).focus();
        self.turn(true);
    }

    // ── Reading the document ────────────────────────────────────────────

    fn doc(&self) -> &Rc<RefCell<RinchDocument>> {
        self.app.doc().expect("a mounted app has a document")
    }

    fn doc_weak(&self) -> Weak<RefCell<dyn DomDocument>> {
        let doc: Rc<RefCell<dyn DomDocument>> = self.doc().clone();
        Rc::downgrade(&doc)
    }

    /// A handle to `target`, for the [`NodeHandle`] API (attributes,
    /// children, scroll offsets, `live_value`, …).
    pub fn handle(&self, target: impl Target) -> NodeHandle {
        NodeHandle::new(target.node_id(self), self.doc_weak())
    }

    /// The first element matching the CSS `selector`, in document order.
    pub fn query(&self, selector: &str) -> Option<NodeHandle> {
        self.query_all(selector).into_iter().next()
    }

    /// Every element matching the CSS `selector`, in document order.
    pub fn query_all(&self, selector: &str) -> Vec<NodeHandle> {
        let ids = self.doc().borrow().query_selector_all(selector);
        let weak = self.doc_weak();
        ids.into_iter()
            .map(|id| NodeHandle::new(id, weak.clone()))
            .collect()
    }

    /// The text in `target`'s subtree (`textContent`).
    pub fn text(&self, target: impl Target) -> String {
        let id = target.node_id(self).0;
        rinch_dom::testing::get_text_content(&self.doc().borrow().tree, id)
    }

    /// `target`'s attribute `name`.
    pub fn attribute(&self, target: impl Target, name: &str) -> Option<String> {
        let id = target.node_id(self).0;
        let d = self.doc().borrow();
        d.tree.get(id)?.attributes.get(name).cloned()
    }

    /// The value of a form control (`<input>`, `<textarea>`, `<select>`) as the
    /// user left it.
    pub fn value(&self, target: impl Target) -> Option<String> {
        let id = target.node_id(self).0;
        let d = self.doc().borrow();
        rinch_dom::form_control::control_value(&d.tree.nodes, id).map(|v| v.into_owned())
    }

    /// One of `target`'s computed style properties, in the form the MCP
    /// `get_computed_styles` tool reports it: `name` is a field of rinch's
    /// computed style (`"display"`, `"background_color"`; a CSS spelling
    /// such as `"background-color"` is accepted), and a value that is not a
    /// plain string comes back as its JSON (`"12.0"`, `"[0,0,0,255]"`).
    ///
    /// **Unstable:** the names and the value spellings are rinch's internal
    /// computed style, and change when it does. Prefer [`Self::bounds`] or an
    /// attribute where they answer the question.
    pub fn computed_style(&self, target: impl Target, name: &str) -> Option<String> {
        let id = target.node_id(self).0;
        let d = self.doc().borrow();
        let style = serde_json::to_value(&d.tree.get(id)?.computed_style).ok()?;
        match style.get(name.replace('-', "_"))? {
            serde_json::Value::String(s) => Some(s.clone()),
            other => Some(other.to_string()),
        }
    }

    /// The box `target` is painted in (its border box), in logical pixels
    /// from the window's top-left — `getBoundingClientRect()`, the box a
    /// click lands in.
    pub fn bounds(&self, target: impl Target) -> ElementBounds {
        let id = target.node_id(self).0;
        let (x, y, width, height) = crate::app::painted_element_box(&self.doc().borrow().tree, id);
        ElementBounds {
            x,
            y,
            width,
            height,
        }
    }

    /// The centre of [`Self::bounds`].
    pub fn center(&self, target: impl Target) -> (f32, f32) {
        let b = self.bounds(target);
        (b.x + b.width / 2.0, b.y + b.height / 2.0)
    }

    /// The element with keyboard focus, if any.
    pub fn focused(&self) -> Option<NodeHandle> {
        let id = self.doc().borrow().active_element()?;
        Some(NodeHandle::new(id, self.doc_weak()))
    }
}
