//! Browser-driven tests for the `contextmenu` delegation.
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown
//! ```
//!
//! Two questions that used to be one. **Dispatch** is "does some ancestor carry
//! a live `data-oncontextmenu`"; **suppression** is "does the browser get to
//! open its own menu". The listener answered both on the same branch, so a
//! right-click that found no handler kept the page's default — including on
//! `ContextMenu`'s overlay, which is portalled to `body` and therefore outside
//! the subtree the handler is on, which is exactly where an app rendering its
//! own menu did *not* want the browser's.
//!
//! The four cases below are the whole matrix: a live handler, no handler with
//! the flag off, no handler with the flag on, and a **stale** handler — an
//! attribute outliving the scope that registered it (issue #141), which is not a
//! handler and must not suppress anything on its own.
//!
//! `defaultPrevented` on a synthetic, cancelable `contextmenu` event is the
//! observable: it is what decides whether the browser goes on to open its menu.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_core::events::{EventHandlerId, has_click_handler};
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
const HOST_MARKER: &str = "data-ctxmenu-test-host";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    count: Rc<Cell<u32>>,
}

impl Fixture {
    fn mount(build: impl FnOnce(&mut RenderScope, Rc<Cell<u32>>) -> NodeHandle + 'static) -> Self {
        // The flag is page-global and every wasm test shares one page, so each
        // fixture states the default rather than inheriting whatever the last
        // test left.
        rinch_web::set_suppress_native_context_menu(false);
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

    fn el(&self, id: &str) -> web_sys::Element {
        document()
            .get_element_by_id(id)
            .unwrap_or_else(|| panic!("no element #{id}"))
    }

    fn teardown(self) {
        rinch_web::set_suppress_native_context_menu(false);
        self.root.unmount();
        self.host.remove();
    }
}

/// A `<div id=..>` with the given attributes and some text to right-click on.
fn div(scope: &mut RenderScope, id: &str, attrs: &[(&str, &str)]) -> NodeHandle {
    let el = scope.create_element("div");
    el.set_attribute("id", id);
    for (k, v) in attrs {
        el.set_attribute(k, v);
    }
    let text = scope.create_text("target");
    el.append_child(&text);
    el
}

/// Right-click `el`, returning whether the browser's own menu was suppressed.
fn right_click(el: &web_sys::Element) -> bool {
    let rect = el.get_bounding_client_rect();
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_button(2);
    init.set_buttons(2);
    init.set_client_x((rect.x() + rect.width() / 2.0) as i32);
    init.set_client_y((rect.y() + rect.height() / 2.0) as i32);
    let event = web_sys::MouseEvent::new_with_mouse_event_init_dict("contextmenu", &init).unwrap();
    el.dispatch_event(&event).unwrap();
    event.default_prevented()
}

/// The case that always worked, kept so the restructuring cannot quietly lose
/// it: a live handler both fires and takes the browser's menu.
#[wasm_bindgen_test]
fn a_live_handler_fires_and_suppresses_the_native_menu() {
    let fixture = Fixture::mount(|scope, count| {
        let id = scope.register_handler(move || count.set(count.get() + 1));
        div(
            scope,
            "ctx-live",
            &[("data-oncontextmenu", &id.0.to_string())],
        )
    });

    let target = fixture.el("ctx-live");
    assert!(
        right_click(&target),
        "a handled right-click must not also open the browser's menu"
    );
    assert_eq!(fixture.dispatches(), 1);
    fixture.teardown();
}

/// The default, and why the flag exists: an island hydrated into somebody
/// else's page must leave the right-click alone everywhere it has no handler.
#[wasm_bindgen_test]
fn without_the_flag_an_unhandled_right_click_keeps_the_browsers_menu() {
    let fixture = Fixture::mount(|scope, _| div(scope, "ctx-plain", &[]));

    let target = fixture.el("ctx-plain");
    assert!(
        !right_click(&target),
        "with no handler and the flag off, the right-click belongs to the page"
    );
    assert_eq!(fixture.dispatches(), 0);
    fixture.teardown();
}

/// The defect: a whole-page app rendering its own menus wore the browser's on
/// top of them, because `ContextMenu` portals its overlay to `body` — outside
/// the subtree the handler sits on, so nothing there carries the attribute.
/// This fixture is that shape: a sibling of the handler's element, standing in
/// for the portalled overlay.
#[wasm_bindgen_test]
fn with_the_flag_an_unhandled_right_click_is_suppressed_and_still_dispatches_nothing() {
    let fixture = Fixture::mount(|scope, count| {
        let id = scope.register_handler(move || count.set(count.get() + 1));
        let wrapper = scope.create_element("div");
        wrapper.append_child(&div(
            scope,
            "ctx-handled",
            &[("data-oncontextmenu", &id.0.to_string())],
        ));
        wrapper.append_child(&div(scope, "ctx-overlay", &[]));
        wrapper
    });

    rinch_web::set_suppress_native_context_menu(true);
    assert!(rinch_web::suppresses_native_context_menu());

    let overlay = fixture.el("ctx-overlay");
    assert!(
        right_click(&overlay),
        "the flag suppresses the browser's menu wherever the click lands"
    );
    assert_eq!(
        fixture.dispatches(),
        0,
        "suppressing is not dispatching: nothing here carries a handler"
    );

    // And the handler beside it still fires, so the flag does not shadow it.
    let handled = fixture.el("ctx-handled");
    assert!(right_click(&handled));
    assert_eq!(fixture.dispatches(), 1);

    fixture.teardown();
}

/// A stale `data-oncontextmenu` is not a handler. The attribute outlives the
/// scope that registered it (issue #141), and before the liveness check it both
/// swallowed the browser's menu and dispatched nothing — a right-click that did
/// precisely nothing. The desktop's `dispatch_oncontextmenu` has always filtered
/// on `has_click_handler`; this is the web catching up.
#[wasm_bindgen_test]
fn a_stale_handler_attribute_neither_fires_nor_suppresses() {
    // The real shape rather than an invented id: a root registers a handler and
    // is then unmounted, which deregisters it, while an attribute naming that id
    // lives on somewhere else.
    let dead_id = Rc::new(Cell::new(0usize));
    let sink = dead_id.clone();
    let doomed_host = document().create_element("div").unwrap();
    doomed_host.set_attribute(HOST_MARKER, "").unwrap();
    document()
        .body()
        .unwrap()
        .append_child(&doomed_host)
        .unwrap();
    let doomed = rinch_web::mount_into(
        &doomed_host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let id = scope.register_handler(|| unreachable!("this handler is dead"));
            sink.set(id.0);
            div(scope, "ctx-doomed", &[])
        },
    );
    doomed.unmount();
    doomed_host.remove();

    let dead = EventHandlerId(dead_id.get());
    assert!(
        !has_click_handler(dead),
        "precondition: unmounting the root must have deregistered the handler"
    );

    let fixture = Fixture::mount(move |scope, _| {
        div(
            scope,
            "ctx-stale",
            &[("data-oncontextmenu", &dead.0.to_string())],
        )
    });

    let target = fixture.el("ctx-stale");
    assert!(
        !right_click(&target),
        "a dead handler must not swallow the browser's menu — the desktop \
         answers `false` here and lets the click path run"
    );
    assert_eq!(fixture.dispatches(), 0);
    fixture.teardown();
}
