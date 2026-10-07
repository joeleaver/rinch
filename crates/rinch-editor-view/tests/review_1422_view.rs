//! Review of #1422 through the handle: the delete, its undo and redo, and
//! typing and pasting over the same selection.
use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{Pos, Selection};
use rinch_editor_view::create_editor;

const NESTED: &str = "<ul><li><p>one</p><ul><li><p>two</p></li></ul></li></ul>";

#[test]
fn delete_then_undo_through_the_handle() {
    let ed = create_editor();
    ed.load_html(NESTED);
    // "o|ne" .. "tw|o": 4 and 12.
    ed.set_selection(Selection::text(Pos(4), Pos(12)));
    assert!(ed.command("deleteCharBackward"));
    assert_eq!(node_to_html(&ed.doc()), "<ul><li><p>oo</p></li></ul>");
    assert!(ed.selection().is_empty());
    assert_eq!(ed.selection().from().0, 4);
    assert!(ed.command("undo"));
    assert_eq!(node_to_html(&ed.doc()), NESTED);
    assert!(ed.command("redo"));
    assert_eq!(node_to_html(&ed.doc()), "<ul><li><p>oo</p></li></ul>");
}

/// Typing over the same selection: the handle deletes the selection (fitted)
/// and then inserts. A text paste goes through `replace_range`.
#[test]
fn typing_and_pasting_over_a_selection_across_depths() {
    let ed = create_editor();
    ed.load_html(NESTED);
    ed.set_selection(Selection::text(Pos(4), Pos(12)));
    assert!(ed.insert_text("X"));
    assert_eq!(node_to_html(&ed.doc()), "<ul><li><p>oXo</p></li></ul>");
    let ed = create_editor();
    ed.load_html(NESTED);
    ed.set_selection(Selection::text(Pos(4), Pos(12)));
    assert!(ed.replace_selection_with_text("X"));
    assert_eq!(node_to_html(&ed.doc()), "<ul><li><p>oXo</p></li></ul>");
}
