//! Browser-driven tests for `data-nofocus` (#312).
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown
//! ```
//!
//! The invariant: a press inside a `data-nofocus` region has its `pointerdown`
//! default prevented — which is how a browser is told "deliver the click, skip
//! the focus change" — while a `data-rid` handler under it still fires. A
//! press anywhere else is untouched.
//!
//! **A `data-rid` element was already covered, incidentally.** `dispatch_rid`
//! calls `prevent_default()` on the pointerdown whenever a handler actually
//! ran — for text selection and `<label>` reasons, not for focus — and that
//! suppresses the compatibility mouse events, and so the focus change with
//! them. So a rinch editor toolbar button on the web has never had #312's
//! defect, unlike its desktop twin. What was *not* covered is everything with
//! no live handler under the press: the toolbar chrome around the buttons, a
//! custom focusable that routes its activation some other way, a button whose
//! handler was freed with its scope. `data-nofocus` states the intent instead
//! of inheriting it from an unrelated mechanism, and makes the attribute mean
//! the same thing on both backends.
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
const HOST_MARKER: &str = "data-nofocus-test-host";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    count: Rc<Cell<u32>>,
}

impl Fixture {
    fn mount(build: impl FnOnce(&mut RenderScope, EventHandlerId) -> NodeHandle + 'static) -> Self {
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
            move |scope: &mut RenderScope| {
                let c = counter.clone();
                let id = scope.register_handler(move || c.set(c.get() + 1));
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
        rinch_web::__reset_activation_state();
        self.root.unmount();
        self.host.remove();
    }
}

/// A toolbar carrying `data-nofocus`, holding a handler-less button and a
/// `data-rid` one, plus two controls outside it.
///
/// The handler-less buttons are what isolate `data-nofocus`: a live `data-rid`
/// suppresses the default on its own (see the module note), so a button with
/// one proves nothing about this attribute either way.
fn toolbar_fixture() -> Fixture {
    Fixture::mount(|scope, rid| {
        let root = scope.create_element("div");
        let button = |scope: &mut RenderScope, id: &str| {
            let b = scope.create_element("button");
            b.set_attribute("id", id);
            let label = scope.create_text("B");
            b.append_child(&label);
            b
        };

        let toolbar = scope.create_element("div");
        toolbar.set_attribute("id", "toolbar");
        toolbar.set_attribute("data-nofocus", "");
        let guarded = button(scope, "guarded");
        let guarded_with_rid = button(scope, "guarded-rid");
        guarded_with_rid.set_attribute("data-rid", &rid.0.to_string());
        toolbar.append_child(&guarded);
        toolbar.append_child(&guarded_with_rid);

        let plain = button(scope, "plain");
        let opted_out = button(scope, "opted-out");
        opted_out.set_attribute("data-nofocus", "false");

        root.append_child(&toolbar);
        root.append_child(&plain);
        root.append_child(&opted_out);
        root
    })
}

/// A primary mouse `pointerdown` at the element's centre. Returns the event so
/// the test can read `default_prevented()`.
fn pointerdown(el: &web_sys::Element) -> web_sys::PointerEvent {
    let r = el.get_bounding_client_rect();
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_id(1);
    init.set_is_primary(true);
    init.set_pointer_type("mouse");
    init.set_button(0);
    init.set_buttons(1);
    init.set_client_x((r.x() + r.width() / 2.0) as i32);
    init.set_client_y((r.y() + r.height() / 2.0) as i32);
    let ev = web_sys::PointerEvent::new_with_event_init_dict("pointerdown", &init).unwrap();
    el.dispatch_event(&ev).unwrap();
    ev
}

#[wasm_bindgen_test]
fn a_press_inside_a_nofocus_region_is_default_prevented() {
    let f = toolbar_fixture();
    // No `data-rid` under this press, so nothing but `data-nofocus` could have
    // suppressed the default.
    let ev = pointerdown(&f.el("guarded"));

    assert!(
        ev.default_prevented(),
        "the browser is told to skip the focus change"
    );
    assert_eq!(f.dispatches(), 0, "and nothing was dispatched to do it");
    f.teardown();
}

#[wasm_bindgen_test]
fn a_press_outside_one_is_untouched() {
    let f = toolbar_fixture();
    let ev = pointerdown(&f.el("plain"));

    assert!(
        !ev.default_prevented(),
        "an ordinary button focuses as the browser intends"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn a_guarded_button_still_dispatches_its_handler() {
    let f = toolbar_fixture();
    let ev = pointerdown(&f.el("guarded-rid"));

    assert!(ev.default_prevented());
    assert_eq!(
        f.dispatches(),
        1,
        "suppressing the focus change must not suppress the click"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn the_false_value_opts_out() {
    let f = toolbar_fixture();
    let ev = pointerdown(&f.el("opted-out"));

    assert!(
        !ev.default_prevented(),
        "`data-nofocus=\"false\"` is the documented opt-out, same rule as `disabled`"
    );
    f.teardown();
}
