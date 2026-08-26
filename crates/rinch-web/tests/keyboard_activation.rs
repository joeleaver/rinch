//! Browser-driven tests for keyboard activation of `data-rid` handlers (#240).
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown
//! ```
//!
//! The invariant under test: a `data-rid` handler fires exactly once per user
//! activation, whether that activation is a pointer press (dispatched from
//! `pointerdown`), the browser's own keyboard-synthesised `click` on a native
//! control, or Enter/Space on a focused `tabindex` element (dispatched from
//! `keydown`). The interaction that must never double-fire is a mouse click:
//! `pointerdown` dispatches, and the trailing `click` — including the trusted,
//! `detail == 0` click a `<label>` fires at its control — must be suppressed.
//!
//! A test can only dispatch *untrusted* events, which the gate always lets
//! through (assistive technology and `element.click()` produce exactly those).
//! The suppression cases therefore run under `__set_trust_override(Some(true))`,
//! a test-only seam that makes the gate read every click as trusted.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_core::events::EventHandlerId;
use rinch_web::RootHandle;
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// A mounted root whose single `data-rid` handler counts its dispatches.
struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    count: Rc<Cell<u32>>,
}

impl Fixture {
    /// Mount `build` into a fresh host. `build` receives the scope and the id of
    /// the counting handler, and must place it on some element as `data-rid`.
    fn mount(build: impl FnOnce(&mut RenderScope, EventHandlerId) -> NodeHandle + 'static) -> Self {
        let count = Rc::new(Cell::new(0u32));
        let counter = count.clone();
        let host = document().create_element("div").unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| {
                let id = scope.register_handler(move || counter.set(counter.get() + 1));
                build(scope, id)
            },
        );
        Self { root, host, count }
    }

    fn dispatches(&self) -> u32 {
        self.count.get()
    }

    fn el(&self, id: &str) -> web_sys::HtmlElement {
        document()
            .get_element_by_id(id)
            .unwrap_or_else(|| panic!("no element #{id}"))
            .dyn_into()
            .unwrap()
    }

    fn teardown(self) {
        rinch_web::__set_trust_override(None);
        self.root.unmount();
        self.host.remove();
    }
}

/// `<tag id=.. data-rid=..>` with optional extra attributes.
fn rid_element(
    scope: &mut RenderScope,
    tag: &str,
    id: &str,
    rid: EventHandlerId,
    attrs: &[(&str, &str)],
) -> NodeHandle {
    let el = scope.create_element(tag);
    el.set_attribute("id", id);
    el.set_attribute("data-rid", &rid.0.to_string());
    for (k, v) in attrs {
        el.set_attribute(k, v);
    }
    let text = scope.create_text("target");
    el.append_child(&text);
    el
}

fn centre(el: &web_sys::Element) -> (i32, i32) {
    let r = el.get_bounding_client_rect();
    (
        (r.x() + r.width() / 2.0) as i32,
        (r.y() + r.height() / 2.0) as i32,
    )
}

/// A primary mouse `pointerdown` at the element's centre.
fn pointerdown(el: &web_sys::Element) {
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_id(1);
    init.set_is_primary(true);
    init.set_pointer_type("mouse");
    init.set_button(0);
    init.set_buttons(1);
    let (x, y) = centre(el);
    init.set_client_x(x);
    init.set_client_y(y);
    let ev = web_sys::PointerEvent::new_with_event_init_dict("pointerdown", &init).unwrap();
    el.dispatch_event(&ev).unwrap();
}

/// A `click` with the given `detail`. `detail == 0` is the shape of a
/// keyboard-synthesised, `element.click()`, or `<label>`-activation click,
/// which also carries `clientX = clientY = 0`; a pointer click carries its
/// count and position.
fn click(el: &web_sys::Element, detail: i32) {
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_detail(detail);
    if detail > 0 {
        let (x, y) = centre(el);
        init.set_client_x(x);
        init.set_client_y(y);
    }
    let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict("click", &init).unwrap();
    el.dispatch_event(&ev).unwrap();
}

/// A `keydown` for `key`; returns the event so the test can inspect
/// `default_prevented()`.
fn keydown(el: &web_sys::Element, key: &str, repeat: bool) -> web_sys::KeyboardEvent {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(key);
    init.set_repeat(repeat);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    el.dispatch_event(&ev).unwrap();
    ev
}

// ── The click path ──────────────────────────────────────────────────────────

#[wasm_bindgen_test]
fn a_click_with_no_pointer_press_dispatches_once() {
    // The browser's keyboard-synthesised click on a focused <button>: no
    // pointerdown precedes it. (Also the shape of an AT / element.click() click.)
    let f = Fixture::mount(|scope, rid| rid_element(scope, "button", "btn", rid, &[]));
    let btn = f.el("btn");
    btn.focus().unwrap();
    click(&btn, 0);
    assert_eq!(
        f.dispatches(),
        1,
        "a keyboard-originated click must dispatch data-rid"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn a_mouse_press_and_its_trailing_click_dispatch_once() {
    let f = Fixture::mount(|scope, rid| rid_element(scope, "button", "btn", rid, &[]));
    rinch_web::__set_trust_override(Some(true));
    let btn = f.el("btn");
    pointerdown(&btn);
    assert_eq!(f.dispatches(), 1, "pointerdown dispatches the click");
    click(&btn, 1);
    assert_eq!(
        f.dispatches(),
        1,
        "the trusted click that follows a pointer press must be suppressed"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn a_label_activation_click_after_a_mouse_press_is_not_a_second_dispatch() {
    // Checkbox/Switch/Radio: `<label data-rid>` wrapping the control. A mouse
    // press on the label text dispatches from pointerdown; the browser then
    // fires a detail-1 click on the label AND the label's activation fires a
    // *trusted, detail-0* click at the control, which bubbles back through the
    // label. A `detail === 0` gate would dispatch that one — a double toggle.
    let f = Fixture::mount(|scope, rid| {
        let label = rid_element(scope, "label", "lbl", rid, &[]);
        let input = scope.create_element("input");
        input.set_attribute("type", "checkbox");
        input.set_attribute("id", "cb");
        label.append_child(&input);
        label
    });
    rinch_web::__set_trust_override(Some(true));
    let label = f.el("lbl");
    let cb = f.el("cb");
    pointerdown(&label);
    assert_eq!(f.dispatches(), 1);
    // The browser's own label activation also fires a click at #cb here; the
    // explicit one below keeps the case deterministic across engines.
    click(&label, 1);
    click(&cb, 0);
    assert_eq!(
        f.dispatches(),
        1,
        "the label's detail-0 activation click is part of the same mouse interaction"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn a_keydown_reopens_the_click_path_after_a_mouse_interaction() {
    let f = Fixture::mount(|scope, rid| rid_element(scope, "button", "btn", rid, &[]));
    rinch_web::__set_trust_override(Some(true));
    let btn = f.el("btn");
    pointerdown(&btn);
    click(&btn, 1);
    assert_eq!(f.dispatches(), 1);
    // Enter on the focused button: the keydown marks a keyboard interaction and
    // the browser synthesises the click that activates the button.
    btn.focus().unwrap();
    keydown(&btn, "Enter", false);
    click(&btn, 0);
    assert_eq!(
        f.dispatches(),
        2,
        "a keyboard activation after a mouse click must dispatch again"
    );
    f.teardown();
}

// ── The keydown path (non-native activatables) ──────────────────────────────

#[wasm_bindgen_test]
fn enter_on_a_focused_tabindex_element_dispatches_once() {
    // The `Tree` node shape: a <div tabindex="0" data-rid>. The browser never
    // synthesises a click for it, so keydown is the only activation route.
    let f =
        Fixture::mount(|scope, rid| rid_element(scope, "div", "node", rid, &[("tabindex", "0")]));
    let node = f.el("node");
    node.focus().unwrap();
    keydown(&node, "Enter", false);
    assert_eq!(
        f.dispatches(),
        1,
        "Enter must activate a focused tabindex element"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn space_on_a_focused_tabindex_element_dispatches_and_does_not_scroll() {
    let f =
        Fixture::mount(|scope, rid| rid_element(scope, "div", "node", rid, &[("tabindex", "0")]));
    let node = f.el("node");
    node.focus().unwrap();
    let ev = keydown(&node, " ", false);
    assert_eq!(
        f.dispatches(),
        1,
        "Space must activate a focused tabindex element"
    );
    assert!(
        ev.default_prevented(),
        "the activating Space must be consumed so the page does not scroll"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn space_on_a_tabindex_element_with_no_live_handler_falls_through() {
    // Nothing to activate: the key must not be consumed (Tab, scrolling and
    // the rest of the page's keyboard behaviour stay the browser's).
    let f = Fixture::mount(|scope, rid| {
        let wrapper = rid_element(scope, "div", "owner", rid, &[]);
        let node = scope.create_element("div");
        node.set_attribute("id", "node");
        node.set_attribute("tabindex", "0");
        // A sibling, not a descendant, of the handler element.
        let parent = scope.create_element("div");
        parent.append_child(&wrapper);
        parent.append_child(&node);
        parent
    });
    let node = f.el("node");
    node.focus().unwrap();
    let ev = keydown(&node, " ", false);
    assert_eq!(f.dispatches(), 0);
    assert!(
        !ev.default_prevented(),
        "an unhandled Space must not be consumed"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn enter_in_a_text_input_under_a_handler_does_not_activate_it() {
    // Enter in a text field is a submit gesture (the `data-onsubmit` path),
    // never an activation of the surrounding clickable.
    let f = Fixture::mount(|scope, rid| {
        let wrapper = rid_element(scope, "div", "wrap", rid, &[]);
        let input = scope.create_element("input");
        input.set_attribute("id", "inp");
        input.set_attribute("type", "text");
        wrapper.append_child(&input);
        wrapper
    });
    let input = f.el("inp");
    input.focus().unwrap();
    keydown(&input, "Enter", false);
    assert_eq!(
        f.dispatches(),
        0,
        "Enter in a text field must not activate its ancestor"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn a_held_key_activates_once_per_press() {
    let f =
        Fixture::mount(|scope, rid| rid_element(scope, "div", "node", rid, &[("tabindex", "0")]));
    let node = f.el("node");
    node.focus().unwrap();
    keydown(&node, "Enter", false);
    keydown(&node, "Enter", true);
    assert_eq!(
        f.dispatches(),
        1,
        "auto-repeat keydowns must not re-activate"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn enter_on_a_native_button_is_left_to_the_browser() {
    // The browser synthesises the click for a <button>; the keydown path must
    // stay out of it or the button would fire twice. Dispatching the keydown
    // alone (no click) must therefore dispatch nothing.
    let f = Fixture::mount(|scope, rid| rid_element(scope, "button", "btn", rid, &[]));
    let btn = f.el("btn");
    btn.focus().unwrap();
    keydown(&btn, "Enter", false);
    assert_eq!(
        f.dispatches(),
        0,
        "the keydown path must not activate a natively activatable element"
    );
    f.teardown();
}
