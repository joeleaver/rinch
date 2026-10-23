//! #1874: `deleteColumn` and `deleteRow` on a grid they cannot remove in one
//! pass (cells that overlap, or cells in no slot past #1176's slot budget)
//! removed one line at a time and recomputed the whole map after each. Past
//! the slot budget a map is 2^22 slots, about 10 ms to build in a release
//! build: deleting a 1000-column cell took 10 s, and a whole row of a
//! 2,097,152-column table hours. Since #1244 a ~60 KB paste makes that table.
//!
//! The one-at-a-time removal now runs only while the maps it would build stay
//! within 2^22 slots in all; past that the lines go in one pass. What is
//! pinned is the transaction's step count: the fallback takes one step per
//! line and recomputes the map before each but the first, so `steps - 1` is
//! the number of maps it built. Each table here is cheap to run either way.

use rinch_editor_core::schema::validation::validate_content;
use rinch_editor_core::tables::{TableMap, column_count};
use rinch_editor_core::*;
use std::rc::Rc;

fn cell(s: &Schema, colspan: i64, rowspan: i64, text: &str) -> Node {
    let p = s
        .branch("paragraph", Fragment::from_node(s.text(text).unwrap()))
        .unwrap();
    s.create_node(
        "table_cell",
        Attrs::from_iter([
            ("colspan", AttrValue::Int(colspan)),
            ("rowspan", AttrValue::Int(rowspan)),
        ]),
        Fragment::from_node(p),
    )
    .unwrap()
}

fn table(s: &Schema, rows: &[Vec<(i64, i64)>]) -> Node {
    let mut n = 0;
    let rows = rows
        .iter()
        .map(|r| {
            let cells = r
                .iter()
                .map(|&(c, h)| {
                    n += 1;
                    cell(s, c, h, &format!("c{n}"))
                })
                .collect();
            s.create_node("table_row", Attrs::new(), Fragment::from_children(cells))
                .unwrap()
        })
        .collect();
    s.create_node("table", Attrs::new(), Fragment::from_children(rows))
        .unwrap()
}

fn state_with(t: Node) -> EditorState {
    let s = Schema::starter_kit();
    let tail = s
        .create_node("paragraph", Attrs::new(), Fragment::empty())
        .unwrap();
    let doc = s
        .branch("doc", Fragment::from_children(vec![t, tail]))
        .unwrap();
    EditorState::create(Rc::new(s), doc, default_plugins())
}

fn cell_pos(t: &Node, r: usize, i: usize) -> usize {
    let mut pos = 1;
    for j in 0..r {
        pos += t.child(j).node_size();
    }
    pos += 1;
    for k in 0..i {
        pos += t.child(r).child(k).node_size();
    }
    pos
}

/// Run `cmd` on `st`: the transaction's step count and the state after it.
fn run(st: &EditorState, cmd: Command) -> (usize, EditorState) {
    let mut out = None;
    assert!(
        cmd(
            st,
            Some(&mut |tr: state::Transaction| {
                out = Some((tr.steps().len(), st.apply(tr)));
            }),
        ),
        "applies"
    );
    out.unwrap()
}

fn assert_schema_valid(schema: &Schema, node: &Node) {
    if node.is_text() {
        return;
    }
    let types: Vec<&str> = node.content().children().iter().map(|c| c.type_name()).collect();
    validate_content(schema, node.type_name(), &types).unwrap();
    for c in node.content().children() {
        assert_schema_valid(schema, c);
    }
}

/// Past the slot budget: a 3-wide cell, then 2,100 cells of 1000 columns in
/// row 0 (the grid is cut at 2^22 / 2 columns, so its last cells are in no
/// slot), and a row of one cell. Deleting the 3-wide cell's columns was three
/// steps and two 2^22-slot maps; it is one step.
#[test]
fn delete_column_past_the_slot_budget_builds_no_map_per_column() {
    let s = Schema::starter_kit();
    let mut row0 = vec![(3, 1)];
    row0.extend(vec![(1000, 1); 2100]);
    let t = table(&s, &[row0, vec![(1, 1)]]);
    assert_eq!(column_count(&t), 1 << 21, "control: the grid is cut");
    let mut st = state_with(t.clone());
    st.selection = Selection::cursor(Pos(cell_pos(&t, 0, 0) + 2));
    let (steps, next) = run(&st, commands::table_ops::delete_column());
    assert_eq!(steps, 1, "no map rebuilt per column");
    assert_schema_valid(next.schema(), &next.doc);
    let nt = next.doc.child(0);
    // The 3-wide cell and row 1's cell (in column 0) are gone; row 1 with it
    // is empty, and the 2,100 wide cells are all still there.
    assert_eq!(nt.child(0).child_count(), 2100);
    assert_eq!(nt.child(1).child_count(), 0);
}

/// Cells that overlap: row 1's 3000-wide cell starts in column 0 and runs
/// over the tall cell in column 1. Deleting the 2998 columns of row 0's last
/// cell one at a time was 2998 steps and 2997 maps of 6002 slots (18 M
/// slots); past the 2^22-slot budget it is one pass.
#[test]
fn delete_column_on_overlapping_cells_stays_within_the_budget() {
    let s = Schema::starter_kit();
    let t = table(&s, &[vec![(1, 1), (1, 2), (2998, 1)], vec![(3000, 1)]]);
    let map = TableMap::compute(&t, 1);
    assert_eq!((map.width(), map.height()), (3001, 2), "control");
    let mut st = state_with(t.clone());
    st.selection = Selection::cursor(Pos(cell_pos(&t, 0, 2) + 2));
    let (steps, next) = run(&st, commands::table_ops::delete_column());
    assert_eq!(steps, 1, "one pass past the budget");
    assert_schema_valid(next.schema(), &next.doc);
    assert_eq!(next.doc.child(0).child(0).child_count(), 2, "the cell is gone");

    // A small overlapping table keeps the exact one-at-a-time removal.
    let t = table(&s, &[vec![(1, 1), (1, 2), (3, 1)], vec![(5, 1)]]);
    let mut st = state_with(t.clone());
    st.selection = Selection::cursor(Pos(cell_pos(&t, 0, 2) + 2));
    let (steps, _) = run(&st, commands::table_ops::delete_column());
    assert_eq!(steps, 3, "control: one step per column under the budget");
}

/// Rows the same way: overlapping cells over 2000 rows, 1998 of them deleted
/// (a CellSelection down column 0): one pass, where it was 1998 steps and
/// 1997 maps of 4000+ slots.
#[test]
fn delete_row_on_overlapping_cells_stays_within_the_budget() {
    let s = Schema::starter_kit();
    // Row 0: a tall cell in column 1 over every row; then rows of one cell
    // 2 wide, which runs over it (an overlap) from row 1 on.
    let mut rows = vec![vec![(1, 1), (1, 2000)]];
    rows.extend(vec![vec![(2, 1)]; 1999]);
    let t = table(&s, &rows);
    let mut st = state_with(t.clone());
    st.selection = Selection::cell(Pos(cell_pos(&t, 1, 0)), Pos(cell_pos(&t, 1998, 0)));
    let (steps, next) = run(&st, commands::table_ops::delete_row());
    assert_eq!(steps, 1, "one pass past the budget");
    assert_schema_valid(next.schema(), &next.doc);
    assert_eq!(next.doc.child(0).child_count(), 2);
}
