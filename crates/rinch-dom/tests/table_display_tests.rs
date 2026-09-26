//! The table-internal displays that hold flow content — `table-cell` and
//! `table-caption` — are block containers, not flex containers (#1072).
//!
//! rinch has no table formatting context. `display_from_stylo` used to send
//! every (outside, inside) pair it did not name to `DisplayValue::Flex`, so a
//! `table-cell` was a flex *row*: two block children sat side by side, and
//! its own text was an anonymous flex item, which draws no `text-overflow`
//! "…". Chrome 153 stacks a cell's block children and draws the "…"
//! (measured; the numbers are quoted at each fixture). The cell is now a
//! block container; `display: table` / `table-row` keep the flex-row
//! approximation, which is what puts cells side by side.
//!
//! Only font-independent facts are asserted: the ellipsis string is far wider
//! than 90px in any face, and every line box is declared.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const CSS: &str = "
    body { margin: 0; font-family: sans-serif; font-size: 16px; line-height: 20px; }
    .clip { width: 90px; max-width: 90px; overflow: hidden; white-space: nowrap;
            text-overflow: ellipsis; }
";

fn el(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let x = doc.create_element("div");
    if !style.is_empty() {
        doc.set_attribute(x, "style", style);
    }
    doc.append_child(parent, x);
    x
}

fn text(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}

/// Whether the box's own inline layout ends in "…" (`None`: not an IFC root).
fn ellipsis_for(display: &str) -> Option<bool> {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let wrap = el(&mut doc, body, "");
    let x = el(&mut doc, wrap, &format!("display: {display}"));
    doc.set_attribute(x, "class", "clip");
    text(
        &mut doc,
        x,
        "Hello wonderful world of text, far too long for ninety px",
    );
    doc.resolve_layout(400.0, 300.0);
    doc.tree
        .get(x.0)
        .unwrap()
        .text_layout
        .as_ref()
        .map(|l| l.text_content.ends_with('\u{2026}'))
}

/// Chrome 153: `lone` (a `table-cell`, 90px, nowrap, ellipsis) and `cap` (a
/// `table-caption`) both draw the "…"; a flex container's text does not.
#[test]
fn a_table_cell_and_a_table_caption_draw_their_ellipsis() {
    let cases: &[(&str, bool)] = &[
        ("table-cell", true),
        ("table-caption", true),
        // Controls: agree with Chrome before and after #1072.
        ("block", true),
        ("flow-root", true),
        ("list-item", true),
        ("flex", false),
    ];
    let mut failures = Vec::new();
    for &(display, want) in cases {
        let got = ellipsis_for(display).unwrap_or(false);
        if got != want {
            failures.push(format!("display: {display}: ellipsis {got}, want {want}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Chrome 153, `display: table; width: 400px` holding two cells, the first
/// holding two block children `A1` / `B1` at `line-height: 20px`:
/// `c1` 0,0 220x40 · `a1` 0,0 · `b1` 0,20 · `c2` 220,0. The cells sit side by
/// side (kept: the table is still a flex row) and a cell's blocks stack
/// (new: the cell was a flex row and put `b1` beside `a1`).
#[test]
fn a_cells_blocks_stack_while_the_cells_stay_side_by_side() {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let t = el(&mut doc, body, "display: table; width: 400px");
    let c1 = el(&mut doc, t, "display: table-cell");
    let a1 = el(&mut doc, c1, "");
    text(&mut doc, a1, "A1");
    let b1 = el(&mut doc, c1, "");
    text(&mut doc, b1, "B1");
    let c2 = el(&mut doc, t, "display: table-cell");
    text(&mut doc, c2, "C2");
    doc.resolve_layout(800.0, 600.0);

    let at = |n: NodeId| {
        let l = &doc.tree.get(n.0).unwrap().layout;
        (l.x, l.y, l.width, l.height)
    };
    let (c1x, c1y, c1w, c1h) = at(c1);
    let (c2x, c2y, ..) = at(c2);
    let (a1x, a1y, ..) = at(a1);
    let (b1x, b1y, ..) = at(b1);

    assert_eq!((c1x, c1y), (0.0, 0.0), "c1");
    assert_eq!(c2y, 0.0, "c2 shares c1's row");
    assert!(
        c2x >= c1w && c2x > 0.0,
        "c2 sits right of c1: c2.x {c2x}, c1.w {c1w}"
    );
    assert_eq!((a1x, a1y), (0.0, 0.0), "a1");
    assert_eq!((b1x, b1y), (0.0, 20.0), "b1 stacks under a1");
    assert_eq!(c1h, 40.0, "c1 holds two 20px lines");
}

// ---------------------------------------------------------------------------
// #1083: a table's rows stack; a row's cells sit side by side.
//
// Every cell holds a fixed-size block, so no number below depends on a font.
// Chrome 153 (standards mode, `body { margin: 0 }`) is quoted at each fixture.
// Chrome also aligns columns across rows (a cell is as wide as the widest cell
// of its column); rinch's flex approximation does not, so the fixtures assert
// the stacking and the side-by-side, never a column width.
// ---------------------------------------------------------------------------

/// A cell holding one `w` x `h` block.
fn cell(doc: &mut RinchDocument, parent: NodeId, w: u32, h: u32) -> NodeId {
    let c = el(doc, parent, "display: table-cell");
    el(doc, c, &format!("width: {w}px; height: {h}px"));
    c
}

fn abs(doc: &RinchDocument, n: NodeId) -> (f32, f32, f32, f32) {
    let (x, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, n.0, 1.0);
    let l = &doc.tree.get(n.0).unwrap().layout;
    (x as f32, y as f32, l.width, l.height)
}

fn xy(doc: &RinchDocument, n: NodeId) -> (f32, f32) {
    let (x, y, ..) = abs(doc, n);
    (x, y)
}

fn fresh() -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let t = el(&mut doc, body, "display: table");
    (doc, t)
}

/// Chrome 153: `r1` 0,0 h20 · `r2` 0,20 h40 · `a` 0,0 · `b` 60,0 · `c` 0,20 ·
/// `d` 60,20 (the 60 is the column's width; rinch puts `b` at 30 and `d` at
/// 60, its cells' own widths).
#[test]
fn a_tables_rows_stack_and_each_rows_cells_sit_side_by_side() {
    let (mut doc, t) = fresh();
    let r1 = el(&mut doc, t, "display: table-row");
    let a = cell(&mut doc, r1, 30, 20);
    let b = cell(&mut doc, r1, 50, 20);
    let r2 = el(&mut doc, t, "display: table-row");
    let c = cell(&mut doc, r2, 60, 20);
    let d = cell(&mut doc, r2, 10, 40);
    doc.resolve_layout(800.0, 600.0);

    let (_, r1y, _, r1h) = abs(&doc, r1);
    let (r2x, r2y, _, r2h) = abs(&doc, r2);
    assert_eq!((r1y, r1h), (0.0, 20.0), "r1");
    assert_eq!((r2x, r2y, r2h), (0.0, 20.0, 40.0), "r2 stacks under r1");
    let (ax, ay, ..) = abs(&doc, a);
    let (bx, by, ..) = abs(&doc, b);
    assert_eq!((ax, ay), (0.0, 0.0), "a");
    assert_eq!((bx, by), (30.0, 0.0), "b sits right of a");
    let (cx, cy, ..) = abs(&doc, c);
    let (dx, dy, ..) = abs(&doc, d);
    assert_eq!((cx, cy), (0.0, 20.0), "c");
    assert_eq!((dx, dy), (60.0, 20.0), "d sits right of c");
}

/// Chrome 153, groups written in their display order: `th` 0,0 · `tb` 0,20 ·
/// `tf` 0,40, and `tb`'s row's two cells at x 0 and 30. (Chrome also moves a
/// `table-header-group` first and a `table-footer-group` last whatever the
/// DOM order; rinch keeps DOM order.)
#[test]
fn row_groups_stack_and_hold_their_rows() {
    let (mut doc, t) = fresh();
    let mut groups = Vec::new();
    for g in ["table-header-group", "table-row-group", "table-footer-group"] {
        let grp = el(&mut doc, t, &format!("display: {g}"));
        let row = el(&mut doc, grp, "display: table-row");
        let first = cell(&mut doc, row, 30, 20);
        let second = cell(&mut doc, row, 40, 20);
        groups.push((grp, first, second));
    }
    doc.resolve_layout(800.0, 600.0);
    for (i, &(grp, first, second)) in groups.iter().enumerate() {
        let want_y = 20.0 * i as f32;
        let (gx, gy, _, gh) = abs(&doc, grp);
        assert_eq!((gx, gy, gh), (0.0, want_y, 20.0), "group {i}");
        assert_eq!(abs(&doc, first).0, 0.0, "group {i} first cell x");
        let (sx, sy, ..) = abs(&doc, second);
        assert_eq!((sx, sy), (30.0, want_y), "group {i} second cell");
    }
}

/// Chrome 153: cells straight in a table share one anonymous row — `a` 0,0,
/// `b` 30,0 — and so do cells straight in a row group. A single-row table
/// is the same row. Kept from before #1083.
#[test]
fn bare_cells_and_a_single_row_stay_side_by_side() {
    for shape in ["bare", "row", "row-group"] {
        let (mut doc, t) = fresh();
        let holder = match shape {
            "bare" => t,
            "row" => el(&mut doc, t, "display: table-row"),
            _ => el(&mut doc, t, "display: table-row-group"),
        };
        let a = cell(&mut doc, holder, 30, 20);
        let b = cell(&mut doc, holder, 50, 40);
        doc.resolve_layout(800.0, 600.0);
        assert_eq!(xy(&doc, a), (0.0, 0.0), "{shape}: a");
        let (bx, by, ..) = abs(&doc, b);
        assert_eq!((bx, by), (30.0, 0.0), "{shape}: b sits right of a");
    }
}

/// Chrome 153: rows behind a `display: contents` wrapper — every `for` in
/// `rsx!` emits one — are the table's rows: `r1` 0,0 · `r2` 0,20. Also when
/// they arrive after the first layout, and when a table that held rows loses
/// them to bare cells (a table whose only rows went is a row of cells again).
#[test]
fn rows_behind_a_contents_wrapper_stack_including_rows_added_later() {
    let (mut doc, t) = fresh();
    let w = el(&mut doc, t, "display: contents");
    doc.resolve_layout(800.0, 600.0);

    let r1 = el(&mut doc, w, "display: table-row");
    cell(&mut doc, r1, 30, 20);
    let r2 = el(&mut doc, w, "display: table-row");
    cell(&mut doc, r2, 50, 20);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(xy(&doc, r1), (0.0, 0.0), "r1");
    assert_eq!(xy(&doc, r2), (0.0, 20.0), "r2 stacks under r1");
}

/// Chrome 153: a table's bare cells share an anonymous row, and a row after
/// them stacks below it (`a` 0,0 · `b` 30,0 · `r` 0,20; rinch stacks all
/// three, see `table_flex_direction`). Removing the row — and nothing else —
/// leaves a table of bare cells, whose cells share a row again.
#[test]
fn removing_the_last_row_puts_the_bare_cells_back_side_by_side() {
    let (mut doc, t) = fresh();
    let a = cell(&mut doc, t, 30, 20);
    let b = cell(&mut doc, t, 40, 20);
    let r = el(&mut doc, t, "display: table-row");
    cell(&mut doc, r, 50, 20);
    doc.resolve_layout(800.0, 600.0);
    let (_, ry) = xy(&doc, r);
    assert!(ry >= 20.0, "r stacks under the cells: y {ry}");

    doc.remove_node(r);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(xy(&doc, a), (0.0, 0.0), "a");
    assert_eq!(xy(&doc, b), (30.0, 0.0), "b sits right of a");
}

/// A restyle that turns a table's cells into rows stacks them, and one that
/// turns them back puts them side by side again: the table's direction
/// follows its children's display, which no style of its own carries.
#[test]
fn a_childs_display_flip_redirects_the_table() {
    let (mut doc, t) = fresh();
    let x = el(&mut doc, t, "display: table-cell");
    el(&mut doc, x, "width: 30px; height: 20px");
    let y = el(&mut doc, t, "display: table-cell");
    el(&mut doc, y, "width: 50px; height: 20px");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(xy(&doc, y), (30.0, 0.0), "cells: side by side");

    doc.set_attribute(x, "style", "display: table-row");
    doc.set_attribute(y, "style", "display: table-row");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(xy(&doc, y), (0.0, 20.0), "rows: stacked");

    doc.set_attribute(x, "style", "display: table-cell");
    doc.set_attribute(y, "style", "display: table-cell");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(xy(&doc, y), (30.0, 0.0), "cells again: side by side");
}
