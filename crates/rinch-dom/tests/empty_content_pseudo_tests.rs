//! `content: ''` creates an (empty) generated box, as Chrome does (#773).
//!
//! `resolve_pseudo_element` (`style_resolution/pseudo.rs`) already has the
//! right check for "no pseudo at all" — `ineffective_content_property()`,
//! which Stylo answers `true` only for `content: none` / `normal`, correctly
//! answering `false` for `content: ''` (a non-empty `Content::Items` list
//! holding one empty string). A second check, `if text.is_empty() { return; }`,
//! then threw the empty-string case away anyway, so a purely decorative
//! `::before`/`::after` — the only kind that ever wants empty content — was
//! never created on desktop. Six rules in the shipped component stylesheet
//! are written this way (issue #773): `tooltip.rs`, `popover.rs`,
//! `divider.rs`, `button.rs`, `badge.rs` all draw nothing without this fix.
//!
//! Measured in Chrome 153: `.w::before { content: ''; display: block; width:
//! 20px; height: 7px; background: rgb(0,128,0) }` on `<div class="w">`
//! generates a box — `getComputedStyle(w, "::before").display` is `"block"`,
//! width 20, height 7 — with no text in it.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::{DimensionValue, DisplayValue};

const VW: f32 = 400.0;
const VH: f32 = 200.0;

/// All pseudo-element nodes under the document, regardless of owner.
fn pseudo_count(doc: &RinchDocument) -> usize {
    doc.tree
        .nodes
        .iter()
        .filter(|(_, n)| n.is_pseudo_element)
        .count()
}

fn generated(doc: &RinchDocument, owner: NodeId) -> Option<usize> {
    doc.tree
        .get(owner.0)
        .unwrap()
        .children
        .iter()
        .copied()
        .find(|&c| doc.tree.get(c).unwrap().is_pseudo_element)
}

/// `body > div.w`, with `css` loaded ahead of it, laid out once.
fn doc_with(css: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(css);
    let body = doc.body();
    let w = doc.create_element("div");
    doc.set_attribute(w, "class", "w");
    doc.append_child(body, w);
    doc.resolve_layout(VW, VH);
    (doc, w)
}

/// The positive control, inverted from the record `css_hook_760_tests` kept
/// of the old behaviour: `content: ''` now creates a pseudo-element, just
/// like `content: 'x'` always did.
#[test]
fn content_empty_string_creates_a_pseudo_element() {
    let (doc, w) = doc_with(".w::after { content: ''; display: block; height: 2px; }");
    assert_eq!(
        pseudo_count(&doc),
        1,
        "an empty `content: ''` now creates a box"
    );
    let b = generated(&doc, w).expect("the ::after exists");
    assert_eq!(
        doc.tree.get(b).unwrap().computed_style.display,
        DisplayValue::Block
    );
}

/// `content: none` and `content: normal` still create nothing — the two
/// spellings css-content-3 itself calls ineffective, and the ones
/// `ineffective_content_property()` already rejected before this fix.
#[test]
fn content_none_and_normal_still_create_no_pseudo_element() {
    for content in ["none", "normal"] {
        let css = format!(".w::after {{ content: {content}; display: block; height: 2px; }}");
        let (doc, _w) = doc_with(&css);
        assert_eq!(
            pseudo_count(&doc),
            0,
            "`content: {content}` must still create no pseudo-element"
        );
    }
}

/// A box with `content: ''` takes its size, background and border from its
/// own style, the way a non-empty one always has — this is the whole point
/// of a decorative pseudo-element: it is a shape, not text.
#[test]
fn an_empty_pseudo_element_is_laid_out_and_painted_from_its_own_style() {
    let (doc, w) = doc_with(
        ".w::before { content: ''; display: block; width: 20px; height: 7px; \
         background-color: rgb(0, 128, 0); }",
    );
    let b = generated(&doc, w).expect("the ::before exists");
    let node = doc.tree.get(b).unwrap();
    assert!(
        matches!(node.computed_style.width, DimensionValue::Length(x) if x == 20.0),
        "own width, got {:?}",
        node.computed_style.width
    );
    assert_eq!(node.layout.width, 20.0, "laid out at its own width");
    assert_eq!(node.layout.height, 7.0, "laid out at its own height");
    let bg = node
        .computed_style
        .background_color()
        .expect("background resolves");
    let rgba = bg.to_rgba8();
    assert_eq!(
        (rgba.r, rgba.g, rgba.b),
        (0, 128, 0),
        "own background color"
    );
}

/// An empty pseudo-element gets no text child — the string is empty, so there
/// is nothing to append, only the wrapper span itself.
#[test]
fn an_empty_pseudo_element_has_no_text_child() {
    let (doc, w) = doc_with(".w::before { content: ''; display: block; height: 2px; }");
    let b = generated(&doc, w).expect("the ::before exists");
    assert!(
        doc.tree.get(b).unwrap().children.is_empty(),
        "an empty `content: ''` pseudo-element has no children"
    );
}

/// `::before` AND `::after` both work for the empty case, same as the
/// non-empty case already covered by `pseudo_element_style_tests.rs`.
#[test]
fn an_empty_before_and_after_both_create_a_pseudo_element() {
    let (doc, w) = doc_with(
        ".w::before { content: ''; display: block; height: 1px; } \
         .w::after { content: ''; display: block; height: 2px; }",
    );
    assert_eq!(pseudo_count(&doc), 2);
    let kids = &doc.tree.get(w.0).unwrap().children;
    assert_eq!(kids.len(), 2, "one before, one after");
    assert!(doc.tree.get(kids[0]).unwrap().is_pseudo_element);
    assert!(doc.tree.get(kids[1]).unwrap().is_pseudo_element);
}
