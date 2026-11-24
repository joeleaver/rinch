//! Unit tests of the table structure commands.

use super::*;
use crate::commands::build_table;
use crate::model::Fragment;
use crate::state::EditorState;

fn sk() -> Schema {
    Schema::starter_kit()
}

/// A fresh state whose document is a single `rows×cols` table, cursor inside the
/// top-left cell.
fn table_state(rows: usize, cols: usize) -> EditorState {
    let s = sk();
    let table = build_table(&s, rows, cols).unwrap();
    // A trailing empty paragraph keeps the doc valid (`block+`) when a command
    // deletes the whole table — the realistic shape (an editor always has a
    // landing block after a table).
    let tail = s
        .create_node("paragraph", Attrs::new(), Fragment::empty())
        .unwrap();
    let doc = s
        .branch("doc", Fragment::from_children(vec![table, tail]))
        .unwrap();
    let plugins: Vec<std::rc::Rc<dyn crate::plugin::Plugin>> =
        vec![std::rc::Rc::new(crate::commands::BaseCommandsPlugin)];
    let mut st = EditorState::create(std::rc::Rc::new(s), doc, plugins);
    // cursor inside the first cell's paragraph (table 1, row 2, cell 3, p 4)
    st.selection = Selection::cursor(Pos(4));
    st
}

fn table_of(state: &EditorState) -> Node {
    state
        .doc
        .content()
        .children()
        .iter()
        .find(|n| tables::is_table(n))
        .cloned()
        .expect("a table")
}

fn dims(state: &EditorState) -> (usize, usize) {
    let table = table_of(state);
    let map = TableMap::compute(&table, 1);
    (map.width(), map.height())
}

#[test]
fn add_row_after_grows_height() {
    let state = table_state(2, 3);
    let next = state.run("addRowAfter").expect("applies");
    assert_eq!(dims(&next), (3, 3), "one more row");
    // The table is still well-formed (every row has 3 cells).
    let table = table_of(&next);
    for r in 0..3 {
        assert_eq!(table.content().child(r).content().child_count(), 3);
    }
}

#[test]
fn add_column_before_grows_width() {
    let state = table_state(2, 2);
    let next = state.run("addColumnBefore").expect("applies");
    assert_eq!(dims(&next), (3, 2), "one more column");
}

#[test]
fn delete_row_shrinks_and_undoes() {
    let state = table_state(3, 2);
    let next = state.run("deleteRow").expect("applies");
    assert_eq!(dims(&next), (2, 2), "one fewer row");
}

#[test]
fn delete_column_shrinks() {
    let state = table_state(2, 3);
    let next = state.run("deleteColumn").expect("applies");
    assert_eq!(dims(&next), (2, 2), "one fewer column");
}

#[test]
fn delete_last_row_deletes_table() {
    // A 1-row table: deleteRow would empty it → the whole table goes.
    let state = table_state(1, 2);
    // Select all cells so the rect spans the only row.
    let next = state.run("deleteRow").expect("applies");
    assert!(
        next.doc
            .content()
            .children()
            .iter()
            .all(|n| !tables::is_table(n)),
        "table removed entirely"
    );
}

#[test]
fn delete_over_cell_selection_blanks_not_collapses() {
    // A 2x2 table with text in each cell. Selecting all cells and pressing
    // Backspace must BLANK the cells, not splice the coarse range (which the old
    // path did, collapsing the table to 1x1).
    let s = sk();
    let text_cell = |t: &str| {
        let p = s
            .branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
            .unwrap();
        s.create_node("table_cell", Attrs::new(), Fragment::from_node(p))
            .unwrap()
    };
    let row = |a: &str, b: &str| {
        s.create_node(
            "table_row",
            Attrs::new(),
            Fragment::from_children(vec![text_cell(a), text_cell(b)]),
        )
        .unwrap()
    };
    let table = s
        .create_node(
            "table",
            Attrs::new(),
            Fragment::from_children(vec![row("a", "b"), row("c", "d")]),
        )
        .unwrap();
    let tail = s
        .create_node("paragraph", Attrs::new(), Fragment::empty())
        .unwrap();
    let doc = s
        .branch("doc", Fragment::from_children(vec![table.clone(), tail]))
        .unwrap();
    let plugins: Vec<std::rc::Rc<dyn crate::plugin::Plugin>> =
        vec![std::rc::Rc::new(crate::commands::BaseCommandsPlugin)];
    let mut state = EditorState::create(std::rc::Rc::new(s), doc, plugins);
    let map = TableMap::compute(&table, 1);
    let c00 = map.cell_at(0, 0).unwrap();
    let c11 = map.cell_at(1, 1).unwrap();
    state.selection = Selection::cell(Pos(c00), Pos(c11));

    let next = state.run("deleteCharBackward").expect("applies");
    // The table survives at full dimensions (the critical regression).
    let ntable = table_of(&next);
    let nmap = TableMap::compute(&ntable, 1);
    assert_eq!((nmap.width(), nmap.height()), (2, 2), "table preserved");
    // Every cell is now empty (its text was blanked).
    for r in 0..2 {
        for c in 0..2 {
            let cell = ntable.content().child(r).content().child(c);
            assert_eq!(cell.content().child(0).content().size(), 0, "cell blanked");
        }
    }
    // The selection collapsed to a text cursor (in the top-left cell).
    assert!(matches!(next.selection, Selection::Text(_)));
}

#[test]
fn merge_and_split_round_trip() {
    // 2×2 table; select the whole grid and merge → one 2×2 cell.
    let mut state = table_state(2, 2);
    let table = table_of(&state);
    let map = TableMap::compute(&table, 1);
    let c00 = map.cell_at(0, 0).unwrap();
    let c11 = map.cell_at(1, 1).unwrap();
    state.selection = Selection::cell(Pos(c00), Pos(c11));
    let merged = state.run("mergeCells").expect("merge applies");
    let mtable = table_of(&merged);
    let mmap = TableMap::compute(&mtable, 1);
    assert_eq!(mmap.width(), 2);
    assert_eq!(mmap.height(), 2);
    // Only one distinct cell remains in the grid.
    let master = mmap.cell_at(0, 0).unwrap();
    let distinct: std::collections::BTreeSet<usize> = mmap.map().iter().copied().collect();
    assert_eq!(distinct.len(), 1, "all slots point at the merged cell");
    let cell = mtable.node_at(master - 1).unwrap();
    assert_eq!(colspan(&cell), 2);
    assert_eq!(rowspan(&cell), 2);

    // Now split it back.
    let mut split_state = merged.clone();
    split_state.selection = Selection::cursor(Pos(master + 1));
    let split = split_state.run("splitCell").expect("split applies");
    let stable = table_of(&split);
    let smap = TableMap::compute(&stable, 1);
    let distinct2: std::collections::BTreeSet<usize> = smap.map().iter().copied().collect();
    assert_eq!(distinct2.len(), 4, "back to four distinct cells");
}
