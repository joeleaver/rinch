//! Two `display: contents` pseudos removed in one restyle (#521, review of #1305).
//!
//! The loop in `style_resolution/resolve.rs` calls
//! `taffy_detach_contribution` once *per* removed pseudo child inside the
//! `for cid in children_to_remove` loop. Every existing fixture (including
//! the PR's own `pseudo_contents_removal_tests.rs`) removes exactly one
//! live `display: contents` pseudo per originator at a time, so a mutant
//! that hoists the detach call out of the loop (running it only for the
//! first `cid`) is invisible to them — the classic "one element" fixed-point
//! trap.
//!
//! This fixture gives one flex originator BOTH a `::before` and an
//! `::after`, both `display: contents`, then removes the rule match for
//! both in one restyle, to force two cids through `children_to_remove` at
//! once.
#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 400.0;
const VH: f32 = 200.0;

const RULES: &str = ".w::before { content: \"x\"; display: contents } \
                      .w::after { content: \"y\"; display: contents }";

fn taffy_child_count(doc: &RinchDocument, node: NodeId) -> usize {
    let taffy_id = doc.tree.get(node.0).unwrap().taffy_id.unwrap();
    doc.tree.taffy.children(taffy_id).unwrap().len()
}

#[test]
fn removing_both_before_and_after_contents_pseudos_in_one_pass_frees_both_ghosts() {
    let mut doc = RinchDocument::new();
    doc.load_css(RULES);
    let body = doc.body();
    let w = doc.create_element("div");
    doc.set_attribute(w, "class", "w");
    doc.set_attribute(w, "style", "display: flex;");
    doc.append_child(body, w);

    doc.resolve_layout(VW, VH);
    assert_eq!(
        taffy_child_count(&doc, w),
        2,
        "sanity: both ::before and ::after spliced a standing Taffy child"
    );

    // Both rules stop matching at once — two cids go through
    // `children_to_remove` in the same restyle pass.
    doc.set_attribute(w, "class", "off");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        taffy_child_count(&doc, w),
        0,
        "both spliced leaves must be freed — a detach that only runs for \
         the first removed pseudo in the loop leaves the second behind as \
         a ghost"
    );
}
