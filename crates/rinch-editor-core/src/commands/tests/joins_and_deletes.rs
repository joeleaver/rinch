//! Backspace and Delete: joins, cross-wrapper lifts and atoms.

use super::*;

#[test]
fn backspace_at_paragraph_after_list_moves_it_into_the_list() {
    // doc(bullet_list(li(p"a"), li(p"b")), paragraph "c"); Backspace at the start
    // of "c" moves it into the list as a third item (PM deleteBarrier findWrapping).
    let s = Rc::new(Schema::starter_kit());
    let li = |t: &str| {
        s.branch(
            "list_item",
            Fragment::from_node(
                s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
                    .unwrap(),
            ),
        )
        .unwrap()
    };
    let list = s
        .branch(
            "bullet_list",
            Fragment::from_children(vec![li("a"), li("b")]),
        )
        .unwrap();
    let para = s
        .branch("paragraph", Fragment::from_node(s.text("c").unwrap()))
        .unwrap();
    let doc = s
        .branch("doc", Fragment::from_children(vec![list, para]))
        .unwrap();
    let mut state = EditorState::create(s, doc, vec![Rc::new(BaseCommandsPlugin)]);

    // Cursor at the start of "c" — just inside the paragraph after the list.
    let list_size = state.doc.child(0).node_size();
    state.selection = Selection::cursor(Pos(list_size + 1));

    let next = state
        .run("deleteCharBackward")
        .expect("backspace moves the paragraph into the list");
    assert_eq!(
        next.doc.child_count(),
        1,
        "only the list remains at top level"
    );
    let list = next.doc.child(0);
    assert_eq!(list.type_name(), "bullet_list");
    assert_eq!(list.child_count(), 3, "a third list item was added");
    assert_eq!(list.child(2).type_name(), "list_item");
    assert_eq!(list.child(2).child(0).type_name(), "paragraph");
    assert_eq!(all_text(list.child(2)), "c");
    // The first two items are untouched.
    assert_eq!(all_text(list.child(0)), "a");
    assert_eq!(all_text(list.child(1)), "b");
}

#[test]
fn backspace_joins_blocks() {
    let s = Rc::new(Schema::starter_kit());
    let p = |t: &str| {
        s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
            .unwrap()
    };
    let doc = s
        .branch("doc", Fragment::from_children(vec![p("ab"), p("cd")]))
        .unwrap();
    let mut state = EditorState::create(s, doc, vec![Rc::new(BaseCommandsPlugin)]);
    state.selection = Selection::cursor(Pos(5)); // start of second paragraph
    let joined = state.run("deleteCharBackward").expect("join applies");
    assert_eq!(joined.doc.child_count(), 1);
    assert_eq!(all_text(&joined.doc), "abcd");
}

#[test]
fn delete_forward_joins_next_block() {
    let s = Rc::new(Schema::starter_kit());
    let p = |t: &str| {
        s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
            .unwrap()
    };
    let doc = s
        .branch("doc", Fragment::from_children(vec![p("ab"), p("cd")]))
        .unwrap();
    let mut state = EditorState::create(s, doc, vec![Rc::new(BaseCommandsPlugin)]);
    state.selection = Selection::cursor(Pos(3)); // end of first paragraph
    let joined = state
        .run("deleteCharForward")
        .expect("forward-join applies");
    assert_eq!(joined.doc.child_count(), 1);
    assert_eq!(all_text(&joined.doc), "abcd");
    // at the very end of the doc, forward-delete does nothing
    let mut at_end = editor("xy");
    at_end.selection = Selection::cursor(Pos(3));
    assert!(!at_end.can_run("deleteCharForward"));
}

#[test]
fn delete_forward_removes_char_mid_text() {
    let mut state = editor("abc");
    state.selection = Selection::cursor(Pos(2)); // between a,b
    let next = state.run("deleteCharForward").unwrap();
    assert_eq!(all_text(&next.doc), "ac");
}

#[test]
fn backspace_deletes_char_mid_text() {
    let mut state = editor("abc");
    state.selection = Selection::cursor(Pos(4)); // after "abc"
    let next = state.run("deleteCharBackward").unwrap();
    assert_eq!(all_text(&next.doc), "ab");
}

// ── cross-wrapper joins (M5.3 — the M4 `deleteBarrier` deferral) ──────────

#[test]
fn backspace_lifts_paragraph_out_of_blockquote() {
    // doc(blockquote(p"abc")); cursor at start of the paragraph (pos 2).
    let s = Rc::new(Schema::starter_kit());
    let bq = s
        .branch("blockquote", Fragment::from_node(p_of(&s, "abc")))
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(bq)).unwrap();
    let state = state_of(doc, s, 2);
    let lifted = state.run("deleteCharBackward").expect("backspace lifts");
    // The paragraph escaped the blockquote (the M4-deferred case is no longer a no-op).
    assert!(!in_node_type(&lifted, "blockquote"));
    assert_eq!(lifted.doc.child(0).type_name(), "paragraph");
    assert_eq!(all_text(&lifted.doc), "abc");
}

#[test]
fn backspace_joins_sibling_paragraphs_inside_blockquote() {
    // doc(blockquote(p"a", p"b")); cursor at start of the 2nd paragraph.
    let s = Rc::new(Schema::starter_kit());
    let bq = s
        .branch(
            "blockquote",
            Fragment::from_children(vec![p_of(&s, "a"), p_of(&s, "b")]),
        )
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(bq)).unwrap();
    // positions: 0[bq 1[p 2 a 3]4[p 5 b 6]7]8 ; start of 2nd para = pos 5
    let state = state_of(doc, s, 5);
    let joined = state
        .run("deleteCharBackward")
        .expect("in-wrapper join applies");
    let bq = joined.doc.child(0);
    assert_eq!(bq.type_name(), "blockquote");
    assert_eq!(bq.child_count(), 1, "the two paragraphs joined into one");
    assert_eq!(all_text(&joined.doc), "ab");
}

#[test]
fn backspace_joins_adjacent_list_items() {
    // doc(bullet_list(list_item(p"a"), list_item(p"b"))); cursor in 2nd item.
    let s = Rc::new(Schema::starter_kit());
    let ul = s
        .branch(
            "bullet_list",
            Fragment::from_children(vec![li_of(&s, "a"), li_of(&s, "b")]),
        )
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(ul)).unwrap();
    // start of 2nd list item's paragraph = pos 8
    let state = state_of(doc, s, 8);
    let joined = state
        .run("deleteCharBackward")
        .expect("list-item join applies");
    let ul = joined.doc.child(0);
    assert_eq!(ul.type_name(), "bullet_list");
    assert_eq!(ul.child_count(), 1, "the two items joined into one");
    // PM-faithful: a depth-1 join of the two list items leaves their two
    // paragraphs stacked inside the surviving item (NOT a single "ab"
    // paragraph). A further Backspace would then join the paragraphs.
    let item = ul.child(0);
    assert_eq!(item.type_name(), "list_item");
    assert_eq!(
        item.child_count(),
        2,
        "both paragraphs survive inside the item"
    );
    assert_eq!(all_text(&joined.doc), "ab");
}

#[test]
fn forward_delete_near_block_atom_does_not_panic() {
    // Regression (adversarial Finding 1): forward-Delete at the end of a
    // paragraph followed by `blockquote(hr)` synthesizes a node-selection of
    // the hr, whose block range is degenerate; the lift path must fail
    // gracefully, never panic on an out-of-bounds path index.
    let s = Rc::new(Schema::starter_kit());
    let hr = s.branch("horizontal_rule", Fragment::empty()).unwrap();
    let bq = s.branch("blockquote", Fragment::from_node(hr)).unwrap();
    let doc = s
        .branch("doc", Fragment::from_children(vec![p_of(&s, "a"), bq]))
        .unwrap();
    let state = state_of(doc, s, 2); // end of "a"
    // Must return cleanly (a no-op here), not panic.
    assert!(state.run("deleteCharForward").is_none());
}

#[test]
fn outdent_on_block_atom_node_selection_does_not_panic() {
    // Regression (adversarial Finding 1): the `lift`/outdent path on a
    // node-selection of a block atom must not panic.
    use crate::selection::{NodeSelection, Selection as Sel};
    let s = Rc::new(Schema::starter_kit());
    let hr = s.branch("horizontal_rule", Fragment::empty()).unwrap();
    let bq = s.branch("blockquote", Fragment::from_node(hr)).unwrap();
    let doc = s.branch("doc", Fragment::from_node(bq)).unwrap();
    let mut state = EditorState::create(s, doc, vec![Rc::new(BaseCommandsPlugin)]);
    // Select the hr (positions: 0[bq 1[hr]2]3 → node selection 1..2).
    state.selection = Sel::Node(NodeSelection {
        anchor: Pos(1),
        head: Pos(2),
    });
    // Should not panic; applicability/exec just reports inapplicable.
    let _ = state.can_run("outdent");
    let _ = state.run("outdent");
}

#[test]
fn backspace_at_start_of_first_block_in_doc_is_noop() {
    // A top-level paragraph at the very start of the doc has nothing to join or lift.
    let mut state = editor("abc");
    state.selection = Selection::cursor(Pos(1)); // start of the only paragraph
    assert!(!state.can_run("deleteCharBackward"));
}

#[test]
fn cross_wrapper_backspace_round_trips_under_undo() {
    // The lift is a real, invertible transaction — undo restores the blockquote.
    let s = Rc::new(Schema::starter_kit());
    let bq = s
        .branch("blockquote", Fragment::from_node(p_of(&s, "abc")))
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(bq.clone())).unwrap();
    let mut state = EditorState::create(s, doc, default_plugins());
    state.selection = Selection::cursor(Pos(2));
    let lifted = state.run("deleteCharBackward").expect("lift");
    assert!(!in_node_type(&lifted, "blockquote"));
    let undone = lifted.run("undo").expect("undo applies");
    assert!(in_node_type(&undone, "blockquote"));
    assert_eq!(all_text(&undone.doc), "abc");
}
