//! Second-round fixtures for #1146 (PR #1580's review): touch presses, the
//! scroll a press must not make, a `<select>` inside a clickable, a draggable
//! tap inside a `data-nofocus` region, and `PasswordInput`'s visibility toggle.
//!
//! Synthetic presses make the browser focus nothing, so every focus asserted
//! here is one rinch made, and a press left to the browser is pinned at
//! `defaultPrevented`. The positive control of each fixture is that the
//! handler ran (a genuine event reached rinch's delegation).
//!
//! **Mount through `rinch_web::mount_into`**: without the owner `mount_tree`
//! pushes, component effects are disposed at once.
#![cfg(target_arch = "wasm32")]

use rinch::components::PasswordInput;
use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_MARKER: &str = "data-test-host-1580-round2";

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

type Log = Rc<RefCell<Vec<String>>>;

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    log: Log,
}

/// `#field` (a text field), then:
/// - `#btn`, a `<button>` with a handler;
/// - `#mover`, a `<button>` whose handler focuses `#other`, and `#other`;
/// - `#scroller` (100px tall, `overflow: auto`) holding a 400px spacer and
///   then `#deep`, a `<button>` with a handler, out of view;
/// - `#selrow`, a handler holding `#sel`, a `<select>`;
/// - `#nfbar` (`data-nofocus`) holding `#drag`, a `draggable` `tabindex` row
///   with a handler;
/// - a `PasswordInput` (`#pw`) with an `ontoggle`.
fn mount() -> Fixture {
    rinch_web::__reset_activation_state();
    if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
        for i in 0..stale.length() {
            if let Some(n) = stale.item(i)
                && let Ok(el) = n.dyn_into::<web_sys::Element>()
            {
                el.remove();
            }
        }
    }
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST_MARKER, "").unwrap();
    host.set_attribute("style", "width: 300px; margin: 20px")
        .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let sink = log.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let el = |scope: &mut RenderScope, tag: &str, id: &str, handler: bool| -> NodeHandle {
                let e = scope.create_element(tag);
                e.set_attribute("id", id);
                e.append_child(&scope.create_text(id));
                if handler {
                    let sink = sink.clone();
                    let name = id.to_string();
                    let rid = scope.register_handler(move || sink.borrow_mut().push(name.clone()));
                    e.set_attribute("data-rid", &rid.0.to_string());
                }
                e
            };
            let field = scope.create_element("input");
            field.set_attribute("id", "field");
            root.append_child(&field);

            root.append_child(&el(scope, "button", "btn", true));

            let mover = el(scope, "button", "mover", false);
            {
                let sink = sink.clone();
                let rid = scope.register_handler(move || {
                    sink.borrow_mut().push("mover".into());
                    by_id("other").focus().unwrap();
                });
                mover.set_attribute("data-rid", &rid.0.to_string());
            }
            root.append_child(&mover);
            root.append_child(&el(scope, "button", "other", false));

            let scroller = el(scope, "div", "scroller", false);
            scroller.set_attribute("style", "height: 100px; overflow: auto");
            let spacer = scope.create_element("div");
            spacer.set_attribute("style", "height: 400px");
            scroller.append_child(&spacer);
            scroller.append_child(&el(scope, "button", "deep", true));
            root.append_child(&scroller);

            let selrow = el(scope, "div", "selrow", true);
            let sel = scope.create_element("select");
            sel.set_attribute("id", "sel");
            let opt = scope.create_element("option");
            opt.append_child(&scope.create_text("one"));
            sel.append_child(&opt);
            selrow.append_child(&sel);
            root.append_child(&selrow);

            let nfbar = el(scope, "div", "nfbar", false);
            nfbar.set_attribute("data-nofocus", "");
            let drag = el(scope, "div", "drag", true);
            drag.set_attribute("draggable", "true");
            drag.set_attribute("tabindex", "0");
            nfbar.append_child(&drag);
            root.append_child(&nfbar);

            let pw_sink = sink.clone();
            let pw = PasswordInput {
                ontoggle: Some(rinch_core::Callback::new(move || {
                    pw_sink.borrow_mut().push("toggle".into())
                })),
                ..Default::default()
            }
            .render(scope, &[]);
            pw.set_attribute("id", "pw");
            root.append_child(&pw);
            root
        },
    );
    Fixture { root, host, log }
}

impl Fixture {
    fn focus(&self, id: &str) {
        by_id(id).focus().unwrap();
        assert_eq!(active_id(), id, "positive control: #{id} is focused");
        self.log.borrow_mut().clear();
    }
    fn teardown(self) {
        rinch_web::__reset_activation_state();
        self.root.unmount();
        self.host.remove();
    }
}

fn by_id(id: &str) -> web_sys::HtmlElement {
    document()
        .get_element_by_id(id)
        .unwrap()
        .dyn_into()
        .unwrap()
}

fn active_id() -> String {
    document()
        .active_element()
        .map(|a| a.id())
        .unwrap_or_default()
}

fn pointer(
    el: &web_sys::Element,
    kind: &str,
    pointer_type: &str,
    dy: f64,
) -> web_sys::PointerEvent {
    let r = el.get_bounding_client_rect();
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_id(if pointer_type == "mouse" { 1 } else { 7 });
    init.set_is_primary(true);
    init.set_pointer_type(pointer_type);
    init.set_button(if kind == "pointermove" { -1 } else { 0 });
    init.set_buttons(if kind == "pointerdown" || kind == "pointermove" {
        1
    } else {
        0
    });
    init.set_client_x((r.x() + r.width() / 2.0) as i32);
    init.set_client_y((r.y() + r.height() / 2.0 + dy) as i32);
    let ev = web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap();
    el.dispatch_event(&ev).unwrap();
    ev
}

fn down(el: &web_sys::Element, pointer_type: &str) -> web_sys::PointerEvent {
    pointer(el, "pointerdown", pointer_type, 0.0)
}

fn up(el: &web_sys::Element, pointer_type: &str) {
    pointer(el, "pointerup", pointer_type, 0.0);
}

/// A finger that lands on a button and pans moves no focus: Chrome sends a
/// `pointercancel` when the pan starts and changes nothing, and blurring the
/// field would dismiss a phone's soft keyboard. The click still fires on
/// contact (#1582).
#[wasm_bindgen_test]
fn a_touch_pan_that_starts_on_a_button_leaves_the_focus_alone() {
    let f = mount();
    f.focus("field");
    let btn = by_id("btn");
    down(&btn, "touch");
    assert_eq!(
        *f.log.borrow(),
        ["btn"],
        "positive control: the handler ran"
    );
    assert_eq!(active_id(), "field", "nothing moves on contact");
    pointer(&btn, "pointercancel", "touch", 0.0);
    up(&btn, "touch");
    assert_eq!(active_id(), "field");
    f.teardown();
}

/// The same pan when the browser has not cancelled the pointer yet: a move
/// past the scroll slop ends the pending focus.
#[wasm_bindgen_test]
fn a_touch_press_that_moves_past_the_slop_leaves_the_focus_alone() {
    let f = mount();
    f.focus("field");
    let btn = by_id("btn");
    down(&btn, "touch");
    pointer(&btn, "pointermove", "touch", 40.0);
    up(&btn, "touch");
    assert_eq!(*f.log.borrow(), ["btn"]);
    assert_eq!(active_id(), "field");
    f.teardown();
}

/// A touch tap focuses the button on release, as Chrome does on a tap.
#[wasm_bindgen_test]
fn a_touch_tap_on_a_button_focuses_it_on_release() {
    let f = mount();
    f.focus("field");
    let btn = by_id("btn");
    down(&btn, "touch");
    assert_eq!(active_id(), "field", "not on contact");
    up(&btn, "touch");
    assert_eq!(*f.log.borrow(), ["btn"]);
    assert_eq!(active_id(), "btn");
    f.teardown();
}

/// A tap whose handler moved the focus itself keeps that move: the held-back
/// focus is not made over it on release.
#[wasm_bindgen_test]
fn a_touch_tap_whose_handler_moves_the_focus_keeps_that_move() {
    let f = mount();
    f.focus("field");
    let mover = by_id("mover");
    down(&mover, "touch");
    up(&mover, "touch");
    assert_eq!(*f.log.borrow(), ["mover"]);
    assert_eq!(active_id(), "other");
    f.teardown();
}

/// A press focuses without scrolling (`preventScroll`): `#deep` sits below its
/// scroller's visible box, and a plain `focus()` scrolls it into view.
#[wasm_bindgen_test]
fn a_press_focuses_without_scrolling() {
    let f = mount();
    f.focus("field");
    let scroller = by_id("scroller");
    assert_eq!(scroller.scroll_top(), 0);
    down(&by_id("deep"), "mouse");
    assert_eq!(*f.log.borrow(), ["deep"]);
    assert_eq!(active_id(), "deep", "positive control: it took the focus");
    assert_eq!(scroller.scroll_top(), 0, "and nothing scrolled");
    // The scroll the guard prevents is real: the same focus without it moves.
    by_id("field").focus().unwrap();
    by_id("deep").focus().unwrap();
    assert!(scroller.scroll_top() > 0, "a plain focus() scrolls here");
    f.teardown();
}

/// A `<select>` inside a clickable is the browser's: its press is not
/// prevented, so its popup opens natively alongside the handler.
#[wasm_bindgen_test]
fn a_select_inside_a_clickable_is_left_to_the_browser() {
    let f = mount();
    f.focus("field");
    let ev = down(&by_id("sel"), "mouse");
    assert_eq!(*f.log.borrow(), ["selrow"]);
    assert!(!ev.default_prevented());
    assert_eq!(active_id(), "field", "rinch did not focus it either");
    f.teardown();
}

/// A tap on a draggable inside a `data-nofocus` region clicks on release and
/// moves no focus: only a `pointerdown` that was not prevented decides one.
#[wasm_bindgen_test]
fn a_draggable_tap_in_a_nofocus_region_moves_no_focus() {
    let f = mount();
    f.focus("field");
    let drag = by_id("drag");
    down(&drag, "mouse");
    up(&drag, "mouse");
    assert_eq!(*f.log.borrow(), ["drag"], "the click fired on release");
    assert_eq!(active_id(), "field");
    f.teardown();
}

/// `PasswordInput`'s visibility toggle takes the click and not the keyboard
/// (`data-nofocus`, Mantine's `preventDefault` on its mousedown): the field
/// keeps the focus and the caret.
#[wasm_bindgen_test]
fn the_password_toggle_leaves_the_field_focused() {
    let f = mount();
    let input = document()
        .query_selector("#pw input")
        .unwrap()
        .unwrap()
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap();
    input.set_id("pwfield");
    f.focus("pwfield");
    let toggle = document()
        .query_selector("#pw .rinch-password-input__toggle")
        .unwrap()
        .unwrap();
    let ev = down(&toggle, "mouse");
    up(&toggle, "mouse");
    assert_eq!(*f.log.borrow(), ["toggle"]);
    assert!(
        ev.default_prevented(),
        "the browser's own focus change is cancelled"
    );
    assert_eq!(active_id(), "pwfield");
    f.teardown();
}

/// A touch tap inside a scroller defers its click to the release
/// (`PendingClick`); its focus goes with it, before the handler, and does not
/// scroll.
#[wasm_bindgen_test]
fn a_deferred_touch_tap_focuses_on_release() {
    let f = mount();
    f.focus("field");
    let deep = by_id("deep");
    down(&deep, "touch");
    assert!(f.log.borrow().is_empty(), "the click waits for the release");
    assert_eq!(active_id(), "field");
    up(&deep, "touch");
    assert_eq!(*f.log.borrow(), ["deep"]);
    assert_eq!(active_id(), "deep");
    assert_eq!(by_id("scroller").scroll_top(), 0);
    f.teardown();
}
