//! A generated `::before` / `::after` keeps the style its own pseudo cascade
//! gave it (#1004).
//!
//! `resolve_pseudo_element` cascades the pseudo-element and writes the result
//! onto a synthetic `<span>` it inserts into the originating element. The
//! element walk then reached that span as an ordinary child and re-cascaded it
//! as a **plain `<span>`** — no `::before` rule matches a plain span — and
//! `apply_stylo_styles_to_taffy` replaced its `computed_style` with that. Only
//! `content` survived, because it had been read before the overwrite.
//!
//! Chrome 153, `.w::before { content: "x"; color: rgb(255,0,0); width: 33px;
//! display: inline-block }` on `<div class="w">`: `getComputedStyle(div,
//! "::before")` is `rgb(255, 0, 0)`, `inline-block`, `33px`. A `span { … }`
//! rule does not match a `::before` in a browser, so it cannot restyle one.
//!
//! Every value below is off the initial value (black / inline / auto), so a
//! plain-span cascade cannot satisfy any assertion by coincidence.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::{DimensionValue, DisplayValue};

const VW: f32 = 400.0;
const VH: f32 = 200.0;

const BEFORE: &str = ".w::before { content: \"x\"; color: rgb(255, 0, 0); width: 33px; \
                      display: inline-block; font-size: 16px; line-height: 20px }";

fn hex(doc: &RinchDocument, n: usize) -> String {
    let c = doc.tree.get(n).unwrap().computed_style.color;
    let rgba = c.expect("color should resolve").to_rgba8();
    format!("#{:02x}{:02x}{:02x}", rgba.r, rgba.g, rgba.b)
}

fn generated(doc: &RinchDocument, owner: NodeId) -> usize {
    doc.tree
        .get(owner.0)
        .unwrap()
        .children
        .iter()
        .copied()
        .find(|&c| doc.tree.get(c).unwrap().is_pseudo_element)
        .expect("the element generates a ::before")
}

fn assert_before_style(doc: &RinchDocument, owner: NodeId) {
    let b = generated(doc, owner);
    let s = &doc.tree.get(b).unwrap().computed_style;
    assert_eq!(hex(doc, b), "#ff0000", "the ::before's own color");
    assert_eq!(
        s.display,
        DisplayValue::InlineBlock,
        "the ::before's own display"
    );
    assert!(
        matches!(s.width, DimensionValue::Length(w) if w == 33.0),
        "the ::before's own width, got {:?}",
        s.width
    );
}

/// `body > div.w`, laid out once, with `extra_css` after the `::before` rule.
fn doc_with(extra_css: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(&format!("{BEFORE}\n{extra_css}"));
    let body = doc.body();
    let w = doc.create_element("div");
    doc.set_attribute(w, "class", "w");
    doc.append_child(body, w);
    doc.resolve_layout(VW, VH);
    (doc, w)
}

#[test]
fn a_before_keeps_its_own_color_display_and_width() {
    let (doc, w) = doc_with("");
    assert_before_style(&doc, w);
}

/// The generated box is laid out from that style: 33px wide.
#[test]
fn a_before_is_laid_out_at_its_own_width() {
    let (doc, w) = doc_with("");
    let b = generated(&doc, w);
    assert_eq!(doc.tree.get(b).unwrap().layout.width, 33.0);
}

/// The originator re-cascades (an inherited property changes above it), so
/// its `::before` is regenerated: the regenerated box keeps its own style too.
#[test]
fn a_before_regenerated_by_an_inherited_change_keeps_its_style() {
    let (mut doc, w) = doc_with("");
    let body = doc.body();
    doc.set_attribute(body, "style", "color: rgb(0, 0, 255)");
    doc.resolve_layout(VW + 1.0, VH);
    assert_eq!(
        hex(&doc, w.0),
        "#0000ff",
        "positive control: the change landed"
    );
    assert_before_style(&doc, w);
}

/// A class change the invalidator answers with a hint on the **descendants**
/// only (`.w.on span`): the originator is not re-cascaded, so its `::before`
/// is not regenerated, and the walk reaches the generated box with a hint.
/// A `span` rule must not reach a `::before` — it did, because the box was
/// re-cascaded as a plain `<span>`.
#[test]
fn a_span_rule_hinted_by_a_class_change_does_not_restyle_a_before() {
    let (mut doc, w) = doc_with(".w.on span { color: rgb(0, 0, 255) }");
    // Positive control: a real span under `.w` does take the rule.
    let real = doc.create_element("span");
    doc.append_child(w, real);
    doc.resolve_layout(VW + 1.0, VH);
    doc.set_attribute(w, "class", "w on");
    doc.resolve_layout(VW + 2.0, VH);
    assert_eq!(
        hex(&doc, real.0),
        "#0000ff",
        "positive control: the rule applies"
    );
    assert_before_style(&doc, w);
}

/// The same for `::after`.
#[test]
fn an_after_keeps_its_own_color() {
    let mut doc = RinchDocument::new();
    doc.load_css(".w::after { content: \"y\"; color: rgb(0, 128, 0); display: block }");
    let body = doc.body();
    let w = doc.create_element("div");
    doc.set_attribute(w, "class", "w");
    doc.append_child(body, w);
    doc.resolve_layout(VW, VH);
    let a = generated(&doc, w);
    assert_eq!(hex(&doc, a), "#008000");
    assert_eq!(
        doc.tree.get(a).unwrap().computed_style.display,
        DisplayValue::Block
    );
}
