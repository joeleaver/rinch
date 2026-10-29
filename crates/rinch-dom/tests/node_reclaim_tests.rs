//! `rinch-dom` frees what a caller discards, and reuses the slot under a new
//! generation (issue #723, PR 2).
//!
//! Before #723 the node slab only ever grew: `discard_node` was the trait
//! default (a detach), and `NodeTree::remove_subtree` — reached by
//! `set_inner_html` and pseudo-element pruning — freed slab slots but never
//! their Taffy nodes. Measured on the issue: 1000 rows discarded and rebuilt
//! 100 times took the slab from 3004 to 303004 entries, with Taffy in lockstep.
//!
//! Freeing makes slots reusable, which is the #304 hazard: a handle kept from
//! before the free names whatever node is minted into the slot next. PR 1 put a
//! generation in every `NodeId` and made every `DomDocument` verb check it; here
//! the generation actually moves, so these fixtures pin both halves — nothing
//! leaks, and a stale handle reaches nothing.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const CSS: &str = "
    .row { height: 20px; font-size: 16px; line-height: 20px; }
    .row:hover { color: red; }
    .row:focus { color: blue; }
";

fn doc_with_list() -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let list = doc.create_element("div");
    doc.append_child(body, list);
    doc.resolve_layout(800.0, 600.0);
    (doc, list)
}

/// `div.row > span > "row N"`.
fn mint_row(doc: &mut RinchDocument, n: usize) -> NodeId {
    let row = doc.create_element("div");
    doc.set_attribute(row, "class", "row");
    let span = doc.create_element("span");
    let text = doc.create_text(&format!("row {n}"));
    doc.append_child(span, text);
    doc.append_child(row, span);
    row
}

fn sizes(doc: &RinchDocument) -> (usize, usize, usize) {
    (
        doc.tree.nodes.len(),
        doc.tree.taffy.total_node_count(),
        doc.tree.taffy_map.len(),
    )
}

/// The issue's measurement, as a fixture: a list whose rows are discarded and
/// rebuilt keeps the slab, the Taffy tree and `taffy_map` within one round's
/// worth of nodes, however many rounds run.
#[test]
fn discarded_rows_are_freed_and_their_slots_reused() {
    const ROWS: usize = 50;
    const ROUNDS: usize = 100;
    let (mut doc, list) = doc_with_list();

    let mut after_first = None;
    for round in 0..ROUNDS {
        let rows: Vec<NodeId> = (0..ROWS).map(|n| mint_row(&mut doc, n)).collect();
        for &row in &rows {
            doc.append_child(list, row);
        }
        // Lay out every round, at a width that moves, so each row gets its
        // Taffy nodes and its measured text (a layout at an unchanged viewport
        // can early-return).
        doc.resolve_layout(800.0 + round as f32, 600.0);
        assert_eq!(doc.get_children(list).len(), ROWS, "precondition: mounted");
        for &row in &rows {
            doc.discard_node(row);
        }
        doc.resolve_layout(700.0 + round as f32, 600.0);
        assert!(doc.get_children(list).is_empty());
        let now = sizes(&doc);
        match after_first {
            None => after_first = Some(now),
            Some(first) => assert_eq!(
                now, first,
                "round {round}: (slab, taffy nodes, taffy_map) must not grow \
                 past the first round's {first:?} — #723"
            ),
        }
    }
    // And the slab really did hold a round's worth at its peak: the rows were
    // there, and are gone.
    let (slab, _, _) = after_first.unwrap();
    assert!(
        slab < ROWS * 3,
        "the discarded rows are not in the slab: {slab}"
    );
}

/// `set_inner_html` freed slab slots before #723 but never their Taffy nodes,
/// nor the `taffy_map` entries naming them.
#[test]
fn set_inner_html_frees_the_taffy_nodes_of_what_it_replaces() {
    let (mut doc, list) = doc_with_list();
    let html: String = (0..30)
        .map(|n| format!("<div class=\"row\"><span>row {n}</span></div>"))
        .collect();
    let mut after_first = None;
    for round in 0..40 {
        doc.set_inner_html(list, &html);
        doc.resolve_layout(800.0 + round as f32, 600.0);
        let now = sizes(&doc);
        match after_first {
            None => after_first = Some(now),
            Some(first) => assert_eq!(now, first, "round {round}: #723"),
        }
    }
}

/// A discarded node's slot goes to the next node minted, under a new
/// generation; the old id names nothing.
#[test]
fn a_discarded_slot_is_reused_under_a_new_generation() {
    let (mut doc, list) = doc_with_list();
    let old = doc.create_element("p");
    doc.set_attribute(old, "id", "old");
    doc.append_child(list, old);
    doc.discard_node(old);

    let new = doc.create_element("em");
    assert_eq!(new.index(), old.index(), "the slot is reused (#723)");
    assert_eq!(new.generation(), old.generation() + 1);
    assert_ne!(new, old);
    assert_eq!(doc.tag_name(old), None, "the old id names nothing");
    assert_eq!(doc.tag_name(new).as_deref(), Some("em"));
}

/// A handle to a discarded node: no verb through it reaches the node minted
/// into its slot (#304's hazard, which freeing makes reachable from every list
/// removal).
#[test]
fn a_stale_handle_never_reaches_the_node_minted_into_its_slot() {
    let (mut doc, list) = doc_with_list();
    let old = mint_row(&mut doc, 0);
    doc.append_child(list, old);
    doc.resolve_layout(800.0, 600.0);
    doc.discard_node(old);

    let new = doc.create_element("div");
    doc.set_attribute(new, "class", "fresh");
    let child = doc.create_text("fresh");
    doc.append_child(new, child);
    doc.append_child(list, new);
    assert_eq!(new.index(), old.index(), "precondition: the slot is reused");
    doc.resolve_layout(801.0, 600.0);

    doc.set_attribute(old, "class", "stale");
    doc.remove_attribute(old, "class");
    doc.set_style(old, "color", "red");
    doc.set_text_content(old, "overwritten");
    doc.set_scroll_top(old, 10.0);
    doc.mark_dirty(old);
    doc.focus_element(old);
    doc.request_scroll_into_view(old);
    doc.remove_node(old);
    doc.discard_node(old);
    doc.append_child(list, old);
    doc.insert_before(list, old, new);

    assert_eq!(doc.get_attribute(new, "class").as_deref(), Some("fresh"));
    assert_eq!(doc.get_attribute(new, "style"), None);
    assert_eq!(doc.get_children(new), vec![child]);
    assert_eq!(doc.text_content(new).as_deref(), Some("fresh"));
    assert_eq!(
        doc.get_children(list),
        vec![new],
        "nothing moved, nothing removed"
    );
    assert_eq!(doc.parent_node(new), Some(list));
    // And the stale request queued nothing a layout would act on.
    doc.resolve_layout(802.0, 600.0);
    assert_eq!(doc.get_children(list), vec![new]);
}

/// The node minted into a freed slot starts clean: none of the hover, focus,
/// active or dirty state the slot's earlier occupant had.
#[test]
fn the_node_minted_into_a_freed_slot_inherits_no_state() {
    let (mut doc, list) = doc_with_list();
    let old = mint_row(&mut doc, 0);
    doc.append_child(list, old);
    doc.resolve_layout(800.0, 600.0);
    let mut changed = false;
    doc.update_hover(Some(old.0), &mut changed);
    doc.update_focus(Some(old.0));
    doc.update_active(Some(old.0));
    doc.set_focus_visible(old.0, true);
    doc.mark_dirty(old);
    assert!(
        doc.tree.nodes.get(old.0).unwrap().is_hovered,
        "precondition"
    );
    doc.discard_node(old);

    let new = doc.create_element("div");
    assert_eq!(new.index(), old.index(), "precondition: the slot is reused");
    doc.append_child(list, new);
    let node = doc.tree.nodes.get(new.0).expect("the new node is live");
    assert!(!node.is_hovered && !node.is_focused && !node.is_active);
    assert!(!node.is_focus_visible);
    assert_ne!(doc.tree.hovered_node, Some(new.0));
    assert_ne!(doc.tree.focused_node, Some(new.0));
    assert_ne!(doc.tree.active_node, Some(new.0));
    // Nothing keyed by the old id survives the free to be read under the slot.
    assert!(
        doc.tree
            .hovered_node
            .is_none_or(|id| doc.tree.nodes.contains(id))
    );
    assert!(
        doc.tree
            .focused_node
            .is_none_or(|id| doc.tree.nodes.contains(id))
    );
    assert!(
        doc.tree
            .active_node
            .is_none_or(|id| doc.tree.nodes.contains(id))
    );
    assert!(!doc.tree.dirty_nodes.contains(&old.0));
    assert!(
        doc.tree
            .taffy_map
            .values()
            .all(|&id| doc.tree.nodes.contains(id))
    );

    // Hovering the new node is a change, and lays out and paints.
    let mut changed = false;
    doc.update_hover(Some(new.0), &mut changed);
    assert!(changed);
    doc.resolve_layout(801.0, 600.0);
}

/// The same through `NodeHandle`, the app's view: a handle kept past its
/// `discard()` reaches nothing through any verb, and in particular not the node
/// another part of the app has since minted into its slot.
#[test]
fn a_discarded_node_handle_never_reaches_its_slots_next_node() {
    use rinch_core::dom::NodeHandle;
    use std::cell::RefCell;
    use std::rc::Rc;

    let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(RinchDocument::new()));
    let weak = Rc::downgrade(&doc);
    let handle = |id: NodeId| NodeHandle::new(id, weak.clone());
    let body = handle(doc.borrow().body());
    let list = handle(doc.borrow_mut().create_element("div"));
    body.append_child(&list);

    let old = handle(doc.borrow_mut().create_element("p"));
    old.set_attribute("tabindex", "0");
    list.append_child(&old);
    doc.borrow_mut().resolve_layout(800.0, 600.0);
    old.discard();

    let new = handle(doc.borrow_mut().create_element("p"));
    new.set_attribute("class", "fresh");
    list.append_child(&new);
    assert_eq!(
        new.node_id().index(),
        old.node_id().index(),
        "precondition: the slot is reused"
    );
    doc.borrow_mut().resolve_layout(801.0, 600.0);

    old.set_attribute("class", "stale");
    old.set_style("color", "red");
    old.set_text("stale");
    old.set_scroll_top(5.0);
    old.scroll_into_view();
    old.scroll_to_fraction(0.5, 0.0);
    old.focus();
    old.set_inner_html("<b>stale</b>");
    old.append_child(&new);
    old.remove();
    old.discard();
    list.append_child(&old);

    assert_eq!(new.get_attribute("class").as_deref(), Some("fresh"));
    assert_eq!(new.get_attribute("style"), None);
    assert!(new.children().is_empty(), "no write reached the new node");
    assert_eq!(
        list.children()
            .iter()
            .map(|c| c.node_id())
            .collect::<Vec<_>>(),
        vec![new.node_id()],
        "the new node stayed put, and the old one did not come back"
    );
    doc.borrow_mut().resolve_layout(802.0, 600.0);
    assert!(doc.borrow().is_retired(old.node_id()));
    assert!(!doc.borrow().is_retired(new.node_id()));
}

/// The out-of-flow pass keeps the boxes it placed (`placed_absolutes`) from one
/// layout to the next, and a scroll write between layouts walks them unchecked
/// (`out_of_flow::replace_after_scroll`). A freed box must leave that list, or
/// the scroll reads the node minted into its slot (a debug build panics on the
/// stale generation; a release build would re-place an unrelated node).
#[test]
fn a_scroll_after_a_discard_does_not_re_place_a_freed_absolute_box() {
    let mut doc = RinchDocument::new();
    doc.load_css(
        "#cb { position: relative; height: 200px; }
         #scroller { overflow: auto; height: 50px; }
         .content { height: 300px; }
         .abs { position: absolute; top: 5px; left: 5px; width: 10px; height: 10px; }",
    );
    let body = doc.body();
    let cb = doc.create_element("div");
    doc.set_attribute(cb, "id", "cb");
    let scroller = doc.create_element("div");
    doc.set_attribute(scroller, "id", "scroller");
    let content = doc.create_element("div");
    doc.set_attribute(content, "class", "content");
    let abs = doc.create_element("div");
    doc.set_attribute(abs, "class", "abs");
    doc.append_child(content, abs);
    doc.append_child(scroller, content);
    doc.append_child(cb, scroller);
    doc.append_child(body, cb);
    doc.resolve_layout(800.0, 600.0);
    assert!(
        doc.tree.placed_absolutes_for_tests().contains(&abs.0),
        "precondition: the box is placed past its layout parent"
    );

    doc.discard_node(content);
    // Mint into the freed slots (the content's, then the box's).
    let minted: Vec<NodeId> = (0..2).map(|_| doc.create_element("span")).collect();
    assert!(
        minted.iter().any(|m| m.index() == abs.index()),
        "precondition: the box's slot is reused"
    );
    assert!(
        !doc.tree.placed_absolutes_for_tests().contains(&abs.0),
        "the freed box left the list"
    );
    doc.set_scroll_top(scroller, 10.0);
    doc.resolve_layout(801.0, 600.0);
}
