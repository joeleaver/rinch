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
//! control, an assistive-technology or `element.click()` click with no pointer
//! gesture behind it, or Enter/Space on a focused `tabindex` element
//! (dispatched from `keydown`). The interaction that must never double-fire is
//! a pointer gesture: `pointerdown` dispatches, and the trailing `click` — the
//! trusted, `detail == 0` click a `<label>` fires at its control included —
//! must be suppressed, whatever keys were pressed while the button was held.
//!
//! A test can only dispatch *untrusted* events, which the gate always lets
//! through (`element.click()` produces exactly those). The suppression cases
//! therefore run under `__force_trusted_clicks(true)`, a test-only seam that
//! makes the gate read every click as trusted. Every wasm test shares one
//! page, so `Fixture::mount` resets the page-global activation state and
//! purges the host a failed test left behind before building its own.
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

/// Marks a fixture's host so a later test can purge one a failed test left in
/// the body (a failed assertion never reaches its own `teardown`).
const HOST_MARKER: &str = "data-test-host";

/// A mounted root whose `data-rid` handler counts its dispatches.
struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    count: Rc<Cell<u32>>,
}

impl Fixture {
    /// Mount `build` into a fresh host. `build` receives the scope and the id of
    /// the counting handler, and must place it on some element as `data-rid`.
    fn mount(build: impl FnOnce(&mut RenderScope, EventHandlerId) -> NodeHandle + 'static) -> Self {
        Self::mount_with(move |scope, count| {
            let id = scope.register_handler(move || count.set(count.get() + 1));
            build(scope, id)
        })
    }

    /// Like [`Fixture::mount`], but `build` registers its own handler(s) around
    /// the counter.
    fn mount_with(
        build: impl FnOnce(&mut RenderScope, Rc<Cell<u32>>) -> NodeHandle + 'static,
    ) -> Self {
        rinch_web::__reset_activation_state();
        if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
            for i in 0..stale.length() {
                if let Some(node) = stale.item(i)
                    && let Ok(el) = node.dyn_into::<web_sys::Element>()
                {
                    el.remove();
                }
            }
        }
        let count = Rc::new(Cell::new(0u32));
        let counter = count.clone();
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| build(scope, counter),
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
        rinch_web::__reset_activation_state();
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

fn pointer_event(el: &web_sys::Element, name: &str) {
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_id(1);
    init.set_is_primary(true);
    init.set_pointer_type("mouse");
    init.set_button(0);
    init.set_buttons(if name == "pointerdown" { 1 } else { 0 });
    let (x, y) = centre(el);
    init.set_client_x(x);
    init.set_client_y(y);
    let ev = web_sys::PointerEvent::new_with_event_init_dict(name, &init).unwrap();
    el.dispatch_event(&ev).unwrap();
}

/// A primary mouse `pointerdown` at the element's centre.
fn pointerdown(el: &web_sys::Element) {
    pointer_event(el, "pointerdown");
}

/// The matching `pointerup`.
fn pointerup(el: &web_sys::Element) {
    pointer_event(el, "pointerup");
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

/// Resolve once the current task has ended (a `setTimeout(0)` macrotask) —
/// when the pointer-gesture flag of a consumed trailing click clears.
async fn next_task() {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

// ── The click path ──────────────────────────────────────────────────────────

#[wasm_bindgen_test]
fn a_trusted_click_with_no_pointer_gesture_dispatches_once() {
    // The browser's keyboard-synthesised click on a focused <button>, and an
    // assistive-technology activation on Firefox/WebKit: trusted, and no
    // pointerdown precedes it. Forcing trust is what makes this exercise the
    // gesture arm of the gate rather than the always-open untrusted arm.
    let f = Fixture::mount(|scope, rid| rid_element(scope, "button", "btn", rid, &[]));
    rinch_web::__force_trusted_clicks(true);
    let btn = f.el("btn");
    btn.focus().unwrap();
    click(&btn, 0);
    assert_eq!(
        f.dispatches(),
        1,
        "a trusted click with no pointer gesture in flight must dispatch data-rid"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn an_untrusted_click_dispatches_once() {
    // element.click(): untrusted, always honoured.
    let f = Fixture::mount(|scope, rid| rid_element(scope, "button", "btn", rid, &[]));
    let btn = f.el("btn");
    click(&btn, 0);
    assert_eq!(f.dispatches(), 1);
    f.teardown();
}

#[wasm_bindgen_test]
fn a_mouse_press_and_its_trailing_click_dispatch_once() {
    let f = Fixture::mount(|scope, rid| rid_element(scope, "button", "btn", rid, &[]));
    rinch_web::__force_trusted_clicks(true);
    let btn = f.el("btn");
    pointerdown(&btn);
    assert_eq!(f.dispatches(), 1, "pointerdown dispatches the click");
    pointerup(&btn);
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
    rinch_web::__force_trusted_clicks(true);
    let label = f.el("lbl");
    let cb = f.el("cb");
    pointerdown(&label);
    assert_eq!(f.dispatches(), 1);
    pointerup(&label);
    // The browser's own label activation also fires a click at #cb here; the
    // explicit one below keeps the case deterministic across engines.
    click(&label, 1);
    click(&cb, 0);
    assert_eq!(
        f.dispatches(),
        1,
        "the label's detail-0 activation click is part of the same mouse gesture"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn a_keydown_reopens_the_click_path_after_a_mouse_interaction() {
    let f = Fixture::mount(|scope, rid| rid_element(scope, "button", "btn", rid, &[]));
    rinch_web::__force_trusted_clicks(true);
    let btn = f.el("btn");
    pointerdown(&btn);
    pointerup(&btn);
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

#[wasm_bindgen_test]
fn a_key_pressed_while_the_button_is_held_does_not_reopen_the_click_path() {
    // Escape to cancel a drag, Shift for an axis lock: the press's trailing
    // click is still to come and is still its duplicate.
    let f = Fixture::mount(|scope, rid| rid_element(scope, "button", "btn", rid, &[]));
    rinch_web::__force_trusted_clicks(true);
    let btn = f.el("btn");
    pointerdown(&btn);
    keydown(&btn, "Escape", false);
    pointerup(&btn);
    click(&btn, 1);
    assert_eq!(
        f.dispatches(),
        1,
        "a key pressed mid-press must not turn the trailing click into a second dispatch"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn an_auto_repeat_keydown_does_not_reopen_the_click_path() {
    // A held Shift auto-repeating through a Shift+click (Windows).
    let f = Fixture::mount(|scope, rid| rid_element(scope, "button", "btn", rid, &[]));
    rinch_web::__force_trusted_clicks(true);
    let btn = f.el("btn");
    pointerdown(&btn);
    keydown(&btn, "Shift", true);
    click(&btn, 1);
    assert_eq!(f.dispatches(), 1);
    f.teardown();
}

#[wasm_bindgen_test]
async fn a_trusted_click_after_the_gesture_ended_dispatches() {
    // Firefox/WebKit assistive technology: a trusted click with no pointer or
    // key event of its own, after the user last clicked with the mouse. The
    // gesture ends with its trailing click, so the AT click is not its
    // duplicate.
    let f = Fixture::mount(|scope, rid| rid_element(scope, "button", "btn", rid, &[]));
    rinch_web::__force_trusted_clicks(true);
    let btn = f.el("btn");
    pointerdown(&btn);
    pointerup(&btn);
    click(&btn, 1);
    assert_eq!(f.dispatches(), 1);
    next_task().await;
    click(&btn, 0);
    assert_eq!(
        f.dispatches(),
        2,
        "a trusted, pointer-less click after the gesture ended must dispatch"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn a_pointerless_click_on_a_label_dispatches_once() {
    // AT / `label.click()` on a `<label data-rid>` wrapping a control: the
    // label's own click dispatches, then the browser forwards a click to the
    // control, which bubbles back through the label — the same interaction.
    let f = Fixture::mount(|scope, rid| {
        let label = rid_element(scope, "label", "lbl", rid, &[]);
        let input = scope.create_element("input");
        input.set_attribute("type", "checkbox");
        input.set_attribute("id", "cb");
        label.append_child(&input);
        label
    });
    let label = f.el("lbl");
    label.click();
    assert_eq!(
        f.dispatches(),
        1,
        "the click a label forwards to its control must not dispatch the label's handler again"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn a_click_a_handler_raises_itself_does_not_re_enter_it() {
    // The hidden-input pattern: the handler opens a picker with
    // `input.click()`; that untrusted click bubbles to the same data-rid.
    let f = Fixture::mount_with(|scope, count| {
        let id = scope.register_handler(move || {
            count.set(count.get() + 1);
            if let Some(input) = document().get_element_by_id("hidden") {
                input.dyn_into::<web_sys::HtmlElement>().unwrap().click();
            }
        });
        let wrapper = rid_element(scope, "div", "wrap", id, &[]);
        let input = scope.create_element("input");
        input.set_attribute("id", "hidden");
        input.set_attribute("type", "text");
        wrapper.append_child(&input);
        wrapper
    });
    let wrapper = f.el("wrap");
    pointerdown(&wrapper);
    assert_eq!(
        f.dispatches(),
        1,
        "a click raised from inside the handler must not run it a second time"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn a_freed_handler_is_walked_past_by_the_pointer_path() {
    // Issue #141: a `data-rid` whose handler is gone stays on the node; the
    // live ancestor must receive the press by mouse as it does by keyboard.
    let f = Fixture::mount(|scope, rid| {
        let card = rid_element(scope, "div", "card", rid, &[]);
        let inner = scope.create_element("div");
        inner.set_attribute("id", "inner");
        inner.set_attribute("data-rid", &usize::MAX.to_string());
        inner.set_attribute("tabindex", "0");
        card.append_child(&inner);
        card
    });
    let inner = f.el("inner");
    pointerdown(&inner);
    assert_eq!(f.dispatches(), 1, "the mouse must reach the live ancestor");
    inner.focus().unwrap();
    keydown(&inner, "Enter", false);
    assert_eq!(f.dispatches(), 2, "so must the keyboard");
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
fn a_held_key_activates_once_and_is_consumed_for_the_whole_press() {
    let f =
        Fixture::mount(|scope, rid| rid_element(scope, "div", "node", rid, &[("tabindex", "0")]));
    let node = f.el("node");
    node.focus().unwrap();
    keydown(&node, " ", false);
    let repeat = keydown(&node, " ", true);
    assert_eq!(
        f.dispatches(),
        1,
        "auto-repeat keydowns must not re-activate"
    );
    assert!(
        repeat.default_prevented(),
        "an auto-repeat of the activating Space must be consumed too, or the page scrolls"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn space_on_a_tabindex_element_with_no_live_handler_falls_through() {
    // Nothing to activate: the key must not be consumed (Tab, scrolling and
    // the rest of the page's keyboard behaviour stay the browser's) — on the
    // press and on its repeats alike.
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
    let repeat = keydown(&node, " ", true);
    assert_eq!(f.dispatches(), 0);
    assert!(
        !ev.default_prevented() && !repeat.default_prevented(),
        "an unhandled Space must not be consumed"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn a_focusable_without_tabindex_under_a_handler_falls_through() {
    // A keyboard-focusable scroll container inside a clickable card: the
    // browser focuses it on its own, and Space there is a page-down, not an
    // activation of the card (desktop's `tabindex`-only rule).
    let f = Fixture::mount(|scope, rid| {
        let card = rid_element(scope, "div", "card", rid, &[]);
        let scroller = scope.create_element("div");
        scroller.set_attribute("id", "scroller");
        scroller.set_attribute("style", "overflow: auto; height: 20px");
        let filler = scope.create_element("div");
        filler.set_attribute("style", "height: 200px");
        scroller.append_child(&filler);
        card.append_child(&scroller);
        card
    });
    let scroller = f.el("scroller");
    let ev = keydown(&scroller, " ", false);
    assert_eq!(
        f.dispatches(),
        0,
        "Space on a focusable without tabindex must not activate the card"
    );
    assert!(!ev.default_prevented(), "and must be left to the browser");
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
fn a_tabindex_node_inside_an_editable_host_does_not_activate() {
    // Keys inside an editing host are the editor's, whatever sits inside it.
    let f = Fixture::mount(|scope, rid| {
        let host = scope.create_element("div");
        host.set_attribute("contenteditable", "true");
        let node = rid_element(scope, "div", "node", rid, &[("tabindex", "0")]);
        host.append_child(&node);
        host
    });
    let node = f.el("node");
    keydown(&node, "Enter", false);
    assert_eq!(f.dispatches(), 0);
    f.teardown();
}

#[wasm_bindgen_test]
fn a_contenteditable_false_node_still_activates() {
    // `contenteditable="false"` is an opt-out of editing, not a text control.
    let f = Fixture::mount(|scope, rid| {
        rid_element(
            scope,
            "div",
            "node",
            rid,
            &[("tabindex", "0"), ("contenteditable", "false")],
        )
    });
    let node = f.el("node");
    node.focus().unwrap();
    keydown(&node, "Enter", false);
    assert_eq!(
        f.dispatches(),
        1,
        "a non-editable tabindex node must activate by keyboard"
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
    let enter = keydown(&btn, "Enter", false);
    let space = keydown(&btn, " ", false);
    assert_eq!(
        f.dispatches(),
        0,
        "the keydown path must not activate a natively activatable element"
    );
    assert!(!enter.default_prevented() && !space.default_prevented());
    f.teardown();
}

#[wasm_bindgen_test]
fn space_on_a_link_dispatches_through_rinch_and_enter_is_left_to_the_browser() {
    // Browser activation is per (element, key): <a href> activates on Enter
    // only, so Space on a focused NavLink goes through the keydown path
    // (desktop parity), consumed so the page does not scroll.
    let f = Fixture::mount(|scope, rid| {
        rid_element(scope, "a", "link", rid, &[("href", "#somewhere")])
    });
    let link = f.el("link");
    link.focus().unwrap();
    let space = keydown(&link, " ", false);
    assert_eq!(f.dispatches(), 1, "Space on a link must activate it");
    assert!(space.default_prevented(), "and must not scroll the page");
    let enter = keydown(&link, "Enter", false);
    assert_eq!(
        f.dispatches(),
        1,
        "Enter on a link is the browser's: its synthesised click dispatches, the keydown does not"
    );
    assert!(
        !enter.default_prevented(),
        "so that the link still navigates"
    );
    f.teardown();
}
