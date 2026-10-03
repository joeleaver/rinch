//! A component's `icon` / `*_icon` prop can take a reactive closure (issue #718).
//!
//! `has_reactive_component_props` / `has_reactive_props` already noticed a
//! closure-valued `icon:`/`*_icon:` prop and routed the whole component onto
//! the reactive path (`reactive_component_dom`, re-render on signal change) —
//! the macro never silently froze it. What was wrong was **branch order** in
//! `generate_component_field_assignments`
//! (`crates/rinch-macros/src/dom_codegen/component_codegen.rs`): the
//! `icon`/`*_icon` arm is checked *before* the `invoke_closures` arm that
//! would call the closure, so it always wrapped the raw closure expression as
//! `Some(#value)` — a closure where the struct field wants a plain
//! `TablerIcon`/`Option<TablerIcon>` — which is a **compile error**, not a
//! silent freeze.
//!
//! `an_icon_prop_can_be_a_reactive_closure` pins the exact repro from the
//! issue body (`List { icon: {move || which.get()}, ... }`) actually
//! compiling and rendering. `a_custom_components_icon_prop_updates_when_the_signal_changes`
//! pins the mechanism directly: a minimal hand-rolled `Component` with an
//! `icon: Option<TablerIcon>` field, read through the painted SVG `<path d>`
//! data (the one observable difference between two icons through this
//! backend-agnostic mock), so the fixture fails if the closure is invoked once
//! and never again as well as if it is never invoked at all.
//!
//! Mutant: restoring the old unconditional `quote! { #name: Some(#value) } }`
//! for the icon arm (i.e. dropping the `invoke_closures`/`get_closure_expr`
//! branch added by this fix) makes both tests **fail to compile** — the
//! closure-vs-field-type mismatch is exactly the issue's own `error[E0308]`,
//! confirmed by reverting the fix locally and re-running `cargo test`.

use rinch::prelude::*;
use rinch_core::dom::DomDocument;
use rinch_core::dom::mock::MockDomDocument;
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};
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

/// Walk down to the first `<path>` under `node` and return its `d` attribute —
/// the one piece of the rendered SVG that differs between two distinct
/// `TablerIcon` variants through this headless backend.
fn first_path_d(node: &NodeHandle) -> Option<String> {
    if node.tag_name().as_deref() == Some("path") {
        return node.get_attribute("d");
    }
    for child in node.children() {
        if let Some(d) = first_path_d(&child) {
            return Some(d);
        }
    }
    None
}

// ============================================================
// The issue's own repro: a real component, a real icon enum
// ============================================================

#[test]
fn an_icon_prop_can_be_a_reactive_closure() {
    let which = Signal::new(TablerIcon::Check);

    let (_doc, _scope, root) = mount(|__scope| {
        rsx! {
            List {
                icon: {move || which.get()},
                ListItem { "one" }
            }
        }
    });

    let before = first_path_d(&root).expect("List renders its default icon into each item");

    which.set(TablerIcon::X);
    let after = first_path_d(&root).expect("still renders an icon after the signal changes");

    assert_ne!(
        before, after,
        "the icon actually changed when the reactive closure's signal flipped"
    );
}

// ============================================================
// Isolated mechanism: a minimal component with an `icon` field
// ============================================================

/// A minimal component with a bare `icon: Option<TablerIcon>` field, so the
/// fixture exercises exactly the `component_codegen` arm this issue is about
/// (`name_str == "icon"`) with none of `List`'s own container-default
/// patching (#707) in the way.
#[derive(Debug, Default)]
struct IconProbe {
    icon: Option<TablerIcon>,
}

impl Component for IconProbe {
    fn render(&self, scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let root = scope.create_element("div");
        if let Some(icon) = self.icon {
            let svg = render_tabler_icon(scope, icon, TablerIconStyle::Outline);
            root.append_child(&svg);
        }
        root
    }
}

#[test]
fn a_custom_components_icon_prop_updates_when_the_signal_changes() {
    let which = Signal::new(TablerIcon::Check);

    let (_doc, _scope, root) = mount(|__scope| {
        rsx! {
            IconProbe { icon: {move || which.get()} }
        }
    });

    let check_d = first_path_d(&root).expect("Check renders a path");

    which.set(TablerIcon::X);
    let x_d = first_path_d(&root).expect("X renders a path too, after the signal changes");

    assert_ne!(
        check_d, x_d,
        "the closure was invoked again and its new value reached the icon field"
    );

    // And back again, to rule out a one-shot "invoked exactly once more" bug.
    which.set(TablerIcon::Check);
    let check_again_d = first_path_d(&root).expect("Check renders a path again");
    assert_eq!(
        check_d, check_again_d,
        "flipping the signal back renders the original icon again"
    );
}

// ============================================================
// Unchanged: a static (non-reactive) icon value still just wraps in `Some`
// ============================================================

#[test]
fn a_static_icon_prop_is_unchanged() {
    let (_doc, _scope, root) = mount(|__scope| {
        rsx! {
            IconProbe { icon: TablerIcon::Heart }
        }
    });
    assert!(
        first_path_d(&root).is_some(),
        "a plain (non-closure) icon value still renders, unwrapped by the macro into Some(...)"
    );
}
