//! `rsx!` writes an HTML **boolean attribute** as a shape, not a string
//! (issue #551).
//!
//! The macro rendered every attribute value through `Display` and handed the
//! result to `set_attribute`, and no generated path ever removed an attribute.
//! So a `bool` that was false wrote `disabled="false"` — *present*, which HTML
//! reads as true. Measured against the unfixed macro, a reactive `disabled`
//! never turned off at all: with the signal false at mount the browser rendered
//! the button already disabled, and it stayed that way through every toggle.
//!
//! `MockDomDocument` is a pass-through — `set_attribute` inserts the string it is
//! handed, `remove_attribute` erases the key — so these observe the macro's
//! output verbatim, with no backend policy in between. The browser half of the
//! same claim (that a present `disabled` really does disable, and that a
//! presence-form write really does un-disable) is
//! `crates/rinch-web/tests/boolean_attributes.rs`, which runs in Chrome; the
//! desktop readers that need the *removal* are pinned in
//! `crates/rinch-dom/tests/boolean_attribute_readers.rs`.
//!
//! Every test here fails against the unfixed codegen — the `assert_eq!(…, None)`
//! lines find `Some("false")`. The enumerated cases fail against the opposite
//! mistake, a fix keyed on the value's *type* rather than the attribute's name:
//! `draggable`, `aria-*` and rinch's own `data-viewport-ready` all carry a
//! `bool` whose `"false"` is meaningful.

use rinch::prelude::*;
use rinch_core::dom::DomDocument;
use rinch_core::dom::mock::MockDomDocument;
use std::cell::RefCell;
use std::rc::Rc;

/// Mount a component into a headless document.
///
/// Returns the document and the scope alongside the root: a [`NodeHandle`] holds
/// only a `Weak` to the document, so letting either go out of scope would leave
/// every handle answering as if the tree were empty.
fn mount(
    build: impl FnOnce(&mut RenderScope) -> NodeHandle,
) -> (Rc<RefCell<MockDomDocument>>, RenderScope, NodeHandle) {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let root = build(&mut scope);
    (doc, scope, root)
}

// ── the reported shape: a reactive `bool` on `disabled` ─────────────────────

#[component]
fn reactive_disabled(busy: Signal<bool>) -> NodeHandle {
    rsx! {
        button { disabled: {move || busy.get()}, "Rename" }
    }
}

/// A reactive `disabled` toggles **both** ways, and its true form is bare
/// presence rather than the string `"true"`.
///
/// Starting the signal false is the load-bearing part: the first render is where
/// the unfixed macro already wrote `disabled="false"`, so the control was dead
/// before any toggle. Pinning only the toggle would sit on the fixed point where
/// broken and fixed agree.
#[test]
fn a_reactive_disabled_clears_when_its_closure_goes_false() {
    let busy = Signal::new(false);
    let (_doc, _scope, btn) = mount(|s| reactive_disabled(s, busy));

    assert_eq!(
        btn.get_attribute("disabled"),
        None,
        "a closure that is false at mount must write no attribute at all"
    );

    busy.set(true);
    assert_eq!(
        btn.get_attribute("disabled").as_deref(),
        Some(""),
        "true is the bare presence form, so `checked=\"\"` and `checked=\"true\"` \
         cannot be two different states"
    );

    busy.set(false);
    assert_eq!(
        btn.get_attribute("disabled"),
        None,
        "false removes the attribute — writing \"false\" leaves it present, and \
         HTML reads present as true"
    );
}

/// The **non-closure** reactive arm of the codegen (a bare expression, wrapped
/// in an effect) obeys the same rule. It is a separate `quote!` branch, so it
/// can drift from the closure one.
#[component]
fn reactive_disabled_bare_expr(busy: Signal<bool>) -> NodeHandle {
    rsx! {
        button { disabled: busy.get(), "Rename" }
    }
}

#[test]
fn the_bare_expression_attribute_arm_clears_too() {
    let busy = Signal::new(true);
    let (_doc, _scope, btn) = mount(|s| reactive_disabled_bare_expr(s, busy));
    assert_eq!(btn.get_attribute("disabled").as_deref(), Some(""));
    busy.set(false);
    assert_eq!(btn.get_attribute("disabled"), None);
}

/// And the **literal** arm: `disabled: false` is a third `quote!` branch, and it
/// wrote `disabled="false"` too — a statically dead control, with no signal
/// anywhere near it.
#[component]
fn literal_booleans() -> NodeHandle {
    rsx! {
        div {
            button { disabled: false, "off" }
            button { disabled: true, "on" }
        }
    }
}

#[test]
fn a_literal_false_boolean_attribute_is_not_written() {
    let (_doc, _scope, root) = mount(literal_booleans);
    let kids = root.children();
    assert_eq!(kids[0].get_attribute("disabled"), None);
    assert_eq!(kids[1].get_attribute("disabled").as_deref(), Some(""));
}

// ── the rest of the set ─────────────────────────────────────────────────────

#[component]
fn the_wider_set(flag: Signal<bool>) -> NodeHandle {
    rsx! {
        div {
            input { r#type: "checkbox", checked: {move || flag.get()} }
            input { readonly: {move || flag.get()} }
            input { required: {move || flag.get()} }
            select { multiple: {move || flag.get()} }
            p { hidden: {move || flag.get()}, "text" }
            option { selected: {move || flag.get()}, "A" }
            details { open: {move || flag.get()} }
            // rinch's own two, read as boolean attributes by both backends.
            div { data-disabled: {move || flag.get()} }
            div { data-nofocus: {move || flag.get()} }
        }
    }
}

/// Nine attributes from the set, each toggling both ways.
///
/// Not one attribute: the fix is a name lookup, and a lookup that answered only
/// for `disabled` would pass every test above.
#[test]
fn every_boolean_attribute_in_the_set_toggles_both_ways() {
    let names = [
        "checked",
        "readonly",
        "required",
        "multiple",
        "hidden",
        "selected",
        "open",
        "data-disabled",
        "data-nofocus",
    ];
    let flag = Signal::new(true);
    let (_doc, _scope, root) = mount(|s| the_wider_set(s, flag));
    let kids = root.children();
    assert_eq!(
        kids.len(),
        names.len(),
        "one child per attribute under test"
    );

    for (kid, name) in kids.iter().zip(names) {
        assert_eq!(
            kid.get_attribute(name).as_deref(),
            Some(""),
            "{name} must be written by presence when true"
        );
    }
    flag.set(false);
    for (kid, name) in kids.iter().zip(names) {
        assert_eq!(
            kid.get_attribute(name),
            None,
            "{name} must be removed when false"
        );
    }
}

// ── the counter-case: attributes whose `"false"` is a value ─────────────────

#[component]
fn enumerated_look_alikes(flag: Signal<bool>) -> NodeHandle {
    rsx! {
        div {
            // Enumerated in HTML ("true"/"false", invalid → auto), and desktop's
            // drag dispatch matches the literal string "true".
            div { draggable: {move || flag.get()} }
            // ARIA states are tri-valued; a removed one means "not applicable".
            div { aria-expanded: {move || flag.get()} }
            // rinch's own opt-out, where *absence* means ready — removing it on
            // false would invert it (see the viewport-holes note in CLAUDE.md).
            div { data-viewport-ready: {move || flag.get()} }
        }
    }
}

/// An attribute that merely *looks* boolean keeps its literal `"false"`.
///
/// This is what makes the fix name-driven rather than type-driven, and it is the
/// half that fails against the tempting alternative: presence-map every `bool`,
/// and `draggable=""` reaches desktop's `== Some("true")` dispatch as false and
/// the browser's enumerated parser as `auto`. Drag stops working on both
/// backends, with nothing in this issue's own tests to say so.
#[test]
fn an_enumerated_attribute_keeps_its_false_string() {
    let names = ["draggable", "aria-expanded", "data-viewport-ready"];
    let flag = Signal::new(true);
    let (_doc, _scope, root) = mount(|s| enumerated_look_alikes(s, flag));
    let kids = root.children();

    for (kid, name) in kids.iter().zip(names) {
        assert_eq!(
            kid.get_attribute(name).as_deref(),
            Some("true"),
            "{name} carries a value, so true is the string \"true\""
        );
    }
    flag.set(false);
    for (kid, name) in kids.iter().zip(names) {
        assert_eq!(
            kid.get_attribute(name).as_deref(),
            Some("false"),
            "{name} carries a value, so false is the string \"false\" — present"
        );
    }
}

// ── a string closure on a boolean attribute ────────────────────────────────

#[component]
fn string_valued_boolean(mode: Signal<&'static str>) -> NodeHandle {
    rsx! {
        button { disabled: {move || mode.get()}, "x" }
    }
}

/// A closure that yields a *string* into a boolean attribute follows the same
/// truthiness rule the readers use: everything is on except the explicit falsey
/// spellings, so `""` — the form components write — stays present.
#[test]
fn a_string_closure_on_a_boolean_attribute_follows_truthiness() {
    let mode = Signal::new("");
    let (_doc, _scope, btn) = mount(|s| string_valued_boolean(s, mode));
    assert_eq!(
        btn.get_attribute("disabled").as_deref(),
        Some(""),
        "an empty string is HTML's own presence form, so it is on"
    );
    mode.set("disabled");
    assert_eq!(btn.get_attribute("disabled").as_deref(), Some(""));
    mode.set("false");
    assert_eq!(btn.get_attribute("disabled"), None);
    mode.set("FALSE");
    assert_eq!(
        btn.get_attribute("disabled"),
        None,
        "the escape is ASCII-case-insensitive, matching every reader of it"
    );
}
