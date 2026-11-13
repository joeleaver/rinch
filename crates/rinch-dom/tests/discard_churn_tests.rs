//! Discarding a long list one `discard_node` call at a time, with no frame in
//! between, does work linear in the number of rows (#2627).
//!
//! The repro: 2N `li` rows under `li:nth-child(odd)`, one frame, N new rows
//! inserted before them, then the N old rows discarded one call each. Two
//! costs were per call and proportional to the list, so the whole was
//! quadratic (debug build: N=500 90 ms, 1000 364 ms, 2000 1.1 s, 4000 5.1 s):
//!
//! - `NodeTree::remove_subtree` pruned `style_roots` (about 2N roots, one per
//!   sibling the selector invalidated) on every free (#2619 brought it);
//! - `note_child_list_changed` marked every later sibling for restyle on every
//!   removal, after cloning the child list (older: before #2619 too).
//!
//! The fixtures count the work (`NodeTree::free_prune_steps`,
//! `NodeTree::sibling_mark_steps`), not wall time: at N and 2N the counts must
//! double, not quadruple, and stay under a small multiple of N.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const CSS: &str = "
    li { display: block; height: 10px; font-size: 16px; line-height: 10px; color: #000000; }
    li:nth-child(odd) { color: #ff0000; }
    li.x { color: #0000ff; }
";

fn hex(doc: &RinchDocument, n: NodeId) -> String {
    let c = doc.tree.get(n.0).unwrap().computed_style.color;
    let rgba = c.expect("color should resolve").to_rgba8();
    format!("#{:02x}{:02x}{:02x}", rgba.r, rgba.g, rgba.b)
}

/// Every row's colour matches its place: odd (1-based) rows red.
fn assert_parity(doc: &RinchDocument, ul: NodeId, what: &str) {
    for (i, li) in doc.get_children(ul).into_iter().enumerate() {
        let want = if i % 2 == 0 { "#ff0000" } else { "#000000" };
        assert_eq!(hex(doc, li), want, "{what}: row {} (1-based)", i + 1);
    }
}

fn list(rows: usize) -> (RinchDocument, NodeId, Vec<NodeId>) {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let ul = doc.create_element("ul");
    doc.append_child(body, ul);
    let old: Vec<NodeId> = (0..rows)
        .map(|_| {
            let li = doc.create_element("li");
            doc.append_child(ul, li);
            li
        })
        .collect();
    doc.resolve_layout(800.0, 600.0);
    (doc, ul, old)
}

/// The issue's repro at size `n`: the work the discard phase did, as
/// (prune steps, sibling-mark steps).
fn discard_phase_work(n: usize) -> (u64, u64) {
    let (mut doc, ul, old) = list(2 * n);
    let first = old[0];
    for _ in 0..n {
        let li = doc.create_element("li");
        doc.insert_before(ul, li, first);
    }
    let (p0, m0) = (doc.tree.free_prune_steps, doc.tree.sibling_mark_steps);
    for &id in &old[..n] {
        doc.discard_node(id);
    }
    let work = (
        doc.tree.free_prune_steps - p0,
        doc.tree.sibling_mark_steps - m0,
    );
    // Not a lost restyle: every remaining row is restyled to its new place.
    // A different viewport, so the resolve does not early-return.
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(doc.get_children(ul).len(), 2 * n);
    assert_parity(&doc, ul, "after the discards");
    work
}

#[test]
fn discarding_rows_one_call_at_a_time_is_linear() {
    const N: usize = 150;
    let (prune_n, marks_n) = discard_phase_work(N);
    let (prune_2n, marks_2n) = discard_phase_work(2 * N);
    eprintln!("N={N}: prune {prune_n}, marks {marks_n}; 2N: prune {prune_2n}, marks {marks_2n}");
    // Positive control: the sweep ran, so the counter counts something.
    assert!(marks_n > 0, "precondition: the selector marked siblings");
    // Before: about 2N² entries pruned and 3N²/2 children marked. After:
    // nothing pruned (the style lists are left to the resolve) and about 3N
    // marked — the first removal's sweep, then one child per removal.
    for (what, at_n, at_2n) in [("prune", prune_n, prune_2n), ("marks", marks_n, marks_2n)] {
        assert!(
            at_n <= 4 * N as u64,
            "#2627: {what} work {at_n} for N={N} is not O(N)"
        );
        assert!(
            at_2n <= 2 * at_n + 8,
            "#2627: {what} work went {at_n} -> {at_2n} when N doubled (quadratic)"
        );
    }
}

/// A row moved within its list keeps its style (#914), so nothing moves the
/// sibling-mark epoch, and its stamp — "this row and every row after it are
/// marked" — travels with it. The sweep for its new place must not trust
/// that stamp: the rows it now precedes were never marked.
#[test]
fn a_stamped_row_moved_to_the_front_still_restyles_the_rows_it_passes() {
    let (mut doc, ul, rows) = list(10);
    assert_parity(&doc, ul, "precondition");
    // Rows 6.. are marked and stamped (rows[6] is now at index 5).
    doc.discard_node(rows[5]);
    // A stamped row moved to the front, no frame in between.
    doc.insert_before(ul, rows[8], rows[0]);
    doc.resolve_layout(801.0, 600.0);
    assert_parity(&doc, ul, "after the discard and the move");
}

/// The deferred prune leaves a freed id in `style_roots` until the next
/// resolve. A node created in the meantime may take its slot; it has a later
/// generation, so the stale id names nothing and the drain skips it.
#[test]
fn a_stale_style_root_does_not_style_the_node_in_its_slot() {
    let (mut doc, ul, rows) = list(6);
    // Restyles rows[1..] (pushing them as style roots), then frees rows[1].
    doc.discard_node(rows[0]);
    let stale = rows[1];
    assert!(
        doc.tree.style_roots.contains(&stale.0),
        "precondition: the row is a pending style root"
    );
    doc.discard_node(stale);
    assert!(
        doc.tree.style_roots.contains(&stale.0),
        "the free leaves its root to the resolve (#2627)"
    );
    let fresh = (0..8)
        .map(|_| doc.create_element("li"))
        .find(|n| n.index() == stale.index())
        .expect("precondition: the slot is reused");
    assert_ne!(fresh, stale, "a later generation in the same slot");
    doc.set_attribute(fresh, "class", "x");

    // Detached: the resolve must not cascade it through the stale root.
    doc.resolve_layout(801.0, 600.0);
    assert!(
        !doc.tree.style_roots.contains(&stale.0),
        "the resolve drained the stale root"
    );
    assert!(
        !doc.tree.get(fresh.0).unwrap().stylo_element_data.has_data(),
        "a detached node is not styled, whoever held its slot before"
    );
    assert_parity(&doc, ul, "the rows left");

    // Connected, it is styled as itself.
    doc.append_child(ul, fresh);
    doc.resolve_layout(802.0, 600.0);
    assert_eq!(hex(&doc, fresh), "#0000ff");
}

/// A stamp does not outlive the resolve that cascades the rows it vouched
/// for: after a frame, a removal before them sweeps them again.
#[test]
fn a_frame_between_discards_takes_the_stamps_with_it() {
    let (mut doc, ul, rows) = list(10);
    // Rows 7.. are marked and stamped, then cascaded by the frame.
    doc.discard_node(rows[6]);
    doc.resolve_layout(801.0, 600.0);
    assert_parity(&doc, ul, "after the first discard");
    // A removal before them: every row after it moves by one.
    doc.discard_node(rows[2]);
    doc.resolve_layout(802.0, 600.0);
    assert_parity(&doc, ul, "after the second discard");
}
