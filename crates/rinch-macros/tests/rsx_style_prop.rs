//! A `style:` prop is laid **over** the element's own inline style, not in
//! place of it (issue #647).
//!
//! `rsx!` applied a component-level `style:` by writing the root's whole
//! `style` attribute, *after* `Component::render` had already written its own
//! declarations there. Everything the component put in was erased — including
//! every CSS custom property it publishes to its own stylesheet, which is how
//! `Modal`, `Drawer`, `Popover`, `DropdownMenu`, `Notification` and
//! `LoadingOverlay` carry `z_index`, `offset`, `overlay_blur` and the rest
//! (#474). The prop kept compiling and silently stopped working:
//! `Modal { z_index: 517 }` computes 517, and `Modal { z_index: 517, style:
//! "margin: 0" }` computed the pre-#474 default of 200.
//!
//! The same write is emitted from four codegen sites, which is why there are
//! four groups of fixtures below:
//!
//! | site | reached by |
//! |---|---|
//! | `html::generate_style_code` | a component with no reactive prop |
//! | `component_codegen::element_to_dom_component_reactive` | a component with a reactive prop, used as an expression |
//! | `component_codegen::generate_reactive_component_stmt` | the same, as a child of an element |
//! | `html::element_to_dom_html`'s attribute loop | a plain HTML element |
//!
//! Every fixture uses a component that writes **two** declarations of its own,
//! one of which the caller also declares. One alone would sit on a fixed point:
//! a caller declaration that collides with nothing cannot tell "merged" from
//! "merged in the wrong order", and a component that writes nothing cannot tell
//! "merged" from "replaced".
//!
//! The end-to-end half — that a custom property surviving this merge really
//! does still move the computed `z-index` through Stylo — is
//! `rinch`'s `app::overlay_z_index_tests::a_caller_style_prop_does_not_cost_the_
//! overlay_its_level`.

use std::cell::RefCell;
use std::rc::Rc;

use rinch::prelude::*;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

/// Mount a component into a real desktop document.
///
/// `RinchDocument` rather than the mock on purpose: a style shorthand reaches
/// the node through `set_style`, whose merge is a *backend* behaviour, and the
/// mock only approximates it.
fn mount(
    build: impl FnOnce(&mut RenderScope) -> NodeHandle,
) -> (Rc<RefCell<RinchDocument>>, RenderScope, NodeHandle) {
    let doc = Rc::new(RefCell::new(RinchDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let root = build(&mut scope);
    (doc, scope, root)
}

/// The node's inline declarations, as `(property, value)` pairs.
fn decls(node: &NodeHandle) -> Vec<(String, String)> {
    rinch_core::split_declarations(&node.get_attribute("style").unwrap_or_default())
}

/// The value the node's inline style gives `property`, if any.
fn decl(node: &NodeHandle, property: &str) -> Option<String> {
    decls(node)
        .into_iter()
        .find(|(k, _)| k == property)
        .map(|(_, v)| v)
}

// ── the component under test ────────────────────────────────────────────────

/// Stands in for the overlay family: publishes one custom property its
/// stylesheet would read, plus one ordinary declaration the caller is going to
/// collide with.
#[component]
fn Overlay(level: i32) -> NodeHandle {
    let root = __scope.create_element("div");
    root.set_attribute("class", "overlay");
    root.set_attribute("style", &format!("--overlay-z: {}; margin: 8px", level));
    root
}

/// The overlay somewhere under `node`, found by its class rather than by what
/// is in its style — the thing under test must not also be the locator.
fn find_overlay(node: &NodeHandle) -> Option<NodeHandle> {
    if node.get_attribute("class").as_deref() == Some("overlay") {
        return Some(node.clone());
    }
    node.children().iter().find_map(find_overlay)
}

// ── 1. the static component path ────────────────────────────────────────────

#[component]
fn static_style() -> NodeHandle {
    rsx! {
        Overlay { level: 517, style: "margin: 0" }
    }
}

/// A literal `style:` keeps the component's own declarations and wins where it
/// collides with them.
#[test]
fn a_literal_style_prop_keeps_the_components_custom_property() {
    let (_doc, _scope, root) = mount(static_style);
    assert_eq!(
        decl(&root, "--overlay-z").as_deref(),
        Some("517"),
        "the published custom property is what the stylesheet reads; replacing \
         the attribute makes the prop inert with nothing warning"
    );
    assert_eq!(
        decl(&root, "margin").as_deref(),
        Some("0"),
        "the caller is the later author, so the caller wins the collision"
    );
}

#[component]
fn reactive_style(css: Signal<String>) -> NodeHandle {
    rsx! {
        Overlay { level: 517, style: {move || css.get()} }
    }
}

/// A reactive `style:` merges on the first run **and** on every re-run.
#[test]
fn a_reactive_style_prop_keeps_the_components_custom_property() {
    let css = Signal::new(String::from("margin: 0"));
    let (_doc, _scope, root) = mount(|s| reactive_style(s, css));
    assert_eq!(decl(&root, "--overlay-z").as_deref(), Some("517"));
    assert_eq!(decl(&root, "margin").as_deref(), Some("0"));

    css.set(String::from("margin: 4px"));
    assert_eq!(
        decl(&root, "--overlay-z").as_deref(),
        Some("517"),
        "a re-run must not eat the component's declarations either"
    );
    assert_eq!(decl(&root, "margin").as_deref(), Some("4px"));
}

/// A re-run replaces the **caller's** previous declarations rather than
/// stacking onto them, and hands back what they displaced.
///
/// Three properties, each with a different fate, because a single one cannot
/// tell the three apart: `padding` is dropped by the caller (pure addition, so
/// it goes), `margin` is dropped by the caller but the component had declared
/// it (so the component's value comes back), and `--overlay-z` was never the
/// caller's at all (so it is untouched throughout).
#[test]
fn a_reactive_style_prop_takes_its_own_previous_declarations_off() {
    let css = Signal::new(String::from("margin: 0; padding: 4px"));
    let (_doc, _scope, root) = mount(|s| reactive_style(s, css));
    assert_eq!(decl(&root, "padding").as_deref(), Some("4px"));

    css.set(String::from("color: red"));
    assert_eq!(
        decl(&root, "padding"),
        None,
        "a declaration the caller no longer makes must not stay behind"
    );
    assert_eq!(
        decl(&root, "margin").as_deref(),
        Some("8px"),
        "the component's own value comes back when the caller stops overriding it"
    );
    assert_eq!(decl(&root, "color").as_deref(), Some("red"));
    assert_eq!(decl(&root, "--overlay-z").as_deref(), Some("517"));
}

// ── 2/3. the reactive-component paths ───────────────────────────────────────

/// A reactive *non-style* prop is what routes a component through
/// `element_to_dom_component_reactive` — as an expression here…
#[component]
fn reactive_prop_expression(level: Signal<i32>) -> NodeHandle {
    rsx! {
        Overlay { level: {move || level.get()}, style: "margin: 0" }
    }
}

/// …and through `generate_reactive_component_stmt` here, by sitting inside an
/// element.
#[component]
fn reactive_prop_child(level: Signal<i32>) -> NodeHandle {
    rsx! {
        div {
            Overlay { level: {move || level.get()}, style: "margin: 0" }
        }
    }
}

#[test]
fn a_style_prop_beside_a_reactive_component_prop_still_merges() {
    let level = Signal::new(517);
    let (_doc, _scope, root) = mount(|s| reactive_prop_expression(s, level));
    // The reactive wrapper re-renders into a fresh element, so the node has to
    // be looked up again after every change.
    let overlay = || find_overlay(&root).expect("the overlay is mounted");
    assert_eq!(decl(&overlay(), "--overlay-z").as_deref(), Some("517"));
    assert_eq!(decl(&overlay(), "margin").as_deref(), Some("0"));

    level.set(742);
    assert_eq!(
        decl(&overlay(), "--overlay-z").as_deref(),
        Some("742"),
        "the re-render publishes the new level…"
    );
    assert_eq!(
        decl(&overlay(), "margin").as_deref(),
        Some("0"),
        "…and the caller's style is laid over it again"
    );
}

#[test]
fn a_style_prop_on_a_reactive_component_child_still_merges() {
    let level = Signal::new(517);
    let (_doc, _scope, root) = mount(|s| reactive_prop_child(s, level));
    // The reactive wrapper re-renders into a fresh element, so the node has to
    // be looked up again after every change.
    let overlay = || find_overlay(&root).expect("the overlay is mounted");
    assert_eq!(decl(&overlay(), "--overlay-z").as_deref(), Some("517"));
    assert_eq!(decl(&overlay(), "margin").as_deref(), Some("0"));

    level.set(742);
    assert_eq!(decl(&overlay(), "--overlay-z").as_deref(), Some("742"));
    assert_eq!(decl(&overlay(), "margin").as_deref(), Some("0"));
}

// ── 4. a plain HTML element ─────────────────────────────────────────────────

/// The identical write on an HTML element, where the second author is a style
/// **shorthand** rather than a component.
#[component]
fn html_style_and_shorthand(css: Signal<String>) -> NodeHandle {
    rsx! {
        div { style: {move || css.get()}, p: "12px" }
    }
}

#[test]
fn a_reactive_style_attribute_keeps_a_shorthand_prop() {
    let css = Signal::new(String::from("color: red"));
    let (_doc, _scope, div) = mount(|s| html_style_and_shorthand(s, css));
    assert_eq!(decl(&div, "padding").as_deref(), Some("12px"));
    assert_eq!(decl(&div, "color").as_deref(), Some("red"));

    css.set(String::from("color: blue"));
    assert_eq!(
        decl(&div, "padding").as_deref(),
        Some("12px"),
        "the shorthand prop is a second author on this attribute and the \
         re-firing style effect must not erase it"
    );
    assert_eq!(decl(&div, "color").as_deref(), Some("blue"));
}

/// An element with a `style:` and nothing else keeps the author's string
/// verbatim — the merge must not reformat what it has no reason to touch.
#[component]
fn html_style_only() -> NodeHandle {
    rsx! {
        div { style: "color:red;gap:4px" }
    }
}

#[test]
fn a_style_prop_with_no_second_author_is_written_through_untouched() {
    let (_doc, _scope, div) = mount(html_style_only);
    assert_eq!(
        div.get_attribute("style").as_deref(),
        Some("color:red;gap:4px")
    );
}
