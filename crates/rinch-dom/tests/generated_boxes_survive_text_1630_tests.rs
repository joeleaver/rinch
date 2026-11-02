//! `set_text_content` and `set_inner_html` on an element replace its DOM
//! children and keep its generated `::before` / `::after` boxes (#1630).
//!
//! The generated boxes are ordinary entries of the element's child list
//! (`is_pseudo_element`), made when the element is cascaded. Both verbs used
//! to take every child away, the generated boxes with them, and neither
//! re-cascades the element, so the boxes stayed gone until something else
//! restyled it. In a browser `textContent =` / `innerHTML =` replace the DOM
//! children only; generated content is not a DOM child. A reactive
//! `{|| text}` written over an element (`NodeHandle::set_text`) is the first
//! verb.
//!
//! Assertions are on child-list shape, the IFC's flat text and node identity —
//! no measured literal. Font size and line height are declared anyway.

#![cfg(feature = "software-renderer")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{DomDocument, NodeId, RenderScope};
use rinch_core::reactive::Signal;
use rinch_dom::RinchDocument;

const CSS: &str = "body { font-size: 16px; line-height: 20px; margin: 0 } \
                   p { margin: 0; width: 600px } \
                   .a::after { content: \" SUFFIX\" } \
                   .b::before { content: \"PREFIX \" } \
                   .flex { display: flex } \
                   .ba::before { content: \"Y\"; display: block; width: 20px; height: 10px } \
                   .aa::after { content: \"Z\"; display: block; width: 30px; height: 10px }";

fn doc(class: &str, children: &[&str]) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let p = doc.create_element("p");
    doc.set_attribute(p, "class", class);
    doc.append_child(body, p);
    for c in children {
        let t = doc.create_text(c);
        doc.append_child(p, t);
    }
    doc.resolve_layout(800.0, 400.0);
    (doc, p)
}

fn flat(doc: &RinchDocument, p: NodeId) -> String {
    doc.tree
        .get(p.0)
        .unwrap()
        .text_layout
        .as_ref()
        .expect("the element is an IFC root")
        .text_content
        .clone()
}

/// `G` for a generated box, `.` for anything else, in child-list order.
fn shape(doc: &RinchDocument, p: NodeId) -> String {
    doc.tree
        .get(p.0)
        .unwrap()
        .children
        .iter()
        .map(|&c| {
            if doc.tree.get(c).unwrap().is_pseudo_element {
                'G'
            } else {
                '.'
            }
        })
        .collect()
}

fn children(doc: &RinchDocument, p: NodeId) -> Vec<u64> {
    doc.tree.get(p.0).unwrap().children.clone()
}

// ── set_text_content ────────────────────────────────────────────────────────

#[test]
fn set_text_keeps_both_generated_boxes() {
    let (mut d, p) = doc("a b", &["old ", "words"]);
    d.set_text_content(p, "new");
    d.resolve_layout(801.0, 400.0);
    assert_eq!(shape(&d, p), "G.G");
    assert_eq!(flat(&d, p), "PREFIX new SUFFIX");
}

/// An element whose only children are its generated boxes gets its text
/// between them — after the `::before`, before the `::after`.
#[test]
fn set_text_on_an_element_holding_only_generated_boxes() {
    for (class, want) in [
        ("a b", "PREFIX new SUFFIX"),
        ("a", "new SUFFIX"),
        ("b", "PREFIX new"),
    ] {
        let (mut d, p) = doc(class, &[]);
        d.set_text_content(p, "new");
        d.resolve_layout(801.0, 400.0);
        assert_eq!(flat(&d, p), want, "{class}");
    }
}

/// The #1487 / #1440 fast path: an element whose only *own* child is a text
/// node keeps that node and writes into it, generated boxes or not. It used
/// to need the text to be the only child of all, so with a `::before` every
/// write orphaned one node and minted another.
#[test]
fn set_text_writes_the_lone_own_text_node_in_place() {
    let (mut d, p) = doc("a b", &["one"]);
    let before = children(&d, p);
    for i in 0..5 {
        d.set_text_content(p, &format!("v{i}"));
        d.resolve_layout(801.0 + i as f32, 400.0);
    }
    assert_eq!(children(&d, p), before, "the same three nodes");
    assert_eq!(flat(&d, p), "PREFIX v4 SUFFIX");
}

/// An identical write is a no-op with generated boxes around the text too.
#[test]
fn an_identical_set_text_changes_nothing() {
    let (mut d, p) = doc("a b", &["same"]);
    let gen_before = d.tree.hit_cache.generation();
    d.set_text_content(p, "same");
    assert_eq!(d.tree.hit_cache.generation(), gen_before, "nothing moved");
}

/// Block generated boxes in a flex container: real Taffy children, so the
/// new text's Taffy attach decides where it is laid out. Compared with the
/// same element built with that text from the start.
#[test]
fn set_text_in_a_flex_container_lays_out_as_built() {
    let geometry = |d: &RinchDocument, p: NodeId| -> Vec<(f32, f32, f32, f32)> {
        children(d, p)
            .iter()
            .filter(|&&c| d.tree.get(c).unwrap().is_element())
            .map(|&c| {
                let l = &d.tree.get(c).unwrap().layout;
                (l.x, l.y, l.width, l.height)
            })
            .collect()
    };
    let (mut d, p) = doc("flex ba aa", &["x", "y"]);
    d.set_text_content(p, "new");
    d.resolve_layout(801.0, 400.0);
    d.resolve_layout(800.0, 400.0);
    let (fresh, q) = doc("flex ba aa", &["new"]);
    assert_eq!(shape(&d, p), "G.G");
    assert_eq!(geometry(&d, p), geometry(&fresh, q));
}

// ── set_inner_html ──────────────────────────────────────────────────────────

#[test]
fn set_inner_html_keeps_both_generated_boxes() {
    let (mut d, p) = doc("a b", &["old"]);
    d.set_inner_html(p, "<b>new</b> words");
    d.resolve_layout(801.0, 400.0);
    assert_eq!(shape(&d, p), "G..G");
    assert_eq!(flat(&d, p), "PREFIX new words SUFFIX");
}

#[test]
fn set_inner_html_to_nothing_keeps_the_generated_boxes() {
    let (mut d, p) = doc("a b", &["old"]);
    d.set_inner_html(p, "");
    d.resolve_layout(801.0, 400.0);
    assert_eq!(shape(&d, p), "GG");
    assert_eq!(flat(&d, p), "PREFIX SUFFIX");
}

// ── through NodeHandle: a reactive text over an element ─────────────────────

#[test]
fn a_reactive_text_over_an_element_keeps_its_generated_boxes() {
    let doc = Rc::new(RefCell::new(RinchDocument::new()));
    doc.borrow_mut().load_css(CSS);
    let body = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    let mut scope = RenderScope::new(dyn_doc, body);
    let p = scope.create_element("p");
    p.set_attribute("class", "a b");
    scope.parent().append_child(&p);
    let label = Signal::new("one".to_string());
    let target = p.clone();
    scope.create_effect(move || target.set_text(&label.get()));
    doc.borrow_mut().resolve_layout(800.0, 400.0);
    label.set("two".into());
    doc.borrow_mut().resolve_layout(801.0, 400.0);
    let pid = NodeId(p.node_id().0);
    assert_eq!(flat(&doc.borrow(), pid), "PREFIX two SUFFIX");
}
