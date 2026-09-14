//! What `detach_subtree_styles` costs on a large removal (#699).
//!
//! `#[ignore]`d — a timing harness, not a gate. Run it with
//! `cargo test -p rinch-dom --release --test detach_reset_bench -- --ignored --nocapture`.
//!
//! # The shape that was worried about
//!
//! A 500-row list unmounted in one go: `for_each_dom_typed`'s `Remove` arm
//! calls `NodeHandle::remove` per row, so `remove_node` runs 500 times, and the
//! reset adds one more subtree walk to each. The question is whether that is a
//! new order of work or noise beside what the call already does.
//!
//! It is noise, and by construction rather than by luck: `remove_node` already
//! walks the whole removed subtree **twice** before the reset ever runs —
//! `mark_subtree_paint_dirty` and `clear_ifc_root_recursive`, both of them the
//! same iterative stack over the same nodes. The reset is a third walk of the
//! same shape whose body is a bool write and a `HashMap::remove`.
//!
//! # Measured
//!
//! Release, best of 200 rounds, 500 rows of 3 nodes each (a row div, a span, a
//! text node) removed one row at a time. The whole removal loop, in
//! microseconds. Two binaries built from committed revisions and **alternated**,
//! three rounds each — the reset commented out of all three call sites for the
//! `without` column, and nothing else changed:
//!
//! | round | with the reset | without |
//! |---|---|---|
//! | 1 | 340.3 | 324.8 |
//! | 2 | 342.8 | 318.2 |
//! | 3 | 332.9 | 319.7 |
//!
//! About **+17us on ~330us**, or +5%: 0.034us per row, ~11ns per node over 1500
//! nodes. The sign is the same in all three rounds, so it is a real cost and not
//! host noise, and it is what a stack walk writing one bool and probing one
//! `HashMap` per node costs. Nothing here is a new order of work.
//!
//! The harness prints `deep node reset: true/false` from the innermost node of
//! the first removed row, which is the positive control: it says the instrument
//! actually fired, and it is what distinguishes the two columns above from two
//! runs of the same binary.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

/// `body > div.list > 500 * (div.row > span > "cell")`, laid out once with
/// transitions armed so every node carries a real style to reset.
fn built_list(rows: usize) -> (RinchDocument, Vec<rinch_core::dom::NodeId>) {
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
    (doc, row_ids)
}

#[test]
#[ignore = "timing harness"]
fn removing_500_rows() {
    const ROWS: usize = 500;
    const ROUNDS: usize = 200;

    let mut best = f64::MAX;
    let mut total = 0.0;
    let mut reset_seen = false;
    for _ in 0..ROUNDS {
        let (mut doc, rows) = built_list(ROWS);
        let t0 = std::time::Instant::now();
        for row in &rows {
            doc.remove_node(*row);
        }
        let us = t0.elapsed().as_secs_f64() * 1e6;
        best = best.min(us);
        total += us;
        // Positive control: the removal really happened, and — printed rather
        // than asserted so that the **same harness runs against a revision
        // without the reset**, which is the only way to get a delta out of it.
        let deep = doc.tree.get(rows[0].0).unwrap().children[0];
        assert!(doc.tree.get(rows[0].0).unwrap().parent.is_none());
        reset_seen = !doc.tree.get(deep).unwrap().has_been_styled;
    }
    println!(
        "remove_node x {ROWS}: best {best:.1}us, mean {:.1}us ({:.3}us/row) \
         [deep node reset: {reset_seen}]",
        total / ROUNDS as f64,
        best / ROWS as f64
    );
}
