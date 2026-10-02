//! `set_style` against a declaration declared **after** the written property
//! that covers it (#470).
//!
//! rinch keeps an element's inline `style` attribute as a string of
//! declarations and rewrites a property that is already there where it stands
//! (#265, #454). CSSOM keeps a list of **longhands** instead: a shorthand is
//! expanded when it is parsed, so `style.left = "25px"` replaces the one `left`
//! longhand whatever `inset` the author wrote after it, and `left` computes to
//! `25px`. Rewritten in place in the string, `left: 25px` still sat before the
//! `inset: 0` that covers it, and `inset` won the cascade: the attribute said
//! `left: 25px` and the element was laid out at `left: 0`.
//!
//! The rule now: a written property that a **later** declaration overlaps — a
//! shared longhand, or (the CSSOM "logical property group" step) a longhand of
//! the same group with the other mapping logic, `left` against
//! `inset-inline-start` — moves to the end of the attribute, where it wins as
//! it does in a browser. Every other rewrite stays in place
//! (`dom_tests::set_style_replaces_a_declaration_in_place` and its
//! neighbours).
//!
//! Every expected position is Chrome 153's, measured by the twin
//! `crates/rinch-web/tests/set_style_shorthand_overlap.rs` on the same
//! markup through the same `set_style` call. `LayoutResult` is relative to the
//! parent's border box, whose border is 5px left and 7px top, so `x = 5 + left`
//! and `y = 7 + top`.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const PARENT: &str = "position: relative; width: 300px; height: 200px; \
                      border-left: 5px solid black; border-top: 7px solid black";

/// body > parent > child, laid out once; returns the document and the child.
fn positioned(child_style: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let parent = doc.create_element("div");
    doc.set_attribute(parent, "style", PARENT);
    doc.append_child(body, parent);
    let child = doc.create_element("div");
    doc.set_attribute(child, "style", child_style);
    doc.append_child(parent, child);
    doc.resolve_layout(800.0, 600.0);
    (doc, child)
}

fn xy(doc: &RinchDocument, node: NodeId) -> (f32, f32) {
    let l = doc.tree.get(node.0).unwrap().layout;
    (l.x, l.y)
}

/// The issue's own shape: a longhand declared before the shorthand that covers
/// it, then written again. Chrome: x = 5 + 25.
#[test]
fn a_longhand_declared_before_a_covering_shorthand_wins_when_written() {
    let (mut doc, child) =
        positioned("position: absolute; left: 5px; inset: 0; width: 10px; height: 10px");
    assert_eq!(xy(&doc, child), (5.0, 7.0), "baseline: `inset: 0` wins the parse");

    doc.set_style(child, "left", "25px");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        xy(&doc, child),
        (30.0, 7.0),
        "Chrome 153 places the child at left: 25px; it laid out at left: 0 (#470)"
    );
    assert_eq!(
        doc.get_attribute(child, "style").unwrap(),
        "position: absolute; inset: 0; width: 10px; height: 10px; left: 25px",
        "the written longhand moves past the shorthand that covered it"
    );
}

/// The other direction: a shorthand written over a longhand declared after it.
/// CSSOM expands `inset: 3px` into all four longhands, so the later `left: 5px`
/// is replaced too. Chrome: x = 5 + 3, y = 7 + 3.
#[test]
fn a_shorthand_written_over_a_later_longhand_covers_it() {
    let (mut doc, child) =
        positioned("position: absolute; inset: 0; left: 15px; width: 10px; height: 10px");
    assert_eq!(xy(&doc, child), (20.0, 7.0), "baseline: the later `left` wins");

    doc.set_style(child, "inset", "3px");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        xy(&doc, child),
        (8.0, 10.0),
        "Chrome 153 gives every inset 3px; the later `left: 15px` used to win"
    );
}

/// CSSOM's logical-group step: `left` and `inset-inline-start` are different
/// longhands, so they share none, but they are one physical side in a
/// left-to-right horizontal writing mode, and Stylo (like Chrome) appends a
/// write of one past a later declaration of the other. Chrome: x = 5 + 25.
#[test]
fn a_physical_longhand_written_before_its_logical_twin_wins() {
    let (mut doc, child) = positioned(
        "position: absolute; left: 5px; inset-inline-start: 0; top: 0; \
         width: 10px; height: 10px",
    );
    assert_eq!(xy(&doc, child), (5.0, 7.0), "baseline: the later logical twin wins");

    doc.set_style(child, "left", "25px");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        xy(&doc, child),
        (30.0, 7.0),
        "Chrome 153 places the child at left: 25px"
    );

    // The write above takes the inset fast path, which reads `left` straight
    // out of the parsed block and so lands at 25px even with the logical twin
    // after it. The cascade does not: the next write that restyles the
    // element must agree with it, not snap it back to `inset-inline-start: 0`.
    doc.set_style(child, "color", "red");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        xy(&doc, child),
        (30.0, 7.0),
        "a full cascade of the same attribute must keep left: 25px"
    );
}

/// A later declaration of the same logical group with the **same** mapping
/// logic is a different side, not a rival: `top` after `left` covers nothing
/// of it, so `left` is rewritten where it stands — the order a reader of the
/// attribute sees is unchanged. (Kills "move on any later declaration of the
/// group".)
#[test]
fn a_later_declaration_that_covers_nothing_leaves_the_write_in_place() {
    let (mut doc, child) =
        positioned("position: absolute; left: 5px; top: 3px; width: 10px; height: 10px");

    doc.set_style(child, "left", "25px");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(xy(&doc, child), (30.0, 10.0));
    assert_eq!(
        doc.get_attribute(child, "style").unwrap(),
        "position: absolute; left: 25px; top: 3px; width: 10px; height: 10px",
    );
}

/// Off the inset fast path: a normal-flow box whose `margin-left` is declared
/// before a `margin` shorthand. Chrome: x = 5 + 25.
#[test]
fn a_margin_longhand_declared_before_the_margin_shorthand_wins_when_written() {
    let (mut doc, child) =
        positioned("margin-left: 5px; margin: 0; width: 10px; height: 10px");
    assert_eq!(xy(&doc, child), (5.0, 7.0), "baseline: `margin: 0` wins the parse");

    doc.set_style(child, "margin-left", "25px");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        xy(&doc, child),
        (30.0, 7.0),
        "Chrome 153 places the child 25px in from its parent's border"
    );
}

/// A batch: each property is judged against the declarations after its own
/// slot, so `left` moves past `inset` and `width`, which nothing later covers,
/// is rewritten in place.
#[test]
fn a_batch_moves_only_the_overlapped_write() {
    let (mut doc, child) =
        positioned("position: absolute; width: 10px; left: 5px; inset: 0; height: 10px");

    doc.set_styles(child, &[("left", "25px"), ("width", "20px")]);
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(xy(&doc, child), (30.0, 7.0));
    assert_eq!(
        doc.get_attribute(child, "style").unwrap(),
        "position: absolute; width: 20px; inset: 0; height: 10px; left: 25px",
    );
}
