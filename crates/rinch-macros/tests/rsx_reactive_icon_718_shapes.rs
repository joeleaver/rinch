//! #718: every icon-prop shape a reactive closure can take (move closures with
//! non-Copy captures, block expressions, the section and stepper icons), checked
//! by what the component paints.

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

// ---------------------------------------------------------------
// 1. left_section / right_section (NavLink) — NOT "icon"/"*_icon"
//    named, so routed through the GENERIC invoke_closures arm, not
//    the PR's new arm. Confirms these already work (via .into()),
//    unaffected by / independent of this PR.
// ---------------------------------------------------------------
#[test]
fn navlink_left_section_reactive_closure_compiles_and_rerenders() {
    let which = Signal::new(TablerIcon::Check);
    let (_doc, _scope, root) = mount(|__scope| {
        rsx! {
            NavLink {
                label: "hi",
                left_section: {move || which.get()},
            }
        }
    });
    let before = first_path_d(&root).expect("renders left_section icon");
    which.set(TablerIcon::X);
    let after = first_path_d(&root).expect("still renders after signal change");
    assert_ne!(
        before, after,
        "left_section updates reactively (generic .into() arm)"
    );
}

// ---------------------------------------------------------------
// 2. completed_icon / progress_icon on Stepper — these DO end with
//    "_icon" so they ARE in the PR's new arm.
// ---------------------------------------------------------------
#[test]
fn stepper_completed_icon_reactive_closure_compiles() {
    let which = Signal::new(TablerIcon::Check);
    let (_doc, _scope, _root) = mount(|__scope| {
        rsx! {
            Stepper {
                active: 0u32,
                completed_icon: {move || which.get()},
                StepperStep { label: "one" }
            }
        }
    });
    which.set(TablerIcon::X);
    // Compiling + running without panicking is the win here.
}

// ---------------------------------------------------------------
// 3. A closure returning Option<TablerIcon> for an icon-family prop.
//    Expect a compile error (type mismatch) -- NOT silently
//    double-wrapped into a valid-looking but wrong value. This is
//    a #[cfg] compile-fail style probe done manually: left commented
//    with the observed error pasted below for the report.
// ---------------------------------------------------------------
// IconProbe { icon: {move || if which.get() == TablerIcon::Check { Some(TablerIcon::X) } else { None }} }
// => E0308 expected enum `TablerIcon`, found enum `Option<TablerIcon>` (manually verified, see report)

// ---------------------------------------------------------------
// 4. Block-expression closure (setup + move closure) for `icon`.
// ---------------------------------------------------------------
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
fn icon_block_expression_with_setup_compiles_and_rerenders() {
    let which = Signal::new(TablerIcon::Check);
    let (_doc, _scope, root) = mount(|__scope| {
        rsx! {
            IconProbe {
                icon: {
                    let fallback = TablerIcon::Heart;
                    move || if which.get() == TablerIcon::Check { fallback } else { which.get() }
                }
            }
        }
    });
    let before = first_path_d(&root).expect("renders");
    which.set(TablerIcon::X);
    let after = first_path_d(&root).expect("still renders");
    assert_ne!(
        before, after,
        "block-expression closure re-renders on signal change"
    );
}

// ---------------------------------------------------------------
// 5. A variable holding a TablerIcon (non-closure, non-literal).
// ---------------------------------------------------------------
#[test]
fn icon_plain_variable_unchanged() {
    let t = TablerIcon::Heart;
    let (_doc, _scope, root) = mount(|__scope| {
        rsx! {
            IconProbe { icon: t }
        }
    });
    assert!(first_path_d(&root).is_some());
}

// ---------------------------------------------------------------
// 7. Non-PascalCase HTML element with an attribute literally named
//    "icon" -- must go through generate_attr_code, unaffected.
// ---------------------------------------------------------------
#[test]
fn html_element_icon_attribute_is_a_plain_string_attribute() {
    let (_doc, _scope, root) = mount(|__scope| {
        rsx! {
            div { icon: "not-a-tabler-icon-prop", "hi" }
        }
    });
    assert_eq!(
        root.get_attribute("icon"),
        Some("not-a-tabler-icon-prop".to_string())
    );
}

// ---------------------------------------------------------------
// 8. A move closure capturing a NON-Copy value (String) for `icon`,
//    where the component ALSO has another reactive prop so the
//    whole thing re-renders (icon closure reconstructed every
//    render). Pins the `is_move_closure` shadow-clone branch: if the
//    mutant drops shadow-cloning, the captured String would need to
//    be moved out of the render closure's own `&self` capture on
//    every invocation -> compile error (E0507), not a runtime bug.
// ---------------------------------------------------------------
#[derive(Debug, Default)]
struct DualProbe {
    icon: Option<TablerIcon>,
    label: String,
}

impl Component for DualProbe {
    fn render(&self, scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let root = scope.create_element("div");
        root.set_attribute("data-label", &self.label);
        if let Some(icon) = self.icon {
            let svg = render_tabler_icon(scope, icon, TablerIconStyle::Outline);
            root.append_child(&svg);
        }
        root
    }
}

#[test]
fn icon_move_closure_with_non_copy_capture_compiles_and_rerenders() {
    let which = Signal::new(TablerIcon::Check);
    let label = Signal::new(0i32);
    let owned = String::from("fallback-icon-name");

    let (_doc, _scope, root) = mount(|__scope| {
        rsx! {
            DualProbe {
                // `label` closure makes the whole component reactive.
                label: {move || label.get().to_string()},
                // `owned` is a non-Copy move-capture: without shadow-cloning,
                // this closure (reconstructed on every re-render of the
                // outer `Fn` render closure) would try to move `owned` out
                // of that render closure's own captured environment on each
                // call -- E0507.
                icon: {move || { let _ = &owned; which.get() }},
            }
        }
    });

    let before = first_path_d(&root).expect("renders");
    which.set(TablerIcon::X);
    label.set(1);
    let after = first_path_d(&root).expect("still renders after both signals change");
    assert_ne!(
        before, after,
        "icon closure re-invoked and re-rendered with a non-Copy move capture"
    );
}
