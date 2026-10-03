//! Browser-driven tests for `NodeHandle::select()` / `set_selection_range()`
//! on the web backend (issue #552).
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown
//! ```
//!
//! The invariant under test: `WebDocument::set_selection_range` calls the
//! control's own `setSelectionRange(start, end, direction)` directly — the
//! same UTF-16 units `start`/`end` already are in JS, so this is the backend
//! with nothing to convert — and `select_text`'s trait default (used by
//! `NodeHandle::select()`) selects the control's whole text.

#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{NodeHandle, RenderScope, SelectionDirection};
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-552";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: NodeHandle,
}

impl Fixture {
    fn mount(attrs: &'static [(&'static str, &'static str)]) -> Self {
        if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
            for i in 0..stale.length() {
                if let Some(node) = stale.item(i)
                    && let Ok(el) = node.dyn_into::<web_sys::Element>()
                {
                    el.remove();
                }
            }
        }
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let slot: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
        let slot_in = slot.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| {
                let el = scope.create_element("input");
                el.set_attribute("id", "field");
                for (k, v) in attrs {
                    el.set_attribute(k, v);
                }
                *slot_in.borrow_mut() = Some(el.clone());
                el
            },
        );
        let handle = slot.borrow_mut().take().expect("the control's handle");
        Self { root, host, handle }
    }

    fn input(&self) -> web_sys::HtmlInputElement {
        document()
            .get_element_by_id("field")
            .expect("no #field")
            .dyn_into()
            .unwrap()
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

fn is_active(el: &web_sys::Element) -> bool {
    document().active_element().as_ref() == Some(el)
}

fn selection(input: &web_sys::HtmlInputElement) -> (Option<u32>, Option<u32>, Option<String>) {
    (
        input.selection_start().unwrap(),
        input.selection_end().unwrap(),
        input.selection_direction().unwrap(),
    )
}

/// `set_selection_range` reaches the real DOM control, direction included —
/// Chrome's own `setSelectionRange` units, which is exactly what this
/// backend passes through with no conversion.
#[wasm_bindgen_test]
fn set_selection_range_reaches_the_control() {
    let f = Fixture::mount(&[("type", "text"), ("value", "hello")]);

    f.handle
        .set_selection_range(1, 4, SelectionDirection::Backward);

    let (start, end, direction) = selection(&f.input());
    assert_eq!(
        (start, end, direction.as_deref()),
        (Some(1), Some(4), Some("backward")),
        "a [1, 4) backward range must land on the real control"
    );
    f.teardown();
}

/// `set_selection_range` works on a control that has **not** been focused —
/// a browser's own `setSelectionRange` sets the property regardless, and so
/// must rinch's (issue #552's "selecting a non-focused field" case, web side:
/// there is no stash to prove here, since the browser itself is the stash).
#[wasm_bindgen_test]
fn set_selection_range_works_before_the_control_is_focused() {
    let f = Fixture::mount(&[("type", "text"), ("value", "hello")]);
    assert!(
        !is_active(&f.input().dyn_into::<web_sys::Element>().unwrap()),
        "the field must start unfocused for this test"
    );

    f.handle
        .set_selection_range(2, 5, SelectionDirection::Forward);

    let (start, end, _) = selection(&f.input());
    assert_eq!((start, end), (Some(2), Some(5)));
    f.teardown();
}

/// `select()` selects the control's entire text — `select_text`'s trait
/// default, which asks `live_value` for the UTF-16 length. A surrogate pair
/// in the value (an emoji: 2 UTF-16 units) would catch a mutant that
/// confused UTF-16 units with `.chars().count()` or byte length.
#[wasm_bindgen_test]
fn select_selects_the_whole_value() {
    let f = Fixture::mount(&[("type", "text"), ("value", "ab\u{1F642}cd")]);

    f.handle.select();

    let value = f.input().value();
    let want_end = value.encode_utf16().count() as u32;
    let (start, end, _) = selection(&f.input());
    assert_eq!((start, end), (Some(0), Some(want_end)));
    f.teardown();
}
