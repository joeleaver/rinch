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

/// What the *deleted* hammer cost on the same removal (#704).
///
/// `NodeHandle::clear_animations()` walked the subtree writing
/// `transition: none` and `animation: none` as two separate `set_style` calls
/// per node, and `rinch-dom`'s `set_style` is not a field write: it re-merges
/// the node's whole inline `style` string, re-parses it into a Stylo
/// declaration block, and invalidates the node's inline style. Five call sites
/// ran it immediately before `remove()`.
///
/// Unlike [`removing_500_rows`] this needs no second binary, because the thing
/// being measured is an extra call at the **caller**, not a change inside
/// `remove_node` — so both arms run in one process, alternated round by round,
/// which removes the build-to-build drift that harness has to live with.
///
/// # Measured
///
/// Release, best of 200 alternated rounds, the same 500 rows of 3 nodes. Two
/// runs of the one binary, on a host with other work on it:
///
/// | run | `remove_node` alone | stamp, then `remove_node` |
/// |---|---|---|
/// | 1 | 343.2us (0.686us/row) | 5559.1us (11.118us/row) |
/// | 2 | 340.1us (0.680us/row) | 5541.2us (11.082us/row) |
///
/// **About 16x**, or +10.4us per row over 3 nodes — roughly 3.5us per node for
/// the two `set_style` calls, against the ~11ns per node the #699 reset costs,
/// which is a factor of 300. That is the difference between re-merging and
/// re-parsing a declaration block twice and writing a bool. Correctness was the
/// reason to delete the call; this is why a deprecated no-op shim would have
/// been the wrong shape too, and why `set_style` is the wrong tool for anything
/// a removal path wants to do.
///
/// The `stamped` control is printed for the same reason the other harness
/// prints its own: it says the stamping arm actually stamped.
#[test]
#[ignore = "timing harness"]
fn stamping_transition_none_before_removing_500_rows() {
    const ROWS: usize = 500;
    const ROUNDS: usize = 200;

    /// `NodeHandle::clear_animations()` as it stood at `main` `c717500`.
    fn stamp_none(doc: &mut RinchDocument, node: rinch_core::dom::NodeId) {
        let mut stack = vec![node.0];
        while let Some(id) = stack.pop() {
            doc.set_style(rinch_core::dom::NodeId(id), "transition", "none");
            doc.set_style(rinch_core::dom::NodeId(id), "animation", "none");
            stack.extend(doc.tree.nodes[id].children.iter().copied());
        }
    }

    let mut best = [f64::MAX; 2];
    let mut stamped = false;
    for _ in 0..ROUNDS {
        for (arm, stamping) in [false, true].into_iter().enumerate() {
            let (mut doc, rows) = built_list(ROWS);
            let t0 = std::time::Instant::now();
            for row in &rows {
                if stamping {
                    stamp_none(&mut doc, *row);
                }
                doc.remove_node(*row);
            }
            let us = t0.elapsed().as_secs_f64() * 1e6;
            best[arm] = best[arm].min(us);
            if stamping {
                let deep = doc.tree.get(rows[0].0).unwrap().children[0];
                stamped = doc.tree.nodes[deep]
                    .attributes
                    .get("style")
                    .is_some_and(|s| s.contains("transition: none"));
            }
        }
    }
    println!(
        "remove_node x {ROWS}: plain best {:.1}us ({:.3}us/row), stamped best \
         {:.1}us ({:.3}us/row) [deep node stamped: {stamped}]",
        best[0],
        best[0] / ROWS as f64,
        best[1],
        best[1] / ROWS as f64
    );
}
