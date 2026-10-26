//! #1935: undoing a `mergeCells` after a peer replaced the table (their own merge,
//! as a collaboration integrate brings it: the table replaced whole, a change the
//! history did not record) panicked in `StepMap::recover`.
//!
//! The undo maps the merge's inverse (a `BatchStep`) through the peer's change. A
//! `BatchStep` drops each replace that change deleted across and still applies the
//! rest, so its map had fewer ranges than the merge's own. `pop_event` mirrored the
//! two anyway, and mapping the selection through the merge's map recovered a
//! deleted position into a range of the inverse's map that is not there.

use std::rc::Rc;

use rinch_editor_core::serialize::{node_to_html, slice_from_html};
use rinch_editor_core::{
    EditorState, Fragment, Node, Pos, Schema, Selection, Slice, default_plugins,
};

fn state() -> EditorState {
    let schema = Rc::new(Schema::starter_kit());
    let html = "<p>a</p><table>\
        <tr><td><p>c00</p></td><td><p>c01</p></td><td><p>c02</p></td></tr>\
        <tr><td><p>c10</p></td><td><p>c11</p></td><td><p>c12</p></td></tr>\
        <tr><td><p>c20</p></td><td><p>c21</p></td><td><p>c22</p></td></tr>\
        </table><p>z</p>";
    let slice = slice_from_html(&schema, html).unwrap();
    let doc = schema.branch("doc", slice.content.clone()).unwrap();
    EditorState::create(schema, doc, default_plugins())
}

/// The position before the cell whose text starts with `needle`.
fn cell(doc: &Node, needle: &str) -> usize {
    let mut found = None;
    doc.nodes_between(0, doc.content_size(), &mut |n: &Node, at, _| {
        if found.is_none()
            && n.type_name().ends_with("cell")
            && first_text(n).is_some_and(|t| t.starts_with(needle))
        {
            found = Some(at);
        }
        found.is_none()
    });
    found.unwrap_or_else(|| panic!("no cell `{needle}`"))
}

/// The first text node's text inside `n`.
fn first_text(n: &Node) -> Option<String> {
    if let Some(t) = n.text() {
        return Some(t.to_string());
    }
    (0..n.child_count()).find_map(|i| first_text(n.child(i)))
}

/// The table and the position before it.
fn table(doc: &Node) -> (usize, Node) {
    let at = doc.child(0).node_size();
    (at, doc.child(1).clone())
}

/// Merge the cells from the one starting `a` to the one starting `b`.
fn merge(st: &EditorState, a: &str, b: &str) -> EditorState {
    let mut tr = st.tr();
    tr.set_selection(Selection::cell(
        Pos(cell(&st.doc, a)),
        Pos(cell(&st.doc, b)),
    ));
    st.apply(tr).run("mergeCells").expect("the cells merge")
}

#[test]
fn undoing_my_merge_after_a_peer_replaced_the_table_does_not_panic() {
    let st = state();
    // I merge the right two columns.
    let mine = merge(&st, "c02", "c21");
    // A peer merges the left column, and the integrate replaces the table whole.
    let theirs = merge(&mine, "c00", "c20");
    let (at, old) = table(&mine.doc);
    let (_, new) = table(&theirs.doc);
    let mut tr = mine.tr();
    tr.replace(
        at,
        at + old.node_size(),
        Slice::new(Fragment::from_node(new), 0, 0),
    )
    .unwrap();
    tr.set_add_to_history(false);
    let peer = mine.apply(tr);
    assert_eq!(
        node_to_html(&peer.doc),
        "<p>a</p><table><tr><td rowspan=\"3\"><p>c00</p><p>c10</p><p>c20</p></td>\
         <td colspan=\"2\" rowspan=\"3\"><p>c01</p><p>c02</p><p>c11</p><p>c12</p><p>c21</p>\
         <p>c22</p></td></tr><tr></tr><tr></tr></table><p>z</p>",
        "the setup"
    );
    // The undo maps the merge's inverse through the peer's table: it panicked here.
    let undone = peer.run("undo").expect("the undo applies");
    // The selection it restores is a valid one of the document it made.
    assert!(undone.selection.from().0 <= undone.doc.content_size());
    // The table the undo made ragged is repaired in its own transaction (#1937):
    // the merge's inverse, mapped through the peer's table, restored some cells
    // and kept the spans.
    assert_eq!(
        problems(&undone.doc),
        Vec::<String>::new(),
        "after the undo"
    );
    // And back.
    let redone = undone.run("redo").expect("the redo applies");
    assert!(redone.selection.from().0 <= redone.doc.content_size());
    assert_eq!(
        problems(&redone.doc),
        Vec::<String>::new(),
        "after the redo"
    );
    // The undo and the redo undo each other.
    let again = redone.run("undo").expect("a second undo applies");
    assert_eq!(node_to_html(&again.doc), node_to_html(&undone.doc));
}

/// How the tables of `doc` fail to tile their grids, one entry per problem.
fn problems(doc: &Node) -> Vec<String> {
    let mut out = Vec::new();
    doc.nodes_between(0, doc.content_size(), &mut |n: &Node, at, _| {
        if n.type_name() == "table" {
            for p in rinch_editor_core::tables::table_problems(n) {
                out.push(format!("table at {at}: {p:?}"));
            }
        }
        true
    });
    out
}

/// A table that was ragged before the undo (loaded so: a browser draws it as it is)
/// and that the undo does not touch stays as it is (#1937): only the tables an undo
/// or redo made ragged are repaired.
#[test]
fn an_undo_leaves_a_table_it_did_not_make_ragged_as_it_is() {
    let schema = Rc::new(Schema::starter_kit());
    let html = "<table><tr><td><p>r0</p></td><td><p>r1</p></td></tr><tr><td><p>short</p></td></tr></table>\
        <p>text</p>";
    let slice = slice_from_html(&schema, html).unwrap();
    let doc = schema.branch("doc", slice.content.clone()).unwrap();
    let st = EditorState::create(schema, doc, default_plugins());
    assert_eq!(problems(&st.doc).len(), 1, "the setup: one short row");
    let at = st.doc.child(0).node_size() + 2;
    let mut tr = st.tr();
    tr.set_selection(Selection::cursor(Pos(at)));
    tr.insert_text("x").unwrap();
    let typed = st.apply(tr);
    let undone = typed.run("undo").expect("the undo applies");
    assert_eq!(node_to_html(&undone.doc), node_to_html(&st.doc));
}
