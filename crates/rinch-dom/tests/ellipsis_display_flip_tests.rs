//! A `display` flip between a flex or grid container and a block container
//! re-decides the container's `text-overflow: ellipsis` (#1046).
//!
//! A container holding only text is an IFC root whether it is `display:
//! block` or `display: grid`, and `build_ifc_layouts` draws the "…" only for
//! one that is not a flex or grid container: that text is an anonymous item,
//! which does not clip, and Chrome 153 draws it clipped with no "…" (#904).
//! The flip changes neither the width nor the text, so unless the cascade
//! treats it as a text-layout input the root keeps the layout it was shaped
//! with, and with it the old decision — in both directions.
//!
//! Each shape is laid out in its first class, flipped, and laid out again at
//! another viewport; the root's line is then compared with a document built
//! fresh in the final class, which is the oracle, and with the answer the
//! final display asks for. The flex shapes are controls: a flex container's
//! text is a Taffy leaf, so the flip is structural and was never stale.
//!
//! Only font-independent facts are asserted: the string is far wider than
//! 60px in any face and the line box is declared.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const LONG: &str = "a line much too long for sixty pixels in any font at all";
const CSS: &str = "
    body { margin: 0; font-family: sans-serif; font-size: 16px; line-height: 20px; }
    .clip { width: 60px; overflow: hidden; white-space: nowrap;
            text-overflow: ellipsis; }
    .grid { display: grid; }
    .igrid { display: inline-grid; }
    .flex { display: flex; }
    .iflex { display: inline-flex; }
    .ib { display: inline-block; }
";

fn build(class: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let wrap = doc.create_element("div");
    doc.append_child(body, wrap);
    let x = doc.create_element("div");
    doc.set_attribute(x, "class", class);
    doc.append_child(wrap, x);
    let t = doc.create_text(LONG);
    doc.append_child(x, t);
    doc.resolve_layout(400.0, 300.0);
    (doc, x)
}

/// Whether the container's own inline layout ends in "…". `None` when it is
/// not an IFC root at all (a flex container's text is a Taffy leaf).
fn ellipsis(doc: &RinchDocument, x: NodeId) -> Option<bool> {
    doc.tree
        .get(x.0)
        .unwrap()
        .text_layout
        .as_ref()
        .map(|l| l.text_content.ends_with('\u{2026}'))
}

#[test]
fn a_display_flip_re_decides_the_ellipsis() {
    // (from, to, whether `to` draws the "…")
    let shapes: &[(&str, &str, bool)] = &[
        ("clip grid", "clip", true),
        ("clip grid", "clip ib", true),
        ("clip igrid", "clip", true),
        ("clip", "clip grid", false),
        ("clip", "clip igrid", false),
        ("clip ib", "clip grid", false),
        // Controls: structural flips, fine before #1046.
        ("clip flex", "clip", true),
        ("clip", "clip flex", false),
        ("clip iflex", "clip ib", true),
    ];
    let mut failures = Vec::new();
    for &(from, to, want) in shapes {
        let (mut doc, x) = build(from);
        doc.set_attribute(x, "class", to);
        doc.resolve_layout(401.0, 300.0);
        let got = ellipsis(&doc, x).unwrap_or(false);
        let (fresh, fx) = build(to);
        let oracle = ellipsis(&fresh, fx).unwrap_or(false);
        if oracle != want {
            failures.push(format!(
                "{from} -> {to}: the fresh oracle says {oracle}, want {want}"
            ));
        }
        if got != oracle {
            failures.push(format!(
                "{from} -> {to}: ellipsis {got} after the flip, {oracle} built fresh"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
