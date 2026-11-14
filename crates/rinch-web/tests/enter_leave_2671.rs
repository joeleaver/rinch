//! Issues #2671 and #2672 in a real browser: `onmouseenter`/`onmouseleave`
//! (`data-onenter`/`data-onleave`) are the DOM's `mouseenter`/`mouseleave`,
//! as on desktop (`rinch/src/app/enter_leave_2671_tests.rs`).
//!
//! - Moving from an element into its descendant leaves nothing: the element
//!   still holds the pointer (#2671). The web backend used to fire the
//!   outer element's leave there, because it kept only the nearest element
//!   with a handler.
//! - A jump across several levels leaves the old elements innermost first and
//!   enters the new ones outermost first, Chrome's order.
//! - The mouse leaving the document leaves everything it was over (#2672).
//!
//! The pointer is a synthetic `pointermove` at the element under the point,
//! where the browser sends it.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test enter_leave_2671
//! ```
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use std::cell::RefCell;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_ATTR: &str = "data-test-host-2671";

const FIXTURE_CSS: &str = "
[data-test-host-2671] { position: absolute; left: 0; top: 0; width: 400px; height: 300px; }
#outer { position: absolute; left: 20px; top: 20px; width: 300px; height: 200px; background: #eee; }
#mid { position: absolute; left: 20px; top: 20px; width: 200px; height: 120px; background: #ccc; }
#inner { position: absolute; left: 20px; top: 20px; width: 80px; height: 40px; background: #999; }
#side { position: absolute; left: 240px; top: 20px; width: 40px; height: 40px; background: #bbb; }
";

thread_local! {
    static PREVIOUS: RefCell<Option<RootHandle>> = const { RefCell::new(None) };
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn doc() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn take_log() -> Vec<String> {
    LOG.with(|l| std::mem::take(&mut *l.borrow_mut()))
}

fn mount(build: impl Fn(&mut RenderScope) -> NodeHandle + 'static) {
    PREVIOUS.with(|p| {
        if let Some(h) = p.borrow_mut().take() {
            h.unmount();
        }
    });
    while let Ok(Some(el)) = doc().query_selector(&format!("[{HOST_ATTR}]")) {
        el.remove();
    }
    LOG.with(|l| l.borrow_mut().clear());
    let host = doc().create_element("div").unwrap();
    host.set_attribute(HOST_ATTR, "").unwrap();
    let style = doc().create_element("style").unwrap();
    style.set_text_content(Some(FIXTURE_CSS));
    host.append_child(&style).unwrap();
    doc().body().unwrap().append_child(&host).unwrap();
    let handle = rinch_web::mount_into(&host, ThemeProviderProps::default(), build);
    PREVIOUS.with(|p| *p.borrow_mut() = Some(handle));
}

/// A `div#id` that logs `enter:id` and `leave:id`.
fn logged(scope: &mut RenderScope, id: &'static str) -> NodeHandle {
    let el = scope.create_element("div");
    el.set_attribute("id", id);
    let enter =
        scope.register_handler(move || LOG.with(|l| l.borrow_mut().push(format!("enter:{id}"))));
    el.set_attribute("data-onenter", &enter.0.to_string());
    let leave =
        scope.register_handler(move || LOG.with(|l| l.borrow_mut().push(format!("leave:{id}"))));
    el.set_attribute("data-onleave", &leave.0.to_string());
    el
}

/// `#outer` > `#mid` (no handlers) > `#inner`, and `#side` in `#outer`.
fn scene(scope: &mut RenderScope) -> NodeHandle {
    let outer = logged(scope, "outer");
    let mid = scope.create_element("div");
    mid.set_attribute("id", "mid");
    let inner = logged(scope, "inner");
    mid.append_child(&inner);
    outer.append_child(&mid);
    let side = logged(scope, "side");
    outer.append_child(&side);
    outer
}

fn pointer(name: &str, (x, y): (f64, f64), related_null: bool) {
    let target = doc()
        .element_from_point(x as f32, y as f32)
        .unwrap_or_else(|| doc().body().unwrap().into());
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_id(1);
    init.set_is_primary(true);
    init.set_pointer_type("mouse");
    init.set_button(-1);
    init.set_buttons(0);
    init.set_client_x(x as i32);
    init.set_client_y(y as i32);
    if related_null {
        init.set_related_target(None);
    }
    let ev = web_sys::PointerEvent::new_with_event_init_dict(name, &init).unwrap();
    target.dispatch_event(&ev).unwrap();
}

fn move_to(p: (f64, f64)) {
    pointer("pointermove", p, false);
}

// Points (client px; the host sits at the page origin).
const OFF: (f64, f64) = (380.0, 280.0); // the host, outside #outer
const ON_OUTER: (f64, f64) = (30.0, 200.0);
const ON_MID: (f64, f64) = (200.0, 150.0); // #mid, outside #inner
const ON_INNER: (f64, f64) = (70.0, 70.0);
const ON_SIDE: (f64, f64) = (280.0, 60.0);

#[wasm_bindgen_test]
fn a_descendant_does_not_leave_its_ancestor() {
    mount(scene);
    // Positive control: the fixture is where the points say.
    let at = |p: (f64, f64)| {
        doc()
            .element_from_point(p.0 as f32, p.1 as f32)
            .unwrap()
            .id()
    };
    assert_eq!(at(ON_INNER), "inner");
    assert_eq!(at(ON_MID), "mid");
    assert_eq!(at(ON_OUTER), "outer");
    assert_eq!(at(ON_SIDE), "side");

    move_to(OFF);
    take_log();
    move_to(ON_OUTER);
    assert_eq!(take_log(), ["enter:outer"]);
    move_to(ON_INNER);
    assert_eq!(take_log(), ["enter:inner"], "into a descendant");
    move_to(ON_MID);
    assert_eq!(take_log(), ["leave:inner"], "back out to the ancestor");
    move_to(ON_OUTER);
    assert_eq!(
        take_log(),
        Vec::<String>::new(),
        "between an ancestor's own children"
    );
    move_to(OFF);
    assert_eq!(take_log(), ["leave:outer"]);
}

#[wasm_bindgen_test]
fn a_jump_leaves_innermost_first_and_enters_outermost_first() {
    mount(scene);
    move_to(OFF);
    take_log();
    move_to(ON_INNER);
    assert_eq!(take_log(), ["enter:outer", "enter:inner"]);
    move_to(ON_SIDE);
    assert_eq!(take_log(), ["leave:inner", "enter:side"]);
    move_to(ON_INNER);
    assert_eq!(take_log(), ["leave:side", "enter:inner"]);
    move_to(OFF);
    assert_eq!(take_log(), ["leave:inner", "leave:outer"]);
}

#[wasm_bindgen_test]
fn the_mouse_leaving_the_document_leaves_everything() {
    mount(scene);
    move_to(OFF);
    move_to(ON_INNER);
    take_log();
    // What the browser sends as the mouse leaves the page: a `pointerout` at
    // the last element, with no element it went to.
    pointer("pointerout", ON_INNER, true);
    assert_eq!(take_log(), ["leave:inner", "leave:outer"]);
    // Coming back enters again.
    move_to(ON_INNER);
    assert_eq!(take_log(), ["enter:outer", "enter:inner"]);
    // A `pointerout` to another element of the page is not a leave of the
    // document: the move that follows it decides.
    let side = doc().get_element_by_id("side").unwrap();
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_pointer_type("mouse");
    init.set_related_target(Some(&side));
    let ev = web_sys::PointerEvent::new_with_event_init_dict("pointerout", &init).unwrap();
    doc()
        .get_element_by_id("inner")
        .unwrap()
        .dispatch_event(&ev)
        .unwrap();
    assert_eq!(take_log(), Vec::<String>::new());
}
