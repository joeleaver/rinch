//! What the #702 connectivity test costs on the move verbs.
//!
//! `#[ignore]`d — a timing harness, not a gate. Run it with
//! `cargo test -p rinch-dom --release --test move_reorder_bench -- --ignored --nocapture`.
//!
//! # The shape that was worried about
//!
//! `append_child`, `insert_before` and `insert_child` are the hottest DOM verbs
//! in the framework, and they are also the **move** routes: a keyed `for`
//! reorder splices rows with `insert_after`, which is one of them. Asking "is
//! the destination still in the document" is an O(depth) ancestor walk, and
//! paying it per row of a 500-row reorder is what made #702 a question rather
//! than an obvious fix.
//!
//! It is not paid there, and by construction rather than by luck. A keyed
//! reorder moves rows **within one container**, so the destination parent is the
//! one the row already had — and a move within one parent cannot change whether
//! the child is connected, because the child's reachability *is* its parent's.
//! `detach_subtree_styles_if_moved_out` short-circuits on `old_parent !=
//! new_parent` before it walks anything, so a reorder pays one integer
//! comparison per row. A freshly created node has no old parent at all, so the
//! initial build of a tree does not reach the helper either.
//!
//! Only a **reparenting** move walks, and [`reparenting_500_rows`] is what that
//! costs.
//!
//! # Measured
//!
//! Release, best of 200 rounds, 500 rows of 3 nodes each (a row div, a span, a
//! text node). The whole move loop, in microseconds. Two binaries built from
//! committed revisions and **alternated**, three rounds each — the body of
//! `detach_subtree_styles_if_moved_out` emptied for the `without` column, and
//! nothing else changed.
//!
//! The numbers are in the PR for #702 and in `report-fix-702.md`; they are not
//! repeated here, because a table in a doc comment goes stale the moment the
//! harness is run on another host and a reader cannot tell which host it came
//! from. What belongs here is the shape and the control.
//!
//! Each harness prints a `moved-out node reset: true/false` line taken **outside
//! the timed loop**, from a node moved into a genuinely detached parent. That is
//! the positive control: it says the instrument fired, and it is the only thing
//! distinguishing the two columns from two runs of the same binary.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

/// `body > div.list > 500 * (div.row > span > "cell")`, laid out once with
/// transitions armed so every node carries a real style to reset.
///
/// Returns `(doc, list, rows)`.
fn built_list(rows: usize) -> (RinchDocument, NodeId, Vec<NodeId>) {
    let mut doc = RinchDocument::new();
    doc.load_css(
        ".row { width: 100px; height: 10px; font-size: 16px; line-height: 20px; \
                transition: width 150ms linear; }",
    );
    let body = doc.body();
    let list = doc.create_element("div");
    doc.append_child(body, list);
    let mut row_ids = Vec::with_capacity(rows);
    for _ in 0..rows {
        let row = doc.create_element("div");
        doc.set_attribute(row, "class", "row");
        let span = doc.create_element("span");
        let text = doc.create_text("cell");
        doc.append_child(span, text);
        doc.append_child(row, span);
        doc.append_child(list, row);
        row_ids.push(row);
    }
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    (doc, list, row_ids)
}

/// Does a move into a detached parent reset the moved node? Printed, not
/// asserted, so that the **same harness runs against a revision without the
/// check** — which is the only way to get a delta out of it.
fn reset_fires(doc: &mut RinchDocument, row: NodeId) -> bool {
    let orphan = doc.create_element("div");
    doc.append_child(orphan, row);
    !doc.tree.get(row.0).unwrap().has_been_styled
}

/// The hot path: a keyed `for` reorder, which moves rows **within one
/// container**.
///
/// Every row is moved to the front, which is the route a reorder takes for
/// every move that is not to the last position (`NodeHandle::insert_after`
/// falls through to `append_child` only when its anchor has no next sibling —
/// the distinction #699's review round turned on).
#[test]
#[ignore = "timing harness"]
fn reordering_500_rows() {
    const ROWS: usize = 500;
    const ROUNDS: usize = 200;

    let mut best = f64::MAX;
    let mut total = 0.0;
    for _ in 0..ROUNDS {
        let (mut doc, list, rows) = built_list(ROWS);
        let first = rows[0];
        let t0 = std::time::Instant::now();
        for row in rows.iter().skip(1) {
            doc.insert_before(list, *row, first);
        }
        let us = t0.elapsed().as_secs_f64() * 1e6;
        best = best.min(us);
        total += us;
    }

    let (mut doc, _list, rows) = built_list(4);
    let fired = reset_fires(&mut doc, rows[0]);
    println!(
        "insert_before within one parent x {} : best {best:.1}us, mean {:.1}us \
         ({:.3}us/row) [moved-out node reset: {fired}]",
        ROWS - 1,
        total / ROUNDS as f64,
        best / (ROWS - 1) as f64
    );
}

/// The path that actually walks: a **reparenting** move whose destination is
/// still in the document.
///
/// Every row is moved from one list into a second one, so `old_parent !=
/// new_parent` and `depth_if_connected` runs on each. The destination is a
/// sibling of the source, so the walk is the shallowest a real one can be; a
/// deeply nested destination costs proportionally more, which is the shape of
/// the bound rather than a separate measurement.
#[test]
#[ignore = "timing harness"]
fn reparenting_500_rows() {
    const ROWS: usize = 500;
    const ROUNDS: usize = 200;

    let mut best = f64::MAX;
    let mut total = 0.0;
    for _ in 0..ROUNDS {
        let (mut doc, _list, rows) = built_list(ROWS);
        let body = doc.body();
        let dest = doc.create_element("div");
        doc.append_child(body, dest);
        let t0 = std::time::Instant::now();
        for row in &rows {
            doc.append_child(dest, *row);
        }
        let us = t0.elapsed().as_secs_f64() * 1e6;
        best = best.min(us);
        total += us;
    }

    let (mut doc, _list, rows) = built_list(4);
    let fired = reset_fires(&mut doc, rows[0]);
    println!(
        "append_child into a different connected parent x {ROWS}: best {best:.1}us, \
         mean {:.1}us ({:.3}us/row) [moved-out node reset: {fired}]",
        total / ROUNDS as f64,
        best / ROWS as f64
    );
}
