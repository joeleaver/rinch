//! An element's **inline** custom properties reach its `::before` / `::after`
//! (#888).
//!
//! A pseudo-element inherits from its originating element's computed style,
//! and the element's `style` attribute is part of that cascade. Chrome,
//! `.x::before { content: ""; display: block; width: var(--pw); color: var(--c) }`
//! on `<div class="x" style="--c: rgb(1,2,3); --pw: 33px">`:
//! `getComputedStyle(div, "::before")` is `width: 33px`,
//! `color: rgb(1, 2, 3)`.
//!
//! Every expected value is off the initial / inherited one (black, auto), so
//! a pseudo that misses the inline block cannot pass by coincidence.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::DimensionValue;

const VW: f32 = 400.0;
const VH: f32 = 200.0;

fn rgb(doc: &RinchDocument, n: usize) -> (u8, u8, u8) {
    let c = doc.tree.get(n).unwrap().computed_style.color;
    let rgba = c.expect("color should resolve").to_rgba8();
    (rgba.r, rgba.g, rgba.b)
}

fn width(doc: &RinchDocument, n: usize) -> DimensionValue {
    doc.tree.get(n).unwrap().computed_style.width.clone()
}

/// The generated box: first pseudo child for `::before`, last for `::after`.
fn generated(doc: &RinchDocument, owner: NodeId, before: bool) -> usize {
    let kids: Vec<usize> = doc
        .tree
        .get(owner.0)
        .unwrap()
        .children
        .iter()
        .copied()
        .filter(|&c| doc.tree.get(c).unwrap().is_pseudo_element)
        .collect();
    let b = if before { kids.first() } else { kids.last() };
    *b.expect("the element generates its pseudo box")
}

fn assert_33(doc: &RinchDocument, n: usize, what: &str) {
    assert!(
        matches!(width(doc, n), DimensionValue::Length(w) if w == 33.0),
        "{what}: width, got {:?}",
        width(doc, n)
    );
}

const RULES: &str = ".x::before, .x::after { content: \"\"; display: block; height: 4px; \
                     width: var(--pw); color: var(--c) }";

fn doc_with(css: &str, style: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(css);
    let body = doc.body();
    let x = doc.create_element("div");
    doc.set_attribute(x, "class", "x");
    if !style.is_empty() {
        doc.set_attribute(x, "style", style);
    }
    doc.append_child(body, x);
    doc.resolve_layout(VW, VH);
    (doc, x)
}

#[test]
fn inline_custom_properties_reach_before_and_after() {
    let (doc, x) = doc_with(RULES, "--c: rgb(1, 2, 3); --pw: 33px");
    for before in [true, false] {
        let g = generated(&doc, x, before);
        let what = if before { "::before" } else { "::after" };
        assert_eq!(rgb(&doc, g), (1, 2, 3), "{what}: color");
        assert_33(&doc, g, what);
        assert_eq!(doc.tree.get(g).unwrap().layout.width, 33.0, "{what}: laid-out width");
    }
}

#[test]
fn issue_fixture_exact() {
    let (doc, x) = doc_with(
        ".x::before { content: \"\"; display: block; width: var(--pw); color: var(--c); }",
        "--c: rgb(1,2,3); --pw: 33px",
    );
    let g = generated(&doc, x, true);
    assert_eq!(rgb(&doc, g), (1, 2, 3));
    assert_33(&doc, g, "::before");
}

#[test]
fn inline_set_later_via_attribute() {
    let (mut doc, x) = doc_with(RULES, "");
    doc.set_attribute(x, "style", "--c: rgb(1, 2, 3); --pw: 33px");
    doc.resolve_layout(VW + 1.0, VH);
    for before in [true, false] {
        let g = generated(&doc, x, before);
        assert_eq!(rgb(&doc, g), (1, 2, 3));
        assert_33(&doc, g, "later");
    }
}

#[test]
fn inline_changed_via_attribute() {
    let (mut doc, x) = doc_with(RULES, "--c: rgb(9, 9, 9); --pw: 10px");
    doc.set_attribute(x, "style", "--c: rgb(1, 2, 3); --pw: 33px");
    doc.resolve_layout(VW + 1.0, VH);
    for before in [true, false] {
        let g = generated(&doc, x, before);
        assert_eq!(rgb(&doc, g), (1, 2, 3));
        assert_33(&doc, g, "changed");
    }
}

#[test]
fn inline_changed_via_set_style() {
    let (mut doc, x) = doc_with(RULES, "--c: rgb(9, 9, 9); --pw: 10px");
    doc.set_style(x, "--c", "rgb(1, 2, 3)");
    doc.set_style(x, "--pw", "33px");
    doc.resolve_layout(VW + 1.0, VH);
    for before in [true, false] {
        let g = generated(&doc, x, before);
        assert_eq!(rgb(&doc, g), (1, 2, 3));
        assert_33(&doc, g, "set_style");
    }
}

#[test]
fn inline_changed_via_set_style_same_viewport() {
    let (mut doc, x) = doc_with(RULES, "--c: rgb(9, 9, 9); --pw: 10px");
    doc.set_style(x, "--c", "rgb(1, 2, 3)");
    doc.set_style(x, "--pw", "33px");
    doc.resolve_layout(VW, VH);
    for before in [true, false] {
        let g = generated(&doc, x, before);
        assert_eq!(rgb(&doc, g), (1, 2, 3));
        assert_33(&doc, g, "set_style same vp");
    }
}

#[test]
fn inline_set_style_first_time_no_prior_attr() {
    let (mut doc, x) = doc_with(RULES, "");
    doc.set_style(x, "--c", "rgb(1, 2, 3)");
    doc.set_style(x, "--pw", "33px");
    doc.resolve_layout(VW, VH);
    for before in [true, false] {
        let g = generated(&doc, x, before);
        assert_eq!(rgb(&doc, g), (1, 2, 3));
        assert_33(&doc, g, "set_style first");
    }
}

#[test]
fn nested_fallback_inline() {
    let (doc, x) = doc_with(
        ".x::before { content: \"\"; display: block; width: var(--missing, var(--pw, 5px)); color: var(--nope, var(--c)) }",
        "--c: rgb(1, 2, 3); --pw: 33px",
    );
    let g = generated(&doc, x, true);
    assert_eq!(rgb(&doc, g), (1, 2, 3));
    assert_33(&doc, g, "nested");
}

#[test]
fn inline_on_ancestor() {
    let mut doc = RinchDocument::new();
    doc.load_css(RULES);
    let body = doc.body();
    let p = doc.create_element("div");
    doc.set_attribute(p, "style", "--c: rgb(1, 2, 3); --pw: 33px");
    let x = doc.create_element("div");
    doc.set_attribute(x, "class", "x");
    doc.append_child(p, x);
    doc.append_child(body, p);
    doc.resolve_layout(VW, VH);
    let g = generated(&doc, x, true);
    assert_eq!(rgb(&doc, g), (1, 2, 3));
    assert_33(&doc, g, "ancestor");
}
