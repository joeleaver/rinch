//! #2210: a cell a table command makes takes the alignment of the cell it is
//! modelled on, so a Markdown pipe table stays a pipe table through it.
//!
//! A pipe table writes one alignment per column (`:---`, `:---:`, `---:`,
//! `---`), and a column whose cells disagree, an explicit `left` beside an
//! unset one, has no delimiter that says so: it is written as an HTML
//! `<table>`. A new cell used to hold an unset paragraph whatever its column
//! said, so `addRowAfter` in a `:---` table turned it into HTML. Now:
//!
//! - `addRowBefore` / `addRowAfter`: each new cell is aligned like the cell of
//!   the row it is inserted beside in its column (the row whose spans it
//!   copies, #1244);
//! - `addColumnBefore` / `addColumnAfter`: each new cell takes the type and
//!   alignment of the cell beside it in its row, from the column on its left
//!   (the right one at the table's left edge), as prosemirror-tables'
//!   `addColumn` picks its reference column; so the header row gets a header
//!   cell;
//! - `splitCell`: the vacated slots get cells of the master's type and
//!   alignment;
//! - clearing a cell selection keeps each cleared cell's alignment.

#![cfg(feature = "markdown")]

use rinch_editor_core::serialize::{doc_from_markdown, doc_to_markdown};
use rinch_editor_core::*;
use std::rc::Rc;

fn state_of(md: &str) -> EditorState {
    let s = Schema::starter_kit();
    let doc = doc_from_markdown(&s, md).expect("markdown loads");
    assert_eq!(doc.child(0).type_name(), "table", "the table is block 0");
    EditorState::create(Rc::new(s), doc, default_plugins())
}

/// The position before row `r`'s cell `i` of the table at doc position 0.
fn cell_pos(doc: &Node, r: usize, i: usize) -> usize {
    let t = doc.child(0);
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

/// Run `cmd` with the caret in row `r`'s cell `i`.
fn run_in(md: &str, r: usize, i: usize, cmd: &str) -> EditorState {
    let mut st = state_of(md);
    st.selection = Selection::cursor(Pos(cell_pos(&st.doc, r, i) + 2));
    st.run(cmd).unwrap_or_else(|| panic!("{cmd} applies"))
}

/// The pipe table's delimiter row, and its row count (header included);
/// panics when the table was written as HTML.
fn pipe_shape(doc: &Node) -> (String, usize) {
    let md = doc_to_markdown(doc);
    assert!(
        md.starts_with('|'),
        "the table is still a pipe table, got:\n{md}"
    );
    let lines: Vec<&str> = md.lines().take_while(|l| l.starts_with('|')).collect();
    (lines[1].to_string(), lines.len() - 1)
}

fn table_md(delim: &str) -> String {
    format!("| a | b |\n| {delim} | --- |\n| 1 | 2 |\n| 3 | 4 |\n")
}

const ALIGNED: [&str; 3] = [":---", ":---:", "---:"];

#[test]
fn add_row_keeps_every_column_alignment() {
    for d in ALIGNED {
        let md = table_md(d);
        let want = format!("| {d} | --- |");
        // A body row, after and before it; and after the header row.
        for (r, cmd) in [(1, "addRowAfter"), (2, "addRowBefore"), (0, "addRowAfter")] {
            let next = run_in(&md, r, 0, cmd);
            assert_eq!(
                pipe_shape(&next.doc),
                (want.clone(), 4),
                "{cmd} in row {r} of a `{d}` table"
            );
        }
    }
}

#[test]
fn add_column_takes_its_neighbours_type_and_alignment() {
    for d in ALIGNED {
        let md = table_md(d);
        // (caret column, command, delimiter row after)
        let cases = [
            // Right of column 0: modelled on column 0.
            (0, "addColumnAfter", format!("| {d} | {d} | --- |")),
            // At the left edge: modelled on the column it pushes right.
            (0, "addColumnBefore", format!("| {d} | {d} | --- |")),
            // Between the columns: modelled on the left one.
            (1, "addColumnBefore", format!("| {d} | {d} | --- |")),
            // Right of the unset column: unset.
            (1, "addColumnAfter", format!("| {d} | --- | --- |")),
        ];
        for (c, cmd, want) in cases {
            for r in [0, 1] {
                let next = run_in(&md, r, c, cmd);
                assert_eq!(
                    pipe_shape(&next.doc),
                    (want.clone(), 3),
                    "{cmd} in row {r} column {c} of a `{d}` table"
                );
            }
        }
    }
}

#[test]
fn clearing_cells_keeps_their_alignment() {
    for d in ALIGNED {
        let mut st = state_of(&table_md(d));
        let a = cell_pos(&st.doc, 1, 0);
        let b = cell_pos(&st.doc, 2, 1);
        st.selection = Selection::cell(Pos(a), Pos(b));
        let next = st.run("deleteCellSelection").expect("clears the cells");
        assert_eq!(
            next.doc.child(0).child(1).child(0).content_size(),
            2,
            "the cell was cleared"
        );
        assert_eq!(
            pipe_shape(&next.doc),
            (format!("| {d} | --- |"), 3),
            "clearing a `{d}` table"
        );
    }
}

/// A 2-row table whose header row is one `colspan=2` header cell aligned
/// `center`, above two centred body cells.
fn merged_header_state() -> EditorState {
    let s = Schema::starter_kit();
    let para = |t: &str| {
        s.create_node(
            "paragraph",
            Attrs::from_iter([("text_align", AttrValue::from("center"))]),
            Fragment::from_node(s.text(t).unwrap()),
        )
        .unwrap()
    };
    let cell = |ty: &str, colspan: i64, t: &str| {
        s.create_node(
            ty,
            Attrs::from_iter([("colspan", AttrValue::Int(colspan))]),
            Fragment::from_node(para(t)),
        )
        .unwrap()
    };
    let row = |cells: Vec<Node>| {
        s.branch("table_row", Fragment::from_children(cells))
            .unwrap()
    };
    let table = s
        .branch(
            "table",
            Fragment::from_children(vec![
                row(vec![cell("table_header_cell", 2, "h")]),
                row(vec![cell("table_cell", 1, "1"), cell("table_cell", 1, "2")]),
            ]),
        )
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(table)).unwrap();
    EditorState::create(Rc::new(s), doc, default_plugins())
}

#[test]
fn split_cell_fills_with_the_masters_type_and_alignment() {
    let mut st = merged_header_state();
    st.selection = Selection::cursor(Pos(cell_pos(&st.doc, 0, 0) + 2));
    let next = st.run("splitCell").expect("splits the merged cell");
    let header = next.doc.child(0).child(0);
    assert_eq!(header.child_count(), 2);
    let made = header.child(1);
    assert_eq!(made.type_name(), "table_header_cell");
    assert_eq!(made.child(0).attrs().get_str("text_align"), Some("center"));
    assert_eq!(pipe_shape(&next.doc), ("| :---: | :---: |".to_string(), 2));
}
