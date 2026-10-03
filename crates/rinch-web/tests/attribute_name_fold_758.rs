//! Browser-driven test for issue #758: an **uppercase** boolean attribute
//! name reaches the DOM attribute (the browser folds that write for an HTML
//! element) but got no live-property mirror, because
//! `WebDocument::set_attribute`/`remove_attribute` matched the name they were
//! given — literally — against `"checked" | "selected" | "muted" |
//! "indeterminate" | "value"`, and an uppercase spelling matches none of them.
//!
//! Desktop already folds the name at `RinchDocument::set_attribute` (#738), so
//! this was a web-only gap (found by the #753 reviewer, `review-753-755.md`).
//!
//! Every reproducing fixture below dirties the control first (a direct
//! property write or a real click), which is the one state where the mirror
//! is the ONLY thing that can move `.checked`/`.selected` — on a pristine
//! control the browser's own default reflection moves it for free and hides
//! a missing `sync_presence_property` call entirely (the same fixed point
//! `boolean_attributes.rs`'s #687 tests are built to get off).
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown
//! ```
#![cfg(target_arch = "wasm32")]

use rinch::prelude::*;
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

const HOST_MARKER: &str = "data-attr-fold-758-test-host";

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

    fn prop(&self, id: &str, key: &str) -> bool {
        js_sys::Reflect::get(&self.el(id), &key.into())
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or_else(|| panic!("#{id} has no boolean `{key}` property"))
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

/// Build a bare checkbox and a two-option `<select>`, handing back their
/// `NodeHandle`s so the test can call `set_attribute`/`remove_attribute`/
/// `write_attribute` directly — bypassing `rsx!`'s own lowercase tag/attribute
/// spelling, which would otherwise hide the bug (the macro never emits an
/// uppercase name).
fn checked_family_fixture(
    out: Rc<RefCell<Option<(NodeHandle, NodeHandle)>>>,
) -> impl FnOnce(&mut RenderScope) -> NodeHandle + 'static {
    move |scope: &mut RenderScope| {
        let root = scope.create_element("div");

        let input = scope.create_element("input");
        input.set_attribute("type", "checkbox");
        input.set_attribute("id", "fold-chk");
        root.append_child(&input);

        let select = scope.create_element("select");
        select.set_attribute("id", "fold-sel");
        let o0 = scope.create_element("option");
        o0.set_attribute("value", "v0");
        let o1 = scope.create_element("option");
        o1.set_attribute("value", "v1");
        o1.set_attribute("id", "fold-opt");
        select.append_child(&o0);
        select.append_child(&o1);
        root.append_child(&select);

        *out.borrow_mut() = Some((input, o1));
        root
    }
}

/// An uppercase `remove_attribute("CHECKED")` must mirror onto `.checked` for
/// a **dirtied** checkbox — the only state where a missing mirror is
/// observable at all.
///
/// Fails at HEAD: `live.set_checked(true)` dirties the control directly
/// (the IDL setter sets the browser's dirty-checkedness flag exactly as a
/// click does) with no `checked` attribute ever written, so nothing a
/// pristine-control reflection could paper over is in play. The call reaches
/// `WebDocument::remove_attribute`'s match on the literal `"CHECKED"`, which
/// (at HEAD) matches neither `"checked"` nor `"selected"`, so
/// `sync_presence_property` is never called and `.checked` stays `true`.
#[wasm_bindgen_test]
fn an_uppercase_removal_mirrors_onto_a_dirtied_checkbox() {
    let out = Rc::new(RefCell::new(None));
    let f = Fixture::mount(checked_family_fixture(out.clone()));
    let (input, _option) = out.borrow().clone().expect("fixture built");

    let live: web_sys::HtmlInputElement = f.el("fold-chk").dyn_into().unwrap();
    live.set_checked(true);
    assert!(
        f.prop("fold-chk", "checked"),
        "dirtied checked, by a property write"
    );

    input.remove_attribute("CHECKED");
    assert!(
        !f.prop("fold-chk", "checked"),
        "an uppercase `remove_attribute(\"CHECKED\")` must still uncheck a \
         dirtied box, exactly as the lowercase spelling does (#758)"
    );

    f.teardown();
}

/// An uppercase `set_attribute("SELECTED", "")` must mirror onto `.selected`
/// for a dirtied option — the literal `set_attribute` entry point (#622),
/// rather than the `write_attribute` pipeline the next test covers.
#[wasm_bindgen_test]
fn an_uppercase_literal_set_attribute_mirrors_onto_a_dirtied_option() {
    let out = Rc::new(RefCell::new(None));
    let f = Fixture::mount(checked_family_fixture(out.clone()));
    let (_input, option) = out.borrow().clone().expect("fixture built");

    // Dirty the option's selectedness directly, left DESELECTED with no
    // attribute — same shape `a_falsey_selected_write_takes_back_a_dirtied_option`
    // uses one attribute over.
    let live: web_sys::HtmlOptionElement = f.el("fold-opt").dyn_into().unwrap();
    live.set_selected(true);
    live.set_selected(false);
    assert!(!f.prop("fold-opt", "selected"), "dirtied, deselected");

    option.set_attribute("SELECTED", "");
    assert!(
        f.prop("fold-opt", "selected"),
        "an uppercase `set_attribute(\"SELECTED\", \"\")` must still select a \
         dirtied option (#758)"
    );

    f.teardown();
}

/// A mixed-case `write_attribute` — the `rsx!` entry point — must take a
/// **dirtied** checkbox back to the opposite of what a click just left it at.
///
/// This is the #687 shape one case fold over: `is_presence_reflected_attribute`
/// already folds case (pre-existing), so the removal guard is never the
/// obstacle here — what was missing is purely the backend's own match on the
/// name it receives. Deliberately sampled off the "write what it already is"
/// fixed point: the click leaves the box CHECKED, and the write asks for
/// FALSE, so the two disagree and only a working mirror resolves it.
#[wasm_bindgen_test]
fn an_uppercase_write_attribute_takes_a_dirtied_checkbox_to_the_opposite_value() {
    let out = Rc::new(RefCell::new(None));
    let f = Fixture::mount(checked_family_fixture(out.clone()));
    let (input, _option) = out.borrow().clone().expect("fixture built");

    assert!(!f.prop("fold-chk", "checked"), "starts unchecked, pristine");
    f.el("fold-chk").click();
    assert!(
        f.prop("fold-chk", "checked"),
        "the click checks it, and dirties it"
    );

    input.write_attribute("Checked", "false");
    assert!(
        !f.prop("fold-chk", "checked"),
        "a falsey mixed-case `Checked` write must take a dirtied, CHECKED \
         control back to unchecked (#687, #758)"
    );

    f.teardown();
}

// ── the measured HTML fact this fix rests on ────────────────────────────────

/// The browser's own case fold on a literal `setAttribute` call, for an
/// element in the HTML namespace — measured outside rinch, so nothing in the
/// framework can be what produced the answer. This is why `el.set_attribute`
/// in `WebDocument::set_attribute` is still given the ORIGINAL (unfolded)
/// name: the browser folds that particular write by itself, for every HTML
/// element, and folding it again in Rust would be redundant there and
/// actively wrong for an SVG element's case-significant attribute names.
///
/// Not itself a reproduction of #758 (it measures the premise, not rinch's
/// behaviour) — it passes unconditionally either side of the fix, like
/// `boolean_attributes.rs`'s own `html_reads_a_present_boolean_attribute_as_true_whatever_its_value`.
#[wasm_bindgen_test]
fn attribute_fold_is_the_browsers_own() {
    let el = document().create_element("input").unwrap();
    el.set_attribute("CHECKED", "").unwrap();
    assert_eq!(
        el.get_attribute("checked").as_deref(),
        Some(""),
        "the browser folds an HTML attribute name to lowercase on its own \
         (DOM spec, Element.setAttribute) — the gap was purely in rinch-web's \
         own Rust match on the literal name it was handed"
    );
    // `getAttribute` is itself case-insensitive for an HTML element (the spec
    // folds its argument too), so the assertion above does NOT independently
    // confirm the *stored* name folded — it only confirms the lookup did.
    // `outerHTML` serializes the attribute's actual stored spelling.
    assert!(
        !el.outer_html().contains("CHECKED"),
        "the uppercase spelling does not survive storage: outerHTML was {:?}",
        el.outer_html()
    );
}
