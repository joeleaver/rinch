//! #1247 — building N siblings of one parent and laying them out once must
//! cost work linear in N.
//!
//! Before the fix every insertion made **before the document's first layout**
//! walked the whole document's style tree (`resolve_styles` chose its full walk
//! whenever transitions were not yet armed, and the insertion fast path
//! `resolve_inserted_subtree` declined for the same reason). Building N rows
//! visited about N²/2 nodes: 36,000 rows took 40–70 s in release.
//!
//! These fixtures count operations, not time. Each builds the same document at
//! two sizes, 1,000 and 4,000 rows, lays it out once, and asserts that **no
//! counter** grows by more than the 4x the rows did. Counters are
//! deterministic, so the check is exact and holds under any load. At the
//! unfixed code `style_nodes_visited` grows 16x (and `full_style_walks`, one
//! per insertion, 4x — linear in count but each walk O(N)).
//!
//! Every size is declared and no element holds text, so nothing here depends
//! on the host's fonts.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::perf::{Counter, FrameStats};

const SMALL: usize = 1_000;
const LARGE: usize = 4_000;

/// How each row is put into the document.
#[derive(Clone, Copy)]
enum Shape {
    /// Styled while detached, then appended — the issue's probe.
    StyleThenAppend,
    /// Appended, styled while attached (a pending snapshot on the row), then
    /// given a child: the child's insertion finds an ancestor waiting for a
    /// restyle and falls back to `resolve_styles`.
    AppendThenStyleThenChild,
}

/// Build `rows` rows under a parent styled `parent_style` and lay the document
/// out once; answer every counter the build and the layout incremented.
fn build_and_layout(parent_style: &str, shape: Shape, rows: usize) -> FrameStats {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let parent = doc.create_element("div");
    doc.set_attribute(parent, "style", parent_style);
    doc.append_child(body, parent);
    doc.tree.perf.reset();
    for _ in 0..rows {
        let row = doc.create_element("div");
        match shape {
            Shape::StyleThenAppend => {
                doc.set_attribute(row, "style", "height: 10px");
                doc.append_child(parent, row);
            }
            Shape::AppendThenStyleThenChild => {
                doc.append_child(parent, row);
                doc.set_attribute(row, "style", "height: 10px");
                let child = doc.create_element("span");
                doc.append_child(row, child);
            }
        }
    }
    doc.resolve_layout(800.0, 600.0);
    let stats = doc.tree.perf.end_frame();

    // The document did lay out: the last row sits where N rows of 10px put it
    // (block and flex column stack them; the one-column grid has a track per
    // row, all under Taffy's 10,000-track clamp).
    let last = *doc.tree.get(parent.0).unwrap().children.last().unwrap();
    let y = doc.tree.get(last).unwrap().layout.y;
    assert_eq!(y, 10.0 * (rows - 1) as f32, "{parent_style}: last row's y");
    stats
}

/// Assert no counter grows faster than the row count between [`SMALL`] and
/// [`LARGE`].
#[track_caller]
fn assert_linear(parent_style: &str, shape: Shape) {
    let small = build_and_layout(parent_style, shape, SMALL);
    let large = build_and_layout(parent_style, shape, LARGE);
    let factor = (LARGE / SMALL) as u64;
    let mut superlinear = Vec::new();
    for (c, s) in small.iter() {
        if c.name().starts_with("time_") {
            continue;
        }
        let l = large.get(c);
        if l > factor * s {
            superlinear.push(format!("{}: {s} at {SMALL} rows, {l} at {LARGE}", c.name()));
        }
    }
    assert!(
        superlinear.is_empty(),
        "{parent_style}: counters grew more than {factor}x for {factor}x the rows:\n  {}",
        superlinear.join("\n  ")
    );
    // A positive control: the instrument the bug showed up in did count.
    assert!(small.get(Counter::StyleNodesVisited) >= SMALL as u64);
}

#[test]
fn block_parent_styled_rows_build_linearly() {
    assert_linear("display: block", Shape::StyleThenAppend);
}

#[test]
fn flex_column_parent_styled_rows_build_linearly() {
    assert_linear("display: flex; flex-direction: column", Shape::StyleThenAppend);
}

#[test]
fn grid_parent_styled_rows_build_linearly() {
    assert_linear(
        "display: grid; grid-template-columns: 100px",
        Shape::StyleThenAppend,
    );
}

#[test]
fn block_parent_rows_restyled_while_attached_build_linearly() {
    assert_linear("display: block", Shape::AppendThenStyleThenChild);
}

#[test]
fn flex_column_parent_rows_restyled_while_attached_build_linearly() {
    assert_linear(
        "display: flex; flex-direction: column",
        Shape::AppendThenStyleThenChild,
    );
}
