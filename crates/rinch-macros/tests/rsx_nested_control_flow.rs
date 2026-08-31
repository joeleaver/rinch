//! Behavioural tests for *unbraced* nested control flow in `rsx!` (issue #221).
//!
//! These are the positive control for the braced cases in
//! `rsx_braced_control_flow.rs`: written without the braces, a nested `match` /
//! `if` / `for` reaches the keyword peek in `RsxNode::parse`, becomes a real
//! `match_dom` / `show_dom` / `for_each_dom`, and updates. They pass **before**
//! the parser fix as well as after — which is the evidence that the reactive
//! runtime nests correctly and the parser is what was at fault.
//!
//! Assertions are on rendered DOM text, not on effect counts: what the issue
//! reports is a screen that stops changing.

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
/// (`<!-- match -->`, `<!-- if -->`, `<!-- for -->`) that `text_content` would
/// otherwise splice in.
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
// match inside match — the shape issue #221 reports
// ============================================================

#[component]
fn match_in_match(outer: Signal<u32>, inner: Signal<u32>) -> NodeHandle {
    rsx! {
        div {
            match outer.get() {
                0 => span { "none" },
                _ => div {
                    match inner.get() {
                        0 => span { "zero" },
                        _ => span { "many" },
                    }
                },
            }
        }
    }
}

#[test]
fn an_unbraced_match_inside_a_match_arm_tracks_its_own_signal() {
    let outer = Signal::new(1u32);
    let inner = Signal::new(0u32);
    let (_doc, _scope, root) = mount(|s| match_in_match(s, outer, inner));

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

// ============================================================
// The rest of the nesting matrix
// ============================================================

#[component]
fn if_in_match(outer: Signal<u32>, flag: Signal<bool>) -> NodeHandle {
    rsx! {
        div {
            match outer.get() {
                0 => span { "none" },
                _ => div {
                    if flag.get() {
                        span { "on" }
                    } else {
                        span { "off" }
                    }
                },
            }
        }
    }
}

#[test]
fn an_unbraced_if_inside_a_match_arm_tracks_its_own_signal() {
    let outer = Signal::new(1u32);
    let flag = Signal::new(false);
    let (_doc, _scope, root) = mount(|s| if_in_match(s, outer, flag));

    assert_eq!(text(&root), "off");
    flag.set(true);
    assert_eq!(text(&root), "on");
}

#[component]
fn match_in_if(shown: Signal<bool>, inner: Signal<u32>) -> NodeHandle {
    rsx! {
        div {
            if shown.get() {
                div {
                    match inner.get() {
                        0 => span { "zero" },
                        _ => span { "many" },
                    }
                }
            } else {
                span { "hidden" }
            }
        }
    }
}

#[test]
fn an_unbraced_match_inside_an_if_branch_tracks_its_own_signal() {
    let shown = Signal::new(true);
    let inner = Signal::new(0u32);
    let (_doc, _scope, root) = mount(|s| match_in_if(s, shown, inner));

    assert_eq!(text(&root), "zero");
    inner.set(2);
    assert_eq!(text(&root), "many");
    shown.set(false);
    assert_eq!(text(&root), "hidden");
}

#[component]
fn if_in_for(flag: Signal<bool>) -> NodeHandle {
    let ids = vec![1u32, 2];
    rsx! {
        div {
            for id in ids.clone() {
                span { key: id, data-id: {id.to_string()},
                    if flag.get() {
                        em { "on" }
                    } else {
                        em { "off" }
                    }
                }
            }
        }
    }
}

#[test]
fn an_unbraced_if_inside_a_for_body_tracks_its_own_signal() {
    let flag = Signal::new(false);
    let (_doc, _scope, root) = mount(|s| if_in_for(s, flag));

    assert_eq!(text(&root), "offoff");
    flag.set(true);
    assert_eq!(text(&root), "onon");
}

// ============================================================
// Why anyone reached for the brace in the first place (#223)
// ============================================================

#[component]
fn nested_match_over_a_non_copy_local(outer: Signal<u32>, inner: Signal<u32>) -> NodeHandle {
    // `label` is named by two arms of the inner match and by the outer one, so
    // three `move` closures want it. Until #223 landed, that was a hard E0382 /
    // E0507 from inside `rsx!` — and wrapping the inner match in braces + `rsx!`
    // was the workaround that compiled, which is exactly how code fell into
    // #221's silent staleness. The unbraced form now compiles, so the pressure
    // is gone.
    let label = String::from("row");
    rsx! {
        div {
            match outer.get() {
                0 => span { {label.clone()} },
                _ => div {
                    match inner.get() {
                        0 => span { {label.clone()} },
                        _ => span { {label.clone()} "!" },
                    }
                },
            }
        }
    }
}

#[test]
fn an_unbraced_nested_match_may_name_a_non_copy_local_in_every_arm() {
    let outer = Signal::new(1u32);
    let inner = Signal::new(0u32);
    let (_doc, _scope, root) = mount(|s| nested_match_over_a_non_copy_local(s, outer, inner));

    assert_eq!(text(&root), "row");
    inner.set(1);
    assert_eq!(text(&root), "row!");
    outer.set(0);
    assert_eq!(text(&root), "row");
}
