//! The history's handling of a step mapped through a change it did not record
//! (a peer's), found by the review of #1938:
//!
//! - #1940: a `BatchStep` (a `mergeCells` undone) that loses a range when mapped
//!   was left unmirrored, so a position inside a range it kept was no longer
//!   recovered, and a later undo silently did nothing.
//! - #1941: `ReplaceAroundStep::map` could produce `from > to`; the step applied
//!   and the history panicked inverting it.
//! - #1942: an undo whose step no longer applies (a peer retyped the block it
//!   reaches into) restored the event's selection unmapped, outside the document.

use std::rc::Rc;

use rinch_editor_core::serialize::{node_to_html, slice_from_html};
use rinch_editor_core::{
    EditorState, Fragment, Mapping, Node, Pos, ReplaceAroundStep, Schema, Selection, Slice, Step,
    StepMap, Transaction, default_plugins,
};

fn state(html: &str) -> EditorState {
    let schema = Rc::new(Schema::starter_kit());
    let slice = slice_from_html(&schema, html).unwrap();
    let doc = schema.branch("doc", slice.content.clone()).unwrap();
    EditorState::create(schema, doc, default_plugins())
}

const T3: &str = "<p>a</p><table>\
    <tr><td><p>c00</p></td><td><p>c01</p></td><td><p>c02</p></td></tr>\
    <tr><td><p>c10</p></td><td><p>c11</p></td><td><p>c12</p></td></tr>\
    <tr><td><p>c20</p></td><td><p>c21</p></td><td><p>c22</p></td></tr>\
    </table><p>z</p>";

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

fn first_text(n: &Node) -> Option<String> {
    if let Some(t) = n.text() {
        return Some(t.to_string());
    }
    (0..n.child_count()).find_map(|i| first_text(n.child(i)))
}

fn merge(st: &EditorState, a: &str, b: &str) -> EditorState {
    let mut tr = st.tr();
    tr.set_selection(Selection::cell(
        Pos(cell(&st.doc, a)),
        Pos(cell(&st.doc, b)),
    ));
    st.apply(tr).run("mergeCells").expect("the cells merge")
}

/// Type `s` at the end of the paragraph in the cell starting `needle`.
fn type_in(st: &EditorState, needle: &str, s: &str) -> EditorState {
    let at = cell(&st.doc, needle) + 2 + needle.len();
    let mut tr = st.tr();
    tr.set_time(0);
    tr.set_selection(Selection::cursor(Pos(at)));
    tr.insert_text(s).unwrap();
    st.apply(tr)
}

/// `theirs`'s table put in place of `mine`'s, unrecorded: a collaboration
/// integrate of a peer's table command.
fn peer_table(mine: &EditorState, theirs: &EditorState) -> EditorState {
    let at = mine.doc.child(0).node_size();
    let old = mine.doc.child(1).clone();
    let new = theirs.doc.child(1).clone();
    let mut tr = mine.tr();
    tr.replace(
        at,
        at + old.node_size(),
        Slice::new(Fragment::from_node(new), 0, 0),
    )
    .unwrap();
    tr.set_add_to_history(false);
    mine.apply(tr)
}

fn unrecorded(st: &EditorState, from: usize, to: usize, slice: Slice) -> EditorState {
    let mut tr = st.tr();
    tr.replace(from, to, slice).unwrap();
    tr.set_add_to_history(false);
    st.apply(tr)
}

/// #1940: I type "q" in a cell, merge c02..c21, a peer merges the left column
/// (the whole table replaced), and I undo twice. The second undo must take my
/// "q" out: the merge's inverse lost the ranges the peer's change deleted
/// across, and the ones it kept still recover the typed position.
#[test]
fn undo_typing_after_a_merge_undone_among_a_peers_merge() {
    for typed_in in ["c02", "c21", "c22", "c12"] {
        let st = state(T3);
        let typed = type_in(&st, typed_in, "q");
        let mine = merge(&typed, "c02", "c21");
        let theirs = merge(&mine, "c00", "c20");
        let peer = peer_table(&mine, &theirs);
        let u1 = peer.run("undo").expect("the merge undoes");
        let u2 = u1.run("undo").expect("the typing undoes");
        let html = node_to_html(&u2.doc);
        assert!(
            !html.contains('q'),
            "typed in {typed_in}: the second undo left my \"q\": {html}\n after the first: {}",
            node_to_html(&u1.doc)
        );
        assert!(u2.selection.to().0 <= u2.doc.content_size());
    }
}

/// #1941: a zero-width `ReplaceAroundStep` mapped across an insertion at its
/// point must not come out with `from > to` (`from` maps after the insertion,
/// `to` before it).
#[test]
fn a_collapsed_replace_around_step_does_not_map_inside_out() {
    let st = state("<p>ab</p><p>cd</p>");
    let schema = st.schema();
    let li = schema
        .create_node("list_item", Default::default(), Fragment::empty())
        .unwrap();
    let step = ReplaceAroundStep::new(
        4,
        4,
        4,
        4,
        Slice::new(Fragment::from_node(li), 0, 0),
        1,
        true,
    );
    let mut mapping = Mapping::new();
    mapping.append_map(StepMap::new(vec![4, 0, 1]));
    if let Some(mapped) = step.map(&mapping) {
        let m = mapped
            .as_any()
            .downcast_ref::<ReplaceAroundStep>()
            .expect("a replace-around step");
        assert!(
            m.from <= m.gap_from && m.gap_from <= m.gap_to && m.gap_to <= m.to,
            "mapped inside out: {m:?}"
        );
    }
    // And a step that is inside out does not apply.
    let bad = ReplaceAroundStep::new(5, 4, 5, 4, Slice::empty(), 0, true);
    assert!(bad.apply(&st.doc).is_err(), "an inside-out step applied");
    // One that would build a document: the gap (the first paragraph) before
    // `from`, wrapped into a blockquote in place of the second.
    let bq = schema
        .create_node("blockquote", Default::default(), Fragment::empty())
        .unwrap();
    let before_from = ReplaceAroundStep::new(
        4,
        8,
        0,
        4,
        Slice::new(Fragment::from_node(bq), 0, 0),
        1,
        true,
    );
    assert!(
        before_from.apply(&st.doc).is_err(),
        "a gap before `from` applied"
    );
}

/// #1941's history: I wrap "cd" in a list, a peer deletes the list, my undo
/// applies a collapsed unwrap, a peer puts a rule at that point, and I redo.
#[test]
fn redo_of_a_collapsed_unwrap_after_a_peers_insert_does_not_panic() {
    let st = state("<p>ab</p><p>cd</p><p>ef</p>");
    let mut s = st.clone();
    s.selection = Selection::cursor(Pos(6));
    let wrapped = s.run("toggleBulletList").expect("wrap");
    let list_to = 4 + wrapped.doc.child(1).node_size();
    let gone = unrecorded(&wrapped, 4, list_to, Slice::empty());
    let undone = gone.run("undo").expect("undo applies");
    let hr = undone
        .schema()
        .create_node("horizontal_rule", Default::default(), Fragment::empty())
        .unwrap();
    let across = unrecorded(&undone, 4, 4, Slice::new(Fragment::from_node(hr), 0, 0));
    if let Some(redone) = across.run("redo") {
        assert!(redone.selection.to().0 <= redone.doc.content_size());
    }
}

/// #1942: I select 13→1 and insert a rule; a peer turns "gh" into a code block,
/// delivered as a block replace that moves no position. My undo's restore of the
/// text no longer fits (its open end joins a paragraph into a code block) and is
/// skipped; the selection it lands must still be inside the document.
#[test]
fn undo_after_a_peers_retype_lands_a_selection_in_the_document() {
    let st = state("<p>a<br>cd</p><ul><li><p></p></li></ul><p>gh</p>");
    let mut s = st.clone();
    s.selection = Selection::text(Pos(13), Pos(1));
    let mine = s.run("insertHorizontalRule").expect("rule");
    let mut m2 = mine.clone();
    m2.selection = Selection::cursor(Pos(mine.doc.content_size() - 2));
    let cmd = m2.command("setCodeBlock").unwrap();
    let got = std::cell::RefCell::new(None);
    let mut d = |tr: Transaction| {
        *got.borrow_mut() = Some(tr);
    };
    assert!(cmd(&m2, Some(&mut d)));
    let target = m2.apply(got.into_inner().unwrap()).doc.clone();
    let peer = unrecorded(
        &mine,
        1,
        mine.doc.content_size(),
        Slice::new(Fragment::from_node(target.child(1).clone()), 0, 0),
    );
    let undone = peer.run("undo").expect("undo");
    let size = undone.doc.content_size();
    let (from, to) = (undone.selection.from().0, undone.selection.to().0);
    assert!(
        to <= size,
        "selection {from}..{to} outside a document of size {size}: {}",
        node_to_html(&undone.doc)
    );
    // And it resolves into text, not just within bounds.
    let head = undone.doc.resolve(undone.selection.head()).unwrap();
    assert!(head.parent().is_textblock(), "{:?}", undone.selection);
}
