//! #683 — a camelCase SVG type selector must match on desktop.
//!
//! `TElement::is_html_element_in_html_document` decides whether a type
//! selector compares its local name ASCII case-insensitively (HTML) or
//! exactly (everything else) — `selectors`' `matching.rs::select_name` /
//! `to_unconditional_case_sensitivity`. rinch hardcoded it to `true` for
//! every element, including one inside an `<svg>`, so `linearGradient { … }`
//! was compared against the selector's *lowercased* spelling
//! (`lineargradient`) and `RinchNode::has_local_name` — which compares
//! exactly against the tag as authored — could never match it.
//!
//! The fix reuses `attr_name::is_svg_content_tag`, the predicate the sibling
//! #688 fix already built for exactly this question (which tags are SVG
//! content, camelCase and all), applied the other direction: an element whose
//! tag is SVG content answers `false` here, so its type selector keeps the
//! author's exact spelling.
//!
//! Every fixture carries `* { margin-bottom: 17px }` as a positive control —
//! same shape as the issue's own measured table — so a failure that is really
//! "the stylesheet never reached this element" is distinguishable from "the
//! type selector did not match".

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

fn margin_top(doc: &RinchDocument, id: usize) -> f32 {
    doc.tree.get(id).unwrap().computed_style.margin_top.to_px()
}

fn margin_bottom(doc: &RinchDocument, id: usize) -> f32 {
    doc.tree
        .get(id)
        .unwrap()
        .computed_style
        .margin_bottom
        .to_px()
}

/// The issue's own case: `linearGradient { … }` must reach a `<linearGradient>`
/// inside an `<svg>`.
#[test]
fn camelcase_svg_type_selector_matches_its_element() {
    let mut doc = RinchDocument::new();
    doc.load_css("linearGradient { margin-top: 11px; } * { margin-bottom: 17px; }");
    let body = doc.body();

    let svg = doc.create_element("svg");
    doc.append_child(body, svg);
    let grad = doc.create_element("linearGradient");
    doc.append_child(svg, grad);

    doc.resolve_layout(VW, VH);

    assert_eq!(
        margin_bottom(&doc, grad.0),
        17.0,
        "positive control: the universal rule must land, or nothing below means anything"
    );
    assert_eq!(
        margin_top(&doc, grad.0),
        11.0,
        "#683: a camelCase SVG type selector must match its element exactly"
    );
}

/// The behaviour that must NOT regress: an ordinary HTML type selector stays
/// ASCII case-insensitive, `SPAN { … }` matching `<span>`.
#[test]
fn uppercase_html_type_selector_still_matches_case_insensitively() {
    let mut doc = RinchDocument::new();
    doc.load_css("SPAN { margin-top: 11px; } * { margin-bottom: 17px; }");
    let body = doc.body();

    let span = doc.create_element("span");
    doc.append_child(body, span);

    doc.resolve_layout(VW, VH);

    assert_eq!(margin_bottom(&doc, span.0), 17.0, "positive control");
    assert_eq!(
        margin_top(&doc, span.0),
        11.0,
        "HTML content is case-insensitive for type selectors — this must stay true"
    );
}

/// A lowercase-spelled rule must not accidentally match the camelCase element
/// by being folded at the SELECTOR side (it never was — Stylo lowercases the
/// selector itself when `is_html_element_in_html_document` says to, which we
/// now refuse to do for SVG content — this just confirms the predicate did
/// not get the comparison backwards).
#[test]
fn lowercase_rule_does_not_match_the_camelcase_svg_element() {
    let mut doc = RinchDocument::new();
    doc.load_css("lineargradient { margin-top: 11px; } * { margin-bottom: 17px; }");
    let body = doc.body();

    let svg = doc.create_element("svg");
    doc.append_child(body, svg);
    let grad = doc.create_element("linearGradient");
    doc.append_child(svg, grad);

    doc.resolve_layout(VW, VH);

    assert_eq!(margin_bottom(&doc, grad.0), 17.0, "positive control");
    assert_eq!(
        margin_top(&doc, grad.0),
        0.0,
        "SVG content is case-SENSITIVE for type selectors: the lowercase \
         spelling must not match the camelCase element"
    );
}

/// `foreignObject` is SVG content itself, but its HTML descendants are not —
/// an ordinary `<div>` inside one must still be matched case-insensitively.
/// This is the case `attr_name.rs`'s doc calls out as "falls out correctly
/// without a special case", and it is worth pinning for the selector question
/// too, not only the attribute-name one it was written for.
#[test]
fn html_descendants_of_a_foreign_object_stay_case_insensitive() {
    let mut doc = RinchDocument::new();
    doc.load_css("DIV { margin-top: 11px; } * { margin-bottom: 17px; }");
    let body = doc.body();

    let svg = doc.create_element("svg");
    doc.append_child(body, svg);
    let fo = doc.create_element("foreignObject");
    doc.append_child(svg, fo);
    let inner = doc.create_element("div");
    doc.append_child(fo, inner);

    doc.resolve_layout(VW, VH);

    assert_eq!(margin_bottom(&doc, inner.0), 17.0, "positive control");
    assert_eq!(
        margin_top(&doc, inner.0),
        11.0,
        "an HTML element inside foreignObject is still HTML, so an \
         uppercase-spelled type selector must still match it"
    );
}
