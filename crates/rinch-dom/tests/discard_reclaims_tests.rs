//! `discard_node` frees what it discards on desktop (issue #723, PR 2).
//!
//! Until #723 `rinch-dom`'s `discard_node` was the trait default: a detach that
//! left every node in the slab and every Taffy node in the Taffy tree, so a
//! window that rebuilt a list kept every row it ever drew. Freeing reuses
//! slots, which is #304's hazard (a kept id naming a different node); the
//! slot's generation is what closes it, and the second half of this file pins
//! that a handle kept from before a free writes nothing into the node that
//! reuses its slot.

#![cfg(feature = "software-renderer")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{DomDocument, NodeId, RenderScope};
use rinch_core::reactive::Signal;
use rinch_dom::RinchDocument;

const CSS: &str = "
    li { display: block; height: 10px; font-size: 16px; line-height: 10px; }
";

/// What the fixture measures: slab entries and Taffy nodes.
fn sizes(doc: &RinchDocument) -> (usize, usize) {
    (doc.tree.nodes.len(), doc.tree.taffy.total_node_count())
}

/// Lay out and consume a paint, so every frame-scoped table the churn fed is
/// read and drained the way a window's frame does.
fn frame(doc: &Rc<RefCell<RinchDocument>>, width: f32) {
    let mut d = doc.borrow_mut();
    d.resolve_layout(width, 600.0);
    let _ = rinch_dom::paint::compute_dirty_region(&d.tree, 1.0, width as f64, 600.0);
    d.tree.consume_paint_dirty();
}

/// The fixture the issue asked for: a 1000-row keyed `for` whose every row is
/// replaced, 100 times over. Each round's keys are new, so the `for` discards
/// all 1000 rows (`li > span > text`) and builds 1000 more.
///
/// Before #723 PR 2: the slab grew by 3000 entries a round (3006 → 303006) and
/// the Taffy tree with it. After: both are flat.
#[test]
fn a_churned_keyed_for_keeps_the_slab_and_the_taffy_tree_flat() {
    const ROWS: usize = 1000;
    const ROUNDS: usize = 100;

    let doc = Rc::new(RefCell::new(RinchDocument::new()));
    doc.borrow_mut().load_css(CSS);
    let body = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    let mut scope = RenderScope::new(dyn_doc, body);
    let list = scope.create_element("ul");
    scope.parent().append_child(&list);

    let round = Signal::new(0usize);
    let _marker = rinch_core::for_loop::for_each_dom_typed(
        &mut scope,
        &list,
        move || {
            let r = round.get();
            (0..ROWS).map(|i| (r, i)).collect::<Vec<_>>()
        },
        |&(r, i): &(usize, usize)| format!("{r}-{i}"),
        |(r, i): (usize, usize), s: &mut RenderScope| {
            let li = s.create_element("li");
            let span = s.create_element("span");
            let text = s.create_text(&format!("row {i} of round {r}"));
            span.append_child(&text);
            li.append_child(&span);
            li
        },
    );
    frame(&doc, 800.0);
    let before = sizes(&doc.borrow());
    assert_eq!(
        doc.borrow().get_children(list.node_id()).len(),
        ROWS + 1,
        "precondition: the rows and the for's marker"
    );

    let t0 = std::time::Instant::now();
    for r in 1..=ROUNDS {
        round.set(r);
        // A frame every tenth round (a different width each time, so
        // `resolve_layout` lays out): the rows of the other nine are discarded
        // without ever being laid out, which is a route of its own, and a
        // frame per round took six minutes in a debug build.
        if r % 10 == 0 {
            frame(&doc, 800.0 + (r % 20) as f32 / 10.0);
        }
    }
    let elapsed = t0.elapsed();
    let after = sizes(&doc.borrow());
    eprintln!(
        "slab {} -> {}, taffy {} -> {} over {ROUNDS} rounds of {ROWS} rows in {:.2?}",
        before.0, after.0, before.1, after.1, elapsed
    );

    assert_eq!(
        doc.borrow().get_children(list.node_id()).len(),
        ROWS + 1,
        "positive control: the list still shows its rows"
    );
    assert_eq!(
        after.0, before.0,
        "#723: a discarded row's nodes leave the slab, so churn does not grow it"
    );
    assert_eq!(
        after.1, before.1,
        "#723: and their Taffy nodes leave the Taffy tree"
    );
    assert_eq!(
        doc.borrow().tree.taffy_map.len(),
        after.1,
        "#723: taffy_map names only live Taffy nodes, one per slab node with one"
    );
}

/// `set_inner_html` frees through the same route: its Taffy nodes leaked
/// until #723 PR 2 even though its slab entries did not.
#[test]
fn set_inner_html_churn_keeps_the_taffy_tree_flat() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let host = doc.create_element("div");
    doc.append_child(body, host);
    doc.set_inner_html(host, "<p>one <em>two</em></p><p>three</p>");
    doc.resolve_layout(800.0, 600.0);
    let before = sizes(&doc);
    for i in 0..50 {
        doc.set_inner_html(host, "<p>one <em>two</em></p><p>three</p>");
        doc.resolve_layout(800.0 + (i % 2) as f32, 600.0);
    }
    assert_eq!(sizes(&doc), before, "#723: slab and Taffy tree flat");
}

/// `body > div > span`, laid out, and the div discarded: its slot (and the
/// span's) is free for the next node created.
fn discarded_panel() -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let panel = doc.create_element("div");
    let inner = doc.create_element("span");
    doc.append_child(panel, inner);
    doc.append_child(body, panel);
    doc.resolve_layout(800.0, 600.0);
    doc.discard_node(panel);
    (doc, panel, inner)
}

/// A node created after a discard that reuses `stale`'s slot, which is what
/// makes the tests below about *another node in the same slot* rather than
/// about a vacant one.
fn reuse_slot_of(doc: &mut RinchDocument, stale: NodeId) -> NodeId {
    for _ in 0..8 {
        let n = doc.create_element("p");
        if n.index() == stale.index() {
            return n;
        }
    }
    panic!("precondition: the slot of {stale:?} is reused");
}

#[test]
fn a_discarded_node_is_retired_and_its_id_names_nothing() {
    let (doc, panel, inner) = discarded_panel();
    assert!(
        doc.is_retired(panel),
        "#723: discard reclaims on desktop too"
    );
    assert!(doc.is_retired(inner), "with its subtree");
    assert!(doc.tree.get(panel.0).is_none());
    assert_eq!(doc.get_attribute(panel, "class"), None);
    assert!(doc.get_children(panel).is_empty());
}

#[test]
fn a_reused_slot_gets_a_new_generation() {
    let (mut doc, panel, _) = discarded_panel();
    let fresh = reuse_slot_of(&mut doc, panel);
    assert_eq!(fresh.index(), panel.index(), "precondition: same slot");
    assert_ne!(fresh, panel, "#723: the slot is re-issued, the id is not");
    assert!(fresh.generation() > panel.generation());
    assert!(!doc.is_retired(fresh));
    assert!(doc.is_retired(panel), "the old id still names nothing");
}

/// The live bug the issue recorded against `set_inner_html` (a handle to an
/// old `<p>` wrote `set_attribute` onto the `<em>` minted into its slot), on
/// the discard route: every verb a stale handle can reach writes nothing into
/// the node that took its slot.
#[test]
fn a_stale_handle_writes_nothing_into_the_node_that_reuses_its_slot() {
    let (mut doc, panel, _) = discarded_panel();
    let body = doc.body();
    let fresh = reuse_slot_of(&mut doc, panel);
    let child = doc.create_element("b");
    doc.append_child(fresh, child);
    doc.append_child(body, fresh);

    doc.set_attribute(panel, "class", "stale");
    doc.set_attribute(panel, "data-x", "1");
    doc.set_style(panel, "color", "red");
    doc.set_text_content(panel, "stale text");
    let orphan = doc.create_element("i");
    doc.append_child(panel, orphan);
    doc.insert_before(panel, orphan, child);
    doc.remove_child(panel, child);
    doc.remove_attribute(fresh, "nothing-to-remove");

    assert_eq!(
        doc.get_attribute(fresh, "class"),
        None,
        "#723: no attribute"
    );
    assert_eq!(doc.get_attribute(fresh, "data-x"), None);
    assert_eq!(doc.get_attribute(fresh, "style"), None, "no style");
    assert_eq!(
        doc.get_children(fresh),
        vec![child],
        "#723: its children are its own: no text, no append, no removal"
    );
    assert_eq!(doc.parent_node(orphan), None);

    // Structural verbs on the stale id leave the live node where it is.
    doc.remove_node(panel);
    doc.discard_node(panel);
    doc.append_child(body, panel);
    assert_eq!(doc.parent_node(fresh), Some(body), "still attached");
    assert!(
        !doc.is_retired(fresh),
        "#723: a stale discard frees nothing"
    );
    assert!(!doc.get_children(body).contains(&panel));
    doc.resolve_layout(801.0, 600.0);
}

/// The same through `set_inner_html`, which reused slots at generation 0
/// before #723 PR 2: the issue's measured case.
#[test]
fn a_handle_into_set_inner_html_content_is_stale_after_the_next_set() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let host = doc.create_element("div");
    doc.append_child(body, host);
    // Childless, so the `<em>` is minted into the `<p>`'s own slot.
    doc.set_inner_html(host, "<p></p>");
    let old_p = doc.get_children(host)[0];

    doc.set_inner_html(host, "<em></em>");
    let em = doc.get_children(host)[0];
    assert_eq!(
        em.index(),
        old_p.index(),
        "precondition: the slot is reused"
    );
    doc.set_attribute(old_p, "class", "stale");
    assert_eq!(
        doc.get_attribute(em, "class"),
        None,
        "#723: the old <p>'s handle does not write into the new <em>"
    );
}

/// Freed nodes leave the document's interaction state: a discarded hovered or
/// focused node is not kept as the tree's hovered or focused node.
#[test]
fn a_discarded_hovered_and_focused_node_is_forgotten() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let panel = doc.create_element("div");
    doc.append_child(body, panel);
    doc.tree.hovered_node = Some(panel.0);
    doc.tree.focused_node = Some(panel.0);
    doc.discard_node(panel);
    assert_eq!(doc.tree.hovered_node, None);
    assert_eq!(doc.tree.focused_node, None);
}
