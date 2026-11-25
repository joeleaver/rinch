//! Inserting hard breaks, rules, blocks and tables.

use super::*;

#[test]
fn insert_hard_break_and_hr() {
    let mut state = editor("ab");
    state.selection = Selection::cursor(Pos(2));
    let hb = state.run("insertHardBreak").unwrap();
    assert!(
        hb.doc
            .child(0)
            .content()
            .iter()
            .any(|n| n.type_name() == "hard_break")
    );

    let mut state2 = editor("ab");
    state2.selection = Selection::cursor(Pos(3)); // end of "ab"
    // hr inserts at a block gap; place cursor at the doc-level gap after the para
    state2.selection = Selection::text(Pos(4), Pos(4));
    let hr = state2.run("insertHorizontalRule").unwrap();
    assert!(
        hr.doc
            .content()
            .iter()
            .any(|n| n.type_name() == "horizontal_rule")
    );
}

#[test]
fn insert_block_at_a_cursor_inside_a_textblock() {
    // End of the paragraph → the rule goes after it: [p "ab"][hr].
    let mut s = editor("ab");
    s.selection = Selection::cursor(Pos(3));
    let end = s.run("insertHorizontalRule").expect("hr at para end");
    assert_eq!(end.doc.child_count(), 2);
    assert_eq!(end.doc.child(0).type_name(), "paragraph");
    assert_eq!(end.doc.child(1).type_name(), "horizontal_rule");
    assert_eq!(all_text(end.doc.child(0)), "ab");

    // Start of the paragraph → the rule goes before it: [hr][p "ab"].
    let mut s = editor("ab");
    s.selection = Selection::cursor(Pos(1));
    let start = s.run("insertHorizontalRule").expect("hr at para start");
    assert_eq!(start.doc.child_count(), 2);
    assert_eq!(start.doc.child(0).type_name(), "horizontal_rule");
    assert_eq!(start.doc.child(1).type_name(), "paragraph");
    assert_eq!(all_text(start.doc.child(1)), "ab");

    // Interior → split the paragraph around the rule: [p "a"][hr][p "b"].
    let mut s = editor("ab");
    s.selection = Selection::cursor(Pos(2));
    let mid = s.run("insertHorizontalRule").expect("hr mid para");
    assert_eq!(mid.doc.child_count(), 3);
    assert_eq!(mid.doc.child(0).type_name(), "paragraph");
    assert_eq!(mid.doc.child(1).type_name(), "horizontal_rule");
    assert_eq!(mid.doc.child(2).type_name(), "paragraph");
    assert_eq!(all_text(mid.doc.child(0)), "a");
    assert_eq!(all_text(mid.doc.child(2)), "b");

    // A table inserts the same way at an interior cursor (the user's bug).
    let mut s = editor("ab");
    s.selection = Selection::cursor(Pos(2));
    let tbl = s.run("insertTable").expect("table mid para");
    assert_eq!(tbl.doc.child_count(), 3);
    assert_eq!(tbl.doc.child(1).type_name(), "table");
}

#[test]
fn insert_table_creates_a_grid_and_undoes() {
    let s = Rc::new(Schema::starter_kit());
    let para = s
        .branch("paragraph", Fragment::from_node(s.text("ab").unwrap()))
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(para)).unwrap();
    let mut state = EditorState::create(s, doc, default_plugins());
    // Insert at the doc-level gap after the paragraph (0[p 1 a 2 b 3]4).
    state.selection = Selection::text(Pos(4), Pos(4));
    let next = state.run("insertTable").expect("insertTable applies");

    let table = next
        .doc
        .content()
        .iter()
        .find(|n| n.type_name() == "table")
        .expect("table inserted");
    assert_eq!(table.content().child_count(), 3, "3 rows");
    for r in 0..3 {
        let row = table.content().child(r);
        assert_eq!(row.type_name(), "table_row");
        assert_eq!(row.content().child_count(), 3, "3 cells per row");
        for c in 0..3 {
            let cell = row.content().child(c);
            assert_eq!(cell.type_name(), "table_cell");
            assert_eq!(cell.content().child(0).type_name(), "paragraph");
        }
    }

    // Structural insert is a single undo step.
    let undone = next.run("undo").expect("undo applies");
    assert!(
        undone
            .doc
            .content()
            .iter()
            .all(|n| n.type_name() != "table"),
        "undo removed the table"
    );
    assert_eq!(all_text(&undone.doc), "ab");
}
