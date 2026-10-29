//! #723: the public paint geometry helpers take a caller's id, so they look it
//! up **checked**. A stale id (the slot's live node is a later generation) must
//! not panic in debug builds (the internal `get_live` assert) and must not
//! answer the live node's geometry in release builds. Found in review of #2017.
#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint;

/// A 30×20 box at (17, 40), and a stale id for its slot.
///
/// Twice: with the stale id built by hand (the box at generation 0, the id one
/// generation on), and with a real reuse (#723 PR 2) — a node discarded, the
/// box minted into its freed slot at generation 1, and the discarded node's own
/// id kept, which is exactly what an app's handle would hold.
fn docs() -> [(RinchDocument, NodeId, NodeId); 2] {
    [doc_with(false), doc_with(true)]
}

fn doc_with(reuse: bool) -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let spacer = doc.create_element("div");
    doc.set_attribute(spacer, "style", "height: 40px");
    doc.append_child(body, spacer);
    let old = reuse.then(|| {
        let old = doc.create_element("div");
        doc.set_attribute(old, "style", "width: 50px; height: 50px");
        doc.append_child(body, old);
        doc.resolve_layout(401.0, 300.0);
        doc.discard_node(old);
        old
    });
    let p = doc.create_element("div");
    doc.set_attribute(p, "style", "width: 30px; height: 20px; margin-left: 17px");
    doc.append_child(body, p);
    doc.resolve_layout(400.0, 300.0);
    let stale = match old {
        Some(old) => {
            assert_eq!(p.index(), old.index(), "precondition: the slot is reused");
            assert_eq!(p.generation(), old.generation() + 1);
            old
        }
        None => NodeId::new(p.index() as u32, 1),
    };
    (doc, p, stale)
}

#[test]
fn compute_absolute_position_of_a_stale_id_is_the_origin() {
    for (doc, p, stale) in docs() {
        let (lx, ly, _) = paint::compute_absolute_position_and_transform(&doc.tree, p.0, 1.0);
        assert!(lx > 10.0 && ly > 30.0, "positive control: ({lx}, {ly})");
        let (x, y, t) = paint::compute_absolute_position_and_transform(&doc.tree, stale.0, 1.0);
        assert_eq!((x, y), (0.0, 0.0));
        assert_eq!(t.as_coeffs(), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    }
}

#[test]
fn painted_border_box_of_a_stale_id_is_empty() {
    for (doc, p, stale) in docs() {
        let live = paint::painted_border_box(&doc.tree, p.0, 1.0);
        assert!(live.x0 > 10.0, "positive control: {live:?}");
        let r = paint::painted_border_box(&doc.tree, stale.0, 1.0);
        assert_eq!((r.x0, r.y0, r.width(), r.height()), (0.0, 0.0, 0.0, 0.0));
    }
}

#[test]
fn point_from_painted_box_of_a_stale_id_does_not_answer_the_live_nodes_origin() {
    for (doc, p, stale) in docs() {
        let live = paint::point_from_painted_box(&doc.tree, p.0, 1.0, 0.0, 0.0);
        assert_eq!(live, (17.0, 40.0), "positive control");
        let s = paint::point_from_painted_box(&doc.tree, stale.0, 1.0, 0.0, 0.0);
        assert_eq!(s, (0.0, 0.0), "a stale id answered the slot's live node");
    }
}

#[test]
fn rect_from_painted_box_of_a_stale_id_is_not_placed_at_the_live_node() {
    for (doc, p, stale) in docs() {
        let live = paint::rect_from_painted_box(&doc.tree, p.0, 1.0, (1.0, 2.0, 3.0, 4.0));
        assert_eq!(live, (18.0, 42.0, 3.0, 4.0), "positive control");
        let s = paint::rect_from_painted_box(&doc.tree, stale.0, 1.0, (1.0, 2.0, 3.0, 4.0));
        assert_eq!(s, (1.0, 2.0, 3.0, 4.0));
    }
}

#[test]
fn point_in_painted_box_of_a_stale_id_is_none() {
    for (doc, p, stale) in docs() {
        let live = paint::point_in_painted_box(&doc.tree, p.0, 1.0, 20.0, 45.0);
        assert_eq!(live, Some((3.0, 5.0)), "positive control");
        assert_eq!(
            paint::point_in_painted_box(&doc.tree, stale.0, 1.0, 20.0, 45.0),
            None
        );
    }
}

#[test]
fn bounding_rect_through_the_dom_document_of_a_stale_id_is_none() {
    for (mut doc, p, stale) in docs() {
        assert!(doc.bounding_rect(p).is_some(), "positive control");
        assert!(doc.bounding_rect(stale).is_none());
    }
}
