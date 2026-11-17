//! The desktop half of `NodeHandle::remove()`'s post-condition, issue #719 —
//! against the real `RinchDocument`, not a mock.
//!
//! # What this file is for
//!
//! #719 was a **backend divergence**: `rinch-dom` left a removed node in the
//! slab on purpose ("Don't remove from slab yet — caller may still reference
//! it", `dom_impl/dom_document_impl.rs`), while `rinch-web` pruned it from both
//! node maps, so the same reactive `if` that toggled a captured `NodeHandle`
//! worked on desktop and silently lost its subtree in a browser.
//!
//! So these fixtures are the **positive control** for the divergence: they were
//! green before the fix as well as after, and that is the point — they say what
//! desktop's behaviour is, so that a future change to `remove_node` that would
//! make the two backends agree by breaking *desktop* fails here rather than in
//! somebody's app.
//!
//! The browser half is `crates/rinch-web/tests/reinsertion.rs` (real Chrome,
//! red before the fix); the helper-level rule is
//! `rinch_core::reinsertion_tests`.
//!
//! # Not about transitions
//!
//! `reinsertion_transition_tests.rs` covers what a re-inserted subtree's
//! *style* must be (#699). This file is only about whether the subtree comes
//! back at all.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

/// `font-size` and `line-height` are declared so nothing below is derived from
/// a font metric — the geometry assertions are declarations a reader can check.
const CSS: &str = "
    .panel { width: 40px; height: 30px; font-size: 16px; line-height: 20px; }
";

/// `body > div.panel > div` laid out once.
fn mounted_panel() -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let panel = doc.create_element("div");
    doc.set_attribute(panel, "class", "panel");
    let inner = doc.create_element("span");
    doc.append_child(panel, inner);
    doc.append_child(body, panel);
    doc.resolve_layout(800.0, 600.0);
    (doc, panel, inner)
}

fn body_children(doc: &RinchDocument) -> Vec<NodeId> {
    doc.get_children(doc.body())
}

/// The post-condition itself: a removed node keeps its identity and its subtree
/// and can be appended again.
#[test]
fn a_removed_node_can_be_inserted_again_with_its_subtree() {
    let (mut doc, panel, inner) = mounted_panel();
    let body = doc.body();
    assert_eq!(body_children(&doc), vec![panel], "precondition: mounted");

    doc.remove_node(panel);
    assert!(
        !body_children(&doc).contains(&panel),
        "precondition: remove unlinks it"
    );

    doc.append_child(body, panel);
    assert_eq!(
        body_children(&doc),
        vec![panel],
        "#719: a removed node must be re-insertable on desktop"
    );
    assert_eq!(
        doc.get_children(panel),
        vec![inner],
        "#719: with its own subtree intact"
    );

    // A different viewport, because `resolve_layout` early-returns on
    // `!layout_dirty` — re-resolving at 800x600 would test nothing.
    doc.resolve_layout(801.0, 600.0);
    let layout = doc.tree.nodes[panel.0].layout;
    assert_eq!(
        (layout.width, layout.height),
        (40.0, 30.0),
        "#719: and it lays out again from its own class"
    );
}

/// The subtree a `replace_node` displaces is a detach too, on the same terms.
#[test]
fn the_node_a_replace_displaces_can_be_inserted_again() {
    let (mut doc, panel, inner) = mounted_panel();
    let body = doc.body();
    let stand_in = doc.create_element("div");

    doc.replace_node(panel, stand_in);
    assert_eq!(
        body_children(&doc),
        vec![stand_in],
        "precondition: the stand-in took its place"
    );

    doc.append_child(body, panel);
    assert_eq!(
        body_children(&doc),
        vec![stand_in, panel],
        "#719: a displaced node must be re-insertable"
    );
    assert_eq!(
        doc.get_children(panel),
        vec![inner],
        "#719: with its own subtree intact"
    );
}

/// `discard_node` is at least as strong as `remove_node` on every backend: the
/// node leaves the tree. On desktop it also **frees** it (#723): the node and
/// its subtree leave the slab and the Taffy tree, as they leave `rinch-web`'s
/// node maps, and `is_retired` says so.
///
/// Until #723 this fixture asserted only the floor (a discard is at least a
/// remove) and its doc comment said what it declined to assert: that the slab
/// entry is gone, because freeing the slot would have recycled the id (#304).
/// The slot's generation is what makes freeing safe, so the assertion is here.
#[test]
fn discard_takes_the_node_out_of_the_tree_here_too() {
    let (mut doc, panel, inner) = mounted_panel();
    let slab = doc.tree.nodes.len();
    let taffy = doc.tree.taffy.total_node_count();

    doc.discard_node(panel);

    assert!(
        !body_children(&doc).contains(&panel),
        "#719: discard removes, whatever else a backend does with the bookkeeping"
    );
    assert_eq!(
        doc.tree.nodes.len(),
        slab - 2,
        "#723: the panel and its span leave the slab"
    );
    assert_eq!(
        doc.tree.taffy.total_node_count(),
        taffy - 2,
        "#723: and their Taffy nodes leave the Taffy tree"
    );
    assert!(doc.tree.get(panel.0).is_none() && doc.tree.get(inner.0).is_none());
    assert!(doc.is_retired(panel) && doc.is_retired(inner));
}

/// **Pins what `discard_node` now does on desktop** (#723), the end of #719's
/// one-sided contract: a discarded node is gone here as it is on `rinch-web`
/// and `MockDomDocument`. It does not re-insert, keeps no subtree, and takes
/// no writes.
///
/// Before #723 PR 2 this fixture was `the_discard_a_desktop_backend_performs_is_inert`
/// and asserted the opposite of each line below: desktop reclaimed nothing,
/// so a discarded node re-inserted with its subtree and took writes. Its doc
/// comment said it would go red when #723 landed, and that it was the list of
/// what changes.
#[test]
fn the_discard_a_desktop_backend_performs_retires_the_node() {
    let (mut doc, panel, inner) = mounted_panel();
    let body = doc.body();

    doc.discard_node(panel);

    doc.append_child(body, panel);
    assert!(
        body_children(&doc).is_empty(),
        "#723: a discarded node does not re-insert"
    );
    assert!(
        doc.get_children(panel).is_empty(),
        "#723: it keeps no subtree"
    );
    assert_eq!(doc.parent_node(inner), None);

    doc.set_attribute(panel, "class", "still-writable");
    assert_eq!(
        doc.get_attribute(panel, "class"),
        None,
        "#723: and it takes no writes"
    );
}

/// **`rinch-dom` recycles slab slots, never ids** (issues #304, #723).
///
/// `set_inner_html` reaches `NodeTree::remove_subtree`, which frees the slab
/// slots, and the slab has a free list, so the next `create_element` gets one
/// of those slots back. Until #723 PR 2 it got the *id* back too (generation
/// 0), so a handle to a freed node named the new one: #304's recycled-slot
/// hazard. Now each free moves the slot's generation on, so the slot comes
/// back under an id no handle holds.
#[test]
fn set_inner_html_recycles_the_slab_slots_it_frees_under_new_ids() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let host = doc.create_element("div");
    doc.append_child(body, host);

    doc.set_inner_html(host, "<p>one</p><p>two</p>");
    let freed = doc.get_children(host);
    assert_eq!(freed.len(), 2, "precondition: two children to free");

    // Blows the first pair away — `remove_subtree`, which frees their slots.
    doc.set_inner_html(host, "<span>fresh</span>");

    let minted = [doc.create_element("div"), doc.create_element("div")];
    let same_slot: Vec<_> = minted
        .iter()
        .filter(|id| freed.iter().any(|f| f.index() == id.index()))
        .collect();
    assert!(
        !same_slot.is_empty(),
        "positive control: a slot freed by set_inner_html comes back — \
         freed {freed:?}, minted {minted:?}"
    );
    assert!(
        minted.iter().all(|id| !freed.contains(id)),
        "#723: but never under a freed id — freed {freed:?}, minted {minted:?}"
    );
}
