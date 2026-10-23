//! #1244: `addRowBefore` / `addRowAfter` copy the reference row's column
//! spans instead of building one cell per grid column.
//!
//! A load keeps Chrome's limits (`colspan` ≤ 1000, `rowspan` ≤ 65534) and
//! nothing more, so a row of many 1000-wide cells is as wide as their sum: a
//! row of 2,100 such cells made a 2,097,152-column grid (#1176's slot budget),
//! and `addRowAfter` built a cell per column — 2,099,253 cells, 32 s and
//! 5 GB through a mounted view. A new row now has a cell per cell of the row
//! it is inserted beside (the row above for `addRowAfter`, the row below for
//! `addRowBefore`), cut where a cell spanning across the new row's line takes
//! the columns; so the cost is that row's cell count.
//!
//! A table with no merged cells gets exactly the row it got before: one
//! plain cell per column.

use rinch_editor_core::tables::{TableMap, column_count, is_cell};
use rinch_editor_core::*;
use std::rc::Rc;

fn para(s: &Schema, text: &str) -> Node {
    if text.is_empty() {
        return s
            .create_node("paragraph", Attrs::new(), Fragment::empty())
            .unwrap();
    }
    s.branch("paragraph", Fragment::from_node(s.text(text).unwrap()))
        .unwrap()
}

fn typed_cell(s: &Schema, ty: &str, colspan: i64, rowspan: i64, text: &str) -> Node {
    s.create_node(
        ty,
        Attrs::from_iter([
            ("colspan", AttrValue::Int(colspan)),
            ("rowspan", AttrValue::Int(rowspan)),
        ]),
        Fragment::from_node(para(s, text)),
    )
    .unwrap()
}

/// A table from rows of `(colspan, rowspan)`; `header_rows` leading rows are
/// `table_header_cell`s.
fn table_h(s: &Schema, rows: &[Vec<(i64, i64)>], header_rows: usize) -> Node {
    let mut n = 0;
    let rows = rows
        .iter()
        .enumerate()
        .map(|(r, cells)| {
            let ty = if r < header_rows {
                "table_header_cell"
            } else {
                "table_cell"
            };
            let cells = cells
                .iter()
                .map(|&(c, h)| {
                    n += 1;
                    typed_cell(s, ty, c, h, &format!("c{n}"))
                })
                .collect();
            s.create_node("table_row", Attrs::new(), Fragment::from_children(cells))
                .unwrap()
        })
        .collect();
    s.create_node("table", Attrs::new(), Fragment::from_children(rows))
        .unwrap()
}

fn table(s: &Schema, rows: &[Vec<(i64, i64)>]) -> Node {
    table_h(s, rows, 0)
}

/// The table at doc position 0, then an empty paragraph.
fn state_with(s: Schema, t: Node) -> EditorState {
    let tail = para(&s, "");
    let doc = s
        .branch("doc", Fragment::from_children(vec![t, tail]))
        .unwrap();
    EditorState::create(Rc::new(s), doc, default_plugins())
}

/// The position before row `r`'s cell `i` (the table is at doc position 0).
fn cell_pos(t: &Node, r: usize, i: usize) -> usize {
    let mut pos = 1;
    for j in 0..r {
        pos += t.child(j).node_size();
    }
    pos += 1;
    let row = t.child(r);
    for k in 0..i {
        pos += row.child(k).node_size();
    }
    pos
}

/// Run `cmd` with the caret in row `r`'s cell `i`; the table after it.
fn run_in(t: &Node, r: usize, i: usize, cmd: &str) -> Node {
    let mut st = state_with(Schema::starter_kit(), t.clone());
    st.selection = Selection::cursor(Pos(cell_pos(t, r, i) + 2));
    let next = st.run(cmd).unwrap_or_else(|| panic!("{cmd} applies"));
    next.doc.child(0).clone()
}

/// `(type, colspan, rowspan)` of every cell, row by row.
fn shape(t: &Node) -> Vec<Vec<(&'static str, i64, i64)>> {
    (0..t.child_count())
        .map(|r| {
            let row = t.child(r);
            (0..row.child_count())
                .map(|i| {
                    let c = row.child(i);
                    assert!(is_cell(c));
                    let ty = if c.type_name() == "table_header_cell" {
                        "th"
                    } else {
                        "td"
                    };
                    let a = c.attrs();
                    (
                        ty,
                        a.get_int("colspan").unwrap_or(1),
                        a.get_int("rowspan").unwrap_or(1),
                    )
                })
                .collect()
        })
        .collect()
}

fn td(c: i64, r: i64) -> (&'static str, i64, i64) {
    ("td", c, r)
}

/// Whether `t` is a well-formed grid: every slot covered once, every span
/// exactly the rectangle it covers.
fn assert_well_formed(t: &Node, what: &str) {
    let map = TableMap::compute(t, 1);
    assert!(!map.map().contains(&usize::MAX), "{what}: a hole");
    let mut area = 0;
    for r in 0..t.child_count() {
        for i in 0..t.child(r).child_count() {
            let rect = map
                .find_cell(cell_pos(t, r, i))
                .unwrap_or_else(|| panic!("{what}: row {r} cell {i} off the grid"));
            let a = t.child(r).child(i).attrs();
            let (cs, rs) = (
                a.get_int("colspan").unwrap_or(1),
                a.get_int("rowspan").unwrap_or(1),
            );
            assert_eq!(
                (cs as usize, rs as usize),
                (rect.right - rect.left, rect.bottom - rect.top),
                "{what}: row {r} cell {i}"
            );
            area += (cs * rs) as usize;
        }
    }
    assert_eq!(area, map.map().len(), "{what}: cells cover the grid");
}

// ------------------------------------------------------------------
// The issue's cost, on a table small enough for the old code to finish.
// ------------------------------------------------------------------

/// Two rows: 20 cells of `colspan = 1000`, then 20 more. The grid is 20,000
/// columns wide; the old `addRow` built 20,000 cells, one per column. The new
/// row copies its neighbour: 20 cells of 1000.
#[test]
fn a_row_beside_wide_cells_has_one_cell_per_neighbouring_cell() {
    let s = Schema::starter_kit();
    let t = table(&s, &[vec![(1000, 1); 20], vec![(1000, 1); 20]]);
    assert_eq!(column_count(&t), 20_000);
    for (r, cmd, at) in [
        (0, "addRowAfter", 1),
        (0, "addRowBefore", 0),
        (1, "addRowAfter", 2),
        (1, "addRowBefore", 1),
    ] {
        let nt = run_in(&t, r, 3, cmd);
        assert_eq!(nt.child_count(), 3, "{cmd} in row {r}");
        assert_eq!(
            nt.child(at).child_count(),
            20,
            "{cmd} in row {r}: cells in the new row"
        );
        assert_eq!(shape(&nt)[at], vec![td(1000, 1); 20], "{cmd} in row {r}");
        assert_well_formed(&nt, cmd);
    }
}

// ------------------------------------------------------------------
// No merged cells: exactly the row the old code built.
// ------------------------------------------------------------------

/// The row the old `addRow` inserted into a table with no merged cells: one
/// `table_cell` with one empty paragraph per grid column (holes included).
fn old_row(s: &Schema, width: usize) -> Node {
    let cells = (0..width)
        .map(|_| {
            s.create_node(
                "table_cell",
                Attrs::new(),
                Fragment::from_node(para(s, "")),
            )
            .unwrap()
        })
        .collect();
    s.create_node("table_row", Attrs::new(), Fragment::from_children(cells))
        .unwrap()
}

/// A node's types, every attribute and text, in full: `Node`'s `==` also
/// compares the schema it was built with, and each state here has its own.
fn canon(n: &Node) -> String {
    let mut out = format!("{}{:?}", n.type_name(), n.attrs());
    if let Some(t) = n.text() {
        out.push_str(&format!("{t:?}"));
    }
    out.push('[');
    for c in n.content().children() {
        out.push_str(&canon(c));
    }
    out.push(']');
    out
}

/// The old row and the new one for both commands with the caret in row `r`'s
/// cell `i` of `t` (no merged cells).
fn assert_same_rows_as_before(s: &Schema, t: &Node, r: usize, i: usize, what: &str) {
    let width = column_count(t);
    for (cmd, at) in [("addRowBefore", r), ("addRowAfter", r + 1)] {
        let nt = run_in(t, r, i, cmd);
        let mut want: Vec<Node> = t.content().children().to_vec();
        want.insert(at, old_row(s, width));
        let want = t.copy_with_content(Fragment::from_children(want));
        assert_eq!(canon(&nt), canon(&want), "{cmd} in row {r} cell {i} of {what}");
    }
}

/// Every table of 1×1 cells up to 3 rows of up to 3 cells, ragged ones
/// included, with the caret in every cell, and a 6×5 table from three of
/// its cells: both commands insert the same row the old code inserted
/// (types, every attribute, content).
#[test]
fn a_table_with_no_merged_cells_gets_the_same_row_as_before() {
    let s = Schema::starter_kit();
    let mut tables = 0;
    for rows in 1..=3usize {
        // Every assignment of 1..=3 cells to each row.
        for code in 0..3usize.pow(rows as u32) {
            let lens: Vec<usize> = (0..rows)
                .map(|r| code / 3usize.pow(r as u32) % 3 + 1)
                .collect();
            let spec: Vec<Vec<(i64, i64)>> = lens.iter().map(|&n| vec![(1, 1); n]).collect();
            let t = table(&s, &spec);
            tables += 1;
            for (r, &len) in lens.iter().enumerate() {
                for i in 0..len {
                    assert_same_rows_as_before(&s, &t, r, i, &format!("{lens:?}"));
                }
            }
        }
    }
    assert_eq!(tables, 3 + 9 + 27);
    let t = table(&s, &vec![vec![(1, 1); 6]; 5]);
    for (r, i) in [(0, 0), (2, 3), (4, 5)] {
        assert_same_rows_as_before(&s, &t, r, i, "6x5");
    }
}

// ------------------------------------------------------------------
// Merged neighbours, rowspans, the first row, headers.
// ------------------------------------------------------------------

/// The new row follows its neighbour's merges: under a 2-wide cell, a 2-wide
/// cell; above a row of single cells, single cells.
#[test]
fn a_merged_neighbour_is_copied() {
    let s = Schema::starter_kit();
    let t = table(&s, &[vec![(2, 1), (1, 1)], vec![(1, 1), (1, 1), (1, 1)]]);
    let nt = run_in(&t, 0, 0, "addRowAfter");
    assert_eq!(
        shape(&nt),
        vec![
            vec![td(2, 1), td(1, 1)],
            vec![td(2, 1), td(1, 1)],
            vec![td(1, 1), td(1, 1), td(1, 1)],
        ]
    );
    assert_well_formed(&nt, "after the merged row");
    // Before row 1 (below the merged row): row 1 is the neighbour.
    let nt = run_in(&t, 1, 2, "addRowBefore");
    assert_eq!(shape(&nt)[1], vec![td(1, 1), td(1, 1), td(1, 1)]);
    assert_well_formed(&nt, "before the plain row");
}

/// A cell spanning across the new row's line grows a row, as before, and
/// the new row's cells fill only the columns it leaves, cut at its edges.
#[test]
fn a_rowspan_across_the_line_grows_and_the_rest_is_copied() {
    let s = Schema::starter_kit();
    // Column 0 is one cell for rows 0..2; row 0's 2-wide cell, then row 1's
    // two singles.
    let t = table(&s, &[vec![(1, 2), (2, 1)], vec![(1, 1), (1, 1)]]);
    let nt = run_in(&t, 0, 1, "addRowAfter");
    assert_eq!(
        shape(&nt),
        vec![
            vec![td(1, 3), td(2, 1)],
            vec![td(2, 1)],
            vec![td(1, 1), td(1, 1)],
        ]
    );
    assert_well_formed(&nt, "addRowAfter inside the rowspan");
    // addRowBefore in row 1: the line between rows 0 and 1 is crossed by the
    // tall cell; row 1 is the neighbour, so two singles.
    let nt = run_in(&t, 1, 0, "addRowBefore");
    assert_eq!(
        shape(&nt),
        vec![
            vec![td(1, 3), td(2, 1)],
            vec![td(1, 1), td(1, 1)],
            vec![td(1, 1), td(1, 1)],
        ]
    );
    assert_well_formed(&nt, "addRowBefore inside the rowspan");
    // After the last row the tall cell ends there and does not grow; the new
    // row copies the last row as the grid has it: the tall cell's column,
    // then the two singles.
    let nt = run_in(&t, 1, 0, "addRowAfter");
    assert_eq!(
        shape(&nt),
        vec![
            vec![td(1, 2), td(2, 1)],
            vec![td(1, 1), td(1, 1)],
            vec![td(1, 1), td(1, 1), td(1, 1)],
        ]
    );
    assert_well_formed(&nt, "addRowAfter at the end");
}

/// A tall, wide cell in the neighbour row: the new row's cells stop at its
/// left edge and start again at its right edge, each keeping the
/// neighbour's own boundaries.
#[test]
fn a_wide_rowspan_cuts_the_copied_row_into_runs() {
    let s = Schema::starter_kit();
    let t = table(
        &s,
        &[
            vec![(2, 1), (2, 2), (3, 1)],
            vec![(1, 1), (1, 1), (1, 1), (2, 1)],
        ],
    );
    let nt = run_in(&t, 0, 0, "addRowAfter");
    assert_eq!(
        shape(&nt),
        vec![
            vec![td(2, 1), td(2, 3), td(3, 1)],
            vec![td(2, 1), td(3, 1)],
            vec![td(1, 1), td(1, 1), td(1, 1), td(2, 1)],
        ]
    );
    assert_well_formed(&nt, "around the tall cell");
}

/// `addRowBefore` on the first row copies the first row's merges.
#[test]
fn add_row_before_the_first_row_copies_it() {
    let s = Schema::starter_kit();
    let t = table(&s, &[vec![(3, 1), (1, 2)], vec![(1, 1), (2, 1)]]);
    let nt = run_in(&t, 0, 0, "addRowBefore");
    assert_eq!(
        shape(&nt),
        vec![
            vec![td(3, 1), td(1, 1)],
            vec![td(3, 1), td(1, 2)],
            vec![td(1, 1), td(2, 1)],
        ]
    );
    assert_well_formed(&nt, "before the first row");
}

/// The new row is plain `table_cell`s, as before, whatever its neighbour is:
/// a header row's `th`s are not copied, only its spans.
#[test]
fn a_header_neighbours_cells_are_not_copied_as_headers() {
    let s = Schema::starter_kit();
    let t = table_h(&s, &[vec![(2, 1), (1, 1)], vec![(1, 1), (1, 1), (1, 1)]], 1);
    for (cmd, at) in [("addRowBefore", 0), ("addRowAfter", 1)] {
        let nt = run_in(&t, 0, 1, cmd);
        let sh = shape(&nt);
        assert_eq!(sh[at], vec![td(2, 1), td(1, 1)], "{cmd}");
        let header = if at == 0 { 1 } else { 0 };
        assert_eq!(sh[header], vec![("th", 2, 1), ("th", 1, 1)], "{cmd}");
        assert_well_formed(&nt, cmd);
    }
}
