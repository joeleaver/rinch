//! #2748: lists of node ids kept across layouts must survive their node being
//! freed and its slot handed to a new node before the next layout.
//!
//! Each fixture frees a node a list names, creates nodes until one takes the
//! freed slot (the positive control: the stale id's slot is live again under a
//! later generation), and lays out. Before the fix the list was read through
//! `NodeSlab::get_live`, whose debug assert panicked ("stale node id … (issue
//! #723)") inside the very pass meant to drop the dead entry; a release build
//! read the slot's new node instead.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::node_slab::{generation_of, slot_of};

const CLAMP: &str = "display: -webkit-box; -webkit-box-orient: vertical; \
    -webkit-line-clamp: 2; overflow: hidden; width: 120px; \
    font: 16px/20px sans-serif";
const TEXT: &str = "one two three four five six seven eight nine ten eleven twelve";

fn by_id(d: &RinchDocument, name: &str) -> u64 {
    rinch_dom::testing::query_selector(&d.tree, &format!("#{name}"))[0]
}

/// Create `<span>`s under `parent` until one takes `stale`'s slot, and return
/// it. Panics (failing the fixture) if none does: then the fixture would not
/// test what it says.
fn take_slot(d: &mut RinchDocument, parent: NodeId, stale: u64) -> NodeId {
    assert!(d.tree.get(stale).is_none(), "the id must be freed first");
    for _ in 0..64 {
        let n = d.create_element("span");
        d.append_child(parent, n);
        if slot_of(n.0) == slot_of(stale) {
            assert!(generation_of(n.0) > generation_of(stale));
            return n;
        }
    }
    panic!("no new node took slot {} of the freed id", slot_of(stale));
}

/// The issue's exact shape: a clamped element is removed, a new node takes its
/// slot, then layout runs (`resolve_block_line_clamps` pruning
/// `line_clamp_roots`).
#[test]
fn a_removed_clamp_whose_slot_is_reused_before_layout_is_dropped() {
    let mut d = RinchDocument::new();
    let body = d.body();
    d.set_inner_html(
        body,
        &format!(r#"<div id="col"><div id="pop" style="{CLAMP}">{TEXT}</div></div>"#),
    );
    d.resolve_layout(400.0, 600.0);
    let col = NodeId(by_id(&d, "col"));
    let pop = by_id(&d, "pop");
    let clamped_h = d.tree.get(pop).unwrap().layout.height;
    assert!(
        (clamped_h - 40.0).abs() < 0.5,
        "positive control: the clamp holds two 20px lines, got {clamped_h}"
    );

    d.discard_node(NodeId(pop));
    let fresh = take_slot(&mut d, col, pop);
    d.set_attribute(fresh, "style", "display: block; height: 7px");
    d.resolve_layout(401.0, 600.0);

    let n = d.tree.get(fresh.0).expect("the new node is live");
    assert_eq!(n.layout.height, 7.0, "the new node lays out as its own");
}

/// The nodes a clamp gave a `clamp_lines`/`clamp_height` (`line_clamped`): an
/// inner block holding the clamp's lines is removed while the clamp stays, and
/// its slot is reused before layout.
#[test]
fn a_removed_clamped_line_box_whose_slot_is_reused_before_layout_is_dropped() {
    let mut d = RinchDocument::new();
    let body = d.body();
    d.set_inner_html(
        body,
        &format!(
            r#"<div id="pop" style="{CLAMP}"><p id="lines" style="margin: 0">{TEXT}</p></div>"#
        ),
    );
    d.resolve_layout(400.0, 600.0);
    let pop = NodeId(by_id(&d, "pop"));
    let lines = by_id(&d, "lines");
    let clamped_h = d.tree.get(pop.0).unwrap().layout.height;
    assert!(
        (clamped_h - 40.0).abs() < 0.5,
        "positive control: the block child's lines are clamped to two, got {clamped_h}"
    );

    d.discard_node(NodeId(lines));
    let fresh = take_slot(&mut d, pop, lines);
    d.set_attribute(fresh, "style", "display: block; height: 7px");
    d.resolve_layout(401.0, 600.0);

    let n = d.tree.get(fresh.0).expect("the new node is live");
    assert_eq!(n.layout.height, 7.0, "the new node lays out as its own");
    assert_eq!(d.tree.get(pop.0).unwrap().layout.height, 7.0);
}
