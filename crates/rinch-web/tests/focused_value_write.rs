//! Browser-driven tests for programmatic value writes to a focused control
//! (issue #238).
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown
//! ```
//!
//! The invariant under test: `set_attribute("value")` on the control that
//! holds focus — what a `value_fn` effect or a normalizing `oninput` handler
//! does — lands in the live `.value` without throwing the caret to the end.
//! The selection is mapped through the rewrite (a kept prefix keeps the caret,
//! a kept suffix keeps the caret's distance from the end, a rewritten middle
//! puts it after the rewrite), in UTF-16 units as the browser counts them.
//! During an IME composition the write is deferred to `compositionend`, and a
//! control type without a selection API (`number`) must simply take the value.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{NodeHandle, RenderScope};
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

/// Marks a fixture's host so a later test can purge one a failed test left in
/// the body (a failed assertion never reaches its own `teardown`).
const HOST_MARKER: &str = "data-test-host-238";

/// A mounted root holding one form control, plus the rinch `NodeHandle` for
/// it — the trait path a component's `value_fn` effect writes through.
struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: NodeHandle,
}

impl Fixture {
    fn mount(tag: &'static str, attrs: &'static [(&'static str, &'static str)]) -> Self {
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
                let el = scope.create_element(tag);
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

    fn textarea(&self) -> web_sys::HtmlTextAreaElement {
        document()
            .get_element_by_id("field")
            .expect("no #field")
            .dyn_into()
            .unwrap()
    }

    /// The component write: `set_attribute("value")` through the trait.
    fn write(&self, value: &str) {
        self.handle.set_attribute("value", value);
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

fn composition(el: &web_sys::Element, name: &str) {
    let init = web_sys::CompositionEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    let ev = web_sys::CompositionEvent::new_with_event_init_dict(name, &init).unwrap();
    el.dispatch_event(&ev).unwrap();
}

#[wasm_bindgen_test]
fn a_write_to_the_focused_field_keeps_the_caret() {
    let f = Fixture::mount("input", &[("type", "text"), ("value", "abcd")]);
    let input = f.input();
    input.focus().unwrap();
    assert!(is_active(&input), "the input must hold focus for this test");
    input.set_selection_range(2, 2).unwrap();

    // The normalize-on-input shape: same length, prefix rewritten.
    f.write("ABCD");

    assert_eq!(input.value(), "ABCD");
    let (start, end, _) = selection(&input);
    assert_eq!(
        (start, end),
        (Some(2), Some(2)),
        "the caret stays before the third character"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn a_selection_and_its_direction_survive_a_write() {
    let f = Fixture::mount("input", &[("type", "text"), ("value", "abcd")]);
    let input = f.input();
    input.focus().unwrap();
    input
        .set_selection_range_with_direction(1, 3, "backward")
        .unwrap();

    // An appended suffix keeps the prefix, so the selection is untouched.
    f.write("abcd!");

    assert_eq!(input.value(), "abcd!");
    assert_eq!(
        selection(&input),
        (Some(1), Some(3), Some("backward".into())),
        "anchor, head and direction all survive"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn a_rewritten_middle_puts_the_caret_after_the_rewrite() {
    let f = Fixture::mount("input", &[("type", "text"), ("value", "12-34")]);
    let input = f.input();
    input.focus().unwrap();
    // Caret inside the part that gets rewritten.
    input.set_selection_range(3, 3).unwrap();

    f.write("12/34");

    assert_eq!(input.value(), "12/34");
    assert_eq!(
        selection(&input).0,
        Some(3),
        "after the rewritten middle, before the kept suffix"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn the_caret_is_mapped_in_utf16_units() {
    // "é" is one UTF-16 unit (2 UTF-8 bytes); "€" is one unit (3 bytes).
    let f = Fixture::mount("input", &[("type", "text"), ("value", "héllo")]);
    let input = f.input();
    input.focus().unwrap();
    input.set_selection_range(2, 2).unwrap(); // after "é"

    f.write("h€llo");

    assert_eq!(input.value(), "h€llo");
    assert_eq!(
        selection(&input).0,
        Some(2),
        "after the replacement character"
    );

    // A surrogate pair: "😀" is two UTF-16 units (4 bytes). Caret before "b";
    // inserting a second emoji ahead of it keeps the caret before "b".
    f.write("a😀b");
    input.set_selection_range(3, 3).unwrap();
    f.write("a😀😀b");
    assert_eq!(input.value(), "a😀😀b");
    assert_eq!(selection(&input).0, Some(5), "still before 'b'");
    f.teardown();
}

#[wasm_bindgen_test]
fn a_number_input_takes_the_value_without_throwing() {
    // `selectionStart`/`setSelectionRange` are not applicable to type=number;
    // the write must land without a selection round-trip.
    let f = Fixture::mount("input", &[("type", "number"), ("value", "12")]);
    let input = f.input();
    input.focus().unwrap();

    f.write("123");

    assert_eq!(input.value(), "123");
    f.teardown();
}

#[wasm_bindgen_test]
fn a_write_to_an_unfocused_field_lands_as_before() {
    let f = Fixture::mount("input", &[("type", "text"), ("value", "abcd")]);
    let input = f.input();
    input.blur().unwrap();
    assert!(!is_active(&input));

    f.write("ABCD");

    assert_eq!(input.value(), "ABCD");
    f.teardown();
}

#[wasm_bindgen_test]
fn a_textarea_write_keeps_the_caret() {
    let f = Fixture::mount("textarea", &[]);
    let ta = f.textarea();
    f.write("abcd");
    ta.focus().unwrap();
    ta.set_selection_range(2, 2).unwrap();

    f.write("ABCD");

    assert_eq!(ta.value(), "ABCD");
    assert_eq!(ta.selection_start().unwrap(), Some(2));
    f.teardown();
}

#[wasm_bindgen_test]
fn a_write_during_a_composition_is_deferred_to_compositionend() {
    let f = Fixture::mount("input", &[("type", "text"), ("value", "abc")]);
    let input = f.input();
    input.focus().unwrap();
    input.set_selection_range(1, 1).unwrap();

    composition(&input, "compositionstart");
    f.write("ABC");
    assert_eq!(
        input.value(),
        "abc",
        "the write is held back while the composition is in flight"
    );

    composition(&input, "compositionend");
    assert_eq!(
        input.value(),
        "ABC",
        "the deferred write applied at compositionend"
    );
    assert_eq!(selection(&input).0, Some(1), "with the caret mapped");

    // The composition state is cleared: a later write lands immediately.
    f.write("ABCD");
    assert_eq!(input.value(), "ABCD");
    f.teardown();
}

#[wasm_bindgen_test]
fn only_the_latest_deferred_write_applies() {
    let f = Fixture::mount("input", &[("type", "text"), ("value", "abc")]);
    let input = f.input();
    input.focus().unwrap();

    composition(&input, "compositionstart");
    f.write("first");
    f.write("second");
    composition(&input, "compositionend");

    assert_eq!(input.value(), "second");
    f.teardown();
}
