//! #2598 — a row grows for `vertical-align: baseline`: it is at least as
//! tall as its cells' lowest first baseline plus the deepest extent of a
//! cell of span 1 below its own baseline (CSS 2.1 §17.5.3, Blink's
//! `RowBaselineTabulator`). rinch moved the cells to the row's baseline
//! (#2359) but sized the row from the cells' heights alone, so a lowered
//! cell was moved no further than its leftover height and the rows below
//! sat higher than Chrome's.
//!
//! Every number is Chrome 156's (`--headless=new`, standards mode, the
//! bundled Inter registered as `ProbeFace` through `@font-face`, `16px/20px`
//! unless a cell says otherwise, `body { margin: 0 } td { padding: 0 }
//! table { border-spacing: 0 }`), `getBoundingClientRect` relative to the
//! table. A line's baseline is marked by a 4x10 inline-block standing on it,
//! so a mark is 10px above it. Chrome lays out in 1/64 px and rinch rounds
//! boxes to whole pixels, so a number agrees within half a pixel.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// `vertical-align: baseline`.
const B: &str = "vertical-align:baseline";

fn lay_out(inner: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    doc.load_css(
        "body { margin: 0; font: 16px/20px ProbeFace } td { padding: 0 } \
         table { border-spacing: 0 } i { display: inline-block; width: 4px; height: 10px } \
         .base td { vertical-align: baseline }",
    );
    let body = doc.body();
    doc.set_inner_html(body, inner);
    doc.resolve_layout(800.0, 600.0);
    doc
}

fn one(doc: &RinchDocument, m: &str) -> rinch_dom::node::RawNodeId {
    let found = query_selector(&doc.tree, &format!("[data-m={m}]"));
    assert_eq!(found.len(), 1, "{m}");
    found[0]
}

/// What a check reads of `[data-m=m]`: its `y` from the table's top, or its
/// height.
#[derive(Clone, Copy, Debug)]
enum At {
    Y(&'static str, f32),
    H(&'static str, f32),
}
use At::{H, Y};

fn read(doc: &RinchDocument, at: At) -> f32 {
    let pos = |id| rinch_dom::paint::compute_absolute_position(&doc.tree, id, 1.0);
    match at {
        Y(m, _) => (pos(one(doc, m)).1 - pos(one(doc, "t")).1) as f32,
        H(m, _) => doc.tree.get(one(doc, m)).unwrap().layout.height,
    }
}

#[track_caller]
fn check(case: &str, doc: &RinchDocument, chrome: &[At]) {
    let got: Vec<(At, f32)> = chrome.iter().map(|&a| (a, read(doc, a))).collect();
    let close = got.iter().all(|&(a, g)| {
        let (Y(_, c) | H(_, c)) = a;
        (g - c).abs() <= 0.5
    });
    assert!(close, "{case}: rinch (Chrome's in the At) {got:?}");
}

#[test]
fn the_issues_row_grows_to_the_lowered_cells_bottom_issue_2598() {
    // The first cell is lowered 15px to the second's baseline (20 + 16), and
    // its 30px bottom padding comes below that: 36 + 4 + 30 = 70, not 55.
    let d = lay_out(&format!(
        "<table data-m=t><tr>\
         <td data-m=a style=\"{B};padding:5px 0 30px\">x<i data-m=sa></i></td>\
         <td data-m=b style=\"{B};padding-top:20px\">x<i data-m=sb></i></td>\
         </tr><tr><td data-m=n>y</td></tr></table>"
    ));
    check(
        "issue",
        &d,
        &[
            H("t", 90.0),
            H("a", 70.0),
            H("b", 70.0),
            Y("sa", 26.0),
            Y("sb", 26.0),
            Y("n", 70.0),
        ],
    );
}

#[test]
fn a_big_line_beside_a_padded_one_and_a_middle_cell_issue_2598() {
    // A 32/40 line below 7px of padding and above 23; a 16/20 line below
    // 29. The `middle` cell takes no part, and stretches to the row.
    let d = lay_out(&format!(
        "<table data-m=t><tr>\
         <td data-m=a style=\"{B};padding:7px 0 23px;font:32px/40px ProbeFace\">X<i data-m=sa></i></td>\
         <td data-m=b style=\"{B};padding-top:29px\">x<i data-m=sb></i></td>\
         <td data-m=m style=\"vertical-align:middle\"><div style=\"width:5px;height:33px\"></div></td>\
         </tr><tr><td data-m=n>y</td></tr></table>"
    ));
    check(
        "a big line",
        &d,
        &[
            H("t", 97.0),
            H("a", 77.0),
            H("m", 77.0),
            Y("sa", 35.0),
            Y("sb", 35.0),
            Y("n", 77.0),
        ],
    );
}

#[test]
fn a_taller_row_or_cell_height_is_kept_issue_2598() {
    // A row `height` above what the baseline needs: 90 stays 90.
    let d = lay_out(&format!(
        "<table data-m=t><tr style=\"height:90px\">\
         <td data-m=a style=\"{B};padding:5px 0 30px\">x<i data-m=sa></i></td>\
         <td data-m=b style=\"{B};padding-top:20px\">x<i data-m=sb></i></td>\
         </tr><tr><td data-m=n>y</td></tr></table>"
    ));
    check(
        "a row height",
        &d,
        &[
            H("t", 110.0),
            H("a", 90.0),
            Y("sa", 26.0),
            Y("sb", 26.0),
            Y("n", 90.0),
        ],
    );
    // A cell `height` is a minimum on the row, not a deeper cell: its 20px
    // line, 5 above and 30 below, reaches 34 below its baseline, so the
    // baseline row needs 70 and the cell's 75 wins (not 36 + 54 = 90).
    let d = lay_out(&format!(
        "<table data-m=t><tr>\
         <td data-m=a style=\"{B};padding:5px 0 30px;height:40px\">x<i data-m=sa></i></td>\
         <td data-m=b style=\"{B};padding-top:20px\">x<i data-m=sb></i></td>\
         </tr><tr><td data-m=n>y</td></tr></table>"
    ));
    check(
        "a cell height",
        &d,
        &[
            H("t", 95.0),
            H("a", 75.0),
            Y("sa", 26.0),
            Y("sb", 26.0),
            Y("n", 75.0),
        ],
    );
}

#[test]
fn a_cell_with_no_line_and_an_empty_cell_issue_2598() {
    // No line: the cell's baseline is its content's bottom (30), lowered 6
    // to the text's 36, with its 15px bottom padding below: 51.
    let d = lay_out(&format!(
        "<table data-m=t><tr>\
         <td data-m=a style=\"{B};padding-bottom:15px\"><div style=\"width:6px;height:30px\"></div></td>\
         <td data-m=b style=\"{B};padding-top:20px\">x<i data-m=sb></i></td>\
         </tr><tr><td data-m=n>y</td></tr></table>"
    ));
    check(
        "no line",
        &d,
        &[H("t", 71.0), H("a", 51.0), Y("sb", 26.0), Y("n", 51.0)],
    );
    // An empty cell takes no part: the line stays at 2 + 16, the row as
    // tall as the empty cell (50 + 9).
    let d = lay_out(&format!(
        "<table data-m=t><tr>\
         <td data-m=a style=\"{B};padding-top:50px;padding-bottom:9px\"></td>\
         <td data-m=b style=\"{B};padding-top:2px\">x<i data-m=sb></i></td>\
         </tr><tr><td data-m=n>y</td></tr></table>"
    ));
    check(
        "empty",
        &d,
        &[H("t", 79.0), H("a", 59.0), Y("sb", 8.0), Y("n", 59.0)],
    );
}

#[test]
fn an_email_header_with_a_meyer_reset_issue_2598() {
    // `td { vertical-align: baseline }`: a 40px logo (an inline-block on
    // its line's baseline) beside 22/28 title text. Chrome's row is 46, the
    // logo's line reaching 6 below its baseline.
    let d = lay_out(
        "<table data-m=t class=base><tr>\
         <td data-m=a><i style=\"width:40px;height:40px\"></i></td>\
         <td data-m=b style=\"font:22px/28px ProbeFace\">Title<i data-m=sb></i></td>\
         </tr><tr><td data-m=n>y</td></tr></table>",
    );
    check(
        "email",
        &d,
        &[H("t", 66.0), H("a", 46.0), Y("sb", 30.0), Y("n", 46.0)],
    );
}

#[test]
fn a_spanning_cell_and_a_block_cell_issue_2598() {
    // The spanning cell's baseline (9 + 16) is its first row's, and its
    // depth is its rows' to share as its height is: row 1 is 25 + 4 = 29,
    // then the 79px cell grows rows 1 and 2 (29 and 20) by their heights.
    let d = lay_out(&format!(
        "<table data-m=t><tr>\
         <td data-m=a rowspan=2 style=\"{B};padding:9px 0 50px\">x<i data-m=sa></i></td>\
         <td data-m=b style=\"{B};padding-top:2px\">x<i data-m=sb></i></td>\
         </tr><tr><td data-m=n>y</td></tr><tr><td data-m=n2>z</td></tr></table>"
    ));
    check(
        "rowspan",
        &d,
        &[
            H("t", 99.0),
            H("a", 79.0),
            H("b", 46.75),
            Y("sa", 15.0),
            Y("sb", 15.0),
            Y("n", 46.75),
            H("n", 32.25),
            Y("n2", 79.0),
        ],
    );
    // A `<td style="display: block">` stands in an anonymous cell: its top
    // margin counts above its line (3 + 16), it is moved itself (11, to the
    // other cell's 14 + 16), and its bottom (11 + 3 + 41) is the row's.
    let d = lay_out(&format!(
        "<table data-m=t><tr>\
         <td data-m=a style=\"display:block;margin-top:3px;padding-bottom:21px\">x<i data-m=sa></i></td>\
         <td data-m=b style=\"{B};padding-top:14px\">x<i data-m=sb></i></td>\
         </tr><tr><td data-m=n>y</td></tr></table>"
    ));
    check(
        "block cell",
        &d,
        &[
            H("t", 75.0),
            Y("a", 14.0),
            H("a", 41.0),
            H("b", 55.0),
            Y("sa", 20.0),
            Y("sb", 20.0),
            Y("n", 55.0),
        ],
    );
}

#[test]
fn a_taller_table_shares_the_grown_rows_heights_issue_2598() {
    // `height: 150px`: the 60 left over goes to the rows by their heights,
    // the grown 70 and the 20 (70 + 46.67, 20 + 13.33).
    let d = lay_out(&format!(
        "<table data-m=t style=\"height:150px\"><tr>\
         <td data-m=a style=\"{B};padding:5px 0 30px\">x<i data-m=sa></i></td>\
         <td data-m=b style=\"{B};padding-top:20px\">x<i data-m=sb></i></td>\
         </tr><tr><td data-m=n>y</td></tr></table>"
    ));
    check(
        "table height",
        &d,
        &[
            H("t", 150.0),
            H("a", 116.656),
            Y("sa", 26.0),
            Y("sb", 26.0),
            Y("n", 116.656),
            H("n", 33.344),
        ],
    );
}

#[test]
fn one_baseline_cell_alone_needs_no_more_room_issue_2598() {
    // A control: one baseline cell beside a `middle` one is not moved and
    // grows nothing (55, its own height).
    let d = lay_out(
        "<table data-m=t><tr>\
         <td data-m=a style=\"vertical-align:baseline;padding:5px 0 30px\">x<i data-m=sa></i></td>\
         <td data-m=b style=\"padding-top:20px\">x<i data-m=sb></i></td>\
         </tr><tr><td data-m=n>y</td></tr></table>",
    );
    check(
        "one",
        &d,
        &[
            H("t", 75.0),
            H("a", 55.0),
            Y("sa", 11.0),
            Y("sb", 33.5),
            Y("n", 55.0),
        ],
    );
}

#[test]
fn a_restyle_to_baseline_grows_the_row_issue_2598() {
    // The issue's row, `middle` at first (55) and then `baseline` by a
    // class on the table (70): the row's height follows `vertical-align`.
    let mut d = lay_out(
        "<table data-m=t><tr>\
         <td data-m=a style=\"padding:5px 0 30px\">x<i data-m=sa></i></td>\
         <td data-m=b style=\"padding-top:20px\">x<i data-m=sb></i></td>\
         </tr><tr><td data-m=n>y</td></tr></table>",
    );
    check("middle", &d, &[H("a", 55.0), Y("n", 55.0)]);
    let t = rinch_core::dom::NodeId(one(&d, "t"));
    d.set_attribute(t, "class", "base");
    // Another width too: `resolve_layout` returns early on an unchanged
    // viewport with nothing dirty, and the restyle must make it dirty.
    d.resolve_layout(800.0, 600.0);
    check(
        "restyled to baseline",
        &d,
        &[
            H("t", 90.0),
            H("a", 70.0),
            Y("sa", 26.0),
            Y("sb", 26.0),
            Y("n", 70.0),
        ],
    );
}

/// Tables nested `depth` deep, each a 2x3 table of baseline-aligned text
/// cells with the next table in its last cell.
fn nested(depth: usize) -> String {
    let mut html = String::from("x");
    for _ in 0..depth {
        html = format!(
            "<table><tr><td>a</td><td>b</td><td>c</td></tr>\
             <tr><td>d</td><td>e</td><td>{html}</td></tr></table>"
        );
    }
    format!("<div class=base>{html}</div>")
}

/// Taffy measure calls of a first layout and of a relayout at another
/// width that changes nothing in the tables (they are narrower than both).
fn nested_measures(depth: usize) -> (u64, u64) {
    use rinch_dom::perf::Counter::TaffyMeasureCalls;
    let mut d = lay_out("");
    d.tree.perf.end_frame();
    let body = d.body();
    d.set_inner_html(body, &nested(depth));
    d.resolve_layout(800.0, 600.0);
    let first = d.tree.perf.end_frame().get(TaffyMeasureCalls);
    d.resolve_layout(790.0, 600.0);
    let again = d.tree.perf.end_frame().get(TaffyMeasureCalls);
    (first, again)
}

#[test]
fn nested_baseline_tables_cost_linear_in_depth_issue_2598() {
    // Review of #2620: the baseline measure of a cell took its final layout's
    // one cache slot, so each table laid its cells out twice per compute and
    // each nesting level doubled the work, on every relayout too (depth 12:
    // 45,046 measure calls on a relayout that changes nothing; 0 before).
    let (first4, again4) = nested_measures(4);
    let (first8, again8) = nested_measures(8);
    let (first12, again12) = nested_measures(12);
    assert_eq!(
        (again4, again8, again12),
        (0, 0, 0),
        "a relayout that changes nothing"
    );
    assert!(first4 > 0, "a positive control: the first layout measures");
    // Linear: twelve levels cost about three times four, not 2^8 times.
    assert!(
        first12 <= 4 * first4 && first8 <= 3 * first4,
        "first layouts at depth 4/8/12: {first4}/{first8}/{first12}"
    );
}

#[test]
fn an_empty_cell_gives_the_row_no_baseline_issue_2598() {
    // The empty cell's content bottom (50) would be the row's baseline if it
    // took part, and the text cell's 34 below its own would make the row 84.
    // Chrome: 52, the text cell's own height.
    let d = lay_out(&format!(
        "<table data-m=t><tr>\
         <td data-m=a style=\"{B};padding-top:50px\"></td>\
         <td data-m=b style=\"{B};padding:2px 0 30px\">x<i data-m=sb></i></td>\
         </tr><tr><td data-m=n>y</td></tr></table>"
    ));
    check(
        "empty",
        &d,
        &[H("t", 72.0), H("a", 52.0), Y("sb", 8.0), Y("n", 52.0)],
    );
}

#[test]
fn a_block_cells_top_margin_is_above_its_baseline_issue_2598() {
    // The block's line is 20 + 16 down, the row's lowest baseline: the other
    // cell moves 6 and its 25 of padding below its line make the row
    // 36 + 29. Chrome: 65 (59 without the margin).
    let d = lay_out(&format!(
        "<table data-m=t><tr>\
         <td data-m=a style=\"display:block;margin-top:20px;padding-bottom:21px\">x<i data-m=sa></i></td>\
         <td data-m=b style=\"{B};padding:14px 0 25px\">x<i data-m=sb></i></td>\
         </tr><tr><td data-m=n>y</td></tr></table>"
    ));
    check(
        "block margin",
        &d,
        &[
            H("t", 85.0),
            Y("a", 20.0),
            Y("sa", 26.0),
            Y("sb", 26.0),
            H("b", 65.0),
            Y("n", 65.0),
        ],
    );
}
