//! Behavioural tests for *braced* control flow in `rsx!` node position
//! (issue #221).
//!
//! `RsxNode::parse` peeks `token::Brace` before it peeks `if`/`for`/`match`, so
//! a braced construct used to be parsed as a plain Rust expression and codegen'd
//! through `IntoNode::into_node` — evaluated exactly once. One brace apart:
//!
//! ```ignore
//! div { match x { … } }        // match_dom, reactive
//! div { { match x { … } } }    // a Rust match, rendered once, stale forever
//! ```
//!
//! Every shape below **compiles without the parser fix** and renders its first
//! value correctly; what it does not do is change. So each test here fails on
//! its second assertion before the fix — the counterfactual reproduces the
//! reported symptom exactly, rather than a compile error. The unbraced twins in
//! `rsx_nested_control_flow.rs` pass either way.
//!
//! A braced `for` is not here: `for` evaluates to `()`, which is not `IntoNode`,
//! so that shape never compiled and was never silently stale. Its parse is
//! pinned in `node.rs`'s unit tests instead.

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

/// The rendered text of a subtree, skipping the control-flow comment markers
/// (`<!-- match -->`, `<!-- if -->`) that `text_content` would otherwise splice in.
fn text(node: &NodeHandle) -> String {
    if node.node_type() == Some(8) {
        return String::new();
    }
    let children = node.children();
    if children.is_empty() {
        return node.text_content().unwrap_or_default();
    }
    children.iter().map(text).collect()
}

// ============================================================
// Braced control flow as an element child
// ============================================================

#[component]
fn braced_match(sel: Signal<u32>) -> NodeHandle {
    rsx! {
        div {
            { match sel.get() {
                0 => "zero",
                _ => "many",
            } }
        }
    }
}

#[test]
fn a_braced_match_in_node_position_is_reactive() {
    let sel = Signal::new(0u32);
    let (_doc, _scope, root) = mount(|s| braced_match(s, sel));

    assert_eq!(text(&root), "zero");
    sel.set(3);
    assert_eq!(text(&root), "many", "the braced match must re-run");
}

#[component]
fn braced_if(flag: Signal<bool>) -> NodeHandle {
    rsx! {
        div {
            { if flag.get() { "on" } else { "off" } }
        }
    }
}

#[test]
fn a_braced_if_in_node_position_is_reactive() {
    let flag = Signal::new(false);
    let (_doc, _scope, root) = mount(|s| braced_if(s, flag));

    assert_eq!(text(&root), "off");
    flag.set(true);
    assert_eq!(text(&root), "on", "the braced if must re-run");
}

// ============================================================
// Braced control flow as a match arm body — what #221 reports
// ============================================================

#[component]
fn braced_match_in_match_arm(outer: Signal<u32>, inner: Signal<u32>) -> NodeHandle {
    rsx! {
        div {
            match outer.get() {
                0 => span { "none" },
                _ => { match inner.get() {
                    0 => "zero",
                    _ => "many",
                } },
            }
        }
    }
}

#[test]
fn a_braced_match_inside_a_match_arm_tracks_its_own_signal() {
    let outer = Signal::new(1u32);
    let inner = Signal::new(0u32);
    let (_doc, _scope, root) = mount(|s| braced_match_in_match_arm(s, outer, inner));

    assert_eq!(text(&root), "zero");
    inner.set(1);
    assert_eq!(
        text(&root),
        "many",
        "the inner match must re-run on its own"
    );
    outer.set(0);
    assert_eq!(text(&root), "none");
}

#[component]
fn braced_if_in_match_arm(outer: Signal<u32>, flag: Signal<bool>) -> NodeHandle {
    rsx! {
        div {
            match outer.get() {
                0 => span { "none" },
                _ => { if flag.get() { "on" } else { "off" } },
            }
        }
    }
}

#[test]
fn a_braced_if_inside_a_match_arm_tracks_its_own_signal() {
    let outer = Signal::new(1u32);
    let flag = Signal::new(false);
    let (_doc, _scope, root) = mount(|s| braced_if_in_match_arm(s, outer, flag));

    assert_eq!(text(&root), "off");
    flag.set(true);
    assert_eq!(text(&root), "on");
}

// ============================================================
// Braced control flow deeper in the tree
// ============================================================

#[component]
fn braced_match_in_if_branch(shown: Signal<bool>, inner: Signal<u32>) -> NodeHandle {
    rsx! {
        div {
            if shown.get() {
                div {
                    { match inner.get() {
                        0 => "zero",
                        _ => "many",
                    } }
                }
            } else {
                span { "hidden" }
            }
        }
    }
}

#[test]
fn a_braced_match_inside_an_if_branch_tracks_its_own_signal() {
    let shown = Signal::new(true);
    let inner = Signal::new(0u32);
    let (_doc, _scope, root) = mount(|s| braced_match_in_if_branch(s, shown, inner));

    assert_eq!(text(&root), "zero");
    inner.set(2);
    assert_eq!(text(&root), "many");
    shown.set(false);
    assert_eq!(text(&root), "hidden");
}

#[component]
fn braced_if_in_for_body(flag: Signal<bool>) -> NodeHandle {
    let ids = vec![1u32, 2];
    rsx! {
        div {
            for id in ids.clone() {
                span { key: id, data-id: {id.to_string()},
                    { if flag.get() { "on" } else { "off" } }
                }
            }
        }
    }
}

#[test]
fn a_braced_if_inside_a_for_body_tracks_its_own_signal() {
    let flag = Signal::new(false);
    let (_doc, _scope, root) = mount(|s| braced_if_in_for_body(s, flag));

    assert_eq!(text(&root), "offoff");
    flag.set(true);
    assert_eq!(text(&root), "onon");
}
