//! #739 — an HTML *tag* name is ASCII case-insensitive; an SVG one is not.
//!
//! The attribute-name half of the same shape is #688
//! (`attribute_name_case_tests.rs`). This is the tag half: `html_parser.rs`'s
//! `parse_element` and a direct `create_element("DIV")` both stored the tag
//! exactly as written, and `TElement::has_local_name` compares it with `==`
//! against Stylo's already-lowercased selector — so `div { … }` matched only
//! an author who happened to write lowercase markup.
//!
//! The fix folds the tag once, in `RinchDocument::create_element` — the one
//! place both writers (`create_node_from_parsed` and a direct call) funnel
//! through, and *before* `Node::element` or any `set_attribute` reads it, so
//! `default_display_for_tag`, `is_atomic_at_display_inline`,
//! `fold_attribute_name` and every other tag-keyed reader all see the folded
//! spelling. See `attr_name::fold_tag_name` for the SVG side (canonical
//! spelling kept, a mis-cased SVG name restored to it, an ordinary HTML tag
//! lowercased).

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

fn margin_bottom(doc: &RinchDocument, n: NodeId) -> f32 {
    doc.tree
        .get(n.0)
        .unwrap()
        .computed_style
        .margin_bottom
        .to_px()
}

fn tag(doc: &RinchDocument, n: NodeId) -> String {
    doc.tree.get(n.0).unwrap().tag().unwrap().to_string()
}

// ── The issue's own case ────────────────────────────────────────────────────

/// The issue's exact fixture: `<DIV>` and `<div>` via `set_inner_html`, both
/// matching `div { margin-bottom: 17px; }`.
///
/// Sampled off the fixed point (17px, not 0px): at HEAD the uppercase tag is
/// stored verbatim as `"DIV"`, `has_local_name` compares it with `==` against
/// Stylo's lowercased selector `"div"`, and the rule never matches — the
/// uppercase element's margin-bottom stays 0.
#[test]
fn uppercase_div_matches_a_lowercase_type_selector() {
    let mut doc = RinchDocument::new();
    doc.load_css("div { margin-bottom: 17px; }");
    let body = doc.body();
    doc.set_inner_html(body, "<DIV></DIV><div></div>");

    doc.resolve_layout(VW, VH);

    let kids: Vec<_> = doc.tree.get(body.0).unwrap().children.clone();
    let upper = NodeId(kids[0]);
    let lower = NodeId(kids[1]);

    assert_eq!(
        tag(&doc, upper),
        "div",
        "#739: the stored tag itself is folded to lowercase"
    );
    assert_eq!(
        margin_bottom(&doc, lower),
        17.0,
        "positive control: the lowercase twin always matched"
    );
    assert_eq!(
        margin_bottom(&doc, upper),
        17.0,
        "#739: `<DIV>` matches the div rule exactly as `<div>` does"
    );
}

/// The same fixture through a direct `create_element` call, not the parser —
/// the other call site the issue names.
#[test]
fn create_element_with_an_uppercase_tag_folds_too() {
    let mut doc = RinchDocument::new();
    doc.load_css("div { margin-bottom: 17px; }");
    let body = doc.body();

    let upper = doc.create_element("DIV");
    doc.append_child(body, upper);
    let lower = doc.create_element("div");
    doc.append_child(body, lower);

    doc.resolve_layout(VW, VH);

    assert_eq!(tag(&doc, upper), "div");
    assert_eq!(margin_bottom(&doc, lower), 17.0, "positive control");
    assert_eq!(
        margin_bottom(&doc, upper),
        17.0,
        "#739: create_element(\"DIV\") is create_element(\"div\")"
    );
}

/// Mixed case, as a component author copy-pasting markup is more likely to
/// write than all-caps: `<Button>` must match `button { … }`.
#[test]
fn mixed_case_tag_matches_too() {
    let mut doc = RinchDocument::new();
    doc.load_css("button { margin-bottom: 17px; }");
    let body = doc.body();
    doc.set_inner_html(body, "<Button></Button>");

    doc.resolve_layout(VW, VH);

    let n = NodeId(doc.tree.get(body.0).unwrap().children[0]);
    assert_eq!(tag(&doc, n), "button");
    assert_eq!(margin_bottom(&doc, n), 17.0);
}

// ── Consumers beyond style matching: focusability, form-control, atomic inline ──

/// A `<BUTTON>` is focusable by tag (CLAUDE.md's "Keyboard Focus": focusable
/// by tag includes `<button>`). `node_is_focusable_by_tag` — the predicate
/// behind Tab order and click-to-focus — reads the stored tag with `==`
/// against `"button"`, so an unfolded `"BUTTON"` would read as an unknown,
/// non-focusable tag.
#[test]
fn uppercase_button_is_focusable() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let n = doc.create_element("BUTTON");
    doc.append_child(body, n);

    assert_eq!(tag(&doc, n), "button");
    assert!(
        rinch_dom::node::is_atomic_at_display_inline(doc.tree.get(n.0).unwrap().tag().unwrap()),
        "#739: an uppercase <BUTTON> is recognised as the atomic-inline/form \
         control tag it is, which is what the focus and form-control readers \
         key on"
    );
}

/// `<INPUT>` must be recognised as a form control — `is_atomic_at_display_inline`
/// is the same predicate `apply_stylo_styles_to_taffy` and the focus/form
/// machinery read, and it matches on exact lowercase tag names.
#[test]
fn uppercase_input_is_a_form_control_tag() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let n = doc.create_element("INPUT");
    doc.append_child(body, n);

    assert_eq!(tag(&doc, n), "input");
    assert!(rinch_dom::node::is_atomic_at_display_inline(
        doc.tree.get(n.0).unwrap().tag().unwrap()
    ));
}

/// Default display mode also keys on the exact lowercase tag
/// (`default_display_for_tag`, private to `node.rs`, read straight from
/// `Node::display_mode` before any cascade has run) — an uppercase `<SPAN>`
/// used to fall through to the `_ => Block` arm instead of `Inline`.
/// `<SPAN>` must behave like `<span>`, not like a `<div>`.
///
/// Read `display_mode` directly rather than the Taffy style: appending a
/// node into a document with a `<body>` runs a style/Taffy re-sync of its
/// own (independent of this fixture), which would make a Taffy-style
/// assertion here pass or fail for reasons unrelated to tag folding.
#[test]
fn uppercase_span_gets_the_inline_default_not_block() {
    let mut doc = RinchDocument::new();
    let upper = doc.create_element("SPAN");
    let lower = doc.create_element("span");
    let block = doc.create_element("DIV");

    assert_eq!(tag(&doc, block), "div");
    assert_eq!(
        doc.tree.get(block.0).unwrap().display_mode,
        rinch_dom::node::DisplayMode::Block,
        "positive control: a block tag still gets the block default"
    );
    assert_eq!(
        doc.tree.get(lower.0).unwrap().display_mode,
        rinch_dom::node::DisplayMode::Inline,
        "positive control: a lowercase span always got the inline default"
    );
    assert_eq!(
        doc.tree.get(upper.0).unwrap().display_mode,
        rinch_dom::node::DisplayMode::Inline,
        "#739: an uppercase <SPAN> gets the same inline default as <span>, not \
         the block default <DIV> gets"
    );
}

// ── The counter-case: SVG content keeps its canonical spelling ─────────────

/// A correctly-cased SVG tag (`linearGradient`) is completely unaffected —
/// the positive control that #739's fold does not also lowercase SVG, which
/// would be the obvious wrong fix (and the mirror image of #688's
/// `svg_camelcase_attributes_are_not_folded`).
#[test]
fn svg_tags_keep_their_canonical_spelling() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let svg = doc.create_element("svg");
    doc.append_child(body, svg);
    let grad = doc.create_element("linearGradient");
    doc.append_child(svg, grad);

    assert_eq!(tag(&doc, svg), "svg");
    assert_eq!(
        tag(&doc, grad),
        "linearGradient",
        "#739: SVG's camelCase tag spelling is untouched by the HTML fold"
    );
}

/// A mis-cased SVG tag is restored to its canonical spelling rather than
/// lowercased to something `paint/svg.rs`'s child dispatch would not
/// recognise — the "SVG's name adjustment" step the issue calls out by name.
#[test]
fn a_miscased_svg_tag_is_restored_to_canonical() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let svg = doc.create_element("svg");
    doc.append_child(body, svg);
    let grad = doc.create_element("LINEARGRADIENT");
    doc.append_child(svg, grad);

    assert_eq!(
        tag(&doc, grad),
        "linearGradient",
        "#739: a mis-cased SVG tag is adjusted to its canonical spelling, not \
         lowercased to \"lineargradient\""
    );
}
