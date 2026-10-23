//! Undo and redo after a change the history did not record (#1249, #1615, #1856).
//!
//! A collaboration peer's edit reaches the editor as a transaction with
//! `add_to_history(false)`. The history used to ignore such a transaction, so an undo
//! after it replayed its inverse steps at their old positions: it reverted a peer's
//! text instead of the local edit, or (for a mark step) left adjacent text nodes with
//! equal marks unjoined. The history now records each such change's step maps and maps
//! the undone steps through them, as ProseMirror's history does (`addMaps`,
//! `popEvent`).

use std::rc::Rc;

use rinch_editor_core::serialize::{node_to_html, slice_from_html};
use rinch_editor_core::{
    Attrs, EditorState, Fragment, Mark, Node, Pos, RemoveMarkStep, Schema, Selection, Step, Transaction, default_plugins,
};

fn state(html: &str) -> EditorState {
    let schema = Rc::new(Schema::starter_kit());
    let slice = slice_from_html(&schema, html).unwrap();
    let doc = schema.branch("doc", slice.content.clone()).unwrap();
    EditorState::create(schema, doc, default_plugins())
}

fn html(st: &EditorState) -> String {
    node_to_html(&st.doc)
}

/// A local edit: `build` fills a transaction stamped `time` (spaced apart so each is
/// its own undo step unless a test wants them merged).
fn local(st: &EditorState, time: u64, build: impl FnOnce(&mut Transaction)) -> EditorState {
    let mut tr = st.tr();
    tr.set_time(time);
    build(&mut tr);
    st.apply(tr)
}

/// A peer's edit: not recorded in the history.
fn remote(st: &EditorState, build: impl FnOnce(&mut Transaction)) -> EditorState {
    let mut tr = st.tr();
    build(&mut tr);
    tr.set_add_to_history(false);
    st.apply(tr)
}

fn text(st: &EditorState, s: &str) -> Fragment {
    Fragment::from_node(st.schema().text(s).unwrap())
}

fn type_at(st: &EditorState, at: usize, s: &str, time: u64) -> EditorState {
    local(st, time, |tr| {
        tr.set_selection(Selection::cursor(Pos(at)));
        tr.insert_text(s).unwrap();
    })
}

fn insert_remote(st: &EditorState, at: usize, s: &str) -> EditorState {
    let f = text(st, s);
    remote(st, |tr| {
        tr.replace_with(at, at, f).unwrap();
    })
}

fn bold(st: &EditorState, from: usize, to: usize) -> EditorState {
    let mut s = st.clone();
    s.selection = Selection::text(Pos(from), Pos(to));
    s.run("toggleBold").expect("bold applies")
}

/// No textblock holds two adjacent text nodes with equal marks (the model a reader of
/// the same HTML builds).
fn assert_normalized(doc: &Node) {
    fn walk(n: &Node) {
        if n.is_textblock() {
            for i in 1..n.child_count() {
                let (a, b) = (n.child(i - 1), n.child(i));
                assert!(
                    !(a.is_text() && b.is_text() && a.marks() == b.marks()),
                    "unjoined text nodes {a:?} {b:?} in {n:?}"
                );
            }
            return;
        }
        for i in 0..n.child_count() {
            walk(n.child(i));
        }
    }
    walk(doc);
}

/// #1615's repro: a peer inserts before my keystroke, then I undo it.
#[test]
fn undo_after_a_peers_insert_before_my_edit_reverts_my_edit() {
    let st = state("<p>x</p>");
    let st = type_at(&st, 2, "a", 0);
    assert_eq!(html(&st), "<p>xa</p>");
    let st = insert_remote(&st, 1, "ZZZ");
    assert_eq!(html(&st), "<p>ZZZxa</p>");
    let st = st.run("undo").expect("undo applies");
    assert_eq!(html(&st), "<p>ZZZx</p>", "my `a` is undone, the peer's text kept");
    let st = st.run("redo").expect("redo applies");
    assert_eq!(html(&st), "<p>ZZZxa</p>");
}

/// #1249's real-editor shape, without collab: a peer's paragraph above my edit.
#[test]
fn undo_after_a_peers_paragraph_above_my_edit_reverts_my_edit() {
    let st = state("<p>tail</p><p>PEER</p>");
    let st = type_at(&st, 5, "XYZ", 0);
    assert_eq!(html(&st), "<p>tailXYZ</p><p>PEER</p>");
    let top = {
        let s = st.schema();
        s.branch("paragraph", Fragment::from_node(s.text("TOPTOP").unwrap()))
            .unwrap()
    };
    let st = remote(&st, |tr| {
        tr.replace_with(0, 0, Fragment::from_node(top)).unwrap();
    });
    let st = st.run("undo").expect("undo applies");
    assert_eq!(html(&st), "<p>TOPTOP</p><p>tail</p><p>PEER</p>");
}

/// A peer's edit after mine in the document moves nothing of mine: the undo reverts my
/// edit and keeps theirs (a peer's insert *after* the edit, off the fixed point of an
/// insert at the start).
#[test]
fn undo_keeps_a_peers_insert_after_my_edit() {
    let st = state("<p>abcd</p>");
    let st = type_at(&st, 2, "X", 0);
    let st = insert_remote(&st, 5, "PP");
    assert_eq!(html(&st), "<p>aXbcPPd</p>");
    let st = st.run("undo").unwrap();
    assert_eq!(html(&st), "<p>abcPPd</p>");
}

/// Two of my edits with a peer's in between, each its own undo step: both undo at
/// their mapped positions, and both redo.
#[test]
fn two_undos_and_two_redos_around_a_peers_insert() {
    let st = state("<p>abcd</p>");
    let st = type_at(&st, 3, "X", 0); // abXcd
    let st = insert_remote(&st, 1, "PP"); // PPabXcd
    let st = type_at(&st, 7, "Y", 1000); // PPabXcYd
    assert_eq!(html(&st), "<p>PPabXcYd</p>");
    let st = insert_remote(&st, 4, "QQ"); // PPaQQbXcYd
    assert_eq!(html(&st), "<p>PPaQQbXcYd</p>");
    let st = st.run("undo").unwrap();
    assert_eq!(html(&st), "<p>PPaQQbXcd</p>");
    let st = st.run("undo").unwrap();
    assert_eq!(html(&st), "<p>PPaQQbcd</p>");
    assert!(st.run("undo").is_none(), "nothing left to undo");
    let st = st.run("redo").unwrap();
    assert_eq!(html(&st), "<p>PPaQQbXcd</p>");
    let st = st.run("redo").unwrap();
    assert_eq!(html(&st), "<p>PPaQQbXcYd</p>");
}

/// Typing merged into one undo step across a peer's edit (within the window): the
/// one undo reverts both keystrokes and nothing of the peer's.
#[test]
fn one_typing_group_spanning_a_peers_insert_undoes_both_keystrokes() {
    let st = state("<p>abcd</p>");
    let st = type_at(&st, 3, "X", 0); // abXcd
    let st = insert_remote(&st, 2, "PP"); // aPPbXcd
    let st = type_at(&st, 6, "Y", 10); // aPPbXYcd
    assert_eq!(html(&st), "<p>aPPbXYcd</p>");
    let st = st.run("undo").unwrap();
    assert_eq!(html(&st), "<p>aPPbcd</p>");
    assert!(st.run("undo").is_none());
    let st = st.run("redo").unwrap();
    assert_eq!(html(&st), "<p>aPPbXYcd</p>");
}

/// A peer deletes the text I typed: undoing my insert has nothing left to delete. The
/// undo is consumed and changes nothing of the peer's.
#[test]
fn undo_of_text_a_peer_deleted_changes_nothing_else() {
    let st = state("<p>abcd</p>");
    let st = type_at(&st, 3, "XY", 0); // abXYcd
    let st = remote(&st, |tr| {
        tr.delete(2, 6).unwrap(); // a|d... deletes bXYc
    });
    assert_eq!(html(&st), "<p>ad</p>");
    let st = st.run("undo").expect("the undo is consumed");
    assert_eq!(html(&st), "<p>ad</p>");
}

/// #1856 (seed 322's shape): I bold a range, a peer deletes text before it, I undo.
/// The bold comes off exactly the range I bolded, and the text nodes it leaves join.
#[test]
fn undo_of_a_bold_after_a_peers_delete_before_it_unbolds_my_range() {
    let st = state("<p>ab</p><p>cXd</p>");
    // `cX` is 5..7.
    let st = bold(&st, 5, 7);
    assert_eq!(html(&st), "<p>ab</p><p><strong>cX</strong>d</p>");
    let st = remote(&st, |tr| {
        tr.delete(1, 3).unwrap();
    });
    assert_eq!(html(&st), "<p></p><p><strong>cX</strong>d</p>");
    let st = st.run("undo").unwrap();
    assert_eq!(html(&st), "<p></p><p>cXd</p>");
    assert_normalized(&st.doc);
}

/// A peer types inside the range I bolded; undoing the bold unbolds the whole mapped
/// range, the peer's plain char included, and leaves one text node.
#[test]
fn undo_of_a_bold_with_a_peers_char_inside_leaves_one_text_node() {
    let st = state("<p>abcd</p>");
    let st = bold(&st, 2, 4); // a<b>bc</b>d
    let st = insert_remote(&st, 3, "P"); // a<b>b</b>P<b>c</b>d — the peer's char is plain
    let st = st.run("undo").unwrap();
    assert_eq!(html(&st), "<p>abPcd</p>");
    assert_normalized(&st.doc);
}

/// The step itself: removing a mark across text nodes that end up with equal marks
/// joins them, as ProseMirror's `mapFragment` (`Fragment.fromArray`) does.
#[test]
fn remove_mark_step_joins_the_text_nodes_it_leaves_equal() {
    let st = state("<p><strong>a</strong>X<strong>b</strong>c</p>");
    let bold = st.schema().mark_type("bold").unwrap().clone();
    let strong = Mark::new(bold, Attrs::new());
    let step = RemoveMarkStep::new(1, 4, strong);
    let doc = step.apply(&st.doc).unwrap();
    assert_eq!(node_to_html(&doc), "<p>aXbc</p>");
    assert_normalized(&doc);
}
