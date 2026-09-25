//! A braced `match` arm body is decided by its **first token** (issue #395).
//!
//! It holds rsx children — several nodes, as an `if`/`for` body does — when it
//! opens with `let`, a string literal, `if`/`for`/`match`, or `Name {` (an
//! element or a component). Anything else (`{ section(__scope) }`,
//! `{a.clone()}`, `{|| …}`) is the one expression it has always been.
//!
//! Before #395 only the `let` spelling reached children; every multi-node shape
//! below was a compile error (`unexpected token, expected }`), so this file does
//! not compile at the pre-fix commit. The expression arms pass either way and
//! are here to pin that the first-token rule did not take them.

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

/// Rendered text, skipping control-flow comment markers.
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

#[component]
fn Chip(label: String) -> NodeHandle {
    rsx! { span { {label.clone()} } }
}

fn section(__scope: &mut RenderScope, name: &str) -> NodeHandle {
    let name = name.to_string();
    rsx! { p { {name} } }
}

#[component]
fn arms(sel: Signal<u32>, flag: Signal<bool>, handle: NodeHandle) -> NodeHandle {
    rsx! {
        div {
            match sel.get() {
                // element + element
                0 => { b { "A" } i { "B" } },
                // element + text
                1 => { b { "C" } "D" },
                // two components
                2 => { Chip { label: "E" } Chip { label: "F" } },
                // text first
                3 => { "G" em { "H" } },
                // control flow first, then a node: the `if` stays reactive
                4 => { if flag.get() { "on" } else { "off" } span { "!" } },
                // `let` still opens children, now with several nodes
                5 => { let k = "K"; b { {k} } i { "L" } },
                // expression arms, unchanged: a call, a captured handle, a closure
                6 => { section(__scope, "call") },
                7 => { handle.clone() },
                8 => {|| format!("n{}", sel.get())},
                _ => "rest",
            }
        }
    }
}

#[test]
fn every_braced_arm_shape_renders_and_switches() {
    let sel = Signal::new(0u32);
    let flag = Signal::new(false);
    let (_doc, _scope, root) = mount(|s| {
        let handle = s.create_text("captured");
        arms(s, sel, flag, handle)
    });

    let want = [
        (0, "AB"),
        (1, "CD"),
        (2, "EF"),
        (3, "GH"),
        (4, "off!"),
        (5, "KL"),
        (6, "call"),
        (7, "captured"),
        (8, "n8"),
        (9, "rest"),
    ];
    for (n, expected) in want {
        sel.set(n);
        assert_eq!(text(&root), expected, "arm {n}");
    }
    // Back through the multi-node arms, off the order they were first shown in.
    for (n, expected) in [(2, "EF"), (0, "AB"), (7, "captured"), (5, "KL")] {
        sel.set(n);
        assert_eq!(text(&root), expected, "arm {n} again");
    }
}

#[test]
fn control_flow_leading_a_braced_arm_tracks_its_own_signal() {
    let sel = Signal::new(4u32);
    let flag = Signal::new(false);
    let (_doc, _scope, root) = mount(|s| {
        let handle = s.create_text("captured");
        arms(s, sel, flag, handle)
    });
    assert_eq!(text(&root), "off!");
    flag.set(true);
    assert_eq!(
        text(&root),
        "on!",
        "the leading `if` must re-run on its own"
    );
    flag.set(false);
    assert_eq!(text(&root), "off!");
}
