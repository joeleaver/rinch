//! Browser-driven tests for reactive HTML **boolean** attributes (issue #551).
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown
//! ```
//!
//! The defect: `rsx!` stringified a reactive value into the attribute, so a
//! `bool` that was false wrote `disabled="false"` — and in HTML a *present*
//! `disabled` attribute disables the control whatever its value. Measured here
//! before the fix, that was worse than "off after the first render": with the
//! signal false at mount the very first frame rendered the button **already
//! disabled**, and no toggle recovered it. It was never on.
//!
//! Only a browser can answer whether a present attribute really disables, so the
//! assertions are `HTMLElement.click()` reaching its handler (the spec makes
//! `click()` a no-op on a disabled form control) and the live IDL properties —
//! not the attribute string, which the backend-neutral half already pins in
//! `crates/rinch-macros/tests/rsx_boolean_attrs.rs`.
//!
//! The `draggable` case is the counter-test. It is an *enumerated* attribute,
//! not a boolean one — `draggable="false"` is a meaningful value that must
//! survive — so it pins the fix to the attribute **name**, and fails against a
//! fix that presence-maps every `bool` it is handed.
#![cfg(target_arch = "wasm32")]

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
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
const HOST_MARKER: &str = "data-bool-attr-test-host";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
}

impl Fixture {
    fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> Self {
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
        let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), build);
        Self { root, host }
    }

    fn el(&self, id: &str) -> web_sys::HtmlElement {
        document()
            .get_element_by_id(id)
            .unwrap_or_else(|| panic!("no element #{id}"))
            .dyn_into()
            .unwrap()
    }

    /// A live IDL property (`.disabled`, `.readOnly`, …) as the browser sees it.
    fn prop(&self, id: &str, key: &str) -> bool {
        js_sys::Reflect::get(&self.el(id), &key.into())
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or_else(|| panic!("#{id} has no boolean `{key}` property"))
    }

    fn attr(&self, id: &str, name: &str) -> Option<String> {
        self.el(id).get_attribute(name)
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

// ── the HTML fact the fix rests on, measured ────────────────────────────────

/// A *present* boolean attribute is true whatever its value — measured, not
/// recalled, and measured outside rinch: this fixture is raw markup, so nothing
/// in the framework can be what produced the answer.
///
/// It is the oracle for both halves of #551. It is why writing the string
/// `"false"` is a one-way latch on the web, and it is why
/// `crates/rinch-dom/tests/boolean_attribute_readers.rs` is allowed to pin
/// desktop's presence-only `:checked` and `<option selected>` as *correct*
/// rather than fixing them: a browser answers the same way.
#[wasm_bindgen_test]
fn html_reads_a_present_boolean_attribute_as_true_whatever_its_value() {
    let host = document().create_element("div").unwrap();
    host.set_inner_html(
        r#"<button id="raw-btn" disabled="false">x</button>
           <input id="raw-chk" type="checkbox" checked="false">
           <input id="raw-ro" type="text" readonly="false">
           <p id="raw-hid" hidden="false">t</p>"#,
    );
    document().body().unwrap().append_child(&host).unwrap();

    let prop = |id: &str, key: &str| -> bool {
        let el = document().get_element_by_id(id).unwrap();
        js_sys::Reflect::get(&el, &key.into())
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or_else(|| panic!("#{id} has no boolean `{key}`"))
    };

    assert!(prop("raw-btn", "disabled"), "disabled=\"false\" disables");
    assert!(prop("raw-chk", "checked"), "checked=\"false\" is checked");
    assert!(
        document()
            .get_element_by_id("raw-chk")
            .unwrap()
            .matches(":checked")
            .unwrap(),
        "and it matches :checked, which is what desktop's matcher mirrors"
    );
    assert!(
        prop("raw-ro", "readOnly"),
        "readonly=\"false\" is read-only"
    );
    // `hidden` is enumerated rather than boolean, but its invalid-value default
    // is the hidden state, so it fails in exactly the same direction.
    assert!(prop("raw-hid", "hidden"), "hidden=\"false\" is hidden");

    host.remove();
}

// ── disabled: the reported shape ────────────────────────────────────────────

#[component]
fn disabled_button(busy: Signal<bool>, clicks: Rc<Cell<u32>>) -> NodeHandle {
    let bump = clicks.clone();
    rsx! {
        div {
            button {
                id: "primary",
                disabled: {move || busy.get()},
                onclick: move || bump.set(bump.get() + 1),
                "Rename"
            }
        }
    }
}

/// A reactive `disabled` turns the control off **and back on**.
///
/// Fails against the unfixed macro on its third assertion block: `busy` goes
/// false, the effect writes `disabled="false"`, the browser keeps the control
/// disabled, and `click()` returns without dispatching.
#[wasm_bindgen_test]
fn a_reactive_disabled_turns_a_button_back_on() {
    let busy = Signal::new(false);
    let clicks = Rc::new(Cell::new(0u32));
    let f = Fixture::mount({
        let clicks = clicks.clone();
        move |scope: &mut RenderScope| disabled_button(scope, busy, clicks)
    });

    // Enabled to begin with: the click lands.
    assert!(!f.prop("primary", "disabled"), "must start enabled");
    f.el("primary").click();
    assert_eq!(clicks.get(), 1, "an enabled button must dispatch");

    // Disabled: no attribute value, just presence — and no click.
    busy.set(true);
    assert!(
        f.prop("primary", "disabled"),
        "busy must disable the button"
    );
    assert_eq!(
        f.attr("primary", "disabled").as_deref(),
        Some(""),
        "a true boolean attribute is written in the bare presence form"
    );
    f.el("primary").click();
    assert_eq!(clicks.get(), 1, "a disabled button must not dispatch");

    // Back on: the attribute is *gone*, not `="false"`.
    busy.set(false);
    assert_eq!(
        f.attr("primary", "disabled"),
        None,
        "a false boolean attribute is removed, not written as the string \"false\""
    );
    assert!(
        !f.prop("primary", "disabled"),
        "clearing busy must re-enable the button"
    );
    f.el("primary").click();
    assert_eq!(clicks.get(), 2, "a re-enabled button must dispatch again");

    f.teardown();
}

// ── the wider set, and the enumerated counter-case ──────────────────────────

#[component]
fn mixed_attributes(flag: Signal<bool>) -> NodeHandle {
    rsx! {
        div {
            input { id: "ro", r#type: "text", readonly: {move || flag.get()} }
            input { id: "req", r#type: "text", required: {move || flag.get()} }
            input { id: "chk", r#type: "checkbox", checked: {move || flag.get()} }
            select { id: "multi", multiple: {move || flag.get()},
                option { value: "a", "A" }
            }
            p { id: "hid", hidden: {move || flag.get()}, "text" }
            // NOT a boolean attribute: `draggable` is enumerated
            // ("true"/"false"), and desktop's drag dispatch reads the literal
            // string `"true"`. Presence-mapping it would break both backends.
            div { id: "drag", draggable: {move || flag.get()}, "handle" }
            // Nor this one: rinch's own `data-viewport-ready` is an opt-out
            // whose *absence* means ready, so removing it inverts the meaning.
            div { id: "vp", data-viewport-ready: {move || flag.get()} }
        }
    }
}

/// Every boolean attribute in the set clears when its closure goes false, and
/// the two enumerated look-alikes keep their literal `"false"`.
///
/// Fails against the unfixed macro on the first `assert_eq!(..., None, ...)`:
/// `readonly="false"` stays present, so `.readOnly` stays true.
#[wasm_bindgen_test]
fn boolean_attributes_clear_and_enumerated_ones_keep_their_value() {
    let flag = Signal::new(true);
    let f = Fixture::mount(move |scope: &mut RenderScope| mixed_attributes(scope, flag));

    // On.
    assert_eq!(f.attr("ro", "readonly").as_deref(), Some(""));
    assert!(f.prop("ro", "readOnly"));
    assert!(f.prop("req", "required"));
    assert!(f.prop("chk", "checked"));
    assert!(f.prop("multi", "multiple"));
    assert!(f.prop("hid", "hidden"));
    assert_eq!(f.attr("drag", "draggable").as_deref(), Some("true"));
    assert_eq!(f.attr("vp", "data-viewport-ready").as_deref(), Some("true"));

    // Off.
    flag.set(false);
    assert_eq!(f.attr("ro", "readonly"), None, "readonly must be removed");
    assert!(!f.prop("ro", "readOnly"));
    assert_eq!(f.attr("req", "required"), None, "required must be removed");
    assert!(!f.prop("req", "required"));
    assert_eq!(f.attr("chk", "checked"), None, "checked must be removed");
    assert!(!f.prop("chk", "checked"));
    assert_eq!(
        f.attr("multi", "multiple"),
        None,
        "multiple must be removed"
    );
    assert!(!f.prop("multi", "multiple"));
    assert_eq!(f.attr("hid", "hidden"), None, "hidden must be removed");
    assert!(!f.prop("hid", "hidden"));
    // The counter-case: still the string, still present.
    assert_eq!(
        f.attr("drag", "draggable").as_deref(),
        Some("false"),
        "draggable is enumerated — \"false\" is a value, not an absence"
    );
    assert_eq!(
        f.attr("vp", "data-viewport-ready").as_deref(),
        Some("false"),
        "data-viewport-ready=\"false\" is rinch's opt-out; removing it inverts it"
    );

    f.teardown();
}
