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
/// wide. The import now cuts each row to 1000 columns (a cell keeps at least
/// one), so row k is 1000 + k wide, and the map is capped at 2^20 slots.
#[test]
fn a_pasted_staircase_has_a_bounded_grid() {
    let html = format!(
        "<table>{}</table>",
        r#"<tr><td colspan="1000" rowspan="65534"><p>x</p></td></tr>"#.repeat(1000)
    );
    assert!(html.len() > 50_000, "the issue's ~57 KB paste");
    let t = imported_table(&html);
    assert_eq!(t.child_count(), 1000);
    assert_eq!(column_count(&t), (1 << 20) / 1000);
    let map = TableMap::compute(&t, 1);
    assert_eq!(map.map().len(), 1048 * 1000);
    let first = t.child(0).child(0);
    assert_eq!(first.attrs().get_int("colspan"), Some(1000));
    assert_eq!(
        first.attrs().get_int("rowspan"),
        Some(1000),
        "cut to the table"
    );
    let second = t.child(1).child(0);
    assert_eq!(second.attrs().get_int("colspan"), Some(1));
    assert_eq!(second.attrs().get_int("rowspan"), Some(999));
}

/// One row of N `colspan="1000"` cells, then H plain rows: the first row used
/// to be 1000·N wide. It is now 1000 + (N − 1), so the map is 1999 × 1000 —
/// past the 2^20-slot floor, so capped there too.
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
    assert_eq!(column_count(&t), (1 << 20) / 1000);
    let map = TableMap::compute(&t, 1);
    assert_eq!(map.map().len(), 1048 * 1000);
    let row = t.child(0);
    assert_eq!(row.child(0).attrs().get_int("colspan"), Some(1000));
    assert_eq!(row.child(999).attrs().get_int("colspan"), Some(1));
}
