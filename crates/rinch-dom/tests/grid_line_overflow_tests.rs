//! #1210 and #1231 — CSS grids whose line count reaches past what Taffy 0.12
//! could number. Taffy 0.12 kept grid lines in an `i16`, so each of these
//! **panicked** inside `resolve_layout` (the panic site is named per fixture).
//! Taffy 0.14 (#1236) clamps every grid axis at 10,000 tracks (`MAX_GRID_TRACKS`,
//! css-grid-1 §7.2.4 "overlarge grids") and every route here lays out.
//!
//! Each fixture asserts two things: that layout returns, and **what it lays
//! out** — so this file is also the record of where Taffy's clamp and Chrome
//! disagree. The clamp is not Chrome's geometry: Chrome 153 lays a 40,000-row
//! one-column list out 400,000px tall (measured in #1210); Taffy puts every
//! item past the 10,000th track into the last one, where they overlap.
//!
//! Every size here is declared (`height: 10px` items, declared container
//! sizes), and no item holds text, so nothing pins the local font set.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

/// `(x, y, width, height)` of a node's layout box.
type Rect = (f32, f32, f32, f32);

/// A grid styled `grid_style` holding `items` children, each styled
/// `item_style`, laid out at 800x600. Returns the document, the grid, and the
/// items.
fn grid(grid_style: &str, item_style: &str, items: usize) -> (RinchDocument, NodeId, Vec<NodeId>) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let g = doc.create_element("div");
    doc.set_attribute(g, "style", &format!("display: grid; {grid_style}"));
    doc.append_child(body, g);
    let mut ids = Vec::with_capacity(items);
    for _ in 0..items {
        let c = doc.create_element("div");
        doc.set_attribute(c, "style", item_style);
        doc.append_child(g, c);
        ids.push(c);
    }
    doc.resolve_layout(VW, VH);
    (doc, g, ids)
}

fn rect(doc: &RinchDocument, id: NodeId) -> Rect {
    let l = &doc.tree.get(id.0).unwrap().layout;
    (l.x, l.y, l.width, l.height)
}

// ---------------------------------------------------------------------------
// #1210 — the implicit grid grows past 32,767 lines
// ---------------------------------------------------------------------------

/// #1210's headline: one column, 40,000 auto-placed 10px rows. Taffy 0.12
/// panicked at `cell_occupancy.rs:166`.
///
/// Taffy 0.14 makes 10,000 row tracks. Items 1..=9,999 take a track each, and
/// the other 30,001 all land in the last one, which they share — so the grid is
/// 10,000 tracks of 10px, and every item from the 10,000th on sits at the same
/// `y`. Chrome 153: 400,000px tall, item `n` at `y = 10 n`.
///
/// Ignored by default for its cost, which is not the grid's: building and
/// laying out 40,000 siblings of one parent takes minutes in a debug build
/// whatever the parent's `display` (a flex column or a block of 36,000 rows
/// costs the same order in release, #1247). Run it with `--ignored`; the
/// routes past 32,767 lines that CI runs are the span fixtures below, and the
/// clamp itself is `ten_and_a_half_thousand_rows_are_now_clamped_too`.
#[test]
#[ignore = "40,000 siblings: minutes in a debug build (rinch-side, not Taffy: #1247)"]
fn forty_thousand_auto_placed_rows_lay_out_clamped_at_ten_thousand_tracks() {
    let (doc, g, items) = grid("grid-template-columns: 100px", "height: 10px", 40_000);
    let (_, _, w, h) = rect(&doc, g);
    assert_eq!(w, VW, "the grid is a block-level box filling the body");
    assert_eq!(
        h, 100_000.0,
        "10,000 tracks of 10px; Chrome 153 gives 400,000"
    );

    // Off the fixed point: an early item is placed exactly as Chrome places it…
    assert_eq!(rect(&doc, items[1234]).1, 12_340.0);
    // …the 10,000th item opens the last track…
    let last_track_y = 99_990.0;
    assert_eq!(rect(&doc, items[9_999]).1, last_track_y);
    // …and every item after it piles into that track (Chrome: 10 n).
    for n in [10_000, 20_000, 39_999] {
        assert_eq!(
            rect(&doc, items[n]).1,
            last_track_y,
            "item {n} overlaps in the last track; Chrome 153 puts it at {}",
            10.0 * n as f32
        );
    }
}

/// The new regression band the clamp opens: a grid of 10,001–32,767 lines lays
/// out correctly on Taffy 0.12 and overlaps its last track on 0.14. 10,500
/// rows: Chrome (and 0.12) gives 105,000px; 0.14 gives 100,000px, and the last
/// 501 items share the last track.
#[test]
fn ten_and_a_half_thousand_rows_are_now_clamped_too() {
    let (doc, g, items) = grid("grid-template-columns: 100px", "height: 10px", 10_500);
    assert_eq!(
        rect(&doc, g).3,
        100_000.0,
        "Chrome 153 and Taffy 0.12: 105,000"
    );
    assert_eq!(
        rect(&doc, items[9_998]).1,
        99_980.0,
        "below the clamp: as Chrome"
    );
    assert_eq!(rect(&doc, items[10_499]).1, 99_990.0, "Chrome: 104,990");
}

/// #1210's second route: 33 items of `grid-row: span 1000` stacked in one
/// column need 33,000 lines. Taffy 0.12 panicked — at `coordinates.rs:94` in
/// the release build #1210 measured, at `coordinates.rs:72` (an `i16`
/// subtraction overflow) in the debug build this test runs in; 32 items laid
/// out. On 0.14 the implicit grid stops at 10,000 rows: the first
/// nine items take 1,000 rows each, and items 9 through 32 all start at row
/// 9,001 and overlap there (y 90 — each 1,000-row band holds one 10px item).
/// Chrome 153 stacks all 33.
#[test]
fn thirty_three_stacked_thousand_row_spans_lay_out() {
    let (doc, g, items) = grid(
        "grid-template-columns: 100px",
        "grid-row: span 1000; height: 10px",
        33,
    );
    let h = rect(&doc, g).3;
    assert!(h.is_finite() && h > 0.0, "laid out: {h}");
    let ys: Vec<f32> = items.iter().map(|&i| rect(&doc, i).1).collect();
    assert_eq!(rect(&doc, g).3, h);
    assert_eq!((h, ys[0], ys[1], ys[32]), RECORDED_SPAN_1000);
}

/// What 0.14 lays `thirty_three_stacked_thousand_row_spans_lay_out` out as:
/// (grid height, item 0's y, item 1's y, item 32's y).
const RECORDED_SPAN_1000: (f32, f32, f32, f32) = (100.0, 0.0, 10.0, 90.0);

/// #1210's debug-build route: four `span 10000` items overflowed the `i16`
/// subtraction at `coordinates.rs:72`. Spans are clamped to 10,000 on 0.14, so
/// all four items span the whole clamped grid and overlap at y 0.
#[test]
fn four_ten_thousand_row_spans_lay_out() {
    let (doc, g, items) = grid(
        "grid-template-columns: 100px",
        "grid-row: span 10000; height: 10px",
        4,
    );
    let h = rect(&doc, g).3;
    let ys: Vec<f32> = items.iter().map(|&i| rect(&doc, i).1).collect();
    assert_eq!((h, ys[0], ys[1], ys[3]), RECORDED_SPAN_10000);
}

const RECORDED_SPAN_10000: (f32, f32, f32, f32) = (10.0, 0.0, 0.0, 0.0);

// ---------------------------------------------------------------------------
// #1231 — repeat(auto-fill | auto-fit, …)
// ---------------------------------------------------------------------------

/// A 1px column repeated across a 40,000px grid: 40,000 columns, past `i16`.
/// Taffy 0.12 panicked at `coordinates.rs:133`. 0.14 clamps the repetition at
/// 10,000, so the one item still sits in the first 1px column.
#[test]
fn auto_fill_one_px_columns_across_forty_thousand_px_lay_out() {
    let (doc, g, items) = grid(
        "width: 40000px; grid-template-columns: repeat(auto-fill, 1px)",
        "height: 1px",
        1,
    );
    assert_eq!(rect(&doc, g).2, 40_000.0);
    assert_eq!(rect(&doc, items[0]), RECORDED_AUTO_FILL_COLS);
}

const RECORDED_AUTO_FILL_COLS: (f32, f32, f32, f32) = (0.0, 0.0, 1.0, 1.0);

/// The same along the block axis, in a 40,000px-tall grid.
#[test]
fn auto_fill_one_px_rows_down_forty_thousand_px_lay_out() {
    let (doc, g, items) = grid(
        "height: 40000px; grid-template-rows: repeat(auto-fill, 1px)",
        "height: 1px",
        1,
    );
    assert_eq!(rect(&doc, g).3, 40_000.0);
    assert_eq!(rect(&doc, items[0]), RECORDED_AUTO_FILL_ROWS);
}

const RECORDED_AUTO_FILL_ROWS: (f32, f32, f32, f32) = (0.0, 0.0, 800.0, 1.0);

/// The sharp one: an ordinary 800px grid, `repeat(auto-fill, 0px)`. A
/// zero-size repetition divided by zero and Taffy 0.12 overflowed a `u16` at
/// `explicit_grid.rs:172`. Chrome floors the repetition at 1px for the count
/// (css-grid-1 §7.2.3.2), giving 800 columns.
#[test]
fn auto_fill_zero_px_columns_at_800_lay_out() {
    let (doc, g, items) = grid(
        "grid-template-columns: repeat(auto-fill, 0px)",
        "height: 10px",
        2,
    );
    assert_eq!(rect(&doc, g).2, VW);
    assert_eq!(
        (rect(&doc, items[0]), rect(&doc, items[1])),
        RECORDED_AUTO_FILL_ZERO
    );
}

const RECORDED_AUTO_FILL_ZERO: (Rect, Rect) = ((0.0, 0.0, 0.0, 10.0), (0.0, 0.0, 0.0, 10.0));

/// Valid CSS that panicked the same way: `minmax(0px, 1fr)` is a
/// `<fixed-size>`, so it may be auto-repeated, and its fixed breadth is 0.
#[test]
fn auto_fill_minmax_zero_one_fr_at_800_lays_out() {
    let (doc, g, items) = grid(
        "grid-template-columns: repeat(auto-fill, minmax(0px, 1fr))",
        "height: 10px",
        2,
    );
    assert_eq!(rect(&doc, g).2, VW);
    assert_eq!(
        (rect(&doc, items[0]), rect(&doc, items[1])),
        RECORDED_AUTO_FILL_MINMAX
    );
}

const RECORDED_AUTO_FILL_MINMAX: (Rect, Rect) = ((0.0, 0.0, 0.0, 10.0), (0.0, 0.0, 0.0, 10.0));

/// `auto-fit` collapses the empty repetitions `auto-fill` keeps, and is
/// reached by the same count computation.
#[test]
fn auto_fit_zero_px_columns_at_800_lay_out() {
    let (doc, g, items) = grid(
        "grid-template-columns: repeat(auto-fit, 0px)",
        "height: 10px",
        2,
    );
    assert_eq!(rect(&doc, g).2, VW);
    assert_eq!(
        (rect(&doc, items[0]), rect(&doc, items[1])),
        RECORDED_AUTO_FIT_ZERO
    );
}

const RECORDED_AUTO_FIT_ZERO: (Rect, Rect) = ((0.0, 0.0, 0.0, 10.0), (0.0, 0.0, 0.0, 10.0));
