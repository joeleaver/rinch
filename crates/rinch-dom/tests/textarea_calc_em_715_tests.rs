//! #715: `Textarea::max_rows` writes `max-height: calc(1.2em * N + ...)`; does
//! resolve `em` against the *textarea's own* computed font-size (not the
//! container's, not the root's), matching rinch's `LineHeightValue::Normal`
//! resolution (`font_size * 1.2`) exactly?

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    doc.set_attribute(e, "style", style);
    doc.append_child(parent, e);
    e
}

fn container(doc: &mut RinchDocument, style: &str) -> NodeId {
    let body = doc.body();
    el(
        doc,
        body,
        "div",
        &format!("width: 300px; font-size: 24px; {style}"),
    )
}

fn height(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().layout.height
}

/// Container font-size (24px) must NOT leak into the calc's `1.2em` — it
/// must resolve against the textarea's OWN font-size (16px here), exactly
/// as the PR claims ("the control's own `normal` line-height").
///
/// Hand calc: 1.2*16*5 = 96; padding 2*10 = 20; border 1px top+1px bottom =
/// 2. Total = 118 (border-box; Taffy's default box_sizing is BorderBox and
/// rinch never overrides it from `box-sizing`, per
/// `form_control_intrinsic_height_tests.rs`'s own module doc).
#[test]
fn em_in_max_height_calc_resolves_against_the_textareas_own_font_size_not_the_parents() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc, "");
    let ta = el(
        &mut doc,
        c,
        "textarea",
        "display: block; font-size: 16px; padding: 10px; border: 1px solid black; \
         max-height: calc(1.2em * 5 + 2 * 10px + 2px); line-height: normal",
    );
    doc.set_attribute(ta, "rows", "20"); // force content height way past the cap
    doc.resolve_layout(800.0, 600.0);
    let h = height(&doc, ta);
    assert_eq!(
        h, 118.0,
        "got {h} — a 24px-font-based resolution would give ~178.4"
    );
}
