//! A hyphenated attribute written on a **component** lands on the component's
//! root DOM element (issue #433).
//!
//! `rsx!` accepted `data-nofocus: ""` on an HTML element and panicked on a
//! component ("hyphenated attribute … cannot be used as a struct field"), so a
//! runtime-read attribute meant for a component's box needed a wrapper element
//! of its own — a layout change to work around syntax. A hyphenated name can
//! never be a Rust field, so it cannot collide with a prop: the macro now
//! routes it to the root after `Component::render`, the way `style:` and
//! `class:` already travel.
//!
//! Four codegen sites apply it, and each has fixtures here:
//!
//! | site | reached by |
//! |---|---|
//! | `component_codegen::element_to_dom_component` (static) | a component with no reactive struct prop |
//! | the same, reactive attribute | a `{|| …}` attribute on such a component — an effect on the stable root, no re-render |
//! | `component_codegen::element_to_dom_component_reactive` | a component with a reactive struct prop, used as the rsx root |
//! | `component_codegen::generate_reactive_component_stmt` | the same, as a child of an element |
//!
//! The mock is a pass-through (`set_attribute` inserts, `remove_attribute`
//! erases), so these observe the macro's output with no backend policy in
//! between.

use rinch::prelude::*;
use rinch_core::dom::DomDocument;
use rinch_core::dom::mock::MockDomDocument;
use std::cell::RefCell;
use std::rc::Rc;

fn mount(
    build: impl FnOnce(&mut RenderScope) -> NodeHandle,
) -> (Rc<RefCell<MockDomDocument>>, RenderScope, NodeHandle) {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let root = build(&mut scope);
    (doc, scope, root)
}

/// A component that writes attributes of its own on its root, so a caller's
/// write has something to collide with, and renders its label so a re-render
/// is observable.
#[component]
fn Tagged(label: String) -> NodeHandle {
    let root = __scope.create_element("section");
    root.set_attribute("class", "tagged");
    root.set_attribute("data-role", "inner");
    root.set_attribute("data-nofocus", "");
    root.set_attribute("aria-label", &label);
    root
}

/// The `.tagged` element at or below `node` — found by class, not by the
/// attributes under test.
fn find_tagged(node: &NodeHandle) -> Option<NodeHandle> {
    if node.get_attribute("class").as_deref() == Some("tagged") {
        return Some(node.clone());
    }
    node.children().iter().find_map(find_tagged)
}

// ── 1. static values on the static path ─────────────────────────────────────

#[component]
fn static_attrs() -> NodeHandle {
    rsx! {
        Tagged { label: "Toolbar", data-testid: "bar", aria-describedby: "help" }
    }
}

/// The reported shape: a literal hyphenated attribute compiles and lands on the
/// component's own root, not on a wrapper.
#[test]
fn a_literal_hyphenated_attribute_lands_on_the_component_root() {
    let (_doc, _scope, root) = mount(static_attrs);
    assert_eq!(
        root.get_attribute("class").as_deref(),
        Some("tagged"),
        "the static path returns the component's root itself"
    );
    assert_eq!(root.get_attribute("data-testid").as_deref(), Some("bar"));
    assert_eq!(
        root.get_attribute("aria-describedby").as_deref(),
        Some("help")
    );
    assert_eq!(
        root.get_attribute("data-role").as_deref(),
        Some("inner"),
        "an attribute the caller does not name is the component's and is kept"
    );
}

#[component]
fn caller_overrides() -> NodeHandle {
    rsx! {
        Tagged { label: "Toolbar", data-role: "outer", aria-label: "Formatting" }
    }
}

/// Last writer wins, and the caller writes last: a collision with an attribute
/// the component wrote itself goes to the caller.
#[test]
fn the_caller_wins_a_collision_with_the_components_own_attribute() {
    let (_doc, _scope, root) = mount(caller_overrides);
    assert_eq!(root.get_attribute("data-role").as_deref(), Some("outer"));
    assert_eq!(
        root.get_attribute("aria-label").as_deref(),
        Some("Formatting")
    );
}

// ── 2. boolean attributes go through write_attribute ────────────────────────

#[component]
fn boolean_attrs() -> NodeHandle {
    rsx! {
        Tagged { label: "x", data-nofocus: false, data-trap-focus: true, aria-hidden: "false" }
    }
}

/// A rinch boolean `data-` attribute follows #551's shape: truthy is the bare
/// presence form, falsey **removes** — including one the component itself wrote
/// (`data-nofocus`), since the caller is the last writer. A non-boolean
/// attribute keeps a meaningful `"false"` verbatim.
#[test]
fn a_boolean_attribute_is_a_presence_and_false_removes_it() {
    let (_doc, _scope, root) = mount(boolean_attrs);
    assert_eq!(
        root.get_attribute("data-nofocus"),
        None,
        "a falsey write removes the attribute the component put there; writing \
         \"false\" would leave it present"
    );
    assert_eq!(
        root.get_attribute("data-trap-focus").as_deref(),
        Some(""),
        "true is the bare presence form"
    );
    assert_eq!(
        root.get_attribute("aria-hidden").as_deref(),
        Some("false"),
        "`aria-*` is not boolean: its \"false\" is a value, not an absence"
    );
}

// ── 3. a reactive attribute on the static path ──────────────────────────────

#[component]
fn reactive_attrs(state: Signal<String>, guard: Signal<bool>) -> NodeHandle {
    rsx! {
        Tagged {
            label: "x",
            data-state: {move || state.get()},
            data-nofocus: {move || guard.get()},
        }
    }
}

/// A `{|| …}` attribute is an effect on the component's (stable) root: it
/// re-applies on change and does not re-render the component.
#[test]
fn a_reactive_attribute_follows_its_signal_without_a_re_render() {
    let state = Signal::new("idle".to_string());
    let guard = Signal::new(false);
    let (_doc, _scope, root) = mount(|s| reactive_attrs(s, state, guard));

    assert_eq!(
        root.get_attribute("class").as_deref(),
        Some("tagged"),
        "a reactive attribute alone must not send the component down the \
         re-render path, whose result is a wrapper"
    );
    assert_eq!(root.get_attribute("data-state").as_deref(), Some("idle"));
    assert_eq!(root.get_attribute("data-nofocus"), None);

    state.set("busy".to_string());
    guard.set(true);
    assert_eq!(root.get_attribute("data-state").as_deref(), Some("busy"));
    assert_eq!(root.get_attribute("data-nofocus").as_deref(), Some(""));

    guard.set(false);
    assert_eq!(
        root.get_attribute("data-nofocus"),
        None,
        "false removes the boolean attribute again"
    );
}

#[component]
fn dynamic_expr_attr(id: String) -> NodeHandle {
    rsx! {
        Tagged { label: "x", data-id: id.clone() }
    }
}

/// A plain (non-literal, non-closure) expression is written too.
#[test]
fn a_plain_expression_attribute_is_written() {
    let (_doc, _scope, root) = mount(|s| dynamic_expr_attr(s, "row-7".to_string()));
    assert_eq!(root.get_attribute("data-id").as_deref(), Some("row-7"));
}

// ── 4. a component that re-renders ──────────────────────────────────────────

#[component]
fn rerendering_root(label: Signal<String>, state: Signal<String>) -> NodeHandle {
    rsx! {
        Tagged {
            label: {move || label.get()},
            data-role: "outer",
            data-state: {move || state.get()},
            data-nofocus: false,
        }
    }
}

/// A reactive struct prop re-renders the component into a fresh root; the
/// attributes must be on that new root, every time.
#[test]
fn a_re_rendered_component_gets_its_attributes_back_on_the_new_root() {
    let label = Signal::new("one".to_string());
    let state = Signal::new("a".to_string());
    let (_doc, _scope, wrapper) = mount(|s| rerendering_root(s, label, state));

    let first = find_tagged(&wrapper).expect("rendered");
    assert_eq!(first.get_attribute("aria-label").as_deref(), Some("one"));
    assert_eq!(first.get_attribute("data-role").as_deref(), Some("outer"));
    assert_eq!(first.get_attribute("data-state").as_deref(), Some("a"));
    assert_eq!(first.get_attribute("data-nofocus"), None);

    label.set("two".to_string());
    let second = find_tagged(&wrapper).expect("re-rendered");
    assert_ne!(
        first.node_id(),
        second.node_id(),
        "positive control: the label change really re-rendered into a new root"
    );
    assert_eq!(second.get_attribute("aria-label").as_deref(), Some("two"));
    assert_eq!(second.get_attribute("data-role").as_deref(), Some("outer"));
    assert_eq!(second.get_attribute("data-state").as_deref(), Some("a"));
    assert_eq!(second.get_attribute("data-nofocus"), None);

    state.set("b".to_string());
    let third = find_tagged(&wrapper).expect("still rendered");
    assert_eq!(third.get_attribute("data-state").as_deref(), Some("b"));
    assert_eq!(third.get_attribute("data-role").as_deref(), Some("outer"));
}

#[component]
fn rerendering_child(label: Signal<String>, state: Signal<String>) -> NodeHandle {
    rsx! {
        div {
            Tagged {
                label: {move || label.get()},
                data-role: "outer",
                data-state: {move || state.get()},
                data-nofocus: false,
            }
        }
    }
}

/// The same, where the component is a child of an element (the statement path).
#[test]
fn a_re_rendered_child_component_gets_its_attributes_back() {
    let label = Signal::new("one".to_string());
    let state = Signal::new("a".to_string());
    let (_doc, _scope, root) = mount(|s| rerendering_child(s, label, state));

    let first = find_tagged(&root).expect("rendered");
    assert_eq!(first.get_attribute("data-role").as_deref(), Some("outer"));
    assert_eq!(first.get_attribute("data-state").as_deref(), Some("a"));

    label.set("two".to_string());
    let second = find_tagged(&root).expect("re-rendered");
    assert_ne!(first.node_id(), second.node_id(), "positive control");
    assert_eq!(second.get_attribute("aria-label").as_deref(), Some("two"));
    assert_eq!(second.get_attribute("data-role").as_deref(), Some("outer"));
    assert_eq!(second.get_attribute("data-state").as_deref(), Some("a"));
    assert_eq!(
        second.get_attribute("data-nofocus"),
        None,
        "a boolean attribute goes through write_attribute on the re-render path too"
    );

    state.set("b".to_string());
    let third = find_tagged(&root).expect("still rendered");
    assert_eq!(third.get_attribute("data-state").as_deref(), Some("b"));
}

#[component]
fn reactive_attr_on_a_child(state: Signal<String>) -> NodeHandle {
    rsx! {
        div {
            Tagged { label: "x", data-state: {move || state.get()} }
        }
    }
}

/// A child component whose only reactive prop is a hyphenated attribute is not
/// a re-rendering component: the attribute is an effect on its stable root, so
/// the root (and any state inside it) survives the change.
#[test]
fn a_reactive_attribute_on_a_child_does_not_re_render_it() {
    let state = Signal::new("idle".to_string());
    let (_doc, _scope, root) = mount(|s| reactive_attr_on_a_child(s, state));

    let first = find_tagged(&root).expect("rendered");
    assert_eq!(first.get_attribute("data-state").as_deref(), Some("idle"));

    state.set("busy".to_string());
    let second = find_tagged(&root).expect("still rendered");
    assert_eq!(second.get_attribute("data-state").as_deref(), Some("busy"));
    assert_eq!(
        first.node_id(),
        second.node_id(),
        "an attribute change must not rebuild the component"
    );
}

// ── 5. a real library component ─────────────────────────────────────────────

#[component]
fn group_toolbar() -> NodeHandle {
    rsx! {
        Group { gap: "xs", data-nofocus: "", role-description: "toolbar",
            span { "B" }
        }
    }
}

/// `Group` — the toolbar container #433 was filed for — carries the attribute
/// on its own root, alongside its own class.
#[test]
fn a_group_carries_data_nofocus_on_its_root() {
    let (_doc, _scope, root) = mount(group_toolbar);
    assert!(
        root.get_attribute("class")
            .unwrap_or_default()
            .contains("rinch-group"),
        "the result is the Group's own root"
    );
    assert_eq!(root.get_attribute("data-nofocus").as_deref(), Some(""));
    assert_eq!(
        root.get_attribute("role-description").as_deref(),
        Some("toolbar")
    );
}
