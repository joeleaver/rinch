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
    Attrs, EditorState, Fragment, Mark, Node, Pos, RemoveMarkStep, ReplaceAroundStep, Schema,
    Selection, Slice, Step, Transaction, default_plugins,
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
    assert_eq!(
        html(&st),
        "<p>ZZZx</p>",
        "my `a` is undone, the peer's text kept"
    );
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

/// A peer's edit that arrives as a replace of the whole paragraph (how
/// `rinch-editor-collab` integrates one): the history maps through only the part
/// that changed, so my edit in the same paragraph still undoes, before and after
/// the peer's char.
#[test]
fn undo_after_a_whole_paragraph_replace_maps_through_only_what_changed() {
    for (text, want) in [
        ("PabXcd", "<p>Pabcd</p><p>ef</p>"),
        ("abXcdP", "<p>abcdP</p><p>ef</p>"),
    ] {
        let st = state("<p>abcd</p><p>ef</p>");
        let st = type_at(&st, 3, "X", 0); // abXcd
        let s = st.schema();
        let p = s
            .branch("paragraph", Fragment::from_node(s.text(text).unwrap()))
            .unwrap();
        let st = remote(&st, |tr| {
            tr.replace_with(0, 7, Fragment::from_node(p)).unwrap();
        });
        let st = st.run("undo").unwrap();
        assert_eq!(html(&st), want, "the peer made {text}");
    }
}

/// PM's `insertInto` checks that the node content goes into takes it: a
/// `ReplaceAroundStep` wrapping bold text in a code block (which allows no marks)
/// fails, as does one wrapping a paragraph in a paragraph.
#[test]
fn a_replace_around_step_does_not_put_content_where_it_is_not_allowed() {
    let st = state("<p><strong>ab</strong></p>");
    let s = st.schema();
    let code = s.branch("code_block", Fragment::empty()).unwrap();
    let wrap_text = ReplaceAroundStep::new(
        0,
        4,
        1,
        3,
        Slice::new(Fragment::from_node(code.clone()), 0, 0),
        1,
        true,
    );
    assert!(
        wrap_text.apply(&st.doc).is_err(),
        "bold text into a code block"
    );
    let wrap_block = ReplaceAroundStep::new(
        0,
        4,
        0,
        4,
        Slice::new(Fragment::from_node(code), 0, 0),
        1,
        true,
    );
    assert!(
        wrap_block.apply(&st.doc).is_err(),
        "a paragraph into a code block"
    );
    // The control: plain text goes into a code block.
    let st = state("<p>ab</p>");
    let code = st.schema().branch("code_block", Fragment::empty()).unwrap();
    let ok = ReplaceAroundStep::new(
        0,
        4,
        1,
        3,
        Slice::new(Fragment::from_node(code), 0, 0),
        1,
        true,
    );
    assert_eq!(node_to_html(&ok.apply(&st.doc).unwrap()), "<pre>ab</pre>");
}

/// One undo step whose later keystroke deleted text its earlier ones typed, with a
/// peer's insert after it: undoing the delete re-inserts that text, and the earlier
/// steps find their positions inside it again through the mirror of the delete
/// (ProseMirror's `mirrorOffset`), so the whole group undoes.
#[test]
fn undo_of_a_group_that_deleted_its_own_typing_after_a_peers_insert() {
    let st = state("<p>q</p>");
    let st = type_at(&st, 2, "abc", 0); // qabc
    let st = type_at(&st, 4, "X", 10); // qabXc
    let st = local(&st, 20, |tr| {
        tr.delete(3, 6).unwrap(); // bXc → qa
    });
    assert_eq!(html(&st), "<p>qa</p>");
    let st = insert_remote(&st, 1, "PP");
    assert_eq!(html(&st), "<p>PPqa</p>");
    let st = st.run("undo").unwrap();
    assert_eq!(html(&st), "<p>PPq</p>");
    assert!(st.run("undo").is_none(), "one typing group");
    let st = st.run("redo").unwrap();
    assert_eq!(html(&st), "<p>PPqa</p>");
}

/// The selection an undo restores is the one before the edit, mapped through the
/// peer's change: the caret goes back to before my `X`, two chars on.
#[test]
fn undo_restores_the_selection_mapped_through_a_peers_insert() {
    let mut st = state("<p>abcd</p>");
    st.selection = Selection::cursor(Pos(3));
    let st = type_at(&st, 3, "X", 0); // abXcd
    let st = insert_remote(&st, 1, "PP"); // PPabXcd
    let st = st.run("undo").unwrap();
    assert_eq!(html(&st), "<p>PPabcd</p>");
    assert_eq!(st.selection, Selection::cursor(Pos(5)));
}

// --- cost: a peer's change and my keystroke cost the history a flat amount ------------

/// A document of 30 paragraphs with my `X` typed into the first, then `n` peer
/// keystrokes in paragraph 15, each applied the way the collab integration applies
/// one (a replace of the whole paragraph, not recorded).
fn after_peer_keystrokes(n: usize) -> EditorState {
    let paras: String = (0..30).map(|i| format!("<p>paragraph {i}</p>")).collect();
    let st = state(&paras);
    let mut st = type_at(&st, 3, "X", 0);
    let s = st.schema().clone();
    for k in 0..n {
        let start: usize = (0..15).map(|i| st.doc.child(i).node_size()).sum();
        let old = st.doc.child(15).clone();
        let mut text = String::new();
        for i in 0..old.child_count() {
            text.push_str(old.child(i).text().unwrap_or(""));
        }
        // Keep the paragraph short, so only the history could grow.
        if text.len() > 40 {
            text = "p".into();
        }
        text.push((b'A' + (k % 26) as u8) as char);
        let p = s
            .branch("paragraph", Fragment::from_node(s.text(&text).unwrap()))
            .unwrap();
        st = remote(&st, |tr| {
            tr.replace_with(start, start + old.node_size(), Fragment::from_node(p))
                .unwrap();
        });
    }
    st
}

fn work_of<T>(f: impl FnOnce() -> T) -> (u64, u64, T) {
    let (w, c) = (
        rinch_editor_core::plugins::__history_work(),
        rinch_editor_core::transform::step_map::__step_map_clones(),
    );
    let out = f();
    (
        rinch_editor_core::plugins::__history_work() - w,
        rinch_editor_core::transform::step_map::__step_map_clones() - c,
        out,
    )
}

/// Review of #1889: the history used to copy its whole item list on every
/// transaction and keep every peer change's map, so a peer's keystroke after 64k of
/// them cost 117 µs (2 µs on main) and a local one 456 µs. The list is persistent
/// and a run of peer changes folds into one item, so each costs the same however
/// many came before; an undo copies the maps of its own event, not every map since.
#[test]
fn a_peers_change_my_keystroke_and_my_undo_cost_the_same_after_100_or_3000_peer_changes() {
    let cost = |n: usize| {
        let st = after_peer_keystrokes(n);
        let peer = work_of(|| after_one_more(&st)).0;
        let key = work_of(|| type_at(&st, 4, "Y", 10_000)).0;
        let (undo_work, undo_clones, undone) = work_of(|| st.run("undo").unwrap());
        assert_eq!(
            node_to_html(&Node::clone(undone.doc.child(0))),
            "<p>paragraph 0</p>",
            "the undo reverted my X"
        );
        (peer, key, undo_work, undo_clones)
    };
    let small = cost(100);
    let large = cost(3000);
    assert_eq!(
        small, large,
        "(peer, keystroke, undo, undo map clones) work"
    );
    assert!(small.2 <= 8 && small.3 <= 8, "{small:?}");
}

fn after_one_more(st: &EditorState) -> EditorState {
    let s = st.schema().clone();
    let start: usize = (0..15).map(|i| st.doc.child(i).node_size()).sum();
    let old = st.doc.child(15).clone();
    let p = s
        .branch("paragraph", Fragment::from_node(s.text("pZ").unwrap()))
        .unwrap();
    remote(st, |tr| {
        tr.replace_with(start, start + old.node_size(), Fragment::from_node(p))
            .unwrap();
    })
}

/// Past the depth cap the oldest events go, and so do the maps before them: the
/// history holds the cap's events however long the session.
#[test]
fn events_past_the_depth_cap_are_dropped_with_their_maps() {
    let schema = Rc::new(Schema::starter_kit());
    let slice = slice_from_html(&schema, "<p>abc</p>").unwrap();
    let doc = schema.branch("doc", slice.content.clone()).unwrap();
    let mut st = EditorState::create(
        schema,
        doc,
        vec![
            Rc::new(rinch_editor_core::BaseCommandsPlugin) as Rc<dyn rinch_editor_core::Plugin>,
            Rc::new(rinch_editor_core::HistoryPlugin::with(500, 5)),
        ],
    );
    for i in 0..200u64 {
        st = type_at(&st, 2, "x", i * 1000);
        st = insert_remote(&st, 1, "P");
    }
    let depth = |st: &EditorState| {
        st.plugin_state::<rinch_editor_core::plugins::HistoryState>(
            rinch_editor_core::plugins::HISTORY_KEY,
        )
        .unwrap()
        .undo_depth()
    };
    assert_eq!(depth(&st), 5);
    // An undo walks only its own event and what came after it.
    let (w, _, st) = work_of(|| st.run("undo").unwrap());
    assert!(w <= 8, "undo walked {w} items");
    let mut st = st;
    let mut undos = 1;
    while let Some(n) = st.run("undo") {
        st = n;
        undos += 1;
    }
    assert_eq!(undos, 5);
}

/// An undo of a long typing group after a peer's change maps each of its steps
/// through a slice of one mapping. A slice shares the maps: the undo copies each
/// map a few times, not the whole mapping once per step (which was 50 × 50 here).
#[test]
fn an_undo_maps_each_step_without_copying_the_mapping() {
    let mut st = state("<p>abc</p>");
    for i in 0..50u64 {
        st = type_at(&st, 2 + i as usize, "x", i);
    }
    let st = insert_remote(&st, 1, "P");
    let (_, clones, undone) = work_of(|| st.run("undo").unwrap());
    assert_eq!(html(&undone), "<p>Pabc</p>");
    assert!(clones <= 5 * 50, "{clones} step maps copied");
}
