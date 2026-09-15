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
use std::cell::{Cell, RefCell};
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
///
/// It is also the oracle for **#612**, which retired desktop's `"false"` escape
/// on `disabled` / `readonly` so that the two backends answer this markup the
/// same way. That is what the two `:disabled` / `:read-only` assertions below are
/// for: the IDL property alone would leave open whether the *styling* followed.
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

    let matches = |id: &str, sel: &str| -> bool {
        document()
            .get_element_by_id(id)
            .unwrap()
            .matches(sel)
            .unwrap()
    };

    assert!(prop("raw-btn", "disabled"), "disabled=\"false\" disables");
    // And it *styles* as disabled, which is the half desktop's `:disabled`
    // matcher mirrors since issue #612 retired its `"false"` escape.
    assert!(
        matches("raw-btn", ":disabled"),
        "disabled=\"false\" matches :disabled"
    );
    assert!(prop("raw-chk", "checked"), "checked=\"false\" is checked");
    assert!(
        matches("raw-chk", ":checked"),
        "and it matches :checked, which is what desktop's matcher mirrors"
    );
    assert!(
        prop("raw-ro", "readOnly"),
        "readonly=\"false\" is read-only"
    );
    assert!(
        matches("raw-ro", ":read-only"),
        "readonly=\"false\" matches :read-only — the oracle for desktop's \
         `node_is_readonly` (#612)"
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

// ── #622: `set_attribute` is the literal primitive, on both backends ─────────

/// Build a checkbox and a two-option `<select>`, handing the test their
/// `NodeHandle`s so it can call the DOM primitive directly.
///
/// The `rsx!` cases above go through `NodeHandle::write_attribute`, which maps
/// truthiness onto presence before the backend ever sees a string (#551). These
/// tests are about the layer *below* that — what `set_attribute` itself does
/// with a string it is handed — so they must not go through the macro.
fn checked_family_fixture(
    out: Rc<RefCell<Option<(NodeHandle, NodeHandle)>>>,
) -> impl FnOnce(&mut RenderScope) -> NodeHandle + 'static {
    move |scope: &mut RenderScope| {
        let root = scope.create_element("div");

        let input = scope.create_element("input");
        input.set_attribute("type", "checkbox");
        input.set_attribute("id", "lit-chk");
        root.append_child(&input);

        let select = scope.create_element("select");
        select.set_attribute("id", "lit-sel");
        let o0 = scope.create_element("option");
        o0.set_attribute("value", "v0");
        let o1 = scope.create_element("option");
        o1.set_attribute("value", "v1");
        o1.set_attribute("id", "lit-opt");
        select.append_child(&o0);
        select.append_child(&o1);
        root.append_child(&select);

        *out.borrow_mut() = Some((input, o1));
        root
    }
}

/// `set_attribute("checked" | "selected", …)` writes the string it is given and
/// the control follows the attribute's **presence**, whatever that string says.
///
/// This is issue #622. The web backend used to route those two names through
/// `attr_is_truthy` and *remove* the attribute for a falsey string, so
/// `set_attribute("checked", "false")` unchecked the box on web and checked it
/// on desktop, whose `:checked` reads presence alone — and so does a browser,
/// for the raw markup measured by
/// `html_reads_a_present_boolean_attribute_as_true_whatever_its_value` above.
///
/// Red at that commit on its very first assertion: the attribute was absent.
///
/// The `indeterminate` block at the end is the counter-case. It is a
/// property-only IDL flag with **no** content attribute, so it has no presence
/// to read and keeps its truthiness mapping; a fix that presence-maps the whole
/// reflected family fails there.
#[wasm_bindgen_test]
fn set_attribute_writes_the_checked_family_literally() {
    let out = Rc::new(RefCell::new(None));
    let f = Fixture::mount(checked_family_fixture(out.clone()));
    let (input, option) = out.borrow().clone().expect("fixture built");

    // The literal string lands in the attribute …
    input.set_attribute("checked", "false");
    assert_eq!(
        f.attr("lit-chk", "checked").as_deref(),
        Some("false"),
        "`set_attribute` is the literal primitive: the string is written, not \
         mapped onto presence (#622). Truthiness lives in `write_attribute`."
    );
    // … and *presence* is what the control follows.
    assert!(
        f.prop("lit-chk", "checked"),
        "a present `checked` checks the box whatever its value — the browser's \
         own rule for raw markup, and desktop's `:checked`"
    );

    // Absence is the only off state.
    input.remove_attribute("checked");
    assert_eq!(f.attr("lit-chk", "checked"), None);
    assert!(!f.prop("lit-chk", "checked"), "removal unchecks it");

    // The bare presence form components write.
    input.set_attribute("checked", "");
    assert_eq!(f.attr("lit-chk", "checked").as_deref(), Some(""));
    assert!(f.prop("lit-chk", "checked"));

    // Same for `<option selected>`, whose selectedness desktop also seeds from
    // the attribute's presence (`collect_options`; #692 is what moves it
    // afterwards).
    option.set_attribute("selected", "false");
    assert_eq!(
        f.attr("lit-opt", "selected").as_deref(),
        Some("false"),
        "literal here too"
    );
    assert!(
        f.prop("lit-opt", "selected"),
        "`selected=\"false\"` selects the option — matching \
         `boolean_attribute_readers::an_option_is_selected_by_the_presence_of_the_attribute`"
    );
    let select: web_sys::HtmlSelectElement = f.el("lit-sel").dyn_into().unwrap();
    assert_eq!(
        select.selected_index(),
        1,
        "and the <select> follows its option"
    );

    option.remove_attribute("selected");
    assert_eq!(f.attr("lit-opt", "selected"), None);
    assert!(!f.prop("lit-opt", "selected"), "removal deselects it");

    // The counter-case: `indeterminate` is a property-only IDL flag. HTML has no
    // such content attribute, so `is_boolean_attribute` does not list it,
    // `write_attribute` cannot map it, and there is no presence for a reader to
    // read — the truthiness mapping is all it has and it stays.
    input.set_attribute("indeterminate", "true");
    assert!(f.prop("lit-chk", "indeterminate"));
    assert_eq!(
        f.attr("lit-chk", "indeterminate"),
        None,
        "no bogus content attribute is materialized for a property-only flag"
    );
    input.set_attribute("indeterminate", "false");
    assert!(
        !f.prop("lit-chk", "indeterminate"),
        "a falsey string still clears a property-only flag: presence is not a \
         thing it has"
    );

    f.teardown();
}

/// A programmatic write wins after the user has toggled the control.
///
/// The browser sets a *dirty checkedness* flag on the first user toggle and
/// stops mirroring the `checked` content attribute onto the live `.checked`
/// property from then on. rinch has no such flag — desktop's `:checked` reads
/// the attribute and nothing else — so the web backend mirrors the property
/// itself (issue #100), and #622 makes that mirror follow **presence** rather
/// than the string.
///
/// Red at the #622 commit on its last assertion only: `checked="false"` removed
/// the attribute and cleared the property, so the app's write silently unchecked
/// a box that desktop would have checked. Everything above that assertion passes
/// at that commit, and is here so a fix that drops the property mirror
/// altogether — the other way to make the first test green — fails.
#[wasm_bindgen_test]
fn a_write_after_the_user_toggled_the_control_still_wins() {
    let out = Rc::new(RefCell::new(None));
    let f = Fixture::mount(checked_family_fixture(out.clone()));
    let (input, _option) = out.borrow().clone().expect("fixture built");

    input.set_attribute("checked", "");
    assert!(f.prop("lit-chk", "checked"), "the app checks it");

    // The user unchecks it. The content attribute is the control's *default* and
    // a user toggle does not touch it, so attribute and property now disagree.
    f.el("lit-chk").click();
    assert!(!f.prop("lit-chk", "checked"), "the click unchecks it");
    assert_eq!(
        f.attr("lit-chk", "checked").as_deref(),
        Some(""),
        "the user toggle leaves the content attribute alone — the dirty flag"
    );

    // The app writes the same attribute again. A browser would change only
    // `defaultChecked` here; rinch makes the control follow, or a reactive
    // binding would go stale after the first click.
    input.set_attribute("checked", "");
    assert!(
        f.prop("lit-chk", "checked"),
        "a programmatic write must re-check a dirtied control (#100)"
    );

    // Removal turns it off, dirty or not.
    input.remove_attribute("checked");
    assert!(!f.prop("lit-chk", "checked"));

    // And a falsey *string* is a presence, so it turns it back on — which is
    // what desktop has always done with the same call (#622).
    input.set_attribute("checked", "false");
    assert!(
        f.prop("lit-chk", "checked"),
        "`set_attribute(\"checked\", \"false\")` makes the attribute present, \
         and presence checks the box on both backends (#622)"
    );

    f.teardown();
}

/// A programmatic `selected` write wins after the option's selectedness has gone
/// **dirty** — the `<option>` half of
/// `a_write_after_the_user_toggled_the_control_still_wins`, which covers only
/// `checked`.
///
/// `HTMLOptionElement.selected`'s *setter* sets the element's dirtiness flag,
/// exactly as a user pick does, so a fixture can dirty an option without driving
/// a real pick. A dirty option stops mirroring its content attribute into its
/// selectedness, so without `sync_presence_property`'s `"selected"` arm the
/// app's write would be invisible.
///
/// **This is the only fixture that kills deleting that arm** — measured: with it
/// deleted the other two are 5/5 green. They exercise `selected` on a *pristine*
/// option, where the browser mirrors the attribute into selectedness for free
/// and a missing arm cannot be seen. That is the same fixed point
/// `a_write_after_the_user_toggled_the_control_still_wins` exists to get off,
/// one attribute over. Found by the review of PR #686.
#[wasm_bindgen_test]
fn a_selected_write_after_the_option_went_dirty_still_wins() {
    let out = Rc::new(RefCell::new(None));
    let f = Fixture::mount(checked_family_fixture(out.clone()));
    let (_input, option) = out.borrow().clone().expect("fixture built");

    // Dirty the second option's selectedness the way a user pick does, and leave
    // it *deselected*: off the fixed point where attribute and selectedness
    // agree by themselves.
    let live: web_sys::HtmlOptionElement = f.el("lit-opt").dyn_into().unwrap();
    live.set_selected(true);
    live.set_selected(false);
    assert!(
        !f.prop("lit-opt", "selected"),
        "the option starts deselected, and dirty"
    );

    // The app writes the attribute. A browser would move only the option's
    // *default* here, because the option is dirty — so nothing would happen.
    option.set_attribute("selected", "");
    assert_eq!(f.attr("lit-opt", "selected").as_deref(), Some(""));
    assert!(
        f.prop("lit-opt", "selected"),
        "a programmatic write must re-select a dirtied option (#100), because \
         desktop seeds an option's selectedness from the attribute's presence"
    );
    let select: web_sys::HtmlSelectElement = f.el("lit-sel").dyn_into().unwrap();
    assert_eq!(select.selected_index(), 1, "and the <select> follows");

    // And removal still deselects it, dirty or not.
    option.remove_attribute("selected");
    assert!(!f.prop("lit-opt", "selected"), "removal deselects it");

    f.teardown();
}

// ── #687: the guard that asked the wrong question ───────────────────────────

/// A `checked` binding that reads a second signal, so the effect can re-run
/// while the value it writes stays `false` — which is the only way an app sees
/// #687 at all. A binding on one signal re-runs only when that signal changes,
/// and a change would have corrected the control anyway.
#[component]
fn rebindable_checkbox(flag: Signal<bool>, rerun: Signal<u32>) -> NodeHandle {
    rsx! {
        div {
            input {
                id: "bound-chk",
                r#type: "checkbox",
                checked: {move || { rerun.get(); flag.get() }},
            }
        }
    }
}

/// A user toggle leaves the box checked against a binding that says `false`,
/// and the binding's next write must take it back (issue #687).
///
/// The browser sets *dirty checkedness* on the first user toggle and from then
/// on the content attribute is only the control's default, so the attribute
/// reads absent while the box reads checked. `write_attribute` guarded its
/// removal on that attribute, saw "already off", and wrote nothing — the box
/// stayed checked with the binding saying false, and stayed that way until the
/// signal genuinely changed.
///
/// Red at `5f16cb0` on the final assertion: `.checked` was still `true` after
/// the effect re-ran. The earlier assertions pass there too, and are the
/// measurement that the browser really does hide the state from
/// `getAttribute` — without them a failure here could be read as the click not
/// having landed.
#[wasm_bindgen_test]
fn a_binding_that_says_false_takes_back_a_user_toggled_checkbox() {
    let flag = Signal::new(false);
    let rerun = Signal::new(0u32);
    let f = Fixture::mount(move |scope: &mut RenderScope| rebindable_checkbox(scope, flag, rerun));

    assert!(!f.prop("bound-chk", "checked"), "starts unchecked");
    assert_eq!(
        f.attr("bound-chk", "checked"),
        None,
        "and with no attribute"
    );

    // The user clicks. The property moves; the attribute does not.
    f.el("bound-chk").click();
    assert!(f.prop("bound-chk", "checked"), "the click checks the box");
    assert_eq!(
        f.attr("bound-chk", "checked"),
        None,
        "and leaves the content attribute absent — the state `get_attribute` \
         cannot see, which is the whole of #687"
    );

    // The effect re-runs and writes the same `false` it has written all along.
    rerun.set(1);
    assert_eq!(
        f.attr("bound-chk", "checked"),
        None,
        "still no attribute to remove"
    );
    assert!(
        !f.prop("bound-chk", "checked"),
        "a binding that says false must un-check a control the user toggled, \
         even with no attribute to remove (#687)"
    );

    f.teardown();
}

/// The `<option>` half: a falsey `selected` write takes back an option whose
/// selectedness went dirty.
///
/// `HTMLOptionElement.selected`'s setter sets the dirtiness flag exactly as a
/// user pick does, so the fixture can reach the state without driving the
/// popup. Off the fixed point on purpose: the option is left *selected* with no
/// attribute, which is precisely where the content attribute stops describing
/// the control.
///
/// This is the fixture that kills a fix listing only `checked` in the pair.
#[wasm_bindgen_test]
fn a_falsey_selected_write_takes_back_a_dirtied_option() {
    let out = Rc::new(RefCell::new(None));
    let f = Fixture::mount(checked_family_fixture(out.clone()));
    let (_input, option) = out.borrow().clone().expect("fixture built");

    let live: web_sys::HtmlOptionElement = f.el("lit-opt").dyn_into().unwrap();
    live.set_selected(true);
    assert!(f.prop("lit-opt", "selected"), "the option is selected …");
    assert_eq!(
        f.attr("lit-opt", "selected"),
        None,
        "… with no `selected` attribute anywhere"
    );

    option.write_attribute("selected", "false");
    assert!(
        !f.prop("lit-opt", "selected"),
        "a falsey `selected` write must deselect a dirtied option (#687)"
    );
    let select: web_sys::HtmlSelectElement = f.el("lit-sel").dyn_into().unwrap();
    assert_eq!(
        select.selected_index(),
        0,
        "and the <select> falls back to its first option, as a browser does \
         when nothing is selected"
    );

    f.teardown();
}
