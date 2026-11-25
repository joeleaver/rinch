//! Nesting list items: sink, indent and outdent in lists, task lists and table
//! cells.

use super::*;

#[test]
fn sink_nests_second_list_item() {
    // build doc(bullet_list(list_item(p"a"), list_item(p"b"))); cursor in 2nd item
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
    let doc = s.branch("doc", Fragment::from_node(list)).unwrap();
    let mut state = EditorState::create(s, doc, vec![Rc::new(BaseCommandsPlugin)]);
    // positions: 0[ul 1[li 2[p 3 a 4]5]6 [li 7[p 8 b 9]10]11]12 ; cursor in 2nd item's text
    state.selection = Selection::cursor(Pos(8));
    assert!(can_sink(&state));
    let nested = state.run("sinkListItem").expect("sink applies");
    // the second item is now nested inside the first item's sublist
    let outer_list = nested.doc.child(0);
    assert_eq!(outer_list.type_name(), "bullet_list");
    assert_eq!(
        outer_list.child_count(),
        1,
        "second item nested under the first"
    );
    let first_item = outer_list.child(0);
    // first item now holds: paragraph "a", bullet_list(list_item(p"b"))
    let inner_list = first_item.child(first_item.child_count() - 1);
    assert_eq!(inner_list.type_name(), "bullet_list");
    assert_eq!(all_text(inner_list), "b");
}

/// True if any paragraph in `doc` carries a non-zero margin `indent` attribute.
fn any_margin_indent(doc: &Node) -> bool {
    let mut found = false;
    doc.nodes_between(0, doc.content_size(), &mut |node, _, _| {
        if node.type_name() == "paragraph" && node.attrs().get_int("indent").unwrap_or(0) != 0 {
            found = true;
        }
        true
    });
    found
}

/// The single direct sublist child (a list node) of `item`, if any.
fn sublist_of<'a>(item: &'a Node, list_ty: &str) -> Option<&'a Node> {
    item.content().iter().find(|n| n.type_name() == list_ty)
}

fn li(s: &Rc<Schema>, t: &str) -> Node {
    s.branch("list_item", Fragment::from_node(p_of(s, t)))
        .unwrap()
}

fn ul(s: &Rc<Schema>, items: Vec<Node>) -> Node {
    s.branch("bullet_list", Fragment::from_children(items))
        .unwrap()
}

#[test]
fn outdent_merges_the_lifted_items_own_sublist_with_its_trailing_siblings() {
    // doc( ul( li(A, ul( li(B, ul(li C)), li D )) ) ): B owns a sublist [C] AND has a
    // trailing sibling D. Outdenting B would leave B with two adjacent sublists
    // (ul[C] then ul[D]); the canJoin/join polish merges them into one ul[C, D].
    let s = Rc::new(Schema::starter_kit());
    let b = s
        .branch(
            "list_item",
            Fragment::from_children(vec![p_of(&s, "B"), ul(&s, vec![li(&s, "C")])]),
        )
        .unwrap();
    let a = s
        .branch(
            "list_item",
            Fragment::from_children(vec![p_of(&s, "A"), ul(&s, vec![b, li(&s, "D")])]),
        )
        .unwrap();
    let doc = s
        .branch("doc", Fragment::from_node(ul(&s, vec![a])))
        .unwrap();
    let mut st = EditorState::create(s, doc, default_plugins());
    st.selection = Selection::cursor(Pos(cursor_in_para(&st.doc, "B")));

    let out = st.run("outdent").expect("outdent lifts B");
    let b_item = out.doc.child(0).child(1); // top list: [A, B]
    assert_eq!(all_text(b_item.child(0)), "B");
    // Exactly ONE sublist under B, holding both C and D in order — not two.
    let sublists: Vec<&Node> = b_item
        .content()
        .iter()
        .filter(|n| n.type_name() == "bullet_list")
        .collect();
    assert_eq!(
        sublists.len(),
        1,
        "the two adjacent sublists were joined into one"
    );
    assert_eq!(
        sublists[0].child_count(),
        2,
        "C and D share the merged sublist"
    );
    assert_eq!(all_text(sublists[0].child(0)), "C");
    assert_eq!(all_text(sublists[0].child(1)), "D");
}

#[test]
fn outdent_nested_list_item_lifts_to_a_sibling_not_a_continuation_paragraph() {
    // doc( ul( li(A, ul(li B)) ) ); outdent B. B must become a SIBLING list item of
    // A in the outer list — NOT a bullet-less continuation paragraph of A.
    let s = Rc::new(Schema::starter_kit());
    let item_a = s
        .branch(
            "list_item",
            Fragment::from_children(vec![p_of(&s, "A"), ul(&s, vec![li(&s, "B")])]),
        )
        .unwrap();
    let doc = s
        .branch("doc", Fragment::from_node(ul(&s, vec![item_a])))
        .unwrap();
    let mut st = EditorState::create(s, doc, default_plugins());
    st.selection = Selection::cursor(Pos(cursor_in_para(&st.doc, "B")));

    let out = st.run("outdent").expect("outdent lifts B to a sibling");
    let top = out.doc.child(0);
    assert_eq!(top.child_count(), 2, "A and B are now sibling items");
    for i in 0..2 {
        let item = top.child(i);
        assert_eq!(item.type_name(), "list_item");
        assert_eq!(
            item.child(0).type_name(),
            "paragraph",
            "each item keeps a bullet"
        );
    }
    assert_eq!(all_text(top.child(1)), "B");
}

#[test]
fn outdent_nested_item_keeps_its_trailing_siblings_nested_under_it() {
    // doc( ul( li(A, ul(li C, li D)) ) ); outdent C. C lifts to a sibling of A, and
    // its trailing sibling D rides along NESTED under C (PM's liftToOuterList) —
    // never orphaned into a bullet-less wrapper.
    let s = Rc::new(Schema::starter_kit());
    let item_a = s
        .branch(
            "list_item",
            Fragment::from_children(vec![p_of(&s, "A"), ul(&s, vec![li(&s, "C"), li(&s, "D")])]),
        )
        .unwrap();
    let doc = s
        .branch("doc", Fragment::from_node(ul(&s, vec![item_a])))
        .unwrap();
    let mut st = EditorState::create(s, doc, default_plugins());
    st.selection = Selection::cursor(Pos(cursor_in_para(&st.doc, "C")));

    let out = st.run("outdent").expect("outdent lifts C");
    let top = out.doc.child(0);
    assert_eq!(top.child_count(), 2, "A and C are sibling items");
    let c = top.child(1);
    assert_eq!(c.child(0).type_name(), "paragraph", "C keeps its bullet");
    assert_eq!(all_text(c.child(0)), "C");
    let d_sub = sublist_of(c, "bullet_list").expect("D nested under C");
    assert_eq!(all_text(d_sub), "D");
}

#[test]
fn outdent_outermost_list_item_unlists_it_to_the_top_level() {
    // doc( ul(li X, li Y) ); outdent Y → Y becomes a top-level paragraph (un-listed).
    let s = Rc::new(Schema::starter_kit());
    let doc = s
        .branch(
            "doc",
            Fragment::from_node(ul(&s, vec![li(&s, "X"), li(&s, "Y")])),
        )
        .unwrap();
    let mut st = EditorState::create(s, doc, default_plugins());
    st.selection = Selection::cursor(Pos(cursor_in_para(&st.doc, "Y")));

    let out = st.run("outdent").expect("outdent un-lists Y");
    assert_eq!(
        out.doc.child_count(),
        2,
        "the list, then the lifted paragraph"
    );
    assert_eq!(out.doc.child(0).type_name(), "bullet_list");
    assert_eq!(all_text(out.doc.child(0)), "X", "X stays in the list");
    assert_eq!(out.doc.child(1).type_name(), "paragraph");
    assert_eq!(all_text(out.doc.child(1)), "Y");
}

#[test]
fn indent_then_outdent_round_trips_a_list_item() {
    // Indent twice to nest deep, then outdent twice — the document returns to a flat
    // three-item list (the indent merge and the list-aware lift are inverses here).
    let s = Rc::new(Schema::starter_kit());
    let doc = s
        .branch(
            "doc",
            Fragment::from_node(ul(&s, vec![li(&s, "A"), li(&s, "B"), li(&s, "C")])),
        )
        .unwrap();
    let mut st = EditorState::create(s, doc, default_plugins());
    // Indent B under A, then C under A (merges), then C under B (deeper).
    st.selection = Selection::cursor(Pos(cursor_in_para(&st.doc, "B")));
    let st = st.run("indent").expect("indent B");
    let mut st = st;
    st.selection = Selection::cursor(Pos(cursor_in_para(&st.doc, "C")));
    let st = st.run("indent").expect("indent C once");
    let mut st = st;
    st.selection = Selection::cursor(Pos(cursor_in_para(&st.doc, "C")));
    let st = st.run("indent").expect("indent C twice");
    // Now outdent C twice back out to the top level.
    let mut st = st;
    st.selection = Selection::cursor(Pos(cursor_in_para(&st.doc, "C")));
    let st = st.run("outdent").expect("outdent C once");
    let mut st = st;
    st.selection = Selection::cursor(Pos(cursor_in_para(&st.doc, "C")));
    let st = st.run("outdent").expect("outdent C twice");

    let top = st.doc.child(0);
    assert_eq!(top.type_name(), "bullet_list");
    // B is still nested under A (we only outdented C); C is back at the top list.
    let texts: Vec<String> = top.content().iter().map(all_text).collect();
    assert!(
        texts.iter().any(|t| t == "C"),
        "C returned to the top-level list, got {texts:?}"
    );
}

#[test]
fn indent_merges_into_previous_sublist_and_then_nests_deeper() {
    // doc( bullet_list( list_item(pA, bullet_list(list_item pB)), list_item(pC) ) ):
    // "A" already owns a sublist holding "B"; "C" is its top-level sibling.
    let s = Rc::new(Schema::starter_kit());
    let inner_b = s
        .branch("list_item", Fragment::from_node(p_of(&s, "B")))
        .unwrap();
    let inner_list = s
        .branch("bullet_list", Fragment::from_node(inner_b))
        .unwrap();
    let item_a = s
        .branch(
            "list_item",
            Fragment::from_children(vec![p_of(&s, "A"), inner_list]),
        )
        .unwrap();
    let item_c = s
        .branch("list_item", Fragment::from_node(p_of(&s, "C")))
        .unwrap();
    let bl = s
        .branch("bullet_list", Fragment::from_children(vec![item_a, item_c]))
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(bl)).unwrap();
    let mut state = EditorState::create(s, doc, default_plugins());

    // Indent #1: "C" joins "A"'s EXISTING sublist (merge) — not a new adjacent one.
    state.selection = Selection::cursor(Pos(cursor_in_para(&state.doc, "C")));
    let s1 = state.run("indent").expect("first indent merges C under A");
    let top = s1.doc.child(0);
    assert_eq!(top.child_count(), 1, "C lifted out of the top-level list");
    let a = top.child(0);
    let a_sublists = a
        .content()
        .iter()
        .filter(|n| n.type_name() == "bullet_list")
        .count();
    assert_eq!(
        a_sublists, 1,
        "C merged into A's one sublist, not an adjacent one"
    );
    let sub = sublist_of(a, "bullet_list").unwrap();
    assert_eq!(sub.child_count(), 2, "the sublist now holds both B and C");

    // Indent #2: with B as its previous sibling, "C" now nests DEEPER under B —
    // proving an item can be indented more than once (the reported bug).
    let mut s1 = s1;
    s1.selection = Selection::cursor(Pos(cursor_in_para(&s1.doc, "C")));
    let s2 = s1.run("indent").expect("second indent nests C under B");
    let sub = sublist_of(s2.doc.child(0).child(0), "bullet_list").unwrap();
    assert_eq!(sub.child_count(), 1, "only B remains at the first sublevel");
    let nested = sublist_of(sub.child(0), "bullet_list").expect("C nested under B");
    assert_eq!(all_text(nested), "C");
}

#[test]
fn indent_in_task_list_nests_and_never_margin_indents() {
    // doc( task_list( task_item(p"a"), task_item(p"b") ) ); cursor in the 2nd item.
    let s = Rc::new(Schema::starter_kit());
    let item_a = s
        .branch("task_item", Fragment::from_node(p_of(&s, "a")))
        .unwrap();
    let item_b = s
        .branch("task_item", Fragment::from_node(p_of(&s, "b")))
        .unwrap();
    let tl = s
        .branch("task_list", Fragment::from_children(vec![item_a, item_b]))
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(tl)).unwrap();
    let mut state = EditorState::create(s, doc, default_plugins());
    state.selection = Selection::cursor(Pos(cursor_in_para(&state.doc, "b")));

    // Indent nests item "b" under item "a" — the top task_list now holds ONE item,
    // and crucially NO paragraph receives a left margin (the checkbox-split bug).
    let nested = state.run("indent").expect("indent nests the task item");
    let tl = nested.doc.child(0);
    assert_eq!(tl.type_name(), "task_list");
    assert_eq!(tl.child_count(), 1, "second item nested under the first");
    assert!(
        !any_margin_indent(&nested.doc),
        "task-list indent nests, never margin-indents"
    );
}

#[test]
fn tab_sink_list_item_nests_a_task_item_and_skips_plain_paragraphs() {
    // Tab (sinkListItem) and can_sink must handle task lists, matching the Indent
    // button / Shift-Tab — but Tab must still be a no-op (never margin-indent) on a
    // plain paragraph.
    let s = Rc::new(Schema::starter_kit());
    let item_a = s
        .branch("task_item", Fragment::from_node(p_of(&s, "a")))
        .unwrap();
    let item_b = s
        .branch("task_item", Fragment::from_node(p_of(&s, "b")))
        .unwrap();
    let tl = s
        .branch("task_list", Fragment::from_children(vec![item_a, item_b]))
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(tl)).unwrap();
    let mut state = EditorState::create(s, doc, default_plugins());
    state.selection = Selection::cursor(Pos(cursor_in_para(&state.doc, "b")));

    assert!(can_sink(&state), "can_sink is true inside a task list");
    let nested = state.run("sinkListItem").expect("Tab nests the task item");
    assert_eq!(nested.doc.child(0).type_name(), "task_list");
    assert_eq!(nested.doc.child(0).child_count(), 1, "b nested under a");
    assert!(
        !any_margin_indent(&nested.doc),
        "Tab nests, never margin-indents"
    );

    // Tab in a plain paragraph does nothing (the button would margin-indent; Tab won't).
    let mut plain = editor("hi");
    plain.selection = Selection::cursor(Pos(2));
    assert!(!can_sink(&plain), "can_sink is false outside a list");
    assert!(
        plain.run("sinkListItem").is_none(),
        "Tab is a no-op in a plain paragraph"
    );
}

#[test]
fn indent_first_task_item_is_a_noop() {
    // The first (only) task item has no previous sibling to nest under. Indent must be
    // a true no-op — in particular it must NOT margin-indent the item's paragraph.
    let s = Rc::new(Schema::starter_kit());
    let tl = s
        .branch(
            "task_list",
            Fragment::from_node(
                s.branch("task_item", Fragment::from_node(p_of(&s, "a")))
                    .unwrap(),
            ),
        )
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(tl)).unwrap();
    let mut state = EditorState::create(s, doc, default_plugins());
    state.selection = Selection::cursor(Pos(cursor_in_para(&state.doc, "a")));
    assert!(
        state.run("indent").is_none(),
        "indent on the first task item is a no-op"
    );
}

#[test]
fn outdent_in_task_list_lifts_the_item_out() {
    // Outdent un-task-lists the item (lifts it to a top-level paragraph) rather than
    // adjusting a margin — consistent with how setParagraph treats task lists.
    let s = Rc::new(Schema::starter_kit());
    let tl = s
        .branch(
            "task_list",
            Fragment::from_node(
                s.branch("task_item", Fragment::from_node(p_of(&s, "a")))
                    .unwrap(),
            ),
        )
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(tl)).unwrap();
    let mut state = EditorState::create(s, doc, default_plugins());
    state.selection = Selection::cursor(Pos(cursor_in_para(&state.doc, "a")));
    let out = state
        .run("outdent")
        .expect("outdent lifts the task item out");
    assert!(
        out.doc
            .content()
            .iter()
            .all(|n| n.type_name() != "task_list"),
        "task list removed"
    );
    assert_eq!(out.doc.child(0).type_name(), "paragraph");
    assert_eq!(all_text(out.doc.child(0)), "a");
}

#[test]
fn indent_paragraph_in_table_cell_is_a_noop() {
    // The Indent toolbar button must never margin-indent a paragraph that lives in a
    // table cell (a left margin there would split the text from the cell layout).
    let s = Rc::new(Schema::starter_kit());
    let cell = s
        .branch("table_cell", Fragment::from_node(p_of(&s, "c")))
        .unwrap();
    let table = s
        .branch(
            "table",
            Fragment::from_node(s.branch("table_row", Fragment::from_node(cell)).unwrap()),
        )
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(table)).unwrap();
    let mut state = EditorState::create(s, doc, default_plugins());
    state.selection = Selection::cursor(Pos(cursor_in_para(&state.doc, "c")));
    assert!(
        state.run("indent").is_none(),
        "indenting a paragraph inside a table cell is a no-op"
    );
    // Shift-Tab in the first cell makes tab_cell fall through to liftListItem, and
    // sinkListItem likewise — both MUST be no-ops in a cell. liftListItem in
    // particular previously unwrapped the paragraph out of the cell and DELETED the
    // whole table; the isolating-boundary guard in `build_lift` now forbids it.
    assert!(
        state.run("liftListItem").is_none(),
        "liftListItem must not lift a paragraph out of a table cell"
    );
    assert!(
        state.run("sinkListItem").is_none(),
        "sinkListItem is a no-op inside a table cell"
    );
    // The starting doc is unchanged — the table is intact.
    assert_eq!(state.doc.child(0).type_name(), "table");
}

#[test]
fn outdent_outermost_list_item_in_table_cell_keeps_the_table() {
    // A real bullet list living *inside* a table cell. Outdenting its only (outermost)
    // item must un-list it within the cell — never lift it past the isolating cell
    // boundary (which would tear the table). `build_lift_list_item` routes the
    // outermost case through `build_lift`, whose isolating floor stops at the cell.
    let s = Rc::new(Schema::starter_kit());
    let item = s
        .branch("list_item", Fragment::from_node(p_of(&s, "a")))
        .unwrap();
    let list = s.branch("bullet_list", Fragment::from_node(item)).unwrap();
    let cell = s.branch("table_cell", Fragment::from_node(list)).unwrap();
    let table = s
        .branch(
            "table",
            Fragment::from_node(s.branch("table_row", Fragment::from_node(cell)).unwrap()),
        )
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(table)).unwrap();
    let mut state = EditorState::create(s, doc, default_plugins());
    state.selection = Selection::cursor(Pos(cursor_in_para(&state.doc, "a")));

    // A no-op is acceptable; what is NOT acceptable is the content escaping the cell.
    let out = state.run("liftListItem").unwrap_or(state);
    assert_eq!(
        out.doc.child_count(),
        1,
        "nothing lifted to the doc top level"
    );
    assert_eq!(out.doc.child(0).type_name(), "table", "table preserved");
    assert!(
        all_text(out.doc.child(0)).contains('a'),
        "content kept in the cell"
    );
}

#[test]
fn outdent_nested_list_item_in_table_cell_stays_in_cell() {
    // A nested list item inside a cell: cell > list > item("a" + sublist > item "b").
    // Outdenting "b" lifts it to a sibling of "a" in the cell's outer list (PM's
    // `liftToOuterList`, target = depth-2); a second outdent un-lists it within the
    // cell. Neither step may cross the cell boundary.
    let s = Rc::new(Schema::starter_kit());
    let inner_item = s
        .branch("list_item", Fragment::from_node(p_of(&s, "b")))
        .unwrap();
    let inner_list = s
        .branch("bullet_list", Fragment::from_node(inner_item))
        .unwrap();
    let outer_item = s
        .branch(
            "list_item",
            Fragment::from_children(vec![p_of(&s, "a"), inner_list]),
        )
        .unwrap();
    let outer_list = s
        .branch("bullet_list", Fragment::from_node(outer_item))
        .unwrap();
    let cell = s
        .branch("table_cell", Fragment::from_node(outer_list))
        .unwrap();
    let table = s
        .branch(
            "table",
            Fragment::from_node(s.branch("table_row", Fragment::from_node(cell)).unwrap()),
        )
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(table)).unwrap();
    let mut state = EditorState::create(s, doc, default_plugins());
    state.selection = Selection::cursor(Pos(cursor_in_para(&state.doc, "b")));

    // First outdent: "b" rises to the cell's outer list — table intact, both in cell.
    let once = state.run("liftListItem").expect("nested item lifts");
    assert_eq!(
        once.doc.child_count(),
        1,
        "nothing lifted to the doc top level"
    );
    assert_eq!(once.doc.child(0).type_name(), "table");
    let table_text = all_text(once.doc.child(0));
    assert!(
        table_text.contains('a') && table_text.contains('b'),
        "both items kept inside the table: {table_text:?}"
    );

    // Second outdent (now outermost): still confined to the cell.
    let twice = {
        let mut s2 = once;
        s2.selection = Selection::cursor(Pos(cursor_in_para(&s2.doc, "b")));
        s2.run("liftListItem").unwrap_or(s2)
    };
    assert_eq!(
        twice.doc.child_count(),
        1,
        "still nothing at the doc top level"
    );
    assert_eq!(
        twice.doc.child(0).type_name(),
        "table",
        "table still intact"
    );
    assert!(all_text(twice.doc.child(0)).contains('b'));
}
