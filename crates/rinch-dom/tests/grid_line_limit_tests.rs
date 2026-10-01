//! Issue #1210: Taffy 0.12 numbers grid lines as `i16`, and a grid whose
//! placement reached past line 32767 **panicked** desktop layout (an overflow
//! in debug, an index or `unwrap` panic in release) — reachable from ordinary
//! markup, and from the editor, whose tables are grids.
//!
//! Every shape below panicked at HEAD before the fix (each is a route through a
//! different part of Taffy's placement), and now lays out. A grid that could
//! reach past the limit is laid out as a row-wrapping flex container
//! (`rinch_dom::grid_budget`); the numbers asserted are Chrome 153's where the
//! degraded layout matches it, and say so where it does not.
//!
//! The grids are built with `rinch_dom`'s own API rather than parsed HTML, so
//! each case is one container and its items and nothing else.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

fn doc_with_grid(grid_style: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let g = doc.create_element("div");
    doc.set_attribute(g, "style", grid_style);
    doc.append_child(body, g);
    (doc, g)
}

fn add(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let c = doc.create_element("div");
    doc.set_attribute(c, "style", style);
    doc.append_child(parent, c);
    c
}

/// A grid of `items` children, laid out at 800x600. The grid is filled
/// before it is attached: appending to a connected parent costs time
/// quadratic in its child count (tens of seconds for 32768 children in a
/// release build), filling a detached one does not.
fn grid(grid_style: &str, items: &[&str]) -> (RinchDocument, NodeId, Vec<NodeId>) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let g = doc.create_element("div");
    doc.set_attribute(g, "style", grid_style);
    let kids = items.iter().map(|s| add(&mut doc, g, s)).collect();
    doc.append_child(body, g);
    doc.resolve_layout(VW, VH);
    (doc, g, kids)
}

fn rect(doc: &RinchDocument, id: NodeId) -> (f32, f32, f32, f32) {
    let l = &doc.tree.get(id.0).unwrap().layout;
    (l.x, l.y, l.width, l.height)
}

fn degraded(doc: &RinchDocument, id: NodeId) -> Option<u32> {
    doc.tree.get(id.0).unwrap().grid_degrade
}

/// The issue's first row: 40000 auto-placed rows in one 100px column. Chrome
/// lays it out 400000px tall, each item 100px wide — which is what the
/// degraded layout gives: one item per line, at the column's width.
#[test]
fn forty_thousand_auto_placed_rows_lay_out_as_chrome_does() {
    let items = vec!["height: 10px"; 40_000];
    let (doc, g, kids) = grid("display: grid; grid-template-columns: 100px", &items);
    assert_eq!(degraded(&doc, g), Some(1));
    assert_eq!(rect(&doc, g).3, 400_000.0);
    assert_eq!(rect(&doc, kids[1]), (0.0, 10.0, 100.0, 10.0));
    assert_eq!(rect(&doc, kids[39_999]), (0.0, 399_990.0, 100.0, 10.0));
}

/// The boundary, measured on a raw `TaffyTree`: 32767 auto rows lay out as a
/// grid and 32768 panicked. The bound is exact here — one line per item in
/// one column — so the first is still handed to Taffy as a grid.
#[test]
fn the_last_row_count_taffy_numbers_is_still_a_grid() {
    for (n, want) in [(32_767, None), (32_768, Some(1))] {
        let items = vec!["height: 10px"; n];
        let (doc, g, kids) = grid("display: grid; grid-template-columns: 100px", &items);
        assert_eq!(degraded(&doc, g), want, "{n} rows");
        assert_eq!(
            rect(&doc, kids[n - 1]),
            (0.0, 10.0 * (n - 1) as f32, 100.0, 10.0),
            "{n} rows"
        );
    }
}

/// 33 items of `span 1000` (33000 rows) in one column: Chrome 330px tall,
/// items stacked 10px apart.
#[test]
fn thirty_three_stacked_spans_of_1000_lay_out() {
    let items = vec!["grid-row: span 1000; height: 10px"; 33];
    let (doc, g, kids) = grid("display: grid; grid-template-columns: 100px", &items);
    assert!(degraded(&doc, g).is_some());
    assert_eq!(rect(&doc, g).3, 330.0);
    assert_eq!(rect(&doc, kids[3]), (0.0, 30.0, 100.0, 10.0));
}

/// The debug-build row of the issue: four items of `span 10000`. Chrome 40px.
#[test]
fn four_spans_of_10000_lay_out() {
    let items = vec!["grid-row: span 10000; height: 10px"; 4];
    let (doc, g, kids) = grid("display: grid; grid-template-columns: 100px", &items);
    assert_eq!(rect(&doc, g).3, 40.0);
    assert_eq!(rect(&doc, kids[3]), (0.0, 30.0, 100.0, 10.0));
}

/// A template of 40000 explicit columns (Stylo caps a `repeat()` count at
/// 10000, not the tracks it repeats). Chrome puts the item in the first 1px
/// column; the degraded layout shares the 800px width between 40000 columns,
/// so it is 0.02px wide — one of the track sizes it gives up.
#[test]
fn a_template_past_the_limit_lays_out() {
    let (doc, g, kids) = grid(
        "display: grid; grid-template-columns: repeat(10000, 1px 1px 1px 1px)",
        &["height: 5px"],
    );
    assert_eq!(degraded(&doc, g), Some(40_000));
    let (x, y, _, h) = rect(&doc, kids[0]);
    assert_eq!((x, y, h), (0.0, 0.0, 5.0));
}

/// Definite lines far on both sides of the explicit grid: 20000 negative
/// implicit rows and 20000 positive ones — each within `i16`, their sum not.
/// Chrome stacks the two items, 20px in all.
#[test]
fn definite_lines_on_both_sides_lay_out() {
    let (doc, g, kids) = grid(
        "display: grid; grid-template-columns: 100px",
        &[
            "grid-row: span 10000 / -10000; height: 10px",
            "grid-row: 10000 / span 10000; height: 10px",
        ],
    );
    assert!(degraded(&doc, g).is_some());
    assert_eq!(rect(&doc, g).3, 20.0);
    assert_eq!(rect(&doc, kids[1]), (0.0, 10.0, 100.0, 10.0));
}

/// Items locked to one row grow the *cross* axis while they look for a free
/// column: four of `span 10000` in row 1 need 40000 columns. (Chrome puts
/// them side by side in one 10px row; the degraded layout, one 100px column
/// wide, stacks them.)
#[test]
fn items_locked_to_a_row_that_outgrow_the_columns_lay_out() {
    let items = vec!["grid-row: 1; grid-column: span 10000; height: 10px"; 4];
    let (doc, g, _) = grid("display: grid; grid-template-columns: 100px", &items);
    assert!(degraded(&doc, g).is_some());
    assert_eq!(rect(&doc, g).3, 40.0);
}

/// Taffy 0.12's own route past the limit, found by the differential below
/// (its case 291): step 2 of placement starts an item locked to a row at the
/// last auto-placed cell in that row, and converts that cell's *column* index
/// back into a line with the *row* axis' negative implicit count. With 19999
/// negative implicit columns and none in rows, `b` is searched for 19999
/// columns past where it belongs — line 39999, past `i16` (a debug overflow
/// panic; in release, wrapped arithmetic until the cell matrix's assertion).
/// Measured by CSS the grid is 20002 columns wide, so a bound that only
/// follows CSS hands it to Taffy. (Chrome: all four in one 10px row.)
#[test]
fn a_locked_items_misplaced_search_start_is_counted() {
    let (doc, g, kids) = grid(
        "display: grid",
        &[
            "grid-row: 1; grid-column: span 10000 / -10000; height: 10px",
            "grid-row: 1; grid-column: -10000 / span 10000; height: 10px",
            "grid-row: 1; height: 10px",
            "grid-row: 1; height: 10px",
        ],
    );
    assert_eq!(degraded(&doc, g), Some(1));
    assert_eq!(rect(&doc, g).3, 40.0);
    assert_eq!(rect(&doc, kids[3]), (0.0, 30.0, 800.0, 10.0));
}

/// The degraded layout of a grid of equal `fr` columns — the editor's table
/// shape — keeps its columns: items fill rows of two, each 400px wide, and a
/// row is as tall as its tallest item. The grid is pushed over the limit by
/// its explicit rows, whose sizes the degraded layout gives up: Chrome puts
/// the second row at y=1 (the first explicit row is 1px tall and the tall
/// item overflows it), rinch at the tall item's bottom. (The tall item is
/// 30px here where Chrome's is 38: rinch lays every box out border-box, grid
/// or not.)
#[test]
fn a_degraded_grid_of_equal_columns_keeps_its_columns() {
    let (doc, g, kids) = grid(
        "display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); \
         grid-template-rows: repeat(10000, 1px 1px 1px 1px)",
        &[
            "height: 30px; padding: 4px",
            "height: 10px",
            "height: 5px",
            "",
        ],
    );
    assert_eq!(degraded(&doc, g), Some(2));
    assert_eq!(rect(&doc, kids[0]), (0.0, 0.0, 400.0, 30.0));
    assert_eq!(rect(&doc, kids[1]).0, 400.0);
    assert_eq!(rect(&doc, kids[1]).2, 400.0);
    assert_eq!(rect(&doc, kids[2]), (0.0, 30.0, 400.0, 5.0));
    assert_eq!(rect(&doc, kids[3]).0, 400.0);
}

/// The rest of the degraded wrap: a `grid-column: span 2` item takes a whole
/// line; the column gap is dropped (with it, two half-width items would not
/// fit a line); and an item whose content is wider than its column keeps its
/// column's width, as in a `minmax(0, 1fr)` grid, rather than growing to the
/// content's.
#[test]
fn a_degraded_grid_spans_columns_drops_the_column_gap_and_keeps_widths() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let g = doc.create_element("div");
    doc.set_attribute(
        g,
        "style",
        "display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); \
         column-gap: 10px; row-gap: 3px; grid-template-rows: repeat(10000, 1px 1px 1px 1px)",
    );
    let wide = add(&mut doc, g, "height: 10px");
    add(&mut doc, wide, "width: 500px; height: 10px");
    let beside = add(&mut doc, g, "height: 10px");
    let spanning = add(&mut doc, g, "grid-column: span 2; height: 10px");
    let after = add(&mut doc, g, "height: 10px");
    doc.append_child(body, g);
    doc.resolve_layout(VW, VH);
    assert_eq!(degraded(&doc, g), Some(2));
    assert_eq!(rect(&doc, wide), (0.0, 0.0, 400.0, 10.0));
    assert_eq!(rect(&doc, beside), (400.0, 0.0, 400.0, 10.0));
    assert_eq!(rect(&doc, spanning), (0.0, 13.0, 800.0, 10.0));
    assert_eq!(rect(&doc, after), (0.0, 26.0, 400.0, 10.0));
}

/// The control: a grid whose bound fits is still handed to Taffy as a grid.
/// Three `span 10000` items and one of `span 2000` reach line 32004 — under
/// the limit even with the bound's per-item slack.
#[test]
fn a_grid_under_the_limit_is_still_a_grid() {
    let (doc, g, kids) = grid(
        "display: grid; grid-template-columns: repeat(2, 100px); column-gap: 7px",
        &[
            "grid-row: span 10000; height: 10px",
            "grid-row: span 10000; height: 10px",
            "grid-row: span 10000; height: 10px",
            "grid-row: span 2000; height: 10px",
        ],
    );
    assert_eq!(degraded(&doc, g), None);
    // A grid keeps its column gap; the degraded layout would drop it.
    assert_eq!(rect(&doc, kids[1]).0, 107.0);
}

/// 1x1 auto items are counted a row per explicit column, not a row each —
/// what keeps a three-column table of 12000 rows a grid. Here 2765 of them in
/// three columns (922 rows) below three `span 10000` items stay under the
/// limit; counted a row each they would not.
#[test]
fn one_by_one_items_are_counted_a_row_per_column() {
    let mut items = vec!["grid-row: span 10000"; 3];
    items.extend(std::iter::repeat_n("height: 1px", 2_765));
    let (doc, g, kids) = grid(
        "display: grid; grid-template-columns: repeat(3, 10px)",
        &items,
    );
    assert_eq!(degraded(&doc, g), None);
    // In the grid the 1x1 items fill rows of three 10px columns below the
    // spanning ones; the degraded layout would make each a third of 800px.
    assert_eq!(rect(&doc, kids[4]).0, 10.0);
    assert_eq!(rect(&doc, kids[4]).2, 10.0);
}

/// A grid laid out small, then given 40000 more items — a `for`'s
/// `display: contents` wrapper, appended once — with no restyle of the
/// container, is re-checked; and is a grid again once they are gone.
#[test]
fn appending_past_the_limit_and_removing_back_under_it() {
    let (mut doc, g) = doc_with_grid("display: grid; grid-template-columns: 100px 100px");
    let first = add(&mut doc, g, "height: 10px");
    let second = add(&mut doc, g, "height: 10px");
    doc.resolve_layout(VW, VH);
    assert_eq!(degraded(&doc, g), None);
    let wrapper = doc.create_element("div");
    doc.set_attribute(wrapper, "style", "display: contents");
    for _ in 0..70_000 {
        add(&mut doc, wrapper, "height: 10px");
    }
    doc.append_child(g, wrapper);
    doc.resolve_layout(VW, VH + 1.0);
    assert_eq!(degraded(&doc, g), Some(2));
    // Rows of two, each item half the width: 35001 of them.
    assert_eq!(rect(&doc, second), (400.0, 0.0, 400.0, 10.0));
    assert_eq!(rect(&doc, g).3, 350_010.0);
    doc.remove_node(wrapper);
    doc.resolve_layout(VW, VH);
    assert_eq!(degraded(&doc, g), None);
    assert_eq!(rect(&doc, first), (0.0, 0.0, 100.0, 10.0));
    assert_eq!(rect(&doc, second), (100.0, 0.0, 100.0, 10.0));
    assert_eq!(rect(&doc, g).3, 10.0);
}

/// An item restyled into a span that takes its grid past the limit re-checks
/// the grid, with no change to the container or its child list.
#[test]
fn an_item_restyled_past_the_limit_rechecks_its_grid() {
    let (mut doc, g, kids) = grid(
        "display: grid; grid-template-columns: 100px",
        &["grid-row: span 10000; height: 10px"; 4],
    );
    assert!(degraded(&doc, g).is_some());
    for &k in &kids[1..] {
        doc.set_attribute(k, "style", "height: 10px");
    }
    doc.resolve_layout(VW, VH);
    assert_eq!(degraded(&doc, g), None);
    for &k in &kids[1..] {
        doc.set_attribute(k, "style", "grid-row: span 10000; height: 10px");
    }
    doc.resolve_layout(VW, VH + 1.0);
    assert!(degraded(&doc, g).is_some());
    assert_eq!(rect(&doc, g).3, 40.0);
}

/// The editor's table shape — `<tr>` is `display: contents`, so the cells are
/// the grid's items, in `repeat(N, minmax(0, 1fr))` — counted through the
/// wrappers. 33000 rows of two cells lay out as a table would: rows of two
/// equal cells, stacked.
#[test]
fn a_table_of_33000_rows_lays_out_as_a_table() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let g = doc.create_element("div");
    doc.set_attribute(
        g,
        "style",
        "display: grid; grid-template-columns: repeat(2, minmax(0, 1fr))",
    );
    let mut cells = Vec::new();
    for _ in 0..33_000 {
        let tr = add(&mut doc, g, "display: contents");
        cells.push(add(&mut doc, tr, "height: 10px"));
        cells.push(add(&mut doc, tr, "height: 20px"));
    }
    doc.append_child(body, g);
    doc.resolve_layout(VW, VH);
    assert_eq!(degraded(&doc, g), Some(2));
    assert_eq!(rect(&doc, g).3, 660_000.0);
    assert_eq!(rect(&doc, cells[2]), (0.0, 20.0, 400.0, 10.0));
    assert_eq!(rect(&doc, cells[65_999]), (400.0, 659_980.0, 400.0, 20.0));
}

/// …and one of 16000 rows is a grid, as it always was: the bound counts a
/// row per two cells, not a row per cell.
#[test]
fn a_table_of_16000_rows_is_still_a_grid() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let g = doc.create_element("div");
    doc.set_attribute(
        g,
        "style",
        "display: grid; grid-template-columns: repeat(2, 100px)",
    );
    let mut cells = Vec::new();
    for _ in 0..16_000 {
        let tr = add(&mut doc, g, "display: contents");
        cells.push(add(&mut doc, tr, "height: 10px"));
        cells.push(add(&mut doc, tr, "height: 10px"));
    }
    doc.append_child(body, g);
    doc.resolve_layout(VW, VH);
    assert_eq!(degraded(&doc, g), None);
    assert_eq!(rect(&doc, cells[31_999]).0, 100.0);
    assert_eq!(rect(&doc, cells[31_999]).2, 100.0);
}

/// Cells spanning rows behind `display: contents` wrappers are counted too.
#[test]
fn spanning_items_behind_display_contents_are_counted() {
    let (mut doc, g) =
        doc_with_grid("display: grid; grid-template-columns: repeat(2, minmax(0, 1fr))");
    let mut cells = Vec::new();
    for _ in 0..2 {
        let tr = add(&mut doc, g, "display: contents");
        cells.push(add(&mut doc, tr, "grid-row: span 10000; height: 10px"));
        cells.push(add(&mut doc, tr, "grid-row: span 10000; height: 10px"));
    }
    doc.resolve_layout(VW, VH);
    assert_eq!(degraded(&doc, g), Some(2));
    assert_eq!(rect(&doc, cells[3]), (400.0, 10.0, 400.0, 10.0));
}

/// A seeded random differential: grids of every placement kind — spans and
/// lines up to Stylo's ±10000, named lines (rinch declares none, so they
/// resolve past the explicit grid), explicit templates, every flow — laid
/// out whole. The oracle is Taffy itself: a grid the bound passes is handed
/// to Taffy as a grid, and one that reaches past line 32767 panics there. The
/// positive control: grids whose bound is within a few thousand lines of the
/// limit, still handed over, are counted and must occur.
///
/// Ignored by default: a grid of thousands of tracks is slow to lay out
/// (about half a second a case in a release build, ten times that in debug).
/// Run it with `cargo test --release -p rinch-dom --test grid_line_limit_tests
/// -- --ignored`; `RINCH_GRID_FUZZ_CASES=n` sets the count (default 300).
#[test]
#[ignore = "slow: run with --release -- --ignored"]
fn random_grids_near_the_limit_never_panic() {
    let cases: u64 = std::env::var("RINCH_GRID_FUZZ_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    let from: u64 = std::env::var("RINCH_GRID_FUZZ_FROM")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let trace = std::env::var("GRID_FUZZ_TRACE").is_ok();
    let mut seed = 0x1210_u64;
    let mut next = move |n: u64| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) % n
    };
    let (mut near, mut degraded_cases) = (0, 0);
    for case in 0..cases {
        let flow = ["row", "row dense", "column", "column dense"][next(4) as usize];
        let template = |next: &mut dyn FnMut(u64) -> u64| match next(4) {
            0 => String::from("none"),
            1 => format!("repeat({}, 10px)", 1 + next(4)),
            2 => format!("repeat({}, 1px 2px)", 1 + next(10_000)),
            _ => format!("repeat({}, 1px)", 1 + next(10_000)),
        };
        let cols = template(&mut next);
        let rows = template(&mut next);
        // Dense packing with large spans is the shape Taffy 0.12's placement
        // search is pathologically slow on (one such case took 98s in a
        // release build — upstream's #1013, fixed in 0.13); dense grids keep
        // small spans here.
        let dense = flow.ends_with("dense");
        let big = |next: &mut dyn FnMut(u64) -> u64| {
            if dense || next(2) == 0 {
                1 + next(10)
            } else {
                1 + next(10_000)
            }
        };
        let placement = |next: &mut dyn FnMut(u64) -> u64| -> String {
            match next(7) {
                0 | 1 => String::from("auto"),
                2 => format!("span {}", big(next)),
                3 => {
                    let n = big(next) as i64 * if next(2) == 0 { 1 } else { -1 };
                    format!("{n}")
                }
                4 => format!("{} / span {}", 1 + next(10_000), big(next)),
                5 => format!("span {} / {}", big(next), -1 - next(10_000) as i64),
                _ => format!("foo {} / span bar {}", 1 + next(50), 1 + next(50)),
            }
        };
        let mut items: Vec<String> = (0..1 + next(12))
            .map(|_| {
                format!(
                    "grid-row: {}; grid-column: {}; height: 1px",
                    placement(&mut next),
                    placement(&mut next)
                )
            })
            .collect();
        if next(3) == 0 {
            items.extend((0..next(6_000)).map(|_| String::from("height: 1px")));
        }
        let style = format!(
            "display: grid; grid-auto-flow: {flow}; \
             grid-template-columns: {cols}; grid-template-rows: {rows}"
        );
        if case < from {
            continue;
        }
        if trace {
            eprintln!(
                "case {case} start: {style} items: {:?}",
                &items[..items.len().min(13)]
            );
        }
        let refs: Vec<&str> = items.iter().map(String::as_str).collect();
        let t0 = std::time::Instant::now();
        let (doc, g, _) = grid(&style, &refs);
        if trace {
            eprintln!(
                "case {case} {:?} degraded={:?} {style} items={}",
                t0.elapsed(),
                degraded(&doc, g),
                items.len()
            );
        }
        let (f, x) = rinch_dom::grid_budget::line_bounds(&doc.tree, g.0).unwrap();
        let limit = rinch_dom::grid_budget::GRID_LINE_LIMIT;
        match degraded(&doc, g) {
            Some(_) => degraded_cases += 1,
            None if f.max(x) > limit - 5_000 => near += 1,
            None => {}
        }
        assert!(
            degraded(&doc, g).is_some() || f.max(x) <= limit,
            "case {case}: {style}"
        );
    }
    eprintln!(
        "{cases} grids: {degraded_cases} degraded, {near} handed over within 5000 lines of the limit"
    );
    assert!(near >= cases / 100, "only {near} grids near the limit");
    assert!(
        degraded_cases >= cases / 20,
        "only {degraded_cases} degraded"
    );
}
