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
/// node leaves the tree.
///
/// Desktop's `discard_node` is the trait default — it reclaims nothing, because
/// its slab is per-document and freeing the slot would recycle the id (#304).
/// So this fixture asserts the *floor the contract guarantees* — a discard is at
/// least a remove — and deliberately does **not** assert that the slab entry is
/// gone: that would pin a promise desktop does not make. That the slab therefore
/// only grows is **#723**, and the fixture that belongs here once it is fixed is
/// named in that issue.
///
/// **Measured, so nobody has to take the inertness on trust**: after this call a
/// discarded node on `rinch-dom` still re-inserts with its subtree and still
/// takes attribute writes. `the_discard_a_desktop_backend_performs_is_inert`
/// below says so out loud, because the trait doc's one-sided contract is only
/// credible if the weak end is written down somewhere that fails when it
/// changes.
#[test]
fn discard_takes_the_node_out_of_the_tree_here_too() {
    let (mut doc, panel, _inner) = mounted_panel();

    doc.discard_node(panel);

    assert!(
        !body_children(&doc).contains(&panel),
        "#719: discard removes, whatever else a backend does with the bookkeeping"
    );
}

/// **Pins the weak end of `discard_node`'s one-sided contract** (issues #719,
/// #723), so the trait doc's "may reclaim nothing" is a measured statement and
/// not an assumption.
///
/// `rinch-dom` takes the trait default, so a discarded node here is exactly a
/// removed one: it re-inserts, keeps its subtree, and takes writes. An app that
/// leaned on that would be right on desktop and dead on `rinch-web`, which is
/// #719's own shape one verb along — `MockDomDocument` retires like the browser
/// and is what makes that mistake fail `cargo test`.
///
/// When #723 lands this fixture goes red. That is the point: it is the list of
/// what changes.
#[test]
fn the_discard_a_desktop_backend_performs_is_inert() {
    let (mut doc, panel, inner) = mounted_panel();
    let body = doc.body();

    doc.discard_node(panel);

    doc.append_child(body, panel);
    assert_eq!(
        body_children(&doc),
        vec![panel],
        "#723: desktop reclaims nothing on discard, so the node still re-inserts"
    );
    assert_eq!(
        doc.get_children(panel),
        vec![inner],
        "#723: with its subtree"
    );

    doc.set_attribute(panel, "class", "still-writable");
    assert_eq!(
        doc.get_attribute(panel, "class").as_deref(),
        Some("still-writable"),
        "#723: and it still takes writes"
    );
}

/// **`rinch-dom` DOES recycle node ids — just never through `discard`** (issues
/// #304, #723).
///
/// The trait doc says a *discarded* id is never handed to a different node, and
/// it is careful to say only that. This fixture is why the qualifier is there:
/// `set_inner_html` reaches `NodeTree::remove_subtree`, which frees the slab
/// key, and `slab::Slab` has a free list, so the next `create_element` gets that
/// key back. #304's recycled-slot hazard is live on desktop today, independently
/// of anything #719 added.
///
/// Constructed rather than assumed, because an earlier round of PR #728 wrote
/// the unqualified claim ("ids are never re-issued on either backend") into
/// three documents.
#[test]
fn set_inner_html_recycles_the_slab_keys_it_frees() {
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
    let recycled: Vec<_> = minted.iter().filter(|id| freed.contains(id)).collect();

    assert!(
        !recycled.is_empty(),
        "#304: a slab key freed by set_inner_html must be observed coming back — \
         freed {freed:?}, minted {minted:?}. If this stops being true, slab has \
         stopped recycling and the qualifier on `discard_node`'s id claim can go"
    );
}
