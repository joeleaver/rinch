//! `RinchDocument`'s `DomDocument` verbs with an id that names no live node
//! (issue #723, PR 1).
//!
//! A `NodeId` is a slot and a generation. Nothing frees a slot into a new
//! generation yet (PR 2 does), so these fixtures build the stale id by hand:
//! a live node's slot one generation on, which is exactly what a handle kept
//! across a free and a reuse will hold. Every verb must treat it as naming
//! nothing — a write is a no-op and a read answers `None` — and must leave the
//! node that does hold the slot alone. That is what `rinch-web` and
//! `MockDomDocument` already do with an id they have retired.
//!
//! Before #723 the same calls indexed the slab directly: a vacant slot
//! panicked, and an id for a reused slot wrote into its new occupant (#304).

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

/// `body > p#live > "text"`, and `p`'s slot one generation on.
fn doc_with_stale_twin() -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let p = doc.create_element("p");
    doc.set_attribute(p, "id", "live");
    let text = doc.create_text("text");
    doc.append_child(p, text);
    doc.append_child(body, p);
    assert_eq!(p.generation(), 0, "every id is issued at generation 0");
    let stale = NodeId::new(p.index() as u32, p.generation() + 1);
    (doc, p, stale)
}

#[test]
fn reads_through_a_stale_id_answer_nothing() {
    let (doc, p, stale) = doc_with_stale_twin();
    // Positive control: the live id reads.
    assert_eq!(doc.get_attribute(p, "id").as_deref(), Some("live"));
    assert_eq!(doc.tag_name(p).as_deref(), Some("p"));

    assert_eq!(doc.get_attribute(stale, "id"), None);
    assert_eq!(doc.tag_name(stale), None);
    assert_eq!(doc.text_content(stale), None);
    assert_eq!(doc.parent_node(stale), None);
    assert!(doc.get_children(stale).is_empty());
}

#[test]
fn writes_through_a_stale_id_leave_the_slots_live_node_alone() {
    let (mut doc, p, stale) = doc_with_stale_twin();
    let body = doc.body();

    doc.set_attribute(stale, "id", "stale");
    doc.set_attribute(stale, "class", "stale");
    doc.remove_attribute(stale, "id");
    doc.set_style(stale, "color", "red");
    doc.set_text_content(stale, "overwritten");
    doc.mark_dirty(stale);
    doc.set_scroll_top(stale, 10.0);
    doc.set_scroll_left(stale, 10.0);
    doc.set_inner_html(stale, "<em>new</em>");

    assert_eq!(doc.get_attribute(p, "id").as_deref(), Some("live"));
    assert_eq!(doc.get_attribute(p, "class"), None);
    assert_eq!(doc.get_attribute(p, "style"), None);
    assert_eq!(doc.text_content(p).as_deref(), Some("text"));
    assert_eq!(doc.get_children(body), vec![p]);
}

#[test]
fn structural_verbs_with_a_stale_id_move_nothing() {
    let (mut doc, p, stale) = doc_with_stale_twin();
    let body = doc.body();
    let other = doc.create_element("div");
    doc.append_child(body, other);
    assert_eq!(doc.get_children(body), vec![p, other]);

    doc.remove_node(stale);
    doc.discard_node(stale);
    doc.remove_child(body, stale);
    doc.append_child(other, stale);
    doc.append_child(stale, other);
    doc.insert_before(body, stale, other);
    doc.insert_before(body, other, stale);
    doc.insert_child(other, stale, 0);
    doc.replace_node(stale, other);
    doc.replace_node(other, stale);

    assert_eq!(doc.get_children(body), vec![p, other], "nothing moved");
    assert_eq!(doc.parent_node(p), Some(body));
    assert!(doc.get_children(other).is_empty());

    // The live id still works after all that.
    doc.remove_node(p);
    assert_eq!(doc.get_children(body), vec![other]);
}

#[test]
fn an_id_past_every_slot_is_a_no_op_rather_than_a_panic() {
    let (mut doc, p, _) = doc_with_stale_twin();
    let body = doc.body();
    let nowhere = NodeId::from_index(100_000);
    doc.set_attribute(nowhere, "id", "x");
    doc.append_child(body, nowhere);
    doc.remove_node(nowhere);
    assert_eq!(doc.get_attribute(nowhere, "id"), None);
    assert_eq!(doc.get_children(body), vec![p]);
}
