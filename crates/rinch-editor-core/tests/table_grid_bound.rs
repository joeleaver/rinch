//! #1176: a pasted table cannot make the editor allocate an unbounded grid.
//!
//! The two pastes from the issue, each about 57 KB of HTML. Each imports as a
//! 1,000,000-column table (since #1244 the paste keeps a row as wide as
//! Chrome does, as a load always has). Before #1176 the first `TableMap` over
//! it (every table command's `can_run`, Tab, the cell-selection overlay)
//! asked for 8 GB and aborted; the map is now cut at the slot budget. `column_count` is asserted first because it
//! allocates nothing; still, run this file against an unbounded map only
//! under `ulimit -v`.

use rinch_editor_core::Schema;
use rinch_editor_core::serialize::slice_from_html;
use rinch_editor_core::tables::{GRID_SLOT_FLOOR, TableMap, column_count};

fn imported_table(html: &str) -> rinch_editor_core::Node {
    let slice = slice_from_html(&Schema::starter_kit(), html).expect("imports");
    slice.content.child(0).clone()
}

/// H rows of `<td colspan="1000" rowspan="65534">`: each row's cell starts
/// past every cell carried down from above, so row k is 1000·(k+1) wide, as
/// Chrome lays it out. Since #1244 the paste keeps that (it used to cut a row
/// at 1000 columns, which a load did not); the grid is bounded where it is
/// mapped, at `GRID_SLOT_FLOOR / H` columns.
#[test]
fn a_pasted_staircase_is_mapped_within_the_slot_budget() {
    for rows in [1000, 4000] {
        let html = format!(
            "<table>{}</table>",
            r#"<tr><td colspan="1000" rowspan="65534"><p>x</p></td></tr>"#.repeat(rows)
        );
        assert!(html.len() > 50_000, "at least the issue's ~57 KB paste");
        let t = imported_table(&html);
        assert_eq!(t.child_count(), rows);
        // Asked first: it allocates nothing, so an unbounded width fails here.
        assert_eq!(column_count(&t), GRID_SLOT_FLOOR / rows, "{rows} rows");
        let map = TableMap::compute(&t, 1);
        assert_eq!(map.map().len(), GRID_SLOT_FLOOR / rows * rows);
        for (r, left) in [(0, rows), (1, rows - 1)] {
            let cell = t.child(r).child(0);
            assert_eq!(cell.attrs().get_int("colspan"), Some(1000));
            assert_eq!(
                cell.attrs().get_int("rowspan"),
                Some(left as i64),
                "cut to the table"
            );
        }
    }
}

/// One row of N `colspan="1000"` cells, then H plain rows: the first row is
/// 1000·N columns wide, as in Chrome (#1244), and the map is cut at the slot
/// budget's `GRID_SLOT_FLOOR / H` columns.
#[test]
fn a_pasted_wide_row_has_a_bounded_grid() {
    let html = format!(
        "<table><tr>{}</tr>{}</table>",
        r#"<td colspan="1000"><p>x</p></td>"#.repeat(1000),
        "<tr><td><p>y</p></td></tr>".repeat(999)
    );
    assert!(html.len() > 50_000, "the issue's ~58 KB paste");
    let t = imported_table(&html);
    assert_eq!(t.child_count(), 1000);
    assert_eq!(column_count(&t), GRID_SLOT_FLOOR / 1000);
    let map = TableMap::compute(&t, 1);
    assert_eq!(map.map().len(), GRID_SLOT_FLOOR / 1000 * 1000);
    let row = t.child(0);
    assert_eq!(row.child(0).attrs().get_int("colspan"), Some(1000));
    assert_eq!(row.child(999).attrs().get_int("colspan"), Some(1000));
}
