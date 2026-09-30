//! #1176: a pasted table cannot make the editor allocate an unbounded grid.
//!
//! The two pastes from the issue, each about 57 KB of HTML. Before the fix
//! each imported as a 1,000,000-column table, and the first `TableMap` over
//! it (every table command's `can_run`, Tab, the cell-selection overlay)
//! asked for 8 GB and aborted. `column_count` is asserted first because it
//! allocates nothing; still, run this file against an unbounded map only
//! under `ulimit -v`.

use rinch_editor_core::Schema;
use rinch_editor_core::serialize::slice_from_html;
use rinch_editor_core::tables::{TableMap, column_count};

fn imported_table(html: &str) -> rinch_editor_core::Node {
    let slice = slice_from_html(&Schema::starter_kit(), html).expect("imports");
    slice.content.child(0).clone()
}

/// H rows of `<td colspan="1000" rowspan="65534">`: each row's cell used to
/// start past every cell carried down from above, so row k was 1000·(k+1)
/// wide. The import now gives row 0's cell the whole of rows 0..H (its
/// rowspan cut to the table) and every later cell finds its row full, so it
/// is cut to one column and one row: the grid is 1001 columns at any H.
#[test]
fn a_pasted_staircase_is_1001_columns_at_any_height() {
    for rows in [1000, 4000] {
        let html = format!(
            "<table>{}</table>",
            r#"<tr><td colspan="1000" rowspan="65534"><p>x</p></td></tr>"#.repeat(rows)
        );
        assert!(html.len() > 50_000, "at least the issue's ~57 KB paste");
        let t = imported_table(&html);
        assert_eq!(t.child_count(), rows);
        assert_eq!(column_count(&t), 1001, "{rows} rows");
        let map = TableMap::compute(&t, 1);
        assert_eq!(map.map().len(), 1001 * rows);
        let first = t.child(0).child(0);
        assert_eq!(first.attrs().get_int("colspan"), Some(1000));
        assert_eq!(
            first.attrs().get_int("rowspan"),
            Some(rows as i64),
            "cut to the table"
        );
        let second = t.child(1).child(0);
        assert_eq!(second.attrs().get_int("colspan"), Some(1));
        assert_eq!(
            second.attrs().get_int("rowspan"),
            Some(1),
            "its row was full"
        );
    }
}

/// One row of N `colspan="1000"` cells, then H plain rows: the first row used
/// to be 1000·N wide. It is now 1000 + (N − 1) — one column for each cell
/// after the first, which finds the row full — so the grid is 1999 × 1000,
/// under the slot budget's floor and mapped whole.
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
    assert_eq!(column_count(&t), 1999);
    let map = TableMap::compute(&t, 1);
    assert_eq!(map.map().len(), 1999 * 1000);
    let row = t.child(0);
    assert_eq!(row.child(0).attrs().get_int("colspan"), Some(1000));
    assert_eq!(row.child(999).attrs().get_int("colspan"), Some(1));
}
