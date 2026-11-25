//! Enter: splitting blocks and list items, and leaving a list.

use super::*;

#[test]
fn split_block_in_paragraph() {
    let mut state = editor("ab");
    state.selection = Selection::cursor(Pos(2)); // between a,b
    let split = state.run("splitBlock").expect("split applies");
    assert_eq!(split.doc.child_count(), 2);
    assert_eq!(all_text(split.doc.child(0)), "a");
    assert_eq!(all_text(split.doc.child(1)), "b");
    // cursor is at the start of the second block
    assert_eq!(split.selection, Selection::cursor(Pos(4)));
}

#[test]
fn split_at_end_of_heading_makes_paragraph() {
    let s = Rc::new(Schema::starter_kit());
    let h = s
        .branch("heading", Fragment::from_node(s.text("T").unwrap()))
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(h)).unwrap();
    let mut state = EditorState::create(s, doc, vec![Rc::new(BaseCommandsPlugin)]);
    state.selection = Selection::cursor(Pos(2)); // end of heading "T"
    let split = state.run("splitBlock").unwrap();
    assert_eq!(split.doc.child(0).type_name(), "heading");
    assert_eq!(split.doc.child(1).type_name(), "paragraph");
}

// ── Enter in lists (splitListItem / liftEmptyBlock) ───────────────────────

fn list_doc(s: &Schema, items: &[&str]) -> Node {
    let lis: Vec<Node> = items.iter().map(|t| li_of(s, t)).collect();
    let list = s
        .branch("bullet_list", Fragment::from_children(lis))
        .unwrap();
    s.branch("doc", Fragment::from_node(list)).unwrap()
}

#[test]
fn enter_in_list_item_splits_into_new_item() {
    // doc(bullet_list(list_item(p"ab"))); cursor between a,b. Positions are
    // 0-based in doc content: list@0(c@1) item@1(c@2) p@2(c@3) "a"@3 "b"@4.
    let s = Rc::new(Schema::starter_kit());
    let state = state_of(list_doc(&s, &["ab"]), s, 4);
    let next = state.run("enter").expect("enter splits the list item");
    let list = next.doc.child(0);
    assert_eq!(list.type_name(), "bullet_list");
    assert_eq!(list.child_count(), 2, "split produced two list items");
    assert_eq!(list.child(0).type_name(), "list_item");
    assert_eq!(list.child(1).type_name(), "list_item");
    assert_eq!(all_text(list.child(0)), "a");
    assert_eq!(all_text(list.child(1)), "b");
}

#[test]
fn enter_at_end_of_list_item_makes_empty_item() {
    // doc(bullet_list(list_item(p"ab"))); cursor at end of "ab" (after 'b').
    let s = Rc::new(Schema::starter_kit());
    let state = state_of(list_doc(&s, &["ab"]), s, 5);
    let next = state.run("enter").expect("enter adds a new item");
    let list = next.doc.child(0);
    assert_eq!(list.child_count(), 2);
    assert_eq!(all_text(list.child(0)), "ab");
    assert_eq!(all_text(list.child(1)), "", "new item is empty");
    assert_eq!(list.child(1).child(0).type_name(), "paragraph");
}

#[test]
fn enter_in_empty_trailing_list_item_exits_list() {
    // doc(bullet_list(list_item(p"ab"), list_item(p""))); cursor in the empty
    // 2nd item — Enter should lift it out of the list.
    let s = Rc::new(Schema::starter_kit());
    let empty_li = s
        .branch(
            "list_item",
            Fragment::from_node(s.branch("paragraph", Fragment::empty()).unwrap()),
        )
        .unwrap();
    let list = s
        .branch(
            "bullet_list",
            Fragment::from_children(vec![li_of(&s, "ab"), empty_li]),
        )
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(list)).unwrap();
    // 0-based: list@0(c@1) li1@1(c@2) p"ab"@2(c@3) "ab"@3-5 li1.close@6 li2@7(c@8) p""@8(c@9)
    let state = state_of(doc, s, 9);
    let next = state.run("enter").expect("enter exits the empty item");
    // The empty item left the list: a top-level paragraph now follows a
    // single-item list (the paragraph is no longer inside any list).
    assert!(
        !in_node_type(&next, "bullet_list"),
        "cursor block escaped the list"
    );
    assert_eq!(next.doc.child(0).type_name(), "bullet_list");
    assert_eq!(
        next.doc.child(0).child_count(),
        1,
        "list shrank to one item"
    );
    assert_eq!(next.doc.child(1).type_name(), "paragraph");
}

#[test]
fn enter_in_plain_paragraph_still_splits_block() {
    // The chain falls through to split_block when not in a list item.
    let mut state = editor("ab");
    state.selection = Selection::cursor(Pos(2));
    let next = state.run("enter").expect("enter splits the paragraph");
    assert_eq!(next.doc.child_count(), 2);
    assert_eq!(all_text(next.doc.child(0)), "a");
    assert_eq!(all_text(next.doc.child(1)), "b");
}

#[test]
fn enter_in_a_task_item_creates_a_new_unchecked_item() {
    // Pressing Enter at the end of a *checked* task item must yield a fresh, separate,
    // *unchecked* task item — not a second paragraph in the same item, and not an
    // inherited tick. (Mirrors the list_item Enter path, made task-aware.)
    let s = Rc::new(Schema::starter_kit());
    let checked = Attrs::from_iter([("checked", crate::AttrValue::Bool(true))]);
    let item = s
        .create_node("task_item", checked, Fragment::from_node(p_of(&s, "ab")))
        .unwrap();
    let list = s.branch("task_list", Fragment::from_node(item)).unwrap();
    let doc = s.branch("doc", Fragment::from_node(list)).unwrap();
    let mut state = EditorState::create(s, doc, default_plugins());
    // End of "ab" (cursor_in_para lands at content offset 0; +2 reaches the end).
    state.selection = Selection::cursor(Pos(cursor_in_para(&state.doc, "ab") + 2));

    let out = state.run("enter").expect("enter splits the task item");
    let list = out.doc.child(0);
    assert_eq!(list.type_name(), "task_list");
    assert_eq!(list.child_count(), 2, "a new sibling task item was created");
    let new_item = list.child(1);
    assert_eq!(new_item.type_name(), "task_item");
    assert_ne!(
        new_item.attrs().get_bool("checked"),
        Some(true),
        "the new task item starts unchecked"
    );
}
